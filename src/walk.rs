//! The file system driver: walks a folder tree and transpiles every `.rs`
//! file into a `.h` / `.cpp` pair.

use crate::cpp::{self, CppStd, Options, Resolver};
use crate::{RUNTIME_HEADER, lower};
use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct Config {
    pub root: PathBuf,
    /// Where generated files go (mirrors the source tree). Defaults to `root`.
    pub out_dir: Option<PathBuf>,
    pub std: CppStd,
    /// Write a `CMakeLists.txt` (only if none exists in the output root).
    pub cmake: bool,
}

#[derive(Debug, Default)]
pub struct Report {
    pub converted: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub dependencies: Vec<String>,
}

fn is_skipped(entry: &walkdir::DirEntry, out: &Path) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    name == "target" || name.starts_with('.') || entry.path() == out
}

/// The directory holding `main.rs` / `lib.rs` (module root of the crate).
pub fn crate_root(root: &Path) -> PathBuf {
    for cand in [root.to_path_buf(), root.join("src")] {
        if cand.join("main.rs").exists() || cand.join("lib.rs").exists() {
            return cand;
        }
    }
    root.to_path_buf()
}

fn rel(from_dir: &Path, to: &Path) -> String {
    let f: Vec<_> = from_dir.components().collect();
    let t: Vec<_> = to.components().collect();
    let mut i = 0;
    while i < f.len() && i < t.len() && f[i] == t[i] {
        i += 1;
    }
    let mut parts: Vec<String> = vec!["..".into(); f.len() - i];
    parts.extend(t[i..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    parts.join("/")
}

struct FsResolver {
    crate_root: PathBuf,
    file: PathBuf,
}

impl FsResolver {
    /// Directory in which child modules of this file live.
    fn child_dir(&self) -> PathBuf {
        let dir = self.file.parent().unwrap().to_path_buf();
        match self.file.file_stem().and_then(|s| s.to_str()) {
            Some("main" | "lib" | "mod") => dir,
            Some(stem) => dir.join(stem),
            None => dir,
        }
    }

    fn module_file(base: &Path, name: &str) -> Option<PathBuf> {
        let a = base.join(format!("{name}.rs"));
        if a.exists() {
            return Some(a);
        }
        let b = base.join(name).join("mod.rs");
        b.exists().then_some(b)
    }
}

impl Resolver for FsResolver {
    fn mod_include(&self, name: &str) -> String {
        let dir = self.file.parent().unwrap();
        match Self::module_file(&self.child_dir(), name) {
            Some(f) => rel(dir, &f.with_extension("h")),
            None => format!("{name}.h"),
        }
    }

    fn use_include(&self, path: &[String]) -> Option<String> {
        if path.first().map(String::as_str) != Some("crate") {
            return None;
        }
        let dir = self.file.parent()?;
        // longest module prefix first: crate::a::b::Item -> a/b, then a
        for end in (2..path.len()).rev() {
            let mut base = self.crate_root.clone();
            for seg in &path[1..end - 1] {
                base.push(seg);
            }
            if let Some(f) = Self::module_file(&base, &path[end - 1]) {
                if f == self.file {
                    return None;
                }
                return Some(rel(dir, &f.with_extension("h")));
            }
        }
        None
    }
}

fn namespace_of(file: &Path, crate_root: &Path, root: &Path) -> Vec<String> {
    let base = if file.starts_with(crate_root) { crate_root } else { root };
    let r = file.strip_prefix(base).unwrap_or(file);
    let mut ns: Vec<String> = r
        .parent()
        .map(|p| p.components().filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        }).collect())
        .unwrap_or_default();
    let stem = r.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if !matches!(stem, "mod" | "lib" | "main") {
        ns.push(stem.to_string());
    }
    ns.iter().map(|s| s.replace('-', "_")).collect()
}

/// Reads the crate names listed in `[dependencies]` of a `Cargo.toml`.
pub fn read_dependencies(manifest: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(manifest) else { return vec![] };
    let mut in_deps = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_deps = l == "[dependencies]";
            continue;
        }
        if in_deps && !l.is_empty() && !l.starts_with('#') {
            if let Some((name, _)) = l.split_once('=') {
                out.push(name.trim().split('.').next().unwrap_or("").to_string());
            }
        }
    }
    out
}

/// Transpiles every `.rs` file below `cfg.root`.
pub fn convert_tree(cfg: &Config) -> Result<Report> {
    let root = cfg.root.canonicalize().with_context(|| format!("cannot open {}", cfg.root.display()))?;
    let out_root = match &cfg.out_dir {
        Some(o) => {
            std::fs::create_dir_all(o)?;
            o.canonicalize()?
        }
        None => root.clone(),
    };
    let croot = crate_root(&root);
    let mut report = Report::default();
    let mut sources = Vec::new();

    for entry in WalkDir::new(&root).into_iter().filter_entry(|e| !is_skipped(e, &out_root)) {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type().is_file() || path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let r = path.strip_prefix(&root)?.to_path_buf();
        match convert_file(path, &r, &root, &croot, &out_root, cfg.std) {
            Ok(()) => {
                report.converted.push(r.clone());
                sources.push(r.with_extension("cpp"));
            }
            Err(e) => report.failed.push((r, format!("{e:#}"))),
        }
    }

    std::fs::write(out_root.join("rust_runtime.hpp"), RUNTIME_HEADER)?;
    let manifest = [root.join("Cargo.toml"), croot.join("Cargo.toml"), croot.join("../Cargo.toml")]
        .into_iter()
        .find(|p| p.exists());
    if let Some(m) = manifest {
        report.dependencies = read_dependencies(&m);
    }
    if cfg.cmake && !out_root.join("CMakeLists.txt").exists() {
        let name = root.file_name().map(|n| n.to_string_lossy().replace('-', "_")).unwrap_or_else(|| "project".into());
        std::fs::write(out_root.join("CMakeLists.txt"), cmake_text(&name, cfg.std, &sources, &report.dependencies))?;
    }
    Ok(report)
}

fn convert_file(path: &Path, r: &Path, root: &Path, croot: &Path, out_root: &Path, std_: CppStd) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let ast = syn::parse_file(&text).map_err(|e| anyhow::anyhow!("syntax error: {e}"))?;
    let ir = lower::lower_file(&ast);
    let out_file = out_root.join(r);
    let depth = r.components().count() - 1;
    let opts = Options {
        std: std_,
        runtime_include: format!("{}rust_runtime.hpp", "../".repeat(depth)),
        namespace: namespace_of(path, croot, root),
        resolver: Box::new(FsResolver { crate_root: croot.to_path_buf(), file: path.to_path_buf() }),
    };
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    let out = cpp::generate(&ir, stem, &opts);
    if let Some(parent) = out_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out_file.with_extension("h"), out.header)?;
    std::fs::write(out_file.with_extension("cpp"), out.source)?;
    Ok(())
}

fn cmake_text(name: &str, std_: CppStd, sources: &[PathBuf], deps: &[String]) -> String {
    let mut t = String::new();
    t.push_str("# Generated by rust-to-cpp.\ncmake_minimum_required(VERSION 3.20)\n");
    t.push_str(&format!("project({name} CXX)\n\nset(CMAKE_CXX_STANDARD {})\nset(CMAKE_CXX_STANDARD_REQUIRED ON)\n\n", std_.number()));
    if !deps.is_empty() {
        t.push_str("# Rust dependencies (Cargo.toml) that need a C++ counterpart:\n");
        for d in deps {
            t.push_str(&format!("#   {d}\n"));
        }
        t.push('\n');
    }
    t.push_str(&format!("add_executable({name}\n"));
    let mut s: Vec<String> = sources.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
    s.sort();
    for p in s {
        t.push_str(&format!("    {p}\n"));
    }
    t.push_str(")\ntarget_include_directories(");
    t.push_str(&format!("{name} PRIVATE ${{CMAKE_CURRENT_SOURCE_DIR}})\n"));
    t
}
