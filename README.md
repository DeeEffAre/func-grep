# What is that project?

func-grep is a long return type capture cli application for C++ projects.

Works perfectly for both source and header files.

The motivation for this project was to use trailing return types in function signatures for readability

```cpp
    [[nodiscard]]
    std::expected<std::vector<std::byte>*, ErrorCode> get(std::span<const std::byte> key) const noexcept;
    // here is the return type is 49 characters and breaks readability of the member function

    // we can use type aliases
    [[nodiscard]]
    std::expected<ByteVec*, ErrorCode> get(ByteSpan key) const noexcept;
    // still 34 characters in return type and pretty long for my liking

    // finally we can use to trailing return types with type aliases
    [[nodiscard]]
    auto get(ByteSpan key) const noexcept -> std::expected<ByteVec*, ErrorCode>;
    // both function name and return type is easily selectable now
```

# Usage

```bash
cargo build --release
./target/release/func_grep <path> [-t <threshold>]
```

Default value for path argument is `./`

Example with `demo/`. Default threshold:

```bash
❯ ./target/release/func_grep demo
demo/test.hpp:8: std::expected<ByteVec*, ErrorCode> get(ByteSpan key) const noexcept
```

Threshold set to 1

```bash
❯ ./target/release/func_grep demo -t1
demo/test.hpp:5: void put(RecordType record_type, ByteVec key, ByteVec value) noexcept
demo/test.hpp:8: std::expected<ByteVec*, ErrorCode> get(ByteSpan key) const noexcept
demo/test.hpp:14: void restore(Memtable&& table_to_load) noexcept
demo/test.hpp:16: void clear_memtable() noexcept
demo/test.cpp:8: void Engine::put(RecordType record_type, ByteVec key, ByteVec value) noexcept
demo/test.cpp:24: void Engine::restore(Memtable&& table_to_load) noexcept
demo/test.cpp:28: void Engine::clear_memtable() noexcept
demo/test.cpp:32: void clear()
demo/test.cpp:34: int& asd()
demo/test.cpp:35: char* asd1()
demo/test.cpp:36: char** asd2()
```

It also detects reference and nested pointer return types

There are no benchmarks yet; I don't think they're necessary for a tool this small. A few performance notes:

- Processing every file (`process_file`) is done in parallel with [rayon](https://crates.io/crates/rayon)
- Store `return_type` and `declarator` values in `&str` with annotated lifetimes in `FunctionInfo` struct to avoid unnecessary heap allocation on data which will only be used for read-only purposes
