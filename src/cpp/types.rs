//! Rust type -> C++ type mapping.

use super::{Emitter, id};
use crate::ir::*;

pub fn is_primitive(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
            | "f32" | "f64" | "bool" | "char"
    )
}

impl Emitter<'_> {
    pub fn ty(&self, t: &Type) -> String {
        match t {
            Type::Path(segs) => self.path_ty(segs),
            Type::Ref { mutable, inner } => match &**inner {
                Type::Path(s) if s.len() == 1 && s[0].name == "str" => "std::string_view".into(),
                Type::Slice(e) => {
                    let e = self.ty(e);
                    if *mutable { format!("std::span<{e}>") } else { format!("std::span<const {e}>") }
                }
                other => {
                    let i = self.ty(other);
                    if *mutable { format!("{i}&") } else { format!("const {i}&") }
                }
            },
            Type::Ptr { mutable, inner } => {
                let i = self.ty(inner);
                if *mutable { format!("{i}*") } else { format!("const {i}*") }
            }
            Type::Tuple(ts) => format!("std::tuple<{}>", self.tys(ts)),
            Type::Unit => "rust::Unit".into(),
            Type::Array(e, n) => format!("std::array<{}, {}>", self.ty(e), self.const_expr(n)),
            Type::Slice(e) => format!("std::span<{}>", self.ty(e)),
            Type::Fn(args, ret) => self.fn_ty(args, ret.as_deref()),
            Type::ImplTrait(bs) => {
                for b in bs {
                    if self.sym.local_trait(&b.name) {
                        return format!("Is{} auto", id(&b.name));
                    }
                }
                "auto".into()
            }
            Type::Dyn(bs) => self.dyn_ty(bs),
            Type::Never => "void".into(),
            Type::Infer => "auto".into(),
            Type::Const(e) => self.const_expr(e),
        }
    }

    pub fn tys(&self, ts: &[Type]) -> String {
        ts.iter().map(|t| self.ty(t)).collect::<Vec<_>>().join(", ")
    }

    fn const_expr(&self, e: &Expr) -> String {
        match e {
            Expr::Lit(Lit::Int(i)) => i.trim_end_matches(|c: char| c.is_alphabetic() && c != 'x').replace('_', ""),
            Expr::Path(p) => p.iter().map(|s| id(&s.name)).collect::<Vec<_>>().join("::"),
            Expr::Binary(op, l, r) => format!("{} {} {}", self.const_expr(l), op, self.const_expr(r)),
            Expr::Paren(e) => format!("({})", self.const_expr(e)),
            _ => "0".into(),
        }
    }

    pub fn fn_ty(&self, args: &[Type], ret: Option<&Type>) -> String {
        format!("std::function<{}({})>", self.ret_ty(ret), self.tys(args))
    }

    fn dyn_ty(&self, bs: &[TraitBoundRef]) -> String {
        for b in bs {
            if let Some((a, r)) = &b.fn_sig {
                return self.fn_ty(a, r.as_deref());
            }
        }
        match bs.first() {
            Some(b) => {
                let args = if b.args.is_empty() { String::new() } else { format!("<{}>", self.tys(&b.args)) };
                format!("{}{}", id(&b.name), args)
            }
            None => "auto".into(),
        }
    }

    pub fn ret_ty(&self, t: Option<&Type>) -> String {
        match t {
            None | Some(Type::Unit) | Some(Type::Never) => "void".into(),
            Some(t) => self.ty(t),
        }
    }

    fn path_ty(&self, segs: &[Seg]) -> String {
        let mut prefix = "";
        let mut segs = segs;
        while segs.len() > 1 && matches!(segs[0].name.as_str(), "crate" | "self" | "super") {
            if segs[0].name == "crate" {
                prefix = "::";
            }
            segs = &segs[1..];
        }
        if segs.len() > 1 && matches!(segs[0].name.as_str(), "std" | "core" | "alloc") {
            segs = &segs[segs.len() - 1..];
            // `fmt::Result` style paths keep their parent for disambiguation.
        }
        if segs.len() == 2 && segs[0].name == "fmt" && segs[1].name == "Result" {
            return "rust::FmtResult".into();
        }
        if segs.len() == 2 && matches!(segs[0].name.as_str(), "fmt" | "io") && segs[1].name == "Formatter" {
            return "std::ostream".into();
        }
        if segs.len() == 1 {
            return self.single_ty(&segs[0]);
        }
        let first = &segs[0].name;
        let dependent = first == "Self" || (first.len() == 1 && first.chars().all(|c| c.is_ascii_uppercase()));
        let joined = segs
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if i == 0 && s.name == "Self" {
                    self.self_name.clone().unwrap_or_else(|| "Self".into())
                } else {
                    format!("{}{}", id(&s.name), self.targs(&s.args))
                }
            })
            .collect::<Vec<_>>()
            .join("::");
        format!("{}{}{}", if dependent { "typename " } else { "" }, prefix, joined)
    }

    pub fn targs(&self, args: &[Type]) -> String {
        if args.is_empty() { String::new() } else { format!("<{}>", self.tys(args)) }
    }

    fn single_ty(&self, s: &Seg) -> String {
        let args = self.targs(&s.args);
        let simple = |n: &str| format!("{n}{args}");
        match s.name.as_str() {
            "String" => "std::string".into(),
            "str" => "std::string_view".into(),
            "char" => "char32_t".into(),
            "Vec" => simple("std::vector"),
            "VecDeque" => simple("std::deque"),
            "HashMap" => simple("std::unordered_map"),
            "HashSet" => simple("std::unordered_set"),
            "BTreeMap" => simple("std::map"),
            "BTreeSet" => simple("std::set"),
            "Option" => simple("std::optional"),
            "Result" => simple("rust::Result"),
            "Rc" | "Arc" => simple("std::shared_ptr"),
            "Weak" => simple("std::weak_ptr"),
            "RefCell" | "Cell" | "RwLock" => simple("rust::RefCell"),
            "Mutex" => simple("rust::Mutex"),
            "PhantomData" => simple("rust::PhantomData"),
            "Formatter" => "std::ostream".into(),
            "Ordering" => "std::strong_ordering".into(),
            "Box" => match s.args.first() {
                Some(Type::Dyn(bs)) if bs.iter().any(|b| b.fn_sig.is_some()) => self.dyn_ty(bs),
                Some(inner) => format!("std::unique_ptr<{}>", self.ty(inner)),
                None => "std::unique_ptr".into(),
            },
            "Self" => self.self_name.clone().unwrap_or_else(|| "Self".into()),
            n => format!("{}{}", id(n), args),
        }
    }
}
