# Architecture

The transpiler has three stages:

1. **Parse and lower:** `syn` parses Rust source. The visitor in `src/lower.rs`
   converts its AST into the owned, `syn`-independent types in `src/ir.rs`.
2. **Generate C++:** `src/cpp/` consumes the IR and produces a header/source
   pair. Its modules handle types, patterns, expressions, formatting, and
   symbols.
3. **Walk project files:** `src/walk.rs` finds Rust files, configures module
   resolution and output paths, writes the generated files, and optionally
   creates CMake configuration.

The library entry point `transpile_str` exposes the parse/lower/generate path
for one source string. The command-line tool uses the directory walker for
project-level conversion. The generator's `Resolver` interface separates
header generation from file-system module lookup.

## Source layout

- `src/lib.rs` — public API and embedded runtime header
- `src/ir.rs` — intermediate representation
- `src/lower.rs` — Rust AST to IR lowering
- `src/cpp/` — C++ output generation
- `src/walk.rs` — directory traversal and module include resolution
- `src/main.rs` — command-line interface
- `runtime/rust_runtime.hpp` — support types and helpers used by generated code
- `tests/fixtures.rs` and `tests/fixtures/` — per-feature output regression tests

The IR deliberately isolates the generator from `syn`. Constructs not modeled
as a C++ equivalent can be retained as unsupported markers rather than silently
discarded. See [features and limitations](features.md) before using generated
output.
