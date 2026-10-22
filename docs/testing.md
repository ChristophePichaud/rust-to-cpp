# Tests and validation

Run the Rust test suite and formatting check from the repository root:

```sh
cargo test
cargo fmt --check
```

`tests/fixtures.rs` compares each Rust input in `tests/fixtures/` with its
checked-in generated `.h` and `.cpp` files. Each fixture focuses on a feature
area, so a code-generation change shows which outputs were affected. When
intentionally changing output, review and update the corresponding golden files
alongside the Rust fixture.

For additional static checks, run:

```sh
cargo clippy --all-targets
cargo build
```

The fixture tests validate stable generated text; they do not compile the C++
output. For end-to-end validation, transpile a representative project and
compile the result using its intended C++20/C++23 compiler, standard library,
runtime header, and any required C++ dependencies. C++ compiler support for
standard-library features can vary between toolchains.

With a C++20 compiler such as GCC, the checked-in fixture outputs can also be
syntax-checked from the repository root:

```sh
g++ -std=c++20 -fsyntax-only -Iruntime tests/fixtures/*.cpp
```
