use anyhow::{Result, bail};
use rust_to_cpp::{CppStd, walk};
use std::path::PathBuf;

const USAGE: &str = "rust-to-cpp - convert Rust sources to C++20/C++23

USAGE:
    rust-to-cpp <ROOT> [--std 20|23] [--out <DIR>] [--no-cmake]

Every `.rs` file below ROOT produces an equivalent `.h` and `.cpp`
(next to the source, or mirrored below --out).";

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut root = None;
    let mut out_dir = None;
    let mut std_ = CppStd::Cpp20;
    let mut cmake = true;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--std" => {
                std_ = match args.next().as_deref() {
                    Some("20") | Some("c++20") => CppStd::Cpp20,
                    Some("23") | Some("c++23") => CppStd::Cpp23,
                    other => bail!("unsupported --std {:?} (use 20 or 23)", other),
                }
            }
            "--out" => out_dir = args.next().map(PathBuf::from),
            "--no-cmake" => cmake = false,
            s if s.starts_with('-') => bail!("unknown option {s}\n\n{USAGE}"),
            s => root = Some(PathBuf::from(s)),
        }
    }
    let Some(root) = root else { bail!("missing ROOT folder\n\n{USAGE}") };
    let report = walk::convert_tree(&walk::Config { root, out_dir, std: std_, cmake })?;
    for f in &report.converted {
        println!("converted {}", f.display());
    }
    for (f, e) in &report.failed {
        eprintln!("FAILED    {}: {e}", f.display());
    }
    if !report.dependencies.is_empty() {
        println!("dependencies needing a C++ counterpart: {}", report.dependencies.join(", "));
    }
    println!("{} file(s) converted, {} failed", report.converted.len(), report.failed.len());
    if !report.failed.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}
