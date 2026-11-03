use std::path::{Path, PathBuf};

use clap::Parser;
use colored::Colorize;
use ignore::WalkBuilder;
use rayon::prelude::*;
use tree_sitter::{Parser as TParser, Query, QueryCursor, StreamingIterator};

#[derive(Parser)]
#[command()]
struct Cli {
    #[arg(default_value = "./")]
    path: String,

    #[arg(
        short,
        long,
        default_value_t = 20,
        help = "character length of the return type"
    )]
    threshold: usize,

    #[arg(
        short,
        long,
        default_value_t = false,
        help = "Auto fix long return types"
    )]
    fix: bool,
}

const QUERY_STR: &str = r"
    [
        (function_definition
            (type_qualifier)? @qualifier
            type: (_) @return_type
            declarator: (_) @declarator
        )
        (field_declaration
            (type_qualifier)?
            type: (_) @return_type
            declarator: (_) @declarator
        )
    ]";

fn main() {
    let cli = Cli::parse();
    let files = find_filepaths(&cli.path);

    let query = Query::new(&tree_sitter_cpp::LANGUAGE.into(), QUERY_STR).unwrap();

    files.par_iter().for_each(|path| {
        process_file(path, cli.threshold, &query, cli.fix);
    });
}

fn find_filepaths(filepath: &str) -> Vec<PathBuf> {
    let path = Path::new(filepath);
    WalkBuilder::new(path)
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry.path().extension().is_some_and(|ext| {
                matches!(
                    ext.to_str(),
                    Some("cpp" | "hpp" | "cc" | "hh" | "cxx" | "hxx" | "h")
                )
            })
        })
        .map(ignore::DirEntry::into_path)
        .collect()
}

struct FunctionInfo<'path> {
    return_type_range: core::ops::Range<usize>,
    declarator_range: core::ops::Range<usize>,
    line: usize,
    path: &'path Path,
}

impl<'a> FunctionInfo<'_> {
    fn return_type(&self, source_code: &'a str) -> &'a str {
        source_code[self.return_type_range.clone()].trim_end()
    }

    fn declarator(&self, source_code: &'a str) -> &'a str {
        &source_code[self.declarator_range.clone()]
    }

    fn display(&self, source_code: &str) -> String {
        format!(
            "{}:{}: {} {}",
            self.path.display().to_string().green(),
            self.line.to_string().bright_blue(),
            self.return_type(source_code).red().bold(),
            self.declarator(source_code)
        )
    }
}

fn parse_source_code<'path>(
    source_code: &str,
    path: &'path Path,
    query: &Query,
) -> Vec<FunctionInfo<'path>> {
    let mut parser = TParser::new();
    parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .expect("Error loading C++ grammar");

    let tree = parser
        .parse(source_code, None)
        .expect("Failed to parse source code file");

    let root_node = tree.root_node();

    let mut cursor = QueryCursor::new();

    let mut query_matches = cursor.matches(query, root_node, source_code.as_bytes());

    // if a capture name is present in the query even if it is marked with "?" or it is not present
    // after the query run, `capture_index_from_name` always return Some and an index inside. To check
    // if it is exist, use `nodes_for_capture_index` on every query match
    let qualifier_idx = query
        .capture_index_for_name("qualifier")
        .expect("Failed to obtain qualifier");

    let return_type_idx = query
        .capture_index_for_name("return_type")
        .expect("Failed to obtain return type");

    let declarator_idx = query
        .capture_index_for_name("declarator")
        .expect("Failed to obtain function declarator");

    let mut return_vec = vec![];
    while let Some(q_match) = query_matches.next() {
        let return_type_node = q_match
            .nodes_for_capture_index(return_type_idx)
            .next()
            .unwrap();

        let mut declarator_node = q_match
            .nodes_for_capture_index(declarator_idx)
            .next()
            .unwrap();

        // Unwrap pointer/reference declarators down to the underlying function_declarator
        while declarator_node.kind() != "function_declarator" {
            let Some(child) = declarator_node.named_child(0) else {
                break;
            };
            declarator_node = child;
        }

        // skip member variables
        if declarator_node.kind() != "function_declarator" {
            continue;
        }

        // check for virtual specifier
        let mut declarator_node_cursor = declarator_node.walk();
        let virtual_node = declarator_node
            .named_children(&mut declarator_node_cursor)
            .find(|child| child.kind() == "virtual_specifier");

        // if qualifier exists (const, volatile) add qualifier to return type so it will be
        // moved to the trailing return section
        let return_type_range = q_match
            .nodes_for_capture_index(qualifier_idx)
            .next()
            .map_or_else(
                || core::ops::Range::<usize> {
                    start: return_type_node.start_byte(),
                    end: declarator_node.start_byte() - 1,
                },
                |qualifier_node| core::ops::Range::<usize> {
                    start: qualifier_node.start_byte(),
                    end: declarator_node.start_byte() - 1,
                },
            );

        let declarator_range = if let Some(spec) = virtual_node {
            core::ops::Range {
                start: declarator_node.start_byte(),
                end: spec.start_byte() - 1,
            }
        } else {
            declarator_node.byte_range()
        };

        let line = return_type_node.start_position().row + 1;

        return_vec.push(FunctionInfo {
            return_type_range,
            declarator_range,
            line,
            path,
        });
    }

    return_vec
}

fn process_file(path: &Path, threshold: usize, query: &Query, fix: bool) {
    let Ok(mut source_code) = std::fs::read_to_string(path) else {
        return;
    };

    let parsed = parse_source_code(&source_code, path, query);
    let filtered_iter = parsed.into_iter().filter(|info| {
        info.return_type(&source_code) != "auto" && info.return_type_range.len() >= threshold
    });

    if fix {
        // sort from last to first
        let mut filtered: Vec<_> = filtered_iter.collect();
        filtered.sort_by(|info1, info2| {
            info2
                .return_type_range
                .start
                .cmp(&info1.return_type_range.start)
        });

        auto_fix(&filtered, &mut source_code);
        let _ = std::fs::write(path, source_code);
    } else {
        filtered_iter.for_each(|info| {
            println!("{}", info.display(&source_code));
        });
    }
}

fn auto_fix(reversed_parsed_arr: &[FunctionInfo], source_code: &mut String) {
    for info in reversed_parsed_arr {
        let trailing_return = format!(" -> {}", info.return_type(source_code));
        source_code.insert_str(info.declarator_range.end, &trailing_return);
        source_code.replace_range(info.return_type_range.clone(), "auto");
    }
}

#[cfg(test)]
mod test {
    use std::fs;
    use std::path::PathBuf;

    use tree_sitter::Query;

    use crate::{QUERY_STR, auto_fix, parse_source_code, process_file};

    const DEFAULT_THRESHOLD: usize = 20;

    fn query() -> Query {
        Query::new(&tree_sitter_cpp::LANGUAGE.into(), QUERY_STR).unwrap()
    }

    fn fixed(source: &str, threshold: usize) -> String {
        let mut source = source.to_string();
        let path = PathBuf::from("test.cpp");
        let query = query();
        let mut functions: Vec<_> = parse_source_code(&source, &path, &query)
            .into_iter()
            .filter(|info| {
                info.return_type(&source) != "auto" && info.return_type(&source).len() >= threshold
            })
            .collect();

        functions.sort_by(|info1, info2| {
            info2
                .return_type_range
                .start
                .cmp(&info1.return_type_range.start)
        });
        auto_fix(&functions, &mut source);
        source
    }

    #[test]
    fn test_auto_fix_from_demo() {
        const EXPECTED_SOURCE_CODE: &str = r"class Engine {
    Memtable memtable_;

public:
    void put(RecordType record_type, ByteVec key, ByteVec value) noexcept;

    [[nodiscard]]
    auto get(ByteSpan key) const noexcept -> std::expected<ByteVec*, ErrorCode>;

    auto get(ByteSpan key) noexcept -> std::expected<ByteVec*, ErrorCode>;

    auto write_sstable(const char* path) const noexcept -> std::expected<SSTable, ErrorCode>;

    void restore(Memtable&& table_to_load) noexcept;

    void clear_memtable() noexcept;
};
";

        let source = r"class Engine {
    Memtable memtable_;

public:
    void put(RecordType record_type, ByteVec key, ByteVec value) noexcept;

    [[nodiscard]]
    std::expected<ByteVec*, ErrorCode> get(ByteSpan key) const noexcept;

    std::expected<ByteVec*, ErrorCode> get(ByteSpan key) noexcept;

    std::expected<SSTable, ErrorCode> write_sstable(const char* path) const noexcept;

    void restore(Memtable&& table_to_load) noexcept;

    void clear_memtable() noexcept;
};
";

        assert_eq!(fixed(source, DEFAULT_THRESHOLD), EXPECTED_SOURCE_CODE);
    }

    #[test]
    fn auto_fix_is_persisted_to_disk() {
        let path = std::env::temp_dir().join(format!(
            "func_grep_autofix_persist_{}.cpp",
            std::process::id()
        ));
        let source = "std::vector<std::string> make_names() { return {}; }\n";
        fs::write(&path, source).unwrap();

        let query = query();
        process_file(&path, 1, &query, true);

        let fixed_source = fs::read_to_string(&path).unwrap();
        assert_eq!(
            fixed_source,
            "auto make_names() -> std::vector<std::string> { return {}; }\n"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn auto_fix_inserts_trailing_return_before_override() {
        let source = "struct Foo {\n    std::vector<std::string> over() const override;\n};\n";
        let expected =
            "struct Foo {\n    auto over() const -> std::vector<std::string> override;\n};\n";

        assert_eq!(fixed(source, 1), expected);
    }

    #[test]
    fn auto_fix_inserts_trailing_return_before_final() {
        let source = "struct Foo {\n    virtual std::vector<std::string> fin() const final;\n};\n";
        let expected =
            "struct Foo {\n    virtual auto fin() const -> std::vector<std::string> final;\n};\n";

        assert_eq!(fixed(source, 1), expected);
    }

    #[test]
    fn auto_fix_inserts_trailing_return_before_requires() {
        let source = r"struct Foo {
    template <typename T>
    std::vector<std::string> req(T t) const requires std::integral<T>;
};
";
        let expected = r"struct Foo {
    template <typename T>
    auto req(T t) const -> std::vector<std::string> requires std::integral<T>;
};
";

        assert_eq!(fixed(source, 1), expected);
    }

    #[test]
    fn auto_fix_moves_leading_const_into_trailing_return() {
        let source = "const std::string& get_name() noexcept { static std::string s; return s; }\n";
        let expected =
            "auto get_name() noexcept -> const std::string& { static std::string s; return s; }\n";

        assert_eq!(fixed(source, 1), expected);
    }

    #[test]
    fn auto_fix_moves_leading_volatile_into_trailing_return() {
        let source = "volatile int& get_ref() { static int x; return x; }\n";
        let expected = "auto get_ref() -> volatile int& { static int x; return x; }\n";

        assert_eq!(fixed(source, 1), expected);
    }

    #[test]
    fn auto_fix_handles_return_type_without_space_before_declarator() {
        let source = "int&no_space() { static int x; return x; }\n";
        let expected = "auto no_space() -> int& { static int x; return x; }\n";

        assert_eq!(fixed(source, 1), expected);
    }

    #[test]
    fn return_type_range_excludes_extra_spaces() {
        let source = "struct Foo {\n    std::string   spaced_name();\n};\n";
        let path = PathBuf::from("test.hpp");
        let query = query();

        let parsed = parse_source_code(source, &path, &query);

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].return_type(source), "std::string");
        assert_eq!(
            parsed[0].return_type_range.len(),
            parsed[0].return_type(source).len()
        );
    }

    #[test]
    fn auto_fix_skips_placeholder_auto_pointer_return_type() {
        let source = "auto* auto_ptr() { return static_cast<int*>(nullptr); }\n";

        assert_eq!(fixed(source, 1), source);
    }

    #[test]
    fn parses_namespace_scope_function_declaration() {
        let source = "std::vector<std::string> free_proto();\n";
        let path = PathBuf::from("test.hpp");
        let query = query();

        let parsed = parse_source_code(source, &path, &query);

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].return_type(source), "std::vector<std::string>");
        assert_eq!(parsed[0].declarator(source), "free_proto()");
    }
}
