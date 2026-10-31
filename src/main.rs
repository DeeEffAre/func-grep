use std::path::{Path, PathBuf};

use clap::Parser;
use ignore::WalkBuilder;
use rayon::prelude::*;
use tree_sitter::{Parser as TParser, Query, QueryCursor, StreamingIterator};

#[derive(Parser)]
#[command()]
struct Cli {
    #[arg(default_value = "./")]
    path: String,

    #[arg(short, long, default_value_t = 20)]
    threshold: usize,
}

fn main() {
    let cli = Cli::parse();
    let files = find_filepaths(&cli.path);

    let query_str = r"
    [
        (function_definition
            type: (_) @return_type
            declarator: (function_declarator) @func_decl
        )
        (field_declaration
            type: (_) @return_type
            declarator: (function_declarator) @func_decl
        )
    ]";
    let query = Query::new(&tree_sitter_cpp::LANGUAGE.into(), query_str).unwrap();

    files.par_iter().for_each(|path| {
        process_file(path, cli.threshold, &query);
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

struct FunctionInfo<'path, 'source_code> {
    return_type: &'source_code str,
    declarator: &'source_code str,
    line: usize,
    path: &'path Path,
}

impl std::fmt::Display for FunctionInfo<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}: {} {}",
            self.path.display(),
            self.line,
            self.return_type,
            self.declarator
        )
    }
}

fn parse_source_code<'path, 'source_code>(
    source_code: &'source_code str,
    path: &'path Path,
    query: &Query,
) -> Vec<FunctionInfo<'path, 'source_code>> {
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

    let mut return_vec = vec![];
    while let Some(q_match) = query_matches.next() {
        let captures = q_match.captures();
        let return_type_capture = captures.first().expect("Failed to obtain return type");
        let return_type_node = return_type_capture.node;
        let return_type_text = &source_code[return_type_node.byte_range()];

        let line = return_type_node.start_position().row + 1;

        let function_declarator_capture = captures.get(1).unwrap();
        let function_declarator_node = function_declarator_capture.node;
        let function_declarator_text = &source_code[function_declarator_node.byte_range()];

        return_vec.push(FunctionInfo {
            return_type: return_type_text,
            declarator: function_declarator_text,
            line,
            path,
        });
    }

    return_vec
}

fn process_file(path: &Path, threshold: usize, query: &Query) {
    let Ok(source_code) = std::fs::read_to_string(path) else {
        return;
    };

    let parsed = parse_source_code(&source_code, path, query);

    parsed
        .iter()
        .filter(|info| info.return_type != "auto" && info.return_type.len() >= threshold)
        .for_each(|info| {
            println!("{info}");
        });
}
