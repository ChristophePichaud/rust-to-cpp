# Usage

## Command line

```text
rust-to-cpp <ROOT> [--std 20|23] [--out <DIR>] [--no-cmake]
```

`ROOT` is a Rust project or source directory. Every `.rs` file below it is
translated independently. The default standard is C++20. `--out` writes the
generated files under a separate directory while preserving their paths;
without it, generated files are written next to the Rust files. The directory
walker skips hidden directories and `target`. Unless `--no-cmake` is supplied,
a `CMakeLists.txt` is created in the output root only when one does not already
exist.

Each translated Rust file produces a `.h` and `.cpp` file. The runtime support
header `rust_runtime.hpp` is also copied to the output root. The generated
headers include it using a path relative to their location. C++ compilation
requires a compiler and standard library with the requested standard and
features used by the generated output, including `<format>`.

## Library API

Call `rust_to_cpp::transpile_str(source, stem, &Options::default())` to
translate source text directly. The returned `Output` contains the generated
header and source strings. This API does not resolve file-system modules;
use the command-line directory driver when translating a project tree.

## Example

```rust
pub fn double(value: i32) -> i32 {
    value * 2
}
```

The input generates `example.h` with the declaration and `example.cpp` with
the function definition. Generated code is intended to be inspected and
validated in the target project; the tool does not compile or execute it.
