#[cfg(test)]
mod test {
    use std::fs;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};
    use tree_sitter::Query;

    use crate::{App, CaptureIndexes, Cli, QUERY_STR};

    const DEFAULT_THRESHOLD: usize = 20;

    fn query() -> Result<Query> {
        Query::new(&tree_sitter_cpp::LANGUAGE.into(), QUERY_STR)
            .context("failed to build tree-sitter query")
    }

    fn app(threshold: usize) -> Result<App> {
        let files = Vec::new();
        let query = query()?;
        let config = Cli {
            path: String::new(),
            threshold,
            fix: false,
        };
        let capture_indexes = CaptureIndexes::new(&query)?;
        Ok(App {
            files,
            query,
            config,
            capture_indexes,
        })
    }

    fn fixed(source: &str, threshold: usize) -> Result<String> {
        let mut source_owned = source.to_owned();
        app(threshold)?.fix_source(&mut source_owned, Path::new("test.cpp"))?;

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

        let cli = Cli {
            path: path.display().to_string(),
            threshold: 1,
            fix: true,
        };
        let app = App::new(cli).context("failed to create app")?;
        app.process_file(&path)?;

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
        let app = app(DEFAULT_THRESHOLD)?;

        let parsed = app.parse_source_code(source, &path)?;

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].return_type(source), "std::vector<std::string>");
        assert_eq!(parsed[0].declarator(source), "free_proto()");
        Ok(())
    }

    #[test]
    fn parses_function_with_multiple_leading_qualifiers_once() -> Result<()> {
        let source = "const volatile int& cv() { static int x; return x; }\n";
        let path = PathBuf::from("test.cpp");
        let app = app(DEFAULT_THRESHOLD)?;

        let parsed = app.parse_source_code(source, &path)?;

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

    #[test]
    fn auto_fix_inserts_trailing_return_before_override_without_space() -> Result<()> {
        let source = "struct B { virtual std::vector<std::string> f(); };\nstruct D : B { std::vector<std::string> f()override; };\n";
        let expected = "struct B { virtual auto f() -> std::vector<std::string>; };\nstruct D : B { auto f() -> std::vector<std::string> override; };\n";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_inserts_trailing_return_before_requires_without_space() -> Result<()> {
        let source = r"struct Foo {
    template <typename T>
    std::vector<std::string> req(T t)requires std::integral<T>;
};
";
        let expected = r"struct Foo {
    template <typename T>
    auto req(T t) -> std::vector<std::string> requires std::integral<T>;
};
";

        assert_eq!(fixed(source, 1)?, expected);
        Ok(())
    }

    #[test]
    fn auto_fix_skips_multi_declarator_declarations() -> Result<()> {
        let source = "struct Foo {\n    std::vector<std::string> a(), b();\n};\n";

        assert_eq!(fixed(source, 1)?, source);
        Ok(())
    }

    #[test]
    fn auto_fix_skips_placeholder_auto_reference_return_types() -> Result<()> {
        let source = "auto& auto_ref() { static int x; return x; }\nauto&& auto_rref() { static int x; return static_cast<int&&>(x); }\nconst auto* const_auto_ptr() { return static_cast<const int*>(nullptr); }\n";

        assert_eq!(fixed(source, 1)?, source);
        Ok(())
    }

    #[test]
    fn app_new_rejects_nonexistent_path() {
        let path =
            std::env::temp_dir().join(format!("func_grep_missing_path_{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);

        let result = App::new(Cli {
            path: path.to_str().unwrap().to_string(),
            threshold: DEFAULT_THRESHOLD,
            fix: false,
        });

        assert!(result.is_err());
    }

    #[test]
    fn process_files_returns_error_when_a_file_cannot_be_read() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("func_grep_bad_file_{}", std::process::id()));
        fs::create_dir_all(&dir).context("failed to create test directory")?;
        fs::write(dir.join("invalid_utf8.cpp"), [0xFF, 0xFE, 0xFD])
            .context("failed to write invalid utf-8 test file")?;

        let cli = Cli {
            path: dir.display().to_string(),
            threshold: DEFAULT_THRESHOLD,
            fix: false,
        };
        let app = App::new(cli).context("failed to create app")?;

        let result = app.process_files();

        let _ = fs::remove_dir_all(&dir);

        assert!(result.is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn fix_does_not_write_unchanged_file() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "func_grep_unchanged_file_{}.cpp",
            std::process::id()
        ));
        fs::write(&path, "int foo() { return 1; }\n")
            .context("failed to write unchanged test file")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444))
            .context("failed to make test file read-only")?;

        let cli = Cli {
            path: path.display().to_string(),
            threshold: DEFAULT_THRESHOLD,
            fix: true,
        };
        let app = App::new(cli).context("failed to create app")?;
        let result = app.process_file(&path);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
        let _ = fs::remove_file(&path);

        result?;
        Ok(())
    }
}
