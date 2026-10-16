//! HTML tree construction (WHATWG HTML 13.2.6) over the pull tokenizer.
//!
//! Written from the standard's insertion-mode rules. Scripting is treated as enabled (a `noscript`
//! element holds raw text) and no script is ever executed. Declarative shadow roots and
//! `<template for>` patching are not implemented; those start tags are handled as plain templates.
//! A `template` element's contents are its own children.

use super::dom::*;
use super::foreign::{SVG_ATTRIBUTE_NAMES, SVG_TAG_NAMES};
use super::quirks::QUIRKS_PUBLIC_ID_PREFIXES;
use super::tokenizer::{State, Token, Tokenizer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Marker,
    Element(NodeId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Default,
    ListItem,
    Button,
    Table,
}

type Attrs = Vec<(String, String)>;

/// Names whose presence in the stack of open elements is counted, so the scope checks that
/// would walk the whole stack can answer "absent" at once (deeply nested pages otherwise make
/// each check linear in the depth).
const TRACKED: &[&str] = &[
    "p", "button", "li", "dd", "dt", "table", "td", "th", "tr", "tbody", "thead", "tfoot",
    "caption", "colgroup", "select", "ruby", "form", "template", "nobr", "body", "applet",
    "marquee", "object", "option", "optgroup", "h1", "h2", "h3", "h4", "h5", "h6",
];

fn tracked_code(name: &str) -> u8 {
    TRACKED
        .iter()
        .position(|n| *n == name)
        .map_or(0, |i| i as u8 + 1)
}

/// The stack of open elements with per-name counts of the tracked HTML element names.
struct Open {
    ids: Vec<NodeId>,
    codes: Vec<u8>,
    counts: [u32; 40],
}

impl std::ops::Deref for Open {
    type Target = [NodeId];
    fn deref(&self) -> &[NodeId] {
        &self.ids
    }
}

impl Default for Open {
    fn default() -> Open {
        Open {
            ids: Vec::new(),
            codes: Vec::new(),
            counts: [0; 40],
        }
    }
}

impl Open {
    fn push(&mut self, id: NodeId, code: u8) {
        self.ids.push(id);
        self.codes.push(code);
        self.counts[code as usize] += 1;
    }
    fn pop(&mut self) -> Option<NodeId> {
        let id = self.ids.pop()?;
        let code = self.codes.pop().unwrap();
        self.counts[code as usize] -= 1;
        Some(id)
    }
    fn truncate(&mut self, len: usize) {
        while self.ids.len() > len {
            self.pop();
        }
    }
    fn remove(&mut self, at: usize) {
        self.ids.remove(at);
        let code = self.codes.remove(at);
        self.counts[code as usize] -= 1;
    }
    fn insert(&mut self, at: usize, id: NodeId, code: u8) {
        self.ids.insert(at, id);
        self.codes.insert(at, code);
        self.counts[code as usize] += 1;
    }
    fn set(&mut self, at: usize, id: NodeId, code: u8) {
        self.counts[self.codes[at] as usize] -= 1;
        self.ids[at] = id;
        self.codes[at] = code;
        self.counts[code as usize] += 1;
    }
    fn has(&self, name: &str) -> bool {
        self.counts[tracked_code(name) as usize] > 0
    }
}

fn is_ws(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{c}' | '\r' | ' ')
}

fn boundary(e: &Element, scope: Scope) -> bool {
    match scope {
        Scope::Table => {
            e.namespace == Namespace::Html
                && matches!(e.name.as_str(), "html" | "table" | "template")
        }
        _ => match e.namespace {
            Namespace::Html => {
                matches!(
                    e.name.as_str(),
                    "applet"
                        | "caption"
                        | "html"
                        | "table"
                        | "td"
                        | "th"
                        | "marquee"
                        | "object"
                        | "select"
                        | "template"
                ) || (scope == Scope::ListItem && matches!(e.name.as_str(), "ol" | "ul"))
                    || (scope == Scope::Button && e.name == "button")
            }
            Namespace::MathMl => matches!(
                e.name.as_str(),
                "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"
            ),
            Namespace::Svg => matches!(e.name.as_str(), "foreignObject" | "desc" | "title"),
        },
    }
}

fn is_special(e: &Element) -> bool {
    match e.namespace {
        Namespace::Html => matches!(
            e.name.as_str(),
            "address"
                | "applet"
                | "area"
                | "article"
                | "aside"
                | "base"
                | "basefont"
                | "bgsound"
                | "blockquote"
                | "body"
                | "br"
                | "button"
                | "caption"
                | "center"
                | "col"
                | "colgroup"
                | "dd"
                | "details"
                | "dir"
                | "div"
                | "dl"
                | "dt"
                | "embed"
                | "fieldset"
                | "figcaption"
                | "figure"
                | "footer"
                | "form"
                | "frame"
                | "frameset"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "head"
                | "header"
                | "hgroup"
                | "hr"
                | "html"
                | "iframe"
                | "img"
                | "input"
                | "keygen"
                | "li"
                | "link"
                | "listing"
                | "main"
                | "marquee"
                | "menu"
                | "meta"
                | "nav"
                | "noembed"
                | "noframes"
                | "noscript"
                | "object"
                | "ol"
                | "p"
                | "param"
                | "plaintext"
                | "pre"
                | "script"
                | "search"
                | "section"
                | "select"
                | "source"
                | "style"
                | "summary"
                | "table"
                | "tbody"
                | "td"
                | "template"
                | "textarea"
                | "tfoot"
                | "th"
                | "thead"
                | "title"
                | "tr"
                | "track"
                | "ul"
                | "wbr"
                | "xmp"
        ),
        Namespace::MathMl => matches!(
            e.name.as_str(),
            "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"
        ),
        Namespace::Svg => matches!(e.name.as_str(), "foreignObject" | "desc" | "title"),
    }
}

fn plain(attrs: &[(String, String)]) -> Vec<Attribute> {
    attrs
        .iter()
        .map(|(n, v)| Attribute {
            namespace: AttributeNamespace::None,
            prefix: None,
            name: n.clone(),
            value: v.clone(),
        })
        .collect()
}

/// Attributes of a foreign element with the case fix-ups and namespaced attributes applied.
fn foreign_attributes(attrs: &[(String, String)], ns: Namespace) -> Vec<Attribute> {
    attrs
        .iter()
        .map(|(n, v)| {
            let mut name = n.clone();
            match ns {
                Namespace::MathMl if name == "definitionurl" => name = "definitionURL".into(),
                Namespace::Svg => {
                    if let Some((_, fixed)) =
                        SVG_ATTRIBUTE_NAMES.iter().find(|(from, _)| *from == name)
                    {
                        name = (*fixed).to_string();
                    }
                }
                _ => {}
            }
            let (namespace, prefix, local) = match name.as_str() {
                "xlink:actuate" | "xlink:arcrole" | "xlink:href" | "xlink:role" | "xlink:show"
                | "xlink:title" | "xlink:type" => {
                    (AttributeNamespace::XLink, Some("xlink"), &name[6..])
                }
                "xml:lang" | "xml:space" => (AttributeNamespace::Xml, Some("xml"), &name[4..]),
                "xmlns" => (AttributeNamespace::Xmlns, None, "xmlns"),
                "xmlns:xlink" => (AttributeNamespace::Xmlns, Some("xmlns"), "xlink"),
                _ => (AttributeNamespace::None, None, name.as_str()),
            };
            Attribute {
                namespace,
                prefix: prefix.map(str::to_string),
                name: local.to_string(),
                value: v.clone(),
            }
        })
        .collect()
}

pub struct TreeBuilder {
    tok: Tokenizer,
    html: Html,
    open: Open,
    active: Vec<Entry>,
    mode: Mode,
    original: Mode,
    template_modes: Vec<Mode>,
    head: Option<NodeId>,
    form: Option<NodeId>,
    frameset_ok: bool,
    foster: bool,
    table_text: Vec<char>,
    context: Option<NodeId>,
    skip_lf: bool,
    stopped: bool,
}

impl TreeBuilder {
    pub fn parse_document(input: &str) -> Html {
        let mut b = TreeBuilder::new(input, Html::empty(Node::Document));
        b.run();
        b.html
    }

    /// Parses a fragment as the children of an element named `context_name` in `context_ns`.
    pub fn parse_fragment(input: &str, context_ns: Namespace, context_name: &str) -> Html {
        let mut html = Html::empty(Node::Document);
        let root = html.alloc(Node::Element(Element {
            name: "html".into(),
            namespace: Namespace::Html,
            attributes: Vec::new(),
            template_contents: None,
        }));
        html.append(DOCUMENT, root);
        html.root_element = root;
        let context = html.alloc(Node::Element(Element {
            name: context_name.into(),
            namespace: context_ns,
            attributes: Vec::new(),
            template_contents: None,
        }));
        let mut b = TreeBuilder::new(input, html);
        b.push_open(root);
        b.context = Some(context);
        if context_ns == Namespace::Html {
            match context_name {
                "title" | "textarea" => b.tok.state = State::Rcdata,
                "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
                    b.tok.state = State::Rawtext
                }
                "script" => b.tok.state = State::ScriptData,
                "plaintext" => b.tok.state = State::Plaintext,
                _ => {}
            }
            if context_name == "template" {
                b.template_modes.push(Mode::InTemplate);
            }
            // The form element pointer is the nearest form ancestor of the context; a bare
            // context element has none.
            if context_name == "form" {
                b.form = Some(context);
            }
        }
        b.reset_insertion_mode();
        b.run();
        b.html
    }

    fn new(input: &str, html: Html) -> TreeBuilder {
        TreeBuilder {
            tok: Tokenizer::new(input),
            html,
            open: Open::default(),
            active: Vec::new(),
            mode: Mode::Initial,
            original: Mode::Initial,
            template_modes: Vec::new(),
            head: None,
            form: None,
            frameset_ok: true,
            foster: false,
            table_text: Vec::new(),
            context: None,
            skip_lf: false,
            stopped: false,
        }
    }

    fn run(&mut self) {
        while !self.stopped {
            self.tok.allow_cdata = self
                .adjusted_current()
                .is_some_and(|n| self.el(n).namespace != Namespace::Html);
            let t = self.tok.next_token();
            if std::mem::take(&mut self.skip_lf) && t == Token::Character('\n') {
                continue;
            }
            let eof = t == Token::Eof;
            self.dispatch(t);
            if eof {
                self.stopped = true;
            }
        }
    }

    // ---- small queries -------------------------------------------------------------------------

    fn el(&self, id: NodeId) -> &Element {
        match self.html.node(id) {
            Node::Element(e) => e,
            _ => unreachable!("the stacks hold elements only"),
        }
    }

    fn el_mut(&mut self, id: NodeId) -> &mut Element {
        match self.html.node_mut(id) {
            Node::Element(e) => e,
            _ => unreachable!("the stacks hold elements only"),
        }
    }

    fn is(&self, id: NodeId, name: &str) -> bool {
        let e = self.el(id);
        e.namespace == Namespace::Html && e.name == name
    }

    fn current(&self) -> NodeId {
        *self
            .open
            .last()
            .expect("the stack of open elements is not empty")
    }

    fn current_is(&self, name: &str) -> bool {
        self.open.last().is_some_and(|&n| self.is(n, name))
    }

    fn adjusted_current(&self) -> Option<NodeId> {
        match (self.context, self.open.len()) {
            (Some(c), 1) => Some(c),
            _ => self.open.last().copied(),
        }
    }

    fn has_template(&self) -> bool {
        self.open.has("template")
    }

    fn in_scope(&self, scope: Scope, pred: impl Fn(NodeId, &Element) -> bool) -> bool {
        for &id in self.open.iter().rev() {
            let e = self.el(id);
            if pred(id, e) {
                return true;
            }
            if boundary(e, scope) {
                return false;
            }
        }
        false
    }

    fn has_in_scope(&self, scope: Scope, name: &str) -> bool {
        if tracked_code(name) != 0 && !self.open.has(name) {
            return false;
        }
        self.in_scope(scope, |_, e| {
            e.namespace == Namespace::Html && e.name == name
        })
    }

    fn has_heading_in_scope(&self) -> bool {
        if !["h1", "h2", "h3", "h4", "h5", "h6"]
            .iter()
            .any(|h| self.open.has(h))
        {
            return false;
        }
        self.in_scope(Scope::Default, |_, e| {
            e.namespace == Namespace::Html
                && matches!(e.name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
        })
    }

    // ---- insertion -----------------------------------------------------------------------------

    fn appropriate_place(&self, target: Option<NodeId>) -> (NodeId, Option<NodeId>) {
        let target = target.unwrap_or_else(|| self.current());
        let tabular = matches!(self.html.node(target), Node::Element(e)
            if e.namespace == Namespace::Html
                && matches!(e.name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr"));
        if self.foster && tabular {
            let last_template = self.open.iter().rposition(|&n| self.is(n, "template"));
            let last_table = self.open.iter().rposition(|&n| self.is(n, "table"));
            if let Some(t) = last_template
                && last_table.is_none_or(|tb| t > tb)
            {
                return (self.contents_of(self.open[t]), None);
            }
            let Some(tb) = last_table else {
                return (self.open[0], None);
            };
            let table = self.open[tb];
            return match self.html.parent(table) {
                Some(parent) => (parent, Some(table)),
                None => (self.open[tb - 1], None),
            };
        }
        (self.contents_of(target), None)
    }

    /// The node children of `id` go into: a template's content fragment, otherwise the node itself.
    fn contents_of(&self, id: NodeId) -> NodeId {
        match self.html.node(id) {
            Node::Element(e) => e.template_contents.unwrap_or(id),
            _ => id,
        }
    }

    fn put(&mut self, parent: NodeId, before: Option<NodeId>, node: NodeId) {
        match before {
            Some(b) => self.html.insert_before(parent, node, b),
            None => self.html.append(parent, node),
        }
    }

    fn insert_element(&mut self, ns: Namespace, name: &str, attributes: Vec<Attribute>) -> NodeId {
        let template_contents =
            (ns == Namespace::Html && name == "template").then(|| self.html.alloc(Node::Fragment));
        let id = self.html.alloc(Node::Element(Element {
            name: name.to_string(),
            namespace: ns,
            attributes,
            template_contents,
        }));
        let (parent, before) = self.appropriate_place(None);
        self.put(parent, before, id);
        self.push_open(id);
        id
    }

    fn code_of(&self, id: NodeId) -> u8 {
        let e = self.el(id);
        if e.namespace == Namespace::Html {
            tracked_code(&e.name)
        } else {
            0
        }
    }

    fn push_open(&mut self, id: NodeId) {
        let code = self.code_of(id);
        self.open.push(id, code);
    }

    fn insert_html(&mut self, name: &str, attrs: &[(String, String)]) -> NodeId {
        self.insert_element(Namespace::Html, name, plain(attrs))
    }

    fn insert_char(&mut self, c: char) {
        let (parent, before) = self.appropriate_place(None);
        if parent == DOCUMENT {
            return;
        }
        match before {
            Some(b) => self.html.insert_text_before(parent, b, c),
            None => self.html.append_text(parent, c),
        }
    }

    fn insert_comment(&mut self, data: &str, at: Option<NodeId>) {
        let (parent, before) = match at {
            Some(p) => (p, None),
            None => self.appropriate_place(None),
        };
        let id = self.html.alloc(Node::Comment(data.to_string()));
        self.put(parent, before, id);
    }

    fn insert_pi(&mut self, target: &str, data: &str, at: Option<NodeId>) {
        let (parent, before) = match at {
            Some(p) => (p, None),
            None => self.appropriate_place(None),
        };
        let id = self.html.alloc(Node::ProcessingInstruction {
            target: target.to_string(),
            data: data.to_string(),
        });
        self.put(parent, before, id);
    }

    fn clone_element(&mut self, id: NodeId) -> NodeId {
        let e = self.el(id).clone();
        self.html.alloc(Node::Element(e))
    }

    fn add_missing_attributes(&mut self, id: NodeId, attrs: &[(String, String)]) {
        for (n, v) in attrs {
            let e = self.el_mut(id);
            if !e.attributes.iter().any(|a| &a.name == n) {
                e.attributes.push(Attribute {
                    namespace: AttributeNamespace::None,
                    prefix: None,
                    name: n.clone(),
                    value: v.clone(),
                });
            }
        }
    }

    // ---- stack helpers -------------------------------------------------------------------------

    fn pop_until(&mut self, pred: impl Fn(&Element) -> bool) {
        while let Some(id) = self.open.pop() {
            if pred(self.el(id)) {
                break;
            }
        }
    }

    fn pop_until_html(&mut self, name: &str) {
        self.pop_until(|e| e.namespace == Namespace::Html && e.name == name);
    }

    fn implied_end_tags(&mut self, except: Option<&str>, thorough: bool) {
        while let Some(&id) = self.open.last() {
            let e = self.el(id);
            let implied = e.namespace == Namespace::Html
                && (matches!(
                    e.name.as_str(),
                    "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc"
                ) || (thorough
                    && matches!(
                        e.name.as_str(),
                        "caption" | "colgroup" | "tbody" | "td" | "tfoot" | "th" | "thead" | "tr"
                    )));
            if !implied || except.is_some_and(|x| e.name == x) {
                break;
            }
            self.open.pop();
        }
    }

    fn context_is_select(&self) -> bool {
        self.context.is_some_and(|c| self.is(c, "select"))
    }

    fn implied_end_tags_except_optgroup(&mut self) {
        while let Some(&id) = self.open.last() {
            let e = self.el(id);
            let implied = e.namespace == Namespace::Html
                && matches!(
                    e.name.as_str(),
                    "dd" | "dt" | "li" | "option" | "p" | "rb" | "rp" | "rt" | "rtc"
                );
            if !implied {
                break;
            }
            self.open.pop();
        }
    }

    fn close_p(&mut self) {
        self.implied_end_tags(Some("p"), false);
        self.pop_until_html("p");
    }

    fn close_p_if_in_button_scope(&mut self) {
        if self.has_in_scope(Scope::Button, "p") {
            self.close_p();
        }
    }

    fn clear_back_to(&mut self, names: &[&str]) {
        while let Some(&id) = self.open.last() {
            let e = self.el(id);
            if e.namespace == Namespace::Html
                && (names.contains(&e.name.as_str()) || e.name == "template" || e.name == "html")
            {
                break;
            }
            self.open.pop();
        }
    }

    // ---- active formatting elements ------------------------------------------------------------

    fn push_active(&mut self, id: NodeId) {
        let mut same = Vec::new();
        for (i, entry) in self.active.iter().enumerate().rev() {
            match entry {
                Entry::Marker => break,
                Entry::Element(other) => {
                    let (a, b) = (self.el(id), self.el(*other));
                    if a.name == b.name
                        && a.namespace == b.namespace
                        && same_attributes(&a.attributes, &b.attributes)
                    {
                        same.push(i);
                    }
                }
            }
        }
        if same.len() >= 3 {
            // `same` runs from the end of the list; its last entry is the earliest.
            self.active.remove(*same.last().unwrap());
        }
        self.active.push(Entry::Element(id));
    }

    fn reconstruct_active(&mut self) {
        let Some(&last) = self.active.last() else {
            return;
        };
        match last {
            Entry::Marker => return,
            Entry::Element(id) if self.open.contains(&id) => return,
            _ => {}
        }
        let mut idx = self.active.len() - 1;
        while idx > 0 {
            match self.active[idx - 1] {
                Entry::Marker => break,
                Entry::Element(id) if self.open.contains(&id) => break,
                _ => idx -= 1,
            }
        }
        for i in idx..self.active.len() {
            let Entry::Element(old) = self.active[i] else {
                continue;
            };
            let copy = self.clone_element(old);
            let (parent, before) = self.appropriate_place(None);
            self.put(parent, before, copy);
            self.push_open(copy);
            self.active[i] = Entry::Element(copy);
        }
    }

    fn clear_active_to_marker(&mut self) {
        while let Some(entry) = self.active.pop() {
            if entry == Entry::Marker {
                break;
            }
        }
    }

    fn active_position(&self, id: NodeId) -> Option<usize> {
        self.active.iter().position(|e| *e == Entry::Element(id))
    }

    fn adoption_agency(&mut self, subject: &str) {
        let current = self.current();
        if self.is(current, subject) && self.active_position(current).is_none() {
            self.open.pop();
            return;
        }
        for _ in 0..8 {
            let formatting = self
                .active
                .iter()
                .rev()
                .take_while(|e| **e != Entry::Marker)
                .find_map(|e| match e {
                    Entry::Element(id) if self.is(*id, subject) => Some(*id),
                    _ => None,
                });
            let Some(fe) = formatting else {
                self.any_other_end_tag(subject);
                return;
            };
            let Some(fe_pos) = self.open.iter().position(|&n| n == fe) else {
                let at = self.active_position(fe).unwrap();
                self.active.remove(at);
                return;
            };
            if !self.in_scope(Scope::Default, |id, _| id == fe) {
                return;
            }
            let furthest = self.open[fe_pos + 1..]
                .iter()
                .position(|&n| is_special(self.el(n)))
                .map(|p| p + fe_pos + 1);
            let Some(fb_pos) = furthest else {
                self.open.truncate(fe_pos);
                let at = self.active_position(fe).unwrap();
                self.active.remove(at);
                return;
            };
            let fb = self.open[fb_pos];
            let common_ancestor = self.open[fe_pos - 1];
            let mut bookmark = self.active_position(fe).unwrap();
            let mut node_pos = fb_pos;
            let mut last_node = fb;
            let mut inner = 0;
            loop {
                inner += 1;
                node_pos -= 1;
                let node = self.open[node_pos];
                if node == fe {
                    break;
                }
                if inner > 3
                    && let Some(at) = self.active_position(node)
                {
                    self.active.remove(at);
                    if at < bookmark {
                        bookmark -= 1;
                    }
                }
                let Some(at) = self.active_position(node) else {
                    self.open.remove(node_pos);
                    continue;
                };
                let fresh = self.clone_element(node);
                self.active[at] = Entry::Element(fresh);
                let code = self.code_of(fresh);
                self.open.set(node_pos, fresh, code);
                if last_node == fb {
                    bookmark = at + 1;
                }
                self.html.append(fresh, last_node);
                last_node = fresh;
            }
            let (target, before) = self.appropriate_place(Some(common_ancestor));
            self.html.detach(last_node);
            self.put(target, before, last_node);
            let replacement = self.clone_element(fe);
            let moved: Vec<NodeId> = self.html.children(fb).to_vec();
            for child in moved {
                self.html.append(replacement, child);
            }
            self.html.append(fb, replacement);
            let at = self.active_position(fe).unwrap();
            self.active.remove(at);
            if at < bookmark {
                bookmark -= 1;
            }
            self.active.insert(bookmark, Entry::Element(replacement));
            let at = self.open.iter().position(|&n| n == fe).unwrap();
            self.open.remove(at);
            let fb_at = self.open.iter().position(|&n| n == fb).unwrap();
            let code = self.code_of(replacement);
            self.open.insert(fb_at + 1, replacement, code);
        }
    }

    fn any_other_end_tag(&mut self, name: &str) {
        let mut i = self.open.len();
        while i > 0 {
            i -= 1;
            let node = self.open[i];
            if self.is(node, name) {
                self.implied_end_tags(Some(name), false);
                self.open.truncate(i);
                return;
            }
            if is_special(self.el(node)) {
                return;
            }
        }
    }

    // ---- insertion mode reset ------------------------------------------------------------------

    fn reset_insertion_mode(&mut self) {
        for i in (0..self.open.len()).rev() {
            let last = i == 0;
            let node = if last && let Some(c) = self.context {
                c
            } else {
                self.open[i]
            };
            let e = self.el(node);
            if e.namespace != Namespace::Html {
                if last {
                    self.mode = Mode::InBody;
                    return;
                }
                continue;
            }
            let mode = match e.name.as_str() {
                "td" | "th" if !last => Some(Mode::InCell),
                "tr" => Some(Mode::InRow),
                "tbody" | "thead" | "tfoot" => Some(Mode::InTableBody),
                "caption" => Some(Mode::InCaption),
                "colgroup" => Some(Mode::InColumnGroup),
                "table" => Some(Mode::InTable),
                "template" => self.template_modes.last().copied(),
                "head" if !last => Some(Mode::InHead),
                "body" => Some(Mode::InBody),
                "frameset" => Some(Mode::InFrameset),
                "html" => Some(if self.head.is_none() {
                    Mode::BeforeHead
                } else {
                    Mode::AfterHead
                }),
                _ => None,
            };
            if let Some(m) = mode {
                self.mode = m;
                return;
            }
            if last {
                self.mode = Mode::InBody;
                return;
            }
        }
    }

    // ---- dispatch ------------------------------------------------------------------------------

    fn dispatch(&mut self, t: Token) {
        let html_content = match self.adjusted_current() {
            None => true,
            Some(n) => {
                let e = self.el(n);
                if e.namespace == Namespace::Html {
                    true
                } else {
                    let start = matches!(&t, Token::StartTag { .. });
                    let chars = matches!(&t, Token::Character(_));
                    let mathml_text = e.namespace == Namespace::MathMl
                        && matches!(e.name.as_str(), "mi" | "mo" | "mn" | "ms" | "mtext");
                    let html_integration = self.html_integration_point(e);
                    mathml_text
                        && (chars
                            || matches!(&t, Token::StartTag { name, .. } if name != "mglyph" && name != "malignmark"))
                        || (e.namespace == Namespace::MathMl
                            && e.name == "annotation-xml"
                            && matches!(&t, Token::StartTag { name, .. } if name == "svg"))
                        || (html_integration && start)
                        || (html_integration && chars)
                        || t == Token::Eof
                }
            }
        };
        if html_content {
            let mode = self.mode;
            self.handle(mode, t);
        } else {
            self.foreign(t);
        }
    }

    fn html_integration_point(&self, e: &Element) -> bool {
        match e.namespace {
            Namespace::MathMl => {
                e.name == "annotation-xml"
                    && e.attr("encoding").is_some_and(|v| {
                        v.eq_ignore_ascii_case("text/html")
                            || v.eq_ignore_ascii_case("application/xhtml+xml")
                    })
            }
            Namespace::Svg => matches!(e.name.as_str(), "foreignObject" | "desc" | "title"),
            Namespace::Html => false,
        }
    }

    fn handle(&mut self, mode: Mode, t: Token) {
        match mode {
            Mode::Initial => self.initial(t),
            Mode::BeforeHtml => self.before_html(t),
            Mode::BeforeHead => self.before_head(t),
            Mode::InHead => self.in_head(t),
            Mode::AfterHead => self.after_head(t),
            Mode::InBody => self.in_body(t),
            Mode::Text => self.text(t),
            Mode::InTable => self.in_table(t),
            Mode::InTableText => self.in_table_text(t),
            Mode::InCaption => self.in_caption(t),
            Mode::InColumnGroup => self.in_column_group(t),
            Mode::InTableBody => self.in_table_body(t),
            Mode::InRow => self.in_row(t),
            Mode::InCell => self.in_cell(t),
            Mode::InTemplate => self.in_template(t),
            Mode::AfterBody => self.after_body(t),
            Mode::InFrameset => self.in_frameset(t),
            Mode::AfterFrameset => self.after_frameset(t),
            Mode::AfterAfterBody => self.after_after_body(t),
            Mode::AfterAfterFrameset => self.after_after_frameset(t),
        }
    }

    fn process(&mut self, t: Token) {
        let mode = self.mode;
        self.handle(mode, t);
    }

    // ---- insertion modes -----------------------------------------------------------------------

    fn initial(&mut self, t: Token) {
        match &t {
            Token::Character(c) if is_ws(*c) => {}
            Token::Comment(d) => self.insert_comment(d, Some(DOCUMENT)),
            Token::ProcessingInstruction { target, data } => {
                self.insert_pi(target, data, Some(DOCUMENT))
            }
            Token::Doctype {
                name,
                public_id,
                system_id,
                force_quirks,
            } => {
                let id = self.html.alloc(Node::Doctype {
                    name: name.clone().unwrap_or_default(),
                    public_id: public_id.clone().unwrap_or_default(),
                    system_id: system_id.clone().unwrap_or_default(),
                });
                self.html.append(DOCUMENT, id);
                self.html.quirks = quirks_mode(
                    name.as_deref(),
                    public_id.as_deref(),
                    system_id.as_deref(),
                    *force_quirks,
                );
                self.mode = Mode::BeforeHtml;
            }
            _ => {
                self.html.quirks = QuirksMode::Quirks;
                self.mode = Mode::BeforeHtml;
                self.process(t);
            }
        }
    }

    fn before_html(&mut self, t: Token) {
        match &t {
            Token::Doctype { .. } => {}
            Token::Comment(d) => self.insert_comment(d, Some(DOCUMENT)),
            Token::ProcessingInstruction { target, data } => {
                self.insert_pi(target, data, Some(DOCUMENT))
            }
            Token::Character(c) if is_ws(*c) => {}
            Token::StartTag { name, attrs, .. } if name == "html" => {
                let id = self.html.alloc(Node::Element(Element {
                    name: "html".into(),
                    namespace: Namespace::Html,
                    attributes: plain(attrs),
                    template_contents: None,
                }));
                self.html.append(DOCUMENT, id);
                self.html.root_element = id;
                self.push_open(id);
                self.mode = Mode::BeforeHead;
            }
            Token::EndTag { name, .. }
                if !matches!(name.as_str(), "head" | "body" | "html" | "br") => {}
            _ => {
                let id = self.html.alloc(Node::Element(Element {
                    name: "html".into(),
                    namespace: Namespace::Html,
                    attributes: Vec::new(),
                    template_contents: None,
                }));
                self.html.append(DOCUMENT, id);
                self.html.root_element = id;
                self.push_open(id);
                self.mode = Mode::BeforeHead;
                self.process(t);
            }
        }
    }

    fn before_head(&mut self, t: Token) {
        match &t {
            Token::Character(c) if is_ws(*c) => {}
            Token::Comment(d) => self.insert_comment(d, None),
            Token::ProcessingInstruction { target, data } => self.insert_pi(target, data, None),
            Token::Doctype { .. } => {}
            Token::StartTag { name, .. } if name == "html" => self.in_body(t),
            Token::StartTag { name, attrs, .. } if name == "head" => {
                let id = self.insert_html("head", attrs);
                self.head = Some(id);
                self.mode = Mode::InHead;
            }
            Token::EndTag { name, .. }
                if !matches!(name.as_str(), "head" | "body" | "html" | "br") => {}
            _ => {
                let id = self.insert_html("head", &[]);
                self.head = Some(id);
                self.mode = Mode::InHead;
                self.process(t);
            }
        }
    }

    fn generic_text_element(&mut self, name: &str, attrs: &Attrs, state: State) {
        self.insert_html(name, attrs);
        self.tok.state = state;
        self.original = self.mode;
        self.mode = Mode::Text;
    }

    fn in_head(&mut self, t: Token) {
        match &t {
            Token::Character(c) if is_ws(*c) => self.insert_char(*c),
            Token::Comment(d) => self.insert_comment(d, None),
            Token::ProcessingInstruction { target, data } => self.insert_pi(target, data, None),
            Token::Doctype { .. } => {}
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "html" => self.in_body(t),
                "base" | "basefont" | "bgsound" | "link" | "meta" => {
                    self.insert_html(name, attrs);
                    self.open.pop();
                }
                "title" => self.generic_text_element(name, attrs, State::Rcdata),
                "noscript" | "noframes" | "style" => {
                    self.generic_text_element(name, attrs, State::Rawtext)
                }
                "script" => self.generic_text_element(name, attrs, State::ScriptData),
                "template" => {
                    self.insert_html(name, attrs);
                    self.active.push(Entry::Marker);
                    self.frameset_ok = false;
                    self.mode = Mode::InTemplate;
                    self.template_modes.push(Mode::InTemplate);
                }
                "head" => {}
                _ => self.in_head_anything_else(t),
            },
            Token::EndTag { name, .. } => match name.as_str() {
                "head" => {
                    self.open.pop();
                    self.mode = Mode::AfterHead;
                }
                "template" => {
                    if !self.has_template() {
                        return;
                    }
                    self.implied_end_tags(None, true);
                    self.pop_until_html("template");
                    self.clear_active_to_marker();
                    self.template_modes.pop();
                    self.reset_insertion_mode();
                }
                "body" | "html" | "br" => self.in_head_anything_else(t),
                _ => {}
            },
            _ => self.in_head_anything_else(t),
        }
    }

    fn in_head_anything_else(&mut self, t: Token) {
        self.open.pop();
        self.mode = Mode::AfterHead;
        self.process(t);
    }

    fn after_head(&mut self, t: Token) {
        match &t {
            Token::Character(c) if is_ws(*c) => self.insert_char(*c),
            Token::Comment(d) => self.insert_comment(d, None),
            Token::ProcessingInstruction { target, data } => self.insert_pi(target, data, None),
            Token::Doctype { .. } => {}
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "html" => self.in_body(t),
                "body" => {
                    self.insert_html("body", attrs);
                    self.frameset_ok = false;
                    self.mode = Mode::InBody;
                }
                "frameset" => {
                    self.insert_html("frameset", attrs);
                    self.mode = Mode::InFrameset;
                }
                "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script"
                | "style" | "template" | "title" => {
                    let head = self
                        .head
                        .expect("the head element pointer is set after the head");
                    self.push_open(head);
                    self.in_head(t);
                    if let Some(at) = self.open.iter().position(|&n| n == head) {
                        self.open.remove(at);
                    }
                }
                "head" => {}
                _ => self.after_head_anything_else(t),
            },
            Token::EndTag { name, .. } => match name.as_str() {
                "template" => self.in_head(t),
                "body" | "html" | "br" => self.after_head_anything_else(t),
                _ => {}
            },
            _ => self.after_head_anything_else(t),
        }
    }

    fn after_head_anything_else(&mut self, t: Token) {
        self.insert_html("body", &[]);
        self.frameset_ok = true;
        self.mode = Mode::InBody;
        self.process(t);
    }

    fn body_end_checks(&self) -> bool {
        self.has_in_scope(Scope::Default, "body")
    }

    fn in_body(&mut self, t: Token) {
        match &t {
            Token::Character('\0') => {}
            Token::Character(c) => {
                self.reconstruct_active();
                self.insert_char(*c);
                if !is_ws(*c) {
                    self.frameset_ok = false;
                }
            }
            Token::Comment(d) => self.insert_comment(d, None),
            Token::ProcessingInstruction { target, data } => self.insert_pi(target, data, None),
            Token::Doctype { .. } => {}
            Token::Eof => {
                if !self.template_modes.is_empty() {
                    self.in_template(t);
                }
            }
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => self.in_body_start(t.clone(), name, attrs, *self_closing),
            Token::EndTag { name, .. } => self.in_body_end(t.clone(), name),
        }
    }

    fn in_body_start(&mut self, t: Token, name: &str, attrs: &Attrs, self_closing: bool) {
        match name {
            "html" => {
                if !self.has_template() {
                    let root = self.open[0];
                    self.add_missing_attributes(root, attrs);
                }
            }
            "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script" | "style"
            | "template" | "title" => self.in_head(t),
            "body" => {
                if self.open.len() == 1 || !self.is(self.open[1], "body") || self.has_template() {
                    return;
                }
                self.frameset_ok = false;
                let body = self.open[1];
                self.add_missing_attributes(body, attrs);
            }
            "frameset" => {
                if self.open.len() == 1 || !self.is(self.open[1], "body") || !self.frameset_ok {
                    return;
                }
                let body = self.open[1];
                self.html.detach(body);
                self.open.truncate(1);
                self.insert_html("frameset", attrs);
                self.mode = Mode::InFrameset;
            }
            "address" | "article" | "aside" | "blockquote" | "center" | "details" | "dialog"
            | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer" | "header"
            | "hgroup" | "main" | "menu" | "nav" | "ol" | "p" | "search" | "section"
            | "summary" | "ul" => {
                self.close_p_if_in_button_scope();
                self.insert_html(name, attrs);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.close_p_if_in_button_scope();
                let cur = self.current();
                let e = self.el(cur);
                if e.namespace == Namespace::Html
                    && matches!(e.name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                {
                    self.open.pop();
                }
                self.insert_html(name, attrs);
            }
            "pre" | "listing" => {
                self.close_p_if_in_button_scope();
                self.insert_html(name, attrs);
                self.skip_lf = true;
                self.frameset_ok = false;
            }
            "form" => {
                if self.form.is_some() && !self.has_template() {
                    return;
                }
                self.close_p_if_in_button_scope();
                let id = self.insert_html(name, attrs);
                if !self.has_template() {
                    self.form = Some(id);
                }
            }
            "li" => {
                self.frameset_ok = false;
                self.list_item_loop(|n| n == "li");
                self.close_p_if_in_button_scope();
                self.insert_html(name, attrs);
            }
            "dd" | "dt" => {
                self.frameset_ok = false;
                self.list_item_loop(|n| n == "dd" || n == "dt");
                self.close_p_if_in_button_scope();
                self.insert_html(name, attrs);
            }
            "plaintext" => {
                self.close_p_if_in_button_scope();
                self.insert_html(name, attrs);
                self.tok.state = State::Plaintext;
            }
            "button" => {
                if self.has_in_scope(Scope::Default, "button") {
                    self.implied_end_tags(None, false);
                    self.pop_until_html("button");
                }
                self.reconstruct_active();
                self.insert_html(name, attrs);
                self.frameset_ok = false;
            }
            "a" => {
                let existing = self
                    .active
                    .iter()
                    .rev()
                    .take_while(|e| **e != Entry::Marker)
                    .find_map(|e| match e {
                        Entry::Element(id) if self.is(*id, "a") => Some(*id),
                        _ => None,
                    });
                if let Some(a) = existing {
                    self.adoption_agency("a");
                    if let Some(at) = self.active_position(a) {
                        self.active.remove(at);
                    }
                    if let Some(at) = self.open.iter().position(|&n| n == a) {
                        self.open.remove(at);
                    }
                }
                self.reconstruct_active();
                let id = self.insert_html(name, attrs);
                self.push_active(id);
            }
            "b" | "big" | "code" | "em" | "font" | "i" | "s" | "small" | "strike" | "strong"
            | "tt" | "u" => {
                self.reconstruct_active();
                let id = self.insert_html(name, attrs);
                self.push_active(id);
            }
            "nobr" => {
                self.reconstruct_active();
                if self.has_in_scope(Scope::Default, "nobr") {
                    self.adoption_agency("nobr");
                    self.reconstruct_active();
                }
                let id = self.insert_html(name, attrs);
                self.push_active(id);
            }
            "applet" | "marquee" | "object" => {
                self.reconstruct_active();
                self.insert_html(name, attrs);
                self.active.push(Entry::Marker);
                self.frameset_ok = false;
            }
            "table" => {
                if self.html.quirks != QuirksMode::Quirks {
                    self.close_p_if_in_button_scope();
                }
                self.insert_html(name, attrs);
                self.frameset_ok = false;
                self.mode = Mode::InTable;
            }
            "area" | "br" | "embed" | "img" | "keygen" | "wbr" => {
                self.reconstruct_active();
                self.insert_html(name, attrs);
                self.open.pop();
                self.frameset_ok = false;
            }
            "input" => {
                if self.context_is_select() {
                    return;
                }
                if self.has_in_scope(Scope::Default, "select") {
                    self.pop_until_html("select");
                }
                self.reconstruct_active();
                self.insert_html(name, attrs);
                self.open.pop();
                let hidden = attrs
                    .iter()
                    .find(|(n, _)| n == "type")
                    .is_some_and(|(_, v)| v.eq_ignore_ascii_case("hidden"));
                if !hidden {
                    self.frameset_ok = false;
                }
            }
            "param" | "source" | "track" => {
                self.insert_html(name, attrs);
                self.open.pop();
            }
            "hr" => {
                self.close_p_if_in_button_scope();
                if self.has_in_scope(Scope::Default, "select") {
                    self.implied_end_tags(None, false);
                }
                self.insert_html(name, attrs);
                self.open.pop();
                self.frameset_ok = false;
            }
            "image" => {
                self.in_body_start(
                    Token::StartTag {
                        name: "img".into(),
                        attrs: attrs.clone(),
                        self_closing,
                    },
                    "img",
                    attrs,
                    self_closing,
                );
            }
            "textarea" => {
                self.insert_html(name, attrs);
                self.skip_lf = true;
                self.tok.state = State::Rcdata;
                self.original = self.mode;
                self.frameset_ok = false;
                self.mode = Mode::Text;
            }
            "xmp" => {
                self.close_p_if_in_button_scope();
                self.reconstruct_active();
                self.frameset_ok = false;
                self.generic_text_element(name, attrs, State::Rawtext);
            }
            "iframe" => {
                self.frameset_ok = false;
                self.generic_text_element(name, attrs, State::Rawtext);
            }
            "noembed" | "noscript" => self.generic_text_element(name, attrs, State::Rawtext),
            "select" => {
                if self.context_is_select() {
                    return;
                }
                if self.has_in_scope(Scope::Default, "select") {
                    self.pop_until_html("select");
                    return;
                }
                self.reconstruct_active();
                self.insert_html(name, attrs);
                self.frameset_ok = false;
            }
            "option" => {
                if self.has_in_scope(Scope::Default, "select") {
                    self.implied_end_tags_except_optgroup();
                } else if self.current_is("option") {
                    self.open.pop();
                }
                self.reconstruct_active();
                self.insert_html(name, attrs);
            }
            "optgroup" => {
                if self.has_in_scope(Scope::Default, "select") {
                    self.implied_end_tags(None, false);
                } else if self.current_is("option") {
                    self.open.pop();
                }
                self.reconstruct_active();
                self.insert_html(name, attrs);
            }
            "rb" | "rtc" => {
                if self.has_in_scope(Scope::Default, "ruby") {
                    self.implied_end_tags(None, false);
                }
                self.insert_html(name, attrs);
            }
            "rp" | "rt" => {
                if self.has_in_scope(Scope::Default, "ruby") {
                    self.implied_end_tags(Some("rtc"), false);
                }
                self.insert_html(name, attrs);
            }
            "math" => {
                self.reconstruct_active();
                self.insert_element(
                    Namespace::MathMl,
                    name,
                    foreign_attributes(attrs, Namespace::MathMl),
                );
                if self_closing {
                    self.open.pop();
                }
            }
            "svg" => {
                self.reconstruct_active();
                self.insert_element(
                    Namespace::Svg,
                    name,
                    foreign_attributes(attrs, Namespace::Svg),
                );
                if self_closing {
                    self.open.pop();
                }
            }
            "caption" | "col" | "colgroup" | "frame" | "head" | "tbody" | "td" | "tfoot" | "th"
            | "thead" | "tr" => {}
            _ => {
                self.reconstruct_active();
                self.insert_html(name, attrs);
            }
        }
    }

    fn list_item_loop(&mut self, closes: impl Fn(&str) -> bool) {
        let mut i = self.open.len();
        while i > 0 {
            i -= 1;
            let node = self.open[i];
            let e = self.el(node);
            if e.namespace == Namespace::Html && closes(&e.name) {
                let name = e.name.clone();
                self.implied_end_tags(Some(&name), false);
                self.pop_until_html(&name);
                return;
            }
            if is_special(e)
                && !(e.namespace == Namespace::Html
                    && matches!(e.name.as_str(), "address" | "div" | "p"))
            {
                return;
            }
        }
    }

    fn in_body_end(&mut self, t: Token, name: &str) {
        match name {
            "template" => self.in_head(t),
            "body" => {
                if self.body_end_checks() {
                    self.mode = Mode::AfterBody;
                }
            }
            "html" => {
                if self.body_end_checks() {
                    self.mode = Mode::AfterBody;
                    self.process(t);
                }
            }
            "address" | "article" | "aside" | "blockquote" | "button" | "center" | "details"
            | "dialog" | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer"
            | "header" | "hgroup" | "listing" | "main" | "menu" | "nav" | "ol" | "pre"
            | "search" | "section" | "select" | "summary" | "ul" => {
                if !self.has_in_scope(Scope::Default, name) {
                    return;
                }
                self.implied_end_tags(None, false);
                self.pop_until_html(name);
            }
            "form" => {
                if !self.has_template() {
                    let node = self.form.take();
                    let Some(node) = node else {
                        return;
                    };
                    if !self.in_scope(Scope::Default, |id, _| id == node) {
                        return;
                    }
                    self.implied_end_tags(None, false);
                    if let Some(at) = self.open.iter().position(|&n| n == node) {
                        self.open.remove(at);
                    }
                } else {
                    if !self.has_in_scope(Scope::Default, "form") {
                        return;
                    }
                    self.implied_end_tags(None, false);
                    self.pop_until_html("form");
                }
            }
            "p" => {
                if !self.has_in_scope(Scope::Button, "p") {
                    self.insert_html("p", &[]);
                }
                self.close_p();
            }
            "li" => {
                if !self.has_in_scope(Scope::ListItem, "li") {
                    return;
                }
                self.implied_end_tags(Some("li"), false);
                self.pop_until_html("li");
            }
            "dd" | "dt" => {
                if !self.has_in_scope(Scope::Default, name) {
                    return;
                }
                self.implied_end_tags(Some(name), false);
                self.pop_until_html(name);
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if !self.has_heading_in_scope() {
                    return;
                }
                self.implied_end_tags(None, false);
                self.pop_until(|e| {
                    e.namespace == Namespace::Html
                        && matches!(e.name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
                });
            }
            "a" | "b" | "big" | "code" | "em" | "font" | "i" | "nobr" | "s" | "small"
            | "strike" | "strong" | "tt" | "u" => self.adoption_agency(name),
            "applet" | "marquee" | "object" => {
                if !self.has_in_scope(Scope::Default, name) {
                    return;
                }
                self.implied_end_tags(None, false);
                self.pop_until_html(name);
                self.clear_active_to_marker();
            }
            "br" => {
                self.in_body_start(
                    Token::StartTag {
                        name: "br".into(),
                        attrs: Vec::new(),
                        self_closing: false,
                    },
                    "br",
                    &Vec::new(),
                    false,
                );
            }
            _ => self.any_other_end_tag(name),
        }
    }

    fn text(&mut self, t: Token) {
        match t {
            Token::Character(c) => self.insert_char(c),
            Token::Eof => {
                self.open.pop();
                self.mode = self.original;
                self.process(t);
            }
            _ => {
                self.open.pop();
                self.mode = self.original;
            }
        }
    }

    fn in_table(&mut self, t: Token) {
        match &t {
            Token::Character(_)
                if self.open.last().is_some_and(|&n| {
                    let e = self.el(n);
                    e.namespace == Namespace::Html
                        && matches!(
                            e.name.as_str(),
                            "table" | "tbody" | "template" | "tfoot" | "thead" | "tr"
                        )
                }) =>
            {
                self.table_text.clear();
                self.original = self.mode;
                self.mode = Mode::InTableText;
                self.process(t);
            }
            Token::Comment(d) => self.insert_comment(d, None),
            Token::ProcessingInstruction { target, data } => self.insert_pi(target, data, None),
            Token::Doctype { .. } => {}
            Token::StartTag { name, attrs, .. } => match name.as_str() {
                "caption" => {
                    self.clear_back_to(&["table"]);
                    self.active.push(Entry::Marker);
                    self.insert_html(name, attrs);
                    self.mode = Mode::InCaption;
                }
                "colgroup" => {
                    self.clear_back_to(&["table"]);
                    self.insert_html(name, attrs);
                    self.mode = Mode::InColumnGroup;
                }
                "col" => {
                    self.clear_back_to(&["table"]);
                    self.insert_html("colgroup", &[]);
                    self.mode = Mode::InColumnGroup;
                    self.process(t);
                }
                "tbody" | "tfoot" | "thead" => {
                    self.clear_back_to(&["table"]);
                    self.insert_html(name, attrs);
                    self.mode = Mode::InTableBody;
                }
                "td" | "th" | "tr" => {
                    self.clear_back_to(&["table"]);
                    self.insert_html("tbody", &[]);
                    self.mode = Mode::InTableBody;
                    self.process(t);
                }
                "table" => {
                    if !self.has_in_scope(Scope::Table, "table") {
                        return;
                    }
                    self.pop_until_html("table");
                    self.reset_insertion_mode();
                    self.process(t);
                }
                "style" | "script" | "template" => self.in_head(t),
                "input" => {
                    let hidden = attrs
                        .iter()
                        .find(|(n, _)| n == "type")
                        .is_some_and(|(_, v)| v.eq_ignore_ascii_case("hidden"));
                    if hidden {
                        self.insert_html(name, attrs);
                        self.open.pop();
                    } else {
                        self.in_table_anything_else(t);
                    }
                }
                "form" => {
                    if self.form.is_some() && !self.has_template() {
                        return;
                    }
                    let id = self.insert_html(name, attrs);
                    if !self.has_template() {
                        self.form = Some(id);
                    }
                    self.open.pop();
                }
                _ => self.in_table_anything_else(t),
            },
            Token::EndTag { name, .. } => match name.as_str() {
                "table" => {
                    if !self.has_in_scope(Scope::Table, "table") {
                        return;
                    }
                    self.pop_until_html("table");
                    self.reset_insertion_mode();
                }
                "body" | "caption" | "col" | "colgroup" | "html" | "tbody" | "td" | "tfoot"
                | "th" | "thead" | "tr" => {}
                "template" => self.in_head(t),
                _ => self.in_table_anything_else(t),
            },
            Token::Eof => self.in_body(t),
            _ => self.in_table_anything_else(t),
        }
    }

    fn in_table_anything_else(&mut self, t: Token) {
        self.foster = true;
        self.in_body(t);
        self.foster = false;
    }

    fn in_table_text(&mut self, t: Token) {
        match t {
            Token::Character('\0') => {}
            Token::Character(c) => self.table_text.push(c),
            _ => {
                let pending = std::mem::take(&mut self.table_text);
                if pending.iter().any(|&c| !is_ws(c)) {
                    for c in pending {
                        self.in_table_anything_else(Token::Character(c));
                    }
                } else {
                    for c in pending {
                        self.insert_char(c);
                    }
                }
                self.mode = self.original;
                self.process(t);
            }
        }
    }

    fn in_caption(&mut self, t: Token) {
        match &t {
            Token::EndTag { name, .. } if name == "caption" => {
                if !self.has_in_scope(Scope::Table, "caption") {
                    return;
                }
                self.implied_end_tags(None, false);
                self.pop_until_html("caption");
                self.clear_active_to_marker();
                self.mode = Mode::InTable;
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                self.caption_close(t)
            }
            Token::EndTag { name, .. } if name == "table" => self.caption_close(t),
            Token::EndTag { name, .. }
                if matches!(
                    name.as_str(),
                    "body"
                        | "col"
                        | "colgroup"
                        | "html"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) => {}
            _ => self.in_body(t),
        }
    }

    fn caption_close(&mut self, t: Token) {
        if !self.has_in_scope(Scope::Table, "caption") {
            return;
        }
        self.implied_end_tags(None, false);
        self.pop_until_html("caption");
        self.clear_active_to_marker();
        self.mode = Mode::InTable;
        self.process(t);
    }

    fn in_column_group(&mut self, t: Token) {
        match &t {
            Token::Character(c) if is_ws(*c) => self.insert_char(*c),
            Token::Comment(d) => self.insert_comment(d, None),
            Token::ProcessingInstruction { target, data } => self.insert_pi(target, data, None),
            Token::Doctype { .. } => {}
            Token::StartTag { name, .. } if name == "html" => self.in_body(t),
            Token::StartTag { name, attrs, .. } if name == "col" => {
                self.insert_html(name, attrs);
                self.open.pop();
            }
            Token::EndTag { name, .. } if name == "colgroup" => {
                if !self.current_is("colgroup") {
                    return;
                }
                self.open.pop();
                self.mode = Mode::InTable;
            }
            Token::EndTag { name, .. } if name == "col" => {}
            Token::StartTag { name, .. } if name == "template" => self.in_head(t),
            Token::EndTag { name, .. } if name == "template" => self.in_head(t),
            Token::Eof => self.in_body(t),
            _ => {
                if !self.current_is("colgroup") {
                    return;
                }
                self.open.pop();
                self.mode = Mode::InTable;
                self.process(t);
            }
        }
    }

    fn in_table_body(&mut self, t: Token) {
        match &t {
            Token::StartTag { name, attrs, .. } if name == "tr" => {
                self.clear_back_to(&["tbody", "tfoot", "thead"]);
                self.insert_html(name, attrs);
                self.mode = Mode::InRow;
            }
            Token::StartTag { name, .. } if name == "th" || name == "td" => {
                self.clear_back_to(&["tbody", "tfoot", "thead"]);
                self.insert_html("tr", &[]);
                self.mode = Mode::InRow;
                self.process(t);
            }
            Token::EndTag { name, .. } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if !self.has_in_scope(Scope::Table, name) {
                    return;
                }
                self.clear_back_to(&["tbody", "tfoot", "thead"]);
                self.open.pop();
                self.mode = Mode::InTable;
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead"
                ) =>
            {
                self.table_body_close(t)
            }
            Token::EndTag { name, .. } if name == "table" => self.table_body_close(t),
            Token::EndTag { name, .. }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" | "tr"
                ) => {}
            _ => self.in_table(t),
        }
    }

    fn table_body_close(&mut self, t: Token) {
        let any = self.in_scope(Scope::Table, |_, e| {
            e.namespace == Namespace::Html && matches!(e.name.as_str(), "tbody" | "thead" | "tfoot")
        });
        if !any {
            return;
        }
        self.clear_back_to(&["tbody", "tfoot", "thead"]);
        self.open.pop();
        self.mode = Mode::InTable;
        self.process(t);
    }

    fn in_row(&mut self, t: Token) {
        match &t {
            Token::StartTag { name, attrs, .. } if name == "th" || name == "td" => {
                self.clear_back_to(&["tr"]);
                self.insert_html(name, attrs);
                self.mode = Mode::InCell;
                self.active.push(Entry::Marker);
            }
            Token::EndTag { name, .. } if name == "tr" => {
                if !self.has_in_scope(Scope::Table, "tr") {
                    return;
                }
                self.clear_back_to(&["tr"]);
                self.open.pop();
                self.mode = Mode::InTableBody;
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr"
                ) =>
            {
                self.row_close(t)
            }
            Token::EndTag { name, .. } if name == "table" => self.row_close(t),
            Token::EndTag { name, .. } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if !self.has_in_scope(Scope::Table, name) {
                    return;
                }
                self.row_close(t);
            }
            Token::EndTag { name, .. }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th"
                ) => {}
            _ => self.in_table(t),
        }
    }

    fn row_close(&mut self, t: Token) {
        if !self.has_in_scope(Scope::Table, "tr") {
            return;
        }
        self.clear_back_to(&["tr"]);
        self.open.pop();
        self.mode = Mode::InTableBody;
        self.process(t);
    }

    fn close_cell(&mut self) {
        self.implied_end_tags(None, false);
        self.pop_until(|e| e.namespace == Namespace::Html && (e.name == "td" || e.name == "th"));
        self.clear_active_to_marker();
        self.mode = Mode::InRow;
    }

    fn in_cell(&mut self, t: Token) {
        match &t {
            Token::EndTag { name, .. } if name == "td" || name == "th" => {
                if !self.has_in_scope(Scope::Table, name) {
                    return;
                }
                self.implied_end_tags(None, false);
                self.pop_until_html(name);
                self.clear_active_to_marker();
                self.mode = Mode::InRow;
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                if !self.in_scope(Scope::Table, |_, e| {
                    e.namespace == Namespace::Html && (e.name == "td" || e.name == "th")
                }) {
                    return;
                }
                self.close_cell();
                self.process(t);
            }
            Token::EndTag { name, .. }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html"
                ) => {}
            Token::EndTag { name, .. }
                if matches!(name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr") =>
            {
                if !self.has_in_scope(Scope::Table, name) {
                    return;
                }
                self.close_cell();
                self.process(t);
            }
            _ => self.in_body(t),
        }
    }

    fn in_template(&mut self, t: Token) {
        match &t {
            Token::Character(_)
            | Token::Comment(_)
            | Token::ProcessingInstruction { .. }
            | Token::Doctype { .. } => self.in_body(t),
            Token::StartTag { name, .. } => {
                let target = match name.as_str() {
                    "base" | "basefont" | "bgsound" | "link" | "meta" | "noframes" | "script"
                    | "style" | "template" | "title" => {
                        self.in_head(t);
                        return;
                    }
                    "caption" | "colgroup" | "tbody" | "tfoot" | "thead" => Mode::InTable,
                    "col" => Mode::InColumnGroup,
                    "tr" => Mode::InTableBody,
                    "td" | "th" => Mode::InRow,
                    _ => Mode::InBody,
                };
                self.template_modes.pop();
                self.template_modes.push(target);
                self.mode = target;
                self.process(t);
            }
            Token::EndTag { name, .. } => {
                if name == "template" {
                    self.in_head(t);
                }
            }
            Token::Eof => {
                if !self.has_template() {
                    return;
                }
                self.pop_until_html("template");
                self.clear_active_to_marker();
                self.template_modes.pop();
                self.reset_insertion_mode();
                self.process(t);
            }
        }
    }

    fn after_body(&mut self, t: Token) {
        match &t {
            Token::Character(c) if is_ws(*c) => self.in_body(t),
            Token::Comment(d) => {
                let root = self.open[0];
                self.insert_comment(d, Some(root));
            }
            Token::ProcessingInstruction { target, data } => {
                let root = self.open[0];
                self.insert_pi(target, data, Some(root));
            }
            Token::Doctype { .. } => {}
            Token::StartTag { name, .. } if name == "html" => self.in_body(t),
            Token::EndTag { name, .. } if name == "html" => {
                if self.context.is_none() {
                    self.mode = Mode::AfterAfterBody;
                }
            }
            Token::Eof => {}
            _ => {
                self.mode = Mode::InBody;
                self.process(t);
            }
        }
    }

    fn in_frameset(&mut self, t: Token) {
        match &t {
            Token::Character(c) if is_ws(*c) => self.insert_char(*c),
            Token::Comment(d) => self.insert_comment(d, None),
            Token::ProcessingInstruction { target, data } => self.insert_pi(target, data, None),
            Token::Doctype { .. } => {}
            Token::StartTag { name, .. } if name == "html" => self.in_body(t),
            Token::StartTag { name, attrs, .. } if name == "frameset" => {
                self.insert_html(name, attrs);
            }
            Token::EndTag { name, .. } if name == "frameset" => {
                if self.open.len() == 1 {
                    return;
                }
                self.open.pop();
                if self.context.is_none() && !self.current_is("frameset") {
                    self.mode = Mode::AfterFrameset;
                }
            }
            Token::StartTag { name, attrs, .. } if name == "frame" => {
                self.insert_html(name, attrs);
                self.open.pop();
            }
            Token::StartTag { name, .. } if name == "noframes" => self.in_head(t),
            _ => {}
        }
    }

    fn after_frameset(&mut self, t: Token) {
        match &t {
            Token::Character(c) if is_ws(*c) => self.insert_char(*c),
            Token::Comment(d) => self.insert_comment(d, None),
            Token::ProcessingInstruction { target, data } => self.insert_pi(target, data, None),
            Token::Doctype { .. } => {}
            Token::StartTag { name, .. } if name == "html" => self.in_body(t),
            Token::EndTag { name, .. } if name == "html" => self.mode = Mode::AfterAfterFrameset,
            Token::StartTag { name, .. } if name == "noframes" => self.in_head(t),
            _ => {}
        }
    }

    fn after_after_body(&mut self, t: Token) {
        match &t {
            Token::Comment(d) => self.insert_comment(d, Some(DOCUMENT)),
            Token::ProcessingInstruction { target, data } => {
                self.insert_pi(target, data, Some(DOCUMENT))
            }
            Token::Doctype { .. } => self.in_body(t),
            Token::Character(c) if is_ws(*c) => self.in_body(t),
            Token::StartTag { name, .. } if name == "html" => self.in_body(t),
            Token::Eof => {}
            _ => {
                self.mode = Mode::InBody;
                self.process(t);
            }
        }
    }

    fn after_after_frameset(&mut self, t: Token) {
        match &t {
            Token::Comment(d) => self.insert_comment(d, Some(DOCUMENT)),
            Token::ProcessingInstruction { target, data } => {
                self.insert_pi(target, data, Some(DOCUMENT))
            }
            Token::Doctype { .. } => self.in_body(t),
            Token::Character(c) if is_ws(*c) => self.in_body(t),
            Token::StartTag { name, .. } if name == "html" => self.in_body(t),
            Token::StartTag { name, .. } if name == "noframes" => self.in_head(t),
            _ => {}
        }
    }

    // ---- foreign content -----------------------------------------------------------------------

    fn foreign(&mut self, t: Token) {
        match &t {
            Token::Character('\0') => self.insert_char('\u{fffd}'),
            Token::Character(c) => {
                self.insert_char(*c);
                if !is_ws(*c) {
                    self.frameset_ok = false;
                }
            }
            Token::Comment(d) => self.insert_comment(d, None),
            Token::ProcessingInstruction { target, data } => self.insert_pi(target, data, None),
            Token::Doctype { .. } => {}
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => {
                let breaks_out = matches!(
                    name.as_str(),
                    "b" | "big"
                        | "blockquote"
                        | "body"
                        | "br"
                        | "center"
                        | "code"
                        | "dd"
                        | "div"
                        | "dl"
                        | "dt"
                        | "em"
                        | "embed"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "head"
                        | "hr"
                        | "i"
                        | "img"
                        | "li"
                        | "listing"
                        | "menu"
                        | "meta"
                        | "nobr"
                        | "ol"
                        | "p"
                        | "pre"
                        | "ruby"
                        | "s"
                        | "small"
                        | "span"
                        | "strong"
                        | "strike"
                        | "sub"
                        | "sup"
                        | "table"
                        | "tt"
                        | "u"
                        | "ul"
                        | "var"
                ) || (name == "font"
                    && attrs
                        .iter()
                        .any(|(n, _)| matches!(n.as_str(), "color" | "face" | "size")));
                if breaks_out {
                    self.leave_foreign();
                    self.process(t);
                    return;
                }
                let adjusted = self
                    .adjusted_current()
                    .expect("foreign content has a current node");
                let ns = self.el(adjusted).namespace;
                let mut local = name.clone();
                if ns == Namespace::Svg
                    && let Some((_, fixed)) = SVG_TAG_NAMES.iter().find(|(from, _)| from == name)
                {
                    local = (*fixed).to_string();
                }
                self.insert_element(ns, &local, foreign_attributes(attrs, ns));
                if *self_closing {
                    self.open.pop();
                }
            }
            Token::EndTag { name, .. } => {
                if name == "br" || name == "p" {
                    self.leave_foreign();
                    self.process(t);
                    return;
                }
                if name == "script" {
                    let cur = self.current();
                    let e = self.el(cur);
                    if e.namespace == Namespace::Svg && e.name == "script" {
                        self.open.pop();
                        return;
                    }
                }
                let mut i = self.open.len() - 1;
                loop {
                    let node = self.open[i];
                    if i == 0 {
                        return;
                    }
                    if self.el(node).name.to_ascii_lowercase() == *name {
                        self.open.truncate(i);
                        return;
                    }
                    i -= 1;
                    if self.el(self.open[i]).namespace == Namespace::Html {
                        self.process(t);
                        return;
                    }
                }
            }
            Token::Eof => self.process(t),
        }
    }

    fn leave_foreign(&mut self) {
        while let Some(&id) = self.open.last() {
            let e = self.el(id);
            let mathml_text = e.namespace == Namespace::MathMl
                && matches!(e.name.as_str(), "mi" | "mo" | "mn" | "ms" | "mtext");
            if e.namespace == Namespace::Html || mathml_text || self.html_integration_point(e) {
                break;
            }
            self.open.pop();
        }
    }
}

fn same_attributes(a: &[Attribute], b: &[Attribute]) -> bool {
    a.len() == b.len()
        && a.iter().all(|x| {
            b.iter().any(|y| {
                x.name == y.name
                    && x.namespace == y.namespace
                    && x.prefix == y.prefix
                    && x.value == y.value
            })
        })
}

fn quirks_mode(
    name: Option<&str>,
    public_id: Option<&str>,
    system_id: Option<&str>,
    force: bool,
) -> QuirksMode {
    let public = public_id.map(str::to_ascii_lowercase);
    let system = system_id.map(str::to_ascii_lowercase);
    let public_s = public.as_deref();
    if force
        || name != Some("html")
        || matches!(
            public_s,
            Some(
                "-//w3o//dtd w3 html strict 3.0//en//"
                    | "-/w3c/dtd html 4.0 transitional/en"
                    | "html"
            )
        )
        || system.as_deref() == Some("http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd")
        || public_s.is_some_and(|p| QUIRKS_PUBLIC_ID_PREFIXES.iter().any(|q| p.starts_with(q)))
        || (system.as_deref().is_none_or(str::is_empty)
            && public_s.is_some_and(|p| {
                p.starts_with("-//w3c//dtd html 4.01 frameset//")
                    || p.starts_with("-//w3c//dtd html 4.01 transitional//")
            }))
    {
        return QuirksMode::Quirks;
    }
    if public_s.is_some_and(|p| {
        p.starts_with("-//w3c//dtd xhtml 1.0 frameset//")
            || p.starts_with("-//w3c//dtd xhtml 1.0 transitional//")
            || (system.as_deref().is_some_and(|s| !s.is_empty())
                && (p.starts_with("-//w3c//dtd html 4.01 frameset//")
                    || p.starts_with("-//w3c//dtd html 4.01 transitional//")))
    }) {
        return QuirksMode::LimitedQuirks;
    }
    QuirksMode::NoQuirks
}

#[cfg(test)]
mod tests {
    use super::super::selector::Selector;
    use super::*;

    #[test]
    fn template_contents_are_detached_from_the_tree() {
        let h = Html::parse_document("<template><p>hidden</p></template><b>shown</b>");
        assert_eq!(h.root_element().text().collect::<String>(), "shown");
        let t = Selector::parse("template").unwrap();
        let template = h.select(&t).next().unwrap();
        assert_eq!(template.children().count(), 0);
        let contents = template.template_contents().unwrap();
        assert_eq!(contents.children().count(), 1);
        let p = Selector::parse("p").unwrap();
        assert_eq!(
            h.select(&p).count(),
            0,
            "content is not part of the document tree"
        );
    }

    #[test]
    fn fragments_are_the_children_of_the_root_element() {
        let h = Html::parse_fragment("<b>a</b>c<table><td>d");
        assert_eq!(h.root_element().value().name(), "html");
        assert_eq!(h.root_element().text().collect::<String>(), "acd");
        let h = Html::parse_fragment_in("<td>x<td>y", Namespace::Html, "tr");
        let td = Selector::parse("td").unwrap();
        assert_eq!(h.select(&td).count(), 2);
    }

    #[test]
    fn misnested_formatting_follows_the_adoption_agency() {
        let h = Html::parse_document("<p><b>1<p>2</b>3");
        let b = Selector::parse("p > b").unwrap();
        assert_eq!(
            h.select(&b).count(),
            2,
            "the b is reopened in the second paragraph"
        );
        assert_eq!(h.root_element().text().collect::<String>(), "123");
    }

    #[test]
    fn very_deep_documents_do_not_overflow_the_stack() {
        let depth = 60_000;
        let doc = "<div>".repeat(depth) + "x";
        let h = Html::parse_document(&doc);
        let div = Selector::parse("div").unwrap();
        assert_eq!(h.select(&div).count(), depth);
        assert_eq!(h.root_element().text().collect::<String>(), "x");
        drop(h);
    }

    #[test]
    fn character_references_and_cdata_follow_the_standard() {
        let h =
            Html::parse_document("<p>&lt;&amp;&#x41;&notit;&nbsp;</p><svg><![CDATA[<x>]]></svg>");
        assert_eq!(
            h.root_element().text().collect::<String>(),
            "<&A\u{ac}it;\u{a0}<x>"
        );
        let h = Html::parse_document("<p><![CDATA[x]]>y</p>");
        assert_eq!(
            h.root_element().text().collect::<String>(),
            "y",
            "CDATA outside foreign content is a bogus comment"
        );
    }
}
