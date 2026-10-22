//! Statement and expression generation.

use super::fmt as rfmt;
use super::pat::Bind;
use super::types::is_primitive;
use super::{Emitter, id};
use crate::ir::*;

/// Where the value of a (block-like) expression has to go.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Discard,
    Return,
    Assign(String),
}

pub struct LoopCtx {
    pub label: Option<String>,
    pub target: Target,
}

/// Methods that only change how a collection is viewed in Rust; the C++
/// container is already usable as-is.
const PASSTHROUGH: &[&str] = &[
    "iter", "iter_mut", "into_iter", "cloned", "copied", "as_ref", "as_mut", "borrow", "borrow_mut", "as_slice",
    "as_mut_slice", "by_ref", "as_deref", "as_deref_mut", "into_boxed_slice", "into",
];

/// Methods implemented by `rust_runtime.hpp` as free functions
/// `rust::name(receiver, args...)`.
const RUNTIME_FNS: &[&str] = &[
    "map", "filter", "rev", "take", "skip", "enumerate", "zip", "sum", "product", "count", "fold", "any", "all",
    "for_each", "unwrap", "expect", "unwrap_or", "unwrap_or_else", "unwrap_or_default", "pop", "push_str",
    "contains", "starts_with", "ends_with", "to_uppercase", "to_lowercase", "trim", "join", "get", "insert",
    "extend", "sort_by", "first", "last", "chars",
];

const MATH: &[(&str, &str)] = &[
    ("abs", "std::abs"),
    ("sqrt", "std::sqrt"),
    ("floor", "std::floor"),
    ("ceil", "std::ceil"),
    ("round", "std::round"),
    ("sin", "std::sin"),
    ("cos", "std::cos"),
    ("tan", "std::tan"),
    ("exp", "std::exp"),
    ("ln", "std::log"),
    ("log10", "std::log10"),
    ("log2", "std::log2"),
    ("powi", "std::pow"),
    ("powf", "std::pow"),
];

fn prec(op: &str) -> u8 {
    match op {
        "*" | "/" | "%" => 10,
        "+" | "-" => 9,
        "<<" | ">>" => 8,
        "&" => 7,
        "^" => 6,
        "|" => 5,
        "==" | "!=" | "<" | ">" | "<=" | ">=" => 4,
        "&&" => 3,
        "||" => 2,
        _ => 1,
    }
}

fn is_cmp(op: &str) -> bool {
    matches!(op, "==" | "!=" | "<" | ">" | "<=" | ">=")
}

fn is_bitop(op: &str) -> bool {
    matches!(op, "&" | "^" | "|")
}

pub fn cpp_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '\0' => o.push_str("\\0"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\{:03o}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn char_lit(c: char) -> String {
    match c {
        '\'' => "U'\\''".into(),
        '"' => "U'\"'".into(),
        c if c.is_ascii() && !c.is_ascii_control() && c != '\\' => format!("U'{c}'"),
        '\n' => "U'\\n'".into(),
        '\r' => "U'\\r'".into(),
        '\t' => "U'\\t'".into(),
        '\\' => "U'\\\\'".into(),
        '\0' => "U'\\0'".into(),
        c => format!("U'\\U{:08x}'", c as u32),
    }
}

const INT_SUFFIXES: &[&str] = &[
    "i128", "u128", "isize", "usize", "i16", "i32", "i64", "u16", "u32", "u64", "i8", "u8",
];

fn int_lit(s: &str) -> String {
    let s = s.replace('_', "");
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => ("-", r.to_string()),
        None => ("", s),
    };
    let mut body = s.clone();
    let mut suffix = "";
    for sf in INT_SUFFIXES {
        if s.ends_with(sf) {
            body = s[..s.len() - sf.len()].to_string();
            suffix = sf;
            break;
        }
    }
    if let Some(r) = body.strip_prefix("0o") {
        body = format!("0{r}");
    }
    if suffix.is_empty() {
        format!("{neg}{body}")
    } else {
        format!("static_cast<{suffix}>({neg}{body})")
    }
}

fn float_lit(s: &str) -> String {
    let s = s.replace('_', "");
    let (mut body, f32_) = if let Some(b) = s.strip_suffix("f32") {
        (b.to_string(), true)
    } else if let Some(b) = s.strip_suffix("f64") {
        (b.to_string(), false)
    } else {
        (s, false)
    };
    if body.ends_with('.') {
        body.push('0');
    }
    if f32_ {
        if !body.contains('.') && !body.contains('e') {
            body.push_str(".0");
        }
        body.push('f');
    }
    body
}

fn is_blocklike(e: &Expr) -> bool {
    matches!(
        e,
        Expr::If { .. } | Expr::Match { .. } | Expr::Block { .. } | Expr::While { .. } | Expr::Loop { .. } | Expr::For { .. }
    )
}

fn is_boolish(e: &Expr) -> bool {
    match e {
        Expr::Lit(Lit::Bool(_)) => true,
        Expr::Binary(op, ..) => is_cmp(op) || op == "&&" || op == "||",
        Expr::Unary(op, e) => op == "!" && is_boolish(e),
        Expr::Paren(e) => is_boolish(e),
        Expr::MethodCall { name, .. } => {
            name.starts_with("is_") || name.starts_with("has_") || matches!(name.as_str(), "contains" | "starts_with" | "ends_with" | "any" | "all")
        }
        Expr::Match { arms, .. } => arms.iter().all(|a| matches!(a.body, Expr::Lit(Lit::Bool(_)))),
        _ => false,
    }
}

fn path_names(segs: &[Seg]) -> Vec<String> {
    segs.iter().map(|s| s.name.clone()).collect()
}

fn single_tail(b: &Block) -> Option<&Expr> {
    match b.stmts.as_slice() {
        [Stmt::Expr { expr, semi: false }] if !is_blocklike(expr) || matches!(expr, Expr::If { .. }) => Some(expr),
        _ => None,
    }
}

impl Emitter<'_> {
    // ------------------------------------------------------------------
    // output helpers
    // ------------------------------------------------------------------
    fn indent(&self) -> String {
        "    ".repeat(self.ind)
    }

    pub fn push(&mut self, s: impl Into<String>) {
        self.flush_pre();
        let l = format!("{}{}", self.indent(), s.into());
        self.lines.push(l);
    }

    fn push_pre(&mut self, s: String) {
        let l = format!("{}{}", self.indent(), s);
        self.pre.push(l);
    }

    fn flush_pre(&mut self) {
        let pre = std::mem::take(&mut self.pre);
        self.lines.extend(pre);
    }

    pub fn capture<F: FnOnce(&mut Self)>(&mut self, f: F) -> Vec<String> {
        let saved_lines = std::mem::take(&mut self.lines);
        let saved_pre = std::mem::take(&mut self.pre);
        f(self);
        self.flush_pre();
        self.pre = saved_pre;
        std::mem::replace(&mut self.lines, saved_lines)
    }

    pub fn fresh(&mut self, base: &str) -> String {
        self.tmp += 1;
        format!("_{base}{}", self.tmp)
    }

    // ------------------------------------------------------------------
    // blocks and statements
    // ------------------------------------------------------------------
    pub fn block_into(&mut self, b: &Block, t: &Target) {
        let n = b.stmts.len();
        for (i, s) in b.stmts.iter().enumerate() {
            self.stmt(s, i + 1 == n, t);
        }
    }

    fn stmt(&mut self, s: &Stmt, last: bool, t: &Target) {
        match s {
            Stmt::Let { pat, ty, init, else_block } => self.let_stmt(pat, ty.as_ref(), init.as_ref(), else_block.as_ref()),
            Stmt::Item(it) => self.local_item(it),
            Stmt::Expr { expr, semi } => {
                let tgt = if last && !*semi { t.clone() } else { Target::Discard };
                self.expr_into(expr, &tgt);
            }
        }
    }

    fn local_item(&mut self, it: &Item) {
        match it {
            Item::Fn(f) => {
                let params = f
                    .params
                    .iter()
                    .map(|p| format!("{} {}", self.ty(&p.ty), self.simple_pat_name(&p.pat)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = self.ret_ty(f.ret.as_ref());
                let target = if f.ret.is_none() { Target::Discard } else { Target::Return };
                let body = f.body.clone().unwrap_or(Block { stmts: vec![] });
                let lines = self.capture(|s| {
                    s.ind += 1;
                    s.block_into(&body, &target);
                    s.ind -= 1;
                });
                self.push(format!("auto {} = [&]({params}) -> {ret} {{", id(&f.name)));
                self.lines.extend(lines);
                self.push("};");
            }
            Item::Const(c) => {
                if let Some(v) = &c.value {
                    let v = self.expr(v);
                    let kw = if c.mutable { "" } else { "const " };
                    self.push(format!("{kw}{} {} = {v};", self.ty(&c.ty), id(&c.name)));
                }
            }
            Item::Struct(_) | Item::Enum(_) | Item::Trait(_) | Item::TypeAlias { .. } | Item::Impl(_) | Item::Mod(_) => {
                let text = self.local_decl(it);
                for l in text.lines() {
                    self.push(l.to_string());
                }
            }
            Item::Use(u) => self.push(format!("// use {};", u.path.join("::"))),
            Item::ExternMod(m) => self.push(format!("// mod {m};")),
            Item::Unsupported(s) => self.push(format!("// unsupported item: {s}")),
        }
    }

    fn simple_pat_name(&self, p: &Pat) -> String {
        match p {
            Pat::Ident { name, .. } => id(name),
            _ => "_arg".into(),
        }
    }

    fn is_default_ctor(&self, e: &Expr) -> bool {
        match e {
            Expr::Call(f, args) => match &**f {
                Expr::Path(s) => {
                    let n = path_names(s);
                    n.len() == 2
                        && matches!(
                            n[0].as_str(),
                            "Vec" | "VecDeque" | "HashMap" | "HashSet" | "BTreeMap" | "BTreeSet" | "String"
                        )
                        && matches!(n[1].as_str(), "new" | "default" | "with_capacity")
                        && args.len() <= 1
                }
                _ => false,
            },
            Expr::Macro { name, args, .. } => name == "vec" && args.is_empty(),
            _ => false,
        }
    }

    fn register_box(&mut self, name: &str, ty: Option<&Type>, init: Option<&Expr>) {
        fn is_boxy(t: &Type) -> bool {
            match t {
                Type::Path(s) => s.last().is_some_and(|s| matches!(s.name.as_str(), "Box" | "Rc" | "Arc")),
                Type::Ref { inner, .. } => is_boxy(inner),
                _ => false,
            }
        }
        let by_init = matches!(init, Some(Expr::Call(f, _)) if matches!(&**f, Expr::Path(s)
            if s.len() == 2 && matches!(s[0].name.as_str(), "Box" | "Rc" | "Arc") && s[1].name == "new"));
        if ty.is_some_and(is_boxy) || by_init {
            self.box_vars.insert(name.to_string());
        }
    }

    pub fn register_param(&mut self, p: &Param) {
        if let Pat::Ident { name, .. } = &p.pat {
            self.register_box(name, Some(&p.ty), None);
        }
    }

    fn let_stmt(&mut self, pat: &Pat, ty: Option<&Type>, init: Option<&Expr>, els: Option<&Block>) {
        match (pat, els) {
            (Pat::Ident { name, mutable, sub: None, .. }, None) => {
                self.register_box(name, ty, init);
                let nm = id(name);
                let Some(e) = init else {
                    match ty {
                        Some(t) => {
                            let t = self.ty(t);
                            self.push(format!("{t} {nm};"));
                        }
                        None => self.push(format!("auto {nm}; /* deferred initialisation: add an explicit type */")),
                    }
                    return;
                };
                if let (Some(t), true) = (ty, self.is_default_ctor(e)) {
                    let t = self.ty(t);
                    self.push(format!("{t} {nm};"));
                    return;
                }
                if ty.is_none() {
                    if let Expr::Ref { mutable: m, expr } = e {
                        let v = self.expr(expr);
                        let kw = if *m { "auto&" } else { "const auto&" };
                        self.push(format!("{kw} {nm} = {v};"));
                        return;
                    }
                }
                let v = self.init_expr(e);
                let prim = matches!(ty, Some(Type::Path(s)) if s.len() == 1 && is_primitive(&s[0].name));
                let is_const = !*mutable && (matches!(e, Expr::Lit(_)) || prim);
                let tys = ty.map(|t| self.ty(t)).unwrap_or_else(|| "auto".into());
                let kw = if is_const { "const " } else { "" };
                self.push(format!("{kw}{tys} {nm} = {v};"));
            }
            (Pat::Wild, None) if init.is_some() => {
                let v = self.expr(init.unwrap());
                self.push(format!("static_cast<void>({v});"));
            }
            (Pat::Tuple(elems), None)
                if init.is_some()
                    && elems.iter().all(|e| matches!(e, Pat::Wild | Pat::Ident { sub: None, by_ref: false, .. })) =>
            {
                let v = self.expr(init.unwrap());
                let names: Vec<String> = elems
                    .iter()
                    .map(|e| match e {
                        Pat::Ident { name, .. } => id(name),
                        _ => self.fresh("unused"),
                    })
                    .collect();
                self.push(format!("auto [{}] = {v};", names.join(", ")));
            }
            _ => {
                let Some(e) = init else { return };
                let v = self.expr(e);
                let tmp = self.fresh("l");
                self.push(format!("auto&& {tmp} = {v};"));
                let (cond, binds) = self.pat_cond(pat, &tmp);
                if let Some(b) = els {
                    self.push(format!("if (!({cond})) {{"));
                    self.ind += 1;
                    self.block_into(b, &Target::Discard);
                    self.ind -= 1;
                    self.push("}");
                }
                self.emit_binds(&binds);
            }
        }
    }

    /// Initialiser of a `let`: block-like expressions are wrapped in a lambda.
    fn init_expr(&mut self, e: &Expr) -> String {
        self.expr(e)
    }

    // ------------------------------------------------------------------
    // block-like expressions in statement position
    // ------------------------------------------------------------------
    pub fn expr_into(&mut self, e: &Expr, t: &Target) {
        match e {
            Expr::Paren(inner) if is_blocklike(inner) => self.expr_into(inner, t),
            Expr::If { cond, then, els } => self.if_into(cond, then, els.as_deref(), t),
            Expr::Match { scrut, arms } => self.match_into(scrut, arms, t),
            Expr::Block { label: None, block } => {
                self.push("{");
                self.ind += 1;
                self.block_into(block, t);
                self.ind -= 1;
                self.push("}");
            }
            Expr::Block { label: Some(l), block } => {
                self.loops.push(LoopCtx { label: Some(l.clone()), target: t.clone() });
                self.push("{");
                self.ind += 1;
                self.block_into(block, t);
                self.ind -= 1;
                self.push("}");
                self.loops.pop();
                if self.used_labels.contains(&format!("{l}_break")) {
                    self.push(format!("{l}_break: ;"));
                }
            }
            Expr::While { .. } | Expr::Loop { .. } | Expr::For { .. } => self.loop_into(e, t),
            Expr::Return(v) => match v {
                Some(x) => {
                    let s = self.expr(x);
                    self.push(format!("return {s};"));
                }
                None if self.is_main => self.push("return 0;"),
                None => self.push("return;"),
            },
            Expr::Break(label, val) => {
                let ctx_target = {
                    let ctx = match label {
                        Some(l) => self.loops.iter().rev().find(|c| c.label.as_deref() == Some(l)),
                        None => self.loops.last(),
                    };
                    ctx.map(|c| c.target.clone()).unwrap_or(Target::Discard)
                };
                if let Some(v) = val {
                    let s = self.expr(v);
                    match ctx_target {
                        Target::Assign(var) => self.push(format!("{var} = {s};")),
                        Target::Return => {
                            self.push(format!("return {s};"));
                            return;
                        }
                        Target::Discard => {}
                    }
                }
                match label {
                    Some(l) => {
                        self.used_labels.insert(format!("{l}_break"));
                        self.push(format!("goto {l}_break;"));
                    }
                    None => self.push("break;"),
                }
            }
            Expr::Continue(label) => match label {
                Some(l) => {
                    self.used_labels.insert(format!("{l}_continue"));
                    self.push(format!("goto {l}_continue;"));
                }
                None => self.push("continue;"),
            },
            Expr::Assign(l, r) => {
                if matches!(&**l, Expr::Path(p) if p.len() == 1 && p[0].name == "_") {
                    let v = self.expr(r);
                    self.push(format!("static_cast<void>({v});"));
                } else {
                    let (l, r) = (self.expr(l), self.expr(r));
                    self.push(format!("{l} = {r};"));
                }
            }
            Expr::Macro { name, args, repeat } if matches!(name.as_str(), "panic" | "unreachable" | "todo" | "unimplemented") => {
                let s = self.macro_expr(name, args, *repeat);
                self.push(format!("{s};"));
            }
            Expr::Tuple(v) if v.is_empty() => {}
            _ => {
                let s = self.expr(e);
                match t {
                    Target::Discard => self.push(format!("{s};")),
                    Target::Return => self.push(format!("return {s};")),
                    Target::Assign(v) => self.push(format!("{v} = {s};")),
                }
            }
        }
    }

    fn scrut_temp(&mut self, e: &Expr) -> (String, bool) {
        let simple = matches!(e, Expr::Path(p) if p.len() == 1)
            || matches!(e, Expr::Field(b, _) if matches!(&**b, Expr::Path(p) if p.len() == 1));
        let v = self.expr(e);
        if simple {
            return (v, false);
        }
        let tmp = self.fresh("m");
        self.push("{");
        self.ind += 1;
        self.push(format!("auto&& {tmp} = {v};"));
        (tmp, true)
    }

    fn close_scope(&mut self, opened: bool) {
        if opened {
            self.ind -= 1;
            self.push("}");
        }
    }

    fn if_into(&mut self, cond: &Expr, then: &Block, els: Option<&Expr>, t: &Target) {
        self.if_chain(cond, then, els, t, "");
    }

    fn if_chain(&mut self, cond: &Expr, then: &Block, els: Option<&Expr>, t: &Target, prefix: &str) {
        let mut opened = false;
        let mut binds = Vec::new();
        let c = if let Expr::Let { pat, expr } = cond {
            let (name, o) = self.scrut_temp(expr);
            opened = o;
            let (c, b) = self.pat_cond(pat, &name);
            binds = b;
            c
        } else {
            self.expr(cond)
        };
        self.push(format!("{prefix}if ({c}) {{"));
        self.ind += 1;
        self.emit_binds(&binds);
        self.block_into(then, t);
        self.ind -= 1;
        match els {
            None => self.push("}"),
            Some(Expr::If { cond, then, els }) if !matches!(&**cond, Expr::Let { .. }) => {
                self.if_chain(cond, then, els.as_deref(), t, "} else ");
                self.close_scope(opened);
                return;
            }
            Some(other) => {
                self.push("} else {");
                self.ind += 1;
                match other {
                    Expr::Block { label: None, block } => self.block_into(block, t),
                    o => self.expr_into(o, t),
                }
                self.ind -= 1;
                self.push("}");
            }
        }
        self.close_scope(opened);
    }

    fn match_into(&mut self, scrut: &Expr, arms: &[Arm], t: &Target) {
        let (name, opened) = self.scrut_temp(scrut);
        let guarded = arms.iter().any(|a| a.guard.is_some());
        let end = self.fresh("match_end");
        let mut need_end = false;
        for (i, arm) in arms.iter().enumerate() {
            let (cond, binds) = self.pat_cond(&arm.pat, &name);
            let irrefutable = cond == "true";
            if guarded {
                let mut body_lines = true;
                if irrefutable {
                    self.push("{");
                } else {
                    self.push(format!("if ({cond}) {{"));
                }
                self.ind += 1;
                self.emit_binds(&binds);
                if let Some(g) = &arm.guard {
                    let g = self.expr(g);
                    self.push(format!("if ({g}) {{"));
                    self.ind += 1;
                } else {
                    body_lines = false;
                }
                self.expr_into(&arm.body, t);
                if *t != Target::Return {
                    self.push(format!("goto {end};"));
                    need_end = true;
                }
                if body_lines {
                    self.ind -= 1;
                    self.push("}");
                }
                self.ind -= 1;
                self.push("}");
            } else {
                let head = match (i, irrefutable) {
                    (0, true) => "{".to_string(),
                    (0, false) => format!("if ({cond}) {{"),
                    (_, true) => "} else {".to_string(),
                    (_, false) => format!("}} else if ({cond}) {{"),
                };
                self.push(head);
                self.ind += 1;
                self.emit_binds(&binds);
                self.expr_into(&arm.body, t);
                self.ind -= 1;
                if irrefutable || i + 1 == arms.len() {
                    self.push("}");
                }
                if irrefutable {
                    break;
                }
            }
        }
        if *t == Target::Return {
            let last_irrefutable = arms.last().is_some_and(|a| a.guard.is_none() && matches!(a.pat, Pat::Wild));
            if !last_irrefutable {
                self.push("rust::unreachable();");
            }
        }
        if need_end {
            self.push(format!("{end}: ;"));
        }
        self.close_scope(opened);
    }

    fn loop_into(&mut self, e: &Expr, t: &Target) {
        match e {
            Expr::While { label, cond, body } => {
                if let Expr::Let { pat, expr } = &**cond {
                    let v = self.expr(expr);
                    let tmp = self.fresh("s");
                    let (c, binds) = self.pat_cond(pat, &tmp);
                    let mut prelude = vec![format!("auto&& {tmp} = {v};"), format!("if (!({c})) break;")];
                    prelude.extend(binds.iter().filter(|b| b.name != "_").map(|b| {
                        format!("{} {} = {};", if b.by_value { "auto" } else { "auto&" }, id(&b.name), b.expr)
                    }));
                    self.loop_emit("while (true)".into(), prelude, body, label.clone(), Target::Discard);
                } else {
                    let c = self.expr(cond);
                    self.loop_emit(format!("while ({c})"), vec![], body, label.clone(), Target::Discard);
                }
            }
            Expr::Loop { label, body } => {
                self.loop_emit("while (true)".into(), vec![], body, label.clone(), t.clone());
            }
            Expr::For { label, pat, iter, body } => {
                let mut iter: &Expr = iter;
                while let Expr::Paren(i) = iter {
                    iter = i;
                }
                if let (Expr::Range { lo: Some(lo), hi: Some(hi), inclusive }, Pat::Ident { name, .. }) = (iter, pat) {
                    let (lo, hi) = (self.expr(lo), self.expr(hi));
                    let n = id(name);
                    let op = if *inclusive { "<=" } else { "<" };
                    self.loop_emit(format!("for (auto {n} = {lo}; {n} {op} {hi}; ++{n})"), vec![], body, label.clone(), Target::Discard);
                    return;
                }
                let it = self.for_iter(iter);
                let mut prelude = Vec::new();
                let head = match pat {
                    Pat::Ident { name, mutable, sub: None, .. } => {
                        format!("for ({} {} : {it})", if *mutable { "auto" } else { "auto&&" }, id(name))
                    }
                    Pat::Wild => format!("for (auto&& {} : {it})", self.fresh("unused")),
                    Pat::Tuple(elems) if elems.iter().all(|e| matches!(e, Pat::Wild | Pat::Ident { sub: None, .. })) => {
                        let names: Vec<String> = elems
                            .iter()
                            .map(|e| match e {
                                Pat::Ident { name, .. } => id(name),
                                _ => self.fresh("unused"),
                            })
                            .collect();
                        format!("for (auto&& [{}] : {it})", names.join(", "))
                    }
                    other => {
                        let tmp = self.fresh("item");
                        let (_, binds) = self.pat_cond(other, &tmp);
                        prelude.extend(binds.iter().filter(|b| b.name != "_").map(|b| {
                            format!("{} {} = {};", if b.by_value { "auto" } else { "auto&" }, id(&b.name), b.expr)
                        }));
                        format!("for (auto&& {tmp} : {it})")
                    }
                };
                self.loop_emit(head, prelude, body, label.clone(), Target::Discard);
            }
            _ => unreachable!(),
        }
    }

    fn for_iter(&mut self, e: &Expr) -> String {
        match e {
            Expr::MethodCall { recv, name, args, .. } if args.is_empty() && matches!(name.as_str(), "iter" | "iter_mut" | "into_iter") => {
                self.for_iter(recv)
            }
            Expr::Ref { expr, .. } => self.for_iter(expr),
            Expr::Range { .. } => self.expr(e),
            other => self.expr(other),
        }
    }

    fn loop_emit(&mut self, head: String, prelude: Vec<String>, body: &Block, label: Option<String>, target: Target) {
        self.loops.push(LoopCtx { label: label.clone(), target });
        self.push(format!("{head} {{"));
        self.ind += 1;
        for l in &prelude {
            self.push(l.clone());
        }
        let wrap = label.is_some();
        if wrap {
            self.ind += 1;
        }
        let lines = self.capture(|s| s.block_into(body, &Target::Discard));
        if wrap {
            self.ind -= 1;
        }
        let cont = label.as_ref().is_some_and(|l| self.used_labels.contains(&format!("{l}_continue")));
        if wrap && cont {
            self.push("{");
            self.lines.extend(lines);
            self.push("}");
            self.push(format!("{}_continue: ;", label.as_ref().unwrap()));
        } else if wrap {
            self.lines.extend(lines.into_iter().map(|l| l.strip_prefix("    ").unwrap_or(&l).to_string()));
        } else {
            self.lines.extend(lines);
        }
        self.ind -= 1;
        self.push("}");
        self.loops.pop();
        if let Some(l) = &label {
            if self.used_labels.contains(&format!("{l}_break")) {
                self.push(format!("{l}_break: ;"));
            }
        }
    }

    // ------------------------------------------------------------------
    // expressions
    // ------------------------------------------------------------------
    fn needs_paren(e: &Expr) -> bool {
        matches!(e, Expr::Binary(..) | Expr::Assign(..))
    }

    pub fn operand(&mut self, e: &Expr) -> String {
        let s = self.expr(e);
        if Self::needs_paren(e) { format!("({s})") } else { s }
    }

    /// Strips redundant parentheses (used for arguments of runtime helpers).
    fn bare(&mut self, e: &Expr) -> String {
        match e {
            Expr::Paren(i) => self.bare(i),
            o => self.expr(o),
        }
    }

    fn binary_operand(&mut self, parent: &str, child: &Expr, right: bool) -> String {
        let s = self.expr(child);
        if let Expr::Binary(op, ..) = child {
            let (pp, cp) = (prec(parent), prec(op));
            let mixed = (is_bitop(parent) && is_cmp(op)) || (is_cmp(parent) && is_bitop(op)) || (is_cmp(parent) && is_cmp(op));
            if cp < pp || (right && cp == pp) || mixed || (cp == 1) {
                return format!("({s})");
            }
        } else if Self::needs_paren(child) {
            return format!("({s})");
        }
        s
    }

    pub fn expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::Lit(l) => self.lit(l),
            Expr::Path(segs) => self.path_expr(segs),
            Expr::Unary(op, x) => {
                let o = self.operand(x);
                if op == "!" && !is_boolish(x) && matches!(&**x, Expr::Lit(Lit::Int(_))) {
                    format!("~{o}")
                } else {
                    format!("{op}{o}")
                }
            }
            Expr::Binary(op, l, r) => {
                let (l, r) = (self.binary_operand(op, l, false), self.binary_operand(op, r, true));
                format!("{l} {op} {r}")
            }
            Expr::Assign(l, r) => {
                let (l, r) = (self.expr(l), self.expr(r));
                format!("{l} = {r}")
            }
            Expr::Call(f, args) => self.call(f, args),
            Expr::MethodCall { recv, name, targs, args } => self.method(recv, name, targs, args),
            Expr::Field(base, name) => self.field(base, name),
            Expr::Index(b, i) => {
                if let Expr::Range { lo, hi, inclusive } = &**i {
                    let base = self.expr(b);
                    return match (lo, hi) {
                        (Some(l), Some(h)) => {
                            let (l, h) = (self.expr(l), self.expr(h));
                            let h = if *inclusive { format!("{h} + 1") } else { h };
                            format!("rust::slice({base}, {l}, {h})")
                        }
                        (Some(l), None) => format!("rust::slice_from({base}, {})", self.expr(l)),
                        (None, Some(h)) => {
                            let h = self.expr(h);
                            let h = if *inclusive { format!("{h} + 1") } else { h };
                            format!("rust::slice_to({base}, {h})")
                        }
                        (None, None) => format!("std::span({base})"),
                    };
                }
                let b = self.operand(b);
                let i = self.expr(i);
                format!("{b}[{i}]")
            }
            Expr::If { cond, then, els } => {
                if let (false, Some(els)) = (matches!(&**cond, Expr::Let { .. }), els) {
                    if let Some(t) = self.ternary(cond, then, els) {
                        return t;
                    }
                }
                self.iife(e)
            }
            Expr::Match { .. } | Expr::While { .. } | Expr::Loop { .. } | Expr::For { .. } => self.iife(e),
            Expr::Block { label: None, block } => match single_tail(block) {
                Some(x) => self.expr(x),
                None => self.iife(e),
            },
            Expr::Block { .. } => self.iife(e),
            Expr::Closure { params, ret, body, is_move } => self.closure(params, ret.as_ref(), body, *is_move),
            Expr::Ref { expr, .. } => self.expr(expr),
            Expr::Deref(x) => {
                if matches!(&**x, Expr::Path(p) if p.len() == 1 && p[0].name == "self") && !self.self_free {
                    "(*this)".into()
                } else {
                    format!("(*{})", self.operand(x))
                }
            }
            Expr::Return(_) | Expr::Break(..) | Expr::Continue(_) => {
                "/* control flow in expression position is not supported */ rust::Unit{}".into()
            }
            Expr::Tuple(v) if v.is_empty() => "rust::Unit{}".into(),
            Expr::Tuple(v) => {
                let a: Vec<String> = v.iter().map(|x| self.expr(x)).collect();
                format!("std::make_tuple({})", a.join(", "))
            }
            Expr::Array(v) => {
                let a: Vec<String> = v.iter().map(|x| self.expr(x)).collect();
                format!("std::array{{{}}}", a.join(", "))
            }
            Expr::Repeat(v, n) => {
                let (v, n) = (self.expr(v), self.expr(n));
                format!("rust::filled<{n}>({v})")
            }
            Expr::Struct { path, fields, rest } => self.struct_lit(path, fields, rest.as_deref()),
            Expr::Range { lo, hi, inclusive } => match (lo, hi) {
                (Some(l), Some(h)) => {
                    let (l, h) = (self.expr(l), self.expr(h));
                    if *inclusive {
                        format!("std::views::iota({l}, {h} + 1)")
                    } else {
                        format!("std::views::iota({l}, {h})")
                    }
                }
                (Some(l), None) => format!("std::views::iota({})", self.expr(l)),
                _ => "rust::Unit{} /* open range */".into(),
            },
            Expr::Cast(x, t) => {
                let x = self.operand(x);
                if matches!(t, Type::Ptr { .. }) {
                    format!("reinterpret_cast<{}>({x})", self.ty(t))
                } else {
                    format!("static_cast<{}>({x})", self.ty(t))
                }
            }
            Expr::Try(x) => {
                let inner = self.expr(x);
                let tmp = self.fresh("t");
                self.push_pre(format!("auto {tmp} = {inner};"));
                self.push_pre(format!("if (!{tmp}) return rust::propagate({tmp});"));
                format!("std::move(*{tmp})")
            }
            Expr::Await(x) => format!("co_await {}", self.operand(x)),
            Expr::Let { .. } => "/* let-chains are not supported */ false".into(),
            Expr::Macro { name, args, repeat } => self.macro_expr(name, args, *repeat),
            Expr::Paren(x) => format!("({})", self.expr(x)),
            Expr::Unsupported(s) => format!("/* unsupported: {} */ rust::Unit{{}}", s.replace("*/", "* /")),
        }
    }

    fn lit(&self, l: &Lit) -> String {
        match l {
            Lit::Int(s) => int_lit(s),
            Lit::Float(s) => float_lit(s),
            Lit::Str(s) | Lit::ByteStr(s) => format!("\"{}\"sv", cpp_escape(s)),
            Lit::Char(c) => char_lit(*c),
            Lit::Byte(b) => format!("static_cast<u8>({b})"),
            Lit::Bool(b) => b.to_string(),
        }
    }

    fn ternary(&mut self, cond: &Expr, then: &Block, els: &Expr) -> Option<String> {
        let t = single_tail(then)?;
        let e = match els {
            Expr::Block { label: None, block } => single_tail(block)?.clone(),
            Expr::If { .. } => els.clone(),
            _ => return None,
        };
        if matches!(&e, Expr::If { cond, els: None, .. } if !matches!(&**cond, Expr::Let { .. })) {
            return None;
        }
        let c = self.expr(cond);
        let a = self.operand(t);
        let b = match &e {
            Expr::If { cond, then, els } => match els {
                Some(els) if !matches!(&**cond, Expr::Let { .. }) => match self.ternary(cond, then, els) {
                    Some(x) => x,
                    None => return None,
                },
                _ => return None,
            },
            other => self.operand(other),
        };
        Some(format!("({c} ? {a} : {b})"))
    }

    fn iife(&mut self, e: &Expr) -> String {
        let cap = if self.in_method { "[&]" } else { "[&]" };
        let saved_loops = std::mem::take(&mut self.loops);
        self.ind += 1;
        let lines = self.capture(|s| s.expr_into(e, &Target::Return));
        self.ind -= 1;
        self.loops = saved_loops;
        format!("{cap}() {{\n{}\n{}}}()", lines.join("\n"), self.indent())
    }

    fn closure(&mut self, params: &[ClosureParam], ret: Option<&Type>, body: &Expr, is_move: bool) -> String {
        let mut ps = Vec::new();
        let mut binds: Vec<Bind> = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let ty = p.ty.as_ref().map(|t| self.ty(t)).unwrap_or_else(|| "auto".into());
            match &p.pat {
                Pat::Ident { name, sub: None, .. } => ps.push(format!("{ty} {}", id(name))),
                Pat::Wild => ps.push(format!("{ty} _{i}")),
                other => {
                    let arg = format!("_arg{i}");
                    ps.push(format!("{ty}&& {arg}"));
                    let (_, b) = self.pat_cond(other, &arg);
                    binds.extend(b);
                }
            }
        }
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_main = std::mem::replace(&mut self.is_main, false);
        self.ind += 1;
        let lines = self.capture(|s| {
            s.emit_binds(&binds);
            match body {
                Expr::Block { label: None, block } => s.block_into(block, &Target::Return),
                other => s.expr_into(other, &Target::Return),
            }
        });
        self.ind -= 1;
        self.is_main = saved_main;
        self.loops = saved_loops;
        let cap = if is_move { "[=]" } else { "[&]" };
        let mutable = if is_move { " mutable" } else { "" };
        let ret = ret.map(|r| format!(" -> {}", self.ty(r))).unwrap_or_default();
        let head = format!("{cap}({}){mutable}{ret}", ps.join(", "));
        if lines.len() == 1 {
            format!("{head} {{ {} }}", lines[0].trim())
        } else {
            format!("{head} {{\n{}\n{}}}", lines.join("\n"), self.indent())
        }
    }

    // ------------------------------------------------------------------
    // paths, fields, calls
    // ------------------------------------------------------------------
    pub fn path_names(&self, names: &[String]) -> String {
        let segs: Vec<Seg> = names.iter().map(|n| Seg { name: n.clone(), args: vec![] }).collect();
        self.path_expr_str(&segs)
    }

    fn path_expr(&mut self, segs: &[Seg]) -> String {
        self.path_expr_str(segs)
    }

    fn wrap_variant(&self, v: &super::pat::VariantRef, inner: String) -> String {
        if v.info.data && !v.info.generic { format!("{}{{{inner}}}", id(&v.enum_name)) } else { inner }
    }

    fn variant_struct(v: &super::pat::VariantRef) -> String {
        if v.info.data { format!("{}_{}", id(&v.enum_name), id(&v.variant)) } else { format!("{}::{}", id(&v.enum_name), id(&v.variant)) }
    }

    fn path_expr_str(&self, segs: &[Seg]) -> String {
        let names = path_names(segs);
        if names.len() == 1 {
            match names[0].as_str() {
                "self" => return if self.self_free { "self".into() } else { "(*this)".into() },
                "None" => return "std::nullopt".into(),
                _ => {}
            }
        }
        if let Some(v) = self.resolve_variant(&names) {
            if matches!(v.kind, super::sym::VKind::Unit) {
                return if v.info.data {
                    self.wrap_variant(&v, format!("{}{{}}", Self::variant_struct(&v)))
                } else {
                    Self::variant_struct(&v)
                };
            }
            return Self::variant_struct(&v);
        }
        // numeric constants
        if names.len() >= 2 {
            let (a, b) = (names[names.len() - 2].as_str(), names[names.len() - 1].as_str());
            if is_primitive(a) {
                let lim = |f: &str| format!("std::numeric_limits<{a}>::{f}()");
                match b {
                    "MAX" => return lim("max"),
                    "MIN" => return lim("lowest"),
                    "EPSILON" => return lim("epsilon"),
                    "INFINITY" => return lim("infinity"),
                    "NAN" => return lim("quiet_NaN"),
                    _ => {}
                }
            }
            if a == "consts" {
                let m = match b {
                    "PI" => "pi",
                    "E" => "e",
                    "SQRT_2" => "sqrt2",
                    "LN_2" => "ln2",
                    "LN_10" => "ln10",
                    "FRAC_1_PI" => "inv_pi",
                    "TAU" => return "(2 * std::numbers::pi)".into(),
                    _ => "",
                };
                if !m.is_empty() {
                    return format!("std::numbers::{m}");
                }
            }
        }
        let mut segs = segs;
        let mut prefix = "";
        while segs.len() > 1 && matches!(segs[0].name.as_str(), "crate" | "self" | "super") {
            if segs[0].name == "crate" {
                prefix = "::";
            }
            segs = &segs[1..];
        }
        if segs.len() > 1 && matches!(segs[0].name.as_str(), "std" | "core" | "alloc") {
            let k = segs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>();
            return match &k[k.len().saturating_sub(2)..] {
                ["cmp", "max"] => "std::max".into(),
                ["cmp", "min"] => "std::min".into(),
                ["mem", "swap"] => "std::swap".into(),
                _ => format!("std::{}", k[1..].join("::")),
            };
        }
        let joined = segs
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let n = if i == 0 && s.name == "Self" {
                    self.self_name.clone().unwrap_or_else(|| "Self".into())
                } else {
                    id(&s.name)
                };
                format!("{n}{}", self.targs(&s.args))
            })
            .collect::<Vec<_>>()
            .join("::");
        format!("{prefix}{joined}")
    }

    fn recv_access(&mut self, base: &Expr) -> String {
        match base {
            Expr::Path(p) if p.len() == 1 && p[0].name == "self" => {
                if self.self_free { "self.".into() } else { "this->".into() }
            }
            Expr::Path(p) if p.len() == 1 && self.box_vars.contains(&p[0].name) => format!("{}->", id(&p[0].name)),
            Expr::Ref { expr, .. } => self.recv_access(expr),
            Expr::Paren(inner) if matches!(&**inner, Expr::Ref { .. }) => self.recv_access(inner),
            other => format!("{}.", self.operand(other)),
        }
    }

    fn field(&mut self, base: &Expr, name: &str) -> String {
        if let Some(idx) = name.strip_prefix('_').and_then(|n| n.parse::<usize>().ok()) {
            let is_self = matches!(base, Expr::Path(p) if p.len() == 1 && p[0].name == "self");
            let self_struct = self.self_name.as_ref().is_some_and(|n| self.sym.structs.get(n).is_some_and(|s| s.tuple));
            if !(is_self && self_struct) {
                let b = self.bare(base);
                return format!("rust::field<{idx}>({b})");
            }
        }
        let acc = self.recv_access(base);
        format!("{acc}{}", id(name))
    }

    fn args(&mut self, args: &[Expr]) -> Vec<String> {
        args.iter().map(|a| self.expr(a)).collect()
    }

    fn call(&mut self, f: &Expr, args: &[Expr]) -> String {
        let Expr::Path(segs) = f else {
            let callee = self.operand(f);
            let a = self.args(args).join(", ");
            return format!("{callee}({a})");
        };
        let names = path_names(segs);
        let mut key: Vec<&str> = names.iter().map(String::as_str).collect();
        while key.len() > 1 && matches!(key[0], "std" | "core" | "alloc" | "crate" | "self" | "super") {
            key.remove(0);
        }
        let key_s = key.join("::");
        let a = self.args(args);
        let a_join = a.join(", ");
        let a0 = a.first().cloned().unwrap_or_default();
        match key_s.as_str() {
            "Some" => return format!("std::make_optional({a0})"),
            "Ok" => return format!("rust::Ok({a0})"),
            "Err" => return format!("rust::Err({a0})"),
            "Box::new" => return format!("rust::make_box({a0})"),
            "Rc::new" | "Arc::new" => return format!("rust::make_rc({a0})"),
            "RefCell::new" | "Cell::new" | "Mutex::new" | "RwLock::new" => return format!("rust::Cell{{{a0}}}"),
            "String::from" | "String::from_str" => return format!("std::string({a0})"),
            "String::new" | "String::with_capacity" => return "std::string()".into(),
            "drop" | "mem::drop" => return format!("rust::drop({a0})"),
            "mem::swap" => return format!("std::swap({a_join})"),
            "mem::replace" => return format!("std::exchange({a_join})"),
            "mem::take" => return format!("rust::take({a0})"),
            "cmp::max" | "max" => return format!("std::max({a_join})"),
            "cmp::min" | "min" => return format!("std::min({a_join})"),
            "process::exit" => return format!("std::exit({a0})"),
            "Default::default" => return "{}".into(),
            _ => {}
        }
        if key.len() == 2 && matches!(key[1], "new" | "with_capacity" | "default") {
            let cname = match key[0] {
                "Vec" => Some("std::vector"),
                "VecDeque" => Some("std::deque"),
                "HashMap" => Some("std::unordered_map"),
                "HashSet" => Some("std::unordered_set"),
                "BTreeMap" => Some("std::map"),
                "BTreeSet" => Some("std::set"),
                _ => None,
            };
            if let Some(c) = cname {
                let targs = &segs[segs.len() - 2].args;
                let targs = if targs.is_empty() {
                    if matches!(key[0], "HashMap" | "BTreeMap") { "rust::Infer, rust::Infer".to_string() } else { "rust::Infer".to_string() }
                } else {
                    self.tys(targs)
                };
                return format!("{c}<{targs}>()");
            }
        }
        if key.len() == 2 && key[1] == "from" && is_primitive(key[0]) {
            return format!("static_cast<{}>({a0})", key[0]);
        }
        if let Some(v) = self.resolve_variant(&names) {
            let inner = format!("{}{{{a_join}}}", Self::variant_struct(&v));
            return self.wrap_variant(&v, inner);
        }
        if names.len() == 2 && self.sym.local_trait(&names[0]) && !a.is_empty() {
            let rest = a[1..].join(", ");
            return format!("{}.{}({rest})", a[0], id(&names[1]));
        }
        if names.len() == 1 && names[0].chars().next().is_some_and(|c| c.is_uppercase()) {
            let n = if names[0] == "Self" { self.self_name.clone().unwrap_or_else(|| "Self".into()) } else { id(&names[0]) };
            return format!("{n}{}{{{a_join}}}", self.targs(&segs[0].args));
        }
        let callee = self.path_expr_str(segs);
        format!("{callee}({a_join})")
    }

    fn method(&mut self, recv: &Expr, name: &str, targs: &[Type], args: &[Expr]) -> String {
        let user = self.sym.methods.contains(name);
        if !user {
            if PASSTHROUGH.contains(&name) && args.is_empty() {
                return self.expr(recv);
            }
            let r = self.bare(recv);
            let a = self.args(args);
            let all = |extra: &[String]| -> String {
                let mut v = vec![r.clone()];
                v.extend(extra.iter().cloned());
                v.join(", ")
            };
            match name {
                "len" if a.is_empty() => return format!("{}.size()", self.operand(recv)),
                "is_empty" if a.is_empty() => return format!("{}.empty()", self.operand(recv)),
                "push" if a.len() == 1 => return format!("{}.push_back({})", self.operand(recv), a[0]),
                "is_some" | "is_ok" if a.is_empty() => return format!("{}.has_value()", self.operand(recv)),
                "is_none" | "is_err" if a.is_empty() => return format!("!{}.has_value()", self.operand(recv)),
                "clone" | "to_owned" | "to_vec" if a.is_empty() => return format!("rust::clone({r})"),
                "to_string" if a.is_empty() => return format!("rust::to_string({r})"),
                "as_str" if a.is_empty() => return format!("std::string_view({r})"),
                "pow" if a.len() == 1 => return format!("rust::pow({})", all(&a)),
                "min" | "max" if a.len() == 1 => return format!("std::{name}({})", all(&a)),
                "min" if a.is_empty() => return format!("rust::iter_min({r})"),
                "max" if a.is_empty() => return format!("rust::iter_max({r})"),
                "parse" => {
                    let t = targs.first().map(|t| self.ty(t)).unwrap_or_else(|| "i32".into());
                    return format!("rust::parse<{t}>({r})");
                }
                "collect" => {
                    return match targs.first() {
                        Some(Type::Path(p)) if p.last().is_some_and(|s| s.name == "String") => format!("rust::collect_string({r})"),
                        Some(Type::Path(p)) if p.last().is_some_and(|s| s.name != "Vec") => {
                            let c = self.ty(&Type::Path(vec![Seg { name: p.last().unwrap().name.clone(), args: vec![] }]));
                            format!("rust::collect<{c}>({r})")
                        }
                        _ => format!("rust::collect_vec({r})"),
                    };
                }
                _ => {}
            }
            if let Some((_, f)) = MATH.iter().find(|(n, _)| *n == name) {
                return format!("{f}({})", all(&a));
            }
            if RUNTIME_FNS.contains(&name) {
                return format!("rust::{name}({})", all(&a));
            }
        }
        let acc = self.recv_access(recv);
        let a = self.args(args).join(", ");
        let t = if targs.is_empty() { String::new() } else { format!("<{}>", self.tys(targs)) };
        format!("{acc}{}{t}({a})", id(name))
    }

    fn struct_lit(&mut self, path: &[Seg], fields: &[(String, Expr)], rest: Option<&Expr>) -> String {
        let names = path_names(path);
        let vals: Vec<(String, String)> = fields.iter().map(|(n, e)| (n.clone(), self.expr(e))).collect();
        let order_fields = |declared: &[String], vals: &[(String, String)]| -> Vec<(String, String)> {
            let mut v: Vec<(String, String)> = declared
                .iter()
                .filter_map(|d| vals.iter().find(|(n, _)| n == d).cloned())
                .collect();
            for x in vals {
                if !v.iter().any(|(n, _)| *n == x.0) {
                    v.push(x.clone());
                }
            }
            v
        };
        if let Some(v) = self.resolve_variant(&names) {
            let declared = match &v.kind {
                super::sym::VKind::Named(n) => n.clone(),
                _ => vec![],
            };
            let vals = order_fields(&declared, &vals);
            let init = vals.iter().map(|(n, e)| format!(".{} = {e}", id(n))).collect::<Vec<_>>().join(", ");
            let inner = format!("{}{{{init}}}", Self::variant_struct(&v));
            return self.wrap_variant(&v, inner);
        }
        let last = names.last().cloned().unwrap_or_default();
        let sname = if last == "Self" { self.self_name.clone().unwrap_or_default() } else { last };
        let tname = {
            let mut p = path.to_vec();
            if let Some(l) = p.last_mut() {
                l.name = sname.clone();
            }
            self.path_expr_str(&p)
        };
        let info = self.sym.structs.get(&sname).cloned();
        let declared = info.as_ref().map(|i| i.fields.clone()).unwrap_or_default();
        let vals = if info.is_some() { order_fields(&declared, &vals) } else { vals };
        if let Some(r) = rest {
            let base = self.expr(r);
            let sets = vals.iter().map(|(n, e)| format!("_s.{} = {e};", id(n))).collect::<Vec<_>>().join(" ");
            return format!("[&] {{ auto _s = {base}; {sets} return _s; }}()");
        }
        let designated = info.as_ref().is_some_and(|i| !i.has_bases && !i.tuple);
        let init = if designated {
            vals.iter().map(|(n, e)| format!(".{} = {e}", id(n))).collect::<Vec<_>>().join(", ")
        } else {
            vals.iter().map(|(_, e)| e.clone()).collect::<Vec<_>>().join(", ")
        };
        format!("{tname}{{{init}}}")
    }

    // ------------------------------------------------------------------
    // macros
    // ------------------------------------------------------------------
    /// `"fmt", args...` for the `format!` family, or `None` if the first
    /// argument is not a string literal.
    fn fmt_args(&mut self, args: &[Expr]) -> Option<String> {
        let Some(Expr::Lit(Lit::Str(f))) = args.first() else { return None };
        let pieces = rfmt::parse(f);
        let mut positional = Vec::new();
        let mut named: Vec<(String, String)> = Vec::new();
        for a in &args[1..] {
            match a {
                Expr::Assign(l, r) if matches!(&**l, Expr::Path(p) if p.len() == 1) => {
                    let Expr::Path(p) = &**l else { unreachable!() };
                    named.push((p[0].name.clone(), self.expr(r)));
                }
                other => positional.push(self.expr(other)),
            }
        }
        let mut names: Vec<String> = named.iter().map(|(n, _)| n.clone()).collect();
        let mut explicit = false;
        for p in &pieces {
            if let rfmt::Piece::Placeholder(rfmt::ArgRef::Name(n), _) = p {
                explicit = true;
                if !names.contains(n) {
                    names.push(n.clone());
                    named.push((n.clone(), id(n)));
                }
            }
        }
        let rendered = rfmt::render(&pieces, &names, positional.len(), explicit);
        let mut all: Vec<String> = vec![format!("\"{}\"", cpp_escape(&rendered))];
        all.extend(positional);
        for n in &names {
            if let Some((_, v)) = named.iter().find(|(x, _)| x == n) {
                all.push(v.clone());
            }
        }
        Some(all.join(", "))
    }

    pub fn macro_expr(&mut self, name: &str, args: &[Expr], repeat: bool) -> String {
        match name {
            "println" | "print" | "eprintln" | "eprint" => match self.fmt_args(args) {
                Some(a) => format!("rust::{name}({a})"),
                None if args.is_empty() => format!("rust::{name}(\"{}\")", if name.ends_with("ln") { "" } else { "" }),
                None => format!("rust::{name}(\"{{}}\", {})", self.expr(&args[0])),
            },
            "format" => match self.fmt_args(args) {
                Some(a) => format!("std::format({a})"),
                None => "std::string()".into(),
            },
            "panic" => match self.fmt_args(args) {
                Some(a) => format!("throw std::runtime_error(std::format({a}))"),
                None if args.is_empty() => "throw std::runtime_error(\"explicit panic\")".into(),
                None => format!("throw std::runtime_error({})", self.expr(&args[0])),
            },
            "unreachable" | "todo" | "unimplemented" => format!("throw std::logic_error(\"{name}\")"),
            "write" | "writeln" => {
                let dest = args.first().map(|a| self.expr(a)).unwrap_or_default();
                match self.fmt_args(&args[1.min(args.len())..]) {
                    Some(a) => format!("rust::{name}({dest}, std::format({a}))"),
                    None => format!("rust::{name}({dest}, std::string())"),
                }
            }
            "assert" | "debug_assert" => {
                let c = self.expr(&args[0]);
                match self.fmt_args(&args[1..]) {
                    Some(m) => format!("rust::assert_msg({c}, std::format({m}))"),
                    None => format!("assert({c})"),
                }
            }
            "assert_eq" | "assert_ne" | "debug_assert_eq" | "debug_assert_ne" => {
                let op = if name.ends_with("_eq") { "==" } else { "!=" };
                let (l, r) = (self.operand(&args[0]), self.operand(&args[1]));
                match self.fmt_args(&args[2..]) {
                    Some(m) => format!("rust::assert_msg({l} {op} {r}, std::format({m}))"),
                    None => format!("assert({l} {op} {r})"),
                }
            }
            "vec" => {
                if repeat {
                    let (v, n) = (self.expr(&args[0]), self.expr(&args[1]));
                    format!("std::vector({n}, {v})")
                } else if args.is_empty() {
                    "std::vector<rust::Infer>{}".into()
                } else {
                    format!("std::vector{{{}}}", self.args(args).join(", "))
                }
            }
            "dbg" => format!("rust::dbg({})", self.args(args).join(", ")),
            other => format!("{}({}) /* macro {other}! */", id(other), self.args(args).join(", ")),
        }
    }
}
