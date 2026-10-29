//! Word 2003 XML Document (WordprocessingML 2003, root `w:wordDocument`, from the public Office
//! 2003 XML Reference Schemas). Import; the writer is `wordml2003_write`.
//!
//! ECMA-376 kept most of the 2003 vocabulary (`w:p`, `w:r`, `w:pPr`, `w:rPr`, `w:tbl`,
//! `w:sectPr`, `w:styles`…), so the file is opened by rewriting it as the package it corresponds
//! to and reading that with the DOCX reader: one reader, one set of property rules. The rewrite
//! renames what changed (hyphenated names, `w:hlink`, `w:listPr`, `w:nfc`, `w:vmerge`…), moves
//! what 2003 keeps inline into parts (lists into numbering, headers and footers, footnotes,
//! endnotes, comments, `w:binData` pictures into media) and turns `aml:annotation` bookmarks,
//! comments and tracked insertions/deletions into their ECMA-376 elements. Bounded: the parser
//! caps depth and element count, pictures are capped in number and size.

use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use wordcraft_doc::Document;

use crate::DocxError;
use crate::flat::W2003_NS;
use crate::package::{MAX_PART, MAX_TOTAL, Package, rt};
use crate::write::PartRels;
use crate::xml::{self, El, MAX_DEPTH, NAMESPACES, Node, W};

/// Auxiliary hints (`wx:`): display hints Word writes alongside the real markup.
const AUX_NS: &str = "http://schemas.microsoft.com/office/word/2003/auxHint";
/// Annotation markup (`aml:`): bookmarks, comments, tracked changes.
const AML_NS: &str = "http://schemas.microsoft.com/aml/2001/core";

/// Most pictures (`w:binData`) taken from one file.
const MAX_MEDIA: usize = 4096;
/// Most header, footer, footnote, endnote and comment stories.
const MAX_STORIES: usize = 50_000;

/// Read a Word 2003 XML Document.
pub fn read(bytes: &[u8]) -> Result<Document, DocxError> {
    crate::read::read_opened(&convert(bytes)?)
}

/// Parts the converted package gets relationships for.
const DOC: usize = 0;
const FOOTNOTES: usize = 1;
const ENDNOTES: usize = 2;
const COMMENTS: usize = 3;

struct Conv {
    /// `w:binData` name (`wordml://…`) → media target relative to `word/`.
    media: HashMap<String, String>,
    files: BTreeMap<String, Vec<u8>>,
    /// Relationships per part: document, footnotes, endnotes, comments, then one per header or
    /// footer (index in `stories` + 4).
    rels: Vec<PartRels>,
    footnotes: Vec<Node>,
    endnotes: Vec<Node>,
    comments: Vec<Node>,
    /// Header and footer parts: (part name under `word/`, root element).
    stories: Vec<(String, El)>,
    next_note: i64,
    story_count: usize,
}

/// Rewrite a Word 2003 XML Document as a WordprocessingML package.
pub(crate) fn convert(bytes: &[u8]) -> Result<Package, DocxError> {
    let root = xml::parse_ns(bytes, &[("w", W2003_NS), ("wx", AUX_NS), ("aml", AML_NS)])?;
    if root.name != "w:wordDocument" {
        return Err(DocxError::NotWord(format!("root element is {}", root.name)));
    }
    let mut c = Conv {
        media: HashMap::new(),
        files: BTreeMap::new(),
        rels: (0..4).map(|_| PartRels::default()).collect(),
        footnotes: Vec::new(),
        endnotes: Vec::new(),
        comments: Vec::new(),
        stories: Vec::new(),
        next_note: 1,
        story_count: 0,
    };
    let mut total = 0u64;
    c.collect_media(&root, 0, &mut total)?;

    // Body: `wx:sect` / `wx:sub-section` wrappers unwrapped; the last section's properties last.
    let mut body = El { name: "w:body".into(), ..El::default() };
    if let Some(b) = root.child("w:body") {
        body.kids = c.kids(b, DOC, 1);
    }
    let last_sect = body.kids.iter().rposition(|n| matches!(n, Node::El(e) if e.name == "w:sectPr"));
    if let Some(i) = last_sect {
        let s = body.kids.remove(i);
        body.kids.retain(|n| !matches!(n, Node::El(e) if e.name == "w:sectPr"));
        body.kids.push(s);
    }
    let mut document = El { name: "w:document".into(), attrs: xml::body_ns(), kids: Vec::new() };
    document.kids.push(Node::El(body));

    let mut overrides: Vec<(String, &str)> = Vec::new();
    let wml = |s: &str| format!("application/vnd.openxmlformats-officedocument.wordprocessingml.{s}+xml");
    let styles = c.styles(&root);
    c.put("word/styles.xml", &styles);
    c.rels[DOC].add(rt::STYLES, "styles.xml", false);
    overrides.push(("/word/styles.xml".into(), "styles"));
    if let Some(n) = root.child("w:lists").map(|l| c.numbering(l)) {
        c.put("word/numbering.xml", &n);
        c.rels[DOC].add(rt::NUMBERING, "numbering.xml", false);
        overrides.push(("/word/numbering.xml".into(), "numbering"));
    }
    for (part, root_name, kids, rel, ct) in [
        (FOOTNOTES, "w:footnotes", std::mem::take(&mut c.footnotes), rt::FOOTNOTES, "footnotes"),
        (ENDNOTES, "w:endnotes", std::mem::take(&mut c.endnotes), rt::ENDNOTES, "endnotes"),
        (COMMENTS, "w:comments", std::mem::take(&mut c.comments), rt::COMMENTS, "comments"),
    ] {
        if kids.is_empty() {
            continue;
        }
        let file = format!("{ct}.xml");
        c.put(&format!("word/{file}"), &El { name: root_name.into(), attrs: xml::body_ns(), kids });
        c.put_rels(&file, part);
        c.rels[DOC].add(rel, &file, false);
        overrides.push((format!("/word/{file}"), ct));
    }
    for (i, (file, story)) in std::mem::take(&mut c.stories).into_iter().enumerate() {
        c.put(&format!("word/{file}"), &story);
        c.put_rels(&file, i + 4);
        overrides.push((format!("/word/{file}"), if story.name == "w:ftr" { "footer" } else { "header" }));
    }
    c.put("word/document.xml", &document);
    c.put_rels("document.xml", DOC);

    let mut root_rels = PartRels::default();
    root_rels.add(rt::OFFICE_DOC, "word/document.xml", false);
    if let Some(props) = root.child("o:DocumentProperties") {
        c.files.insert("docProps/core.xml".into(), core_xml(props));
        root_rels.add(rt::CORE, "docProps/core.xml", false);
    }
    c.files.insert("_rels/.rels".into(), root_rels.xml());

    let mut w = W::new();
    w.open("Types", &[("xmlns", "http://schemas.openxmlformats.org/package/2006/content-types")]);
    w.empty("Default", &[("Extension", "rels"), ("ContentType", "application/vnd.openxmlformats-package.relationships+xml")]);
    w.empty("Default", &[("Extension", "xml"), ("ContentType", "application/xml")]);
    let mut exts: Vec<&str> = c.media.values().filter_map(|t| t.rsplit_once('.').map(|(_, e)| e)).collect();
    exts.sort_unstable();
    exts.dedup();
    for ext in exts {
        w.empty("Default", &[("Extension", ext), ("ContentType", media_type(ext))]);
    }
    w.empty("Override", &[("PartName", "/word/document.xml"), ("ContentType", &wml("document.main"))]);
    for (part, kind) in &overrides {
        w.empty("Override", &[("PartName", part), ("ContentType", &wml(kind))]);
    }
    if c.files.contains_key("docProps/core.xml") {
        w.empty("Override", &[("PartName", "/docProps/core.xml"), ("ContentType", "application/vnd.openxmlformats-package.core-properties+xml")]);
    }
    w.close("Types");
    c.files.insert("[Content_Types].xml".into(), w.into_bytes());
    Ok(Package::from_parts(c.files))
}

impl Conv {
    /// Store an element as a part.
    fn put(&mut self, path: &str, root: &El) {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
        s.push_str(&root.to_xml());
        self.files.insert(path.into(), s.into_bytes());
    }

    /// Store the relationships of `word/{file}` (if it has any).
    fn put_rels(&mut self, file: &str, part: usize) {
        if let Some(r) = self.rels.get(part).filter(|r| !r.is_empty()) {
            let x = r.xml();
            self.files.insert(format!("word/_rels/{file}.rels"), x);
        }
    }

    /// Every `w:binData` in the file, decoded into a media part.
    fn collect_media(&mut self, e: &El, depth: usize, total: &mut u64) -> Result<(), DocxError> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        for k in e.els() {
            if k.name == "w:binData" {
                let Some(name) = k.attr("w:name").filter(|n| !self.media.contains_key(*n)) else { continue };
                if self.media.len() >= MAX_MEDIA {
                    continue;
                }
                let text = k.text();
                let b64: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
                if (b64.len() / 4 * 3) as u64 > MAX_PART {
                    return Err(DocxError::Limit(format!("picture {name} is larger than the size limit")));
                }
                let Ok(bytes) = base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64.trim_end_matches('=')) else {
                    log::warn!("wordml2003: skipping picture {name}: invalid base64");
                    continue;
                };
                *total = total.saturating_add(bytes.len() as u64);
                if *total > MAX_TOTAL {
                    return Err(DocxError::Limit("pictures larger than the size limit".into()));
                }
                let ext = picture_ext(&bytes, name);
                let target = format!("media/image{}.{ext}", self.media.len() + 1);
                self.files.insert(format!("word/{target}"), bytes);
                self.media.insert(name.to_string(), target);
            } else {
                self.collect_media(k, depth + 1, total)?;
            }
        }
        Ok(())
    }

    /// The converted children of `e`.
    fn kids(&mut self, e: &El, part: usize, depth: usize) -> Vec<Node> {
        let mut out = Vec::with_capacity(e.kids.len());
        for n in &e.kids {
            match n {
                Node::Text(t) => out.push(Node::Text(t.clone())),
                Node::El(k) => out.extend(self.node(k, part, depth + 1)),
            }
        }
        out
    }

    /// `e` converted: nothing (dropped), its children (unwrapped) or one element.
    fn node(&mut self, e: &El, part: usize, depth: usize) -> Vec<Node> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }
        match e.name.as_str() {
            "wx:sect" | "wx:sub-section" | "wx:pBdrGroup" => return self.kids(e, part, depth),
            "w:binData" | "w:fonts" | "w:docPr" | "w:lists" | "w:styles" | "w:latentStyles" | "w:versionOfBuiltInStyleNames" => return Vec::new(),
            "w:hlink" => return vec![Node::El(self.hyperlink(e, part, depth))],
            "w:listPr" => return vec![Node::El(list_pr(e))],
            "w:footnote" | "w:endnote" => return self.note(e, depth).into_iter().map(Node::El).collect(),
            "aml:annotation" => return self.annotation(e, part, depth),
            "w:sectPr" => return vec![Node::El(self.section(e, part, depth))],
            "w:nfc" => return vec![Node::El(num_fmt(e))],
            _ => {}
        }
        if !keep(&e.name) {
            return Vec::new();
        }
        let mut el = El { name: rename(&e.name), attrs: attrs(e), kids: self.kids(e, part, depth) };
        if el.name == "v:imagedata"
            && let Some(target) = e.attr("src").and_then(|s| self.media.get(s)).cloned()
            && let Some(r) = self.rels.get_mut(part)
        {
            let id = r.add(rt::IMAGE, &target, false);
            el.attrs.retain(|(k, _)| k != "r:id");
            el.attrs.push(("r:id".into(), id));
        }
        if el.name == "w:style" {
            for (k, v) in &mut el.attrs {
                if k == "w:default" && v == "on" {
                    *v = "1".into();
                }
            }
        }
        vec![Node::El(el)]
    }

    /// `w:hlink` → `w:hyperlink` (an external relationship for `w:dest`).
    fn hyperlink(&mut self, e: &El, part: usize, depth: usize) -> El {
        let mut attrs = Vec::new();
        if let Some(dest) = e.attr("w:dest").filter(|d| !d.is_empty())
            && let Some(r) = self.rels.get_mut(part)
        {
            attrs.push(("r:id".into(), r.add(rt::HYPERLINK, dest, true)));
        }
        for (from, to) in [("w:bookmark", "w:anchor"), ("w:screenTip", "w:tooltip"), ("w:target", "w:tgtFrame")] {
            if let Some(v) = e.attr(from) {
                attrs.push((to.into(), v.to_string()));
            }
        }
        El { name: "w:hyperlink".into(), attrs, kids: self.kids(e, part, depth) }
    }

    /// A footnote or endnote written inline in its run → a reference, the note in its part.
    fn note(&mut self, e: &El, depth: usize) -> Option<El> {
        // Separator definitions (`w:type`) live in document settings, not in the text.
        if e.attr("w:type").is_some() || self.story_count >= MAX_STORIES {
            return None;
        }
        self.story_count += 1;
        let foot = e.name == "w:footnote";
        let id = self.next_note.to_string();
        self.next_note = self.next_note.saturating_add(1);
        let part = if foot { FOOTNOTES } else { ENDNOTES };
        let kids = self.kids(e, part, depth);
        let note = El { name: e.name.clone(), attrs: vec![("w:id".into(), id.clone())], kids };
        if foot {
            self.footnotes.push(Node::El(note));
        } else {
            self.endnotes.push(Node::El(note));
        }
        let name = if foot { "w:footnoteReference" } else { "w:endnoteReference" };
        Some(El { name: name.into(), attrs: vec![("w:id".into(), id)], kids: Vec::new() })
    }

    /// `aml:annotation`: bookmarks, comments and tracked insertions/deletions.
    fn annotation(&mut self, e: &El, part: usize, depth: usize) -> Vec<Node> {
        let id = e.attr("aml:id").filter(|v| v.parse::<i64>().is_ok()).unwrap_or("0").to_string();
        let el = |name: &str, attrs: Vec<(String, String)>| vec![Node::El(El { name: name.into(), attrs, kids: Vec::new() })];
        let author_date = |e: &El| {
            let mut a = vec![("w:id".to_string(), id.clone())];
            if let Some(v) = e.attr("aml:author") {
                a.push(("w:author".into(), v.to_string()));
            }
            if let Some(v) = e.attr("aml:createdate") {
                a.push(("w:date".into(), v.to_string()));
            }
            a
        };
        let content = |c: &mut Conv, part: usize| e.child("aml:content").map(|k| c.kids(k, part, depth + 1)).unwrap_or_default();
        match e.attr("w:type").unwrap_or("") {
            "Word.Bookmark.Start" => {
                el("w:bookmarkStart", vec![("w:id".into(), id.clone()), ("w:name".into(), e.attr("w:name").unwrap_or("").to_string())])
            }
            "Word.Bookmark.End" => el("w:bookmarkEnd", vec![("w:id".into(), id)]),
            "Word.Comment.Start" => el("w:commentRangeStart", vec![("w:id".into(), id)]),
            "Word.Comment.End" => el("w:commentRangeEnd", vec![("w:id".into(), id)]),
            "Word.Comment" => {
                if self.story_count >= MAX_STORIES {
                    return Vec::new();
                }
                self.story_count += 1;
                let mut attrs = author_date(e);
                if let Some(v) = e.attr("w:initials") {
                    attrs.push(("w:initials".into(), v.to_string()));
                }
                let kids = content(self, COMMENTS);
                self.comments.push(Node::El(El { name: "w:comment".into(), attrs, kids }));
                el("w:commentReference", vec![("w:id".into(), id)])
            }
            t @ ("Word.Insertion" | "Word.Deletion") => {
                let name = if t == "Word.Insertion" { "w:ins" } else { "w:del" };
                vec![Node::El(El { name: name.into(), attrs: author_date(e), kids: content(self, part) })]
            }
            _ => Vec::new(),
        }
    }

    /// `w:sectPr`: inline `w:hdr` / `w:ftr` become header and footer parts.
    fn section(&mut self, e: &El, part: usize, depth: usize) -> El {
        let mut refs = Vec::new();
        let mut rest = Vec::new();
        for n in &e.kids {
            let Node::El(k) = n else { continue };
            if (k.name == "w:hdr" || k.name == "w:ftr") && self.story_count < MAX_STORIES {
                self.story_count += 1;
                let footer = k.name == "w:ftr";
                let kind = match k.attr("w:type") {
                    Some("even") => "even",
                    Some("first") => "first",
                    _ => "default",
                };
                let idx = self.stories.len() + 4;
                self.rels.push(PartRels::default());
                let kids = self.kids(k, idx, depth + 1);
                let file = format!("{}{}.xml", if footer { "footer" } else { "header" }, self.stories.len() + 1);
                self.stories.push((file.clone(), El { name: k.name.clone(), attrs: xml::body_ns(), kids }));
                let id = self.rels.get_mut(part).map(|r| r.add(if footer { rt::FOOTER } else { rt::HEADER }, &file, false)).unwrap_or_default();
                let name = if footer { "w:footerReference" } else { "w:headerReference" };
                refs.push(Node::El(El { name: name.into(), attrs: vec![("w:type".into(), kind.into()), ("r:id".into(), id)], kids: Vec::new() }));
            } else if k.name != "w:hdr" && k.name != "w:ftr" {
                rest.extend(self.node(k, part, depth + 1));
            }
        }
        refs.extend(rest);
        El { name: "w:sectPr".into(), attrs: attrs(e), kids: refs }
    }

    /// `w:styles` → `styles.xml`, with the document's default fonts (`w:fonts/w:defaultFonts`).
    fn styles(&mut self, root: &El) -> El {
        let mut kids = Vec::new();
        if let Some(f) = root.child("w:fonts").and_then(|f| f.child("w:defaultFonts")) {
            let fonts = El { name: "w:rFonts".into(), attrs: attrs(f), kids: Vec::new() };
            let rpr = El { name: "w:rPr".into(), attrs: Vec::new(), kids: vec![Node::El(fonts)] };
            let rpr_default = El { name: "w:rPrDefault".into(), attrs: Vec::new(), kids: vec![Node::El(rpr)] };
            kids.push(Node::El(El { name: "w:docDefaults".into(), attrs: Vec::new(), kids: vec![Node::El(rpr_default)] }));
        }
        if let Some(s) = root.child("w:styles") {
            for st in s.children("w:style") {
                kids.extend(self.node(st, DOC, 2));
            }
        }
        El { name: "w:styles".into(), attrs: xml::body_ns(), kids }
    }

    /// `w:lists` → `numbering.xml`: `w:listDef` → `w:abstractNum`, `w:list` → `w:num`.
    fn numbering(&mut self, lists: &El) -> El {
        let mut kids = Vec::new();
        for def in lists.children("w:listDef") {
            let Some(id) = def.attr("w:listDefId") else { continue };
            let mut a = El { name: "w:abstractNum".into(), attrs: vec![("w:abstractNumId".into(), id.to_string())], kids: Vec::new() };
            for k in def.els() {
                let converted = match k.name.as_str() {
                    "w:plt" => {
                        let v = match k.attr("w:val").unwrap_or("") {
                            "SingleLevel" => "singleLevel",
                            "HybridMultilevel" => "hybridMultilevel",
                            _ => "multilevel",
                        };
                        vec![Node::El(val_el("w:multiLevelType", v))]
                    }
                    "w:lsid" => vec![Node::El(val_el("w:nsid", k.attr("w:val").unwrap_or("")))],
                    "w:listStyleLink" => vec![Node::El(val_el("w:numStyleLink", k.attr("w:val").unwrap_or("")))],
                    _ => self.node(k, DOC, 2),
                };
                a.kids.extend(converted);
            }
            kids.push(Node::El(a));
        }
        for list in lists.children("w:list") {
            let Some(id) = list.attr("w:ilfo") else { continue };
            let Some(abs) = list.child_val("w:ilst") else { continue };
            let mut n =
                El { name: "w:num".into(), attrs: vec![("w:numId".into(), id.to_string())], kids: vec![Node::El(val_el("w:abstractNumId", abs))] };
            for o in list.children("w:lvlOverride") {
                n.kids.extend(self.node(o, DOC, 2));
            }
            kids.push(Node::El(n));
        }
        El { name: "w:numbering".into(), attrs: xml::body_ns(), kids }
    }
}

/// `<name w:val="v"/>`.
fn val_el(name: &str, v: &str) -> El {
    El { name: name.into(), attrs: vec![("w:val".into(), v.into())], kids: Vec::new() }
}

/// `w:listPr` (`w:ilvl`, `w:ilfo`) → `w:numPr` (`w:ilvl`, `w:numId`).
fn list_pr(e: &El) -> El {
    let mut kids = Vec::new();
    if let Some(v) = e.child_val("w:ilvl") {
        kids.push(Node::El(val_el("w:ilvl", v)));
    }
    if let Some(v) = e.child_val("w:ilfo") {
        kids.push(Node::El(val_el("w:numId", v)));
    }
    El { name: "w:numPr".into(), attrs: Vec::new(), kids }
}

/// `w:nfc` (a number format code) → `w:numFmt`.
fn num_fmt(e: &El) -> El {
    let f = match e.attr("w:val").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(0) {
        1 => "upperRoman",
        2 => "lowerRoman",
        3 => "upperLetter",
        4 => "lowerLetter",
        5 => "ordinal",
        6 => "cardinalText",
        7 => "ordinalText",
        9 => "chicago",
        14 | 19 => "decimalFullWidth",
        15 => "decimalHalfWidth",
        18 => "decimalEnclosedCircle",
        22 => "decimalZero",
        23 => "bullet",
        24 => "ganada",
        25 => "chosung",
        26 => "decimalEnclosedFullstop",
        27 => "decimalEnclosedParen",
        255 => "none",
        _ => "decimal",
    };
    val_el("w:numFmt", f)
}

/// Is an element or attribute with this name kept (a namespace the package knows, or none)?
fn keep(name: &str) -> bool {
    match name.split_once(':') {
        None => true,
        Some(("xml", _)) => true,
        Some((p, _)) => NAMESPACES.iter().any(|(q, _)| *q == p),
    }
}

/// A WordprocessingML 2003 name in its ECMA-376 spelling: `w:sz-cs` → `w:szCs`, `w:first-line`
/// → `w:firstLine`, `w:fareast` → `w:eastAsia`, `w:vmerge` → `w:vMerge`.
fn rename(name: &str) -> String {
    let Some(local) = name.strip_prefix("w:") else { return name.to_string() };
    match local {
        "fareast" => return "w:eastAsia".into(),
        "vmerge" => return "w:vMerge".into(),
        "hmerge" => return "w:hMerge".into(),
        _ => {}
    }
    let mut out = String::from("w:");
    let mut up = false;
    for c in local.chars() {
        if c == '-' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// The attributes of `e` that the package keeps, renamed.
fn attrs(e: &El) -> Vec<(String, String)> {
    e.attrs.iter().filter(|(k, _)| keep(k)).map(|(k, v)| (rename(k), v.clone())).collect()
}

/// A picture's file extension: from its bytes, else from its `wordml://` name.
fn picture_ext(b: &[u8], name: &str) -> String {
    let sniffed = if b.starts_with(b"\x89PNG") {
        "png"
    } else if b.starts_with(&[0xFF, 0xD8]) {
        "jpeg"
    } else if b.starts_with(b"GIF8") {
        "gif"
    } else if b.starts_with(b"BM") {
        "bmp"
    } else if b.get(40..44) == Some(b" EMF") {
        "emf"
    } else if b.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || b.starts_with(&[0x01, 0x00, 0x09, 0x00]) {
        "wmf"
    } else {
        ""
    };
    if !sniffed.is_empty() {
        return sniffed.into();
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    if !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext != "xml" && ext != "rels" {
        ext
    } else {
        "bin".into()
    }
}

fn media_type(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpeg" | "jpg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        _ => "application/octet-stream",
    }
}

/// `o:DocumentProperties` → `docProps/core.xml`.
fn core_xml(props: &El) -> Vec<u8> {
    let mut w = W::new();
    w.open(
        "cp:coreProperties",
        &[
            ("xmlns:cp", "http://schemas.openxmlformats.org/package/2006/metadata/core-properties"),
            ("xmlns:dc", "http://purl.org/dc/elements/1.1/"),
            ("xmlns:dcterms", "http://purl.org/dc/terms/"),
        ],
    );
    for (from, to) in [
        ("o:Title", "dc:title"),
        ("o:Subject", "dc:subject"),
        ("o:Author", "dc:creator"),
        ("o:Keywords", "cp:keywords"),
        ("o:Description", "dc:description"),
        ("o:LastAuthor", "cp:lastModifiedBy"),
        ("o:Revision", "cp:revision"),
        ("o:Category", "cp:category"),
        ("o:Created", "dcterms:created"),
        ("o:LastSaved", "dcterms:modified"),
    ] {
        if let Some(t) = props.child(from).map(El::text).filter(|t| !t.trim().is_empty()) {
            w.leaf(to, &[], t.trim());
        }
    }
    w.close("cp:coreProperties");
    w.into_bytes()
}
