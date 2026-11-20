//! A small element tree in the shape of Python's ElementTree (text before the first child,
//! tail after each element), which the converter walks. Names are normalised to the canonical
//! prefixes in [`NAMESPACES`] ("hp:p" whatever prefix the file uses), `hp:switch` blocks are
//! resolved to the branch we read, and section paragraphs are numbered ([`ORDINAL`]).
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use craft_xml::{Event, Options};

use crate::error::{Error, Result};

/// Namespace URI -> the prefix the code uses for it.
pub const NAMESPACES: &[(&str, &str)] = &[
    ("http://www.hancom.co.kr/hwpml/2011/version", "hv"),
    ("http://www.hancom.co.kr/hwpml/2011/app", "ha"),
    ("http://www.hancom.co.kr/hwpml/2011/paragraph", "hp"),
    ("http://www.hancom.co.kr/hwpml/2016/paragraph", "hp10"),
    ("http://www.hancom.co.kr/hwpml/2011/section", "hs"),
    ("http://www.hancom.co.kr/hwpml/2011/core", "hc"),
    ("http://www.hancom.co.kr/hwpml/2011/head", "hh"),
    ("http://www.hancom.co.kr/hwpml/2011/history", "hhs"),
    ("http://www.hancom.co.kr/hwpml/2011/master-page", "hm"),
    ("http://www.hancom.co.kr/schema/2011/hpf", "hpf"),
    ("http://purl.org/dc/elements/1.1/", "dc"),
    ("http://www.idpf.org/2007/opf/", "opf"),
    ("http://www.idpf.org/2007/opf", "opf"),
    ("urn:oasis:names:tc:opendocument:xmlns:container", "ocf"),
    ("urn:oasis:names:tc:opendocument:xmlns:manifest:1.0", "odf"),
    ("urn:oasis:names:tc:opendocument:xmlns:config:1.0", "config"),
    ("http://www.w3.org/1999/02/22-rdf-syntax-ns#", "rdf"),
    ("http://www.hancom.co.kr/hwpml/2016/ooxmlchart", "ooxmlchart"),
    ("http://www.idpf.org/2007/ops", "epub"),
    ("http://www.hancom.co.kr/hwpml/2016/HwpUnitChar", "hwpunitchar"),
    // WordprocessingML, for reading back the DOCX of an edited document (writer.rs)
    ("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "w"),
    ("http://schemas.openxmlformats.org/markup-compatibility/2006", "mc"),
];

/// Attribute added to each section paragraph: its place among the `p` start tags of the raw
/// part, counted before `hp:switch` branches are dropped, so the writer finds it again.
pub const ORDINAL: &str = "hpord";

/// `hp:case` branches we read. In HwpUnitChar blocks the `hp:default` copy of every paragraph
/// margin is doubled, and in 2016-paragraph blocks `hp:default` drops outline headings (real
/// files), so `hp:case` is the authoritative copy.
const KNOWN_CASES: &[&str] = &["http://www.hancom.co.kr/hwpml/2016/HwpUnitChar", "http://www.hancom.co.kr/hwpml/2016/paragraph"];

const MAX_DEPTH: usize = 256;
const MAX_ELEMENTS: usize = 8_000_000;
const MAX_ATTRS: usize = 256;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    /// Text before the first child.
    pub text: String,
    /// Text after this element, before the next sibling.
    pub tail: String,
    pub children: Vec<Element>,
}

impl Element {
    pub fn new(tag: &str) -> Self {
        Element { tag: tag.to_string(), ..Default::default() }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.attrs.iter_mut().find(|(k, _)| k == key) {
            Some(a) => a.1 = value,
            None => self.attrs.push((key.to_string(), value)),
        }
    }

    /// First child named `tag`.
    pub fn find(&self, tag: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.tag == tag)
    }

    /// Children named `tag`.
    pub fn findall<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.tag == tag)
    }

    /// This element and its descendants, in document order.
    pub fn iter_all(&self) -> Descendants<'_> {
        Descendants { stack: vec![self] }
    }

    /// [`Self::iter_all`] named `tag`.
    pub fn iter<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.iter_all().filter(move |e| e.tag == tag)
    }

    /// The element as XML text (its tail not included), `extra` added to its start tag (namespace
    /// declarations for a root). Elements nested deeper than the parser follows are left out.
    pub fn to_xml(&self, extra: &str) -> String {
        let mut out = String::new();
        self.write_xml(&mut out, extra, 0);
        out
    }

    fn write_xml(&self, out: &mut String, extra: &str, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        out.push('<');
        out.push_str(&self.tag);
        if !extra.is_empty() {
            out.push(' ');
            out.push_str(extra);
        }
        for (k, v) in &self.attrs {
            out.push_str(&format!(" {k}={}", crate::docx::attr(v)));
        }
        if self.text.is_empty() && self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        out.push_str(&crate::docx::esc(&self.text));
        for c in &self.children {
            c.write_xml(out, "", depth + 1);
            out.push_str(&crate::docx::esc(&c.tail));
        }
        out.push_str(&format!("</{}>", self.tag));
    }
}

pub struct Descendants<'a> {
    stack: Vec<&'a Element>,
}

impl<'a> Iterator for Descendants<'a> {
    type Item = &'a Element;
    fn next(&mut self) -> Option<&'a Element> {
        let e = self.stack.pop()?;
        self.stack.extend(e.children.iter().rev());
        Some(e)
    }
}

fn canonical(uri: &str) -> Option<&'static str> {
    NAMESPACES.iter().find(|(u, _)| *u == uri).map(|(_, p)| *p)
}

/// In-scope namespace declarations, innermost last.
struct Scopes(Vec<Vec<(String, String)>>);

impl Scopes {
    fn uri(&self, prefix: &str) -> Option<&str> {
        self.0.iter().rev().flat_map(|s| s.iter().rev()).find(|(p, _)| p == prefix).map(|(_, u)| u.as_str())
    }

    /// "pfx:local" with the canonical prefix of pfx's namespace (unknown ones kept as written).
    fn name(&self, raw: &str, is_attr: bool) -> String {
        let (prefix, local) = match raw.split_once(':') {
            Some((p, l)) => (p, l),
            None if is_attr => return raw.to_string(), // unprefixed attributes have no namespace
            None => ("", raw),
        };
        match self.uri(prefix).and_then(canonical) {
            Some(c) => format!("{c}:{local}"),
            None => raw.to_string(),
        }
    }
}

fn open(name: &str, attrs: Vec<(String, String)>, scopes: &mut Scopes, ordinal: &mut usize, number: bool) -> Element {
    let mut decls = Vec::new();
    let mut raw_attrs = Vec::new();
    for (k, v) in attrs.into_iter().take(MAX_ATTRS) {
        if k == "xmlns" {
            decls.push((String::new(), v));
        } else if let Some(p) = k.strip_prefix("xmlns:") {
            decls.push((p.to_string(), v));
        } else {
            raw_attrs.push((k, v));
        }
    }
    scopes.0.push(decls);
    let tag = scopes.name(name, false);
    let mut el = Element { tag, ..Default::default() };
    el.attrs = raw_attrs.into_iter().map(|(k, v)| (scopes.name(&k, true), v)).collect();
    if number && el.tag == "hp:p" {
        el.set(ORDINAL, ordinal.to_string());
        *ordinal += 1;
    }
    el
}

fn push_text(stack: &mut [Element], s: &str) {
    if let Some(top) = stack.last_mut() {
        match top.children.last_mut() {
            Some(last) => last.tail.push_str(s),
            None => top.text.push_str(s),
        }
    }
}

fn close(stack: &mut Vec<Element>, scopes: &mut Scopes) -> Option<Element> {
    scopes.0.pop();
    let done = stack.pop()?;
    match stack.last_mut() {
        Some(parent) => {
            parent.children.push(done);
            None
        }
        None => Some(done),
    }
}

/// Parse a part. `number_paragraphs` marks section paragraphs with [`ORDINAL`].
pub fn parse(xml: &[u8], number_paragraphs: bool) -> Result<Element> {
    // Lenient, as Hangul files need: mismatched or stray end tags, unclosed elements, unknown
    // entities dropped. Namespace declarations stay attributes so `Scopes` maps the prefixes.
    let opts = Options::default();
    let mut events = opts.events(xml);
    let mut stack: Vec<Element> = Vec::new();
    let mut scopes = Scopes(Vec::new());
    let mut root = None;
    let (mut count, mut ordinal) = (0usize, 0usize);
    loop {
        let ev = match events.next() {
            Ok(Some(ev)) => ev,
            Ok(None) => break,
            Err(craft_xml::Error::TooDeep(_)) => return Err(Error::TooLarge("nesting")),
            Err(e) if e.is_limit() => return Err(Error::TooLarge("XML part")),
            Err(e) => return Err(Error::format(format!("XML: {e}"))),
        };
        match ev {
            Event::Start { .. } if root.is_some() => break, // content after the root
            Event::Start { name, attrs } => {
                count += 1;
                if count > MAX_ELEMENTS {
                    return Err(Error::TooLarge("elements"));
                }
                let el = open(&name, attrs, &mut scopes, &mut ordinal, number_paragraphs);
                stack.push(el);
            }
            Event::End { .. } => {
                if stack.is_empty() {
                    continue;
                }
                if let Some(done) = close(&mut stack, &mut scopes) {
                    root = Some(done);
                }
            }
            Event::Text(t) | Event::CData(t) => push_text(&mut stack, &t),
            _ => {}
        }
    }
    // An unclosed document: close what is open.
    while root.is_none() && !stack.is_empty() {
        root = close(&mut stack, &mut scopes);
    }
    let mut root = root.ok_or_else(|| Error::format("empty XML part"))?;
    resolve_switches(&mut root, 0);
    Ok(root)
}

fn branch(switch: &Element) -> Option<&Element> {
    let cases = switch.findall("hp:case");
    for c in cases {
        let ns = c.get("hp:required-namespace").or_else(|| c.get("required-namespace")).unwrap_or("");
        if KNOWN_CASES.contains(&ns) {
            return Some(c);
        }
    }
    switch.find("hp:default").or_else(|| switch.find("hp:case"))
}

/// Replace each `hp:switch` by the children of the branch we read (see [`KNOWN_CASES`]).
fn resolve_switches(el: &mut Element, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    if el.children.iter().any(|c| c.tag == "hp:switch") {
        let mut out = Vec::with_capacity(el.children.len());
        for c in std::mem::take(&mut el.children) {
            if c.tag != "hp:switch" {
                out.push(c);
            } else if let Some(b) = branch(&c) {
                out.extend(b.children.iter().cloned());
            }
        }
        el.children = out;
    }
    for c in &mut el.children {
        resolve_switches(c, depth + 1);
    }
}
