#[cfg(test)]
mod test {
    use std::fs;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};
    use tree_sitter::Query;

    use crate::{App, Cli, QUERY_STR};

    const DEFAULT_THRESHOLD: usize = 20;

    fn query() -> Result<Query> {
        Query::new(&tree_sitter_cpp::LANGUAGE.into(), QUERY_STR)
            .context("failed to build tree-sitter query")
    }

    fn fixed(source: &str, threshold: usize) -> Result<String> {
        let mut source_owned = source.to_owned();
        App::fix_source(
            &mut source_owned,
            Path::new("test.cpp"),
            &query()?,
            threshold,
        )?;

        Ok(source_owned)
    }

    #[test]
    fn test_auto_fix_from_demo() -> Result<()> {
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

        assert_eq!(fixed(source, DEFAULT_THRESHOLD)?, EXPECTED_SOURCE_CODE);
        Ok(())
    }

    #[test]
    fn auto_fix_is_persisted_to_disk() -> Result<()> {
        let path = std::env::temp_dir().join(format!(
            "func_grep_autofix_persist_{}.cpp",
            std::process::id()
        ));
        let source = "std::vector<std::string> make_names() { return {}; }\n";
        fs::write(&path, source).context("failed to write test source file")?;

        let query = query()?;
        let cli = Cli {
            path: path.display().to_string(),
            threshold: 1,
            fix: true,
        };
        App::process_file(&path, &cli, &query)?;

        let fixed_source = fs::read_to_string(&path).context("failed to read fixed source file")?;
        assert_eq!(
            fixed_source,
            "auto make_names() -> std::vector<std::string> { return {}; }\n"
        );

        let _ = fs::remove_file(&path);
        Ok(())
    }

    #[test]
    fn auto_fix_inserts_trailing_return_before_override() -> Result<()> {
        let source = "struct Foo {\n    std::vector<std::string> over() const override;\n};\n";
        let expected =
            "struct Foo {\n    auto over() const -> std::vector<std::string> override;\n};\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_inserts_trailing_return_before_final() -> Result<()> {
        let source = "struct Foo {\n    virtual std::vector<std::string> fin() const final;\n};\n";
        let expected =
            "struct Foo {\n    virtual auto fin() const -> std::vector<std::string> final;\n};\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_inserts_trailing_return_before_requires() -> Result<()> {
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

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_moves_leading_const_into_trailing_return() -> Result<()> {
        let source = "const std::string& get_name() noexcept { static std::string s; return s; }\n";
        let expected =
            "auto get_name() noexcept -> const std::string& { static std::string s; return s; }\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_moves_leading_volatile_into_trailing_return() -> Result<()> {
        let source = "volatile int& get_ref() { static int x; return x; }\n";
        let expected = "auto get_ref() -> volatile int& { static int x; return x; }\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_handles_return_type_without_space_before_declarator() -> Result<()> {
        let source = "int&no_space() { static int x; return x; }\n";
        let expected = "auto no_space() -> int& { static int x; return x; }\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_skips_placeholder_auto_pointer_return_type() -> Result<()> {
        let source = "auto* auto_ptr() { return static_cast<int*>(nullptr); }\n";

        assert_eq!(fixed(source, 1)?, source);
        Ok(())
    }

    #[test]
    fn parses_namespace_scope_function_declaration() -> Result<()> {
        let source = "std::vector<std::string> free_proto();\n";
        let path = PathBuf::from("test.hpp");
        let query = query()?;

        let parsed = App::parse_source_code(source, &path, &query)?;

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].return_type(source), "std::vector<std::string>");
        assert_eq!(parsed[0].declarator(source), "free_proto()");
        Ok(())
    }

    #[test]
    fn parses_function_with_multiple_leading_qualifiers_once() -> Result<()> {
        let source = "const volatile int& cv() { static int x; return x; }\n";
        let path = PathBuf::from("test.cpp");
        let query = query()?;

        let parsed = App::parse_source_code(source, &path, &query)?;

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].return_type(source), "const volatile int&");
        Ok(())
    }

    #[test]
    fn auto_fix_moves_multiple_leading_qualifiers_into_trailing_return() -> Result<()> {
        let source = "const volatile int& cv() { static int x; return x; }\n";
        let expected = "auto cv() -> const volatile int& { static int x; return x; }\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_keeps_constexpr_as_function_specifier() -> Result<()> {
        let source = "constexpr std::string& cx() { static std::string s; return s; }\n";
        let expected = "constexpr auto cx() -> std::string& { static std::string s; return s; }\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_keeps_consteval_as_function_specifier() -> Result<()> {
        let source = "consteval std::string& ce() { static std::string s; return s; }\n";
        let expected = "consteval auto ce() -> std::string& { static std::string s; return s; }\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_moves_const_qualifier_for_member_function_declaration() -> Result<()> {
        let source = "struct Foo {\n    const std::string& name() const noexcept;\n};\n";
        let expected = "struct Foo {\n    auto name() const noexcept -> const std::string&;\n};\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_preserves_tab_separator() -> Result<()> {
        let source = "int&\tget_ref() { static int x; return x; }\n";
        let expected = "auto\tget_ref() -> int& { static int x; return x; }\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }
}
