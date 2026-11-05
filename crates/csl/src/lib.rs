//! CSL 1.0.2 style files, parsed and rendered for citations.
//!
//! The Citation Style Language (docs.citationstyles.org — a public spec) is the XML format
//! Zotero/Mendeley styles are written in. This crate reads a style file into the [`Style`]
//! model: the subset of CSL 1.0.2 that WordCraft's renderer draws on (text, numbers, dates,
//! names, labels, groups and conditionals), with the en-US locale terms from the spec's
//! appendix. It then renders that model against a document's sources — plain-text
//! citations ([`Style::cite`]) and bibliography entries in sorted order
//! ([`Style::entries`]) — mapping a [`wordcraft_doc::Source`] onto CSL item variables.
//!
//! Parsing is lenient the way the format demands forward-compatibility (unknown elements
//! and attributes are ignored) but honest about the subset's edges: constructs that would
//! silently change rendered output — collapsing, disambiguation, formatting attributes,
//! non-en-US locales, name/date options the model doesn't carry — load with a
//! human-readable entry in [`Style::warned`]. Hostile input never panics or hangs: files are
//! capped at 2 MiB, XML nesting at 64 elements, macro expansion at 32 levels (references
//! beyond are dropped with a warning), and real XML errors are `Err(String)`. Rendering
//! keeps the promise from its side: missing variables render empty, hostile values render
//! something, and nothing panics.
//!
//! Layer L2: the renderer draws on `wordcraft-doc`'s `Source` (L1) and nothing above it.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod model;
mod parse;
mod render;
mod terms;
mod xml;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_render;

pub use model::{
    AndStyle, Bibliography, Branch, Choose, Condition, Date, DatePart, DatePartName, DateParts, Element, Form, Group, Label, Layout, Match,
    NameAsSortOrder, NameOptions, Names, Number, Plural, Position, SortDirection, SortField, SortKey, Style, Text, TextSource,
};
pub use parse::parse;
pub use terms::{Term, TermSet};

use crate::model::Element as ModelElement;

/// Walk every element of a rendering tree, mutably (used to cut macro references).
pub(crate) fn walk_elements(els: &mut [ModelElement], f: &mut impl FnMut(&mut ModelElement)) {
    for e in els.iter_mut() {
        f(e);
        match e {
            ModelElement::Group(g) => walk_elements(&mut g.elements, f),
            ModelElement::Choose(c) => {
                for b in &mut c.branches {
                    walk_elements(&mut b.layout.elements, f);
                }
                if let Some(e) = &mut c.else_branch {
                    walk_elements(&mut e.elements, f);
                }
            }
            ModelElement::Names(n) => {
                if let Some(s) = &mut n.substitute {
                    walk_elements(s, f);
                }
            }
            _ => {}
        }
    }
}

/// Walk every element of a rendering tree, read-only (used to inspect references).
pub(crate) fn for_each_element(els: &[ModelElement], f: &mut impl FnMut(&ModelElement)) {
    for e in els {
        f(e);
        match e {
            ModelElement::Group(g) => for_each_element(&g.elements, f),
            ModelElement::Choose(c) => {
                for b in &c.branches {
                    for_each_element(&b.layout.elements, f);
                }
                if let Some(e) = &c.else_branch {
                    for_each_element(&e.elements, f);
                }
            }
            ModelElement::Names(n) => {
                if let Some(s) = &n.substitute {
                    for_each_element(s, f);
                }
            }
            _ => {}
        }
    }
}

/// The macro names referenced by a rendering tree (used to bound expansion).
pub(crate) fn referenced_macros(els: &[ModelElement], out: &mut Vec<String>) {
    for_each_element(els, &mut |e| {
        if let ModelElement::Text(t) = e
            && let TextSource::Macro(m) = &t.source
        {
            out.push(m.clone());
        }
    });
}
