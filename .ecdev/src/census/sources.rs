//! Source observation: a Rust lexer that finds crate-root paths, `extern crate`, native link
//! attributes, build-script link directives, process invocations and programs located on `PATH`;
//! the npm packages JavaScript/TypeScript imports; the programs shell scripts run; plus a scan of
//! executable files for references to held donor source trees.

use super::{is_excluded, js, scope_of_file, shell, Census, Observation, Via};
use crate::declare::Declaration;
use crate::repository::files::Files;
use crate::schema::Ecosystem;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Punct(char),
    Str(String),
}

/// Tokenizes Rust source, dropping comments, and keeping string literal contents.
pub fn lex(src: &str) -> Vec<Tok> {
    let s: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < s.len() {
        let c = s[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && s.get(i + 1) == Some(&'/') {
            while i < s.len() && s[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && s.get(i + 1) == Some(&'*') {
            let mut depth = 1;
            i += 2;
            while i < s.len() && depth > 0 {
                if s[i] == '/' && s.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if s[i] == '*' && s.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if (c == 'r' || c == 'b') && raw_start(&s, i).is_some() {
            let (hashes, body) = raw_start(&s, i).unwrap();
            let mut j = body;
            let mut text = String::new();
            while j < s.len() {
                if s[j] == '"' && (0..hashes).all(|k| s.get(j + 1 + k) == Some(&'#')) {
                    j += 1 + hashes;
                    break;
                }
                text.push(s[j]);
                j += 1;
            }
            out.push(Tok::Str(text));
            i = j;
        } else if c == '"' || (c == 'b' && s.get(i + 1) == Some(&'"')) {
            i += if c == 'b' { 2 } else { 1 };
            let mut text = String::new();
            while i < s.len() && s[i] != '"' {
                if s[i] == '\\' && i + 1 < s.len() {
                    text.push(s[i + 1]);
                    i += 2;
                } else {
                    text.push(s[i]);
                    i += 1;
                }
            }
            i += 1;
            out.push(Tok::Str(text));
        } else if c == '\'' {
            // A char literal ('x', '\n', '\u{..}') or a lifetime ('a).
            if s.get(i + 1) == Some(&'\\') {
                i += 2;
                while i < s.len() && s[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if s.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
                while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_') {
                    i += 1;
                }
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(s[start..i].iter().collect()));
        } else if c.is_ascii_digit() {
            while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_' || s[i] == '.') {
                i += 1;
            }
        } else {
            out.push(Tok::Punct(c));
            i += 1;
        }
    }
    out
}

/// If a raw string (`r"`, `r#"`, `br#"`) starts at `i`, returns (hash count, body start).
fn raw_start(s: &[char], i: usize) -> Option<(usize, usize)> {
    let mut j = i;
    if s.get(j) == Some(&'b') {
        j += 1;
    }
    if s.get(j) != Some(&'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while s.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    (s.get(j) == Some(&'"')).then_some((hashes, j + 1))
}

const NOT_CRATES: &[&str] = &[
    "crate",
    "self",
    "super",
    "Self",
    "std",
    "core",
    "alloc",
    "proc_macro",
    "test",
    // Primitive types: `u128::from(x)` and `usize::MAX` are paths, not crates.
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "f32",
    "f64",
    "bool",
    "char",
    "str",
];

/// Root identifiers of paths (`x::..`, `use x`, `extern crate x`) in one file.
pub fn crate_roots(toks: &[Tok]) -> BTreeSet<String> {
    let mut roots = BTreeSet::new();
    let mut local_mods = BTreeSet::new();
    let is = |i: usize, c: char| matches!(toks.get(i), Some(Tok::Punct(p)) if *p == c);
    for i in 0..toks.len() {
        if let Tok::Ident(x) = &toks[i] {
            if x == "mod" {
                if let Some(Tok::Ident(m)) = toks.get(i + 1) {
                    local_mods.insert(m.clone());
                }
            }
            // A path root: not preceded by `::`, or preceded by a leading (global) `::`.
            let after_colons = i >= 2 && is(i - 1, ':') && is(i - 2, ':');
            let continuation = after_colons
                && i >= 3
                && matches!(
                    toks.get(i - 3),
                    Some(Tok::Ident(_)) | Some(Tok::Punct('>')) | Some(Tok::Punct(')'))
                );
            let path_start = !after_colons || !continuation;
            // `.collect::<T>()` is a turbofish method call, not a path.
            let method = i >= 1 && is(i - 1, '.');
            if path_start && !method && is(i + 1, ':') && is(i + 2, ':') {
                roots.insert(x.clone());
            }
            if (x == "use" || x == "crate") && i + 2 < toks.len() {
                // `use x;` / `extern crate x;` / `extern crate x as y;`
                if let (Some(Tok::Ident(n)), true) = (
                    toks.get(i + 1),
                    is(i + 2, ';') || matches!(toks.get(i + 2), Some(Tok::Ident(a)) if a == "as"),
                ) {
                    if x == "use"
                        || matches!(toks.get(i.wrapping_sub(1)), Some(Tok::Ident(e)) if e == "extern")
                    {
                        roots.insert(n.clone());
                    }
                }
            }
        }
    }
    // Names a `use crate::…` / `use self::…` / `use super::…` (or local module) statement
    // brings into scope shadow crates of the same name.
    let mut local_names = BTreeSet::new();
    let mut i = 0;
    while i < toks.len() {
        if matches!(&toks[i], Tok::Ident(u) if u == "use") {
            let end = (i..toks.len()).find(|j| is(*j, ';')).unwrap_or(toks.len());
            let root = toks[i + 1..end].iter().find_map(|t| match t {
                Tok::Ident(x) => Some(x.as_str()),
                _ => None,
            });
            if root
                .is_some_and(|r| matches!(r, "crate" | "self" | "super") || local_mods.contains(r))
            {
                for j in i + 1..end {
                    if let Tok::Ident(x) = &toks[j] {
                        let next_closes =
                            matches!(toks.get(j + 1), Some(Tok::Punct(',' | '}' | ';')));
                        let renamed =
                            matches!(toks.get(j.wrapping_sub(1)), Some(Tok::Ident(a)) if a == "as");
                        if next_closes || renamed || j + 1 == end {
                            local_names.insert(x.clone());
                        }
                    }
                }
            }
            i = end;
        }
        i += 1;
    }
    roots
        .into_iter()
        .filter(|r| {
            !NOT_CRATES.contains(&r.as_str())
                && !local_mods.contains(r)
                && !local_names.contains(r)
                && r.chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        })
        .collect()
}

/// Functions that locate a program on `PATH` by name (`find_on_path("x", …)`, `which("x")`).
const LOCATORS: &[&str] = &[
    "find_on_path",
    "find_in_path",
    "find_program",
    "find_executable",
    "find_binary",
    "which",
    "which_in",
    "which_global",
];

/// The program locators of one file: the known ones, plus the file's own functions and closures
/// whose body calls one (`let find = |name| find_on_path(name, &path);`).
fn locators(toks: &[Tok]) -> BTreeSet<String> {
    let mut found: BTreeSet<String> = LOCATORS.iter().map(|s| s.to_string()).collect();
    let punct = |i: usize, c: char| matches!(toks.get(i), Some(Tok::Punct(p)) if *p == c);
    let calls = |from: usize, to: usize, set: &BTreeSet<String>| {
        (from..to.min(toks.len())).any(|j| {
            // `locator(name, …)`: the wrapper passes its own argument through.
            matches!(&toks[j], Tok::Ident(x) if set.contains(x))
                && punct(j + 1, '(')
                && matches!(toks.get(j + 2), Some(Tok::Ident(_)))
                && !(j > 0 && punct(j - 1, '.'))
        })
    };
    // Aliases of aliases: iterate to a fixpoint (bounded by the number of definitions).
    loop {
        let mut added = false;
        for i in 0..toks.len() {
            let Tok::Ident(kw) = &toks[i] else { continue };
            let Some(Tok::Ident(name)) = toks.get(i + 1) else {
                continue;
            };
            if found.contains(name) {
                continue;
            }
            let body = if kw == "let" && punct(i + 2, '=') {
                // A closure: `let name = [move] |…| body;`
                let mut j = i + 3;
                if matches!(toks.get(j), Some(Tok::Ident(m)) if m == "move") {
                    j += 1;
                }
                if !punct(j, '|') {
                    continue;
                }
                let end = (j..toks.len())
                    .find(|k| punct(*k, ';'))
                    .unwrap_or(toks.len());
                (j, end)
            } else if kw == "fn" {
                // A function: its body is the first brace block after the signature.
                let Some(open) = (i..toks.len()).find(|k| punct(*k, '{') || punct(*k, ';')) else {
                    continue;
                };
                if !punct(open, '{') {
                    continue;
                }
                let mut depth = 0;
                let mut end = toks.len();
                for (k, t) in toks.iter().enumerate().skip(open) {
                    match t {
                        Tok::Punct('{') => depth += 1,
                        Tok::Punct('}') => {
                            depth -= 1;
                            if depth == 0 {
                                end = k;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                (open, end)
            } else {
                continue;
            };
            if calls(body.0, body.1, &found) {
                found.insert(name.clone());
                added = true;
            }
        }
        if !added {
            return found;
        }
    }
}

/// Whether a literal can name a program (`unshare`, `g++`, `/usr/bin/proj`).
fn program_literal(p: &str) -> Option<String> {
    let program = p.rsplit('/').next().unwrap_or(p);
    let ok = !program.is_empty()
        && program
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'));
    ok.then(|| program.to_string())
}

/// `#[link(name = "x")]` libraries and `Command::new("x")` programs, and programs located on
/// `PATH` by name (`find_on_path("x", …)`, `which("x")` and the file's own wrappers of them).
pub fn links_and_processes(toks: &[Tok]) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut links = BTreeSet::new();
    let mut procs = BTreeSet::new();
    let ident = |i: usize, w: &str| matches!(toks.get(i), Some(Tok::Ident(x)) if x == w);
    let punct = |i: usize, c: char| matches!(toks.get(i), Some(Tok::Punct(p)) if *p == c);
    let locate = locators(toks);
    for i in 0..toks.len() {
        if let Some(Tok::Ident(f)) = toks.get(i) {
            // A located program: `locator("x", …)` / `locator("x")`, not a method (`.find("x")`)
            // and not the locator's own definition.
            if locate.contains(f)
                && punct(i + 1, '(')
                && !(i > 0 && punct(i - 1, '.'))
                && !(i > 0 && ident(i - 1, "fn"))
                && (punct(i + 3, ',') || punct(i + 3, ')'))
            {
                if let Some(p) = toks.get(i + 2).and_then(|t| match t {
                    Tok::Str(s) => program_literal(s),
                    _ => None,
                }) {
                    procs.insert(p);
                }
            }
        }
        if ident(i, "link") && punct(i + 1, '(') && ident(i + 2, "name") && punct(i + 3, '=') {
            if let Some(Tok::Str(n)) = toks.get(i + 4) {
                links.insert(n.clone());
            }
        }
        if ident(i, "Command")
            && punct(i + 1, ':')
            && punct(i + 2, ':')
            && ident(i + 3, "new")
            && punct(i + 4, '(')
        {
            if let Some(Tok::Str(p)) = toks.get(i + 5) {
                let program = p.rsplit('/').next().unwrap_or(p).to_string();
                if !program.is_empty() {
                    procs.insert(program);
                }
            }
        }
        if let Some(Tok::Str(s)) = toks.get(i) {
            for marker in ["rustc-link-lib=", "rustc-link-lib:"] {
                if let Some(rest) = s.split(marker).nth(1) {
                    let lib = rest.rsplit('=').next().unwrap_or(rest).trim();
                    if !lib.is_empty() {
                        links.insert(lib.to_string());
                    }
                }
            }
        }
    }
    (links, procs)
}

const EXECUTABLE: &[&str] = &[
    ".rs",
    ".sh",
    ".bash",
    ".py",
    ".js",
    ".mjs",
    ".cjs",
    ".jsx",
    ".ts",
    ".mts",
    ".cts",
    ".tsx",
    ".c",
    ".h",
    ".cc",
    ".cpp",
    ".cmake",
    ".yml",
    ".yaml",
    ".ps1",
    "Makefile",
    "Justfile",
    "justfile",
    "Dockerfile",
];

/// A shell script: a `.sh`/`.bash` file, or an extensionless file with a `sh`/`bash` shebang.
fn is_shell_script(files: &Files, f: &str) -> bool {
    let name = f.rsplit('/').next().unwrap_or(f);
    if name.ends_with(".sh") || name.ends_with(".bash") {
        return true;
    }
    if name.contains('.') {
        return false;
    }
    use std::io::Read;
    let mut head = [0u8; 128];
    let n = std::fs::File::open(files.root.join(f))
        .and_then(|mut h| h.read(&mut head))
        .unwrap_or(0);
    let head = String::from_utf8_lossy(&head[..n]);
    shell::is_shebang(head.lines().next().unwrap_or(""))
}

/// The repository's own programs: workspace package names, `[[bin]]` targets and `src/bin`
/// binaries. Running them is not a foreign dependency.
fn own_binaries(files: &Files, c: &Census) -> BTreeSet<String> {
    let mut own = BTreeSet::new();
    for m in &c.members {
        own.insert(m.package.clone());
        let manifest = if m.dir.is_empty() {
            "Cargo.toml".to_string()
        } else {
            format!("{}/Cargo.toml", m.dir)
        };
        if let Some(v) = files
            .read(&manifest)
            .and_then(|t| crate::formats::toml::parse(&t).ok())
        {
            for b in v.items_of("bin") {
                if let Some(n) = b.str("name") {
                    own.insert(n.to_string());
                }
            }
        }
        let bin_dir = if m.dir.is_empty() {
            "src/bin".to_string()
        } else {
            format!("{}/src/bin", m.dir)
        };
        for p in files.under(&bin_dir) {
            let rest = &p[bin_dir.len() + 1..];
            let stem = match rest.split_once('/') {
                Some((d, "main.rs")) => d,
                Some(_) => continue,
                None => rest.trim_end_matches(".rs"),
            };
            own.insert(stem.to_string());
        }
    }
    own
}

pub fn observe(files: &Files, d: &Declaration, excluded: &[String], c: &mut Census) {
    let donor_paths: Vec<String> = d
        .donors
        .iter()
        .flat_map(|dn| dn.source_paths.iter())
        .map(|p| p.trim_end_matches('/').to_string())
        .filter(|p| !p.is_empty())
        .collect();
    // Paths whose code is not the repository's own: the subsystem, installed packages, research.
    let foreign = |f: &str| {
        is_excluded(f, excluded)
            || f.starts_with(".ecdev/")
            || f.starts_with("research/")
            || f.split('/').any(|s| s == "node_modules")
    };
    let shell_scripts: BTreeSet<&String> = files
        .paths
        .iter()
        .filter(|f| !foreign(f) && is_shell_script(files, f))
        .collect();
    // Functions of every script are in scope everywhere: libraries are sourced.
    let mut shell_functions = shell::Functions::new();
    let mut shell_texts = Vec::new();
    for f in &shell_scripts {
        if let Some(text) = files.read(f) {
            for (name, h) in shell::functions(&text) {
                let e = shell_functions.entry(name).or_default();
                if *e == shell::Helper::Plain {
                    *e = h;
                }
            }
            shell_texts.push((f.to_string(), text));
        }
    }
    let own = own_binaries(files, c);
    for (f, text) in &shell_texts {
        for p in shell::programs(text, &shell_functions) {
            if own.contains(&p) {
                continue;
            }
            c.observations.insert(Observation {
                file: f.clone(),
                ecosystem: Ecosystem::Native,
                name: p.clone(),
                ident: p,
                scope: scope_of_file(f),
                via: Via::Process,
            });
        }
    }
    let local = js::Local::of(files, foreign);
    for f in files.paths.iter() {
        if is_excluded(f, excluded) {
            continue;
        }
        let exec = EXECUTABLE.iter().any(|e| f.ends_with(e));
        if !exec && !shell_scripts.contains(&f) {
            continue;
        }
        let in_governance = f.starts_with(".ecdev/");
        let Some(text) = files.read(f) else {
            continue;
        };
        if js::SOURCE.iter().any(|e| f.ends_with(e)) && !foreign(f) {
            for spec in js::specifiers(&text) {
                let Some(package) = js::package_of(&spec) else {
                    continue;
                };
                if local.resolves(files, &spec, &package) {
                    continue;
                }
                c.observations.insert(Observation {
                    file: f.clone(),
                    ecosystem: Ecosystem::Npm,
                    name: package.clone(),
                    ident: package,
                    scope: scope_of_file(f),
                    via: Via::Import,
                });
            }
        }
        if f.ends_with(".rs") {
            let toks = lex(&text);
            let roots = crate_roots(&toks);
            if !roots.is_empty() {
                c.imports.insert(f.clone(), roots);
            }
            if !in_governance {
                let (links, procs) = links_and_processes(&toks);
                let scope = scope_of_file(f);
                for l in links {
                    c.observations.insert(Observation {
                        file: f.clone(),
                        ecosystem: Ecosystem::Native,
                        name: l.clone(),
                        ident: l,
                        scope: if scope == crate::schema::Scope::Test {
                            scope
                        } else {
                            crate::schema::Scope::Linked
                        },
                        via: Via::Link,
                    });
                }
                for p in procs {
                    c.observations.insert(Observation {
                        file: f.clone(),
                        ecosystem: Ecosystem::Native,
                        name: p.clone(),
                        ident: p,
                        scope,
                        via: Via::Process,
                    });
                }
            }
        }
        if !in_governance {
            for p in &donor_paths {
                if text.contains(p.as_str()) {
                    c.observations.insert(Observation {
                        file: f.clone(),
                        ecosystem: Ecosystem::Native,
                        name: p.clone(),
                        ident: p.clone(),
                        scope: scope_of_file(f),
                        via: Via::SourceReference,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_crate_roots_not_comments_or_strings() {
        let src = r##"
            // use commented::Out;
            /* nested /* block */ proj::x */
            use serde::Serialize;
            use geo;
            extern crate rstar;
            mod local; use local::thing;
            use crate::formats::{json, toml as t, Value};
            fn h() { json::parse(); t::parse(); }
            fn f() { let s = "tokio::spawn"; let r = r#"hyper::x"#; let c = 'a'; }
            fn g<'a>(x: &'a str) -> std::string::String { crate::m(); NodeKind::Kernel; ::regex::Regex::new(x) }
            fn p(v: &[u64]) -> u128 { let n: u64 = v.iter().sum::<u64>(); u128::from(n) + usize::MAX as u128 }
            fn q(v: &[u8]) -> Vec<u8> { v.iter().copied().collect::<Vec<_>>() }
        "##;
        let roots = crate_roots(&lex(src));
        let got: Vec<&str> = roots.iter().map(String::as_str).collect();
        assert_eq!(got, vec!["geo", "regex", "rstar", "serde"]);
    }

    #[test]
    fn finds_links_and_processes() {
        let src = r#"
            #[link(name = "proj")] extern "C" {}
            fn main() { println!("cargo:rustc-link-lib=static=modbus"); std::process::Command::new("/usr/bin/proj"); }
        "#;
        let (l, p) = links_and_processes(&lex(src));
        assert!(l.contains("proj") && l.contains("modbus"));
        assert!(p.contains("proj"));
    }

    #[test]
    fn finds_programs_located_on_path() {
        let src = r#"
            fn find_on_path(name: &str, path: &str) -> Option<PathBuf> { None }
            fn which(name: &str) -> Option<PathBuf> { None }
            fn probe() { let p = find_on_path("unshare", &path); let d = which("chromedriver"); }
            fn host() {
                let find = |name: &str| find_on_path(name, &path);
                let again = move |n: &str| find(n);
                let (a, b) = (find("mount"), again("ln"));
                let c = which::which("jq");
            }
            fn locate(name: &str) -> Option<PathBuf> { find_on_path(name, "/bin") }
            fn other() { let x = locate("tar"); }
            fn run() { let u = find_on_path("unshare", &p); }
            fn caller() { run("not-a-program"); }
        "#;
        let (_, p) = links_and_processes(&lex(src));
        let got: Vec<&str> = p.iter().map(String::as_str).collect();
        assert_eq!(
            got,
            vec!["chromedriver", "jq", "ln", "mount", "tar", "unshare"]
        );
    }

    #[test]
    fn locators_are_not_string_methods_or_lookalikes() {
        let src = r#"
            fn f(text: &str) {
                let i = text.find("install_requires");
                let find = |k: &str| map.get(k);
                let h = find("content-length");
                let t = tool("unshare", &self.unshare);
                let w = which(name);
                let s = which("two words");
                let r = find_on_path(format!("x{}", 1), &p);
            }
        "#;
        let (_, p) = links_and_processes(&lex(src));
        assert!(p.is_empty(), "{p:?}");
    }
}
