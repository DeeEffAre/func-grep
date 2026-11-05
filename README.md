# func_grep

A small CLI I wrote for my own C++ projects. It finds function signatures whose
return type is longer than a threshold I choose, and can rewrite those signatures
to use trailing return types.

I don't want every function to use a trailing return type. Short types read fine
inline; long template types push the function name out of view. `clang-tidy`'s
`modernize-use-trailing-return-type` converts everything. This tool only touches return types above the threshold,
and it works directly on source files.

```cpp
[[nodiscard]]
std::expected<std::vector<std::byte>*, ErrorCode> get(std::span<const std::byte> key) const noexcept;
// 49 characters before the function name

[[nodiscard]]
auto get(std::span<const std::byte> key) const noexcept -> std::expected<std::vector<std::byte>*, ErrorCode>;
// same signature, easier to scan
```

## Usage

```bash
cargo build --release
./target/release/func_grep <path> [-t <threshold>] [--fix]
```

`path` defaults to `./`. `threshold` is the character length of the return type
and defaults to `20`. `--fix` rewrites files in place; there is no dry-run.

Scan the demo directory:

```bash
$ ./target/release/func_grep demo
demo/test.hpp:8: std::expected<ByteVec*, ErrorCode> get(ByteSpan key) const noexcept
```

Lower the threshold to see more:

```bash
$ ./target/release/func_grep demo -t 1
demo/test.hpp:5: void put(RecordType record_type, ByteVec key, ByteVec value) noexcept
demo/test.hpp:8: std::expected<ByteVec*, ErrorCode> get(ByteSpan key) const noexcept
...
demo/test.cpp:36: char** asd2()
```

## How it works

- tree-sitter-cpp parses each file; no compiler or build setup is required.
- It looks at function definitions and declarations, including member functions.
- Pointer/reference declarators are unwrapped, and `const`/`volatile` stay with
  the return type when it moves.
- Files are processed in parallel with rayon.
- `--fix` inserts the trailing return before `override`, `final`, or `requires`
  when they are present.

## Scope and limitations

This is a personal tool, not a clang-tidy replacement. It does not do semantic
analysis, so macros, unusual templates, or very creative formatting can confuse
it. Current known gaps:

- declarations with multiple declarators (`T a(), b();`) are not handled;
- spacing around `override`/`requires` is assumed; run clang-format first;
- only `.cpp`, `.hpp`, `.cc`, `.hh`, `.cxx`, `.hxx`, and `.h` files are scanned;
- comments between the return type and function name can move with the return
  type;
- `--fix` writes directly to disk, so use version control.

If you want every function converted, use clang-tidy's
`modernize-use-trailing-return-type`. If you only care about long return types,
this should do the job.
