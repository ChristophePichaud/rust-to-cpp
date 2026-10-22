//! rust-to-cpp: a source-to-source transpiler from Rust to C++20/C++23.
//!
//! Pipeline: Rust source -> `syn` AST -> [`lower`] (visitor) -> [`ir`] -> [`cpp`] generator.

pub mod cpp;
pub mod ir;
pub mod lower;
pub mod walk;

pub use cpp::{CppStd, Options, Output};

/// Runtime support header, written next to the generated files.
pub const RUNTIME_HEADER: &str = include_str!("../runtime/rust_runtime.hpp");

/// Transpiles one Rust source text (all three layers).
pub fn transpile_str(source: &str, stem: &str, opts: &Options) -> Result<Output, syn::Error> {
    let ast = syn::parse_file(source)?;
    let ir = lower::lower_file(&ast);
    Ok(cpp::generate(&ir, stem, opts))
}
