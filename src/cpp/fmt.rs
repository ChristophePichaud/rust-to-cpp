//! Translation of Rust `format!`-style strings to `std::format` strings.

#[derive(Debug, PartialEq)]
pub enum ArgRef {
    Auto,
    Index(usize),
    Name(String),
}

#[derive(Debug, PartialEq)]
pub enum Piece {
    Lit(String),
    Placeholder(ArgRef, String),
}

pub fn parse(s: &str) -> Vec<Piece> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' && chars.get(i + 1) == Some(&'{') {
            lit.push_str("{{");
            i += 2;
        } else if c == '}' && chars.get(i + 1) == Some(&'}') {
            lit.push_str("}}");
            i += 2;
        } else if c == '{' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '}' {
                j += 1;
            }
            let inner: String = chars[i + 1..j.min(chars.len())].iter().collect();
            let (arg, spec) = match inner.split_once(':') {
                Some((a, s)) => (a.trim().to_string(), s.to_string()),
                None => (inner.trim().to_string(), String::new()),
            };
            let arg = if arg.is_empty() {
                ArgRef::Auto
            } else if let Ok(n) = arg.parse::<usize>() {
                ArgRef::Index(n)
            } else {
                ArgRef::Name(arg)
            };
            if !lit.is_empty() {
                out.push(Piece::Lit(std::mem::take(&mut lit)));
            }
            out.push(Piece::Placeholder(arg, spec));
            i = j + 1;
        } else {
            lit.push(c);
            i += 1;
        }
    }
    if !lit.is_empty() {
        out.push(Piece::Lit(lit));
    }
    out
}

/// `{:?}`/`{:#?}` have no direct counterpart: both map to `{}`.
fn clean_spec(spec: &str) -> String {
    let s = spec.replace("#?", "").replace('?', "");
    s
}

/// Rebuilds a `std::format` string. `names` maps named/inline arguments to
/// their position after the `offset` positional arguments.
pub fn render(pieces: &[Piece], names: &[String], offset: usize, explicit: bool) -> String {
    let mut out = String::new();
    let mut auto = 0usize;
    for p in pieces {
        match p {
            Piece::Lit(l) => out.push_str(l),
            Piece::Placeholder(arg, spec) => {
                let spec = clean_spec(spec);
                let idx = match arg {
                    ArgRef::Auto => {
                        auto += 1;
                        Some(auto - 1)
                    }
                    ArgRef::Index(n) => Some(*n),
                    ArgRef::Name(_) => None,
                };
                let text = match (arg, explicit) {
                    (ArgRef::Name(n), _) => {
                        let pos = names.iter().position(|x| x == n).unwrap_or(0) + offset;
                        format!("{{{}", pos)
                    }
                    (_, true) => format!("{{{}", idx.unwrap()),
                    (ArgRef::Index(n), false) => format!("{{{}", n),
                    _ => "{".to_string(),
                };
                out.push_str(&text);
                if !spec.is_empty() {
                    out.push(':');
                    out.push_str(&spec);
                }
                out.push('}');
            }
        }
    }
    out
}
