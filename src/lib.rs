use std::{
    fmt::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use colored::Colorize;
use ignore::WalkBuilder;
use rayon::prelude::*;
use tree_sitter::{Parser as TParser, Query, QueryCursor, StreamingIterator};

mod lib_test;

const QUERY_STR: &str = r"
    [
        (declaration
            (type_qualifier)* @qualifier
            type: (_) @return_type
            declarator: (_) @declarator
        )
        (function_definition
            (type_qualifier)* @qualifier
            type: (_) @return_type
            declarator: (_) @declarator
        )
        (field_declaration
            (type_qualifier)* @qualifier
            type: (_) @return_type
            declarator: (_) @declarator
        )
    ]";

#[derive(Parser)]
#[command()]
pub struct Cli {
    #[arg(default_value = "./")]
    pub path: String,

    #[arg(
        short,
        long,
        default_value_t = 20,
        help = "character length of the return type"
    )]
    pub threshold: usize,

    #[arg(
        short,
        long,
        default_value_t = false,
        help = "Auto fix long return types"
    )]
    pub fix: bool,
}

struct FunctionInfo<'path> {
    return_type_range: core::ops::Range<usize>,
    declarator_range: core::ops::Range<usize>,
    line: usize,
    path: &'path Path,
    auto_replacement: &'static str,
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

struct CaptureIndexes {
    qualifier: u32,
    return_type: u32,
    declarator: u32,
}

impl CaptureIndexes {
    fn new(query: &Query) -> Result<Self> {
        // if a capture name is present in the query even if it is marked with "?" or it is not present
        // after the query run, `capture_index_from_name` always return Some and an index inside. To check
        // if it is exist, use `nodes_for_capture_index` on every query match
        let qualifier_idx = query
            .capture_index_for_name("qualifier")
            .with_context(|| "Failed to obtain qualifier")?;

        let return_type_idx = query
            .capture_index_for_name("return_type")
            .with_context(|| "Failed to obtain return type")?;

        let declarator_idx = query
            .capture_index_for_name("declarator")
            .with_context(|| "Failed to obtain function declarator")?;

        Ok(Self {
            qualifier: qualifier_idx,
            return_type: return_type_idx,
            declarator: declarator_idx,
        })
    }
}

pub struct App {
    pub files: Vec<PathBuf>,
    query: Query,
    config: Cli,
    capture_indexes: CaptureIndexes,
}

impl App {
    pub fn new(config: Cli) -> Result<Self> {
        let files = Self::find_filepaths(&config.path);
        let query = Query::new(&tree_sitter_cpp::LANGUAGE.into(), QUERY_STR)?;
        let capture_indexes = CaptureIndexes::new(&query)?;

        Ok(Self {
            files,
            query,
            config,
            capture_indexes,
        })
    }

    pub fn process_files(&self) -> Result<()> {
        let process_results = self
            .files
            .par_iter()
            .map(|path| {
                self.process_file(path)
                    .with_context(|| format!("Failed to process {}", path.display()))
            })
            .collect::<Vec<Result<_>>>();

        for result in process_results {
            if let Err(error) = result {
                eprintln!("{error}");
            }
        }
        Ok(())
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

    fn parse_source_code<'path>(
        &self,
        source_code: &str,
        path: &'path Path,
    ) -> Result<Vec<FunctionInfo<'path>>> {
        let mut parser = TParser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .with_context(|| "Error loading C++ grammar")?;

        let tree = parser
            .parse(source_code, None)
            .ok_or_else(|| anyhow!("Failed to parse source code file"))?;

        let root_node = tree.root_node();

        let mut cursor = QueryCursor::new();

        let mut query_matches = cursor.matches(&self.query, root_node, source_code.as_bytes());

        let mut return_vec = vec![];
        while let Some(q_match) = query_matches.next() {
            let return_type_node = q_match
                .nodes_for_capture_index(self.capture_indexes.return_type)
                .next()
                .ok_or_else(|| {
                    anyhow!(
                        "Failed to find return type node in treesitter parse tree in {}",
                        path.display()
                    )
                })?;

            if return_type_node.kind() == "placeholder_type_specifier" {
                continue;
            }

            let mut declarator_node = q_match
                .nodes_for_capture_index(self.capture_indexes.declarator)
                .next()
                .ok_or_else(|| {
                    anyhow!(
                        "Failed to find declarator node in treesitter parse tree in {}",
                        path.display()
                    )
                })?;

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

            // check for virtual specifier and/or requires_clause
            let postfix_start = {
                let mut cursor = declarator_node.walk();
                declarator_node
                    .named_children(&mut cursor)
                    .filter(|child| matches!(child.kind(), "virtual_specifier" | "requires_clause"))
                    .map(|child| child.start_byte())
                    .min()
            };

            // if there space between return_type and declarator, decrement return_type's end byte by 1
            // int&no_space()
            // int& no_space()
            // former one is also a valid syntax (weird)
            let has_space_between_return_type_and_declarator =
                source_code.as_bytes()[declarator_node.start_byte() - 1].is_ascii_whitespace();
            let return_type_end_byte = if has_space_between_return_type_and_declarator {
                declarator_node.start_byte() - 1
            } else {
                declarator_node.start_byte()
            };

            // if qualifier exists (const, volatile) add qualifier to return type so it will be
            // moved to the trailing return section
            let return_type_range = q_match
                .nodes_for_capture_index(self.capture_indexes.qualifier)
                .find(|node| {
                    node.utf8_text(source_code.as_bytes())
                        .is_ok_and(|text| matches!(text, "const" | "volatile")) // skip contsexpr and consteval
                })
                .map_or_else(
                    || core::ops::Range::<usize> {
                        start: return_type_node.start_byte(),
                        end: return_type_end_byte,
                    },
                    |qualifier_node| core::ops::Range::<usize> {
                        start: qualifier_node.start_byte(),
                        end: return_type_end_byte,
                    },
                );

            let declarator_range = core::ops::Range::<usize> {
                start: declarator_node.start_byte(),
                end: postfix_start.map_or_else(|| declarator_node.end_byte(), |start| start - 1),
            };

            let line = return_type_node.start_position().row + 1;

            // if there is no space between, it should be manually implemented, if not it will be
            // something like this after autofix `autono_space()` which is not a valid syntax unlike
            // `int&no_space()`
            let auto_replacement = if has_space_between_return_type_and_declarator {
                "auto"
            } else {
                "auto "
            };

            return_vec.push(FunctionInfo {
                return_type_range,
                declarator_range,
                line,
                path,
                auto_replacement,
            });
        }

        Ok(return_vec)
    }

    fn process_file(&self, path: &Path) -> Result<()> {
        let mut source_code = std::fs::read_to_string(path)?;

        if self.config.fix {
            self.fix_source(&mut source_code, path)?;
            std::fs::write(path, source_code)?;
        } else {
            let functions = self.filtered_functions(&source_code, path)?;
            let mut output = String::new();
            for info in functions {
                let _ = writeln!(output, "{}", info.display(&source_code));
            }

            if !output.is_empty() {
                print!("{output}");
            }
        }
        Ok(())
    }

    fn fix_source(&self, source_code: &mut String, path: &Path) -> Result<()> {
        let mut functions: Vec<_> = self.filtered_functions(source_code, path)?.collect();
        functions.sort_unstable_by_key(|info| std::cmp::Reverse(info.return_type_range.start));

        for info in functions {
            let trailing_return = format!(" -> {}", info.return_type(source_code));
            source_code.insert_str(info.declarator_range.end, &trailing_return);
            source_code.replace_range(info.return_type_range.clone(), info.auto_replacement);
        }

        Ok(())
    }

    fn filtered_functions<'path>(
        &self,
        source_code: &str,
        path: &'path Path,
    ) -> Result<impl Iterator<Item = FunctionInfo<'path>>> {
        let parsed = self.parse_source_code(source_code, path)?;
        let threshold = self.config.threshold;
        Ok(parsed
            .into_iter()
            .filter(move |info| info.return_type(source_code).chars().count() >= threshold))
    }
}
