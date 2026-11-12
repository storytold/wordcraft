use std::borrow::Cow;
use std::collections::HashSet;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::{El, MAX_BYTES, MAX_DEPTH, MAX_NODES, spend};
use crate::xml::Node;

const MAX_ATTRS: usize = 64;
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

#[derive(Clone, Copy)]
enum Encoding {
    Utf8,
    Le,
    Be,
}

struct Frame {
    el: El,
    bindings: usize,
    base: bool,
    opaque: bool,
}

/// Parse bounded, namespace-aware InkML XML; reject DTDs and malformed document envelopes.
pub(super) fn parse(bytes: &[u8], work: &mut usize) -> Option<El> {
    let (source, encoding) = decode(bytes, work)?;
    let version = initial_version(&source, encoding)?;
    spend(work, source.len().div_ceil(8))?;
    if !source.chars().all(|c| literal(c, version)) {
        return None;
    }
    let text = normalize(&source, version);
    let mut reader = Reader::from_str(&text);
    reader.config_mut().enable_all_checks(true);
    let mut stack: Vec<Frame> = Vec::new();
    let mut bindings = vec![("xml".to_string(), XML_NS.to_string())];
    let mut ids = HashSet::new();
    let mut root = None;
    let mut nodes = 0usize;
    loop {
        let position = reader.buffer_position();
        let event = reader.read_event().ok()?;
        spend(work, 1)?;
        match event {
            Event::Decl(decl) => {
                if position != 0 {
                    return None;
                }
                if declaration(decl.as_ref(), encoding)? != version {
                    return None;
                }
            }
            Event::Start(ref start) | Event::Empty(ref start) => {
                nodes = nodes.checked_add(1)?;
                if nodes > MAX_NODES || stack.len() > MAX_DEPTH || (root.is_some() && stack.is_empty()) {
                    return None;
                }
                let checkpoint = bindings.len();
                let attrs = attributes(start, version, work)?;
                for (key, value) in &attrs {
                    let prefix = if key == "xmlns" {
                        ""
                    } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                        prefix
                    } else {
                        continue;
                    };
                    if prefix == "xmlns"
                        || value == XMLNS_NS
                        || (prefix == "xml") != (value == XML_NS)
                        || (!prefix.is_empty() && value.is_empty() && version != XmlVersion::Explicit1_1)
                        || value.chars().any(char::is_whitespace)
                    {
                        return None;
                    }
                    bindings.push((prefix.to_string(), value.clone()));
                }
                let raw_name = start.name();
                let raw_name = std::str::from_utf8(raw_name.as_ref()).ok()?;
                let (uri, local) = resolve(raw_name, &bindings, false, work)?;
                let mut el = El { name: canonical(&uri, local), ..Default::default() };
                let opaque = stack.last().is_some_and(|f| f.opaque) || matches!(el.name.as_str(), "ink:annotation" | "ink:annotationXML");
                let mut expanded = HashSet::new();
                let mut base = stack.last().is_some_and(|f| f.base);
                for (key, mut value) in attrs {
                    if key == "xmlns" || key.starts_with("xmlns:") {
                        continue;
                    }
                    let (uri, local) = resolve(&key, &bindings, true, work)?;
                    let name = canonical(&uri, local);
                    if !expanded.insert((uri, local.to_string())) {
                        return None;
                    }
                    if name == "xml:id" {
                        value = value.trim_matches(space).to_string();
                        if !ncname(&value) || !ids.insert(value.clone()) {
                            return None;
                        }
                    }
                    if name == "xml:base" && !value.is_empty() {
                        base = true;
                    }
                    el.attrs.push((name, value));
                }
                if !opaque
                    && base
                    && el.name.starts_with("ink:")
                    && el.attrs.iter().any(|(key, value)| key.ends_with("Ref") && value.starts_with('#'))
                {
                    return None;
                }
                if matches!(event, Event::Start(_)) {
                    stack.push(Frame { el, bindings: checkpoint, base, opaque });
                } else {
                    attach(&mut stack, &mut root, el)?;
                    bindings.truncate(checkpoint);
                }
            }
            Event::End(_) => {
                let frame = stack.pop()?;
                bindings.truncate(frame.bindings);
                attach(&mut stack, &mut root, frame.el)?;
            }
            Event::Text(text) => {
                if text.as_ref().windows(3).any(|w| w == b"]]>") {
                    return None;
                }
                content(&mut stack, &text.xml_content(version).ok()?, true)?;
            }
            Event::CData(text) => content(&mut stack, &text.xml_content(version).ok()?, false)?,
            Event::GeneralRef(reference) => {
                let value = match reference.resolve_char_ref().ok()? {
                    Some(c) if character(c, version) => c.to_string(),
                    Some(_) => return None,
                    None => quick_xml::escape::resolve_predefined_entity(&reference.decode().ok()?)?.to_string(),
                };
                content(&mut stack, &value, false)?;
            }
            Event::PI(pi) => {
                let target = std::str::from_utf8(pi.target()).ok()?;
                if !name(target, true) || target.eq_ignore_ascii_case("xml") {
                    return None;
                }
            }
            Event::Comment(_) => {}
            Event::DocType(_) => return None,
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return None;
    }
    root
}

/// Return only a qualified `xml:id`, never an unqualified or foreign `id` attribute.
pub(super) fn id(el: &El) -> Option<&str> {
    el.attrs.iter().find(|(key, _)| key == "xml:id").map(|(_, value)| value.as_str())
}

/// Decode strict UTF-8 or BOM-marked UTF-16, charging and bounding UTF-16 expansion.
fn decode<'a>(bytes: &'a [u8], work: &mut usize) -> Option<(Cow<'a, str>, Encoding)> {
    let (body, encoding) = match bytes.get(..2) {
        Some([0xff, 0xfe]) => (bytes.get(2..)?, Encoding::Le),
        Some([0xfe, 0xff]) => (bytes.get(2..)?, Encoding::Be),
        _ => return Some((Cow::Borrowed(std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)).ok()?), Encoding::Utf8)),
    };
    if body.len() % 2 != 0 {
        return None;
    }
    spend(work, body.len().div_ceil(16))?;
    let mut text = String::new();
    let units = body.as_chunks::<2>().0.iter().map(|pair| match encoding {
        Encoding::Le => u16::from_le_bytes(*pair),
        Encoding::Be => u16::from_be_bytes(*pair),
        Encoding::Utf8 => 0,
    });
    for c in char::decode_utf16(units) {
        let c = c.ok()?;
        if text.len().checked_add(c.len_utf8())? > MAX_BYTES {
            return None;
        }
        text.push(c);
    }
    Some((Cow::Owned(text), encoding))
}

/// Determine the XML version before applying version-specific newline normalization.
fn initial_version(text: &str, encoding: Encoding) -> Option<XmlVersion> {
    let Some(rest) = text.strip_prefix("<?xml") else {
        return Some(XmlVersion::Implicit1_0);
    };
    if !rest.chars().next().is_some_and(|c| space(c) || matches!(c, '\u{85}' | '\u{2028}')) {
        return Some(XmlVersion::Implicit1_0);
    }
    let end = text.find("?>")?;
    let header = normalize(text.get(2..end)?, XmlVersion::Explicit1_1);
    declaration(header.as_bytes(), encoding)
}

/// Normalize literal line endings according to XML 1.0 or 1.1 without expanding references.
fn normalize(text: &str, version: XmlVersion) -> Cow<'_, str> {
    let xml11 = version == XmlVersion::Explicit1_1;
    if !text.chars().any(|c| c == '\r' || (xml11 && matches!(c, '\u{85}' | '\u{2028}'))) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek().is_some_and(|c| *c == '\n' || (xml11 && *c == '\u{85}')) {
                chars.next();
            }
            out.push('\n');
        } else if xml11 && matches!(c, '\u{85}' | '\u{2028}') {
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Validate declaration ordering, version, encoding agreement and standalone values.
fn declaration(raw: &[u8], encoding: Encoding) -> Option<XmlVersion> {
    let start = BytesStart::from_content(std::str::from_utf8(raw).ok()?, 3);
    if !separated(start.attributes_raw()) {
        return None;
    }
    let mut attrs = start.attributes();
    let version = attrs.next()?.ok()?;
    if version.key.as_ref() != b"version" {
        return None;
    }
    let version = match version.value.as_ref() {
        b"1.0" => XmlVersion::Explicit1_0,
        b"1.1" => XmlVersion::Explicit1_1,
        _ => return None,
    };
    let mut encoded = false;
    let mut standalone = false;
    for attribute in attrs {
        let attribute = attribute.ok()?;
        match attribute.key.as_ref() {
            b"encoding" if !encoded && !standalone => {
                let value = std::str::from_utf8(&attribute.value).ok()?;
                let valid = match encoding {
                    Encoding::Utf8 => value.eq_ignore_ascii_case("utf-8"),
                    Encoding::Le => value.eq_ignore_ascii_case("utf-16") || value.eq_ignore_ascii_case("utf-16le"),
                    Encoding::Be => value.eq_ignore_ascii_case("utf-16") || value.eq_ignore_ascii_case("utf-16be"),
                };
                if !valid {
                    return None;
                }
                encoded = true;
            }
            b"standalone" if !standalone && matches!(attribute.value.as_ref(), b"yes" | b"no") => standalone = true,
            _ => return None,
        }
    }
    Some(version)
}

/// Require XML whitespace between quoted attributes and reject unterminated quotes.
fn separated(raw: &[u8]) -> bool {
    if raw.first().is_some_and(|c| !matches!(c, b' ' | b'\t' | b'\r' | b'\n')) {
        return false;
    }
    let mut quote = None;
    for (i, byte) in raw.iter().copied().enumerate() {
        if let Some(open) = quote {
            if byte == open {
                if raw.get(i.saturating_add(1)).is_some_and(|c| !matches!(c, b' ' | b'\t' | b'\r' | b'\n')) {
                    return false;
                }
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        }
    }
    quote.is_none()
}

/// Read a bounded set of unique, normalized attributes with valid qualified names and values.
fn attributes(start: &BytesStart<'_>, version: XmlVersion, work: &mut usize) -> Option<Vec<(String, String)>> {
    if !separated(start.attributes_raw()) {
        return None;
    }
    let mut attrs = Vec::new();
    let mut raw = HashSet::new();
    for attribute in start.attributes().with_checks(false) {
        spend(work, 1)?;
        if attrs.len() >= MAX_ATTRS {
            return None;
        }
        let attribute = attribute.ok()?;
        let key = std::str::from_utf8(attribute.key.as_ref()).ok()?;
        if !qname(key) || !raw.insert(key.to_string()) || attribute.value.contains(&b'<') {
            return None;
        }
        let value = attribute.normalized_value(version).ok()?.into_owned();
        if !value.chars().all(|c| character(c, version)) {
            return None;
        }
        attrs.push((key.to_string(), value));
    }
    Some(attrs)
}

/// Resolve a qualified name against scoped bindings; default namespaces do not apply to attributes.
fn resolve<'a>(raw: &'a str, bindings: &[(String, String)], attribute: bool, work: &mut usize) -> Option<(String, &'a str)> {
    if !qname(raw) {
        return None;
    }
    let (prefix, local) = raw.split_once(':').unwrap_or(("", raw));
    if prefix == "xmlns" {
        return None;
    }
    if attribute && prefix.is_empty() {
        return Some((String::new(), local));
    }
    for (key, value) in bindings.iter().rev() {
        spend(work, 1)?;
        if key == prefix {
            if value.is_empty() && !prefix.is_empty() {
                return None;
            }
            spend(work, value.len().div_ceil(16))?;
            return Some((value.clone(), local));
        }
    }
    prefix.is_empty().then(|| (String::new(), local))
}

/// Assign known namespace prefixes and mark foreign names so they cannot impersonate InkML.
fn canonical(uri: &str, local: &str) -> String {
    let prefix = match uri {
        "" => return local.to_string(),
        "http://www.w3.org/2003/InkML" => "ink",
        "http://www.w3.org/1998/Math/MathML" => "mml",
        XML_NS => "xml",
        _ => "?",
    };
    format!("{prefix}:{local}")
}

/// Attach a completed element to its parent or establish the document's single root.
fn attach(stack: &mut [Frame], root: &mut Option<El>, el: El) -> Option<()> {
    if let Some(parent) = stack.last_mut() {
        parent.el.kids.push(Node::El(el));
    } else if root.is_none() {
        *root = Some(el);
    } else {
        return None;
    }
    Some(())
}

/// Merge adjacent text inside elements; only literal whitespace is allowed outside the root.
fn content(stack: &mut [Frame], value: &str, outside: bool) -> Option<()> {
    if let Some(top) = stack.last_mut() {
        if let Some(Node::Text(text)) = top.el.kids.last_mut() {
            text.push_str(value);
        } else {
            top.el.kids.push(Node::Text(value.to_string()));
        }
        Some(())
    } else {
        (outside && value.chars().all(space)).then_some(())
    }
}

/// Recognize the four XML whitespace characters.
fn space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Check whether a character is permitted by the selected XML version, including references.
fn character(c: char, version: XmlVersion) -> bool {
    match version {
        XmlVersion::Explicit1_1 => matches!(c, '\u{1}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'),
        _ => space(c) || matches!(c, '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'),
    }
}

/// Exclude XML 1.1 restricted characters that must be written as character references.
fn literal(c: char, version: XmlVersion) -> bool {
    character(c, version)
        && (version != XmlVersion::Explicit1_1
            || !matches!(c, '\u{1}'..='\u{8}' | '\u{b}'..='\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}'..='\u{84}' | '\u{86}'..='\u{9f}'))
}

/// Validate a qualified name with at most one colon separating nonempty NCNames.
fn qname(value: &str) -> bool {
    match value.split_once(':') {
        Some((prefix, local)) => ncname(prefix) && ncname(local),
        None => ncname(value),
    }
}

/// Validate an XML name that cannot contain a namespace separator.
fn ncname(value: &str) -> bool {
    name(value, false)
}

/// Validate XML name start and continuation characters, optionally allowing colons.
fn name(value: &str, colon: bool) -> bool {
    let mut chars = value.chars();
    let start = |c| {
        matches!(c, 'A'..='Z' | '_' | 'a'..='z' | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
            | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}'
            | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}'
            | '\u{10000}'..='\u{effff}')
            || (colon && c == ':')
    };
    chars.next().is_some_and(start)
        && chars.all(|c| start(c) || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}'))
}
