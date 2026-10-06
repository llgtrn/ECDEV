//! JavaScript/TypeScript module observation: the bare specifiers a source file imports
//! (`import … from 'x'`, `import 'x'`, `import('x')`, `export … from 'x'`, `require('x')`),
//! whether or not any `package.json` declares them. Relative paths, `node:` and other
//! scheme-prefixed specifiers, Node built-in modules, workspace packages and `tsconfig` path
//! aliases are not foreign packages.

use crate::formats::{json, Value};
use crate::repository::files::Files;
use std::collections::BTreeSet;

/// Source extensions scanned for module imports.
pub const SOURCE: &[&str] = &[".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx"];

/// Node's built-in modules (importable without the `node:` prefix).
const NODE_BUILTINS: &[&str] = &[
    "assert",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "domain",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "string_decoder",
    "sys",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Punct(char),
    Str(String),
    /// A template literal (its content is never a static specifier).
    Template,
    /// A number or regular expression literal.
    Other,
}

/// Keywords after which a `/` starts a regular expression, not a division.
const REGEX_AFTER: &[&str] = &[
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
];

fn lex(src: &str) -> Vec<Tok> {
    let s: Vec<char> = src.chars().collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    // A hashbang line.
    if s.starts_with(&['#', '!']) {
        while i < s.len() && s[i] != '\n' {
            i += 1;
        }
    }
    let regex_allowed = |out: &[Tok]| match out.last() {
        None => true,
        Some(Tok::Punct(c)) => !matches!(c, ')' | ']' | '}'),
        Some(Tok::Ident(w)) => REGEX_AFTER.contains(&w.as_str()),
        _ => false,
    };
    while i < s.len() {
        let c = s[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && s.get(i + 1) == Some(&'/') {
            while i < s.len() && s[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && s.get(i + 1) == Some(&'*') {
            i += 2;
            while i < s.len() && !(s[i] == '*' && s.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if c == '/' && regex_allowed(&out) {
            // A regular expression literal ends on its line; if it does not, it was a division.
            let mut j = i + 1;
            let mut class = false;
            let mut closed = false;
            while j < s.len() && s[j] != '\n' {
                match s[j] {
                    '\\' => j += 1,
                    '[' => class = true,
                    ']' => class = false,
                    '/' if !class => {
                        closed = true;
                        break;
                    }
                    _ => {}
                }
                j += 1;
            }
            if closed {
                j += 1;
                while j < s.len() && s[j].is_alphabetic() {
                    j += 1;
                }
                out.push(Tok::Other);
                i = j;
            } else {
                out.push(Tok::Punct('/'));
                i += 1;
            }
        } else if c == '\'' || c == '"' {
            // Quoted strings end on their line (a stray apostrophe in JSX text cannot swallow
            // the rest of the file).
            let mut text = String::new();
            i += 1;
            while i < s.len() && s[i] != c && s[i] != '\n' {
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
        } else if c == '`' {
            i = skip_template(&s, i + 1);
            out.push(Tok::Template);
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_' || s[i] == '$') {
                i += 1;
            }
            out.push(Tok::Ident(s[start..i].iter().collect()));
        } else if c.is_ascii_digit() {
            while i < s.len() && (s[i].is_alphanumeric() || s[i] == '_' || s[i] == '.') {
                i += 1;
            }
            out.push(Tok::Other);
        } else {
            out.push(Tok::Punct(c));
            i += 1;
        }
    }
    out
}

/// Skips a template literal body starting after its opening backtick; returns the index after
/// the closing one. `${…}` substitutions are skipped by brace depth.
fn skip_template(s: &[char], mut i: usize) -> usize {
    while i < s.len() {
        match s[i] {
            '\\' => i += 2,
            '`' => return i + 1,
            '$' if s.get(i + 1) == Some(&'{') => {
                let mut depth = 1;
                i += 2;
                while i < s.len() && depth > 0 {
                    match s[i] {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        '`' => {
                            i = skip_template(s, i + 1);
                            continue;
                        }
                        '\'' | '"' => {
                            let q = s[i];
                            i += 1;
                            while i < s.len() && s[i] != q && s[i] != '\n' {
                                if s[i] == '\\' {
                                    i += 1;
                                }
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    i
}

/// Every static module specifier a source file imports, requires or re-exports.
pub fn specifiers(src: &str) -> BTreeSet<String> {
    let toks = lex(src);
    let mut out = BTreeSet::new();
    let ident = |i: usize, w: &str| matches!(toks.get(i), Some(Tok::Ident(x)) if x == w);
    let punct = |i: usize, c: char| matches!(toks.get(i), Some(Tok::Punct(p)) if *p == c);
    let string = |i: usize| match toks.get(i) {
        Some(Tok::Str(s)) => Some(s.clone()),
        _ => None,
    };
    for i in 0..toks.len() {
        // A member access (`x.require(…)`, `y.from 'z'`) is not module syntax.
        if i > 0 && punct(i - 1, '.') && !(i > 1 && punct(i - 2, '.')) {
            continue;
        }
        let call_arg = |at: usize| string(at).filter(|_| punct(at + 1, ')') || punct(at + 1, ','));
        if ident(i, "require") {
            // require('x'), require.resolve('x')
            if punct(i + 1, '(') {
                out.extend(call_arg(i + 2));
            } else if punct(i + 1, '.') && ident(i + 2, "resolve") && punct(i + 3, '(') {
                out.extend(call_arg(i + 4));
            }
        } else if ident(i, "import") {
            if punct(i + 1, '(') {
                out.extend(call_arg(i + 2));
            } else {
                out.extend(string(i + 1));
            }
        } else if ident(i, "from") {
            // `from 'x'` only occurs in import and export declarations.
            out.extend(string(i + 1));
        }
    }
    out
}

/// The npm package a bare specifier names (`@a/b/sub` → `@a/b`, `x/sub` → `x`), or `None` for a
/// relative or absolute path, a scheme-prefixed specifier (`node:fs`, `virtual:x`), a subpath
/// import (`#x`), a Node built-in, or text that cannot be a package name.
pub fn package_of(spec: &str) -> Option<String> {
    let valid = |seg: &str| {
        !seg.is_empty()
            && seg.len() <= 214
            && seg.starts_with(|c: char| c.is_ascii_alphanumeric())
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~'))
    };
    if spec.contains(':') || spec.contains(char::is_whitespace) {
        return None;
    }
    let mut parts = spec.split('/');
    let first = parts.next()?;
    let name = if let Some(scope) = first.strip_prefix('@') {
        let pkg = parts.next()?;
        if !valid(scope) || !valid(pkg) {
            return None;
        }
        format!("@{scope}/{pkg}")
    } else {
        if !valid(first) || NODE_BUILTINS.contains(&first) {
            return None;
        }
        first.to_string()
    };
    Some(name)
}

/// Removes `//` and `/* */` comments and trailing commas from JSONC (`tsconfig.json`).
fn strip_jsonc(text: &str) -> String {
    let s: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c == '"' {
            out.push(c);
            i += 1;
            while i < s.len() && s[i] != '"' {
                if s[i] == '\\' && i + 1 < s.len() {
                    out.push(s[i]);
                    i += 1;
                }
                out.push(s[i]);
                i += 1;
            }
            if i < s.len() {
                out.push('"');
            }
            i += 1;
        } else if c == '/' && s.get(i + 1) == Some(&'/') {
            while i < s.len() && s[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && s.get(i + 1) == Some(&'*') {
            i += 2;
            while i < s.len() && !(s[i] == '*' && s.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if c == ',' {
            let next = s[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}') | Some(']')) {
                out.push(c);
            }
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Specifiers that resolve inside the repository: workspace package names (`name` of every
/// tracked `package.json`) and `tsconfig`/`jsconfig` `paths` aliases and `baseUrl` roots.
#[derive(Debug, Default)]
pub struct Local {
    packages: BTreeSet<String>,
    /// Alias prefixes: an exact key, or the part before `*`.
    aliases: BTreeSet<(String, bool)>,
    /// Directories bare specifiers resolve against (`<config dir>/<baseUrl>`).
    base_dirs: BTreeSet<String>,
}

impl Local {
    pub fn of(files: &Files, skip: impl Fn(&str) -> bool) -> Local {
        let mut l = Local::default();
        for f in files.paths.iter().filter(|f| !skip(f)) {
            let name = f.rsplit('/').next().unwrap_or(f);
            let dir = f.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            if name == "package.json" {
                if let Some(n) = files
                    .read(f)
                    .and_then(|t| json::parse(&t).ok())
                    .and_then(|v| v.str("name").map(str::to_string))
                {
                    l.packages.insert(n);
                }
            } else if (name.starts_with("tsconfig") || name.starts_with("jsconfig"))
                && name.ends_with(".json")
            {
                let Some(v) = files
                    .read(f)
                    .and_then(|t| json::parse(&strip_jsonc(&t)).ok())
                else {
                    continue;
                };
                let Some(opts) = v.get("compilerOptions") else {
                    continue;
                };
                if let Some(Value::Table(paths)) = opts.get("paths") {
                    for key in paths.keys() {
                        match key.split_once('*') {
                            Some((prefix, _)) => l.aliases.insert((prefix.to_string(), true)),
                            None => l.aliases.insert((key.clone(), false)),
                        };
                    }
                }
                if let Some(base) = opts.str("baseUrl") {
                    l.base_dirs.insert(super::cargo::join(dir, base));
                }
            }
        }
        l
    }

    /// Whether `spec` resolves to something in the repository rather than an npm package.
    pub fn resolves(&self, files: &Files, spec: &str, package: &str) -> bool {
        if self.packages.contains(package) {
            return true;
        }
        if self.aliases.iter().any(|(a, wildcard)| {
            if *wildcard {
                spec.starts_with(a.as_str())
            } else {
                spec == a
            }
        }) {
            return true;
        }
        let first = spec.split('/').next().unwrap_or(spec);
        self.base_dirs.iter().any(|b| {
            let p = super::cargo::join(b, first);
            files.exists(&p)
                || SOURCE
                    .iter()
                    .any(|e| files.paths.contains(&format!("{p}{e}")))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packages(src: &str) -> Vec<String> {
        let set: BTreeSet<String> = specifiers(src)
            .iter()
            .filter_map(|s| package_of(s))
            .collect();
        set.into_iter().collect()
    }

    #[test]
    fn finds_bare_imports_of_every_form() {
        let src = r#"#!/usr/bin/env node
            import Database from 'better-sqlite3';
            import { parse } from "yaml"
            import type { Foo } from '@scope/pkg/sub/path';
            import 'side-effect/register';
            export * from 'reexported';
            export { a } from "@other/lib";
            const x = require('lodash/fp');
            const y = await import('dynamic');
            const z = require.resolve('resolved/package.json');
            import fs from 'fs';
            import { join } from 'node:path';
            import p from "path/posix";
            import local from './local.js';
            import up from '../up';
            import abs from '/abs/x';
            import sub from '#internal';
        "#;
        assert_eq!(
            packages(src),
            vec![
                "@other/lib",
                "@scope/pkg",
                "better-sqlite3",
                "dynamic",
                "lodash",
                "reexported",
                "resolved",
                "side-effect",
                "yaml"
            ]
        );
    }

    #[test]
    fn ignores_comments_strings_templates_and_member_calls() {
        let src = r#"
            // import x from 'commented';
            /* const y = require('blocked'); */
            const s = "require('in-a-string')";
            const t = `import('in-a-template') ${require('inside-substitution')}`;
            const u = Array.from('abc');
            obj.require('member');
            const re = /'/g; const after = require('after-regex');
            const ratio = a / b; const q = 'quoted';
            const label = <p>Don't import this</p>;
            const dyn = import(`./${name}.js`);
            const key = { from: 'not-an-import' };
            jest.mock('mocked');
            const msg = require('two words');
        "#;
        // A `require` inside a `${}` substitution is skipped with its template: templates are
        // rare carriers of imports, and their content is not lexed.
        assert_eq!(packages(src), vec!["after-regex"]);
    }

    #[test]
    fn package_names() {
        assert_eq!(package_of("@a/b/c").as_deref(), Some("@a/b"));
        assert_eq!(package_of("x/y/z").as_deref(), Some("x"));
        assert_eq!(package_of("node:fs"), None);
        assert_eq!(package_of("fs/promises"), None);
        assert_eq!(package_of("child_process"), None);
        assert_eq!(package_of("@/components/x"), None);
        assert_eq!(package_of("~/x"), None);
        assert_eq!(package_of("virtual:pwa"), None);
        assert_eq!(package_of("https://cdn/x.js"), None);
        assert_eq!(package_of("$lib/x"), None);
        assert_eq!(package_of("@scope"), None);
    }

    #[test]
    fn tsconfig_is_jsonc() {
        let v = json::parse(&strip_jsonc(
            "{ // c\n \"compilerOptions\": { /* b */ \"paths\": { \"@fi/v1\": [\"x\"], }, \"u\": \"a//b\" }, }",
        ))
        .unwrap();
        assert!(v.at("compilerOptions.paths").is_some());
        assert_eq!(
            v.at("compilerOptions.u").and_then(Value::as_str),
            Some("a//b")
        );
    }
}
