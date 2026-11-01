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
        help = "character length the return type"
    )]
    threshold: usize,
}

fn main() {
    let cli = Cli::parse();
    let files = find_filepaths(&cli.path);

    let query_str = r"
    [
        (function_definition
            type: (_) @return_type
            declarator: (_) @declarator
        )
        (field_declaration
            type: (_) @return_type
            declarator: (_) @declarator
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
            self.path.display().to_string().green(),
            self.line.to_string().bright_blue(),
            self.return_type.red().bold(),
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

        let return_type_text =
            source_code[return_type_node.start_byte()..declarator_node.start_byte()].trim_end();
        let declarator_text = &source_code[declarator_node.byte_range()];

        let line = return_type_node.start_position().row + 1;

        return_vec.push(FunctionInfo {
            return_type: return_type_text,
            declarator: declarator_text,
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
