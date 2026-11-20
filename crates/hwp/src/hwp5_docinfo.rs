//! New character shapes, paragraph shapes, border fills and fonts in an HWP 5.0 DocInfo stream.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
//!
//! Saving formatting to HWP goes through the HWPX header the reader rebuilds from DocInfo
//! ([`crate::hwp5`]): the write-back adds shapes to that header as XML, each one a copy of an
//! existing shape with the changed values, or (for a generated table) made from scratch. Here each
//! added shape becomes a record: a copy of the record it was copied from (else of the first of
//! its kind) with only the fields set whose XML differs, so everything the XML does not hold
//! (shadow colours, emphasis marks, border offsets ...) stays as it was. The records go after the
//! last of their kind, and ID_MAPPINGS counts them.
//!
//! Field positions are those the reader decodes ([`crate::hwp5`]): from Hancom's document, with
//! what the files showed (margins stored doubled, the underline shape table shifted by one).

use std::collections::HashMap;

use crate::header::LANGS;
use crate::hwp5::{ALIGNS, BORDER_FILL, CHAR_SHAPE, FACE_NAME, HEADINGS, ID_MAPPINGS, LINE_SPACING, LINE_TYPES, LINE_WIDTHS, PARA_SHAPE, Records};
use crate::hwp5_enc::{Rec, child, put, put_bits, u32_at};
use crate::xml::Element;

/// ID_MAPPINGS count index of each kind (Hancom's table 16).
const FONT_COUNT: usize = 1;
const BORDER_FILL_COUNT: usize = 8;
const CHAR_SHAPE_COUNT: usize = 9;
const PARA_SHAPE_COUNT: usize = 13;

/// "#RRGGBB" (or "none") as a COLORREF.
pub(crate) fn colorref(v: &str) -> Option<u32> {
    if v == "none" {
        return Some(0xFFFF_FFFF);
    }
    let h = v.strip_prefix('#')?;
    let n = u32::from_str_radix(h.get(..6)?, 16).ok()?;
    Some(((n >> 16) & 0xFF) | (n & 0xFF00) | ((n & 0xFF) << 16))
}

fn attr<'a>(e: &'a Element, name: &str) -> &'a str {
    e.get(name).unwrap_or("")
}

/// A child's attribute, or "" when the child or attribute is missing.
fn child_attr<'a>(e: &'a Element, tag: &str, name: &str) -> &'a str {
    child(e, tag).map(|c| attr(c, name)).unwrap_or("")
}

fn num(v: &str) -> i64 {
    v.trim().parse().unwrap_or(0)
}

fn index(table: &[&str], v: &str) -> Option<u32> {
    table.iter().position(|t| *t == v).map(|i| i as u32)
}

/// A character shape record `d` (the source's) with the fields where `new` differs from `old`.
pub(crate) fn patch_char(d: &mut [u8], old: &Element, new: &Element) {
    let differs = |name: &str| attr(old, name) != attr(new, name);
    if differs("height") {
        put(d, 42, &(num(attr(new, "height")) as i32).to_le_bytes());
    }
    for (name, o) in [("textColor", 52), ("shadeColor", 60)] {
        if differs(name)
            && let Some(c) = colorref(attr(new, name))
        {
            put(d, o, &c.to_le_bytes());
        }
    }
    if differs("borderFillIDRef") {
        put(d, 68, &(num(attr(new, "borderFillIDRef")) as u16).to_le_bytes());
    }
    // per language: font ids (WORD), ratio, spacing, relative size, offset (one byte each)
    for (tag, base, width) in [("hh:fontRef", 0usize, 2usize), ("hh:ratio", 14, 1), ("hh:spacing", 21, 1), ("hh:relSz", 28, 1), ("hh:offset", 35, 1)]
    {
        for (k, a) in LANGS.iter().enumerate() {
            let (o, n) = (child_attr(old, tag, a), child_attr(new, tag, a));
            if o == n || n.is_empty() {
                continue;
            }
            let v = num(n);
            if width == 2 {
                put(d, base + 2 * k, &(v as u16).to_le_bytes());
            } else {
                put(d, base + k, &[v as u8]);
            }
        }
    }
    let flag = |e: &Element, tag: &str| child(e, tag).is_some();
    for (tag, bit) in [("hh:italic", 0), ("hh:bold", 1), ("hh:emboss", 13), ("hh:engrave", 14), ("hh:supscript", 15), ("hh:subscript", 16)] {
        if flag(old, tag) != flag(new, tag) {
            put_bits(d, 46, bit, 1, u32::from(flag(new, tag)));
        }
    }
    // a line kind written explicitly as NONE is the same as none written
    let kind = |e: &Element, tag: &str, a: &str| match child_attr(e, tag, a) {
        "" | "NONE" => "NONE".to_string(),
        v => v.to_string(),
    };
    let underline = |e: &Element| (kind(e, "hh:underline", "type"), child_attr(e, "hh:underline", "shape").to_string());
    if underline(old) != underline(new) {
        let (t, shape) = underline(new);
        put_bits(
            d,
            46,
            2,
            2,
            if t == "NONE" {
                0
            } else if t == "TOP" {
                3
            } else {
                1
            },
        );
        if t != "NONE" {
            put_bits(d, 46, 4, 4, index(LINE_TYPES, &shape).unwrap_or(1).saturating_sub(1));
        }
    }
    if child_attr(old, "hh:underline", "color") != child_attr(new, "hh:underline", "color")
        && let Some(c) = colorref(child_attr(new, "hh:underline", "color"))
    {
        put(d, 56, &c.to_le_bytes());
    }
    let strike = |e: &Element| kind(e, "hh:strikeout", "shape");
    if strike(old) != strike(new) {
        let s = strike(new);
        put_bits(d, 46, 18, 3, u32::from(s != "NONE"));
        if s != "NONE" {
            put_bits(d, 46, 26, 4, index(LINE_TYPES, &s).unwrap_or(1).saturating_sub(1));
        }
    }
    if kind(old, "hh:outline", "type") != kind(new, "hh:outline", "type") {
        put_bits(d, 46, 8, 3, u32::from(kind(new, "hh:outline", "type") != "NONE"));
    }
    if kind(old, "hh:shadow", "type") != kind(new, "hh:shadow", "type") {
        put_bits(d, 46, 11, 2, u32::from(kind(new, "hh:shadow", "type") != "NONE"));
    }
}

/// A paragraph shape record `d` with the fields where `new` differs from `old`.
pub(crate) fn patch_para(d: &mut [u8], old: &Element, new: &Element) {
    let align = |e: &Element| child_attr(e, "hh:align", "horizontal").to_string();
    if align(old) != align(new) {
        put_bits(d, 0, 2, 3, index(ALIGNS, &align(new)).unwrap_or(0));
    }
    let heading = |e: &Element| ["type", "idRef", "level"].map(|a| child_attr(e, "hh:heading", a).to_string());
    if heading(old) != heading(new) {
        let [t, id, level] = heading(new);
        put_bits(d, 0, 23, 2, index(HEADINGS, &t).unwrap_or(0));
        put_bits(d, 0, 25, 3, num(&level) as u32);
        put(d, 30, &(num(&id) as u16).to_le_bytes());
    }
    for (a, bit) in [("widowOrphan", 16), ("keepWithNext", 17), ("keepLines", 18), ("pageBreakBefore", 19)] {
        let (o, n) = (child_attr(old, "hh:breakSetting", a), child_attr(new, "hh:breakSetting", a));
        if o != n && !n.is_empty() {
            put_bits(d, 0, bit, 1, u32::from(n == "1"));
        }
    }
    let word = |e: &Element| child_attr(e, "hh:breakSetting", "breakNonLatinWord") == "BREAK_WORD";
    if word(old) != word(new) {
        put_bits(d, 0, 7, 1, u32::from(word(new)));
    }
    // margins are stored doubled (see the reader)
    let margin = |e: &Element, tag: &str| child(e, "hh:margin").map(|m| child_attr(m, tag, "value").to_string()).unwrap_or_default();
    for (tag, o) in [("hc:intent", 12), ("hc:left", 4), ("hc:right", 8), ("hc:prev", 16), ("hc:next", 20)] {
        if margin(old, tag) != margin(new, tag) {
            put(d, o, &((num(&margin(new, tag)) * 2) as i32).to_le_bytes());
        }
    }
    let spacing = |e: &Element| ["type", "value"].map(|a| child_attr(e, "hh:lineSpacing", a).to_string());
    if spacing(old) != spacing(new) {
        let [t, v] = spacing(new);
        let t = index(LINE_SPACING, &t).unwrap_or(0);
        let v = num(&v) as u32;
        if d.len() >= 54 {
            put_bits(d, 46, 0, 5, t);
            put(d, 50, &v.to_le_bytes());
        } else {
            put_bits(d, 0, 0, 2, t);
            put(d, 24, &v.to_le_bytes());
        }
    }
    if attr(old, "tabPrIDRef") != attr(new, "tabPrIDRef") {
        put(d, 28, &(num(attr(new, "tabPrIDRef")) as u16).to_le_bytes());
    }
    if child_attr(old, "hh:border", "borderFillIDRef") != child_attr(new, "hh:border", "borderFillIDRef") {
        put(d, 32, &(num(child_attr(new, "hh:border", "borderFillIDRef")) as u16).to_le_bytes());
    }
}

/// A border fill record for `new` (a generated cell's: four sides and maybe a solid fill), made
/// from a record of the same fill kind when the document has one.
pub(crate) fn border_fill(templates: &[&[u8]], new: &Element) -> Vec<u8> {
    let face = child(new, "hc:fillBrush").and_then(|f| child(f, "hc:winBrush")).map(|w| attr(w, "faceColor"));
    let solid = face.and_then(colorref);
    let kind = |d: &[u8]| u32_at(d, 32) & 7;
    let template = templates.iter().find(|d| kind(d) == if solid.is_some() { 1 } else { 0 } && d.len() >= 36).or(templates.first());
    let mut d = template.map(|t| t.to_vec()).unwrap_or_else(|| vec![0u8; 40]);
    d.resize(d.len().max(36), 0);
    match solid {
        Some(c) if kind(&d) == 1 => put(&mut d, 36, &c.to_le_bytes()),
        Some(c) => {
            // no solid template: fill kind 1, colour, hatch colour, no hatch; then the extra size
            d.truncate(32);
            d.extend_from_slice(&1u32.to_le_bytes());
            d.extend_from_slice(&c.to_le_bytes());
            d.extend_from_slice(&0u32.to_le_bytes());
            d.extend_from_slice(&u32::MAX.to_le_bytes());
            d.extend_from_slice(&0u32.to_le_bytes());
            d.push(0);
        }
        None if kind(&d) != 0 => {
            d.truncate(32);
            d.extend_from_slice(&[0u8; 8]);
        }
        None => {}
    }
    put(&mut d, 0, &0u16.to_le_bytes()); // no slashes
    for (k, tag) in ["hh:leftBorder", "hh:rightBorder", "hh:topBorder", "hh:bottomBorder"].iter().enumerate() {
        let o = 2 + 6 * k;
        let t = index(LINE_TYPES, child_attr(new, tag, "type")).unwrap_or(0);
        let w = index(LINE_WIDTHS, child_attr(new, tag, "width")).unwrap_or(0);
        put(&mut d, o, &[t as u8, w as u8]);
        put(&mut d, o + 2, &colorref(child_attr(new, tag, "color")).unwrap_or(0).to_le_bytes());
    }
    d
}

/// A font name record (no substitute or type information).
pub(crate) fn face_name(name: &str) -> Vec<u8> {
    let w: Vec<u16> = name.encode_utf16().take(u16::MAX as usize).collect();
    let mut d = vec![0u8];
    d.extend_from_slice(&(w.len() as u16).to_le_bytes());
    d.extend(w.iter().flat_map(|c| c.to_le_bytes()));
    d
}

/// The `tag` elements of the list `list` in `head`, by their id.
fn by_id<'a>(head: &'a Element, list: &'a str, tag: &str) -> HashMap<i64, &'a Element> {
    head.iter(list).flat_map(|l| l.children.iter()).filter(|e| e.tag == tag).map(|e| (num(attr(e, "id")), e)).collect()
}

/// Font names per language.
fn fonts(head: &Element) -> [Vec<String>; 7] {
    let mut out: [Vec<String>; 7] = Default::default();
    for ff in head.iter("hh:fontface") {
        let Some(k) = LANGS.iter().position(|l| l.eq_ignore_ascii_case(attr(ff, "lang"))) else { continue };
        let mut list: Vec<(i64, String)> =
            ff.children.iter().filter(|f| f.tag == "hh:font").map(|f| (num(attr(f, "id")), attr(f, "face").to_string())).collect();
        list.sort();
        if let Some(o) = out.get_mut(k) {
            *o = list.into_iter().map(|(_, n)| n).collect();
        }
    }
    out
}

/// The DocInfo records `data` with the shapes `after` (the header XML after write-back) adds to
/// `before` (the reader's header); `sources` names the shape each added one copies.
pub(crate) fn add_shapes(data: &[u8], before: &Element, after: &Element, sources: &[(&'static str, i64, Option<String>)]) -> Option<Vec<Rec>> {
    let r = Records::parse(data);
    let mut recs: Vec<Rec> = (0..r.recs.len()).map(|k| (r.tag(k), r.recs.get(k).map(|x| x.level).unwrap_or(0), r.data(k).to_vec())).collect();
    let of = |tag: u32| -> Vec<usize> { (0..recs.len()).filter(|&k| recs.get(k).is_some_and(|x| x.0 == tag)).collect() };
    let mapping = of(ID_MAPPINGS).first().copied()?;
    let mut counts: Vec<i64> =
        recs.get(mapping).map(|m| m.2.as_chunks::<4>().0.iter().map(|b| i64::from(i32::from_le_bytes(*b))).collect()).unwrap_or_default();
    let source: HashMap<(&str, i64), Option<i64>> = sources.iter().map(|(el, id, src)| ((*el, *id), src.as_deref().map(num))).collect();
    // new records to put after record index `k` (in order)
    let mut after_rec: HashMap<usize, Vec<Rec>> = HashMap::new();
    let mut before_rec: HashMap<usize, Vec<Rec>> = HashMap::new();

    for (el, list, tag, count_at, first_id) in [
        ("charPr", "hh:charProperties", CHAR_SHAPE, CHAR_SHAPE_COUNT, 0i64),
        ("paraPr", "hh:paraProperties", PARA_SHAPE, PARA_SHAPE_COUNT, 0),
        ("borderFill", "hh:borderFills", BORDER_FILL, BORDER_FILL_COUNT, 1),
    ] {
        let existing = of(tag);
        let olds = by_id(before, list, &format!("hh:{el}"));
        let news = by_id(after, list, &format!("hh:{el}"));
        let mut added: Vec<(i64, &Element)> = news.iter().filter(|(id, _)| !olds.contains_key(id)).map(|(id, e)| (*id, *e)).collect();
        if added.is_empty() {
            continue;
        }
        added.sort_by_key(|(id, _)| *id);
        let (&last, &first) = (existing.last()?, existing.first()?);
        let level = recs.get(first).map(|x| x.1).unwrap_or(1);
        let mut out = Vec::new();
        for (id, new) in &added {
            let src = source.get(&(el, *id)).copied().flatten();
            let data = if el == "borderFill" && src.is_none() {
                let templates: Vec<&[u8]> = existing.iter().filter_map(|&k| recs.get(k).map(|x| x.2.as_slice())).collect();
                border_fill(&templates, new)
            } else {
                let src_id = src.unwrap_or(first_id);
                let k = *existing.get(usize::try_from(src_id - first_id).ok()?)?;
                let mut d = recs.get(k)?.2.clone();
                let old = olds.get(&src_id)?;
                match el {
                    "charPr" => patch_char(&mut d, old, new),
                    "paraPr" => patch_para(&mut d, old, new),
                    _ => {}
                }
                d
            };
            out.push((tag, level, data));
        }
        if let Some(c) = counts.get_mut(count_at) {
            *c += out.len() as i64;
        }
        after_rec.entry(last).or_default().extend(out);
    }

    // fonts: each language's go after the last font of that language
    let (old_fonts, new_fonts) = (fonts(before), fonts(after));
    let faces = of(FACE_NAME);
    let mut pos = 0usize;
    for k in 0..LANGS.len() {
        let n = counts.get(FONT_COUNT + k).copied().unwrap_or(0).max(0) as usize;
        let have = old_fonts.get(k).map(Vec::len).unwrap_or(0);
        let names: Vec<String> = new_fonts.get(k).map(|l| l.iter().skip(have).cloned().collect()).unwrap_or_default();
        pos += n;
        if names.is_empty() {
            continue;
        }
        let level = faces.first().and_then(|&f| recs.get(f)).map(|x| x.1).unwrap_or(1);
        let new: Vec<Rec> = names.iter().map(|nm| (FACE_NAME, level, face_name(nm))).collect();
        match (pos.checked_sub(1).and_then(|p| faces.get(p)), faces.first()) {
            (Some(&prev), _) => after_rec.entry(prev).or_default().extend(new),
            (None, Some(&first)) => before_rec.entry(first).or_default().extend(new),
            (None, None) => return None,
        }
        if let Some(c) = counts.get_mut(FONT_COUNT + k) {
            *c += names.len() as i64;
        }
    }
    if after_rec.is_empty() && before_rec.is_empty() {
        return None;
    }
    if let Some(m) = recs.get_mut(mapping) {
        m.2 = counts.iter().flat_map(|c| (*c as i32).to_le_bytes()).collect();
    }
    let mut out = Vec::with_capacity(recs.len());
    for (k, rec) in recs.into_iter().enumerate() {
        out.extend(before_rec.remove(&k).unwrap_or_default());
        out.push(rec);
        out.extend(after_rec.remove(&k).unwrap_or_default());
    }
    Some(out)
}
