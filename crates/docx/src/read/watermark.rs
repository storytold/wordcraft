//! Word's picture watermark (Design › Watermark › Picture watermark): a VML picture with Word's
//! `WordPictureWatermark` id in the headers. It becomes the document's watermark only when every
//! page shows the same one, placed as Word places it; otherwise each stays a picture in its header,
//! so a cover page without it, a section with another picture, or one moved by hand keep theirs.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use wordcraft_doc::picfx::PicFx;
use wordcraft_doc::watermark::{MAX_SCALE, MIN_SCALE, own_size};
use wordcraft_doc::{Watermark, WatermarkPicture};

use super::Reader;
use super::props::sectpr;
use super::story::{StoryCtx, children_of_choice, vml_style, vml_watermark};
use crate::package::Rels;
use crate::units::{measure, vml_number};
use crate::xml::{El, MAX_DEPTH};

/// Word's watermark picture shape id starts with this.
const WORD_ID: &str = "WordPictureWatermark";

/// Most sections, and header or footer parts, looked at; past them the pictures stay put.
const MAX_SECTIONS: usize = 10_000;
const MAX_PARTS: usize = 64;

/// The document's picture watermark as the plan found it: the header parts whose Word picture
/// watermark it is (empty: none), and the scale and Washout every one of them agrees on.
#[derive(Default)]
pub(super) struct Plan {
    headers: HashSet<String>,
    scale: Option<f32>,
    washout: bool,
}

/// A Word picture watermark shape as Word places it: absolutely, centred on the margins both
/// ways, behind the text, shown, not turned or flipped, its picture carrying Word's Washout or
/// nothing.
struct Shape<'a> {
    rid: &'a str,
    size: Option<(f32, f32)>,
    washout: bool,
}

/// `e` as Word's picture watermark shape, if it is one placed as Word places it.
fn shape(e: &El) -> Option<Shape<'_>> {
    if e.name != "v:shape" || !e.attr("id").is_some_and(|id| id.starts_with(WORD_ID)) {
        return None;
    }
    let style = e.attr("style").unwrap_or("");
    let is = |k: &str, v: &str| vml_style(style, k).is_some_and(|x| x.eq_ignore_ascii_case(v));
    let placed = is("position", "absolute")
        && is("mso-position-horizontal", "center")
        && is("mso-position-horizontal-relative", "margin")
        && is("mso-position-vertical", "center")
        && is("mso-position-vertical-relative", "margin");
    let behind = vml_style(style, "z-index").and_then(|z| z.trim().parse::<i64>().ok()).is_some_and(|z| z < 0);
    let spin = super::story::vml_spin(style);
    if !placed || !behind || is("visibility", "hidden") || spin.deg != 0.0 || spin.flip_h || spin.flip_v {
        return None;
    }
    let data = e.child("v:imagedata")?;
    if data.attrs.iter().any(|(k, _)| !matches!(k.as_str(), "r:id" | "r:pict" | "src" | "o:title" | "gain" | "blacklevel")) {
        return None;
    }
    // Word's Washout is gain 0.3 and black level 0.35; other values stay on a header picture.
    let num = |k: &str| data.attr(k).and_then(vml_number);
    let fx = PicFx::from_vml(num("gain").unwrap_or(1.0), num("blacklevel").unwrap_or(0.0));
    let washout = if data.attr("gain").is_none() && data.attr("blacklevel").is_none() {
        false
    } else if WatermarkPicture::is_washout(fx) {
        true
    } else {
        return None;
    };
    let len = |k: &str| vml_style(style, k).and_then(|v| measure(v, 1.0)).filter(|v| v.is_finite() && *v > 0.0);
    let size = len("width").zip(len("height"));
    Some(Shape { rid: data.attr("r:id").or_else(|| data.attr("r:pict"))?, size, washout })
}

/// One header's Word picture watermark: its picture, its width as a percentage of the picture's
/// own size (pixels at 96 ppi), whether it fills its section's margins (Auto), and Washout.
struct Seen {
    bytes: Arc<Vec<u8>>,
    percent: f32,
    fits: bool,
    washout: bool,
}

/// What shape `s` shows, its picture `bytes`, in a header whose first section has room `fit`.
/// None when the picture can't be read, or the shape has no size or is stretched out of proportion.
fn seen(bytes: &Arc<Vec<u8>>, s: &Shape, fit: (f32, f32)) -> Option<Seen> {
    let (own_w, own_h) = own_size(bytes)?;
    let (w, h) = s.size?;
    if (h - w * own_h / own_w).abs() > 1.0 {
        return None;
    }
    let (auto_w, _) = WatermarkPicture::default().size(own_w, own_h, fit)?;
    Some(Seen { bytes: bytes.clone(), percent: w / own_w * 100.0, fits: (auto_w - w).abs() <= 1.0, washout: s.washout })
}

impl Reader<'_> {
    /// The document's watermark from a header's Word picture watermark shape `e`, when the plan
    /// made that header's one the document's watermark.
    pub(super) fn picture_watermark(&mut self, sc: &StoryCtx, e: &El, rels: &Rels) -> Option<Watermark> {
        if !sc.picture_wm {
            return None;
        }
        let s = shape(e)?;
        let media = self.media_for(rels, s.rid)?;
        let pic = WatermarkPicture { media, scale: self.picture_wm.scale, washout: self.picture_wm.washout };
        Some(Watermark { picture: Some(pic), ..Default::default() })
    }

    /// Whether header part `path`'s Word picture watermark is the document's (see [`plan`]).
    pub(super) fn picture_wm_header(&self, path: &str) -> bool {
        self.picture_wm.headers.contains(path)
    }
}

/// The document's picture watermark, worked out from the section properties and header and
/// footer parts before the body is read. Every page shows a header (default; first page with a
/// title page; even pages with different odd and even pages, each linked to the section before
/// when it has none), each header holds at most one Word picture watermark placed as Word places
/// it and the shown ones hold one, all have the same picture and Washout, and all fill their
/// section's margins (Auto) or all have the same scale. No header or footer has a text
/// watermark, and no footer a Word picture watermark. Otherwise the plan is empty.
pub(super) fn plan(r: &Reader, body: &El, rels: &Rels) -> Plan {
    let mut sects = Vec::new();
    sect_prs(body, 0, true, &mut sects);
    if let Some(s) = body.child("w:sectPr") {
        sects.push((s, true));
    }
    if sects.len() > MAX_SECTIONS {
        return Plan::default();
    }
    let even = r.doc.settings.even_odd_headers;
    let mut linked: [Option<String>; 3] = Default::default();
    let mut shown: Vec<Option<String>> = Vec::new();
    // Each header part, with the room Auto fills in the first section naming it
    // (`Document::watermark_fit`).
    let mut headers: HashMap<String, (f32, f32)> = HashMap::new();
    let mut footers: HashSet<String> = HashSet::new();
    for (s, top) in &sects {
        let (sp, refs) = sectpr(&r.pc, s);
        for h in &refs {
            let Some(rel) = rels.by_id(&h.rid).filter(|x| !x.external) else { continue };
            if h.footer {
                footers.insert(rel.target.clone());
                continue;
            }
            headers.entry(rel.target.clone()).or_insert_with(|| (sp.text_width(), sp.text_height()));
            if *top {
                let slot = match h.kind.as_str() {
                    "first" => 1,
                    "even" => 2,
                    _ => 0,
                };
                if let Some(l) = linked.get_mut(slot) {
                    *l = Some(rel.target.clone());
                }
            }
        }
        if !*top {
            continue;
        }
        let [default, first, even_page] = &linked;
        shown.push(default.clone());
        if sp.title_page {
            shown.push(first.clone());
        }
        if even {
            shown.push(even_page.clone());
        }
    }
    let has_word_id = |path: &str| r.pkg.get(path).is_some_and(|b| b.windows(WORD_ID.len()).any(|w| w == WORD_ID.as_bytes()));
    if headers.len() > MAX_PARTS
        || footers.len() > MAX_PARTS
        || shown.is_empty()
        || shown.iter().any(Option::is_none)
        || !headers.keys().any(|p| has_word_id(p))
    {
        return Plan::default();
    }
    // A footer's text watermark is the document's watermark, and a footer's picture watermark
    // stays where it is: either way the headers' pictures stay too.
    for path in &footers {
        let Ok(Some(root)) = r.pkg.xml(path) else { continue };
        let mut shapes = Vec::new();
        if !word_shapes(&root, 0, "", false, false, &mut shapes) || !shapes.is_empty() {
            return Plan::default();
        }
    }
    let mut found: HashMap<&str, Option<Seen>> = HashMap::new();
    for (path, fit) in &headers {
        let Ok(Some(root)) = r.pkg.xml(path) else { return Plan::default() };
        let mut shapes = Vec::new();
        if !word_shapes(&root, 0, "", false, false, &mut shapes) || shapes.len() > 1 {
            return Plan::default();
        }
        let wm = match shapes.first() {
            None => None,
            Some(e) => {
                let hrels = r.pkg.rels(path);
                let s = shape(e);
                let bytes = s.as_ref().and_then(|s| hrels.by_id(s.rid)).filter(|x| !x.external).and_then(|x| r.pkg.get_shared(&x.target));
                let Some(w) = s.zip(bytes).and_then(|(s, b)| seen(b, &s, *fit)) else { return Plan::default() };
                Some(w)
            }
        };
        found.insert(path.as_str(), wm);
    }
    if shown.iter().flatten().any(|p| found.get(p.as_str()).is_none_or(Option::is_none)) {
        return Plan::default();
    }
    let all: Vec<&Seen> = found.values().flatten().collect();
    // The first shown header's, whatever order the map holds them in.
    let Some(first) = shown.iter().flatten().find_map(|p| found.get(p.as_str())?.as_ref()) else { return Plan::default() };
    if !all.iter().all(|s| s.bytes == first.bytes && s.washout == first.washout) {
        return Plan::default();
    }
    let scale = if all.iter().all(|s| s.fits) {
        None
    } else {
        let percent = (first.percent * 10.0).round() / 10.0;
        if !(MIN_SCALE..=MAX_SCALE).contains(&percent) || all.iter().any(|s| (s.percent - first.percent).abs() > 0.1) {
            return Plan::default();
        }
        Some(percent)
    };
    let washout = first.washout;
    Plan { headers: found.into_iter().filter(|(_, w)| w.is_some()).map(|(p, _)| p.to_string()).collect(), scale, washout }
}

/// The `w:sectPr` elements the reader reads, in order, as (element, whether it ends a section of
/// the document): in paragraph properties through the containers `read_blocks` follows, and in
/// table cells (which load headers but make no section).
fn sect_prs<'a>(e: &'a El, depth: usize, top: bool, out: &mut Vec<(&'a El, bool)>) {
    if depth > MAX_DEPTH || out.len() > MAX_SECTIONS {
        return;
    }
    for k in e.els() {
        match k.name.as_str() {
            "w:p" => {
                if let Some(s) = k.child("w:pPr").and_then(|p| p.child("w:sectPr")) {
                    out.push((s, top));
                }
            }
            "w:sdt" => {
                if let Some(c) = k.child("w:sdtContent") {
                    sect_prs(c, depth + 1, top, out);
                }
            }
            "w:customXml" | "w:smartTag" | "w:ins" | "w:moveTo" => sect_prs(k, depth + 1, top, out),
            "mc:AlternateContent" => {
                if let Some(c) = children_of_choice(k) {
                    sect_prs(c, depth + 1, top, out);
                }
            }
            "w:tbl" | "w:tr" | "w:tc" => sect_prs(k, depth + 1, false, out),
            _ => {}
        }
    }
}

/// Collects the Word picture watermark shapes a header or footer would read as such (straight
/// in a `w:pict` or `w:object`, not in a tracked deletion) into `out`. False when the part has a
/// text watermark, or a Word picture watermark anywhere else (a group, a text box, alternate
/// content, a deleted run).
fn word_shapes<'a>(e: &'a El, depth: usize, parent: &str, nested: bool, deleted: bool, out: &mut Vec<&'a El>) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    for k in e.els() {
        if k.name == "v:shape" && vml_watermark(k).is_some() {
            return false;
        }
        if k.name == "v:shape" && k.attr("id").is_some_and(|id| id.starts_with(WORD_ID)) {
            if nested || deleted || !matches!(parent, "w:pict" | "w:object") {
                return false;
            }
            out.push(k);
            continue;
        }
        let inner = nested || matches!(k.name.as_str(), "w:txbxContent" | "mc:AlternateContent" | "v:group");
        let gone = deleted || matches!(k.name.as_str(), "w:del" | "w:moveFrom");
        if !word_shapes(k, depth + 1, &k.name, inner, gone, out) {
            return false;
        }
    }
    true
}
