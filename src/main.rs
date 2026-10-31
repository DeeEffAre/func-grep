use std::path::{Path, PathBuf};

use clap::Parser;
use ignore::WalkBuilder;
use rayon::prelude::*;
use tree_sitter::{Parser as TParser, Query, QueryCursor, StreamingIterator};

#[derive(Parser)]
#[command()]
pub struct Cli {
    pub path: Option<String>,
}

fn main() {
    let mut cli = Cli::parse();
    if cli.path.is_none() {
        cli.path = Some(String::from("./"));
    }

    let binding = cli.path.unwrap();

    let files = find_filepaths(&binding);
    files.par_iter().for_each(|path| {
        process_file(path);
    });
}

fn find_filepaths(filepath: &str) -> Vec<PathBuf> {
    let path = Path::new(filepath);
    WalkBuilder::new(path)
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            if let Some(ext) = entry.path().extension() {
                ext == "cpp" || ext == "hpp"
            } else {
                false
            }
        })
        .map(ignore::DirEntry::into_path)
        .collect()
}

struct FunctionInfo<'path> {
    return_type: String,
    declarator: String,
    line: usize,
    path: &'path Path,
}

impl std::fmt::Display for FunctionInfo<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}: {} {}",
            self.path.to_str().unwrap(),
            self.line,
            self.return_type,
            self.declarator
        )
    }
}

fn parse_source_code<'path>(source_code: &str, path: &'path Path) -> Vec<FunctionInfo<'path>> {
    let mut parser = TParser::new();
    parser
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
        .expect("Error loading C++ grammar");

    let tree = parser
        .parse(source_code, None)
        .expect("Failed to parse source code file");

    let root_node = tree.root_node();

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
    let mut cursor = QueryCursor::new();

    let mut query_matches = cursor.matches(&query, root_node, source_code.as_bytes());

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
            return_type: return_type_text.to_string(),
            declarator: function_declarator_text.to_string(),
            line,
            path,
        });
    }

    return_vec
}

fn process_file(path: &Path) {
    let source_code = std::fs::read_to_string(path).expect("Failed to read source code");
    let parsed = parse_source_code(&source_code, path);

    parsed
        .iter()
        .filter(|info| info.return_type.len() >= 20)
        .for_each(|info| {
            println!("{info}");
        });
}
