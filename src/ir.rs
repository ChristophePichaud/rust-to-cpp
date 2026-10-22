//! Layer 2 - the semantic intermediate representation (IR).
//!
//! The IR is a small, owned, `syn`-independent tree. It keeps every Rust
//! construct the generator needs (items, types, patterns, expressions) in a
//! normalised form so the C++ back end never has to look at `syn` again.

#[derive(Debug, Clone, Default)]
pub struct File {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Use(UseDecl),
    Fn(Function),
    Struct(StructDef),
    Enum(EnumDef),
    Impl(ImplDef),
    Trait(TraitDef),
    Mod(ModDef),
    /// `mod name;` - a module living in another file.
    ExternMod(String),
    Const(ConstDef),
    TypeAlias { name: String, generics: Generics, ty: Type },
    /// Something that has no C++ equivalent; kept as a comment.
    Unsupported(String),
}

#[derive(Debug, Clone)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub alias: Option<String>,
    pub glob: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Generics {
    pub params: Vec<GenericParam>,
    /// Trait bounds: bounded type -> traits.
    pub bounds: Vec<(Type, Vec<TraitBound>)>,
}

#[derive(Debug, Clone)]
pub enum GenericParam {
    Type(String),
    Const(String, Type),
}

#[derive(Debug, Clone)]
pub struct TraitBound {
    pub name: String,
    pub args: Vec<Type>,
    /// `Fn(A, B) -> C` style bounds.
    pub fn_sig: Option<(Vec<Type>, Option<Box<Type>>)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Receiver {
    Ref,
    RefMut,
    Value,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub pat: Pat,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub generics: Generics,
    pub receiver: Option<Receiver>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Option<Block>,
    pub is_const: bool,
    pub is_async: bool,
    pub is_unsafe: bool,
}

#[derive(Debug, Clone)]
pub enum Fields {
    Named(Vec<(String, Type)>),
    Tuple(Vec<Type>),
    Unit,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub generics: Generics,
    pub fields: Fields,
    pub derives: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub fields: Fields,
    pub discriminant: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub generics: Generics,
    pub variants: Vec<Variant>,
    pub derives: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ImplDef {
    pub generics: Generics,
    pub trait_: Option<TraitBound>,
    pub self_ty: Type,
    pub fns: Vec<Function>,
    pub consts: Vec<ConstDef>,
}

#[derive(Debug, Clone)]
pub struct TraitDef {
    pub name: String,
    pub generics: Generics,
    pub supertraits: Vec<TraitBound>,
    pub fns: Vec<Function>,
}

#[derive(Debug, Clone)]
pub struct ModDef {
    pub name: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub struct ConstDef {
    pub name: String,
    pub ty: Type,
    pub value: Option<Expr>,
    pub mutable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Seg {
    pub name: String,
    pub args: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Path(Vec<Seg>),
    Ref { mutable: bool, inner: Box<Type> },
    Ptr { mutable: bool, inner: Box<Type> },
    Tuple(Vec<Type>),
    Unit,
    Array(Box<Type>, Box<Expr>),
    Slice(Box<Type>),
    Fn(Vec<Type>, Option<Box<Type>>),
    ImplTrait(Vec<TraitBoundRef>),
    Dyn(Vec<TraitBoundRef>),
    Never,
    Infer,
    Const(Box<Expr>),
}

/// A bound as it appears inside `impl ...` / `dyn ...` types.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitBoundRef {
    pub name: String,
    pub args: Vec<Type>,
    pub fn_sig: Option<(Vec<Type>, Option<Box<Type>>)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pat {
    Wild,
    Rest,
    Ident { name: String, by_ref: bool, mutable: bool, sub: Option<Box<Pat>> },
    Lit(Box<Expr>),
    Tuple(Vec<Pat>),
    TupleStruct { path: Vec<String>, elems: Vec<Pat> },
    Struct { path: Vec<String>, fields: Vec<(String, Pat)> },
    Path(Vec<String>),
    Ref(Box<Pat>),
    Or(Vec<Pat>),
    Range { lo: Option<Box<Expr>>, hi: Option<Box<Expr>>, inclusive: bool },
    Slice(Vec<Pat>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let { pat: Pat, ty: Option<Type>, init: Option<Expr>, else_block: Option<Block> },
    Expr { expr: Expr, semi: bool },
    Item(Box<Item>),
}

// `Item` contains no PartialEq; give statements a manual escape hatch.
impl PartialEq for Item {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Int(String),
    Float(String),
    Str(String),
    ByteStr(String),
    Char(char),
    Byte(u8),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub pat: Pat,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureParam {
    pub pat: Pat,
    pub ty: Option<Type>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(Lit),
    Path(Vec<Seg>),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Assign(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    MethodCall { recv: Box<Expr>, name: String, targs: Vec<Type>, args: Vec<Expr> },
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    If { cond: Box<Expr>, then: Block, els: Option<Box<Expr>> },
    Match { scrut: Box<Expr>, arms: Vec<Arm> },
    While { label: Option<String>, cond: Box<Expr>, body: Block },
    Loop { label: Option<String>, body: Block },
    For { label: Option<String>, pat: Pat, iter: Box<Expr>, body: Block },
    Block { label: Option<String>, block: Block },
    Closure { params: Vec<ClosureParam>, ret: Option<Type>, body: Box<Expr>, is_move: bool },
    Ref { mutable: bool, expr: Box<Expr> },
    Deref(Box<Expr>),
    Return(Option<Box<Expr>>),
    Break(Option<String>, Option<Box<Expr>>),
    Continue(Option<String>),
    Tuple(Vec<Expr>),
    Array(Vec<Expr>),
    Repeat(Box<Expr>, Box<Expr>),
    Struct { path: Vec<Seg>, fields: Vec<(String, Expr)>, rest: Option<Box<Expr>> },
    Range { lo: Option<Box<Expr>>, hi: Option<Box<Expr>>, inclusive: bool },
    Cast(Box<Expr>, Type),
    Try(Box<Expr>),
    Await(Box<Expr>),
    /// `if let` / `while let` condition.
    Let { pat: Pat, expr: Box<Expr> },
    Macro { name: String, args: Vec<Expr>, repeat: bool },
    Paren(Box<Expr>),
    Unsupported(String),
}
