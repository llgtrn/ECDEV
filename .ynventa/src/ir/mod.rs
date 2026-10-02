//! YIR — the Ynventa Intermediate Representation. Public surfaces of Rust and TypeScript are
//! normalized into one language-neutral form, so that
//!
//! ```text
//! pub fn send(peer: PeerId, frame: Frame) -> Result<Ack, TransportError>   (Rust)
//! export function send(peer: PeerId, frame: Frame): Promise<Ack>            (TypeScript)
//! ```
//!
//! are the same operation `send(PeerId, Frame) -> Ack`, with the same fingerprint. Shards read
//! each other through YIR in capsules, never by grepping each other's source.

use crate::census::sources::{lex, Tok};
use crate::declare::Declaration;
use crate::digest::{hex, Sha256};
use crate::donors::NodeIndex;
use crate::repository::files::Files;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SymbolKind {
    Operation,
    Type,
    Interface,
    Constant,
}

impl SymbolKind {
    pub const ALL: &'static [SymbolKind] = &[
        SymbolKind::Operation,
        SymbolKind::Type,
        SymbolKind::Interface,
        SymbolKind::Constant,
    ];
    pub fn wire(self) -> &'static str {
        match self {
            SymbolKind::Operation => "OPERATION",
            SymbolKind::Type => "TYPE",
            SymbolKind::Interface => "INTERFACE",
            SymbolKind::Constant => "CONSTANT",
        }
    }
    pub fn rank(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Symbol {
    /// Owning node key (Chronica-wide).
    pub node: String,
    pub kind: SymbolKind,
    pub name: String,
    /// Normalized input types (operations only).
    pub inputs: Vec<String>,
    /// Normalized output type (operations only; empty for unit).
    pub output: String,
    pub file: String,
}

impl Symbol {
    /// `<node>.<name>`: the symbol's semantic key.
    pub fn semantic_key(&self) -> String {
        format!("{}.{}", self.node, self.name)
    }
    /// Language-neutral identity of the symbol's shape (not of where it lives).
    pub fn fingerprint(&self) -> String {
        let mut h = Sha256::new();
        h.field(self.kind.wire().as_bytes());
        h.field(self.name.as_bytes());
        for i in &self.inputs {
            h.field(i.as_bytes());
        }
        h.field(b"->");
        h.field(self.output.as_bytes());
        hex(&h.finish())[..16].to_string()
    }
    pub fn signature(&self) -> String {
        match self.kind {
            SymbolKind::Operation => format!(
                "{}({}) -> {}",
                self.name,
                self.inputs.join(", "),
                if self.output.is_empty() {
                    "()"
                } else {
                    &self.output
                }
            ),
            k => format!("{} {}", k.wire().to_ascii_lowercase(), self.name),
        }
    }
}

/// Wrappers that carry no domain meaning: `Result<Ack, E>`, `Promise<Ack>`, `&mut Frame` → the
/// inner type.
const TRANSPARENT: &[&str] = &[
    "Result", "Option", "Box", "Arc", "Rc", "Cow", "Promise", "Future", "impl", "dyn", "mut",
    "const", "Readonly", "Awaited",
];

/// Normalizes a type written as tokens.
pub fn normalize_type(toks: &[Tok]) -> String {
    let idents: Vec<&str> = toks
        .iter()
        .filter_map(|t| match t {
            Tok::Ident(x) => Some(x.as_str()),
            _ => None,
        })
        .filter(|x| !TRANSPARENT.contains(x))
        .collect();
    let array = toks.iter().any(|t| matches!(t, Tok::Punct('[')));
    let base = match idents.first() {
        Some(&"Vec") | Some(&"Array") | Some(&"ReadonlyArray") => {
            return format!("[{}]", idents.get(1).copied().unwrap_or("?"));
        }
        Some(x) => x.to_string(),
        None => String::new(),
    };
    let base = match base.as_str() {
        "String" | "str" | "string" => "text".to_string(),
        "bool" | "boolean" => "bool".to_string(),
        "void" | "undefined" => String::new(),
        "number" | "f64" | "f32" => "number".to_string(),
        _ => base,
    };
    if array && !base.is_empty() {
        format!("[{base}]")
    } else {
        base
    }
}

/// Splits a token run on top-level commas (ignoring those nested in <>, (), [] or {}).
fn split_top(toks: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Punct('<' | '(' | '[' | '{') => depth += 1,
            Tok::Punct('>' | ')' | ']' | '}') => depth -= 1,
            Tok::Punct(',') if depth == 0 => {
                out.push(&toks[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < toks.len() {
        out.push(&toks[start..]);
    }
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

/// Index of the matching closer for the opener at `i`.
fn matching(toks: &[Tok], i: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    for (j, t) in toks.iter().enumerate().skip(i) {
        match t {
            Tok::Punct(c) if *c == open => depth += 1,
            Tok::Punct(c) if *c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
    }
    None
}

fn params(toks: &[Tok]) -> Vec<String> {
    split_top(toks)
        .into_iter()
        .filter_map(|p| {
            let colon = p.iter().position(|t| matches!(t, Tok::Punct(':')))?;
            // `self: Box<Self>` and plain `self` carry no domain input.
            if matches!(p.first(), Some(Tok::Ident(s)) if s == "self") {
                return None;
            }
            // Skip a second ':' of a path (`a: std::x::Y` is fine: first colon splits name/type).
            Some(normalize_type(&p[colon + 1..]))
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// Public symbols of one Rust file.
pub fn rust_symbols(src: &str) -> Vec<(SymbolKind, String, Vec<String>, String)> {
    let toks = lex(src);
    let mut out = Vec::new();
    let ident = |i: usize| match toks.get(i) {
        Some(Tok::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let mut i = 0;
    while i < toks.len() {
        if ident(i) != Some("pub") || matches!(toks.get(i + 1), Some(Tok::Punct('('))) {
            i += 1;
            continue;
        }
        // Qualifiers: `pub const fn`, `pub async unsafe fn`, `pub extern "C" fn`.
        let mut j = i + 1;
        loop {
            match ident(j) {
                Some("async" | "unsafe") => j += 1,
                Some("const") if matches!(ident(j + 1), Some("fn" | "async" | "unsafe")) => j += 1,
                Some("extern") => {
                    j += 1;
                    if matches!(toks.get(j), Some(Tok::Str(_))) {
                        j += 1;
                    }
                }
                _ => break,
            }
        }
        match ident(j) {
            Some("fn") => {
                if let Some(name) = ident(j + 1) {
                    let mut k = j + 2;
                    if matches!(toks.get(k), Some(Tok::Punct('<'))) {
                        k = matching(&toks, k, '<', '>').map_or(k, |e| e + 1);
                    }
                    if let (Some(Tok::Punct('(')), Some(end)) =
                        (toks.get(k), matching(&toks, k, '(', ')'))
                    {
                        let inputs = params(&toks[k + 1..end]);
                        let mut output = String::new();
                        if matches!(toks.get(end + 1), Some(Tok::Punct('-')))
                            && matches!(toks.get(end + 2), Some(Tok::Punct('>')))
                        {
                            let stop = (end + 3..toks.len())
                                .find(|&m| {
                                    matches!(toks.get(m), Some(Tok::Punct('{' | ';')))
                                        || ident(m) == Some("where")
                                })
                                .unwrap_or(toks.len());
                            // `Result<Ack, E>`: the success type is the first argument.
                            let ret = &toks[end + 3..stop];
                            let first = split_top(ret)
                                .first()
                                .map(|s| s.to_vec())
                                .unwrap_or_default();
                            let first = if first.len() < ret.len()
                                && matches!(ret.first(), Some(Tok::Ident(r)) if r == "Result")
                            {
                                let inner_start = ret
                                    .iter()
                                    .position(|t| matches!(t, Tok::Punct('<')))
                                    .map_or(0, |p| p + 1);
                                let inner = &ret[inner_start..];
                                split_top(inner)
                                    .first()
                                    .map(|s| s.to_vec())
                                    .unwrap_or_default()
                            } else {
                                ret.to_vec()
                            };
                            output = normalize_type(&first);
                        }
                        out.push((SymbolKind::Operation, name.to_string(), inputs, output));
                        i = end;
                        continue;
                    }
                }
            }
            Some("struct" | "enum" | "type" | "union") => {
                if let Some(name) = ident(j + 1) {
                    out.push((SymbolKind::Type, name.to_string(), vec![], String::new()));
                }
            }
            Some("trait") => {
                if let Some(name) = ident(j + 1) {
                    out.push((
                        SymbolKind::Interface,
                        name.to_string(),
                        vec![],
                        String::new(),
                    ));
                }
            }
            Some("const" | "static") => {
                if let Some(name) = ident(j + 1) {
                    if name != "fn" {
                        out.push((
                            SymbolKind::Constant,
                            name.to_string(),
                            vec![],
                            String::new(),
                        ));
                    }
                }
            }
            _ => {}
        }
        i = j + 1;
    }
    out
}

/// Exported symbols of one TypeScript file.
pub fn typescript_symbols(src: &str) -> Vec<(SymbolKind, String, Vec<String>, String)> {
    let toks = lex(src);
    let ident = |i: usize| match toks.get(i) {
        Some(Tok::Ident(x)) => Some(x.as_str()),
        _ => None,
    };
    let mut out = Vec::new();
    for i in 0..toks.len() {
        if ident(i) != Some("export") {
            continue;
        }
        let mut j = i + 1;
        while matches!(ident(j), Some("async" | "default" | "declare" | "abstract")) {
            j += 1;
        }
        match ident(j) {
            Some("function") => {
                let Some(name) = ident(j + 1) else { continue };
                let mut k = j + 2;
                if matches!(toks.get(k), Some(Tok::Punct('<'))) {
                    k = matching(&toks, k, '<', '>').map_or(k, |e| e + 1);
                }
                if let (Some(Tok::Punct('(')), Some(end)) =
                    (toks.get(k), matching(&toks, k, '(', ')'))
                {
                    let inputs = params(&toks[k + 1..end]);
                    let mut output = String::new();
                    if matches!(toks.get(end + 1), Some(Tok::Punct(':'))) {
                        let stop = (end + 2..toks.len())
                            .find(|&m| matches!(toks.get(m), Some(Tok::Punct('{' | ';'))))
                            .unwrap_or(toks.len());
                        output = normalize_type(&toks[end + 2..stop]);
                    }
                    out.push((SymbolKind::Operation, name.to_string(), inputs, output));
                }
            }
            Some("class" | "type" | "enum") => {
                if let Some(name) = ident(j + 1) {
                    out.push((SymbolKind::Type, name.to_string(), vec![], String::new()));
                }
            }
            Some("interface") => {
                if let Some(name) = ident(j + 1) {
                    out.push((
                        SymbolKind::Interface,
                        name.to_string(),
                        vec![],
                        String::new(),
                    ));
                }
            }
            Some("const") => {
                if let Some(name) = ident(j + 1) {
                    out.push((
                        SymbolKind::Constant,
                        name.to_string(),
                        vec![],
                        String::new(),
                    ));
                }
            }
            _ => {}
        }
    }
    out
}

/// Compiles the YIR of a shard: every public symbol of every node's production source.
pub fn compile(files: &Files, d: &Declaration) -> Vec<Symbol> {
    let index = NodeIndex::new(d);
    let excluded = crate::census::excluded_roots(d);
    let mut out = Vec::new();
    for f in &files.paths {
        // The installed subsystem is Ynventa's surface, not the shard's: only the template
        // shard compiles it into YIR (every other shard carries an identical copy).
        if (f.starts_with(".ynventa/") && !crate::conformance::is_template(d))
            || crate::census::is_excluded(f, &excluded)
            || f.starts_with(".ynventa/tests")
            || crate::census::scope_of_file(f) == crate::schema::Scope::Test
        {
            continue;
        }
        let rust = f.ends_with(".rs");
        let ts = (f.ends_with(".ts") || f.ends_with(".tsx"))
            && !f.ends_with(".d.ts")
            && !f.contains("node_modules");
        if !(rust || ts) {
            continue;
        }
        let Some(node) = index.owner(f) else { continue };
        let Some(text) = files.read(f) else { continue };
        let syms = if rust {
            rust_symbols(&text)
        } else {
            typescript_symbols(&text)
        };
        for (kind, name, inputs, output) in syms {
            out.push(Symbol {
                node: node.to_string(),
                kind,
                name,
                inputs,
                output,
                file: f.clone(),
            });
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_and_typescript_normalize_to_one_operation() {
        let r = rust_symbols("pub fn send(peer: PeerId, frame: &Frame) -> Result<Ack, TransportError> { todo!() }\nfn private() {}\npub(crate) fn hidden() {}\npub struct PeerId;\npub trait Transport {}\n");
        let t = typescript_symbols("export async function send(peer: PeerId, frame: Frame): Promise<Ack> { }\nexport interface Transport {}\nfunction internal() {}\n");
        let op = |v: &[(SymbolKind, String, Vec<String>, String)]| {
            v.iter()
                .find(|s| s.0 == SymbolKind::Operation)
                .cloned()
                .unwrap()
        };
        assert_eq!(
            op(&r),
            (
                SymbolKind::Operation,
                "send".into(),
                vec!["PeerId".into(), "Frame".into()],
                "Ack".into()
            )
        );
        assert_eq!(op(&r), op(&t));
        let sym = |x: (SymbolKind, String, Vec<String>, String), node: &str| Symbol {
            node: node.into(),
            kind: x.0,
            name: x.1,
            inputs: x.2,
            output: x.3,
            file: String::new(),
        };
        assert_eq!(
            sym(op(&r), "a").fingerprint(),
            sym(op(&t), "b").fingerprint(),
            "same shape, whatever the shard or language"
        );
        assert!(r
            .iter()
            .any(|s| s.0 == SymbolKind::Interface && s.1 == "Transport"));
        assert!(!r.iter().any(|s| s.1 == "private" || s.1 == "hidden"));
        assert_eq!(r.len(), 3);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn methods_drop_self_and_generic_wrappers() {
        let r = rust_symbols("impl X { pub fn get<'a>(&'a self, keys: Vec<Key>) -> Option<&'a Value> { None } pub async fn run(self) {} }");
        assert_eq!(
            r[0],
            (
                SymbolKind::Operation,
                "get".into(),
                vec!["[Key]".into()],
                "Value".into()
            )
        );
        assert_eq!(
            r[1],
            (SymbolKind::Operation, "run".into(), vec![], String::new())
        );
    }
}
