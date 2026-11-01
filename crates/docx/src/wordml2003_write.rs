//! Word 2003 XML Document writer (WordprocessingML 2003, root `w:wordDocument`, from the public
//! Office 2003 XML Reference Schemas).
//!
//! The mirror image of the reader in `wordml2003`: the DOCX writer produces the package, and the
//! package is rewritten into the 2003 vocabulary. ECMA-376 kept most of the 2003 element names
//! (`w:p`, `w:r`, `w:rPr`, `w:tbl`, `w:sectPr`…), so the body is mostly renamed (`w:szCs` →
//! `w:sz-cs`, `w:eastAsia` → `w:fareast`, `w:vMerge` → `w:vmerge`), and what 2003 keeps inline is
//! moved back inline: hyperlink targets (`w:hlink`), footnotes and endnotes (inside their
//! reference run), headers and footers (inside `w:sectPr`), comments, bookmarks and tracked
//! changes (`aml:annotation`), lists (`w:listPr` and a `w:lists` block), pictures (`w:pict` with
//! `w:binData` in base64) and the core properties (`o:DocumentProperties`).
//!
//! What the 2003 format cannot carry is dropped or degraded, never an error: drawings other than
//! pictures (shapes, text boxes, groups, charts, SmartArt, ink), picture crop, rotation and
//! floating position (pictures become inline), content controls (their content stays), math,
//! revisions of formatting, theme colours and fonts, custom XML, macros, embedded objects and
//! `docDefaults` other than fonts (folded into the Normal style). Elements outside the 2003
//! vocabulary are not written, so Word has nothing to reject. Bounded: nesting is capped by the
//! parser's depth limit, pictures and stories are capped in number.

use std::collections::{BTreeMap, HashMap, HashSet};

use base64::Engine as _;
use wordcraft_doc::Document;

use crate::flat::W2003_NS;
use crate::package::{Package, Rels, rt};
use crate::xml::{El, MAX_DEPTH, Node};
use crate::{DocxError, Flavor};

const AUX_NS: &str = "http://schemas.microsoft.com/office/word/2003/auxHint";
const AML_NS: &str = "http://schemas.microsoft.com/aml/2001/core";
const VML_NS: &str = "urn:schemas-microsoft-com:vml";
const OFFICE_NS: &str = "urn:schemas-microsoft-com:office:office";
const W10_NS: &str = "urn:schemas-microsoft-com:office:word";

/// Most pictures written (the reader takes as many).
const MAX_MEDIA: usize = 4096;
/// Most header, footer, footnote, endnote and comment stories written.
const MAX_STORIES: usize = 50_000;
/// Most bytes of header and footer parts inlined, all sections together.
const MAX_INLINED_BYTES: usize = 64 << 20;
/// A picture's longest side, points (a corrupt extent must not give a huge frame).
const MAX_PICT_PT: f64 = 31_680.0;

/// Write `doc` as a Word 2003 XML Document.
pub fn write(doc: &Document) -> Result<Vec<u8>, DocxError> {
    let parts: BTreeMap<String, Vec<u8>> = crate::write::write_parts(doc, Flavor::Document)?.into_iter().collect();
    let pkg = Package::from_parts(parts);
    let doc_part = pkg
        .rels("_rels/.rels")
        .by_type(rt::OFFICE_DOC)
        .map(|r| r.target.clone())
        .filter(|t| pkg.get(t).is_some())
        .unwrap_or_else(|| "word/document.xml".to_string());
    let mut c = Conv::new(&pkg, &doc_part);

    let body = match pkg.xml(&doc_part)? {
        Some(root) => {
            let rels = c.doc_rels.clone();
            match root.child("w:body") {
                Some(b) => c.kids(b, &rels, "w:body", 1),
                None => Vec::new(),
            }
        }
        None => Vec::new(),
    };
    if c.over_budget {
        return Err(DocxError::Limit(format!("headers and footers inlined past {MAX_INLINED_BYTES} bytes")));
    }

    let mut kids: Vec<Node> = vec![Node::El(core_props(doc))];
    if let Some(f) = c.default_fonts() {
        kids.push(Node::El(f));
    }
    if let Some(l) = c.lists() {
        kids.push(Node::El(l));
    }
    kids.push(Node::El(c.styles()));
    kids.push(Node::El(plain(
        "w:docPr",
        vec![Node::El(attr_el("w:view", &[("w:val", "print")])), Node::El(attr_el("w:zoom", &[("w:percent", "100")]))],
    )));
    kids.push(Node::El(plain("w:body", vec![Node::El(plain("wx:sect", body))])));

    let attrs = [
        ("xmlns:w", W2003_NS),
        ("xmlns:wx", AUX_NS),
        ("xmlns:o", OFFICE_NS),
        ("xmlns:v", VML_NS),
        ("xmlns:w10", W10_NS),
        ("xmlns:aml", AML_NS),
        ("w:macrosPresent", "no"),
        ("w:embeddedObjPresent", "no"),
        ("w:ocxPresent", "no"),
        ("xml:space", "preserve"),
    ];
    let root = El { name: "w:wordDocument".into(), attrs: attrs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect(), kids };
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<?mso-application progid=\"Word.Document\"?>\n");
    s.push_str(&root.to_xml());
    Ok(s.into_bytes())
}

fn plain(name: &str, kids: Vec<Node>) -> El {
    El { name: name.into(), attrs: Vec::new(), kids }
}

fn attr_el(name: &str, attrs: &[(&str, &str)]) -> El {
    El { name: name.into(), attrs: attrs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect(), kids: Vec::new() }
}

fn text_el(name: &str, text: &str) -> El {
    El { name: name.into(), attrs: Vec::new(), kids: vec![Node::Text(text.to_string())] }
}

/// `o:DocumentProperties` from the core properties.
fn core_props(doc: &Document) -> El {
    let c = &doc.core;
    let mut kids = Vec::new();
    let revision = c.revision.to_string();
    for (name, v) in [
        ("o:Title", c.title.as_str()),
        ("o:Subject", c.subject.as_str()),
        ("o:Author", c.creator.as_str()),
        ("o:Keywords", c.keywords.as_str()),
        ("o:Description", c.description.as_str()),
        ("o:LastAuthor", c.last_modified_by.as_str()),
        ("o:Revision", if c.revision > 0 { revision.as_str() } else { "" }),
        ("o:Created", c.created.as_str()),
        ("o:LastSaved", c.modified.as_str()),
        ("o:Category", c.category.as_str()),
    ] {
        if !v.trim().is_empty() {
            kids.push(Node::El(text_el(name, v.trim())));
        }
    }
    plain("o:DocumentProperties", kids)
}

struct Conv<'a> {
    pkg: &'a Package,
    doc_rels: Rels,
    footnotes: HashMap<String, El>,
    endnotes: HashMap<String, El>,
    comments: HashMap<String, El>,
    foot_rels: Rels,
    end_rels: Rels,
    comment_rels: Rels,
    styles_part: Option<El>,
    numbering_part: Option<El>,
    /// Picture part path → `wordml://` name, for the pictures already given a `w:binData`.
    media: HashMap<String, String>,
    /// `w:binData` elements waiting for the `w:pict` being written.
    bins: Vec<El>,
    /// Annotation ids: (kind, original id) → the id written (one number space, as in 2003).
    ann_ids: HashMap<(char, String), i64>,
    next_ann: i64,
    next_shape: i64,
    stories: usize,
    in_note: bool,
    /// Notes and comments already inlined: (kind, id). A repeated reference does not copy the body again.
    inlined_notes: HashSet<(char, String)>,
    /// The header or footer in effect per (footer, type), as the previous sections left it.
    hf_in_effect: HashMap<(bool, &'static str), String>,
    /// Bytes of header and footer parts inlined so far.
    inlined_bytes: usize,
    /// A header or footer did not fit in `MAX_INLINED_BYTES`; the export fails.
    over_budget: bool,
}

impl<'a> Conv<'a> {
    fn new(pkg: &'a Package, doc_part: &str) -> Conv<'a> {
        let doc_rels = pkg.rels(doc_part);
        let part_of = |kind: &str| doc_rels.by_type(kind).map(|r| r.target.clone());
        let notes = |kind: &str, local: &str| -> (HashMap<String, El>, Rels) {
            let Some(path) = part_of(kind) else { return (HashMap::new(), Rels::default()) };
            let map = pkg
                .xml(&path)
                .ok()
                .flatten()
                .map(|root| {
                    root.els()
                        .filter(|e| e.name == local)
                        .filter(|e| e.attr("w:type").is_none())
                        .filter_map(|e| e.attr("w:id").map(|id| (id.to_string(), e.clone())))
                        .collect()
                })
                .unwrap_or_default();
            (map, pkg.rels(&path))
        };
        let (footnotes, foot_rels) = notes(rt::FOOTNOTES, "w:footnote");
        let (endnotes, end_rels) = notes(rt::ENDNOTES, "w:endnote");
        let (comments, comment_rels) = notes(rt::COMMENTS, "w:comment");
        let load = |kind: &str| part_of(kind).and_then(|p| pkg.xml(&p).ok().flatten());
        Conv {
            pkg,
            styles_part: load(rt::STYLES),
            numbering_part: load(rt::NUMBERING),
            doc_rels,
            footnotes,
            endnotes,
            comments,
            foot_rels,
            end_rels,
            comment_rels,
            media: HashMap::new(),
            bins: Vec::new(),
            ann_ids: HashMap::new(),
            next_ann: 0,
            next_shape: 1025,
            stories: 0,
            in_note: false,
            inlined_notes: HashSet::new(),
            hf_in_effect: HashMap::new(),
            inlined_bytes: 0,
            over_budget: false,
        }
    }

    /// The annotation id for `(kind, id)`: the same each time it is asked.
    fn ann_id(&mut self, kind: char, id: &str) -> i64 {
        if let Some(n) = self.ann_ids.get(&(kind, id.to_string())) {
            return *n;
        }
        let n = self.next_ann;
        self.next_ann = self.next_ann.saturating_add(1);
        self.ann_ids.insert((kind, id.to_string()), n);
        n
    }

    fn kids(&mut self, e: &El, rels: &Rels, parent: &str, depth: usize) -> Vec<Node> {
        let mut out = Vec::with_capacity(e.kids.len());
        for n in &e.kids {
            match n {
                Node::Text(t) => out.push(Node::Text(t.clone())),
                Node::El(k) => out.extend(self.node(k, rels, parent, depth + 1)),
            }
        }
        out
    }

    /// `e` rewritten for 2003: nothing (dropped), its content (unwrapped) or one element.
    fn node(&mut self, e: &El, rels: &Rels, parent: &str, depth: usize) -> Vec<Node> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }
        let name = e.name.as_str();
        match name {
            // Content controls: the content stays, the control goes.
            "w:sdt" => return e.child("w:sdtContent").map(|c| self.kids(c, rels, parent, depth)).unwrap_or_default(),
            "w:drawing" => return self.picture(e, rels),
            // A VML fallback is kept for what the modern drawing could not be.
            "mc:AlternateContent" => return e.child("mc:Fallback").map(|c| self.kids(c, rels, parent, depth)).unwrap_or_default(),
            "w:object" | "w:txbxContent" => return Vec::new(),
            "w:footnoteReference" | "w:endnoteReference" => return self.note(e, depth),
            "w:commentReference" => return self.comment(e, depth),
            "w:commentRangeStart" | "w:commentRangeEnd" | "w:bookmarkStart" | "w:bookmarkEnd" => return self.range_marker(e),
            "w:ins" | "w:del" | "w:moveFrom" | "w:moveTo" => {
                // Paragraph-mark and row revisions sit in property blocks: no 2003 form.
                if matches!(parent, "w:rPr" | "w:trPr") {
                    return Vec::new();
                }
                return self.revision(e, rels, parent, depth);
            }
            "w:hyperlink" => return vec![Node::El(self.hyperlink(e, rels, depth))],
            "w:numPr" if parent == "w:pPr" => return vec![Node::El(list_pr(e))],
            "w:numFmt" if parent == "w:lvl" => return vec![Node::El(nfc(e))],
            "w:sectPr" => return vec![Node::El(self.section(e, rels, depth))],
            "w:pict" => return vec![Node::El(self.vml_pict(e, rels, depth))],
            "w:tblLook" => {
                return match e.attr("w:val") {
                    Some(v) => vec![Node::El(attr_el("w:tblLook", &[("w:val", v)]))],
                    None => Vec::new(),
                };
            }
            _ => {}
        }
        if !allowed(name) {
            return Vec::new();
        }
        let mut el = El { name: rename(name), attrs: attrs(e), kids: Vec::new() };
        if name == "v:imagedata" {
            let id = e.attr("r:id").or_else(|| e.attr("r:pict")).map(str::to_string);
            if let Some(src) = id.and_then(|id| rels.by_id(&id).map(|r| r.target.clone())).and_then(|t| self.bin_for(&t)) {
                el.attrs.retain(|(k, _)| k != "src");
                el.attrs.push(("src".into(), src));
            } else {
                return Vec::new();
            }
        }
        el.kids = self.kids(e, rels, name, depth);
        vec![Node::El(el)]
    }

    /// A `wordml://` name for the picture at package path `target`, queueing its `w:binData` the
    /// first time. `None` for a format 2003 XML pictures don't hold, or past the cap.
    fn bin_for(&mut self, target: &str) -> Option<String> {
        if let Some(n) = self.media.get(target) {
            return Some(n.clone());
        }
        let ext = target.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
        if !matches!(ext.as_str(), "png" | "jpeg" | "jpg" | "gif" | "bmp" | "emf" | "wmf" | "tif" | "tiff") || self.media.len() >= MAX_MEDIA {
            return None;
        }
        let bytes = self.pkg.get(target)?;
        let name = format!("wordml://0300{:04}.{ext}", self.media.len() + 1);
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        let bin = El {
            name: "w:binData".into(),
            attrs: vec![("w:name".into(), name.clone()), ("xml:space".into(), "preserve".into())],
            kids: vec![Node::Text(b64)],
        };
        self.bins.push(bin);
        self.media.insert(target.to_string(), name.clone());
        Some(name)
    }

    /// A DrawingML picture as a `w:pict` (inline, at its extent); any other drawing is dropped.
    fn picture(&mut self, drawing: &El, rels: &Rels) -> Vec<Node> {
        let Some(id) = drawing.find("a:blip").and_then(|b| b.attr("r:embed")) else { return Vec::new() };
        let Some(target) = rels.by_id(id).filter(|r| !r.external).map(|r| r.target.clone()) else { return Vec::new() };
        let Some(src) = self.bin_for(&target) else { return Vec::new() };
        let pt = |k: &str| {
            let emu = drawing.find("wp:extent").and_then(|x| x.attr(k)).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
            (if emu.is_finite() { emu / 12700.0 } else { 0.0 }).clamp(0.0, MAX_PICT_PT)
        };
        let style = format!("width:{}pt;height:{}pt", trim_num(pt("cx")), trim_num(pt("cy")));
        let alt = drawing.find("wp:docPr").and_then(|d| d.attr("descr")).unwrap_or("").to_string();
        let shape_id = format!("_x0000_i{}", self.next_shape);
        self.next_shape = self.next_shape.saturating_add(1);
        let mut kids: Vec<Node> = std::mem::take(&mut self.bins).into_iter().map(Node::El).collect();
        kids.push(Node::El(self.shape_frame(&shape_id, &style, &src, &alt)));
        vec![Node::El(plain("w:pict", kids))]
    }

    /// The `v:shape` holding a picture's `v:imagedata`.
    fn shape_frame(&self, id: &str, style: &str, src: &str, alt: &str) -> El {
        let mut shape = attr_el(
            "v:shape",
            &[
                ("id", id),
                ("style", style),
                ("coordsize", "21600,21600"),
                ("o:spt", "75"),
                ("path", "m0,0l0,21600,21600,21600,21600,0xe"),
                ("filled", "f"),
                ("stroked", "f"),
            ],
        );
        if !alt.is_empty() {
            shape.attrs.push(("alt".into(), alt.to_string()));
        }
        let mut data = attr_el("v:imagedata", &[("src", src)]);
        data.attrs.push(("o:title".into(), alt.to_string()));
        shape.kids.push(Node::El(data));
        shape
    }

    /// A legacy `w:pict` (VML) kept as it is, its pictures given `w:binData`.
    fn vml_pict(&mut self, e: &El, rels: &Rels, depth: usize) -> El {
        let kids = self.kids(e, rels, "w:pict", depth);
        let mut all: Vec<Node> = std::mem::take(&mut self.bins).into_iter().map(Node::El).collect();
        all.extend(kids);
        plain("w:pict", all)
    }

    fn hyperlink(&mut self, e: &El, rels: &Rels, depth: usize) -> El {
        let mut a = Vec::new();
        if let Some(r) = e.attr("r:id").and_then(|id| rels.by_id(id)).filter(|r| r.external) {
            a.push(("w:dest".to_string(), r.target.clone()));
        }
        for (from, to) in [("w:anchor", "w:bookmark"), ("w:tooltip", "w:screenTip"), ("w:tgtFrame", "w:target")] {
            if let Some(v) = e.attr(from) {
                a.push((to.to_string(), v.to_string()));
            }
        }
        El { name: "w:hlink".into(), attrs: a, kids: self.kids(e, rels, "w:hlink", depth) }
    }

    /// A footnote or endnote reference as the note itself, inline in its run.
    fn note(&mut self, e: &El, depth: usize) -> Vec<Node> {
        let foot = e.name == "w:footnoteReference";
        let Some(id) = e.attr("w:id") else { return Vec::new() };
        if self.in_note || self.stories >= MAX_STORIES {
            return Vec::new();
        }
        let Some(note) = (if foot { self.footnotes.get(id) } else { self.endnotes.get(id) }).cloned() else { return Vec::new() };
        let name = if foot { "w:footnote" } else { "w:endnote" };
        // The body goes at the first reference only; a repeated one keeps its mark, empty.
        if !self.inlined_notes.insert((if foot { 'f' } else { 'e' }, id.to_string())) {
            return vec![Node::El(plain(name, Vec::new()))];
        }
        self.stories += 1;
        let rels = if foot { self.foot_rels.clone() } else { self.end_rels.clone() };
        self.in_note = true;
        let kids = self.kids(&note, &rels, name, depth);
        self.in_note = false;
        vec![Node::El(plain(name, kids))]
    }

    fn annotation(&self, kind: &str, id: i64, mut extra: Vec<(String, String)>, content: Option<Vec<Node>>) -> El {
        let mut a = vec![("aml:id".to_string(), id.to_string())];
        a.append(&mut extra);
        a.push(("w:type".into(), kind.into()));
        El { name: "aml:annotation".into(), attrs: a, kids: content.map(|c| vec![Node::El(plain("aml:content", c))]).unwrap_or_default() }
    }

    fn comment(&mut self, e: &El, depth: usize) -> Vec<Node> {
        let Some(id) = e.attr("w:id") else { return Vec::new() };
        if self.in_note || self.stories >= MAX_STORIES {
            return Vec::new();
        }
        let Some(c) = self.comments.get(id).cloned() else { return Vec::new() };
        // A comment is written once, at its first reference.
        if !self.inlined_notes.insert(('c', id.to_string())) {
            return Vec::new();
        }
        self.stories += 1;
        let rels = self.comment_rels.clone();
        let n = self.ann_id('c', id);
        self.in_note = true;
        let content = self.kids(&c, &rels, "w:comment", depth);
        self.in_note = false;
        let mut extra = Vec::new();
        if let Some(v) = c.attr("w:author") {
            extra.push(("aml:author".to_string(), v.to_string()));
        }
        if let Some(v) = c.attr("w:date") {
            extra.push(("aml:createdate".to_string(), v.to_string()));
        }
        if let Some(v) = c.attr("w:initials") {
            extra.push(("w:initials".to_string(), v.to_string()));
        }
        vec![Node::El(self.annotation("Word.Comment", n, extra, Some(content)))]
    }

    fn range_marker(&mut self, e: &El) -> Vec<Node> {
        let Some(id) = e.attr("w:id") else { return Vec::new() };
        let (kind, ty) = match e.name.as_str() {
            "w:commentRangeStart" => ('c', "Word.Comment.Start"),
            "w:commentRangeEnd" => ('c', "Word.Comment.End"),
            "w:bookmarkStart" => ('b', "Word.Bookmark.Start"),
            _ => ('b', "Word.Bookmark.End"),
        };
        let n = self.ann_id(kind, id);
        let extra = if e.name == "w:bookmarkStart" { vec![("w:name".to_string(), e.attr("w:name").unwrap_or("").to_string())] } else { Vec::new() };
        vec![Node::El(self.annotation(ty, n, extra, None))]
    }

    /// Tracked insertions and deletions (moves become a deletion and an insertion).
    fn revision(&mut self, e: &El, rels: &Rels, parent: &str, depth: usize) -> Vec<Node> {
        let ty = if matches!(e.name.as_str(), "w:ins" | "w:moveTo") { "Word.Insertion" } else { "Word.Deletion" };
        let n = self.ann_id('r', e.attr("w:id").unwrap_or(""));
        let mut extra = Vec::new();
        if let Some(v) = e.attr("w:author") {
            extra.push(("aml:author".to_string(), v.to_string()));
        }
        if let Some(v) = e.attr("w:date") {
            extra.push(("aml:createdate".to_string(), v.to_string()));
        }
        let content = self.kids(e, rels, parent, depth);
        vec![Node::El(self.annotation(ty, n, extra, Some(content)))]
    }

    /// `w:sectPr` with its headers and footers inline (`w:hdr`, `w:ftr`).
    fn section(&mut self, e: &El, rels: &Rels, depth: usize) -> El {
        let mut stories = Vec::new();
        let mut rest = Vec::new();
        for k in e.els() {
            if k.name == "w:headerReference" || k.name == "w:footerReference" {
                let footer = k.name == "w:footerReference";
                if self.stories >= MAX_STORIES {
                    continue;
                }
                let Some(path) = k.attr("r:id").and_then(|id| rels.by_id(id)).filter(|r| !r.external).map(|r| r.target.clone()) else { continue };
                let ty = match k.attr("w:type") {
                    Some("even") => "even",
                    Some("first") => "first",
                    _ => "odd",
                };
                // A section without the story inherits the previous section's, as in OOXML: write
                // it only where it changes.
                if self.hf_in_effect.get(&(footer, ty)) == Some(&path) {
                    continue;
                }
                let size = self.pkg.get(&path).map_or(0, |b| b.len());
                if self.inlined_bytes.saturating_add(size) > MAX_INLINED_BYTES {
                    // Skipping would leave later sections inheriting the wrong story: fail instead.
                    self.over_budget = true;
                    continue;
                }
                let Some(root) = self.pkg.xml(&path).ok().flatten() else { continue };
                self.stories += 1;
                self.inlined_bytes = self.inlined_bytes.saturating_add(size);
                self.hf_in_effect.insert((footer, ty), path.clone());
                let story_rels = self.pkg.rels(&path);
                let name = if footer { "w:ftr" } else { "w:hdr" };
                let was = std::mem::replace(&mut self.in_note, true);
                let kids = self.kids(&root, &story_rels, name, depth + 1);
                self.in_note = was;
                let mut story = plain(name, kids);
                story.attrs.push(("w:type".into(), ty.into()));
                stories.push(Node::El(story));
            } else {
                rest.extend(self.node(k, rels, "w:sectPr", depth + 1));
            }
        }
        stories.extend(rest);
        El { name: "w:sectPr".into(), attrs: Vec::new(), kids: stories }
    }

    /// `w:fonts/w:defaultFonts` from the document defaults.
    fn default_fonts(&self) -> Option<El> {
        let f = self.styles_part.as_ref()?.child("w:docDefaults")?.child("w:rPrDefault")?.child("w:rPr")?.child("w:rFonts")?;
        let a = attrs(f);
        (!a.is_empty()).then(|| plain("w:fonts", vec![Node::El(El { name: "w:defaultFonts".into(), attrs: a, kids: Vec::new() })]))
    }

    /// `w:styles`: the styles, with the document defaults (other than fonts) folded into the
    /// default paragraph style, as 2003 has no `docDefaults`.
    fn styles(&mut self) -> El {
        let Some(root) = self.styles_part.clone() else { return plain("w:styles", Vec::new()) };
        let defaults = root.child("w:docDefaults");
        let default_of = |part: &str, block: &str| -> Vec<El> {
            defaults
                .and_then(|d| d.child(part))
                .and_then(|p| p.child(block))
                .map(|b| b.els().filter(|e| e.name != "w:rFonts").cloned().collect())
                .unwrap_or_default()
        };
        let (def_r, def_p) = (default_of("w:rPrDefault", "w:rPr"), default_of("w:pPrDefault", "w:pPr"));
        let rels = self.doc_rels.clone();
        let mut kids = Vec::new();
        for st in root.children("w:style") {
            let mut st = st.clone();
            if st.attr("w:type") == Some("paragraph") && matches!(st.attr("w:default"), Some("1" | "true" | "on")) {
                fold(&mut st, "w:pPr", &def_p);
                fold(&mut st, "w:rPr", &def_r);
            }
            kids.extend(self.node(&st, &rels, "w:styles", 2));
        }
        plain("w:styles", kids)
    }

    /// `w:lists`: `w:abstractNum` → `w:listDef`, `w:num` → `w:list`.
    fn lists(&mut self) -> Option<El> {
        let root = self.numbering_part.clone()?;
        let rels = self.doc_rels.clone();
        let mut kids = Vec::new();
        for abs in root.children("w:abstractNum") {
            let Some(id) = abs.attr("w:abstractNumId") else { continue };
            let mut def = El { name: "w:listDef".into(), attrs: vec![("w:listDefId".into(), id.to_string())], kids: Vec::new() };
            for k in abs.els() {
                match k.name.as_str() {
                    "w:nsid" => def.kids.push(Node::El(attr_el("w:lsid", &[("w:val", k.attr("w:val").unwrap_or(""))]))),
                    "w:multiLevelType" => {
                        let v = match k.attr("w:val").unwrap_or("") {
                            "singleLevel" => "SingleLevel",
                            "hybridMultilevel" => "HybridMultilevel",
                            _ => "Multilevel",
                        };
                        def.kids.push(Node::El(attr_el("w:plt", &[("w:val", v)])));
                    }
                    "w:numStyleLink" => def.kids.push(Node::El(attr_el("w:listStyleLink", &[("w:val", k.attr("w:val").unwrap_or(""))]))),
                    "w:lvl" => def.kids.extend(self.node(k, &rels, "w:abstractNum", 3)),
                    _ => {}
                }
            }
            kids.push(Node::El(def));
        }
        for num in root.children("w:num") {
            let (Some(id), Some(abs)) = (num.attr("w:numId"), num.child_val("w:abstractNumId")) else { continue };
            let mut list = El { name: "w:list".into(), attrs: vec![("w:ilfo".into(), id.to_string())], kids: Vec::new() };
            list.kids.push(Node::El(attr_el("w:ilst", &[("w:val", abs)])));
            for o in num.children("w:lvlOverride") {
                list.kids.extend(self.node(o, &rels, "w:num", 3));
            }
            kids.push(Node::El(list));
        }
        (!kids.is_empty()).then(|| plain("w:lists", kids))
    }
}

/// Add the properties of `defaults` that block `name` of `style` doesn't set (at its start).
fn fold(style: &mut El, name: &str, defaults: &[El]) {
    if defaults.is_empty() {
        return;
    }
    let pos = style.kids.iter().position(|n| matches!(n, Node::El(e) if e.name == name));
    let idx = match pos {
        Some(i) => i,
        None => {
            // Schema order within a style: name, basedOn, next, link, … then pPr, rPr.
            let at = if name == "w:pPr" {
                style
                    .kids
                    .iter()
                    .position(|n| matches!(n, Node::El(e) if e.name == "w:rPr" || e.name.starts_with("w:tbl") || e.name.starts_with("w:t")))
            } else {
                style.kids.iter().position(|n| matches!(n, Node::El(e) if e.name.starts_with("w:tbl") || e.name.starts_with("w:t")))
            }
            .unwrap_or(style.kids.len());
            style.kids.insert(at, Node::El(plain(name, Vec::new())));
            at
        }
    };
    if let Some(Node::El(block)) = style.kids.get_mut(idx) {
        let have: Vec<String> = block.els().map(|e| e.name.clone()).collect();
        let mut add: Vec<Node> = defaults.iter().filter(|d| !have.contains(&d.name)).cloned().map(Node::El).collect();
        add.append(&mut block.kids);
        block.kids = add;
    }
}

/// `w:numPr` (`w:ilvl`, `w:numId`) → `w:listPr` (`w:ilvl`, `w:ilfo`).
fn list_pr(e: &El) -> El {
    let mut kids = Vec::new();
    if let Some(v) = e.child_val("w:ilvl") {
        kids.push(Node::El(attr_el("w:ilvl", &[("w:val", v)])));
    }
    if let Some(v) = e.child_val("w:numId") {
        kids.push(Node::El(attr_el("w:ilfo", &[("w:val", v)])));
    }
    plain("w:listPr", kids)
}

/// `w:numFmt` → `w:nfc` (a number format code).
fn nfc(e: &El) -> El {
    let code = match e.attr("w:val").unwrap_or("decimal") {
        "upperRoman" => "1",
        "lowerRoman" => "2",
        "upperLetter" => "3",
        "lowerLetter" => "4",
        "ordinal" => "5",
        "cardinalText" => "6",
        "ordinalText" => "7",
        "chicago" => "9",
        "decimalFullWidth" => "14",
        "decimalHalfWidth" => "15",
        "decimalEnclosedCircle" => "18",
        "decimalZero" => "22",
        "bullet" => "23",
        "ganada" => "24",
        "chosung" => "25",
        "decimalEnclosedFullstop" => "26",
        "decimalEnclosedParen" => "27",
        "none" => "255",
        _ => "0",
    };
    attr_el("w:nfc", &[("w:val", code)])
}

/// A number without a pointless fraction (`36`, `10.5`).
fn trim_num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Is this element written? WordprocessingML elements of the 2003 vocabulary, and the VML a
/// picture needs; everything else (DrawingML, `w14`, math, markup compatibility…) is not.
fn allowed(name: &str) -> bool {
    match name.split_once(':') {
        Some(("w", local)) => allowed_w(local),
        Some(("v" | "o" | "w10", _)) => true,
        _ => false,
    }
}

fn allowed_w(local: &str) -> bool {
    matches!(
        local,
        "p" | "pPr"
            | "r"
            | "rPr"
            | "t"
            | "tab"
            | "br"
            | "cr"
            | "noBreakHyphen"
            | "softHyphen"
            | "sym"
            | "pgNum"
            | "delText"
            | "instrText"
            | "fldChar"
            | "fldSimple"
            | "footnoteRef"
            | "endnoteRef"
            | "annotationRef"
            | "separator"
            | "continuationSeparator"
            | "tbl"
            | "tblPr"
            | "tblGrid"
            | "gridCol"
            | "tr"
            | "trPr"
            | "tc"
            | "tcPr"
            | "sectPr"
            | "hdr"
            | "ftr"
            | "pStyle"
            | "keepNext"
            | "keepLines"
            | "pageBreakBefore"
            | "framePr"
            | "widowControl"
            | "suppressLineNumbers"
            | "pBdr"
            | "shd"
            | "tabs"
            | "suppressAutoHyphens"
            | "kinsoku"
            | "wordWrap"
            | "overflowPunct"
            | "topLinePunct"
            | "autoSpaceDE"
            | "autoSpaceDN"
            | "bidi"
            | "adjustRightInd"
            | "snapToGrid"
            | "spacing"
            | "ind"
            | "contextualSpacing"
            | "mirrorIndents"
            | "textAlignment"
            | "textDirection"
            | "textboxTightWrap"
            | "outlineLvl"
            | "jc"
            | "divId"
            | "top"
            | "left"
            | "bottom"
            | "right"
            | "between"
            | "bar"
            | "insideH"
            | "insideV"
            | "tl2br"
            | "tr2bl"
            | "rStyle"
            | "rFonts"
            | "b"
            | "bCs"
            | "i"
            | "iCs"
            | "caps"
            | "smallCaps"
            | "strike"
            | "dstrike"
            | "outline"
            | "shadow"
            | "emboss"
            | "imprint"
            | "noProof"
            | "vanish"
            | "webHidden"
            | "color"
            | "w"
            | "kern"
            | "position"
            | "sz"
            | "szCs"
            | "highlight"
            | "u"
            | "effect"
            | "bdr"
            | "fitText"
            | "vertAlign"
            | "rtl"
            | "cs"
            | "em"
            | "lang"
            | "eastAsianLayout"
            | "specVanish"
            | "tblStyle"
            | "tblpPr"
            | "tblOverlap"
            | "bidiVisual"
            | "tblStyleRowBandSize"
            | "tblStyleColBandSize"
            | "tblW"
            | "tblCellSpacing"
            | "tblInd"
            | "tblBorders"
            | "tblLayout"
            | "tblCellMar"
            | "cantSplit"
            | "trHeight"
            | "tblHeader"
            | "gridBefore"
            | "gridAfter"
            | "wBefore"
            | "wAfter"
            | "hidden"
            | "tcW"
            | "gridSpan"
            | "hMerge"
            | "vMerge"
            | "tcBorders"
            | "noWrap"
            | "tcMar"
            | "tcFitText"
            | "vAlign"
            | "pgSz"
            | "pgMar"
            | "paperSrc"
            | "pgBorders"
            | "lnNumType"
            | "pgNumType"
            | "cols"
            | "col"
            | "formProt"
            | "noEndnote"
            | "titlePg"
            | "rtlGutter"
            | "docGrid"
            | "type"
            | "style"
            | "name"
            | "aliases"
            | "basedOn"
            | "next"
            | "link"
            | "autoRedefine"
            | "semiHidden"
            | "qFormat"
            | "locked"
            | "tblStylePr"
            | "lvl"
            | "start"
            | "lvlRestart"
            | "isLgl"
            | "suff"
            | "lvlText"
            | "legacy"
            | "lvlJc"
            | "lvlOverride"
            | "startOverride"
            | "ilvl"
            | "binData"
    )
}

/// The 2003 spelling of an ECMA-376 name: `w:szCs` → `w:sz-cs`, `w:eastAsia` → `w:fareast`,
/// `w:vMerge` → `w:vmerge`. Names outside `w:` are kept.
fn rename(name: &str) -> String {
    let Some(local) = name.strip_prefix("w:") else { return name.to_string() };
    let new = match local {
        "eastAsia" => "fareast",
        "vMerge" => "vmerge",
        "hMerge" => "hmerge",
        "bCs" => "b-cs",
        "iCs" => "i-cs",
        "szCs" => "sz-cs",
        "hAnsi" => "h-ansi",
        "firstLine" => "first-line",
        "lineRule" => "line-rule",
        "hRule" => "h-rule",
        "linePitch" => "line-pitch",
        "firstLineChars" => "first-line-chars",
        "hangingChars" => "hanging-chars",
        "leftChars" => "left-chars",
        "rightChars" => "right-chars",
        other => other,
    };
    format!("w:{new}")
}

/// Elements whose `w:val` is an on/off switch (2003 writes `on` / `off`).
fn is_toggle(local: &str) -> bool {
    matches!(
        local,
        "b" | "bCs"
            | "i"
            | "iCs"
            | "caps"
            | "smallCaps"
            | "strike"
            | "dstrike"
            | "outline"
            | "shadow"
            | "emboss"
            | "imprint"
            | "noProof"
            | "snapToGrid"
            | "vanish"
            | "webHidden"
            | "keepNext"
            | "keepLines"
            | "pageBreakBefore"
            | "widowControl"
            | "suppressLineNumbers"
            | "suppressAutoHyphens"
            | "wordWrap"
            | "contextualSpacing"
            | "mirrorIndents"
            | "rtl"
            | "cs"
            | "specVanish"
            | "tblHeader"
            | "cantSplit"
            | "noWrap"
            | "hidden"
            | "semiHidden"
            | "qFormat"
            | "locked"
            | "tcFitText"
            | "isLgl"
            | "bidi"
            | "overflowPunct"
            | "kinsoku"
            | "autoSpaceDE"
            | "autoSpaceDN"
            | "adjustRightInd"
            | "topLinePunct"
            | "titlePg"
    )
}

/// The attributes of `e` that 2003 has, renamed: theme references, revision ids and attributes of
/// other vocabularies (`w14:`, `r:`…) are dropped.
fn attrs(e: &El) -> Vec<(String, String)> {
    let local_el = e.name.strip_prefix("w:").unwrap_or("");
    let vml = e.name.starts_with("v:") || e.name.starts_with("o:") || e.name.starts_with("w10:");
    let mut out = Vec::new();
    for (k, v) in &e.attrs {
        let key = match k.split_once(':') {
            Some(("w", l)) => {
                let lower = l.to_ascii_lowercase();
                if lower.starts_with("rsid") || lower.contains("theme") {
                    continue;
                }
                rename(k)
            }
            Some(("xml", "space")) => k.clone(),
            Some(("v" | "o" | "w10", _)) if vml => k.clone(),
            None if vml => k.clone(),
            _ => continue,
        };
        let mut val = v.clone();
        if (key == "w:val" && is_toggle(local_el)) || ((key == "w:default" || key == "w:customStyle") && e.name == "w:style") {
            val = switch(&val);
        }
        out.push((key, val));
    }
    out
}

/// An on/off value in 2003's spelling.
fn switch(v: &str) -> String {
    match v {
        "1" | "true" | "on" => "on".into(),
        "0" | "false" | "off" => "off".into(),
        other => other.into(),
    }
}
