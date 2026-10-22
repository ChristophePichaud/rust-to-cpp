# rust-to-cpp

`rust-to-cpp` is an experimental source-to-source transpiler that converts
Rust files into C++20 or C++23 headers and source files. It is intended as a
starting point for translating Rust projects, not as a complete or
semantics-preserving Rust compiler.

## Quick start

```sh
cargo run -- path/to/rust-project
```

The command writes a `.h` and `.cpp` file for each `.rs` file, along with the
`rust_runtime.hpp` support header and (by default) a `CMakeLists.txt`. To choose
the C++ standard or a separate output directory:

```sh
cargo run -- path/to/rust-project --std 23 --out generated
```

Use `--no-cmake` to skip the CMake file. Run `cargo run -- --help` for the full
command syntax.

## Documentation

- [Usage and generated output](docs/usage.md)
- [Supported features and limitations](docs/features.md)
- [Architecture](docs/architecture.md)
- [Tests and validation](docs/testing.md)

## Development

Run the fixture regression tests and formatting check with:

```sh
cargo test
cargo fmt --check
```
