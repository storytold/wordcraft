//! A minimal, bounded XML tree on `quick-xml` — the same base `crates/docx` uses, sized for
//! CSL style files: local names only (CSL is namespace-agnostic; a style may bind its
//! elements to any prefix), no namespace machinery, hard caps on depth and element count.
//!
//! Lenient in the ways styles in the wild need: mismatched end tags and unclosed elements at
//! EOF are tolerated (the tree closes them), unknown entities and invalid UTF-8 in text
//! become replacement characters. Real XML errors surface as `Err(String)`.

use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// Maximum element nesting (CSL styles nest a handful of levels; hostile files stop here).
pub(crate) const MAX_DEPTH: usize = 64;
/// Maximum number of elements in one file (a 2 MiB file can't need more).
pub(crate) const MAX_ELEMENTS: usize = 100_000;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    El(El),
    Text(String),
}

/// An element with local (prefix-stripped) name and attributes.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub kids: Vec<Node>,
}

impl El {
    /// Attribute by local name, ignoring any prefix it was written with.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| local(k) == name).map(|(_, v)| v.as_str())
    }
    /// Child elements.
    pub fn els(&self) -> impl Iterator<Item = &El> {
        self.kids.iter().filter_map(|n| match n {
            Node::El(e) => Some(e),
            Node::Text(_) => None,
        })
    }
    /// Direct text content.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for n in &self.kids {
            if let Node::Text(t) = n {
                s.push_str(t);
            }
        }
        s
    }
    /// Local part of the element's name.
    pub fn local(&self) -> &str {
        local(&self.name)
    }
    /// First descendant with this local name (depth-first, bounded by the reader's cap).
    #[cfg(test)]
    pub(crate) fn find_descendant(&self, name: &str) -> Option<&El> {
        fn go<'a>(e: &'a El, name: &str, depth: usize) -> Option<&'a El> {
            if depth > MAX_DEPTH {
                return None;
            }
            for c in e.els() {
                if c.local() == name {
                    return Some(c);
                }
                if let Some(f) = go(c, name, depth + 1) {
                    return Some(f);
                }
            }
            None
        }
        go(self, name, 0)
    }
}

/// Local part of a prefixed name (`xml:lang` → `lang`).
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Parse XML bytes into the root element.
pub(crate) fn parse(bytes: &[u8]) -> Result<El, String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    let mut r = Reader::from_reader(bytes);
    {
        let c = r.config_mut();
        c.trim_text_start = true;
        c.trim_text_end = true;
        c.expand_empty_elements = false;
        c.check_end_names = false;
        c.allow_unmatched_ends = true;
    }
    let mut stack: Vec<El> = Vec::new();
    let mut root: Option<El> = None;
    let mut count = 0usize;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let ev = r.read_event_into(&mut buf).map_err(|e| format!("XML: {e}"))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                count += 1;
                if count > MAX_ELEMENTS {
                    return Err("XML: too many elements".into());
                }
                let el = read_el(e)?;
                if matches!(ev, Event::Start(_)) {
                    if stack.len() >= MAX_DEPTH {
                        return Err(format!("XML: nested deeper than {MAX_DEPTH} elements"));
                    }
                    stack.push(el);
                } else {
                    attach(&mut stack, &mut root, el);
                }
            }
            Event::End(_) => {
                if let Some(el) = stack.pop() {
                    attach(&mut stack, &mut root, el);
                }
            }
            Event::Text(ref t) => {
                if let Some(top) = stack.last_mut() {
                    let s = t.decode().map(|c| c.into_owned()).unwrap_or_else(|_| {
                        // Hostile bytes: keep what decodes rather than failing the file.
                        String::from_utf8_lossy(t.as_ref()).into_owned()
                    });
                    push_text(top, &s);
                }
            }
            Event::CData(ref t) => {
                if let Some(top) = stack.last_mut() {
                    let s = String::from_utf8_lossy(t.as_ref()).into_owned();
                    push_text(top, &s);
                }
            }
            Event::GeneralRef(ref g) => {
                if let Some(top) = stack.last_mut() {
                    let name = String::from_utf8_lossy(g.as_ref());
                    let s = match name.as_ref() {
                        "amp" => "&".to_string(),
                        "lt" => "<".to_string(),
                        "gt" => ">".to_string(),
                        "quot" => "\"".to_string(),
                        "apos" => "'".to_string(),
                        _ => g.resolve_char_ref().ok().flatten().map(|c| c.to_string()).unwrap_or_default(),
                    };
                    push_text(top, &s);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    // Unclosed elements at EOF: close them (lenient).
    while let Some(el) = stack.pop() {
        attach(&mut stack, &mut root, el);
    }
    root.ok_or_else(|| "XML: no root element".into())
}

fn read_el(e: &quick_xml::events::BytesStart<'_>) -> Result<El, String> {
    let mut el = El { name: String::from_utf8_lossy(e.name().as_ref()).into_owned(), attrs: Vec::new(), kids: Vec::new() };
    for a in e.attributes().with_checks(false).flatten().take(512) {
        let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let val =
            a.normalized_value(XmlVersion::Explicit1_0).map(|c| c.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into_owned());
        el.attrs.push((key, val));
    }
    Ok(el)
}

fn push_text(top: &mut El, s: &str) {
    if let Some(Node::Text(prev)) = top.kids.last_mut() {
        prev.push_str(s);
    } else {
        top.kids.push(Node::Text(s.to_string()));
    }
}

fn attach(stack: &mut [El], root: &mut Option<El>, el: El) {
    match stack.last_mut() {
        Some(parent) => parent.kids.push(Node::El(el)),
        None => {
            if root.is_none() {
                *root = Some(el);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_tree_with_local_names() {
        let x = br#"<style xmlns="http://purl.org/net/csl" xml:lang="en-US"><citation><layout prefix="(">)a&amp;b&#65;</layout></citation></style>"#;
        let root = parse(x).unwrap();
        assert_eq!(root.name, "style");
        assert_eq!(root.attr("lang"), Some("en-US"));
        let layout = root.els().next().unwrap().els().next().unwrap();
        assert_eq!(layout.name, "layout");
        assert_eq!(layout.attr("prefix"), Some("("));
        assert_eq!(layout.text(), ")a&bA");
    }

    #[test]
    fn prefixed_names_and_attrs_strip() {
        let x = br#"<c:style xmlns:c="urn:x"><c:citation><c:layout><c:text c:variable="title"/></c:layout></c:citation></c:style>"#;
        let root = parse(x).unwrap();
        let text = root.find_descendant("text").unwrap();
        assert_eq!(text.attr("variable"), Some("title"));
    }

    #[test]
    fn lenient_about_broken_files() {
        // Unclosed elements close at EOF.
        let root = parse(b"<a><b><c>").unwrap();
        assert_eq!(root.find_descendant("c").unwrap().name, "c");
        // A stray end tag is dropped.
        let _ = parse(b"<a></b></a>");
        // Text outside the root, comments, declarations, DOCTYPE are ignored.
        let _ = parse(b"<?xml version=\"1.0\"?><!-- c --><a/>");
    }

    #[test]
    fn junk_is_error_not_panic() {
        for junk in [&b""[..], b"not xml at all", b"<", b"<a", b"<a x='1' x='2'>", &(0..=255u8).collect::<Vec<u8>>()] {
            let _ = parse(junk);
        }
        assert!(parse(b"").is_err());
        assert!(parse(b"not xml at all").is_err());
    }

    #[test]
    fn depth_and_element_caps_error() {
        let deep = format!("{}{}", "<a>".repeat(MAX_DEPTH + 1), "</a>".repeat(MAX_DEPTH + 1));
        let err = parse(deep.as_bytes()).unwrap_err();
        assert!(err.contains("deeper"), "{err}");
        let many = format!("{}{}", "<a/>".repeat(MAX_ELEMENTS + 1), "");
        let err = parse(many.as_bytes()).unwrap_err();
        assert!(err.contains("too many"), "{err}");
    }
}
