//! Shell script observation: the programs a POSIX/Bash script runs. A program is the literal
//! command word of a simple command — the first word of a pipeline segment, of a command
//! substitution, or after a wrapper (`sudo`, `env`, `exec`, `command`, `timeout N`, `nohup`,
//! `nice`) — plus programs probed with `command -v x` / `which x`. Builtins, keywords, variable
//! assignments, functions (defined in any script of the repository, since libraries are
//! sourced), expansions, quoted arguments, comments, heredoc bodies and paths are never programs.
//! Precision over recall: a false program becomes a fake donor.

use std::collections::{BTreeMap, BTreeSet};

/// Shell builtins and reserved words (POSIX and Bash).
const BUILTINS: &[&str] = &[
    ":",
    ".",
    "[",
    "[[",
    "]]",
    "]",
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "caller",
    "cd",
    "command",
    "compgen",
    "complete",
    "compopt",
    "continue",
    "declare",
    "dirs",
    "disown",
    "echo",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "getopts",
    "hash",
    "help",
    "history",
    "jobs",
    "kill",
    "let",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "set",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "wait",
    "time",
    "coproc",
    "if",
    "then",
    "else",
    "elif",
    "fi",
    "case",
    "esac",
    "for",
    "select",
    "while",
    "until",
    "do",
    "done",
    "in",
    "function",
    "{",
    "}",
    "!",
];

/// Directories whose programs are named by their basename (`/usr/bin/env` → `env`).
const SYSTEM_DIRS: &[&str] = &[
    "/bin/",
    "/usr/bin/",
    "/sbin/",
    "/usr/sbin/",
    "/usr/local/bin/",
    "/usr/local/sbin/",
];

/// Whether the first line of a file is a `sh`/`bash`/`dash` shebang.
pub fn is_shebang(first_line: &str) -> bool {
    let Some(rest) = first_line.strip_prefix("#!") else {
        return false;
    };
    let mut words = rest.split_whitespace();
    let Some(mut interp) = words.next() else {
        return false;
    };
    if interp.rsplit('/').next() == Some("env") {
        match words.find(|w| !w.starts_with('-')) {
            Some(w) => interp = w,
            None => return false,
        }
    }
    matches!(interp.rsplit('/').next(), Some("sh" | "bash" | "dash"))
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// A word: its text with quotes removed, and whether it is free of expansions.
    Word { text: String, literal: bool },
    /// A control operator or newline.
    Op(&'static str),
    /// A redirection operator; the next word is its target.
    Redir,
}

struct Lexer<'a> {
    s: &'a [char],
    i: usize,
    toks: Vec<Tok>,
    /// Bodies of command substitutions and process substitutions, scanned as scripts.
    subs: Vec<String>,
    word: String,
    in_word: bool,
    literal: bool,
    heredocs: Vec<(String, bool)>,
}

/// The index of the `)` closing a `(` whose body starts at `i`, skipping quoted text.
fn closing_paren(s: &[char], mut i: usize) -> usize {
    let mut depth = 1;
    while i < s.len() {
        match s[i] {
            '\\' => i += 1,
            '\'' => {
                i += 1;
                while i < s.len() && s[i] != '\'' {
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                while i < s.len() && s[i] != '"' {
                    if s[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    s.len()
}

impl<'a> Lexer<'a> {
    fn flush(&mut self) {
        if self.in_word {
            self.toks.push(Tok::Word {
                text: std::mem::take(&mut self.word),
                literal: self.literal,
            });
        }
        self.in_word = false;
        self.literal = true;
    }

    fn start(&mut self) {
        self.in_word = true;
    }

    /// `$…` at `self.i`: a substitution, arithmetic or parameter expansion. Never literal.
    fn dollar(&mut self) {
        let s = self.s;
        self.start();
        self.literal = false;
        let next = s.get(self.i + 1).copied();
        if next == Some('(') && s.get(self.i + 2) == Some(&'(') {
            let end = closing_paren(s, self.i + 2);
            self.i = (end + 2).min(s.len());
        } else if next == Some('(') {
            let end = closing_paren(s, self.i + 2);
            self.subs
                .push(s[self.i + 2..end.min(s.len())].iter().collect());
            self.i = end + 1;
        } else if next == Some('{') {
            let mut depth = 0;
            while self.i < s.len() {
                match s[self.i] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                self.i += 1;
            }
            self.i += 1;
        } else if next == Some('\'') {
            self.i += 2;
            while self.i < s.len() && s[self.i] != '\'' {
                if s[self.i] == '\\' {
                    self.i += 1;
                }
                self.i += 1;
            }
            self.i += 1;
        } else {
            self.word.push('$');
            self.i += 1;
        }
    }

    fn backtick(&mut self) {
        let s = self.s;
        self.start();
        self.literal = false;
        let mut j = self.i + 1;
        let mut body = String::new();
        while j < s.len() && s[j] != '`' {
            if s[j] == '\\' && j + 1 < s.len() {
                body.push(s[j + 1]);
                j += 2;
            } else {
                body.push(s[j]);
                j += 1;
            }
        }
        self.subs.push(body);
        self.i = j + 1;
    }

    fn run(mut self) -> (Vec<Tok>, Vec<String>) {
        let s = self.s;
        while self.i < s.len() {
            let c = s[self.i];
            let next = s.get(self.i + 1).copied();
            match c {
                '\\' if next == Some('\n') => self.i += 2,
                '\\' => {
                    self.start();
                    if let Some(n) = next {
                        self.word.push(n);
                    }
                    self.i += 2;
                }
                ' ' | '\t' | '\r' => {
                    self.flush();
                    self.i += 1;
                }
                '\n' => {
                    self.flush();
                    self.toks.push(Tok::Op("\n"));
                    self.i += 1;
                    self.skip_heredocs();
                }
                '#' if !self.in_word => {
                    while self.i < s.len() && s[self.i] != '\n' {
                        self.i += 1;
                    }
                }
                '\'' => {
                    self.start();
                    self.i += 1;
                    while self.i < s.len() && s[self.i] != '\'' {
                        self.word.push(s[self.i]);
                        self.i += 1;
                    }
                    self.i += 1;
                }
                '"' => {
                    self.start();
                    self.i += 1;
                    while self.i < s.len() && s[self.i] != '"' {
                        match s[self.i] {
                            '\\' => {
                                if let Some(n) = s.get(self.i + 1) {
                                    self.word.push(*n);
                                }
                                self.i += 2;
                            }
                            '$' => self.dollar(),
                            '`' => self.backtick(),
                            ch => {
                                self.word.push(ch);
                                self.i += 1;
                            }
                        }
                    }
                    self.i += 1;
                }
                '$' => self.dollar(),
                '`' => self.backtick(),
                ';' => {
                    self.flush();
                    if next == Some(';') {
                        self.i += if s.get(self.i + 2) == Some(&'&') {
                            3
                        } else {
                            2
                        };
                        self.toks.push(Tok::Op(";;"));
                    } else if next == Some('&') {
                        self.i += 2;
                        self.toks.push(Tok::Op(";;"));
                    } else {
                        self.i += 1;
                        self.toks.push(Tok::Op(";"));
                    }
                }
                '&' => {
                    self.flush();
                    if next == Some('&') {
                        self.i += 2;
                        self.toks.push(Tok::Op("&&"));
                    } else if next == Some('>') {
                        self.i += if s.get(self.i + 2) == Some(&'>') {
                            3
                        } else {
                            2
                        };
                        self.toks.push(Tok::Redir);
                    } else {
                        self.i += 1;
                        self.toks.push(Tok::Op("&"));
                    }
                }
                '|' => {
                    self.flush();
                    if next == Some('|') {
                        self.i += 2;
                        self.toks.push(Tok::Op("||"));
                    } else {
                        self.i += if next == Some('&') { 2 } else { 1 };
                        self.toks.push(Tok::Op("|"));
                    }
                }
                '(' if self.in_word && self.word.ends_with('=') => {
                    // An array assignment `a=(x y)`: its elements are not commands.
                    let end = closing_paren(s, self.i + 1);
                    self.literal = false;
                    self.i = end + 1;
                }
                '(' if !self.in_word && next == Some('(') => {
                    // An arithmetic command `(( … ))`.
                    let end = closing_paren(s, self.i + 2);
                    self.i = (end + 2).min(s.len());
                }
                '(' => {
                    self.flush();
                    self.toks.push(Tok::Op("("));
                    self.i += 1;
                }
                ')' => {
                    self.flush();
                    self.toks.push(Tok::Op(")"));
                    self.i += 1;
                }
                '<' | '>' if next == Some('(') => {
                    // Process substitution `<(cmd)`.
                    self.start();
                    self.literal = false;
                    let end = closing_paren(s, self.i + 2);
                    self.subs
                        .push(s[self.i + 2..end.min(s.len())].iter().collect());
                    self.i = end + 1;
                }
                '<' | '>' => self.redirection(),
                ch => {
                    self.start();
                    self.word.push(ch);
                    self.i += 1;
                }
            }
        }
        self.flush();
        (self.toks, self.subs)
    }

    fn redirection(&mut self) {
        let s = self.s;
        // A file descriptor number written before the operator (`2>`) belongs to it.
        if self.in_word && self.literal && self.word.chars().all(|c| c.is_ascii_digit()) {
            self.word.clear();
            self.in_word = false;
        }
        self.flush();
        let c = s[self.i];
        let next = s.get(self.i + 1).copied();
        if c == '<' && next == Some('<') {
            if s.get(self.i + 2) == Some(&'<') {
                // A here-string: its word is data.
                self.i += 3;
                self.toks.push(Tok::Redir);
                return;
            }
            self.i += 2;
            let strip = s.get(self.i) == Some(&'-');
            if strip {
                self.i += 1;
            }
            while self.i < s.len() && matches!(s[self.i], ' ' | '\t') {
                self.i += 1;
            }
            let mut delim = String::new();
            while self.i < s.len()
                && !s[self.i].is_whitespace()
                && !matches!(s[self.i], ';' | '|' | '&' | '<' | '>' | ')')
            {
                if !matches!(s[self.i], '\'' | '"' | '\\') {
                    delim.push(s[self.i]);
                }
                self.i += 1;
            }
            if !delim.is_empty() {
                self.heredocs.push((delim, strip));
            }
            return;
        }
        self.i += 1;
        if matches!(s.get(self.i), Some('>' | '&' | '|')) {
            self.i += 1;
        }
        self.toks.push(Tok::Redir);
    }

    /// After a newline, skips the bodies of pending heredocs.
    fn skip_heredocs(&mut self) {
        let s = self.s;
        for (delim, _) in std::mem::take(&mut self.heredocs) {
            loop {
                if self.i >= s.len() {
                    return;
                }
                let start = self.i;
                while self.i < s.len() && s[self.i] != '\n' {
                    self.i += 1;
                }
                let line: String = s[start..self.i].iter().collect();
                self.i += 1;
                if line.trim() == delim {
                    break;
                }
            }
        }
    }
}

fn lex(src: &str) -> (Vec<Tok>, Vec<String>) {
    let s: Vec<char> = src.chars().collect();
    Lexer {
        s: &s,
        i: 0,
        toks: Vec::new(),
        subs: Vec::new(),
        word: String::new(),
        in_word: false,
        literal: true,
        heredocs: Vec::new(),
    }
    .run()
}

/// What a shell function does with its arguments, as far as the census can tell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Helper {
    #[default]
    Plain,
    /// Runs its arguments after the first `n` as a command (`step() { shift; "$@"; }`).
    Runs(usize),
    /// Probes its first argument as a program (`require() { command -v "$1"; }`).
    Probes,
}

/// The functions in scope, by name.
pub type Functions = BTreeMap<String, Helper>;

fn literal_is(t: Option<&Tok>, words: &[&str]) -> bool {
    matches!(t, Some(Tok::Word { text, literal: true }) if words.contains(&text.as_str()))
}

/// The tokens of a function body starting at `i` (after the name and `()`): a `{ … }` group or
/// a `( … )` subshell.
fn body(toks: &[Tok], mut i: usize) -> &[Tok] {
    while toks.get(i) == Some(&Tok::Op("\n")) {
        i += 1;
    }
    let brace = literal_is(toks.get(i), &["{"]);
    if !brace && toks.get(i) != Some(&Tok::Op("(")) {
        return &[];
    }
    let open = |t: &Tok| {
        if brace {
            literal_is(Some(t), &["{"])
        } else {
            *t == Tok::Op("(")
        }
    };
    let close = |t: &Tok| {
        if brace {
            literal_is(Some(t), &["}"])
        } else {
            *t == Tok::Op(")")
        }
    };
    let mut depth = 0;
    for (k, t) in toks.iter().enumerate().skip(i) {
        if open(t) {
            depth += 1;
        } else if close(t) {
            depth -= 1;
            if depth == 0 {
                return &toks[i + 1..k];
            }
        }
    }
    &toks[i + 1..]
}

/// Whether a function body runs `"$@"` as a command (after shifting some arguments away) or
/// probes `"$1"` with `command -v` / `which`.
fn helper(body: &[Tok]) -> Helper {
    let at_command = |k: usize| {
        k == 0
            || matches!(body[k - 1], Tok::Op(_))
            || literal_is(
                body.get(k - 1),
                &[
                    "if", "then", "do", "else", "elif", "while", "until", "!", "{", "exec", "time",
                ],
            )
    };
    let text = |k: usize| match body.get(k) {
        Some(Tok::Word { text, .. }) => Some(text.as_str()),
        _ => None,
    };
    let mut shifts = 0;
    let mut first_arg: BTreeSet<String> = ["$1".to_string()].into();
    for (k, t) in body.iter().enumerate() {
        let Tok::Word { text: w, literal } = t else {
            continue;
        };
        if *literal && w == "shift" && at_command(k) {
            shifts += text(k + 1)
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(1);
        } else if !*literal && w == "$@" && at_command(k) {
            return Helper::Runs(shifts);
        } else if let Some((name, "$1")) = w.split_once('=') {
            if is_assignment(w) {
                first_arg.insert(format!("${name}"));
            }
        } else if *literal && at_command(k) {
            let probe = match w.as_str() {
                "command" if matches!(text(k + 1), Some("-v" | "-V")) => text(k + 2),
                "which" => text(k + 1),
                _ => None,
            };
            if probe.is_some_and(|p| first_arg.contains(p)) {
                return Helper::Probes;
            }
        }
    }
    Helper::Plain
}

/// Functions a script defines (`name() …`, `function name …`) and what each does with its
/// arguments.
pub fn functions(src: &str) -> Functions {
    let mut out = Functions::new();
    let mut pending = vec![src.to_string()];
    while let Some(text) = pending.pop() {
        let (toks, subs) = lex(&text);
        pending.extend(subs);
        for (i, t) in toks.iter().enumerate() {
            let Tok::Word {
                text,
                literal: true,
            } = t
            else {
                continue;
            };
            let parens = |j: usize| {
                toks.get(j) == Some(&Tok::Op("(")) && toks.get(j + 1) == Some(&Tok::Op(")"))
            };
            let (name, at) = if text == "function" {
                let Some(Tok::Word { text: n, .. }) = toks.get(i + 1) else {
                    continue;
                };
                (n, if parens(i + 2) { i + 4 } else { i + 2 })
            } else if parens(i + 1) {
                (text, i + 3)
            } else {
                continue;
            };
            let h = helper(body(&toks, at));
            let e = out.entry(name.clone()).or_default();
            if *e == Helper::Plain {
                *e = h;
            }
        }
    }
    out
}

fn is_assignment(w: &str) -> bool {
    let Some((name, _)) = w.split_once('=') else {
        return false;
    };
    let name = name.trim_end_matches('+');
    let name = match name.split_once('[') {
        Some((n, rest)) if rest.ends_with(']') => n,
        Some(_) => return false,
        None => name,
    };
    name.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The program a literal command word names, if it can be one: a bare name, or a path into a
/// system binary directory. Paths into the repository and script files are not programs.
pub fn program_name(word: &str) -> Option<&str> {
    let name = match SYSTEM_DIRS.iter().find_map(|d| word.strip_prefix(d)) {
        Some(rest) => rest,
        None if word.contains('/') => return None,
        None => word,
    };
    let script = [
        ".sh", ".bash", ".py", ".js", ".mjs", ".cjs", ".ts", ".rb", ".pl",
    ]
    .iter()
    .any(|e| name.ends_with(e));
    let shaped = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
        && name.chars().any(|c| c.is_ascii_alphabetic());
    (shaped && !script && !BUILTINS.contains(&name)).then_some(name)
}

/// Programs the script runs or probes. `functions` are the shell functions in scope.
pub fn programs(src: &str, functions: &Functions) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut pending = vec![src.to_string()];
    while let Some(text) = pending.pop() {
        let (toks, subs) = lex(&text);
        pending.extend(subs);
        commands(&toks, functions, &mut out);
    }
    out
}

fn commands(toks: &[Tok], functions: &Functions, out: &mut BTreeSet<String>) {
    let word = |i: usize| match toks.get(i) {
        Some(Tok::Word { text, literal }) => Some((text.as_str(), *literal)),
        _ => None,
    };
    let record = |w: &str, out: &mut BTreeSet<String>| {
        if let Some(p) = program_name(w) {
            if !functions.contains_key(p) {
                out.insert(p.to_string());
            }
        }
    };
    let mut cmd = true;
    // Open `case` statements; true while reading patterns.
    let mut cases: Vec<bool> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let in_pattern = cases.last() == Some(&true);
        match &toks[i] {
            Tok::Op(op) => {
                if in_pattern {
                    if *op == ")" {
                        *cases.last_mut().unwrap() = false;
                        cmd = true;
                    }
                } else if *op == ";;" {
                    if let Some(c) = cases.last_mut() {
                        *c = true;
                    }
                } else {
                    cmd = *op != ")";
                }
                i += 1;
                continue;
            }
            Tok::Redir => {
                // Its target is a file, not a command.
                i += if word(i + 1).is_some() { 2 } else { 1 };
                continue;
            }
            Tok::Word { .. } => {}
        }
        let (w, literal) = word(i).unwrap();
        if in_pattern {
            if literal && w == "esac" {
                cases.pop();
                cmd = false;
            }
            i += 1;
            continue;
        }
        if !cmd {
            if literal && w == "esac" && !cases.is_empty() {
                cases.pop();
            }
            i += 1;
            continue;
        }
        if literal {
            match w {
                "if" | "then" | "else" | "elif" | "while" | "until" | "do" | "!" | "{" => {
                    i += 1;
                    continue;
                }
                "time" => {
                    i += 1;
                    if word(i).is_some_and(|(o, _)| o == "-p") {
                        i += 1;
                    }
                    continue;
                }
                "fi" | "done" | "}" => {
                    cmd = false;
                    i += 1;
                    continue;
                }
                "esac" => {
                    cases.pop();
                    cmd = false;
                    i += 1;
                    continue;
                }
                "for" | "select" => {
                    while i < toks.len() && word(i).map(|(x, _)| x) != Some("do") {
                        i += 1;
                    }
                    i += 1;
                    continue;
                }
                "case" => {
                    while i < toks.len() && word(i).map(|(x, _)| x) != Some("in") {
                        i += 1;
                    }
                    cases.push(true);
                    i += 1;
                    continue;
                }
                "[[" => {
                    while i < toks.len() && word(i).map(|(x, _)| x) != Some("]]") {
                        i += 1;
                    }
                    cmd = false;
                    i += 1;
                    continue;
                }
                "function" => {
                    i += 2;
                    if toks.get(i) == Some(&Tok::Op("(")) && toks.get(i + 1) == Some(&Tok::Op(")"))
                    {
                        i += 2;
                    }
                    continue;
                }
                _ => {}
            }
        }
        if is_assignment(w) {
            i += 1;
            continue;
        }
        if toks.get(i + 1) == Some(&Tok::Op("(")) && toks.get(i + 2) == Some(&Tok::Op(")")) {
            // A function definition; its body follows.
            i += 3;
            continue;
        }
        cmd = false;
        i += 1;
        if !literal {
            continue;
        }
        // Options of a wrapper: skipped, with the given ones taking an argument.
        let skip_options = |i: &mut usize, with_arg: &[&str]| {
            while let Some((o, _)) = word(*i) {
                if !o.starts_with('-') || o == "-" {
                    break;
                }
                *i += 1;
                if o == "--" {
                    break;
                }
                if with_arg.contains(&o) {
                    *i += 1;
                }
            }
        };
        match w {
            "sudo" => {
                record(w, out);
                skip_options(
                    &mut i,
                    &["-u", "-g", "-h", "-p", "-C", "-D", "-r", "-t", "-U"],
                );
                cmd = true;
            }
            "env" => {
                record(w, out);
                skip_options(&mut i, &["-u", "-C", "-S"]);
                while word(i).is_some_and(|(a, _)| is_assignment(a)) {
                    i += 1;
                }
                cmd = true;
            }
            "exec" => {
                skip_options(&mut i, &["-a"]);
                cmd = true;
            }
            "nohup" => {
                record(w, out);
                cmd = true;
            }
            "nice" => {
                record(w, out);
                skip_options(&mut i, &["-n"]);
                cmd = true;
            }
            "timeout" => {
                record(w, out);
                skip_options(&mut i, &["-s", "-k"]);
                if word(i).is_some() {
                    i += 1;
                }
                cmd = true;
            }
            "command" => match word(i) {
                Some(("-v" | "-V", _)) => {
                    // A probe: `command -v x` locates the program x.
                    i += 1;
                    while let Some((p, true)) = word(i) {
                        record(p, out);
                        i += 1;
                    }
                }
                _ => {
                    skip_options(&mut i, &[]);
                    cmd = true;
                }
            },
            "which" => {
                record(w, out);
                skip_options(&mut i, &[]);
                while let Some((p, true)) = word(i) {
                    record(p, out);
                    i += 1;
                }
            }
            _ => match functions.get(w) {
                None => record(w, out),
                Some(Helper::Plain) => {}
                Some(Helper::Runs(n)) => {
                    // `step "name" cmd …`: the words after the first n are a command.
                    for _ in 0..*n {
                        if word(i).is_some() {
                            i += 1;
                        }
                    }
                    cmd = true;
                }
                Some(Helper::Probes) => {
                    if let Some((p, true)) = word(i) {
                        record(p, out);
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progs(src: &str) -> Vec<String> {
        programs(src, &functions(src)).into_iter().collect()
    }

    #[test]
    fn shebangs() {
        assert!(is_shebang("#!/bin/sh"));
        assert!(is_shebang("#!/bin/bash -eu"));
        assert!(is_shebang("#!/usr/bin/env bash"));
        assert!(is_shebang("#!/usr/bin/env -S bash -e"));
        assert!(!is_shebang("#!/usr/bin/env node"));
        assert!(!is_shebang("#!/usr/bin/python3"));
        assert!(!is_shebang("# just a comment"));
    }

    #[test]
    fn finds_command_words() {
        let src = r#"#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
# jq is mentioned in a comment only
mkdir -p "$out/proj-build"
sqlite=$(command -v sqlite3 || true)
cat > "$sqlite" <<'PY'
#!/usr/bin/env python3
import sqlite3, sys
PY
cmake "$src" -DENABLE_TIFF=OFF >/dev/null 2>&1
make -j"$(nproc)" binproj | tee build.log
FOO=1 BAR=2 curl -sS https://example.com
sudo -u builder apt-get install -y gcc
env -u X LC_ALL=C sort file
timeout 5 nc -z localhost 80
if ! grep -q x file; then echo "run rsync now"; fi
case "$1" in
  start|run) docker compose up ;;
  *) printf '%s\n' "usage" ;;
esac
for f in a b c; do wc -l "$f"; done
while read -r line; do sed -n 1p "$line"; done < input.txt
x=`hostname`
/usr/bin/awk '{print}' file
diff <(jq . a.json) b.json
"#;
        assert_eq!(
            progs(src),
            vec![
                "apt-get", "awk", "cat", "cmake", "curl", "diff", "dirname", "docker", "env",
                "grep", "hostname", "jq", "make", "mkdir", "nc", "nproc", "sed", "sort", "sqlite3",
                "sudo", "tee", "timeout", "wc"
            ]
        );
    }

    #[test]
    fn ignores_builtins_functions_variables_paths_and_arguments() {
        let src = r#"#!/usr/bin/env bash
step() {
  local name=$1; shift
  if "$@" >/tmp/log 2>&1; then echo "PASSED $name"; else tail -3 /tmp/log; fi
}
function helper { return 0; }
step "rustfmt" cargo fmt --all
helper
"$mount" -t tmpfs x
./tools/build.sh --release
tools/gate/push.sh
target/debug/app run
arr=(git curl wget)
(( count++ ))
[[ -n "$x" && -f "$y" ]]
[ -d apps/node_modules ] && echo "npm missing"
echo rsync git curl
printf '%s\n' "$(printf x)"
exit 0
"#;
        // `step` runs its arguments after the first as a command.
        assert_eq!(progs(src), vec!["cargo", "tail"]);
    }

    #[test]
    fn helper_functions_that_run_or_probe_their_arguments() {
        let src = r#"
fail() { echo "$*" >&2; exit 1; }
require_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || fail "missing required command: $cmd"
}
retry() { for i in 1 2 3; do if "$@"; then return 0; fi; sleep 1; done; }
step() { local name=$1; shift; echo "==> $name"; "$@"; }
log() { echo "$@"; }
quiet() { node - "$1" "$@" <<'JS'
console.log(1)
JS
}
require_cmd shasum
retry git fetch origin
step "frontend build" npm run build
log docker is not run
quiet jq
"#;
        let f = functions(src);
        assert_eq!(f.get("require_cmd"), Some(&Helper::Probes));
        assert_eq!(f.get("retry"), Some(&Helper::Runs(0)));
        assert_eq!(f.get("step"), Some(&Helper::Runs(1)));
        assert_eq!(f.get("log"), Some(&Helper::Plain));
        assert_eq!(f.get("quiet"), Some(&Helper::Plain));
        assert_eq!(progs(src), vec!["git", "node", "npm", "shasum", "sleep"]);
    }

    #[test]
    fn functions_of_sourced_libraries_are_known() {
        let lib = "release_fail() {\n  echo \"$*\" >&2; exit 1\n}\n";
        let script = ". ./lib.sh\nrelease_fail oops\ngit tag v1\n";
        let f = functions(lib);
        let got: Vec<String> = programs(script, &f).into_iter().collect();
        assert_eq!(got, vec!["git"]);
    }

    #[test]
    fn program_names() {
        assert_eq!(program_name("/usr/bin/env"), Some("env"));
        assert_eq!(program_name("./x"), None);
        assert_eq!(program_name("tools/x"), None);
        assert_eq!(program_name("run.sh"), None);
        assert_eq!(program_name("echo"), None);
        assert_eq!(program_name("123"), None);
        assert_eq!(program_name("g++"), Some("g++"));
    }
}
