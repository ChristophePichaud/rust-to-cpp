//! Pattern lowering: a Rust pattern becomes a C++ condition plus a list of
//! variable bindings.

use super::sym::{EnumInfo, VKind};
use super::{Emitter, id};
use crate::ir::*;

pub struct Bind {
    pub name: String,
    pub expr: String,
    pub by_value: bool,
}

pub struct VariantRef {
    pub enum_name: String,
    pub variant: String,
    pub index: usize,
    pub info: EnumInfo,
    pub kind: VKind,
}

impl Emitter<'_> {
    pub fn resolve_variant(&self, path: &[String]) -> Option<VariantRef> {
        if path.len() < 2 {
            return None;
        }
        let mut en = path[path.len() - 2].clone();
        if en == "Self" {
            en = self.self_name.clone()?;
        }
        let variant = path.last()?.clone();
        let info = self.sym.enums.get(&en)?;
        let index = info.variants.iter().position(|(n, _)| *n == variant)?;
        Some(VariantRef { enum_name: en, variant, index, kind: info.variants[index].1.clone(), info: info.clone() })
    }

    /// Condition testing that `scrut` currently holds `v`, and the expression
    /// that yields the variant payload.
    fn variant_test(&self, v: &VariantRef, scrut: &str) -> (Option<String>, String) {
        if !v.info.data {
            return (Some(format!("{scrut} == {}::{}", id(&v.enum_name), id(&v.variant))), scrut.to_string());
        }
        if v.info.generic {
            (Some(format!("{scrut}.index() == {}", v.index)), format!("std::get<{}>({scrut})", v.index))
        } else {
            let vs = format!("{}_{}", id(&v.enum_name), id(&v.variant));
            (Some(format!("std::holds_alternative<{vs}>({scrut})")), format!("std::get<{vs}>({scrut})"))
        }
    }

    pub fn pat_cond(&mut self, p: &Pat, scrut: &str) -> (String, Vec<Bind>) {
        let mut conds = Vec::new();
        let mut binds = Vec::new();
        self.pat(p, scrut, &mut conds, &mut binds);
        let c = if conds.is_empty() { "true".to_string() } else { conds.join(" && ") };
        (c, binds)
    }

    pub fn emit_binds(&mut self, binds: &[Bind]) {
        for b in binds {
            if b.name == "_" {
                continue;
            }
            let kw = if b.by_value { "auto" } else { "auto&" };
            self.push(format!("{kw} {} = {};", id(&b.name), b.expr));
        }
    }

    fn elem_indices(elems: &[Pat], total: Option<usize>) -> Vec<Option<usize>> {
        let rest = elems.iter().position(|e| matches!(e, Pat::Rest));
        let n = elems.len();
        (0..n)
            .map(|i| match rest {
                Some(r) if i == r => None,
                Some(r) if i > r => total.map(|t| t - (n - i)),
                _ => Some(i),
            })
            .collect()
    }

    pub fn pat(&mut self, p: &Pat, scrut: &str, conds: &mut Vec<String>, binds: &mut Vec<Bind>) {
        match p {
            Pat::Wild | Pat::Rest => {}
            Pat::Ident { name, mutable, sub, .. } => {
                if name == "None" {
                    conds.push(format!("!{scrut}.has_value()"));
                } else if name.len() > 1 && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
                    conds.push(format!("{scrut} == {}", id(name)));
                } else {
                    binds.push(Bind { name: name.clone(), expr: scrut.to_string(), by_value: *mutable });
                    if let Some(s) = sub {
                        self.pat(s, scrut, conds, binds);
                    }
                }
            }
            Pat::Lit(e) => {
                let e = self.expr(e);
                conds.push(format!("{scrut} == {e}"));
            }
            Pat::Tuple(elems) => {
                let rest = elems.iter().position(|e| matches!(e, Pat::Rest));
                let size = format!("std::tuple_size_v<std::remove_cvref_t<decltype({scrut})>>");
                for (i, e) in elems.iter().enumerate() {
                    let sub = match rest {
                        Some(r) if i == r => continue,
                        Some(r) if i > r => format!("std::get<{size} - {}>({scrut})", elems.len() - i),
                        _ => format!("std::get<{i}>({scrut})"),
                    };
                    self.pat(e, &sub, conds, binds);
                }
            }
            Pat::TupleStruct { path, elems } => {
                let last = path.last().map(String::as_str).unwrap_or("");
                if path.len() == 1 && matches!(last, "Some" | "Ok" | "Err") {
                    let (c, inner) = match last {
                        "Some" | "Ok" => (format!("{scrut}.has_value()"), format!("(*{scrut})")),
                        _ => (format!("!{scrut}.has_value()"), format!("{scrut}.error()")),
                    };
                    conds.push(c);
                    if let Some(e) = elems.first() {
                        self.pat(e, &inner, conds, binds);
                    }
                    return;
                }
                if let Some(v) = self.resolve_variant(path) {
                    let (c, payload) = self.variant_test(&v, scrut);
                    conds.extend(c);
                    let total = match v.kind {
                        VKind::Tuple(n) => Some(n),
                        _ => None,
                    };
                    for (e, idx) in elems.iter().zip(Self::elem_indices(elems, total)) {
                        if let Some(i) = idx {
                            self.pat(e, &format!("{payload}._{i}"), conds, binds);
                        }
                    }
                } else {
                    let total = self
                        .sym
                        .structs
                        .get(last)
                        .map(|s| s.fields.len());
                    for (e, idx) in elems.iter().zip(Self::elem_indices(elems, total)) {
                        if let Some(i) = idx {
                            self.pat(e, &format!("{scrut}._{i}"), conds, binds);
                        }
                    }
                }
            }
            Pat::Struct { path, fields } => {
                let payload = if let Some(v) = self.resolve_variant(path) {
                    let (c, payload) = self.variant_test(&v, scrut);
                    conds.extend(c);
                    payload
                } else {
                    scrut.to_string()
                };
                for (name, fp) in fields {
                    self.pat(fp, &format!("{payload}.{}", id(name)), conds, binds);
                }
            }
            Pat::Path(path) => {
                if let Some(v) = self.resolve_variant(path) {
                    let (c, _) = self.variant_test(&v, scrut);
                    conds.extend(c);
                } else if path.last().map(String::as_str) == Some("None") {
                    conds.push(format!("!{scrut}.has_value()"));
                } else {
                    let e = self.path_names(path);
                    conds.push(format!("{scrut} == {e}"));
                }
            }
            Pat::Ref(inner) => self.pat(inner, scrut, conds, binds),
            Pat::Or(alts) => {
                let mut parts = Vec::new();
                for (i, a) in alts.iter().enumerate() {
                    let mut c = Vec::new();
                    let mut b = Vec::new();
                    self.pat(a, scrut, &mut c, &mut b);
                    if i == 0 {
                        binds.extend(b);
                    }
                    parts.push(if c.is_empty() { "true".to_string() } else { c.join(" && ") });
                }
                conds.push(format!("({})", parts.join(" || ")));
            }
            Pat::Range { lo, hi, inclusive } => {
                let mut parts = Vec::new();
                if let Some(l) = lo {
                    parts.push(format!("{scrut} >= {}", self.expr(l)));
                }
                if let Some(h) = hi {
                    let op = if *inclusive { "<=" } else { "<" };
                    parts.push(format!("{scrut} {op} {}", self.expr(h)));
                }
                conds.push(format!("({})", parts.join(" && ")));
            }
            Pat::Slice(elems) => {
                let rest = elems.iter().position(|e| matches!(e, Pat::Rest));
                let n = elems.len() - rest.is_some() as usize;
                conds.push(if rest.is_some() { format!("{scrut}.size() >= {n}") } else { format!("{scrut}.size() == {n}") });
                for (i, e) in elems.iter().enumerate() {
                    match rest {
                        Some(r) if i == r => {}
                        Some(r) if i > r => {
                            self.pat(e, &format!("{scrut}[{scrut}.size() - {}]", elems.len() - i), conds, binds)
                        }
                        _ => self.pat(e, &format!("{scrut}[{i}]"), conds, binds),
                    }
                }
            }
        }
    }
}
