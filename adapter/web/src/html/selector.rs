//! CSS selectors (Selectors Level 4 subset): type, universal, id, class, attribute selectors with
//! all operators and the `i`/`s` flags, compound selectors, the four combinators and selector
//! lists. Pseudo-classes and pseudo-elements are not supported and are rejected at parse time, so
//! an unsupported selector never silently matches nothing. Identifiers and class names always
//! compare case-sensitively (quirks-mode case folding is not applied).

use super::dom::{AttributeNamespace, ElementRef, Namespace};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectorError(pub String);

impl fmt::Display for SelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid selector: {}", self.0)
    }
}
impl std::error::Error for SelectorError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Sensitive,
    Insensitive,
    /// ASCII case-insensitive only for the HTML attributes the HTML Standard lists as such.
    HtmlDefault,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Simple {
    Id(String),
    Class(String),
    Attr {
        name: String,
        op: Op,
        value: String,
        case: Case,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Compound {
    tag: Option<String>,
    simple: Vec<Simple>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Combinator {
    Descendant,
    Child,
    NextSibling,
    LaterSibling,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Complex {
    first: Compound,
    rest: Vec<(Combinator, Compound)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    alternatives: Vec<Complex>,
}

const HTML_CASE_INSENSITIVE_ATTRIBUTES: &[&str] = &[
    "accept",
    "accept-charset",
    "align",
    "alink",
    "axis",
    "bgcolor",
    "charset",
    "checked",
    "clear",
    "codetype",
    "color",
    "compact",
    "declare",
    "defer",
    "dir",
    "direction",
    "disabled",
    "enctype",
    "face",
    "frame",
    "hreflang",
    "http-equiv",
    "lang",
    "language",
    "link",
    "media",
    "method",
    "multiple",
    "nohref",
    "noresize",
    "noshade",
    "nowrap",
    "readonly",
    "rel",
    "rev",
    "rules",
    "scope",
    "scrolling",
    "selected",
    "shape",
    "target",
    "text",
    "type",
    "valign",
    "valuetype",
    "vlink",
];

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}
fn is_ident(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit() || c == '-'
}
fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

impl Parser<'_> {
    fn err<T>(&self, message: &str) -> Result<T, SelectorError> {
        Err(SelectorError(message.to_string()))
    }

    fn skip_ws(&mut self) -> bool {
        let mut any = false;
        while self.chars.peek().copied().is_some_and(is_ws) {
            self.chars.next();
            any = true;
        }
        any
    }

    fn escape(&mut self) -> Result<char, SelectorError> {
        // After the backslash.
        match self.chars.next() {
            None => Ok('\u{fffd}'),
            Some(c) if c.is_ascii_hexdigit() => {
                let mut value = c.to_digit(16).unwrap();
                let mut digits = 1;
                while digits < 6 && self.chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                    value = value * 16 + self.chars.next().unwrap().to_digit(16).unwrap();
                    digits += 1;
                }
                if self.chars.peek().copied().is_some_and(is_ws) {
                    self.chars.next();
                }
                Ok(char::from_u32(value)
                    .filter(|&c| c != '\0')
                    .unwrap_or('\u{fffd}'))
            }
            Some('\n') | Some('\r') | Some('\u{c}') => self.err("escaped newline"),
            Some(c) => Ok(c),
        }
    }

    fn ident(&mut self) -> Result<String, SelectorError> {
        let mut out = String::new();
        if self.chars.peek() == Some(&'-') {
            out.push('-');
            self.chars.next();
        }
        match self.chars.peek().copied() {
            Some(c) if is_ident_start(c) => {}
            Some('\\') => {}
            Some('-') if out == "-" => {}
            _ => return self.err("identifier expected"),
        }
        while let Some(&c) = self.chars.peek() {
            if is_ident(c) {
                out.push(c);
                self.chars.next();
            } else if c == '\\' {
                self.chars.next();
                out.push(self.escape()?);
            } else {
                break;
            }
        }
        Ok(out)
    }

    fn string(&mut self, quote: char) -> Result<String, SelectorError> {
        let mut out = String::new();
        loop {
            match self.chars.next() {
                None => return self.err("unterminated string"),
                Some(c) if c == quote => return Ok(out),
                Some('\\') => match self.chars.peek() {
                    Some('\n') => {
                        self.chars.next();
                    }
                    Some(_) => out.push(self.escape()?),
                    None => {}
                },
                Some('\n') => return self.err("newline in string"),
                Some(c) => out.push(c),
            }
        }
    }

    fn attribute(&mut self) -> Result<Simple, SelectorError> {
        self.skip_ws();
        let name = self.ident()?;
        if self.chars.peek() == Some(&'|') {
            let mut ahead = self.chars.clone();
            ahead.next();
            if ahead.next() != Some('=') {
                return self.err("attribute namespaces are not supported");
            }
        }
        self.skip_ws();
        let op = match self.chars.next() {
            Some(']') => {
                return Ok(Simple::Attr {
                    name,
                    op: Op::Exists,
                    value: String::new(),
                    case: Case::Sensitive,
                });
            }
            Some('=') => Op::Equals,
            Some(c @ ('~' | '|' | '^' | '$' | '*')) => {
                if self.chars.next() != Some('=') {
                    return self.err("= expected");
                }
                match c {
                    '~' => Op::Includes,
                    '|' => Op::DashMatch,
                    '^' => Op::Prefix,
                    '$' => Op::Suffix,
                    _ => Op::Substring,
                }
            }
            _ => return self.err("attribute operator expected"),
        };
        self.skip_ws();
        let value = match self.chars.peek().copied() {
            Some(q @ ('"' | '\'')) => {
                self.chars.next();
                self.string(q)?
            }
            _ => self.ident()?,
        };
        let had_ws = self.skip_ws();
        let mut case = Case::HtmlDefault;
        match self.chars.next() {
            Some(']') => {}
            Some(f @ ('i' | 'I' | 's' | 'S')) if had_ws => {
                case = if f.eq_ignore_ascii_case(&'i') {
                    Case::Insensitive
                } else {
                    Case::Sensitive
                };
                self.skip_ws();
                if self.chars.next() != Some(']') {
                    return self.err("] expected");
                }
            }
            _ => return self.err("] expected"),
        }
        Ok(Simple::Attr {
            name,
            op,
            value,
            case,
        })
    }

    fn compound(&mut self) -> Result<Compound, SelectorError> {
        let mut tag = None;
        let mut simple = Vec::new();
        match self.chars.peek().copied() {
            Some('*') => {
                self.chars.next();
                if self.chars.peek() == Some(&'|') {
                    return self.err("namespaces are not supported");
                }
            }
            Some(c) if is_ident_start(c) || c == '-' || c == '\\' => {
                tag = Some(self.ident()?);
                if self.chars.peek() == Some(&'|') {
                    return self.err("namespaces are not supported");
                }
            }
            _ => {}
        }
        loop {
            match self.chars.peek().copied() {
                Some('#') => {
                    self.chars.next();
                    // An id is a name, which may start with a digit.
                    let mut out = String::new();
                    while let Some(&c) = self.chars.peek() {
                        if is_ident(c) {
                            out.push(c);
                            self.chars.next();
                        } else if c == '\\' {
                            self.chars.next();
                            out.push(self.escape()?);
                        } else {
                            break;
                        }
                    }
                    if out.is_empty() {
                        return self.err("id expected");
                    }
                    simple.push(Simple::Id(out));
                }
                Some('.') => {
                    self.chars.next();
                    simple.push(Simple::Class(self.ident()?));
                }
                Some('[') => {
                    self.chars.next();
                    simple.push(self.attribute()?);
                }
                Some(':') => {
                    return self.err("pseudo-classes and pseudo-elements are not supported");
                }
                _ => break,
            }
        }
        Ok(Compound { tag, simple })
    }
}

impl Selector {
    pub fn parse(text: &str) -> Result<Selector, SelectorError> {
        let mut p = Parser {
            chars: text.chars().peekable(),
        };
        let mut alternatives = Vec::new();
        loop {
            p.skip_ws();
            let start_empty = p.chars.peek().is_none();
            if start_empty {
                return p.err("empty selector");
            }
            let consumed_star = p.chars.peek() == Some(&'*');
            let first = p.compound()?;
            if first.tag.is_none() && first.simple.is_empty() && !consumed_star {
                return p.err("selector expected");
            }
            let mut rest = Vec::new();
            loop {
                let ws = p.skip_ws();
                let combinator = match p.chars.peek().copied() {
                    None | Some(',') => break,
                    Some('>') => {
                        p.chars.next();
                        Combinator::Child
                    }
                    Some('+') => {
                        p.chars.next();
                        Combinator::NextSibling
                    }
                    Some('~') => {
                        p.chars.next();
                        Combinator::LaterSibling
                    }
                    Some(_) if ws => Combinator::Descendant,
                    Some(_) => return p.err("unexpected character"),
                };
                p.skip_ws();
                let consumed_star = p.chars.peek() == Some(&'*');
                let compound = p.compound()?;
                if compound.tag.is_none() && compound.simple.is_empty() && !consumed_star {
                    return p.err("selector expected");
                }
                rest.push((combinator, compound));
            }
            alternatives.push(Complex { first, rest });
            match p.chars.next() {
                None => break,
                Some(',') => continue,
                Some(_) => return p.err("unexpected character"),
            }
        }
        Ok(Selector { alternatives })
    }

    pub fn matches(&self, element: ElementRef<'_>) -> bool {
        self.alternatives
            .iter()
            .any(|c| matches_complex(c, c.rest.len(), element))
    }
}

fn compound_at(c: &Complex, index: usize) -> &Compound {
    if index == 0 {
        &c.first
    } else {
        &c.rest[index - 1].1
    }
}

fn matches_complex(c: &Complex, index: usize, e: ElementRef<'_>) -> bool {
    if !matches_compound(compound_at(c, index), e) {
        return false;
    }
    if index == 0 {
        return true;
    }
    match c.rest[index - 1].0 {
        Combinator::Child => e.parent().is_some_and(|p| matches_complex(c, index - 1, p)),
        Combinator::Descendant => {
            let mut at = e.parent();
            while let Some(p) = at {
                if matches_complex(c, index - 1, p) {
                    return true;
                }
                at = p.parent();
            }
            false
        }
        Combinator::NextSibling => previous_siblings(e)
            .next()
            .is_some_and(|s| matches_complex(c, index - 1, s)),
        Combinator::LaterSibling => previous_siblings(e).any(|s| matches_complex(c, index - 1, s)),
    }
}

/// Preceding sibling elements, nearest first.
fn previous_siblings<'a>(e: ElementRef<'a>) -> impl Iterator<Item = ElementRef<'a>> {
    let mut before: Vec<ElementRef<'a>> = Vec::new();
    if let Some(parent) = e.node().parent() {
        for sibling in parent.children() {
            if sibling.id() == e.id() {
                break;
            }
            if let Some(el) = ElementRef::wrap(sibling) {
                before.push(el);
            }
        }
    }
    before.into_iter().rev()
}

fn matches_compound(c: &Compound, e: ElementRef<'_>) -> bool {
    let el = e.value();
    if let Some(tag) = &c.tag {
        let ok = if el.namespace() == Namespace::Html {
            el.name().eq_ignore_ascii_case(tag)
        } else {
            el.name() == tag
        };
        if !ok {
            return false;
        }
    }
    c.simple.iter().all(|s| match s {
        Simple::Id(id) => el.id() == Some(id.as_str()),
        Simple::Class(class) => el
            .attr("class")
            .is_some_and(|v| v.split(|c: char| is_ws(c)).any(|part| part == class)),
        Simple::Attr {
            name,
            op,
            value,
            case,
        } => {
            let html = el.namespace() == Namespace::Html;
            let found = el.attributes().iter().find(|a| {
                a.namespace == AttributeNamespace::None
                    && if html {
                        a.name.eq_ignore_ascii_case(name)
                    } else {
                        a.name == *name
                    }
            });
            let Some(attr) = found else {
                return false;
            };
            let insensitive = match case {
                Case::Insensitive => true,
                Case::Sensitive => false,
                Case::HtmlDefault => {
                    html && HTML_CASE_INSENSITIVE_ATTRIBUTES
                        .contains(&attr.name.to_ascii_lowercase().as_str())
                }
            };
            attribute_matches(*op, &attr.value, value, insensitive)
        }
    })
}

fn attribute_matches(op: Op, actual: &str, wanted: &str, insensitive: bool) -> bool {
    let (a, w) = if insensitive {
        (actual.to_ascii_lowercase(), wanted.to_ascii_lowercase())
    } else {
        (actual.to_string(), wanted.to_string())
    };
    match op {
        Op::Exists => true,
        Op::Equals => a == w,
        Op::Includes => !w.is_empty() && !w.contains(is_ws) && a.split(is_ws).any(|part| part == w),
        Op::DashMatch => a == w || (a.starts_with(&w) && a[w.len()..].starts_with('-')),
        Op::Prefix => !w.is_empty() && a.starts_with(&w),
        Op::Suffix => !w.is_empty() && a.ends_with(&w),
        Op::Substring => !w.is_empty() && a.contains(&w),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::Html;

    fn ids(html: &Html, selector: &str) -> Vec<String> {
        let s = Selector::parse(selector).unwrap();
        html.select(&s)
            .map(|e| e.value().id().unwrap_or(e.value().name()).to_string())
            .collect()
    }

    const DOC: &str = r#"<div id=a class="x y"><p id=p1 lang=en-GB>one</p><span id=s1></span><p id=p2 data-k=" v w " class=y>two</p>
        <ul id=u><li id=l1><a id=a1 href="https://e.example/x.PDF" type=TEXT/HTML>l</a><li id=l2></ul></div><input id=i1 type=Checkbox><input id=i2 type=text>"#;

    #[test]
    fn simple_selectors_and_combinators() {
        let h = Html::parse_document(DOC);
        assert_eq!(ids(&h, "p"), ["p1", "p2"]);
        assert_eq!(
            ids(&h, "P"),
            ["p1", "p2"],
            "HTML type selectors are case-insensitive"
        );
        assert_eq!(ids(&h, "#s1"), ["s1"]);
        assert_eq!(ids(&h, ".y"), ["a", "p2"]);
        assert_eq!(ids(&h, "div.x.y"), ["a"]);
        assert_eq!(ids(&h, "div > p"), ["p1", "p2"]);
        assert_eq!(ids(&h, "ul > a"), Vec::<String>::new());
        assert_eq!(ids(&h, "div a"), ["a1"]);
        assert_eq!(ids(&h, "p + span"), ["s1"]);
        assert_eq!(ids(&h, "p ~ p"), ["p2"]);
        assert_eq!(ids(&h, "li + li"), ["l2"]);
        assert_eq!(ids(&h, "p, span"), ["p1", "s1", "p2"]);
        assert_eq!(ids(&h, "ul *"), ["l1", "a1", "l2"]);
    }

    #[test]
    fn attribute_selectors() {
        let h = Html::parse_document(DOC);
        assert_eq!(ids(&h, "[lang]"), ["p1"]);
        assert_eq!(ids(&h, "[lang=en-GB]"), ["p1"]);
        assert_eq!(ids(&h, "[lang|=en]"), ["p1"]);
        assert_eq!(ids(&h, "[lang|=e]"), Vec::<String>::new());
        assert_eq!(ids(&h, "[data-k~=v]"), ["p2"]);
        assert_eq!(
            ids(&h, "[data-k~='v w']"),
            Vec::<String>::new(),
            "a word never contains whitespace"
        );
        assert_eq!(ids(&h, "[href^='https://']"), ["a1"]);
        assert_eq!(ids(&h, "[href$='.PDF']"), ["a1"]);
        assert_eq!(
            ids(&h, "[href$='.pdf']"),
            Vec::<String>::new(),
            "href is case-sensitive"
        );
        assert_eq!(ids(&h, "[href$='.pdf' i]"), ["a1"]);
        assert_eq!(ids(&h, "[href*=example]"), ["a1"]);
        assert_eq!(
            ids(&h, "[href^='']"),
            Vec::<String>::new(),
            "an empty value matches nothing"
        );
        assert_eq!(
            ids(&h, "input[type=checkbox]"),
            ["i1"],
            "type is case-insensitive in HTML"
        );
        assert_eq!(ids(&h, "[TYPE=text]"), ["i2"]);
        assert_eq!(ids(&h, "[id='p\\31']"), ["p1"], "hex escape in a string");
        assert_eq!(
            ids(&h, "[id=\"p\\31 \"]"),
            ["p1"],
            "a hex escape swallows one trailing space"
        );
    }

    #[test]
    fn unsupported_and_malformed_selectors_are_rejected() {
        for bad in [
            "",
            " ",
            "p:first-child",
            "p::before",
            ":not(p)",
            "a|b",
            "[a|b=c]",
            "p >",
            "> p",
            "p,",
            ",p",
            "[a",
            "[a=]",
            "p.",
            "#",
            "p !",
            "[a='x]",
            "[a=b c]",
        ] {
            assert!(Selector::parse(bad).is_err(), "{bad:?}");
        }
    }
}
