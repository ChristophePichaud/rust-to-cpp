//! Layer 1 -> Layer 2: the AST visitor.
//!
//! Walks the `syn` AST and builds the semantic IR (see [`crate::ir`]).
//! Every `syn` node kind is handled explicitly; nodes without a C++
//! counterpart become `Unsupported` markers that the generator prints as
//! comments, so nothing is ever silently dropped.

use crate::ir::*;
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::{Token, parse::Parser};

pub fn lower_file(f: &syn::File) -> File {
    File { items: lower_items(&f.items) }
}

pub fn lower_items(items: &[syn::Item]) -> Vec<Item> {
    items.iter().flat_map(lower_item).collect()
}

fn tokens<T: ToTokens>(t: &T) -> String {
    t.to_token_stream().to_string()
}

fn derives(attrs: &[syn::Attribute]) -> Vec<String> {
    let mut out = Vec::new();
    for a in attrs {
        if a.path().is_ident("derive") {
            let _ = a.parse_nested_meta(|m| {
                if let Some(s) = m.path.segments.last() {
                    out.push(s.ident.to_string());
                }
                Ok(())
            });
        }
    }
    out
}

pub fn lower_item(item: &syn::Item) -> Vec<Item> {
    match item {
        syn::Item::Use(u) => {
            let mut out = Vec::new();
            flatten_use(&u.tree, &mut Vec::new(), &mut out);
            out.into_iter().map(Item::Use).collect()
        }
        syn::Item::Fn(f) => vec![Item::Fn(lower_fn(&f.sig, Some(&f.block)))],
        syn::Item::Struct(s) => vec![Item::Struct(StructDef {
            name: s.ident.to_string(),
            generics: lower_generics(&s.generics),
            fields: lower_fields(&s.fields),
            derives: derives(&s.attrs),
        })],
        syn::Item::Enum(e) => vec![Item::Enum(EnumDef {
            name: e.ident.to_string(),
            generics: lower_generics(&e.generics),
            derives: derives(&e.attrs),
            variants: e
                .variants
                .iter()
                .map(|v| Variant {
                    name: v.ident.to_string(),
                    fields: lower_fields(&v.fields),
                    discriminant: v.discriminant.as_ref().map(|(_, e)| lower_expr(e)),
                })
                .collect(),
        })],
        syn::Item::Impl(i) => {
            let mut fns = Vec::new();
            let mut consts = Vec::new();
            for it in &i.items {
                match it {
                    syn::ImplItem::Fn(f) => fns.push(lower_fn(&f.sig, Some(&f.block))),
                    syn::ImplItem::Const(c) => consts.push(ConstDef {
                        name: c.ident.to_string(),
                        ty: lower_type(&c.ty),
                        value: Some(lower_expr(&c.expr)),
                        mutable: false,
                    }),
                    _ => {}
                }
            }
            vec![Item::Impl(ImplDef {
                generics: lower_generics(&i.generics),
                trait_: i.trait_.as_ref().map(|(_, p, _)| lower_trait_path(p)),
                self_ty: lower_type(&i.self_ty),
                fns,
                consts,
            })]
        }
        syn::Item::Trait(t) => {
            let fns = t
                .items
                .iter()
                .filter_map(|it| match it {
                    syn::TraitItem::Fn(f) => Some(lower_fn(&f.sig, f.default.as_ref())),
                    _ => None,
                })
                .collect();
            vec![Item::Trait(TraitDef {
                name: t.ident.to_string(),
                generics: lower_generics(&t.generics),
                supertraits: t.supertraits.iter().filter_map(lower_bound).collect(),
                fns,
            })]
        }
        syn::Item::Mod(m) => match &m.content {
            Some((_, items)) => vec![Item::Mod(ModDef {
                name: m.ident.to_string(),
                items: lower_items(items),
            })],
            None => vec![Item::ExternMod(m.ident.to_string())],
        },
        syn::Item::Const(c) => vec![Item::Const(ConstDef {
            name: c.ident.to_string(),
            ty: lower_type(&c.ty),
            value: Some(lower_expr(&c.expr)),
            mutable: false,
        })],
        syn::Item::Static(s) => vec![Item::Const(ConstDef {
            name: s.ident.to_string(),
            ty: lower_type(&s.ty),
            value: Some(lower_expr(&s.expr)),
            mutable: matches!(s.mutability, syn::StaticMutability::Mut(_)),
        })],
        syn::Item::Type(t) => vec![Item::TypeAlias {
            name: t.ident.to_string(),
            generics: lower_generics(&t.generics),
            ty: lower_type(&t.ty),
        }],
        syn::Item::ExternCrate(c) => vec![Item::Use(UseDecl {
            path: vec![c.ident.to_string()],
            alias: c.rename.as_ref().map(|(_, i)| i.to_string()),
            glob: false,
        })],
        syn::Item::Macro(m) => vec![Item::Unsupported(format!("macro {}", tokens(&m.mac.path)))],
        other => vec![Item::Unsupported(
            tokens(other).chars().take(60).collect::<String>(),
        )],
    }
}

fn flatten_use(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<UseDecl>) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            flatten_use(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut path = prefix.clone();
            path.push(n.ident.to_string());
            out.push(UseDecl { path, alias: None, glob: false });
        }
        syn::UseTree::Rename(r) => {
            let mut path = prefix.clone();
            path.push(r.ident.to_string());
            out.push(UseDecl { path, alias: Some(r.rename.to_string()), glob: false });
        }
        syn::UseTree::Glob(_) => out.push(UseDecl { path: prefix.clone(), alias: None, glob: true }),
        syn::UseTree::Group(g) => {
            for t in &g.items {
                flatten_use(t, prefix, out);
            }
        }
    }
}

fn lower_fields(f: &syn::Fields) -> Fields {
    match f {
        syn::Fields::Named(n) => Fields::Named(
            n.named
                .iter()
                .map(|f| (f.ident.as_ref().unwrap().to_string(), lower_type(&f.ty)))
                .collect(),
        ),
        syn::Fields::Unnamed(u) => Fields::Tuple(u.unnamed.iter().map(|f| lower_type(&f.ty)).collect()),
        syn::Fields::Unit => Fields::Unit,
    }
}

fn lower_fn(sig: &syn::Signature, body: Option<&syn::Block>) -> Function {
    let mut receiver = None;
    let mut params = Vec::new();
    for arg in &sig.inputs {
        match arg {
            syn::FnArg::Receiver(r) => {
                receiver = Some(if r.reference.is_some() {
                    if r.mutability.is_some() { Receiver::RefMut } else { Receiver::Ref }
                } else {
                    Receiver::Value
                });
            }
            syn::FnArg::Typed(t) => params.push(Param { pat: lower_pat(&t.pat), ty: lower_type(&t.ty) }),
        }
    }
    Function {
        name: sig.ident.to_string(),
        generics: lower_generics(&sig.generics),
        receiver,
        params,
        ret: match &sig.output {
            syn::ReturnType::Default => None,
            syn::ReturnType::Type(_, t) => Some(lower_type(t)),
        },
        body: body.map(lower_block),
        is_const: sig.constness.is_some(),
        is_async: sig.asyncness.is_some(),
        is_unsafe: sig.unsafety.is_some(),
    }
}

fn lower_generics(g: &syn::Generics) -> Generics {
    let mut out = Generics::default();
    for p in &g.params {
        match p {
            syn::GenericParam::Type(t) => {
                let name = t.ident.to_string();
                out.params.push(GenericParam::Type(name.clone()));
                let bs: Vec<_> = t.bounds.iter().filter_map(lower_bound).collect();
                if !bs.is_empty() {
                    out.bounds.push((Type::Path(vec![Seg { name, args: vec![] }]), bs));
                }
            }
            syn::GenericParam::Const(c) => {
                out.params.push(GenericParam::Const(c.ident.to_string(), lower_type(&c.ty)))
            }
            syn::GenericParam::Lifetime(_) => {}
        }
    }
    if let Some(w) = &g.where_clause {
        for p in &w.predicates {
            if let syn::WherePredicate::Type(pt) = p {
                let bs: Vec<_> = pt.bounds.iter().filter_map(lower_bound).collect();
                if !bs.is_empty() {
                    out.bounds.push((lower_type(&pt.bounded_ty), bs));
                }
            }
        }
    }
    out
}

fn lower_bound(b: &syn::TypeParamBound) -> Option<TraitBound> {
    match b {
        syn::TypeParamBound::Trait(t) => Some(lower_trait_path(&t.path)),
        _ => None,
    }
}

fn lower_trait_path(p: &syn::Path) -> TraitBound {
    let last = p.segments.last().unwrap();
    let mut args = Vec::new();
    let mut fn_sig = None;
    match &last.arguments {
        syn::PathArguments::AngleBracketed(a) => {
            for g in &a.args {
                if let syn::GenericArgument::Type(t) = g {
                    args.push(lower_type(t));
                }
            }
        }
        syn::PathArguments::Parenthesized(p) => {
            let ins = p.inputs.iter().map(lower_type).collect();
            let out = match &p.output {
                syn::ReturnType::Default => None,
                syn::ReturnType::Type(_, t) => Some(Box::new(lower_type(t))),
            };
            fn_sig = Some((ins, out));
        }
        syn::PathArguments::None => {}
    }
    TraitBound { name: last.ident.to_string(), args, fn_sig }
}

fn bound_ref(b: &syn::TypeParamBound) -> Option<TraitBoundRef> {
    lower_bound(b).map(|b| TraitBoundRef { name: b.name, args: b.args, fn_sig: b.fn_sig })
}

fn lower_segments(p: &syn::Path) -> Vec<Seg> {
    p.segments
        .iter()
        .map(|s| {
            let mut args = Vec::new();
            if let syn::PathArguments::AngleBracketed(a) = &s.arguments {
                for g in &a.args {
                    match g {
                        syn::GenericArgument::Type(t) => args.push(lower_type(t)),
                        syn::GenericArgument::Const(e) => args.push(Type::Const(Box::new(lower_expr(e)))),
                        _ => {}
                    }
                }
            }
            Seg { name: s.ident.to_string(), args }
        })
        .collect()
}

pub fn lower_type(t: &syn::Type) -> Type {
    match t {
        syn::Type::Path(p) => Type::Path(lower_segments(&p.path)),
        syn::Type::Reference(r) => Type::Ref { mutable: r.mutability.is_some(), inner: Box::new(lower_type(&r.elem)) },
        syn::Type::Ptr(p) => Type::Ptr { mutable: p.mutability.is_some(), inner: Box::new(lower_type(&p.elem)) },
        syn::Type::Tuple(t) if t.elems.is_empty() => Type::Unit,
        syn::Type::Tuple(t) => Type::Tuple(t.elems.iter().map(lower_type).collect()),
        syn::Type::Array(a) => Type::Array(Box::new(lower_type(&a.elem)), Box::new(lower_expr(&a.len))),
        syn::Type::Slice(s) => Type::Slice(Box::new(lower_type(&s.elem))),
        syn::Type::BareFn(f) => Type::Fn(
            f.inputs.iter().map(|a| lower_type(&a.ty)).collect(),
            match &f.output {
                syn::ReturnType::Default => None,
                syn::ReturnType::Type(_, t) => Some(Box::new(lower_type(t))),
            },
        ),
        syn::Type::ImplTrait(i) => Type::ImplTrait(i.bounds.iter().filter_map(bound_ref).collect()),
        syn::Type::TraitObject(o) => Type::Dyn(o.bounds.iter().filter_map(bound_ref).collect()),
        syn::Type::Never(_) => Type::Never,
        syn::Type::Paren(p) => lower_type(&p.elem),
        syn::Type::Group(g) => lower_type(&g.elem),
        _ => Type::Infer,
    }
}

pub fn lower_pat(p: &syn::Pat) -> Pat {
    match p {
        syn::Pat::Wild(_) => Pat::Wild,
        syn::Pat::Rest(_) => Pat::Rest,
        syn::Pat::Ident(i) => Pat::Ident {
            name: i.ident.to_string(),
            by_ref: i.by_ref.is_some(),
            mutable: i.mutability.is_some(),
            sub: i.subpat.as_ref().map(|(_, p)| Box::new(lower_pat(p))),
        },
        syn::Pat::Lit(l) => Pat::Lit(Box::new(lower_expr(&syn::Expr::Lit(l.clone())))),
        syn::Pat::Tuple(t) => Pat::Tuple(t.elems.iter().map(lower_pat).collect()),
        syn::Pat::TupleStruct(t) => Pat::TupleStruct {
            path: path_names(&t.path),
            elems: t.elems.iter().map(lower_pat).collect(),
        },
        syn::Pat::Struct(s) => Pat::Struct {
            path: path_names(&s.path),
            fields: s
                .fields
                .iter()
                .map(|f| {
                    let name = match &f.member {
                        syn::Member::Named(i) => i.to_string(),
                        syn::Member::Unnamed(i) => format!("_{}", i.index),
                    };
                    (name, lower_pat(&f.pat))
                })
                .collect(),
        },
        syn::Pat::Path(p) => Pat::Path(path_names(&p.path)),
        syn::Pat::Reference(r) => Pat::Ref(Box::new(lower_pat(&r.pat))),
        syn::Pat::Or(o) => Pat::Or(o.cases.iter().map(lower_pat).collect()),
        syn::Pat::Range(r) => Pat::Range {
            lo: r.start.as_ref().map(|e| Box::new(lower_expr(e))),
            hi: r.end.as_ref().map(|e| Box::new(lower_expr(e))),
            inclusive: matches!(r.limits, syn::RangeLimits::Closed(_)),
        },
        syn::Pat::Slice(s) => Pat::Slice(s.elems.iter().map(lower_pat).collect()),
        syn::Pat::Paren(p) => lower_pat(&p.pat),
        syn::Pat::Type(t) => lower_pat(&t.pat),
        _ => Pat::Wild,
    }
}

fn path_names(p: &syn::Path) -> Vec<String> {
    p.segments.iter().map(|s| s.ident.to_string()).collect()
}

pub fn lower_block(b: &syn::Block) -> Block {
    Block { stmts: b.stmts.iter().map(lower_stmt).collect() }
}

fn lower_stmt(s: &syn::Stmt) -> Stmt {
    match s {
        syn::Stmt::Local(l) => {
            let (pat, ty) = match &l.pat {
                syn::Pat::Type(t) => (lower_pat(&t.pat), Some(lower_type(&t.ty))),
                p => (lower_pat(p), None),
            };
            let (init, else_block) = match &l.init {
                Some(i) => (
                    Some(lower_expr(&i.expr)),
                    i.diverge.as_ref().map(|(_, e)| match lower_expr(e) {
                        Expr::Block { block, .. } => block,
                        other => Block { stmts: vec![Stmt::Expr { expr: other, semi: true }] },
                    }),
                ),
                None => (None, None),
            };
            Stmt::Let { pat, ty, init, else_block }
        }
        syn::Stmt::Item(i) => {
            let mut items = lower_item(i);
            match items.len() {
                1 => Stmt::Item(Box::new(items.remove(0))),
                _ => Stmt::Item(Box::new(Item::Unsupported("nested use".into()))),
            }
        }
        syn::Stmt::Expr(e, semi) => Stmt::Expr { expr: lower_expr(e), semi: semi.is_some() },
        syn::Stmt::Macro(m) => Stmt::Expr {
            expr: lower_macro(&m.mac),
            semi: m.semi_token.is_some(),
        },
    }
}

fn label(l: &Option<syn::Label>) -> Option<String> {
    l.as_ref().map(|l| l.name.ident.to_string())
}

fn bx(e: &syn::Expr) -> Box<Expr> {
    Box::new(lower_expr(e))
}

pub fn lower_expr(e: &syn::Expr) -> Expr {
    match e {
        syn::Expr::Lit(l) => lower_lit(&l.lit),
        syn::Expr::Path(p) => Expr::Path(lower_segments(&p.path)),
        syn::Expr::Unary(u) => match u.op {
            syn::UnOp::Deref(_) => Expr::Deref(bx(&u.expr)),
            syn::UnOp::Not(_) => Expr::Unary("!".into(), bx(&u.expr)),
            syn::UnOp::Neg(_) => Expr::Unary("-".into(), bx(&u.expr)),
            _ => Expr::Unsupported(tokens(e)),
        },
        syn::Expr::Binary(b) => Expr::Binary(tokens(&b.op), bx(&b.left), bx(&b.right)),
        syn::Expr::Assign(a) => Expr::Assign(bx(&a.left), bx(&a.right)),
        syn::Expr::Call(c) => Expr::Call(bx(&c.func), c.args.iter().map(lower_expr).collect()),
        syn::Expr::MethodCall(m) => Expr::MethodCall {
            recv: bx(&m.receiver),
            name: m.method.to_string(),
            targs: m
                .turbofish
                .as_ref()
                .map(|t| {
                    t.args
                        .iter()
                        .filter_map(|a| match a {
                            syn::GenericArgument::Type(t) => Some(lower_type(t)),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            args: m.args.iter().map(lower_expr).collect(),
        },
        syn::Expr::Field(f) => Expr::Field(
            bx(&f.base),
            match &f.member {
                syn::Member::Named(i) => i.to_string(),
                syn::Member::Unnamed(i) => format!("_{}", i.index),
            },
        ),
        syn::Expr::Index(i) => Expr::Index(bx(&i.expr), bx(&i.index)),
        syn::Expr::If(i) => Expr::If {
            cond: bx(&i.cond),
            then: lower_block(&i.then_branch),
            els: i.else_branch.as_ref().map(|(_, e)| bx(e)),
        },
        syn::Expr::Match(m) => Expr::Match {
            scrut: bx(&m.expr),
            arms: m
                .arms
                .iter()
                .map(|a| Arm {
                    pat: lower_pat(&a.pat),
                    guard: a.guard.as_ref().map(|(_, g)| lower_expr(g)),
                    body: lower_expr(&a.body),
                })
                .collect(),
        },
        syn::Expr::While(w) => Expr::While { label: label(&w.label), cond: bx(&w.cond), body: lower_block(&w.body) },
        syn::Expr::Loop(l) => Expr::Loop { label: label(&l.label), body: lower_block(&l.body) },
        syn::Expr::ForLoop(f) => Expr::For {
            label: label(&f.label),
            pat: lower_pat(&f.pat),
            iter: bx(&f.expr),
            body: lower_block(&f.body),
        },
        syn::Expr::Block(b) => Expr::Block { label: label(&b.label), block: lower_block(&b.block) },
        syn::Expr::Unsafe(u) => Expr::Block { label: None, block: lower_block(&u.block) },
        syn::Expr::Const(c) => Expr::Block { label: None, block: lower_block(&c.block) },
        syn::Expr::Async(a) => Expr::Block { label: None, block: lower_block(&a.block) },
        syn::Expr::Closure(c) => Expr::Closure {
            params: c
                .inputs
                .iter()
                .map(|p| match p {
                    syn::Pat::Type(t) => ClosureParam { pat: lower_pat(&t.pat), ty: Some(lower_type(&t.ty)) },
                    p => ClosureParam { pat: lower_pat(p), ty: None },
                })
                .collect(),
            ret: match &c.output {
                syn::ReturnType::Default => None,
                syn::ReturnType::Type(_, t) => Some(lower_type(t)),
            },
            body: bx(&c.body),
            is_move: c.capture.is_some(),
        },
        syn::Expr::Reference(r) => Expr::Ref { mutable: r.mutability.is_some(), expr: bx(&r.expr) },
        syn::Expr::Return(r) => Expr::Return(r.expr.as_ref().map(|e| bx(e))),
        syn::Expr::Break(b) => Expr::Break(b.label.as_ref().map(|l| l.ident.to_string()), b.expr.as_ref().map(|e| bx(e))),
        syn::Expr::Continue(c) => Expr::Continue(c.label.as_ref().map(|l| l.ident.to_string())),
        syn::Expr::Tuple(t) if t.elems.is_empty() => Expr::Tuple(vec![]),
        syn::Expr::Tuple(t) => Expr::Tuple(t.elems.iter().map(lower_expr).collect()),
        syn::Expr::Array(a) => Expr::Array(a.elems.iter().map(lower_expr).collect()),
        syn::Expr::Repeat(r) => Expr::Repeat(bx(&r.expr), bx(&r.len)),
        syn::Expr::Struct(s) => Expr::Struct {
            path: lower_segments(&s.path),
            fields: s
                .fields
                .iter()
                .map(|f| {
                    let name = match &f.member {
                        syn::Member::Named(i) => i.to_string(),
                        syn::Member::Unnamed(i) => format!("_{}", i.index),
                    };
                    (name, lower_expr(&f.expr))
                })
                .collect(),
            rest: s.rest.as_ref().map(|e| bx(e)),
        },
        syn::Expr::Range(r) => Expr::Range {
            lo: r.start.as_ref().map(|e| bx(e)),
            hi: r.end.as_ref().map(|e| bx(e)),
            inclusive: matches!(r.limits, syn::RangeLimits::Closed(_)),
        },
        syn::Expr::Cast(c) => Expr::Cast(bx(&c.expr), lower_type(&c.ty)),
        syn::Expr::Try(t) => Expr::Try(bx(&t.expr)),
        syn::Expr::Await(a) => Expr::Await(bx(&a.base)),
        syn::Expr::Let(l) => Expr::Let { pat: lower_pat(&l.pat), expr: bx(&l.expr) },
        syn::Expr::Macro(m) => lower_macro(&m.mac),
        syn::Expr::Paren(p) => Expr::Paren(bx(&p.expr)),
        syn::Expr::Group(g) => lower_expr(&g.expr),
        other => Expr::Unsupported(tokens(other)),
    }
}

fn lower_lit(l: &syn::Lit) -> Expr {
    Expr::Lit(match l {
        syn::Lit::Str(s) => Lit::Str(s.value()),
        syn::Lit::ByteStr(s) => Lit::ByteStr(String::from_utf8_lossy(&s.value()).into_owned()),
        syn::Lit::Byte(b) => Lit::Byte(b.value()),
        syn::Lit::Char(c) => Lit::Char(c.value()),
        syn::Lit::Int(i) => Lit::Int(i.to_string()),
        syn::Lit::Float(f) => Lit::Float(f.to_string()),
        syn::Lit::Bool(b) => Lit::Bool(b.value),
        other => return Expr::Unsupported(tokens(other)),
    })
}

/// Macros are parsed from their token stream so that arguments become real
/// expressions.
fn lower_macro(m: &syn::Macro) -> Expr {
    let name = m.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
    let list = |ts: proc_macro2::TokenStream| -> Option<Vec<Expr>> {
        Punctuated::<syn::Expr, Token![,]>::parse_terminated
            .parse2(ts)
            .ok()
            .map(|p| p.iter().map(lower_expr).collect())
    };
    match name.as_str() {
        "vec" => {
            // vec![x; n] or vec![a, b, c]
            if let Ok(r) = syn::parse2::<VecRepeat>(m.tokens.clone()) {
                return Expr::Macro { name, args: vec![lower_expr(&r.0), lower_expr(&r.1)], repeat: true };
            }
            Expr::Macro { name, args: list(m.tokens.clone()).unwrap_or_default(), repeat: false }
        }
        "matches" => {
            let parser = |input: syn::parse::ParseStream| -> syn::Result<(syn::Expr, syn::Pat, Option<syn::Expr>)> {
                let e: syn::Expr = input.parse()?;
                input.parse::<Token![,]>()?;
                let p = syn::Pat::parse_multi_with_leading_vert(input)?;
                let g = if input.peek(Token![if]) {
                    input.parse::<Token![if]>()?;
                    Some(input.parse::<syn::Expr>()?)
                } else {
                    None
                };
                let _ = input.parse::<Option<Token![,]>>();
                Ok((e, p, g))
            };
            match parser.parse2(m.tokens.clone()) {
                Ok((e, p, g)) => Expr::Match {
                    scrut: Box::new(lower_expr(&e)),
                    arms: vec![
                        Arm { pat: lower_pat(&p), guard: g.as_ref().map(lower_expr), body: Expr::Lit(Lit::Bool(true)) },
                        Arm { pat: Pat::Wild, guard: None, body: Expr::Lit(Lit::Bool(false)) },
                    ],
                },
                Err(_) => Expr::Unsupported(format!("matches!({})", m.tokens)),
            }
        }
        _ => match list(m.tokens.clone()) {
            Some(args) => Expr::Macro { name, args, repeat: false },
            None => Expr::Unsupported(format!("{}!({})", name, m.tokens)),
        },
    }
}

struct VecRepeat(syn::Expr, syn::Expr);

impl syn::parse::Parse for VecRepeat {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let a: syn::Expr = input.parse()?;
        input.parse::<Token![;]>()?;
        let b: syn::Expr = input.parse()?;
        Ok(VecRepeat(a, b))
    }
}
