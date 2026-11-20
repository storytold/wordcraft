//! Write a document edited in WordCraft back into its original HWPX.
//!
//! The converter marks every DOCX paragraph with a hidden bookmark naming the HWPX paragraph it
//! came from ("_hp_<section>_<ordinal>", see `Converter::mark_origin`). To save, the original is
//! converted again (the "before" picture) and compared with the edited DOCX paragraph by
//! paragraph:
//!
//! - unchanged paragraphs keep their original XML byte for byte;
//! - a changed paragraph gets its text runs rebuilt: typed characters take the character shape
//!   of their neighbour, deleted ones go, tabs / line breaks / full-width spaces keep their
//!   original elements, and the paragraph's layout cache (hp:linesegarray) is dropped so Hancom
//!   lays it out again;
//! - a new paragraph copies the shape of the paragraph before it; a deleted one is removed.
//!
//! Only "plain" paragraphs are rewritten: text runs that hold nothing but text, tabs, line breaks
//! and spaces, not interleaved with fields, notes or objects, and whose text the converter shows
//! unchanged. Page controls among the text (page hiding, page numbers) are kept in a run of their
//! own after it. Edits elsewhere are reported, not written. The original XML is never
//! re-serialised: edits are spliced into the raw text of the section parts, and every other part
//! of the package keeps its content (written again in order, stored or deflated as it was).
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::docx::esc;
use crate::error::{Error, Result};
use crate::package::Package;
use crate::xml::Element;

const P_NS: &str = "http://www.hancom.co.kr/hwpml/2011/paragraph";
const C_NS: &str = "http://www.hancom.co.kr/hwpml/2011/core";
/// What Hancom writes for most paragraphs.
const NEW_PARAGRAPH_ID: &str = "2147483648";
/// Comparing two texts takes about (length of both) × (edits) steps. Past this many, or past
/// `MAX_DIFF_EDITS` edits, the part between the common start and end counts as one replaced
/// block (hostile sizes; the edited text is still written whole).
const MAX_DIFF_CELLS: usize = 50_000_000;
/// The most single-character edits looked for between two texts; the search keeps up to
/// (edits + 1)² positions (4 bytes each), so this bounds its memory to about 16 MB.
const MAX_DIFF_EDITS: usize = 2_000;
const MAX_NODES: usize = 8_000_000;

/// What a write-back did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveReport {
    pub modified: usize,
    pub inserted: usize,
    pub deleted: usize,
    pub unchanged: usize,
    /// Paragraphs given another paragraph shape (alignment, indents, spacing).
    pub restyled: usize,
    /// Tables written again or added.
    pub tables: usize,
    /// Pictures added, replaced, resized or removed.
    pub pictures: usize,
    /// Edits not written, with why ("section 1, paragraph 12: text and objects in one run").
    pub skipped: Vec<String>,
}

/// (kind, section, ordinal); kind is "hp" (a paragraph's first DOCX paragraph), "hq" (a further
/// piece of it) or "ht" (a paragraph shown only as a table).
pub(crate) type Mark = (&'static str, usize, usize);

pub(crate) fn parse_mark(name: &str) -> Option<Mark> {
    let rest = name.strip_prefix('_')?;
    let (kind, rest) = rest.split_once('_')?;
    let kind = match kind {
        "hp" => "hp",
        "hq" => "hq",
        "ht" => "ht",
        _ => return None,
    };
    let (s, o) = rest.split_once('_')?;
    let digits = |v: &str| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit());
    if !digits(s) || !digits(o) {
        return None;
    }
    Some((kind, s.parse().ok()?, o.parse().ok()?))
}

// ==== the edited DOCX ======================================================================

#[derive(Clone, Debug)]
struct DocPara {
    /// Paragraphs in the same body / cell / text box share this.
    container: String,
    /// None: a mark between paragraphs (a table's source paragraph).
    text: Option<String>,
    marks: Vec<Mark>,
    /// Pictures in the paragraph itself.
    pictures: usize,
}

/// Text, bookmarks and pictures of a paragraph, not of what is nested in it (text boxes).
fn own(el: &Element, out: &mut String, marks: &mut Vec<Mark>, pictures: &mut usize, depth: usize) {
    if depth > 256 {
        return;
    }
    for c in &el.children {
        match c.tag.as_str() {
            // pictures, drawings and groups (a text box comes wrapped in mc:AlternateContent)
            "w:drawing" => *pictures += 1,
            "mc:AlternateContent" => *pictures += usize::from(c.iter("w:drawing").next().is_some()),
            "w:txbxContent" | "w:pict" | "w:del" | "w:moveFrom" | "w:instrText" | "w:p" => {}
            "w:t" => out.push_str(&c.text),
            "w:tab" => out.push('\t'),
            "w:br" | "w:cr" => {
                if matches!(c.get("w:type"), None | Some("textWrapping")) {
                    out.push('\n');
                }
            }
            "w:noBreakHyphen" => out.push('-'),
            "w:bookmarkStart" => {
                // table marks ("ht") are taken out again by docx_paragraphs
                if let Some(m) = c.get("w:name").and_then(parse_mark) {
                    marks.push(m);
                }
            }
            _ => own(c, out, marks, pictures, depth + 1),
        }
    }
}

struct Walker<'o> {
    out: &'o mut Vec<DocPara>,
    name: String,
    counter: usize,
    /// Table marks already given a place.
    placed: HashSet<Mark>,
}

impl Walker<'_> {
    fn table_mark(&mut self, key: Mark, container: &str) {
        if self.placed.insert(key) {
            self.out.push(DocPara { container: container.to_string(), text: None, marks: vec![key], pictures: 0 });
        }
    }

    fn walk(&mut self, el: &Element, container: &str, in_p: bool, depth: usize) {
        if depth > 256 {
            return;
        }
        let mut prev_mark = false; // the previous sibling was a table mark between paragraphs
        for c in &el.children {
            let was = std::mem::replace(&mut prev_mark, false);
            let tag = c.tag.as_str();
            if tag == "w:p" {
                let (mut text, mut marks, mut pictures) = (String::new(), Vec::new(), 0);
                own(c, &mut text, &mut marks, &mut pictures, 0);
                let tables: Vec<Mark> = marks.iter().copied().filter(|m| m.0 == "ht").collect();
                marks.retain(|m| m.0 != "ht");
                self.out.push(DocPara { container: container.to_string(), text: Some(text), marks, pictures });
                for key in tables {
                    // moved into a paragraph by WordCraft: the table follows
                    self.table_mark(key, container);
                }
                self.walk(c, container, true, depth + 1); // text boxes inside it
            } else if in_p && tag != "w:txbxContent" {
                self.walk(c, container, true, depth + 1); // only looking for text boxes here
            } else if matches!(tag, "w:tc" | "w:txbxContent" | "w:footnote" | "w:endnote" | "w:comment") {
                self.counter += 1;
                let inner = format!("{}#{}", self.name, self.counter);
                self.walk(c, &inner, false, depth + 1);
            } else if tag == "mc:Fallback" {
                // the same content as mc:Choice
            } else if let Some(m) = (tag == "w:bookmarkStart").then(|| c.get("w:name").and_then(parse_mark)).flatten() {
                // between paragraphs: a source paragraph shown only as a table
                self.table_mark(m, container);
                prev_mark = true;
            } else if matches!(tag, "w:bookmarkEnd" | "w:proofErr") {
                prev_mark = was;
            } else if tag == "w:tbl" {
                if !was {
                    // moved by WordCraft into the table's first paragraph
                    let first = c.iter("w:p").next();
                    let m = first
                        .into_iter()
                        .flat_map(|p| p.iter("w:bookmarkStart"))
                        .filter_map(|b| b.get("w:name").and_then(parse_mark))
                        .find(|m| m.0 == "ht");
                    if let Some(m) = m {
                        self.table_mark(m, container);
                    }
                }
                self.walk(c, container, false, depth + 1);
            } else {
                self.walk(c, container, false, depth + 1);
            }
        }
    }
}

/// Story parts of a DOCX, the document first.
fn story_part(name: &str) -> bool {
    let Some(stem) = name.strip_prefix("word/").and_then(|n| n.strip_suffix(".xml")) else { return false };
    let numbered = |p: &str| stem.strip_prefix(p).is_some_and(|d| d.bytes().all(|b| b.is_ascii_digit()));
    matches!(stem, "document" | "footnotes" | "endnotes" | "comments") || numbered("header") || numbered("footer")
}

/// Every paragraph of the document's stories, in order, with its container.
fn docx_paragraphs(docx: &[u8]) -> Result<Vec<DocPara>> {
    let mut z = craft_zip::Archive::open(docx, ZIP_LIMITS).map_err(|e| Error::format(format!("not a DOCX: {e}")))?;
    let mut names: Vec<String> = z.names().filter(|n| story_part(n)).map(|n| n.into_owned()).collect();
    names.sort_by(|a, b| (a != "word/document.xml", a).cmp(&(b != "word/document.xml", b)));
    let mut out = Vec::new();
    for name in names {
        let Some(data) = z.read(&name, MAX_PART).map_err(Error::format)? else { continue };
        let root = crate::xml::parse(&data, false)?;
        let mut w = Walker { out: &mut out, name: name.clone(), counter: 0, placed: HashSet::new() };
        // the root element itself is walked like Python's walk(root, name): its children
        let wrapper = Element { children: vec![root], ..Default::default() };
        w.walk(&wrapper, &name, false, 0);
    }
    Ok(out)
}

/// Where a paragraph without a source mark sits: ("after", mark, k) for the k-th one after a
/// marked paragraph of its container, ("head", mark, k) before the first marked one.
type Place = (&'static str, Option<Mark>, usize);

/// Paragraphs without a source mark (indices into `paras`), each with its place.
fn unmarked(paras: &[DocPara]) -> Vec<(usize, Place)> {
    let mut order: Vec<&str> = Vec::new();
    let mut by_container: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, d) in paras.iter().enumerate() {
        by_container
            .entry(d.container.as_str())
            .or_insert_with(|| {
                order.push(d.container.as_str());
                Vec::new()
            })
            .push(i);
    }
    let mut out = Vec::new();
    for c in order {
        let Some(items) = by_container.get(c) else { continue };
        let (mut prev, mut k, mut head): (Option<Mark>, usize, Vec<usize>) = (None, 0, Vec::new());
        for &i in items {
            let Some(d) = paras.get(i) else { continue };
            let primary = d.marks.iter().copied().find(|m| m.0 == "hp" || m.0 == "ht");
            if primary.is_some() || d.marks.iter().any(|m| m.0 == "hq") {
                if let Some(p) = primary {
                    for (j, h) in head.drain(..).enumerate() {
                        out.push((h, ("head", Some(p), j)));
                    }
                    prev = Some(p);
                    k = 0;
                }
                continue;
            }
            match prev {
                None => head.push(i),
                Some(p) => {
                    out.push((i, ("after", Some(p), k)));
                    k += 1;
                }
            }
        }
        out.extend(head.into_iter().enumerate().map(|(j, h)| (h, ("head", None, j))));
    }
    out
}

// ==== the original section XML ===============================================================

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) name: String,
    pub(crate) start: usize,
    /// End of the start tag.
    pub(crate) tag_end: usize,
    pub(crate) parent: Option<usize>,
    /// End of the element.
    pub(crate) end: usize,
    /// Start of the end tag.
    pub(crate) inner_end: usize,
    pub(crate) children: Vec<usize>,
}

fn name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '-'
}

/// One tag at `at` (which holds '<'): (closing, name, self-closing, end), or None if the text
/// there is no tag. Attribute values may hold '>' inside quotes.
fn scan_tag(text: &str, at: usize) -> Option<(bool, String, bool, usize)> {
    let rest = text.get(at + 1..)?;
    let (closing, rest, base) = match rest.strip_prefix('/') {
        Some(r) => (true, r, at + 2),
        None => (false, rest, at + 1),
    };
    let mut n = rest.chars().take_while(|&c| name_char(c)).map(char::len_utf8).sum::<usize>();
    if n == 0 {
        return None;
    }
    if rest.get(n..).is_some_and(|r| r.starts_with(':')) {
        let m = rest.get(n + 1..)?.chars().take_while(|&c| name_char(c)).map(char::len_utf8).sum::<usize>();
        if m > 0 {
            n += 1 + m;
        }
    }
    let name = rest.get(..n)?.to_string();
    let mut i = base + n;
    loop {
        let r = text.get(i..)?;
        if r.starts_with("/>") {
            return Some((closing, name, true, i + 2));
        }
        let c = r.chars().next()?;
        match c {
            '>' => return Some((closing, name, false, i + 1)),
            '"' | '\'' => {
                let close = r.get(1..)?.find(c)?;
                i += 1 + close + 1;
            }
            _ => i += c.len_utf8(),
        }
    }
}

/// Elements of `text` with their byte spans (comments, processing instructions and CDATA are
/// skipped).
pub(crate) fn parse_spans(text: &str) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while let Some(off) = text.get(i..).and_then(|r| r.find('<')) {
        let at = i + off;
        let r = text.get(at..).unwrap_or("");
        let skip = [("<!--", "-->"), ("<?", "?>"), ("<![CDATA[", "]]>")].iter().find(|(o, _)| r.starts_with(o));
        if let Some((o, c)) = skip {
            match r.get(o.len()..).and_then(|x| x.find(c)) {
                Some(e) => {
                    i = at + o.len() + e + c.len();
                    continue;
                }
                None => {
                    if *o == "<?" || *o == "<!--" || *o == "<![CDATA[" {
                        // unterminated: not a tag here; go on after '<'
                        i = at + 1;
                        continue;
                    }
                }
            }
        }
        let Some((closing, name, self_closing, end)) = scan_tag(text, at) else {
            i = at + 1;
            continue;
        };
        i = end;
        if closing {
            while let Some(top) = stack.pop() {
                if let Some(n) = nodes.get_mut(top) {
                    n.inner_end = at;
                    n.end = end;
                    if n.name == name {
                        break;
                    }
                }
            }
            continue;
        }
        if nodes.len() >= MAX_NODES {
            break;
        }
        let id = nodes.len();
        let parent = stack.last().copied();
        nodes.push(Node { name, start: at, tag_end: end, parent, end: text.len(), inner_end: text.len(), children: Vec::new() });
        if let Some(p) = parent.and_then(|p| nodes.get_mut(p)) {
            p.children.push(id);
        }
        if self_closing {
            if let Some(n) = nodes.get_mut(id) {
                n.end = end;
                n.inner_end = end;
            }
        } else {
            stack.push(id);
        }
    }
    nodes
}

/// `name="value"` pairs of a start tag (a later one of the same name wins).
pub(crate) fn tag_attrs(tag: &str) -> HashMap<String, String> {
    attr_spans(tag).into_iter().map(|(name, a, b)| (name.to_string(), tag.get(a..b).unwrap_or("").to_string())).collect()
}

/// The attributes of start tag `tag` (`<name a="1" ...>`), in order: each name and the byte span
/// of its value, which XML lets stand in either quote with spaces around `=` (`a = '1'`).
pub(crate) fn attr_spans(tag: &str) -> Vec<(&str, usize, usize)> {
    let b = tag.as_bytes();
    let at = |i: usize| b.get(i).copied();
    let space = |i: usize| at(i).is_some_and(|c| c.is_ascii_whitespace());
    let mut out = Vec::new();
    // past the element's name
    let mut i = tag.find(|c: char| c.is_ascii_whitespace()).unwrap_or(tag.len());
    loop {
        while space(i) {
            i += 1;
        }
        let start = i;
        while at(i).is_some_and(|c| !matches!(c, b'=' | b'/' | b'>') && !c.is_ascii_whitespace()) {
            i += 1;
        }
        if i == start {
            break;
        }
        // the delimiters are ASCII, so the name ends on a character boundary
        let name = tag.get(start..i).unwrap_or("");
        while space(i) {
            i += 1;
        }
        if at(i) != Some(b'=') {
            continue;
        }
        i += 1;
        while space(i) {
            i += 1;
        }
        let Some(q) = at(i).filter(|&c| c == b'"' || c == b'\'') else { break };
        let from = i + 1;
        let Some(len) = b.get(from..).and_then(|r| r.iter().position(|&c| c == q)) else { break };
        out.push((name, from, from + len));
        i = from + len + 1;
    }
    out
}

/// XML text -> characters (the predefined entities and character references).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i..).unwrap_or("");
        let Some(semi) = after.find(';').filter(|&k| k <= 12) else {
            out.push('&');
            rest = after.get(1..).unwrap_or("");
            continue;
        };
        let ent = after.get(1..semi).unwrap_or("");
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                let code = if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                    u32::from_str_radix(h, 16).ok()
                } else if let Some(d) = ent.strip_prefix('#') {
                    d.parse::<u32>().ok()
                } else {
                    None
                };
                code.and_then(char::from_u32)
            }
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = after.get(semi + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// The names of the paragraph element in this part: `<prefix>:p` for each prefix declared for
/// the paragraph namespace, and `p` when it is the default namespace ("hp:p" when none is).
fn paragraph_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut from = 0;
    while let Some(i) = text.get(from..).and_then(|r| r.find("xmlns")) {
        let s = from + i + "xmlns".len();
        let rest = text.get(s..).unwrap_or("");
        let (name, after) = match rest.strip_prefix(':') {
            Some(r) => {
                let n = r.chars().take_while(|&c| name_char(c)).map(char::len_utf8).sum::<usize>();
                (Some(r.get(..n).unwrap_or("")), r.get(n..).unwrap_or(""))
            }
            None => (None, rest),
        };
        let value = after.trim_start().strip_prefix('=').map(str::trim_start).and_then(|v| {
            let q = v.chars().next().filter(|&c| c == '"' || c == '\'')?;
            v.get(1..).and_then(|r| r.split(q).next())
        });
        if value == Some(P_NS) {
            let tag = match name {
                Some(n) if !n.is_empty() => format!("{n}:p"),
                _ => "p".to_string(),
            };
            if !names.contains(&tag) {
                names.push(tag);
            }
        }
        from = s;
    }
    if names.is_empty() {
        names.push("hp:p".to_string());
    }
    names
}

/// The prefix this part uses for the paragraph namespace (normally "hp").
fn paragraph_prefix(text: &str) -> String {
    let want = format!("=\"{P_NS}\"");
    let mut from = 0;
    while let Some(i) = text.get(from..).and_then(|r| r.find("xmlns:")) {
        let s = from + i + "xmlns:".len();
        let n = text.get(s..).unwrap_or("").chars().take_while(|&c| name_char(c)).map(char::len_utf8).sum::<usize>();
        if n > 0 && text.get(s + n..).is_some_and(|r| r.starts_with(&want)) {
            return text.get(s..s + n).unwrap_or("hp").to_string();
        }
        from = s;
    }
    "hp".to_string()
}

/// (character shape id, hp:t attributes)
type Key = (String, String);

/// One editable character: (char, key, its raw element: a tab or line break kept as written).
type Ch = (char, Key, Option<String>);

/// A paragraph's text runs, character by character.
struct Plain {
    chars: Vec<Ch>,
    /// (start, end) of the text runs, or an empty range where to add them.
    region: (usize, usize),
    /// Written before the runs: ">" when the paragraph is an empty tag (`<hp:p .../>`), whose
    /// "/>" is the region; `tail` then ends with its end tag.
    lead: String,
    /// Character shape for text typed into an empty paragraph.
    key: Key,
    /// Span of hp:linesegarray.
    lineseg: Option<(usize, usize)>,
    tab_raw: Option<String>,
    /// The region starts inside a run with settings (see runs_xml).
    open_key: Option<(Key, String)>,
    /// Groups of text runs with objects or fields between them: each one's (start, end) in the
    /// XML and its characters' range in `chars`. One group: `region` holds it.
    segments: Vec<((usize, usize), (usize, usize))>,
    /// Page controls found among the text of the region's runs (see [`PAGE_CONTROLS`]), as a run
    /// of their own to write after the rewritten text; empty when there are none.
    tail: String,
}

/// Controls that act on the page or section, not at a place in the text: nothing of them is in
/// the converted paragraph, so the text around them can be rewritten with them kept in a run of
/// their own after it. (Hancom puts the section's controls in such a run, before the text.)
const PAGE_CONTROLS: &[&str] = &["colPr", "pageHiding", "pageNum", "newNum"];

fn special(local: &str) -> Option<char> {
    Some(match local {
        "tab" => '\t',
        "lineBreak" => '\n',
        "fwSpace" => '\u{3000}',
        "nbSpace" => '\u{a0}',
        _ => return None,
    })
}

struct Section {
    text: String,
    pfx: String,
    nodes: Vec<Node>,
    paras: Vec<usize>,
}

impl Section {
    fn new(text: String) -> Self {
        let pfx = paragraph_prefix(&text);
        let nodes = parse_spans(&text);
        // every name the paragraph element goes by here, as the reader numbers them all (two
        // prefixes may name the namespace, or it may be the default one)
        let names = paragraph_names(&text);
        let paras = nodes.iter().enumerate().filter(|(_, n)| names.contains(&n.name)).map(|(i, _)| i).collect();
        Section { text, pfx, nodes, paras }
    }

    fn node(&self, i: usize) -> Option<&Node> {
        self.nodes.get(i)
    }

    fn local_of<'a>(&self, n: &'a Node) -> &'a str {
        n.name.split_once(':').map(|(_, l)| l).unwrap_or(&n.name)
    }

    fn local(&self, i: usize) -> &str {
        self.node(i).map(|n| n.name.split_once(':').map(|(_, l)| l).unwrap_or(&n.name)).unwrap_or("")
    }

    /// `xml` (a table, picture or drawing the HWPX writer made, with the `hp` and `hc` prefixes)
    /// with those prefixes bound on its outer element where this part doesn't bind them itself
    /// (one using other prefixes for the same namespaces): the fragment then reads the same.
    fn fragment(&self, xml: &str) -> String {
        let decls: String = [("hp", P_NS), ("hc", C_NS)]
            .iter()
            .filter(|(p, ns)| xml.contains(&format!("<{p}:")) && !self.text.contains(&format!("xmlns:{p}=\"{ns}\"")))
            .map(|(p, ns)| format!(" xmlns:{p}=\"{ns}\""))
            .collect();
        let name_end = xml.find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/');
        match name_end.filter(|_| !decls.is_empty() && xml.starts_with('<')) {
            Some(at) => format!("{}{decls}{}", xml.get(..at).unwrap_or(""), xml.get(at..).unwrap_or("")),
            None => xml.to_string(),
        }
    }

    fn kids(&self, i: usize, local: Option<&str>) -> Vec<usize> {
        self.node(i).map(|n| n.children.iter().copied().filter(|&k| local.is_none_or(|l| self.local(k) == l)).collect()).unwrap_or_default()
    }

    fn slice(&self, a: usize, b: usize) -> &str {
        self.text.get(a..b.max(a)).unwrap_or("")
    }

    fn attr(&self, i: usize, name: &str, default: &str) -> String {
        let Some(n) = self.node(i) else { return default.to_string() };
        tag_attrs(self.slice(n.start, n.tag_end)).remove(name).unwrap_or_else(|| default.to_string())
    }

    /// Paragraphs inside `i` other than those of its tables (a text box, a note): the paragraph
    /// holds more than tables.
    fn has_paragraph_inside_outside_tables(&self, i: usize) -> bool {
        let Some(n) = self.node(i) else { return false };
        let tables: Vec<(usize, usize)> =
            self.nodes.iter().filter(|t| t.start > n.start && t.end <= n.end && t.name.ends_with(":tbl")).map(|t| (t.start, t.end)).collect();
        self.paras
            .iter()
            .filter_map(|&p| self.node(p))
            .any(|p| p.start > n.start && p.end <= n.end && !tables.iter().any(|(s, e)| p.start > *s && p.end <= *e))
    }

    /// An hp:ctrl holding only [`PAGE_CONTROLS`].
    fn is_page_control(&self, i: usize) -> bool {
        let kids = self.kids(i, None);
        self.local(i) == "ctrl" && !kids.is_empty() && kids.iter().all(|&k| PAGE_CONTROLS.contains(&self.local(k)))
    }

    fn has_paragraph_inside(&self, i: usize) -> bool {
        let Some(n) = self.node(i) else { return false };
        self.paras.iter().filter_map(|&p| self.node(p)).any(|p| p.start > n.start && p.end <= n.end)
    }

    /// The paragraph's text as editable characters, or why it cannot be edited.
    fn plain(&self, ordinal: usize) -> std::result::Result<Plain, String> {
        let Some(&p) = self.paras.get(ordinal) else { return Err("paragraph not found in the original".into()) };
        let runs = self.kids(p, Some("run"));
        let mut text_runs: Vec<usize> = Vec::new();
        let mut chars: Vec<Ch> = Vec::new();
        let mut tab_raw: Option<String> = None;
        let mut open_run: Option<usize> = None; // a first text run whose settings come before its text
        let mut starts: Vec<usize> = Vec::new(); // each text run's first character
        let mut tail = String::new(); // page controls among text, see PAGE_CONTROLS
        let mut lead = String::new(); // before the runs, see `Plain::lead`
        for &r in &runs {
            let mut kids = self.kids(r, None);
            if kids.iter().any(|&k| self.local(k) == "t") && kids.iter().any(|&k| self.is_page_control(k)) {
                let (page, rest): (Vec<usize>, Vec<usize>) = kids.iter().partition(|&&k| self.is_page_control(k));
                let raw: String = page.iter().filter_map(|&k| self.node(k)).map(|n| self.slice(n.start, n.end)).collect();
                tail.push_str(&format!("<{0}:run charPrIDRef=\"{1}\">{raw}</{0}:run>", self.pfx, self.attr(r, "charPrIDRef", "0")));
                kids = rest;
            }
            let ts: Vec<usize> = kids.iter().copied().filter(|&k| self.local(k) == "t").collect();
            if !ts.is_empty() && ts.len() != kids.len() {
                let lead = kids.get(..kids.len() - ts.len()).unwrap_or(&[]);
                let others: Vec<usize> = kids.iter().copied().filter(|&k| self.local(k) != "t").collect();
                if !text_runs.is_empty() || lead != others.as_slice() {
                    return Err("text and objects in one run".into());
                }
                open_run = Some(r);
            }
            if !kids.is_empty() && ts.is_empty() {
                continue; // a run of objects or controls
            }
            text_runs.push(r);
            starts.push(chars.len());
            let cpr = self.attr(r, "charPrIDRef", "0");
            for &t in &ts {
                let Some(tn) = self.node(t) else { continue };
                let close = if tn.end == tn.tag_end { 2 } else { 1 };
                let tattrs = self.slice(tn.start + tn.name.len() + 1, tn.tag_end.saturating_sub(close)).trim().to_string();
                let key: Key = (cpr.clone(), tattrs);
                let mut pos = tn.tag_end;
                for &c in &tn.children {
                    let Some(cn) = self.node(c) else { continue };
                    chars.extend(unescape(self.slice(pos, cn.start)).chars().map(|ch| (ch, key.clone(), None)));
                    let sp = special(self.local(c)).filter(|_| cn.children.is_empty());
                    let Some(sp) = sp else { return Err(format!("<{}> inside text", cn.name)) };
                    let raw = self.slice(cn.start, cn.end).to_string();
                    if sp == '\t' && tab_raw.is_none() {
                        tab_raw = Some(raw.clone());
                    }
                    chars.push((sp, key.clone(), Some(raw)));
                    pos = cn.end;
                }
                if tn.inner_end > pos {
                    chars.extend(unescape(self.slice(pos, tn.inner_end)).chars().map(|ch| (ch, key.clone(), None)));
                }
            }
        }
        let idx: Vec<usize> = text_runs.iter().filter_map(|r| runs.iter().position(|x| x == r)).collect();
        // groups of adjacent text runs: (first run, last run, first character, end character)
        let mut groups: Vec<(usize, usize, usize, usize)> = Vec::new();
        for (k, (&r, &i)) in text_runs.iter().zip(&idx).enumerate() {
            let from = starts.get(k).copied().unwrap_or(0);
            let to = starts.get(k + 1).copied().unwrap_or(chars.len());
            match groups.last_mut() {
                Some(g) if k > 0 && idx.get(k - 1).is_some_and(|&j| j + 1 == i) => {
                    g.1 = r;
                    g.3 = to;
                }
                _ => groups.push((r, r, from, to)),
            }
        }
        if groups.len() > 1 && !tail.is_empty() {
            return Err("text and objects in one run".into());
        }
        let mut segments = Vec::new();
        if groups.len() > 1 {
            for (k, &(first, last, from, to)) in groups.iter().enumerate() {
                let mut start = self.node(first).map(|n| n.start).unwrap_or(0);
                if k == 0
                    && let Some(t) = open_run.and_then(|o| self.kids(o, Some("t")).first().copied()).and_then(|t| self.node(t))
                {
                    start = t.start;
                }
                segments.push(((start, self.node(last).map(|n| n.end).unwrap_or(start)), (from, to)));
            }
        }
        let (region, key) = match (text_runs.first(), text_runs.last()) {
            (Some(&first), Some(&last)) => {
                let first_cpr = self.attr(first, "charPrIDRef", "0");
                let mut start = self.node(first).map(|n| n.start).unwrap_or(0);
                if let Some(t) = open_run.and_then(|o| self.kids(o, Some("t")).first().copied()).and_then(|t| self.node(t)) {
                    start = t.start; // rewrite from its first hp:t on; its settings stay
                }
                let end = self.node(last).map(|n| n.end).unwrap_or(start);
                let key = chars.first().map(|c| c.1.clone()).unwrap_or((first_cpr, String::new()));
                ((start, end), key)
            }
            _ => match runs.last() {
                Some(&r) => {
                    let e = self.node(r).map(|n| n.end).unwrap_or(0);
                    ((e, e), (self.attr(r, "charPrIDRef", "0"), String::new()))
                }
                None => {
                    let seg = self.kids(p, Some("linesegarray"));
                    let at = match seg.first().and_then(|&s| self.node(s)) {
                        Some(s) => s.start,
                        None => self.node(p).map(|n| n.inner_end).unwrap_or(0),
                    };
                    match open_empty(self, p) {
                        // an empty paragraph tag: its "/>" becomes ">" + the runs + its end tag
                        Some((s, close)) => {
                            lead = ">".to_string();
                            tail.push_str(&close);
                            ((s, s + 2), ("0".to_string(), String::new()))
                        }
                        None => ((at, at), ("0".to_string(), String::new())),
                    }
                }
            },
        };
        let lineseg = self.kids(p, Some("linesegarray")).first().and_then(|&s| self.node(s)).map(|s| (s.start, s.end));
        // with the run's own name, to close it as it was opened (a run of the default namespace
        // has no prefix)
        let open_key = open_run.map(|o| {
            ((self.attr(o, "charPrIDRef", "0"), String::new()), self.node(o).map_or_else(|| format!("{}:run", self.pfx), |n| n.name.clone()))
        });
        Ok(Plain { chars, region, key, lineseg, tab_raw, open_key, segments, lead, tail })
    }

    /// Text runs for the characters; consecutive ones with one key share a run. With `open_key`
    /// the output continues a run already opened (its settings kept): the leading text of that
    /// run's shape goes in plain hp:t elements, then the run closes.
    fn runs_xml(&self, chars: &[Ch], key: &Key, tab_raw: Option<&str>, open_key: Option<&(Key, String)>) -> String {
        let pfx = &self.pfx;
        if let Some((ok, open_name)) = open_key {
            let n = chars.iter().take_while(|c| c.1.0 == ok.0).count();
            let mut lead = String::new();
            if n > 0 {
                lead = strip_run_tags(&self.runs_xml(chars.get(..n).unwrap_or(&[]), key, tab_raw, None));
            }
            let rest = if n < chars.len() { self.runs_xml(chars.get(n..).unwrap_or(&[]), key, tab_raw, None) } else { String::new() };
            return format!("{lead}</{open_name}>{rest}");
        }
        if chars.is_empty() {
            return format!("<{pfx}:run charPrIDRef=\"{}\"/>", key.0);
        }
        let mut out = String::new();
        let mut i = 0;
        while let Some(first) = chars.get(i) {
            let k = &first.1;
            let mut body = String::new();
            let mut j = i;
            while let Some(c) = chars.get(j).filter(|c| &c.1 == k) {
                match (&c.2, c.0) {
                    (Some(raw), _) => body.push_str(raw),
                    (None, '\t') => body.push_str(tab_raw.map(str::to_string).unwrap_or_else(|| format!("<{pfx}:tab/>")).as_str()),
                    (None, '\n') => body.push_str(&format!("<{pfx}:lineBreak/>")),
                    (None, ch) => body.push_str(&esc(&ch.to_string())),
                }
                j += 1;
            }
            let attrs = if k.1.is_empty() { String::new() } else { format!(" {}", k.1) };
            out.push_str(&format!("<{pfx}:run charPrIDRef=\"{}\"><{pfx}:t{attrs}>{body}</{pfx}:t></{pfx}:run>", k.0));
            i = j;
        }
        out
    }
}

/// A run's XML without its leading start tag and trailing end tag (`<x:run ...>` ... `</x:run>`).
fn strip_run_tags(s: &str) -> String {
    let mut s = s.to_string();
    if s.starts_with('<')
        && let Some(gt) = s.find('>')
        && s.get(1..gt).is_some_and(|t| t.contains(":run "))
    {
        s = s.get(gt + 1..).unwrap_or("").to_string();
    }
    if let Some(body) = s.strip_suffix(":run>") {
        let after_gt = body.rfind('>').map(|g| g + 1).unwrap_or(0);
        if let Some(lt) = body.get(after_gt..).and_then(|r| r.find("</")) {
            s = body.get(..after_gt + lt).unwrap_or("").to_string();
        }
    }
    s
}

// ==== difference of two texts ==================================================================
//
// The shortest edit script (fewest characters deleted and inserted) by the greedy search of
// E. W. Myers, "An O(ND) Difference Algorithm and Its Variations", Algorithmica 1 (1986).

/// (tag, a1, a2, b1, b2) with tag "equal", "replace", "delete" or "insert": `a[a1..a2]` became
/// `b[b1..b2]`. The list covers both texts in order.
type Opcode = (&'static str, usize, usize, usize, usize);

/// A run of characters both texts share: (start in a, start in b, length).
type Block = (usize, usize, usize);

/// How `a` became `b`.
fn opcodes(a: &[char], b: &[char]) -> Vec<Opcode> {
    let (la, lb) = (a.len(), b.len());
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (a_rest, b_rest) = (a.get(pre..).unwrap_or(&[]), b.get(pre..).unwrap_or(&[]));
    let suf = a_rest.iter().rev().zip(b_rest.iter().rev()).take_while(|(x, y)| x == y).count();
    let mut blocks: Vec<Block> = Vec::new();
    if pre > 0 {
        blocks.push((0, 0, pre));
    }
    let (a_mid, b_mid) = (a.get(pre..la - suf).unwrap_or(&[]), b.get(pre..lb - suf).unwrap_or(&[]));
    if !a_mid.is_empty() && !b_mid.is_empty() {
        // too many edits: the middle is one replaced block
        for (i, j, k) in shared_runs(a_mid, b_mid).unwrap_or_default() {
            blocks.push((pre + i, pre + j, k));
        }
    }
    if suf > 0 {
        blocks.push((la - suf, lb - suf, suf));
    }
    let mut out: Vec<Opcode> = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (bi, bj, size) in blocks.into_iter().chain([(la, lb, 0)]) {
        match (i < bi, j < bj) {
            (true, true) => out.push(("replace", i, bi, j, bj)),
            (true, false) => out.push(("delete", i, bi, j, j)),
            (false, true) => out.push(("insert", i, i, j, bj)),
            (false, false) => {}
        }
        if size > 0 {
            match out.last_mut() {
                Some(last) if last.0 == "equal" && last.2 == bi && last.4 == bj => (last.2, last.4) = (bi + size, bj + size),
                _ => out.push(("equal", bi, bi + size, bj, bj + size)),
            }
        }
        (i, j) = (bi + size, bj + size);
    }
    out
}

/// The runs `a` and `b` share along a shortest edit script, in order; `None` past the edit or
/// size budget. Positions are on diagonals k = x − y (x in `a`, y in `b`); `ends[d]` holds, for
/// each diagonal −d, −d+2, …, d, how far x gets with d edits (−1: not reached).
fn shared_runs(a: &[char], b: &[char]) -> Option<Vec<Block>> {
    let (n, m) = (i64::try_from(a.len()).ok()?, i64::try_from(b.len()).ok()?);
    let max_d = MAX_DIFF_EDITS.min(MAX_DIFF_CELLS / a.len().saturating_add(b.len()).max(1)).min(a.len() + b.len());
    let same = |x: i64, y: i64| matches!((usize::try_from(x).ok().and_then(|x| a.get(x)), usize::try_from(y).ok().and_then(|y| b.get(y))), (Some(p), Some(q)) if p == q);
    let mut ends: Vec<Vec<i32>> = Vec::new();
    for d in 0..=max_d {
        let di = d as i64;
        let mut row = vec![-1i32; 2 * d + 1];
        for k in (-di..=di).step_by(2) {
            let start = if d == 0 { Some((0, k)) } else { ends.get(d - 1).and_then(|prev| step(prev, di - 1, k, n, m)) };
            let Some((mut x, _)) = start else { continue };
            while same(x, x - k) {
                x += 1;
            }
            if let Some(slot) = usize::try_from(k + di).ok().and_then(|at| row.get_mut(at)) {
                *slot = i32::try_from(x).ok()?;
            }
            if x == n && x - k == m {
                ends.push(row);
                return trace_back(&ends, n, m);
            }
        }
        ends.push(row);
    }
    None
}

/// Where diagonal `k` starts with one more edit after `prev` (the ends of edit count `dp`): one
/// step down (a character inserted, from diagonal k+1) or right (a character deleted, from k−1),
/// whichever gets further, down on a tie. Returns (x, the diagonal it came from).
fn step(prev: &[i32], dp: i64, k: i64, n: i64, m: i64) -> Option<(i64, i64)> {
    let end = |kk: i64| -> Option<i64> {
        if kk.abs() > dp {
            return None;
        }
        let x = i64::from(*prev.get(usize::try_from(kk + dp).ok()?)?);
        (x >= 0).then_some(x)
    };
    let fits = |x: i64| x <= n && (0..=m).contains(&(x - k));
    let down = end(k + 1).filter(|&x| fits(x)).map(|x| (x, k + 1));
    let right = end(k - 1).map(|x| x + 1).filter(|&x| fits(x)).map(|x| (x, k - 1));
    match (down, right) {
        (Some(d), Some(r)) => Some(if r.0 > d.0 { r } else { d }),
        (d, r) => d.or(r),
    }
}

/// The shared runs of the path that reached (n, m) with `ends.len() - 1` edits, walked back.
fn trace_back(ends: &[Vec<i32>], n: i64, m: i64) -> Option<Vec<Block>> {
    let mut runs: Vec<Block> = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..ends.len()).rev() {
        let k = x - y;
        let (dp, prev) = (d as i64 - 1, ends.get(d - 1)?);
        let (sx, from) = step(prev, dp, k, n, m)?;
        if x > sx {
            runs.push((usize::try_from(sx).ok()?, usize::try_from(sx - k).ok()?, usize::try_from(x - sx).ok()?));
        }
        x = i64::from(*prev.get(usize::try_from(from + dp).ok()?)?);
        y = x - from;
    }
    if x > 0 && x == y {
        runs.push((0, 0, usize::try_from(x).ok()?));
    }
    runs.reverse();
    Some(runs)
}

/// Apply the edit before -> after (DOCX texts) to the original characters.
fn rewrite(before: &str, after: &str, chars: &[Ch], key: &Key) -> Vec<Ch> {
    let a: Vec<char> = before.chars().collect();
    let b: Vec<char> = after.chars().collect();
    let mut out = Vec::new();
    for (op, a1, a2, b1, b2) in opcodes(&a, &b) {
        if op == "equal" {
            out.extend(chars.get(a1..a2).unwrap_or(&[]).iter().cloned());
            continue;
        }
        // typed text takes the shape of the character before it (or after, at the start)
        let near = if a1 > 0 { chars.get(a1 - 1).map(|c| c.1.clone()) } else { chars.get(a2).map(|c| c.1.clone()) }.unwrap_or_else(|| key.clone());
        if op == "replace" || op == "insert" {
            out.extend(b.get(b1..b2).unwrap_or(&[]).iter().map(|&ch| (ch, near.clone(), None)));
        }
    }
    out
}

/// Where each group of a paragraph's text (`segments`: their character ranges in `before`) ends up
/// in `after`, or `None` when an edit runs across the place between two groups (an object). Text
/// typed at such a place goes with the group before it.
fn segment_spans(before: &str, after: &str, segments: &[((usize, usize), (usize, usize))]) -> Option<Vec<(usize, usize)>> {
    let a: Vec<char> = before.chars().collect();
    let b: Vec<char> = after.chars().collect();
    let ops = opcodes(&a, &b);
    let map = |at: usize| -> Option<usize> {
        if at == 0 {
            return Some(0);
        }
        if at >= a.len() {
            return Some(b.len());
        }
        if ops.iter().any(|&(op, a1, a2, _, _)| op != "equal" && a1 < at && at < a2) {
            return None;
        }
        // text typed at the place goes with the group before it
        if let Some(&(_, _, _, _, b2)) = ops.iter().find(|o| o.0 == "insert" && o.1 == at) {
            return Some(b2);
        }
        if let Some(&(_, _, _, _, b2)) = ops.iter().find(|o| o.2 == at) {
            return Some(b2);
        }
        ops.iter().find(|o| o.0 == "equal" && o.1 <= at && at < o.2).map(|&(_, a1, _, b1, _)| b1 + (at - a1))
    };
    let mut out = Vec::with_capacity(segments.len());
    for (k, &(_, (from, to))) in segments.iter().enumerate() {
        let start = if k == 0 { 0 } else { map(from)? };
        let end = if k + 1 == segments.len() { b.len() } else { map(to)? };
        out.push((start, end.max(start)));
    }
    Some(out)
}

// ==== the whole save ==========================================================================

/// (start, end, new text) in a section's raw XML.
pub(crate) type Edit = (usize, usize, String);

struct Job<'d> {
    sections: Vec<Section>,
    edits: Vec<Vec<Edit>>,
    report: SaveReport,
    /// The documents before and after editing, and the header new shapes go to (formatting and
    /// table edits; without them only text is written).
    style: Option<Restyle<'d>>,
    /// Picture edits are written (HWPX); otherwise they are reported.
    pictures: bool,
    /// An HWP 5.0 file: a table written again keeps its unchanged cells' records (numbering
    /// included), so numbers left out of the table's XML aren't lost there.
    hwp: bool,
    /// Image files added to the package.
    media: Vec<NewMedia>,
    /// The number of the next new image item ("image<n>").
    next_image: usize,
    /// The pictures of new body paragraphs, in order (see `restyle::DocMarks::new_pics`).
    new_pics: std::collections::VecDeque<Vec<crate::restyle::Pic>>,
}

/// An image file added to the package: (part name, item id, media type, bytes).
pub(crate) type NewMedia = (String, String, &'static str, Vec<u8>);

/// `tag` with attribute `name` set to `value` (left as it is without the attribute).
fn set_attr(tag: &str, name: &str, value: &str) -> String {
    let Some((_, from, to)) = attr_spans(tag).into_iter().find(|a| a.0 == name) else { return tag.to_string() };
    // a value in single quotes can't hold one: written as a character reference
    let value = esc(value).replace('\'', "&apos;");
    format!("{}{value}{}", tag.get(..from).unwrap_or(""), tag.get(to..).unwrap_or(""))
}

struct Restyle<'d> {
    before: crate::restyle::DocMarks<'d>,
    after: crate::restyle::DocMarks<'d>,
    after_doc: &'d wordcraft_doc::Document,
    header: crate::restyle::HeaderEdit,
}

impl Restyle<'_> {
    /// Did the formatting of paragraph `m`'s characters change (its text being the same)?
    fn chars_changed(&self, m: Mark) -> bool {
        let (Some(b), Some(a)) = (self.before.paras.get(&m), self.after.paras.get(&m)) else { return false };
        b.chars.len() == a.chars.len() && b.chars.iter().zip(&a.chars).any(|(x, y)| !crate::restyle::same_format(&x.1, &y.1))
    }

    /// New character shapes for `chars` (paragraph `m` after the edit; `orig` its characters as
    /// written in the original) where WordCraft shows other formatting than the original had.
    fn restyle_chars(&mut self, m: Mark, orig: &[Ch], chars: &mut [Ch]) {
        let (Some(b), Some(a)) = (self.before.paras.get(&m), self.after.paras.get(&m)) else { return };
        if b.chars.len() != orig.len() || b.chars.iter().zip(orig).any(|(x, o)| x.0 != o.0) {
            return;
        }
        if a.chars.len() != chars.len() || a.chars.iter().zip(chars.iter()).any(|(x, c)| x.0 != c.0) {
            return;
        }
        // each original shape's formatting as WordCraft showed it before
        let mut was: HashMap<String, &wordcraft_doc::resolve::ResolvedChar> = HashMap::new();
        for (o, (_, rc)) in orig.iter().zip(&b.chars) {
            was.entry(o.1.0.clone()).or_insert(rc);
        }
        for (c, (_, now)) in chars.iter_mut().zip(&a.chars) {
            let Some(old) = was.get(&c.1.0) else { continue };
            if crate::restyle::same_format(old, now) {
                continue;
            }
            // A copy of its own shape with the change, not another run's shape that looks the
            // same: write-back compares only what it writes, so that shape may differ in what it
            // doesn't (character width, spacing, outline ...). The same change makes one variant.
            if let Some(id) = self.header.char_variant(&c.1.0, old, now) {
                c.1.0 = id;
            }
        }
    }
}

impl Job<'_> {
    fn skip(&mut self, m: Mark, why: &str) {
        self.report.skipped.push(format!("section {}, paragraph {}: {why}", m.1 + 1, m.2));
    }

    /// Can paragraph `m` be deleted (removed, or its text cleared)? Not when its text has
    /// something the writer can't rewrite.
    fn deletable(&self, m: Mark) -> bool {
        self.sections.get(m.1).is_some_and(|sec| sec.paras.get(m.2).is_some() && sec.plain(m.2).is_ok())
    }

    /// Paragraphs `extra` merged into paragraph `m`: deleted when its merged text was written,
    /// kept (and reported) when it was `refused`, as deleting them would lose their text.
    fn merged(&mut self, m: Mark, extra: &[Mark], refused: bool) {
        if !refused {
            for &x in extra {
                self.delete(x);
            }
        } else if !extra.is_empty() {
            self.skip(m, "paragraphs merged into it are kept as they were (the merged text could not be written)");
        }
    }

    fn modify(&mut self, m: Mark, before: &str, after: &str) {
        let Some(sec) = self.sections.get(m.1) else { return self.skip(m, "paragraph not found in the original") };
        let plain = match sec.plain(m.2) {
            Ok(p) => p,
            Err(why) => return self.skip(m, &why),
        };
        if plain.chars.iter().map(|c| c.0).collect::<String>() != before {
            return self.skip(m, "the paragraph holds content shown differently in WordCraft");
        }
        let mut chars = rewrite(before, after, &plain.chars, &plain.key);
        if let Some(st) = self.style.as_mut() {
            st.restyle_chars(m, &plain.chars, &mut chars);
        }
        let same = before == after && chars.iter().zip(&plain.chars).all(|(a, b)| a.1 == b.1);
        if same {
            self.report.unchanged += 1;
            return;
        }
        let Some(sec) = self.sections.get(m.1) else { return };
        let mut edits: Vec<Edit> = Vec::new();
        if plain.segments.is_empty() {
            let runs = sec.runs_xml(&chars, &plain.key, plain.tab_raw.as_deref(), plain.open_key.as_ref());
            edits.push((plain.region.0, plain.region.1, plain.lead.clone() + &runs + &plain.tail));
        } else {
            // objects or fields between text runs: each group rewritten on its own, the edit
            // inside groups (one that runs across an object is not written)
            let Some(spans) = segment_spans(before, after, &plain.segments) else {
                return self.skip(m, "an edit across an object or field");
            };
            for (k, (&(region, (from, to)), (nf, nt))) in plain.segments.iter().zip(spans).enumerate() {
                let old = plain.chars.get(from..to).unwrap_or(&[]);
                let new = chars.get(nf..nt).unwrap_or(&[]);
                if old.len() == new.len() && old.iter().zip(new).all(|(a, b)| a.0 == b.0 && a.1 == b.1) {
                    continue;
                }
                let key = old.first().map(|c| c.1.clone()).unwrap_or_else(|| plain.key.clone());
                let open = if k == 0 { plain.open_key.as_ref() } else { None };
                edits.push((region.0, region.1, sec.runs_xml(new, &key, plain.tab_raw.as_deref(), open)));
            }
        }
        if let Some(e) = self.edits.get_mut(m.1) {
            e.extend(edits);
            if let Some((a, b)) = plain.lineseg {
                e.push((a, b, String::new()));
            }
        }
        self.report.modified += 1;
    }

    /// A new paragraph shape for paragraph `m` when its alignment, indents or spacing changed.
    fn restyle_para(&mut self, m: Mark) {
        let Some(st) = self.style.as_mut() else { return };
        let (Some(b), Some(a)) = (st.before.paras.get(&m), st.after.paras.get(&m)) else { return };
        if b.para == a.para {
            return;
        }
        let Some(sec) = self.sections.get(m.1) else { return };
        let Some(node) = sec.paras.get(m.2).and_then(|&p| sec.node(p)) else { return };
        let tag = sec.slice(node.start, node.tag_end).to_string();
        let Some(id) = tag_attrs(&tag).get("paraPrIDRef").cloned() else { return };
        let (bp, ap) = (b.para.clone(), a.para.clone());
        let Some((new, unwritten)) = st.header.para_variant(&id, &bp, &ap) else { return };
        // whichever quote the attribute stands in
        let new_tag = set_attr(&tag, "paraPrIDRef", &new);
        if let Some(e) = self.edits.get_mut(m.1) {
            e.push((node.start, node.tag_end, new_tag));
        }
        if !unwritten.is_empty() {
            self.report.skipped.push(format!("a paragraph's {} (its shape in the file has no element for it)", unwritten.join(", ")));
        }
        // counted only when a change other than those is written
        if unwritten.len() < para_changes(&bp, &ap) {
            self.report.restyled += 1;
        }
    }

    /// The nearest hp:tbl around paragraph `m`.
    fn table_of(&self, m: Mark, inside: usize) -> Option<(usize, usize)> {
        let sec = self.sections.get(m.1)?;
        let mut i = *sec.paras.get(m.2)?;
        let mut guard = 0;
        let mut skip = inside; // tables around the paragraph that are inside the one wanted
        while let Some(p) = sec.node(i).and_then(|n| n.parent) {
            if sec.local(p) == "tbl" {
                if skip == 0 {
                    return sec.node(p).map(|n| (n.start, n.end));
                }
                skip -= 1;
            }
            i = p;
            guard += 1;
            if guard > 256 {
                break;
            }
        }
        None
    }

    /// The "ht" marks of original tables that edited tables match only by a cell other than
    /// their first (see `Job::tables`).
    fn tables_found_by_cells(&self) -> Vec<Mark> {
        let Some(st) = self.style.as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for t in &st.after.tables {
            let Some(m) = t.mark else { continue };
            if st.before.tables.iter().any(|b| b.mark == Some(m) && b.depth == t.depth) {
                continue;
            }
            let Some(b) = st.before.tables.iter().find(|b| b.depth == t.depth && b.marks.contains(&m)) else { continue };
            let Some(bm) = b.mark else { continue };
            if let Some(h) = self.holder_of(bm, b.inside) {
                out.push(("ht", bm.1, h));
            }
        }
        out
    }

    /// The paragraph number (as marks count them) of the paragraph holding the table around
    /// paragraph `m`, `inside` tables up as in [`Job::table_of`].
    fn holder_of(&self, m: Mark, inside: usize) -> Option<usize> {
        let sec = self.sections.get(m.1)?;
        let (start, _) = self.table_of(m, inside)?;
        let tbl = sec.nodes.iter().position(|n| n.start == start && sec.local_of(n) == "tbl")?;
        let mut i = tbl;
        for _ in 0..256 {
            let p = sec.node(i)?.parent?;
            if sec.local(p) == "p" {
                return sec.paras.iter().position(|&q| q == p);
            }
            i = p;
        }
        None
    }

    /// The paragraph holding the outermost table around paragraph `m` (node index).
    fn table_holder(&self, m: Mark) -> Option<usize> {
        let sec = self.sections.get(m.1)?;
        let mut i = *sec.paras.get(m.2)?;
        let mut holder = None;
        let mut seen_table = false;
        let mut guard = 0;
        while let Some(p) = sec.node(i).and_then(|n| n.parent) {
            match sec.local(p) {
                "tbl" => seen_table = true,
                "p" if seen_table => {
                    holder = Some(p);
                    seen_table = false;
                }
                _ => {}
            }
            i = p;
            guard += 1;
            if guard > 256 {
                break;
            }
        }
        holder
    }

    /// Tables whose structure or look changed are written again; new ones are added.
    fn tables(&mut self) {
        let Some(st) = self.style.as_mut() else { return };
        let doc = st.after_doc;
        let mut work: Vec<(Option<(Mark, usize)>, Option<(Mark, bool)>, usize, &wordcraft_doc::table::Table)> = Vec::new();
        for t in &st.after.tables {
            match t.mark {
                Some(m) => {
                    // by its first mark, else (its first row deleted) by any of its cells' marks
                    let before = st
                        .before
                        .tables
                        .iter()
                        .find(|b| b.mark == Some(m) && b.depth == t.depth)
                        .or_else(|| st.before.tables.iter().find(|b| b.depth == t.depth && b.marks.contains(&m)));
                    if let Some(b) = before.filter(|b| crate::restyle::table_signature(b.table) != crate::restyle::table_signature(t.table))
                        && let Some(bm) = b.mark
                    {
                        // found from the original's own first mark, as the original has it
                        work.push((Some((bm, b.inside)), None, 0, t.table));
                    }
                }
                None if t.top => work.push((None, t.prev, t.new_before, t.table)),
                None => {}
            }
        }
        for (mark, prev, new_before, table) in work {
            let Some(st) = self.style.as_mut() else { return };
            let (xml, shapes, rep, media) = crate::hwpx_out::table_alone(doc, table);
            let (chars, paras, fills) = st.header.add_generated(&shapes);
            let xml = crate::restyle::remap_ids(&xml, &chars, &paras, &fills);
            let hwp = self.hwp;
            // a .hwp keeps the records of cells that read as they did, numbering included: it
            // reports the numbered cells it writes again (marked), and the table's count here
            // would also name the kept ones; a .hwpx writes every cell again, unmarked
            self.report.skipped.extend(rep.skipped.into_iter().filter(|s| !(hwp && s.contains(crate::hwpx_out::NUMBERS_LEFT_OUT))));
            let xml = if hwp { xml } else { xml.replace(&format!(" {}=\"1\"", crate::hwpx_out::NUMBERED_CELL), "") };
            let xml = if self.pictures { self.table_pictures(xml, media) } else { xml };
            match (mark, prev) {
                (Some((m, inside)), _) => {
                    let Some((s, e)) = self.table_of(m, inside) else { continue };
                    if let Some(ed) = self.edits.get_mut(m.1) {
                        ed.push((s, e, xml));
                    }
                    self.report.tables += 1;
                }
                (None, Some((p, after_table))) => {
                    let Some(sec) = self.sections.get(p.1) else { continue };
                    let holder = if after_table { self.table_holder(p) } else { sec.paras.get(p.2).copied() };
                    let Some(node) = holder.and_then(|q| sec.node(q)) else { continue };
                    let mut start_tag = new_start_tag(sec.slice(node.start, node.tag_end));
                    if let Some(s) = start_tag.strip_suffix("/>") {
                        start_tag = format!("{s}>");
                    }
                    let cpr = if after_table {
                        holder
                            .map(|q| sec.kids(q, Some("run")))
                            .and_then(|r| r.first().map(|&r| sec.attr(r, "charPrIDRef", "0")))
                            .unwrap_or_else(|| "0".into())
                    } else {
                        sec.plain(p.2).map(|pl| pl.key.0).unwrap_or_else(|_| "0".into())
                    };
                    // closed by the name its copied start tag opens (`p:p` where the part says so)
                    let holder =
                        format!("{start_tag}<{pfx}:run charPrIDRef=\"{cpr}\">{}</{pfx}:run></{}>", sec.fragment(&xml), node.name, pfx = sec.pfx);
                    let table_tag = format!("<{}:tbl", sec.pfx);
                    if let Some(ed) = self.edits.get_mut(p.1) {
                        // after the new paragraphs that come before it there (insertions at one
                        // place are written in the order they are listed)
                        let at = node.end;
                        let slot = ed
                            .iter()
                            .enumerate()
                            .filter(|(_, e)| e.0 == at && e.1 == at && !e.2.contains(&table_tag))
                            .nth(new_before)
                            .map(|(i, _)| i);
                        match slot {
                            Some(i) => ed.insert(i, (at, at, holder)),
                            None => ed.push((at, at, holder)),
                        }
                    }
                    self.report.tables += 1;
                }
                (None, None) => self.report.skipped.push("a new table before the first paragraph".into()),
            }
        }
    }

    /// Remove the table paragraph `m` holds (the whole paragraph when it holds nothing else).
    fn delete_table(&mut self, m: Mark) {
        let Some(sec) = self.sections.get(m.1) else { return };
        let Some(&p) = sec.paras.get(m.2) else { return };
        let Some(node) = sec.node(p) else { return };
        let runs = sec.kids(p, Some("run"));
        let tables: Vec<usize> = runs.iter().flat_map(|&r| sec.kids(r, Some("tbl"))).collect();
        if tables.is_empty() {
            return self.skip(m, "deleted object not found in the original");
        }
        let has_text = runs.iter().flat_map(|&r| sec.kids(r, Some("t"))).filter_map(|t| sec.node(t)).any(|t| t.inner_end > t.tag_end);
        // the paragraph goes whole only when its runs hold nothing but tables (and empty text):
        // section settings, controls or objects beside the table must survive it
        let only_tables = runs.iter().flat_map(|&r| sec.kids(r, None)).all(|k| matches!(sec.local(k), "tbl" | "t" | "linesegarray"));
        let siblings = node.parent.map(|q| sec.kids(q, Some("p"))).unwrap_or_default();
        let edits: Vec<Edit> = if !has_text && only_tables && siblings.len() > 1 && !sec.has_paragraph_inside_outside_tables(p) {
            vec![(node.start, node.end, String::new())]
        } else {
            tables.iter().filter_map(|&t| sec.node(t)).map(|t| (t.start, t.end, String::new())).collect()
        };
        if let Some(e) = self.edits.get_mut(m.1) {
            e.extend(edits);
        }
        self.report.deleted += 1;
    }

    fn delete(&mut self, m: Mark) {
        let Some(sec) = self.sections.get(m.1) else { return };
        let Some(&p) = sec.paras.get(m.2) else { return };
        let Some(node) = sec.node(p) else { return };
        let plain = sec.plain(m.2);
        let siblings = match node.parent {
            Some(parent) => sec.kids(parent, Some("p")),
            None => vec![p],
        };
        // a run holding settings before its text (`secPr`, then `t`: `open_key`) keeps them, so
        // the paragraph is cleared, not removed
        let removable = plain.as_ref().is_ok_and(|pl| pl.tail.is_empty() && pl.open_key.is_none())
            && !sec.has_paragraph_inside(p)
            && !sec.kids(p, Some("run")).iter().any(|&r| !sec.kids(r, None).is_empty() && sec.kids(r, Some("t")).is_empty());
        let plain = match plain {
            Ok(pl) => pl,
            Err(why) => return self.skip(m, &format!("deleted, but {why}")),
        };
        let cleared =
            (plain.region.0, plain.region.1, plain.lead.clone() + &sec.runs_xml(&[], &plain.key, None, plain.open_key.as_ref()) + &plain.tail);
        let edits = self.edits.get(m.1).map(Vec::as_slice).unwrap_or(&[]);
        let alive = siblings.iter().filter(|&&s| s != p).filter_map(|&s| sec.node(s)).any(|s| !edits.iter().any(|e| e.0 == s.start && e.1 == s.end));
        // a paragraph holding section settings or objects stays, its text cleared; so does the
        // last paragraph of a cell or text box
        let edit = if removable && alive { (node.start, node.end, String::new()) } else { cleared };
        if let Some(e) = self.edits.get_mut(m.1) {
            e.push(edit);
        }
        self.report.deleted += 1;
    }

    /// A new image item for picture `o` of the edited document: its hp:pic XML, the image added
    /// to the package.
    fn new_picture(&mut self, o: &wordcraft_doc::InlineObject) -> Option<String> {
        let doc = self.style.as_ref()?.after_doc;
        let wordcraft_doc::InlineObject::Image { media, .. } = o else {
            // a drawing, text box or group: its text's shapes go into the header, its pictures
            // into the package, as for a written table
            let (xml, shapes, rep, media) = crate::hwpx_out::object_alone(doc, o, 1000 + self.media.len())?;
            let st = self.style.as_mut()?;
            let (chars, paras, fills) = st.header.add_generated(&shapes);
            let xml = crate::restyle::remap_ids(&xml, &chars, &paras, &fills);
            self.report.skipped.extend(rep.skipped);
            self.report.pictures += 1;
            return Some(if self.pictures { self.table_pictures(xml, media) } else { xml });
        };
        let data = doc.media.get(media)?;
        let item = format!("image{}", self.next_image);
        let (xml, ext, mime, left_out) = crate::hwpx_out::picture_alone(doc, o, &item, 1000 + self.media.len())?;
        self.report.skipped.extend(left_out);
        self.media.push((format!("BinData/{item}.{ext}"), item, mime, data.to_vec()));
        self.next_image += 1;
        self.report.pictures += 1;
        Some(xml)
    }

    /// Give the pictures of a written table (items from "image1") new items of the package.
    fn table_pictures(&mut self, xml: String, media: crate::hwpx_out::PieceMedia) -> String {
        let Some(doc) = self.style.as_ref().map(|st| st.after_doc) else { return xml };
        let mut xml = xml;
        // through placeholders first, so "image1" -> "image2" never meets "image2" -> "image3"
        for (_, item, _, _) in &media {
            xml = xml.replace(&format!("binaryItemIDRef=\"{item}\""), &format!("binaryItemIDRef=\"\u{1}{item}\""));
        }
        for (key, item, name, mime) in media {
            let Some(data) = doc.media.get(&key) else { continue };
            let new = format!("image{}", self.next_image);
            self.next_image += 1;
            let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("png");
            xml = xml.replace(&format!("binaryItemIDRef=\"\u{1}{item}\""), &format!("binaryItemIDRef=\"{new}\""));
            self.media.push((format!("BinData/{new}.{ext}"), new, mime, data.to_vec()));
        }
        xml
    }

    /// Write the picture edits of paragraph `m`: pictures resized, replaced, removed or added.
    fn pictures(&mut self, m: Mark) {
        let Some(st) = self.style.as_ref() else { return };
        let (Some(b), Some(a)) = (st.before.paras.get(&m), st.after.paras.get(&m)) else { return };
        if b.pics.len() == a.pics.len() && b.pics.iter().zip(&a.pics).all(|(x, y)| x.same(y)) {
            // the same pictures, but one moved within the text (the text itself unchanged, so an
            // offset shift isn't typing before it): moves aren't written, so it says so
            let text = |p: &crate::restyle::ParaInfo| p.chars.iter().map(|c| c.0).collect::<String>();
            if b.pics.iter().zip(&a.pics).any(|(x, y)| x.text_before != y.text_before) && text(b) == text(a) {
                self.skip(m, "a picture moved within its paragraph is not written (it keeps its place)");
            }
            return;
        }
        if !self.pictures {
            return self.skip(m, "a picture was added, changed or removed; pictures are not written to this format yet");
        }
        let (before, after) = (b.pics.clone(), a.pics.clone());
        let Some(sec) = self.sections.get(m.1) else { return };
        let Some(&p) = sec.paras.get(m.2) else { return };
        let runs = sec.kids(p, Some("run"));
        let nodes: Vec<usize> = runs.iter().flat_map(|&r| sec.kids(r, None)).filter(|&q| DRAWING_TAGS.contains(&sec.local(q))).collect();
        if nodes.len() != before.len() {
            return self.skip(m, "its pictures are not where WordCraft shows them; picture edits not written");
        }
        // which picture became which: in place when as many are left, else by image
        let pairs: Vec<(Option<usize>, Option<usize>)> =
            if before.len() == after.len() { (0..before.len()).map(|k| (Some(k), Some(k))).collect() } else { match_pictures(&before, &after) };
        let mut edits: Vec<Edit> = Vec::new();
        let mut moved = false;
        // the last picture kept: where it ends, and the text before it now
        let mut kept: Option<(usize, usize)> = None;
        for (bk, ak) in pairs {
            let node = bk.and_then(|k| nodes.get(k)).and_then(|&q| self.sections.get(m.1).and_then(|sec| sec.node(q)).map(|n| (q, n.start, n.end)));
            if let (Some((_, _, e)), Some(new)) = (node, ak.and_then(|k| after.get(k))) {
                kept = Some((e, new.text_before));
            }
            match (bk.and_then(|k| before.get(k)), ak.and_then(|k| after.get(k)), node) {
                (Some(old), Some(new), Some((q, s, e))) => {
                    // another image, or the same one cropped, turned or placed anew: written whole
                    if old.data != new.data || old.settings != new.settings {
                        if let Some(xml) = self.new_picture(&new.obj) {
                            edits.push((s, e, xml));
                        }
                    } else if !old.same(new) {
                        let Some(sec) = self.sections.get(m.1) else { return };
                        edits.extend(resize_edits(sec, q, new.w, new.h));
                        self.report.pictures += 1;
                    }
                }
                (Some(_), None, Some((_, s, e))) => {
                    edits.push((s, e, String::new()));
                    self.report.pictures += 1;
                }
                (None, Some(new), _) => {
                    let Some(xml) = self.new_picture(&new.obj) else { continue };
                    if let Some((end, _)) = kept.filter(|k| k.1 == new.text_before) {
                        edits.push((end, end, xml)); // right after the picture before it, in its run
                        continue;
                    }
                    let Some(sec) = self.sections.get(m.1) else { return };
                    let at_start = new.text_before == 0;
                    moved |= !at_start && new.text_after > 0;
                    let (at, cpr) = new_run_place(sec, p, at_start);
                    let run = format!("<{pfx}:run charPrIDRef=\"{cpr}\">{}</{pfx}:run>", sec.fragment(&xml), pfx = sec.pfx);
                    match open_empty(sec, p) {
                        Some((s, close)) => edits.push((s, s + 2, format!(">{run}{close}"))),
                        None => edits.push((at, at, run)),
                    }
                }
                _ => {}
            }
        }
        if moved {
            self.skip(m, "a picture added inside a line was put at the end of its paragraph");
        }
        let Some(sec) = self.sections.get(m.1) else { return };
        if !edits.is_empty()
            && let Some(seg) = sec.kids(p, Some("linesegarray")).first().and_then(|&s| sec.node(s))
        {
            edits.push((seg.start, seg.end, String::new())); // Hancom lays the lines out again
        }
        if let Some(e) = self.edits.get_mut(m.1) {
            e.extend(edits);
        }
    }

    fn insert(&mut self, anchor: Option<Mark>, next: Option<Mark>, text: &str, pics: &[crate::restyle::Pic]) {
        let Some(r) = anchor.or(next) else { return };
        let Some(sec) = self.sections.get(r.1) else { return self.skip(r, "new paragraph next to a paragraph not found in the original") };
        let Some(node) = sec.paras.get(r.2).and_then(|&p| sec.node(p)) else {
            return self.skip(r, "new paragraph next to a paragraph not found in the original");
        };
        let mut start_tag = new_start_tag(sec.slice(node.start, node.tag_end));
        if let Some(s) = start_tag.strip_suffix("/>") {
            start_tag = format!("{s}>");
        }
        let (key, tab_raw) = match sec.plain(r.2) {
            Ok(pl) => (pl.chars.last().map(|c| c.1.clone()).unwrap_or(pl.key), pl.tab_raw),
            Err(_) => {
                // e.g. a table's paragraph: take its last run's shape
                let runs = sec.paras.get(r.2).map(|&p| sec.kids(p, Some("run"))).unwrap_or_default();
                let cpr = runs.last().map(|&l| sec.attr(l, "charPrIDRef", "0")).unwrap_or_else(|| "0".into());
                ((cpr, String::new()), None)
            }
        };
        let chars: Vec<Ch> = text.chars().map(|ch| (ch, key.clone(), None)).collect();
        let pfx = sec.pfx.clone();
        let close = node.name.clone();
        let body = sec.runs_xml(&chars, &key, tab_raw.as_deref(), None);
        let at = if anchor.is_some() { node.end } else { node.start };
        let (mut lead, mut tail) = (String::new(), String::new());
        for pic in pics {
            let Some(xml) = self.new_picture(&pic.obj) else { continue };
            let xml = self.sections.get(r.1).map_or(xml.clone(), |sec| sec.fragment(&xml));
            let run = format!("<{pfx}:run charPrIDRef=\"{}\">{xml}</{pfx}:run>", key.0);
            if pic.text_before == 0 { lead.push_str(&run) } else { tail.push_str(&run) }
        }
        // closed by the name its copied start tag opens (`p:p` where the part says so)
        let xml = format!("{start_tag}{lead}{body}{tail}</{close}>");
        if let Some(e) = self.edits.get_mut(r.1) {
            e.push((at, at, xml));
        }
        self.report.inserted += 1;
    }
}

/// Which picture became which, by image (sizes aside): (before, after) pairs in order, a side
/// `None` for a picture removed or added.
/// The run children that are pictures, drawings or groups: what `restyle`'s pictures are.
const DRAWING_TAGS: [&str; 11] = ["pic", "rect", "ellipse", "line", "arc", "polygon", "curve", "container", "connectLine", "ole", "textart"];

fn match_pictures(before: &[crate::restyle::Pic], after: &[crate::restyle::Pic]) -> Vec<(Option<usize>, Option<usize>)> {
    let (n, m) = (before.len().min(256), after.len().min(256));
    let same = |i: usize, j: usize| before.get(i).zip(after.get(j)).is_some_and(|(x, y)| x.data == y.data);
    // longest common subsequence of the images
    let mut t = vec![vec![0usize; m + 1]; n + 1];
    let at = |t: &Vec<Vec<usize>>, i: usize, j: usize| t.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0);
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let v = if same(i, j) { at(&t, i + 1, j + 1) + 1 } else { at(&t, i + 1, j).max(at(&t, i, j + 1)) };
            if let Some(c) = t.get_mut(i).and_then(|r| r.get_mut(j)) {
                *c = v;
            }
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n || j < m {
        if i < n && j < m && same(i, j) {
            out.push((Some(i), Some(j)));
            (i, j) = (i + 1, j + 1);
        } else if j < m && (i >= n || at(&t, i, j + 1) >= at(&t, i + 1, j)) {
            out.push((None, Some(j)));
            j += 1;
        } else {
            out.push((Some(i), None));
            i += 1;
        }
    }
    out
}

/// Edits giving picture `q` the size `w` x `h` points: its size, current size and scale.
fn resize_edits(sec: &Section, q: usize, w: f32, h: f32) -> Vec<Edit> {
    // one side may be 0 (a level or upright line)
    let (wd, ht) = match (crate::hwpx_out::hu(w).max(0), crate::hwpx_out::hu(h).max(0)) {
        (0, 0) => (1, 1),
        wh => wh,
    };
    let org = sec.kids(q, Some("orgSz")).first().map(|&o| (sec.attr(o, "width", "0"), sec.attr(o, "height", "0")));
    let org = org.map(|(a, b)| (a.parse::<f64>().unwrap_or(0.0), b.parse::<f64>().unwrap_or(0.0)));
    let mut out = Vec::new();
    let mut set = |k: usize, values: &[(&str, String)]| {
        if let Some(n) = sec.node(k) {
            let mut tag = sec.slice(n.start, n.tag_end).to_string();
            for (name, v) in values {
                tag = set_attr(&tag, name, v);
            }
            out.push((n.start, n.tag_end, tag));
        }
    };
    for k in sec.kids(q, None) {
        match sec.local(k) {
            "sz" | "curSz" => set(k, &[("width", wd.to_string()), ("height", ht.to_string())]),
            "renderingInfo" => {
                if let Some((ow, oh)) = org.filter(|(a, b)| *a > 0.0 && *b > 0.0) {
                    for sca in sec.kids(k, Some("scaMatrix")) {
                        set(sca, &[("e1", format!("{:.6}", wd as f64 / ow)), ("e5", format!("{:.6}", ht as f64 / oh))]);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// For paragraph `p` written as an empty tag (`<hp:p .../>`): where its "/>" starts, and the end
/// tag it takes when content goes in. Content put at its end would land after it, outside.
fn open_empty(sec: &Section, p: usize) -> Option<(usize, String)> {
    let n = sec.node(p)?;
    let tag = sec.slice(n.start, n.tag_end);
    (n.end == n.tag_end && tag.ends_with("/>")).then(|| (n.tag_end - 2, format!("</{}>", n.name)))
}

/// Where a new run goes in paragraph `p` (node index): at its start (after a first run holding
/// the section or column settings) or end, and the character shape of the run next to it.
fn new_run_place(sec: &Section, p: usize, at_start: bool) -> (usize, String) {
    let runs = sec.kids(p, Some("run"));
    let fallback = || match sec.kids(p, Some("linesegarray")).first().and_then(|&s| sec.node(s)) {
        Some(s) => s.start,
        None => sec.node(p).map(|n| n.inner_end).unwrap_or(0),
    };
    let pick = if at_start { runs.first() } else { runs.last() };
    let Some(&r) = pick else { return (fallback(), "0".into()) };
    let cpr = sec.attr(r, "charPrIDRef", "0");
    let Some(n) = sec.node(r) else { return (fallback(), cpr) };
    let settings = sec.kids(r, None).iter().any(|&k| matches!(sec.local(k), "secPr" | "ctrl"));
    let at = if at_start && !settings { n.start } else { n.end };
    (at, cpr)
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A copied paragraph start tag for a new paragraph: no page or column break, the usual id.
fn new_start_tag(tag: &str) -> String {
    let mut out = String::with_capacity(tag.len());
    let mut i = 0;
    while i < tag.len() {
        let rest = tag.get(i..).unwrap_or("");
        let boundary = tag.get(..i).and_then(|b| b.chars().next_back()).is_none_or(|c| !word_char(c));
        if boundary {
            // attributes only the readers add (paragraph number, source record) don't carry over
            if let Some(name) = [crate::xml::ORDINAL, crate::hwp5::HWP_REC].iter().find(|n| rest.starts_with(&format!("{n}=\""))) {
                let v = rest.get(name.len() + 2..).unwrap_or("");
                let close = v.find('"').map_or(v.len(), |c| c + 1);
                i += name.len() + 2 + close;
                if tag.get(i..).is_some_and(|r| r.starts_with(' ')) {
                    i += 1;
                }
                continue;
            }
            if let Some(name) = ["pageBreak", "columnBreak"].iter().find(|n| rest.starts_with(&format!("{n}=\"1\""))) {
                out.push_str(&format!("{name}=\"0\""));
                i += name.len() + 4;
                continue;
            }
            if let Some(v) = rest.strip_prefix("id=\"")
                && let Some(close) = v.find('"')
            {
                out.push_str(&format!("id=\"{NEW_PARAGRAPH_ID}\""));
                i += 4 + close + 1;
                continue;
            }
        }
        let Some(c) = rest.chars().next() else { break };
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Splice the edits into text. Insertions at one place keep their order; an edit overlapping one
/// already applied is dropped (cannot happen for the edits planned here, which never touch nested
/// paragraphs).
pub(crate) fn apply(text: &str, edits: &[Edit]) -> String {
    let mut order: Vec<(usize, &Edit)> = edits.iter().enumerate().collect();
    order.sort_by_key(|(k, e)| (e.0, e.1, *k));
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for (_, (s, e, new)) in order {
        if *s < pos {
            continue;
        }
        out.push_str(text.get(pos..*s).unwrap_or(""));
        out.push_str(new);
        pos = *e;
    }
    out.push_str(text.get(pos..).unwrap_or(""));
    out
}

/// Write the edits of `edited` (DOCX bytes of the document opened from `original`) into a copy of
/// the original HWPX. Returns the new package and what was written. Text only: see
/// [`write_back_with`] for formatting and tables.
pub fn write_back(original: &[u8], edited: &[u8]) -> Result<(Vec<u8>, SaveReport)> {
    write_back_with(original, edited, None)
}

/// [`write_back`], with formatting and table edits too when `docs` gives the document before
/// editing (the original's conversion as WordCraft reads it) and after.
pub fn write_back_with(
    original: &[u8],
    edited: &[u8],
    docs: Option<(&wordcraft_doc::Document, &wordcraft_doc::Document)>,
) -> Result<(Vec<u8>, SaveReport)> {
    let pkg = Package::open(original)?;
    let mut texts = Vec::new();
    for name in &pkg.section_names {
        let data = pkg.read(name).ok_or_else(|| Error::format(format!("missing part {name}")))?;
        texts.push(String::from_utf8(data.to_vec()).map_err(|e| Error::format(format!("{name}: {e}")))?);
    }
    let header = pkg.header_name.as_deref().and_then(|h| pkg.read(h)).map(|h| String::from_utf8_lossy(h).into_owned());
    let mut plan = plan(&pkg, texts.clone(), header, edited, docs, true, true)?;
    let has_manifest = pkg.rootfile.as_deref().and_then(|r| pkg.read(r)).is_some_and(|d| manifest_end(&String::from_utf8_lossy(d)).is_some());
    if !plan.media.is_empty() && !has_manifest {
        // New pictures need a manifest to list their images in. Without one the sections would
        // name images the package doesn't hold, so only the text is written (as for an .hwp
        // whose header can't take new shapes).
        let n = plan.media.len();
        plan = crate::writer::plan(&pkg, texts, None, edited, None, false, false)?;
        plan.report
            .skipped
            .push(format!("{n} new picture(s) can't be added: the package has no manifest to list them in; only text edits are written"));
    }
    let mut replaced: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (text, name) in plan.sections.into_iter().zip(&pkg.section_names) {
        if let Some(text) = text {
            replaced.insert(name.clone(), text.into_bytes());
        }
    }
    if let (Some(head), Some(name)) = (plan.header, pkg.header_name.clone()) {
        replaced.insert(name, head.into_bytes());
    }
    let mut report = plan.report;
    let mut added: Vec<(String, Vec<u8>)> = Vec::new();
    if !plan.media.is_empty() {
        // the new images go into the manifest; without one to add them to they are left out
        match pkg.rootfile.as_deref().and_then(|r| pkg.read(r).map(|d| (r, String::from_utf8_lossy(d).into_owned()))) {
            Some((name, hpf)) if manifest_spot(&hpf).is_some() => {
                let (at, pfx) = manifest_spot(&hpf).unwrap_or((hpf.len(), String::new()));
                let items: String = plan
                    .media
                    .iter()
                    .map(|(part, id, mime, _)| {
                        format!("<{pfx}item id=\"{}\" href=\"{}\" media-type=\"{mime}\" isEmbeded=\"1\"/>", esc(id), esc(part))
                    })
                    .collect();
                replaced.insert(name.to_string(), format!("{}{items}{}", hpf.get(..at).unwrap_or(""), hpf.get(at..).unwrap_or("")).into_bytes());
                added = plan.media.into_iter().map(|(part, _, _, data)| (part, data)).collect();
            }
            _ => report.skipped.push(format!("{} picture(s): the package has no manifest to list them in", plan.media.len())),
        }
    }
    Ok((write_package(original, &replaced, &added)?, report))
}

/// What a save changes: each section's XML when it changed, the header when shapes were added,
/// and the report.
pub(crate) struct Plan {
    pub(crate) sections: Vec<Option<String>>,
    pub(crate) header: Option<String>,
    /// The shapes added to the header: (element, new id, the shape it copies).
    pub(crate) sources: Vec<(&'static str, i64, Option<String>)>,
    /// Image files to add to the package.
    pub(crate) media: Vec<NewMedia>,
    pub(crate) report: SaveReport,
}

/// Where the manifest's end tag starts in an OPF document.
fn manifest_end(hpf: &str) -> Option<usize> {
    manifest_spot(hpf).map(|(at, _)| at)
}

/// Where new items go in an OPF document: the end of its manifest element's content, found as
/// an element (not text that looks like a tag inside a comment), and the prefix the manifest
/// goes by (`opf:`, `pkg:`, or none), for the items to share its namespace. `None` without a
/// manifest that holds content (one written closed takes the text-only fallback).
fn manifest_spot(hpf: &str) -> Option<(usize, String)> {
    let nodes = parse_spans(hpf);
    let n = nodes.iter().find(|n| n.name == "manifest" || n.name.ends_with(":manifest"))?;
    if hpf.get(n.start..n.tag_end).is_some_and(|t| t.ends_with("/>")) {
        return None;
    }
    let pfx = n.name.strip_suffix("manifest").unwrap_or("").to_string();
    Some((n.inner_end, pfx))
}

/// The number after the highest "image<n>" item id or BinData file name of the package.
fn next_image_number(pkg: &Package) -> usize {
    let number = |s: &str| {
        s.strip_prefix("image").map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>()).and_then(|d| d.parse::<usize>().ok())
    };
    let ids = pkg.items.iter().filter_map(|i| number(&i.id));
    let files = pkg.entries.iter().filter_map(|e| e.name.strip_prefix("BinData/").and_then(number));
    ids.chain(files).max().unwrap_or(0).saturating_add(1)
}

/// The edits of `edited` (DOCX bytes of the document opened from `pkg`) applied to the section
/// XML `texts` of `pkg` (and to its `header` XML, for formatting and, with `tables`, table edits
/// when `docs` are given).
pub(crate) fn plan(
    pkg: &Package,
    texts: Vec<String>,
    header: Option<String>,
    edited: &[u8],
    docs: Option<(&wordcraft_doc::Document, &wordcraft_doc::Document)>,
    tables: bool,
    pictures: bool,
) -> Result<Plan> {
    let (base_docx, _) = crate::convert::Converter::new(pkg)?.convert()?;
    let before = docx_paragraphs(&base_docx)?;
    let after = docx_paragraphs(edited)?;

    // mark -> text in the "before" picture (the first one seen), in order
    let mut base_order: Vec<Mark> = Vec::new();
    let mut base_text: HashMap<Mark, Option<String>> = HashMap::new();
    // a paragraph's further pieces all carry its "hq" mark: their texts, in order
    let mut piece_texts: HashMap<Mark, Vec<Option<String>>> = HashMap::new();
    for d in &before {
        for &m in &d.marks {
            if m.0 == "hq" {
                piece_texts.entry(m).or_default().push(d.text.clone());
            }
            if let std::collections::hash_map::Entry::Vacant(e) = base_text.entry(m) {
                e.insert(d.text.clone());
                base_order.push(m);
            }
        }
    }
    let mut pieces_seen: HashMap<Mark, usize> = HashMap::new();
    // Paragraphs the converter adds itself (separators, spacers) carry no mark; they are not
    // new if the "before" picture has them at the same place with the same text.
    let mut synthetic: HashMap<(&'static str, Option<Mark>, Option<String>), usize> = HashMap::new();
    for (i, place) in unmarked(&before) {
        let text = before.get(i).and_then(|d| d.text.clone());
        *synthetic.entry((place.0, place.1, text)).or_insert(0) += 1;
    }

    let sections: Vec<Section> = texts.into_iter().map(Section::new).collect();
    let n = sections.len();
    let style = match (docs, header) {
        (Some((before, after)), Some(head)) => Some(Restyle {
            before: crate::restyle::collect(before),
            after: crate::restyle::collect(after),
            after_doc: after,
            header: crate::restyle::HeaderEdit::new(head),
        }),
        _ => None,
    };
    let new_pics = style.as_ref().map(|st| st.after.new_pics.clone().into()).unwrap_or_default();
    let mut job = Job {
        sections,
        edits: vec![Vec::new(); n],
        report: SaveReport::default(),
        style,
        pictures,
        hwp: pkg.hwp5_header.is_some(),
        media: Vec::new(),
        next_image: next_image_number(pkg),
        new_pics,
    };

    let mut seen: HashSet<Mark> = HashSet::new();
    // where each source paragraph was: one that now comes before a paragraph it followed was
    // moved, and the write-back keeps the original order
    let base_at: HashMap<Mark, usize> = base_order.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    let mut body_last: Option<usize> = None;
    for d in &after {
        let Some(text) = &d.text else {
            seen.extend(d.marks.iter().copied()); // a table's place
            continue;
        };
        let primary: Vec<Mark> = d.marks.iter().copied().filter(|m| m.0 == "hp" && base_text.contains_key(m) && !seen.contains(m)).collect();
        if d.container == "word/document.xml"
            && let Some(&at) = primary.first().and_then(|m| base_at.get(m))
        {
            const MOVED: &str = "paragraphs were moved: their new order is not written (they keep their places)";
            if body_last.is_some_and(|last| at < last) && !job.report.skipped.iter().any(|s| s == MOVED) {
                job.report.skipped.push(MOVED.into());
            }
            body_last = Some(body_last.map_or(at, |last| last.max(at)));
        }
        let Some(&m) = primary.first() else {
            let cont = d.marks.iter().copied().find(|m| m.0 == "hq" && base_text.contains_key(m));
            if let Some(c) = cont {
                // compared with the piece at the same place (the n-th with this mark)
                let n = pieces_seen.entry(c).or_insert(0);
                let was = piece_texts.get(&c).and_then(|v| v.get(*n)).and_then(|t| t.as_ref());
                *n += 1;
                if was != Some(text) {
                    job.skip(c, "edited text after a table or object in the same paragraph");
                }
            }
            continue;
        };
        seen.extend(primary.iter().copied());
        let restyled = job.style.as_ref().is_some_and(|st| st.chars_changed(m));
        let extra = primary.get(1..).unwrap_or(&[]);
        // a merge is written whole or not at all: every paragraph merged in must be removable
        // before the survivor takes their text, or both texts would stay
        if let Some(&stuck) = extra.iter().find(|&&x| !job.deletable(x)) {
            job.skip(stuck, "a paragraph merged into the one before can't be removed: the merge is not written (both stay)");
            continue;
        }
        let refused_before = job.report.skipped.len();
        match base_text.get(&m).cloned().flatten() {
            Some(b) if b != *text || restyled => job.modify(m, &b, text),
            _ => job.report.unchanged += 1,
        }
        job.merged(m, extra, job.report.skipped.len() > refused_before);
        job.restyle_para(m);
        job.pictures(m);
    }
    // A table whose first row was deleted lost the mark of the paragraph holding it ("ht", read
    // into its first cell): found among the original's tables by its other cells, that mark
    // counts as kept, so the table is written again (row deleted), not deleted.
    let found_tables = job.tables_found_by_cells();
    seen.extend(found_tables);
    let deleted_comments = deleted_comment_marks(&before, &seen);
    let unplaced = unplaced_parts(&before, &after, &synthetic, &mut job.report);
    for m in base_order {
        if deleted_comments.contains(&m) || unplaced.contains(&m) {
            continue;
        }
        if (m.0 == "hp" || m.0 == "ht") && !seen.contains(&m) {
            if base_text.get(&m).is_some_and(Option::is_none) {
                if job.style.is_some() && tables {
                    job.delete_table(m);
                } else {
                    job.skip(m, "a table or object was deleted; deleting tables is not written yet");
                }
            } else {
                job.delete(m);
            }
        }
    }
    for (i, place) in unmarked(&after) {
        let text = after.get(i).and_then(|d| d.text.clone());
        if let Some(c) = synthetic.get_mut(&(place.0, place.1, text.clone()))
            && *c > 0
        {
            *c -= 1;
            continue;
        }
        let d = after.get(i);
        let has_pics = d.is_some_and(|d| d.pictures > 0);
        // a new body paragraph's pictures come from the edited document, in order
        let pics = if has_pics && d.is_some_and(|d| d.container == "word/document.xml") { job.new_pics.pop_front() } else { None };
        let Some(mark) = place.1 else { continue };
        let (anchor, next) = if place.0 == "after" { (Some(mark), None) } else { (None, Some(mark)) };
        if base_text.contains_key(&mark) {
            let pics = if job.pictures { pics.unwrap_or_default() } else { Vec::new() };
            if has_pics && pics.is_empty() {
                job.skip(mark, "a picture in a new paragraph next to this one is not written yet");
            }
            job.insert(anchor, next, text.as_deref().unwrap_or(""), &pics);
        }
    }

    if tables {
        job.tables();
    }
    let sections = finish_sections(&job.sections, &job.edits, &mut job.report);
    let header = job.style.as_ref().and_then(|st| st.header.finish());
    let sources = job.style.map(|st| st.header.sources).unwrap_or_default();
    let mut report = job.report;
    if let Some((before, after)) = docs {
        comment_changes(before, after, &mut report);
    }
    Ok(Plan { sections, header, sources, media: job.media, report })
}

/// The source paragraphs of parts (the body, a header, a footer) whose every source paragraph
/// is gone while new text took their place: the new paragraphs have no source paragraph to be
/// placed by, so they can't be written. Deleting the source paragraphs too would leave the part
/// empty, so they are kept, and the save says the new text isn't written. New paragraphs with no
/// text (everything deleted) don't count: nothing of them is lost.
fn unplaced_parts(
    before: &[DocPara],
    after: &[DocPara],
    synthetic: &HashMap<(&'static str, Option<Mark>, Option<String>), usize>,
    report: &mut SaveReport,
) -> HashSet<Mark> {
    let mut parts: Vec<&str> = Vec::new();
    for (i, place) in unmarked(after) {
        let Some(d) = after.get(i).filter(|_| place.1.is_none()) else { continue };
        let text = d.text.clone().filter(|t| !t.trim().is_empty());
        // a part numbered per document (comments, notes, text boxes) can't be told apart here
        if text.is_none() || d.container.contains('#') || synthetic.contains_key(&(place.0, None, text.clone())) {
            continue;
        }
        if !parts.contains(&d.container.as_str()) {
            parts.push(d.container.as_str());
        }
    }
    let mut kept = HashSet::new();
    for part in parts {
        let marks: Vec<Mark> = before.iter().filter(|d| d.container == part).flat_map(|d| d.marks.iter().copied()).collect();
        if !marks.is_empty() {
            let name = match part {
                "word/document.xml" => "the body",
                p if p.contains("header") => "a header",
                p if p.contains("footer") => "a footer",
                p => p,
            };
            report.skipped.push(format!("new text replacing every paragraph of {name} is not written; the original paragraphs are kept"));
            kept.extend(marks);
        }
    }
    kept
}

/// The writer names what it writes with `hp:`. A part whose root doesn't bind that prefix (one
/// using the paragraph namespace as its default) gets the binding on its root, so the new runs
/// read as the reader reads the rest; one binding `hp` to another namespace keeps its text,
/// as what the writer adds would mean something else there.
fn finish_sections(secs: &[Section], edits: &[Vec<Edit>], report: &mut SaveReport) -> Vec<Option<String>> {
    let mut sections = Vec::new();
    for (k, (sec, edits)) in secs.iter().zip(edits).enumerate() {
        if edits.is_empty() {
            sections.push(None);
            continue;
        }
        let mut edits = edits.clone();
        if let Some(root) = sec.nodes.first() {
            let tag = sec.slice(root.start, root.tag_end);
            let hp = attr_spans(tag).into_iter().find(|a| a.0 == "xmlns:hp").map(|(_, a, b)| tag.get(a..b).unwrap_or(""));
            match hp {
                Some(ns) if ns == P_NS => {}
                Some(_) => {
                    report.skipped.push(format!("section {}: it binds the hp prefix to another namespace; its edits are not written", k + 1));
                    sections.push(None);
                    continue;
                }
                None => {
                    let at = root.start + 1 + root.name.len();
                    edits.push((at, at, format!(" xmlns:hp=\"{P_NS}\"")));
                }
            }
        }
        sections.push(Some(apply(&sec.text, &edits)));
    }
    sections
}

/// The paragraphs of comments deleted whole (none of their paragraphs is in the edited
/// document): their memos stay in the file as they were (deleting memos isn't written;
/// [`comment_changes`] reports it), so those paragraphs aren't deleted or cleared.
fn deleted_comment_marks(before: &[DocPara], seen: &HashSet<Mark>) -> HashSet<Mark> {
    let mut by_comment: HashMap<&str, Vec<Mark>> = HashMap::new();
    for d in before.iter().filter(|d| d.container.starts_with("word/comments.xml#")) {
        by_comment.entry(d.container.as_str()).or_default().extend(d.marks.iter().copied());
    }
    by_comment.into_values().filter(|marks| !marks.iter().any(|m| seen.contains(m))).flatten().collect()
}

/// How many of the paragraph settings a shape variant patches differ between `a` and `b`.
fn para_changes(a: &crate::hwpx_out::ParaKey, b: &crate::hwpx_out::ParaKey) -> usize {
    let flags = |k: &crate::hwpx_out::ParaKey| (k.keep_next, k.keep_lines, k.page_break_before, k.widow, k.break_words);
    [
        a.align != b.align,
        a.indent != b.indent,
        a.left != b.left,
        a.right != b.right,
        a.before != b.before,
        a.after != b.after,
        (a.ls_type, a.ls_value) != (b.ls_type, b.ls_value),
        flags(a) != flags(b),
    ]
    .into_iter()
    .filter(|&c| c)
    .count()
}

/// Comments (Hancom's memos) added or deleted in WordCraft aren't written into the original
/// file: the write-back changes text, not the memo fields around it. Each is reported, so the
/// user knows the file doesn't have the new one, or still has the deleted one, instead of a
/// silent difference. A comment is matched by its author and time, so one whose text was
/// edited (written into its memo's paragraphs) is the same comment.
fn comment_changes(before: &wordcraft_doc::Document, after: &wordcraft_doc::Document, report: &mut SaveReport) {
    let key = |_: &wordcraft_doc::Document, c: &wordcraft_doc::Comment| (c.author.clone(), c.date_utc.clone().unwrap_or_else(|| c.date.clone()));
    let mut left: HashMap<(String, String), usize> = HashMap::new();
    for c in before.comments.values() {
        *left.entry(key(before, c)).or_insert(0) += 1;
    }
    let mut added = 0;
    for c in after.comments.values() {
        match left.get_mut(&key(after, c)) {
            Some(n) if *n > 0 => *n -= 1,
            _ => added += 1,
        }
    }
    let removed: usize = left.values().sum();
    if added > 0 {
        report.skipped.push(format!("{added} new comment(s) are not written to the original file (comments are written to a new .hwpx or .hwp)"));
    }
    if removed > 0 {
        report.skipped.push(format!("{removed} deleted comment(s) stay in the original file as they were"));
    }
}

/// One part of a package read back (the original HWPX, or the DOCX of the edited document).
const MAX_PART: u64 = 256 * 1024 * 1024;
const ZIP_LIMITS: craft_zip::Limits = craft_zip::Limits { max_entries: 65_536, max_total_bytes: 1024 * 1024 * 1024 };

/// Copy the package entry by entry, in order, each stored or deflated as in the original with
/// its content unchanged; replaced parts get new content, and `added` parts come last,
/// deflated. "mimetype" stays first and stored. Folder entries are left out, as in the reader.
fn write_package(original: &[u8], replaced: &BTreeMap<String, Vec<u8>>, added: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut src = craft_zip::Archive::open(original, ZIP_LIMITS).map_err(Error::format)?;
    let mut z = craft_zip::Writer::new();
    for i in 0..src.len() {
        let (Some(name), Some(false)) = (src.entry_name(i).map_err(Error::format)?, src.entry_is_dir(i).map_err(Error::format)?) else { continue };
        let how = if src.is_stored(&name).unwrap_or(false) { craft_zip::Compression::Stored } else { craft_zip::Compression::Deflated };
        match replaced.get(&name) {
            Some(data) => z.add(&name, data, how).map_err(Error::format)?,
            None => {
                let Some(data) = src.read(&name, MAX_PART).map_err(Error::format)? else { continue };
                z.add(&name, &data, how).map_err(Error::format)?;
            }
        }
    }
    for (name, data) in added {
        z.add(name, data, craft_zip::Compression::Deflated).map_err(Error::format)?;
    }
    z.finish().map_err(Error::format)
}

#[cfg(test)]
pub(crate) mod tests_api {
    use super::{Ch, Key};

    /// Where the groups of `before` (their character ranges) end up in `after`.
    pub fn segment_spans(before: &str, after: &str, ranges: &[(usize, usize)]) -> Option<Vec<(usize, usize)>> {
        let segs: Vec<((usize, usize), (usize, usize))> = ranges.iter().map(|&r| ((0, 0), r)).collect();
        super::segment_spans(before, after, &segs)
    }

    pub fn opcodes(a: &str, b: &str) -> Vec<super::Opcode> {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        super::opcodes(&a, &b)
    }

    /// Rewrite characters given as (char, shape id) to `after`; returns (char, shape id).
    pub fn rewrite_text(chars: &[(&str, &str)], after: &str) -> Vec<(char, String)> {
        let chars: Vec<Ch> = chars.iter().filter_map(|(c, k)| Some((c.chars().next()?, (k.to_string(), String::new()), None))).collect();
        let before: String = chars.iter().map(|c| c.0).collect();
        let key: Key = ("0".into(), String::new());
        super::rewrite(&before, after, &chars, &key).into_iter().map(|c| (c.0, c.1.0)).collect()
    }
}
