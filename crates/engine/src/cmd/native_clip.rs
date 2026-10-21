//! WordCraft's own clipboard format, "WordCraft Fragment": a copy with what it needs to paste the
//! same in another WordCraft window — each window is a process of its own, and a copy's rich
//! fragment lives only in the window it was made in. Beside the text, HTML and RTF, Copy puts the
//! fragment on the system clipboard as JSON: its blocks and text box stories, and the styles,
//! lists, tracked changes and pictures of the document it came from.
//!
//! Pasting it in another window keeps the source formatting the way Word's Keep Source
//! Formatting does: styles the destination lacks come with it, and where a style it uses means
//! something else there (that document's Normal is another font, its Heading 1 another size, its
//! defaults another spacing), its look is given to the pasted paragraphs and runs as direct
//! formatting. Styles that mean the same in both documents are left alone.
//!
//! The JSON comes from another program as far as we know: it is size-capped, read leniently and
//! repaired like any document opened from JSON.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::{Block, CharProps, Document, ParaProps, StyleKind, StyleSheet};

use super::insert::{base64_decode, base64_encode};
use super::paste::each_para;
use crate::Session;

/// The clipboard format's name on Windows (a registered format).
pub const FORMAT_NAME: &str = "WordCraft Fragment";
/// The clipboard format's MIME type (Wayland).
pub const MIME: &str = "application/x-wordcraft-fragment";
/// Largest payload written or read, in bytes (pictures travel in it, base64).
pub const MAX_NATIVE_BYTES: usize = 64 << 20;
/// What the payload's `format` says.
const MAGIC: &str = "wordcraft-fragment";
/// The payload's version.
const VERSION: u32 = 1;
/// Deepest style chain or table nesting followed.
const MAX_DEPTH: usize = 64;

#[derive(Serialize, Deserialize)]
struct Payload {
    /// [`MAGIC`]; first, so [`looks_native`] can tell a payload by its start.
    format: String,
    version: u32,
    /// The copy's text, as the system clipboard's text beside it.
    #[serde(default)]
    text: String,
    /// The copy as a document: its body is the fragment's blocks, its parts the text box stories
    /// they show; styles, lists and tracked changes are the source document's. Pictures go in
    /// [`Payload::media`].
    doc: Document,
    /// The pictures the copy shows, base64, by the source document's media key.
    #[serde(default)]
    media: BTreeMap<String, String>,
    /// The copy is whole paragraphs (see `Session::clipboard_whole`).
    #[serde(default)]
    whole: bool,
}

/// A payload read back: the copy's text, the copy as a document, and whether it is whole
/// paragraphs.
pub struct Decoded {
    pub text: String,
    pub doc: Document,
    pub whole: bool,
}

/// The copy in the session's clipboard as a document of its own: the fragment's blocks and text
/// box stories with the source document's styles, lists, tracked changes and the pictures it
/// shows. What Copy exports as HTML, RTF and WordCraft Fragment, so other programs and windows
/// see it as it looks here (not in the built-in styles). `None` when nothing was copied.
pub fn copy_document(s: &Session) -> Option<Document> {
    let f = s.clipboard.as_ref()?;
    let mut d = Document::new();
    d.body = f.blocks.iter().cloned().map(Arc::new).collect();
    d.parts = f.parts.clone();
    d.styles = s.doc.styles.clone();
    d.numbering = s.doc.numbering.clone();
    d.revisions = s.doc.revisions.clone();
    d.media = used_media(f, &s.doc.media);
    Some(d)
}

/// The pictures of `media` that the fragment refers to (by key, anywhere in its blocks or text
/// box stories).
fn used_media(f: &Fragment, media: &BTreeMap<String, Arc<Vec<u8>>>) -> BTreeMap<String, Arc<Vec<u8>>> {
    if media.is_empty() {
        return BTreeMap::new();
    }
    let Ok(json) = serde_json::to_string(f) else { return BTreeMap::new() };
    media.iter().filter(|(k, _)| serde_json::to_string(k).is_ok_and(|quoted| json.contains(&quoted))).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// The session's copy as a WordCraft Fragment payload; `None` when nothing was copied or it is
/// larger than [`MAX_NATIVE_BYTES`].
pub fn encode(s: &Session, doc: Option<&Document>) -> Option<String> {
    let mut doc = match doc {
        Some(d) => d.clone(),
        None => copy_document(s)?,
    };
    let media = std::mem::take(&mut doc.media).iter().map(|(k, v)| (k.clone(), base64_encode(v))).collect();
    let payload = Payload { format: MAGIC.into(), version: VERSION, text: s.clipboard_text.clone(), doc, media, whole: s.clipboard_whole };
    let json = serde_json::to_string(&payload).ok()?;
    (json.len() <= MAX_NATIVE_BYTES).then_some(json)
}

/// Whether `json` starts like a WordCraft Fragment payload (a cheap test; [`decode`] reads it).
pub fn looks_native(json: &str) -> bool {
    json.len() <= MAX_NATIVE_BYTES && json.trim_start().starts_with(&format!("{{\"format\":\"{MAGIC}\""))
}

/// A payload's text and document, its pictures restored and its runs repaired; `None` for
/// anything that isn't a payload we can read.
pub fn decode(json: &str) -> Option<Decoded> {
    if !looks_native(json) {
        return None;
    }
    let p: Payload = serde_json::from_str(json).ok()?;
    if p.format != MAGIC || p.version == 0 {
        return None;
    }
    let mut doc = p.doc;
    doc.media = p.media.iter().filter_map(|(k, b64)| Some((k.clone(), Arc::new(base64_decode(b64)?)))).collect();
    doc.normalize_runs();
    Some(Decoded { text: p.text, doc, whole: p.whole })
}

/// A short id of a payload, to tell the window's own copy from another window's.
pub fn id(json: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    json.hash(&mut h);
    // 0 means "no copy" in the session.
    h.finish().max(1)
}

/// Another window's copy as a fragment of this document, keeping its look (see the module docs).
pub(crate) fn adopt(s: &mut Session, mut src: Document) -> Fragment {
    // The styles here once the paste brings in those this document lacks, as `paste::adopt` does.
    let mut here = s.doc.styles.clone();
    for st in &src.styles.styles {
        if here.get(&st.id).is_none() {
            here.upsert(st.clone());
        }
    }
    keep_source_look(&src.styles, &here, &mut src.body);
    for part in src.parts.values_mut() {
        keep_source_look(&src.styles, &here, &mut part.blocks);
    }
    super::paste::adopt(s, src)
}

/// Paragraph properties that aren't formatting a style gives (or that refer to the source
/// document's own ids): never copied from a style into direct formatting.
const PARA_SKIP: [&str; 4] = ["style", "numbering", "numChange", "fmtChange"];
/// The same for character properties.
const CHAR_SKIP: [&str; 5] = ["style", "link", "ins", "del", "fmtChange"];

/// Give `blocks` (from a document styled by `src`) the look they had there in a document styled
/// by `here`: where the style chain (with the document defaults) of a paragraph, its mark or a
/// run gives other formatting here, the source's goes in as direct formatting, unless the
/// paragraph or run sets that property itself.
pub(crate) fn keep_source_look(src: &StyleSheet, here: &StyleSheet, blocks: &mut [Arc<Block>]) {
    let para_neutral = neutral(&StyleSheet::empty().resolve_para(&ParaProps::default()), |o| serde_json::from_value::<ParaProps>(o).is_ok());
    let char_neutral = neutral(&StyleSheet::empty().resolve_char(None, &CharProps::default()), |o| serde_json::from_value::<CharProps>(o).is_ok());
    let mut para_patches: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut char_patches: BTreeMap<(String, Option<String>), Map<String, Value>> = BTreeMap::new();
    for b in blocks.iter_mut() {
        each_para(Arc::make_mut(b), 0, &mut |p| {
            let style = p.props.style.clone().unwrap_or_else(|| "Normal".into());
            let mut changed = false;
            let patch = para_patches.entry(style.clone()).or_insert_with(|| {
                let (sp, _) = src.para_style_props(&style);
                let (hp, _) = here.para_style_props(&style);
                diff(&object(&sp), &object(&hp), &para_neutral, &PARA_SKIP)
            });
            changed |= fill(&mut p.props, patch);
            let mut char_patch = |c: &mut CharProps| {
                let key = (style.clone(), c.style.clone());
                let patch = char_patches.entry(key).or_insert_with(|| {
                    let s = char_base(src, &style, c.style.as_deref());
                    let h = char_base(here, &style, c.style.as_deref());
                    diff(&object(&s), &object(&h), &char_neutral, &CHAR_SKIP)
                });
                fill(c, patch)
            };
            changed |= char_patch(&mut p.mark);
            for run in &mut p.runs {
                changed |= char_patch(&mut run.props);
            }
            if changed {
                p.normalize();
                p.touch();
            }
        });
    }
}

/// The character formatting a run gets from the document defaults, its paragraph's style chain
/// and its character style chain (the order `StyleSheet::resolve_char` applies them in).
fn char_base(styles: &StyleSheet, para_style: &str, char_style: Option<&str>) -> CharProps {
    let (_, mut cp) = styles.para_style_props(para_style);
    if let Some(cs) = char_style {
        for st in styles.chain(cs).into_iter().take(MAX_DEPTH) {
            if matches!(st.kind, StyleKind::Character | StyleKind::Paragraph) {
                cp.overlay(&st.chr);
            }
        }
    }
    cp.style = None;
    cp
}

/// `v` as a JSON object (properties that are set; an unset one is absent).
fn object<T: Serialize>(v: &T) -> Map<String, Value> {
    match serde_json::to_value(v) {
        Ok(Value::Object(o)) => o,
        _ => Map::new(),
    }
}

/// What a property resolves to when nothing sets it, for the properties `fits` takes (a resolved
/// value of another type than the property's is left out).
fn neutral<T: Serialize>(resolved: &T, fits: impl Fn(Value) -> bool) -> Map<String, Value> {
    object(resolved)
        .into_iter()
        .filter(|(k, v)| {
            let mut one = Map::new();
            one.insert(k.clone(), v.clone());
            fits(Value::Object(one))
        })
        .collect()
}

/// The properties to set so formatting `here` looks like formatting `src`: those `src` sets to
/// another value, and those only `here` sets, back to what they are when unset (where known).
fn diff(src: &Map<String, Value>, here: &Map<String, Value>, neutral: &Map<String, Value>, skip: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in src {
        if !skip.contains(&k.as_str()) && here.get(k) != Some(v) {
            out.insert(k.clone(), v.clone());
        }
    }
    for (k, v) in here {
        if skip.contains(&k.as_str()) || src.contains_key(k) {
            continue;
        }
        if let Some(n) = neutral.get(k)
            && n != v
        {
            out.insert(k.clone(), n.clone());
        }
    }
    out
}

/// Set the properties of `patch` that `props` doesn't set itself. `true` when something changed.
fn fill<T: Serialize + DeserializeOwned>(props: &mut T, patch: &Map<String, Value>) -> bool {
    if patch.is_empty() {
        return false;
    }
    let mut o = object(&*props);
    let before = o.len();
    for (k, v) in patch {
        if !o.contains_key(k) {
            o.insert(k.clone(), v.clone());
        }
    }
    if o.len() == before {
        return false;
    }
    match serde_json::from_value::<T>(Value::Object(o)) {
        Ok(t) => {
            *props = t;
            true
        }
        Err(e) => {
            log::warn!("keep source formatting: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::StyleSheet;

    use super::*;

    /// A session whose document styles Normal unlike the built-in one: Georgia 14 pt, no space
    /// after, 1.5 lines; with a heading, then an indented paragraph in bold.
    fn source() -> Session {
        let mut doc = Document::from_text("Title\nIndented body\nSecond");
        if let Some(n) = doc.styles.get_mut("Normal") {
            n.chr.font = Some("Georgia".into());
            n.chr.size = Some(14.0);
            n.para.space_after = Some(0.0);
            n.para.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.5));
        }
        if let Some(Block::Para(p)) = doc.body.get_mut(0).map(Arc::make_mut) {
            p.props.style = Some("Heading1".into());
        }
        if let Some(Block::Para(p)) = doc.body.get_mut(1).map(Arc::make_mut) {
            p.props.indent_left = Some(36.0);
            for r in &mut p.runs {
                r.props.bold = Some(true);
            }
        }
        Session::new(doc)
    }

    fn body_para(s: &Session, i: usize) -> wordcraft_doc::Paragraph {
        match s.doc.body.get(i).map(|b| &**b) {
            Some(Block::Para(p)) => p.clone(),
            _ => panic!("no paragraph {i}"),
        }
    }

    fn select_all_and_copy(s: &mut Session) -> Value {
        s.run("select.all", &json!({})).unwrap();
        s.run("edit.copy", &json!({})).unwrap()
    }

    #[test]
    fn a_copy_carries_a_payload_that_reads_back() {
        let mut a = source();
        let r = select_all_and_copy(&mut a);
        let payload = r["wordcraft"].as_str().expect("the copy has a WordCraft Fragment");
        assert!(looks_native(payload));
        let Decoded { text, doc, .. } = decode(payload).expect("it reads back");
        assert_eq!(text, a.clipboard_text);
        assert_eq!(doc.styles.get("Normal").and_then(|n| n.chr.font.clone()).as_deref(), Some("Georgia"));
        assert_eq!(a.clipboard_native_id, id(payload));
    }

    #[test]
    fn pasting_in_another_window_keeps_the_source_look() {
        let mut a = source();
        let r = select_all_and_copy(&mut a);
        let mut b = Session::new(Document::new());
        b.run("edit.paste", &json!({"text": r["text"], "html": r["html"], "wordcraft": r["wordcraft"]})).unwrap();
        let here = &b.doc.styles;
        for i in 0..3 {
            let p = body_para(&b, i);
            let src = body_para(&a, i);
            assert_eq!(p.text, src.text);
            let pa = a.doc.styles.resolve_para(&src.props);
            let pb = here.resolve_para(&p.props);
            assert_eq!(pb.space_after, pa.space_after, "paragraph {i} spacing after");
            assert_eq!(pb.line_spacing, pa.line_spacing, "paragraph {i} line spacing");
            assert_eq!(pb.indent_left, pa.indent_left, "paragraph {i} indent");
            let ca = a.doc.styles.resolve_char(src.props.style.as_deref(), &src.runs.first().map(|r| r.props.clone()).unwrap_or_default());
            let cb = here.resolve_char(p.props.style.as_deref(), &p.runs.first().map(|r| r.props.clone()).unwrap_or_default());
            assert_eq!((cb.font, cb.size, cb.bold), (ca.font, ca.size, ca.bold), "paragraph {i} characters");
        }
        // The heading stays a heading; the destination's own Normal is untouched.
        assert_eq!(body_para(&b, 0).props.style.as_deref(), Some("Heading1"));
        assert_eq!(here.get("Normal"), StyleSheet::builtin().get("Normal"));
    }

    #[test]
    fn the_same_styles_paste_without_direct_formatting() {
        let mut a = Session::new(Document::new());
        a.run("text.insert", &json!({"text": "Plain words"})).unwrap();
        let r = select_all_and_copy(&mut a);
        let mut b = Session::new(Document::new());
        b.run("edit.paste", &json!({"text": r["text"], "wordcraft": r["wordcraft"]})).unwrap();
        let p = body_para(&b, 0);
        assert_eq!(p.text, "Plain words");
        assert!(p.runs.iter().all(|r| r.props.font.is_none() && r.props.size.is_none()), "{:?}", p.runs);
    }

    #[test]
    fn the_window_s_own_copy_pastes_as_copied() {
        let mut a = source();
        a.run("select.all", &json!({})).unwrap();
        a.run("para.bullets", &json!({})).unwrap();
        let lists = a.doc.numbering.nums.len();
        assert!(lists > 0);
        let r = select_all_and_copy(&mut a);
        a.run("edit.paste", &json!({"text": r["text"], "wordcraft": r["wordcraft"]})).unwrap();
        // This window's copy, not another document's: its lists aren't brought in again.
        assert_eq!(a.doc.numbering.nums.len(), lists);
        // Another window's paste brings them, numbered there.
        let mut b = Session::new(Document::new());
        b.run("edit.paste", &json!({"text": r["text"], "wordcraft": r["wordcraft"]})).unwrap();
        assert!(!b.doc.numbering.nums.is_empty());
        assert!(body_para(&b, 1).props.numbering.is_some_and(|n| b.doc.numbering.nums.iter().any(|x| x.id == n.num)));
    }

    #[test]
    fn a_stale_payload_gives_way_to_the_clipboard_text() {
        let mut a = source();
        let r = select_all_and_copy(&mut a);
        let mut b = Session::new(Document::new());
        // Another program copied since: the text isn't the payload's.
        b.run("edit.paste", &json!({"text": "Something else", "wordcraft": r["wordcraft"]})).unwrap();
        assert_eq!(body_para(&b, 0).text, "Something else");
    }

    #[test]
    fn hostile_payloads_never_panic() {
        let mut deep = String::from("{\"format\":\"wordcraft-fragment\",\"version\":1,\"doc\":");
        deep.push_str(&"[".repeat(5000));
        let junk = [
            "",
            "{",
            "{\"format\":\"wordcraft-fragment\"}",
            "{\"format\":\"wordcraft-fragment\",\"version\":1,\"doc\":{\"body\":[{\"Para\":{\"text\":\"abc\",\"runs\":[{\"len\":99999}]}}]}}",
            "{\"format\":\"wordcraft-fragment\",\"version\":1,\"doc\":{},\"media\":{\"x\":\"!!!\"}}",
            deep.as_str(),
        ];
        for j in junk {
            let mut b = Session::new(Document::new());
            let _ = b.run("edit.paste", &json!({"text": "abc", "wordcraft": j}));
            let _ = b.run("edit.pasteSpecial", &json!({"as": "formatted", "text": "abc", "wordcraft": j}));
            let _ = b.run("edit.pasteMerge", &json!({"text": "abc", "wordcraft": j}));
        }
    }

    #[test]
    fn pictures_travel_with_the_copy() {
        let mut a = Session::new(Document::new());
        let png = wordcraft_render::Rendered { width: 2, height: 2, pixels: vec![255; 16] }.to_png();
        a.run("insert.picture", &json!({"data": base64_encode(&png)})).unwrap();
        let r = select_all_and_copy(&mut a);
        let mut b = Session::new(Document::new());
        b.run("edit.paste", &json!({"text": r["text"], "wordcraft": r["wordcraft"]})).unwrap();
        assert_eq!(b.doc.media.len(), 1, "the picture came with the paste");
        assert!(b.doc.media.values().any(|d| **d == png));
    }

    /// Spacing before and after and line spacing come with a copy, as with Word's Keep Source
    /// Formatting: the first pasted paragraph keeps its own though the paragraph it joins has
    /// formatting of its own, and a whole paragraph pasted into an empty one brings its
    /// formatting; a piece of a paragraph pasted into text takes that paragraph's.
    #[test]
    fn paragraph_spacing_comes_with_the_copy() {
        use wordcraft_doc::props::LineSpacing;
        let spaced = |d: &mut Document| {
            for b in d.body.iter_mut() {
                if let Block::Para(p) = Arc::make_mut(b) {
                    p.props.space_before = Some(18.0);
                    p.props.space_after = Some(30.0);
                    p.props.line_spacing = Some(LineSpacing::Multiple(2.0));
                }
            }
        };
        // The destination's paragraphs state spacing of their own, as a DOCX's often do.
        let dest = || {
            let mut d = Document::from_text("Here\n");
            for b in d.body.iter_mut() {
                if let Block::Para(p) = Arc::make_mut(b) {
                    p.props.space_before = Some(0.0);
                    p.props.space_after = Some(0.0);
                }
            }
            Session::new(d)
        };
        let spacing = |s: &Session, i: usize| {
            let p = body_para(s, i);
            (p.props.space_before, p.props.space_after, p.props.line_spacing)
        };
        let want = (Some(18.0), Some(30.0), Some(LineSpacing::Multiple(2.0)));
        let mut src = Document::from_text("One\nTwo\nThree");
        spaced(&mut src);
        let mut a = Session::new(src);
        let copy = |a: &mut Session, from: (usize, usize), to: (usize, usize)| {
            a.run("select.range", &json!({"anchor": {"block": from.0, "off": from.1}, "focus": {"block": to.0, "off": to.1}})).unwrap();
            a.run("edit.copy", &json!({})).unwrap()
        };
        let paste_at = |b: &mut Session, r: &Value, block: usize, off: usize| {
            b.run("select.range", &json!({"anchor": {"block": block, "off": off}, "focus": {"block": block, "off": off}})).unwrap();
            b.run("edit.paste", &json!({"text": r["text"], "wordcraft": r["wordcraft"]})).unwrap();
        };
        // Two whole paragraphs into the empty last line.
        let r = copy(&mut a, (0, 0), (1, 3));
        let mut b = dest();
        paste_at(&mut b, &r, 1, 0);
        assert_eq!(body_para(&b, 1).text, "One");
        assert_eq!(spacing(&b, 1), want, "the first pasted paragraph");
        assert_eq!(spacing(&b, 2), want, "the last pasted paragraph");
        assert_eq!(spacing(&b, 0), (Some(0.0), Some(0.0), None), "the paragraph before is untouched");
        // One whole paragraph into an empty line.
        let r = copy(&mut a, (2, 0), (2, 5));
        let mut b = dest();
        paste_at(&mut b, &r, 1, 0);
        assert_eq!(body_para(&b, 1).text, "Three");
        assert_eq!(spacing(&b, 1), want);
        // A piece of a paragraph into text: the paragraph there keeps its own.
        let r = copy(&mut a, (2, 1), (2, 4));
        let mut b = dest();
        paste_at(&mut b, &r, 0, 2);
        assert_eq!(body_para(&b, 0).text, "Hehrere");
        assert_eq!(spacing(&b, 0), (Some(0.0), Some(0.0), None));
        // The window's own copy behaves the same.
        let r = copy(&mut a, (0, 0), (1, 3));
        a.run("select.range", &json!({"anchor": {"block": 2, "off": 5}, "focus": {"block": 2, "off": 5}})).unwrap();
        a.run("edit.paste", &json!({"text": r["text"], "wordcraft": r["wordcraft"]})).unwrap();
        assert_eq!(spacing(&a, 2), want);
    }

    /// Paste Special's dialog gets the WordCraft Fragment with the formats it lists, so its
    /// Formatted Text pastes another window's copy.
    #[test]
    fn the_paste_special_dialog_keeps_the_fragment() {
        let mut a = source();
        let r = select_all_and_copy(&mut a);
        let mut b = Session::new(Document::new());
        b.run("edit.pasteSpecial", &json!({"text": r["text"], "html": r["html"], "wordcraft": r["wordcraft"]})).unwrap();
        let req = b.ui_requests.last().cloned().unwrap();
        assert_eq!(req["open"], "pasteSpecial");
        assert_eq!(req["formats"].get(0).and_then(Value::as_str), Some("formatted"));
        // OK in the dialog sends the request's clipboard back with the chosen format.
        let mut params = json!({"as": "formatted"});
        for k in ["text", "html", "rtf", "picture", "wordcraft"] {
            if let Some(v) = req.get(k) {
                params[k] = v.clone();
            }
        }
        assert!(params["wordcraft"].as_str().is_some_and(looks_native));
        let out = b.run("edit.pasteSpecial", &params).unwrap();
        assert_eq!(out["pastedAs"], "formatted");
        assert_eq!(body_para(&b, 0).props.style.as_deref(), Some("Heading1"));
    }

    /// The Clipboard pane pastes an item as a piece of a paragraph would go in, whatever the last
    /// copy was: whether an item is whole paragraphs isn't kept.
    #[test]
    fn a_clipboard_pane_item_doesnt_take_the_last_copy_s_wholeness() {
        let mut doc = Document::from_text("Three\n");
        if let Some(Block::Para(p)) = doc.body.get_mut(0).map(Arc::make_mut) {
            p.props.space_before = Some(18.0);
            p.props.space_after = Some(30.0);
        }
        if let Some(Block::Para(p)) = doc.body.get_mut(1).map(Arc::make_mut) {
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(0.0);
        }
        let mut a = Session::new(doc);
        let range = |a: &mut Session, from: usize, to: usize| {
            a.run("select.range", &json!({"anchor": {"block": 0, "off": from}, "focus": {"block": 0, "off": to}})).unwrap();
        };
        // A piece, then the whole paragraph: the piece is item 1.
        range(&mut a, 1, 4);
        a.run("edit.copy", &json!({})).unwrap();
        range(&mut a, 0, 5);
        a.run("edit.copy", &json!({})).unwrap();
        assert!(a.clipboard_whole);
        a.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 1, "off": 0}})).unwrap();
        a.run("edit.pasteClipboardItem", &json!({"index": 1})).unwrap();
        let p = body_para(&a, 1);
        assert_eq!(p.text, "hre");
        assert_eq!((p.props.space_before, p.props.space_after), (Some(0.0), Some(0.0)), "the line keeps its own");
        assert!(a.clipboard_whole, "the clipboard's own copy is still whole paragraphs");
    }
}
