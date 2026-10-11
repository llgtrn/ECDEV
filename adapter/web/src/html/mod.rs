//! A native HTML parser and document tree: the WHATWG tokenizer and tree construction stages,
//! a selector engine, and a read API for extraction.

pub mod dom;
mod foreign;
mod quirks;
pub mod selector;
pub mod tokenizer;
mod tree_builder;

pub use dom::{
    Attribute, AttributeNamespace, Element, ElementRef, Html, Namespace, Node, NodeId, NodeRef,
    QuirksMode,
};
pub use selector::{Selector, SelectorError};
