//! The document tree produced by the HTML parser: an arena of nodes addressed by `NodeId`.
//!
//! The read API follows the shape the extraction code needs (`Html`, `ElementRef`, `NodeRef`),
//! with owned strings and no interior mutability. `template` contents are the template element's
//! own children (the tree builder does not keep a separate content fragment).

use super::selector::Selector;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    Html,
    MathMl,
    Svg,
}

impl Namespace {
    pub fn uri(self) -> &'static str {
        match self {
            Namespace::Html => "http://www.w3.org/1999/xhtml",
            Namespace::MathMl => "http://www.w3.org/1998/Math/MathML",
            Namespace::Svg => "http://www.w3.org/2000/svg",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeNamespace {
    None,
    XLink,
    Xml,
    Xmlns,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub namespace: AttributeNamespace,
    pub prefix: Option<String>,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub(crate) name: String,
    pub(crate) namespace: Namespace,
    pub(crate) attributes: Vec<Attribute>,
    /// For `template`: the detached fragment holding its content (not a child of the element).
    pub(crate) template_contents: Option<NodeId>,
}

impl Element {
    /// The local name (case as the parser fixed it: lowercase in HTML, adjusted in SVG).
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn namespace(&self) -> Namespace {
        self.namespace
    }
    pub fn template_contents(&self) -> Option<NodeId> {
        self.template_contents
    }
    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }
    /// The first attribute with this local name, whatever its namespace.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }
    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }
    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .is_some_and(|c| c.split_ascii_whitespace().any(|part| part == class))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Document,
    Fragment,
    Doctype {
        name: String,
        public_id: String,
        system_id: String,
    },
    Comment(String),
    Text(String),
    Element(Element),
    ProcessingInstruction {
        target: String,
        data: String,
    },
}

impl Node {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Node::Text(t) => Some(t),
            _ => None,
        }
    }
    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Node::Element(e) => Some(e),
            _ => None,
        }
    }
    pub fn is_element(&self) -> bool {
        matches!(self, Node::Element(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuirksMode {
    NoQuirks,
    LimitedQuirks,
    Quirks,
}

#[derive(Debug)]
pub(crate) struct Slot {
    pub(crate) node: Node,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
}

#[derive(Debug)]
pub struct Html {
    pub(crate) slots: Vec<Slot>,
    pub(crate) quirks: QuirksMode,
    /// The document element (`html`), or for a fragment the `html` element holding the fragment.
    pub(crate) root_element: NodeId,
}

pub const DOCUMENT: NodeId = NodeId(0);

impl Html {
    pub(crate) fn empty(root: Node) -> Html {
        Html {
            slots: vec![Slot {
                node: root,
                parent: None,
                children: Vec::new(),
            }],
            quirks: QuirksMode::NoQuirks,
            root_element: DOCUMENT,
        }
    }

    pub(crate) fn alloc(&mut self, node: Node) -> NodeId {
        let id = NodeId(self.slots.len() as u32);
        self.slots.push(Slot {
            node,
            parent: None,
            children: Vec::new(),
        });
        id
    }

    pub(crate) fn node(&self, id: NodeId) -> &Node {
        &self.slots[id.0 as usize].node
    }
    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.slots[id.0 as usize].node
    }
    pub(crate) fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.slots[id.0 as usize].parent
    }
    pub(crate) fn children(&self, id: NodeId) -> &[NodeId] {
        &self.slots[id.0 as usize].children
    }

    pub(crate) fn detach(&mut self, id: NodeId) {
        if let Some(p) = self.slots[id.0 as usize].parent.take() {
            let kids = &mut self.slots[p.0 as usize].children;
            if let Some(at) = kids.iter().rposition(|&k| k == id) {
                kids.remove(at);
            }
        }
    }

    pub(crate) fn append(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        self.slots[child.0 as usize].parent = Some(parent);
        self.slots[parent.0 as usize].children.push(child);
    }

    pub(crate) fn insert_before(&mut self, parent: NodeId, child: NodeId, before: NodeId) {
        self.detach(child);
        self.slots[child.0 as usize].parent = Some(parent);
        let kids = &mut self.slots[parent.0 as usize].children;
        let at = kids.iter().position(|&k| k == before).unwrap_or(kids.len());
        kids.insert(at, child);
    }

    /// Appends text at the end of `parent`, extending a trailing text node when there is one.
    pub(crate) fn append_text(&mut self, parent: NodeId, c: char) {
        if let Some(&last) = self.slots[parent.0 as usize].children.last()
            && let Node::Text(t) = &mut self.slots[last.0 as usize].node
        {
            t.push(c);
            return;
        }
        let id = self.alloc(Node::Text(c.to_string()));
        self.append(parent, id);
    }

    pub(crate) fn insert_text_before(&mut self, parent: NodeId, before: NodeId, c: char) {
        let kids = &self.slots[parent.0 as usize].children;
        if let Some(at) = kids.iter().position(|&k| k == before)
            && at > 0
        {
            let prev = kids[at - 1];
            if let Node::Text(t) = &mut self.slots[prev.0 as usize].node {
                t.push(c);
                return;
            }
        }
        let id = self.alloc(Node::Text(c.to_string()));
        self.insert_before(parent, id, before);
    }

    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks
    }

    /// Parses a complete document.
    pub fn parse_document(input: &str) -> Html {
        super::tree_builder::TreeBuilder::parse_document(input)
    }

    /// Parses a fragment in the context of a `body` element; the root element holds the fragment.
    pub fn parse_fragment(input: &str) -> Html {
        Html::parse_fragment_in(input, Namespace::Html, "body")
    }

    /// Parses a fragment as the children of the named context element.
    pub fn parse_fragment_in(input: &str, namespace: Namespace, context: &str) -> Html {
        super::tree_builder::TreeBuilder::parse_fragment(input, namespace, context)
    }

    pub fn root_element(&self) -> ElementRef<'_> {
        ElementRef {
            html: self,
            id: self.root_element,
        }
    }

    pub fn node_ref(&self, id: NodeId) -> NodeRef<'_> {
        NodeRef { html: self, id }
    }

    pub fn tree_root(&self) -> NodeRef<'_> {
        NodeRef {
            html: self,
            id: DOCUMENT,
        }
    }

    /// Every element matching the selector, in document order.
    pub fn select<'a, 'b>(
        &'a self,
        selector: &'b Selector,
    ) -> impl Iterator<Item = ElementRef<'a>> + use<'a, 'b> {
        let mut matches = Vec::new();
        let mut stack = vec![DOCUMENT];
        while let Some(id) = stack.pop() {
            if let Node::Element(_) = self.node(id) {
                let e = ElementRef { html: self, id };
                if selector.matches(e) {
                    matches.push(e);
                }
            }
            stack.extend(self.children(id).iter().rev().copied());
        }
        matches.into_iter()
    }
}

#[derive(Clone, Copy)]
pub struct NodeRef<'a> {
    html: &'a Html,
    id: NodeId,
}

impl<'a> NodeRef<'a> {
    pub fn id(&self) -> NodeId {
        self.id
    }
    pub fn value(&self) -> &'a Node {
        self.html.node(self.id)
    }
    pub fn parent(&self) -> Option<NodeRef<'a>> {
        self.html.parent(self.id).map(|id| NodeRef {
            html: self.html,
            id,
        })
    }
    pub fn children(&self) -> impl Iterator<Item = NodeRef<'a>> + use<'a> {
        let html = self.html;
        html.children(self.id)
            .iter()
            .map(move |&id| NodeRef { html, id })
    }
    /// Parent, grandparent, ... up to and including the document node.
    pub fn ancestors(&self) -> impl Iterator<Item = NodeRef<'a>> + use<'a> {
        let html = self.html;
        let mut next = html.parent(self.id);
        std::iter::from_fn(move || {
            let id = next?;
            next = html.parent(id);
            Some(NodeRef { html, id })
        })
    }
}

#[derive(Clone, Copy)]
pub struct ElementRef<'a> {
    html: &'a Html,
    id: NodeId,
}

impl PartialEq for ElementRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.html, other.html) && self.id == other.id
    }
}
impl Eq for ElementRef<'_> {}

impl<'a> ElementRef<'a> {
    pub fn wrap(node: NodeRef<'a>) -> Option<ElementRef<'a>> {
        node.value().is_element().then_some(ElementRef {
            html: node.html,
            id: node.id,
        })
    }
    pub fn id(&self) -> NodeId {
        self.id
    }
    pub fn node(&self) -> NodeRef<'a> {
        NodeRef {
            html: self.html,
            id: self.id,
        }
    }
    pub fn value(&self) -> &'a Element {
        match self.html.node(self.id) {
            Node::Element(e) => e,
            _ => unreachable!("an ElementRef always addresses an element"),
        }
    }
    pub fn quirks(&self) -> QuirksMode {
        self.html.quirks
    }
    /// The content fragment of a `template` element.
    pub fn template_contents(&self) -> Option<NodeRef<'a>> {
        self.value().template_contents.map(|id| NodeRef {
            html: self.html,
            id,
        })
    }
    pub fn parent(&self) -> Option<ElementRef<'a>> {
        self.node().parent().and_then(ElementRef::wrap)
    }
    pub fn children(&self) -> impl Iterator<Item = NodeRef<'a>> + use<'a> {
        self.node().children()
    }
    pub fn child_elements(&self) -> impl Iterator<Item = ElementRef<'a>> + use<'a> {
        self.children().filter_map(ElementRef::wrap)
    }
    pub fn ancestors(&self) -> impl Iterator<Item = NodeRef<'a>> + use<'a> {
        self.node().ancestors()
    }
    /// Descendant text, in document order.
    pub fn text(&self) -> impl Iterator<Item = &'a str> + use<'a> {
        let html = self.html;
        let mut stack: Vec<NodeId> = html.children(self.id).iter().rev().copied().collect();
        std::iter::from_fn(move || {
            while let Some(id) = stack.pop() {
                match html.node(id) {
                    Node::Text(t) => return Some(t.as_str()),
                    _ => stack.extend(html.children(id).iter().rev().copied()),
                }
            }
            None
        })
    }
    pub fn select<'b>(
        &self,
        selector: &'b Selector,
    ) -> impl Iterator<Item = ElementRef<'a>> + use<'a, 'b> {
        let html = self.html;
        let mut matches = Vec::new();
        let mut stack: Vec<NodeId> = html.children(self.id).iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            if let Node::Element(_) = html.node(id) {
                let e = ElementRef { html, id };
                if selector.matches(e) {
                    matches.push(e);
                }
            }
            stack.extend(html.children(id).iter().rev().copied());
        }
        matches.into_iter()
    }
}
