//! A small symbol table collected before generation so that the generator
//! can resolve enum variants, struct field order and local traits.

use crate::ir::*;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub enum VKind {
    Unit,
    Tuple(usize),
    Named(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct EnumInfo {
    /// Generated as a `std::variant` based struct (has payloads or methods).
    pub data: bool,
    pub generic: bool,
    pub variants: Vec<(String, VKind)>,
}

#[derive(Debug, Clone)]
pub struct StructInfo {
    pub fields: Vec<String>,
    pub tuple: bool,
    pub has_bases: bool,
}

#[derive(Debug, Default)]
pub struct Symbols {
    pub structs: HashMap<String, StructInfo>,
    pub enums: HashMap<String, EnumInfo>,
    pub traits: HashMap<String, bool>,
    /// Names of all user-defined methods (never remapped to runtime helpers).
    pub methods: HashSet<String>,
}

impl Symbols {
    pub fn collect(items: &[Item]) -> Symbols {
        let mut s = Symbols::default();
        let mut impl_targets = HashSet::new();
        let mut trait_impls: HashMap<String, bool> = HashMap::new();
        s.walk(items, &mut impl_targets, &mut trait_impls);
        // Trait impls have to be known first to decide on base classes.
        let mut bases = HashSet::new();
        collect_bases(items, &s.traits, &mut bases);
        for (name, st) in s.structs.iter_mut() {
            st.has_bases = bases.contains(name);
        }
        for (name, e) in s.enums.iter_mut() {
            if impl_targets.contains(name) {
                e.data = true;
            }
        }
        s
    }

    fn walk(&mut self, items: &[Item], impls: &mut HashSet<String>, _t: &mut HashMap<String, bool>) {
        for it in items {
            match it {
                Item::Struct(st) => {
                    let (fields, tuple) = match &st.fields {
                        Fields::Named(f) => (f.iter().map(|(n, _)| n.clone()).collect(), false),
                        Fields::Tuple(t) => ((0..t.len()).map(|i| format!("_{i}")).collect(), true),
                        Fields::Unit => (vec![], false),
                    };
                    self.structs.insert(st.name.clone(), StructInfo { fields, tuple, has_bases: false });
                }
                Item::Enum(e) => {
                    let data = e.variants.iter().any(|v| !matches!(v.fields, Fields::Unit));
                    self.enums.insert(
                        e.name.clone(),
                        EnumInfo {
                            data,
                            generic: !e.generics.params.is_empty(),
                            variants: e
                                .variants
                                .iter()
                                .map(|v| {
                                    (
                                        v.name.clone(),
                                        match &v.fields {
                                            Fields::Unit => VKind::Unit,
                                            Fields::Tuple(t) => VKind::Tuple(t.len()),
                                            Fields::Named(n) => VKind::Named(n.iter().map(|(n, _)| n.clone()).collect()),
                                        },
                                    )
                                })
                                .collect(),
                        },
                    );
                }
                Item::Trait(t) => {
                    self.methods.extend(t.fns.iter().map(|f| f.name.clone()));
                    self.traits.insert(t.name.clone(), !t.generics.params.is_empty());
                }
                Item::Impl(i) => {
                    self.methods.extend(i.fns.iter().map(|f| f.name.clone()));
                    if let Some(n) = type_name(&i.self_ty) {
                        impls.insert(n);
                    }
                }
                Item::Mod(m) => self.walk(&m.items, impls, _t),
                _ => {}
            }
        }
    }

    pub fn local_trait(&self, name: &str) -> bool {
        self.traits.contains_key(name)
    }
}

fn collect_bases(items: &[Item], traits: &HashMap<String, bool>, out: &mut HashSet<String>) {
    for it in items {
        match it {
            Item::Impl(i) => {
                if let (Some(t), Some(n)) = (&i.trait_, type_name(&i.self_ty)) {
                    if traits.contains_key(&t.name) {
                        out.insert(n);
                    }
                }
            }
            Item::Mod(m) => collect_bases(&m.items, traits, out),
            _ => {}
        }
    }
}

/// The simple name of a single-segment path type (`Point`, `Stack<T>`).
pub fn type_name(t: &Type) -> Option<String> {
    match t {
        Type::Path(s) if s.len() == 1 => Some(s[0].name.clone()),
        _ => None,
    }
}
