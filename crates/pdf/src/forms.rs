//! Fillable form fields (AcroForm) for [`crate::PdfOptions::form_fields`] and
//! [`crate::PdfOptions::blank_fields`].
//!
//! krilla writes no interactive forms, so they are added to the finished PDF: the document's
//! fields are found where the same layout put them on the page, and each becomes a widget with
//! its own appearance.
//!
//! - **Form fields:** legacy form fields (`FORMCHECKBOX`, `FORMTEXT`, `FORMDROPDOWN`) and content
//!   controls that may be edited (plain and rich text, date, check box, drop-down, combo box),
//!   named after the field (`w:name`, tag or title), with the document's value. Their widgets are
//!   opaque: they cover the field drawn below, so a printed or flattened copy shows the value
//!   filled in, not the one under it.
//! - **Blanks:** runs of underscores (`Ref. No. ________`) and date patterns (`__/__/20__`) as text
//!   fields named after the words before them. Their widgets are transparent, so the page looks
//!   as before (underscores, highlight); a date blank turns white once filled in, so the printed
//!   pattern does not show through the date.
//!
//! The PDF is read and written again with `lopdf`; the page content, fonts and structure tree are
//! kept as they are.

use lopdf::{Object, ObjectId, Stream, StringFormat, dictionary};
use wordcraft_doc::control::ControlKind;
use wordcraft_doc::legacy_form::LegacyForm;
use wordcraft_doc::para::{InlineObject, OBJ, ShapeKind};
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_layout::DocLayout;

use crate::PdfError;
use wordcraft_layout::display::{self, CHECK_BOX_INSET, DisplayOptions, Draw};

/// Most fields added to one document (hostile input).
const MAX_FIELDS: usize = 5000;

/// A field's value and kind.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FieldKind {
    CheckBox { checked: bool },
    Text { value: String, max_len: Option<u32> },
    Choice { options: Vec<String>, selected: Option<usize>, editable: bool },
}

/// A fillable field where the layout put it: `rect` is (x, y, w, h) in points from the page's
/// top-left corner, on 0-based layout page `page`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FormField {
    name: String,
    pub(crate) page: usize,
    rect: [f32; 4],
    kind: FieldKind,
    /// An opaque white background covering what is drawn below (a field's static value); off
    /// for blanks, whose underscores stay visible under the text typed in.
    opaque: bool,
    /// A white background once filled in (the viewer redraws the field with it), though the empty
    /// field shows what is below: a date blank's printed `__/__/20__` is a hint until the date typed
    /// in replaces it.
    cover_when_filled: bool,
}

/// The fillable fields of `doc`'s body as laid out in `lay`.
pub(crate) fn collect(doc: &Document, lay: &DocLayout) -> Vec<FormField> {
    let mut out = Vec::new();
    let mut squares: Vec<Option<Vec<[f32; 4]>>> = vec![None; lay.pages.len()];
    let story = StoryRef::Body;
    for path in doc.para_paths(story) {
        let Some(p) = doc.para(story, &path) else {
            continue;
        };
        for off in p.object_offsets() {
            if out.len() >= MAX_FIELDS {
                return out;
            }
            let Some(obj) = p.object_at(off) else {
                continue;
            };
            let Some(form) = obj.legacy_form() else {
                continue;
            };
            let start = Pos::new(story, path.clone(), off);
            let end = Pos::new(story, path.clone(), off + OBJ.len_utf8());
            let Some(band) = span(lay, &start, &end) else {
                continue;
            };
            let (page, rect) = band;
            let (kind, rect) = match &form {
                LegacyForm::CheckBox { checked, .. } => {
                    // The square the layout drew, for its exact size; else the room inset.
                    let drawn = page_squares(doc, lay, page, &mut squares);
                    let want_x = rect[0] + CHECK_BOX_INSET;
                    // The square on this field's line: same left edge, centre nearest the line's.
                    let mid = rect[1] + rect[3] / 2.0;
                    let sq = drawn
                        .iter()
                        .filter(|s| (s[0] - want_x).abs() < 0.6 && (s[1] + s[3] / 2.0 - mid).abs() < rect[3].max(s[3]))
                        .min_by(|a, b| (a[1] + a[3] / 2.0 - mid).abs().total_cmp(&(b[1] + b[3] / 2.0 - mid).abs()))
                        .copied()
                        .unwrap_or([
                            want_x,
                            rect[1] + CHECK_BOX_INSET,
                            (rect[2] - 2.0 * CHECK_BOX_INSET).max(4.0),
                            (rect[2] - 2.0 * CHECK_BOX_INSET).max(4.0),
                        ]);
                    (FieldKind::CheckBox { checked: *checked }, sq)
                }
                LegacyForm::Text { max_len, .. } => {
                    let value = match obj {
                        InlineObject::Field { result, .. } => clean(result),
                        _ => String::new(),
                    };
                    (FieldKind::Text { value, max_len: *max_len }, rect)
                }
                LegacyForm::DropDown { entries, selected, .. } => {
                    (FieldKind::Choice { options: entries.clone(), selected: (!entries.is_empty()).then_some(*selected), editable: false }, rect)
                }
            };
            let base = if form.name().trim().is_empty() { default_name(&kind) } else { form.name().trim().to_string() };
            out.push(FormField { name: base, page, rect, kind, opaque: true, cover_when_filled: false });
        }
    }
    // Content controls the user may edit.
    for r in doc.control_ranges(story) {
        if out.len() >= MAX_FIELDS {
            break;
        }
        if !r.control.lock.can_edit() || r.start.path != r.end.path {
            continue;
        }
        let Some(p) = doc.para(story, &r.start.path) else {
            continue;
        };
        let from = Pos::new(story, r.start.path.clone(), r.start.off + OBJ.len_utf8());
        let Some((page, rect)) = span(lay, &from, &r.end) else {
            continue;
        };
        let text = p.text.get(from.off..r.end.off).map(clean).unwrap_or_default();
        let text = if r.control.showing_placeholder { String::new() } else { text };
        let kind = match &r.control.kind {
            ControlKind::CheckBox { checked, .. } => FieldKind::CheckBox { checked: *checked },
            ControlKind::DropDown { items, .. } | ControlKind::ComboBox { items, .. } => {
                let options: Vec<String> = items.iter().map(|i| i.display.clone()).collect();
                let selected = options.iter().position(|o| *o == text);
                FieldKind::Choice { options, selected, editable: matches!(r.control.kind, ControlKind::ComboBox { .. }) }
            }
            ControlKind::Text { .. } | ControlKind::RichText | ControlKind::Date { .. } => FieldKind::Text { value: text, max_len: None },
            _ => continue,
        };
        // A check box control's square is its symbol's box.
        let rect = match kind {
            FieldKind::CheckBox { .. } => {
                let side = rect[3].min(rect[2].max(4.0));
                [rect[0], rect[1] + (rect[3] - side) / 2.0, side, side]
            }
            _ => rect,
        };
        let name = [r.control.tag.trim(), r.control.title.trim()]
            .into_iter()
            .find(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| default_name(&kind));
        out.push(FormField { name, page, rect, kind, opaque: true, cover_when_filled: false });
    }
    unique_names(&mut out);
    out
}

/// Width of the border the layout strokes around a check box's square.
const CHECK_BOX_STROKE: f32 = 0.72;

/// White margin around a check box widget, covering the drawn square should a viewer round its
/// position differently.
const CHECK_BOX_PAD: f32 = 1.0 + CHECK_BOX_STROKE / 2.0;

/// Shortest run of underscores taken for a blank to fill in.
const MIN_BLANK: usize = 4;

/// Blanks drawn with underscores (`Ref. No. __________`, `__/__/20__`) as fillable text fields,
/// one per line the blank covers, named after the words before it. A date pattern of
/// underscore groups joined by slashes (`__/__/20__`, `__/__/__`, `__/__`) is one field;
/// otherwise a blank is a run of at least [`MIN_BLANK`] underscores.
pub(crate) fn collect_blanks(doc: &Document, lay: &DocLayout) -> Vec<FormField> {
    let mut out = Vec::new();
    let story = StoryRef::Body;
    for path in doc.para_paths(story) {
        let Some(p) = doc.para(story, &path) else {
            continue;
        };
        for (start, end) in blank_runs(&p.text) {
            if out.len() >= MAX_FIELDS {
                return out;
            }
            let name = name_before(p.text.get(..start).unwrap_or(""));
            let is_date = p.text.get(start..end).is_some_and(|t| t.contains('/'));
            let a = Pos::new(story, path.clone(), start);
            let b = Pos::new(story, path.clone(), end);
            for (page, r) in lay.selection_rects(doc, &a, &b, 0) {
                if r.w < 4.0 || r.h < 4.0 {
                    continue;
                }
                out.push(FormField {
                    name: name.clone(),
                    page,
                    rect: [r.x, r.y, r.w, r.h],
                    kind: FieldKind::Text { value: String::new(), max_len: None },
                    opaque: false,
                    cover_when_filled: is_date,
                });
            }
        }
    }
    out
}

/// Byte ranges of the blanks in `text`: date patterns, and long underscore runs.
fn blank_runs(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let underscores = |from: usize| b.get(from..).map_or(0, |r| r.iter().take_while(|c| **c == b'_').count());
    let spaces = |mut j: usize| {
        while b.get(j) == Some(&b' ') {
            j += 1;
        }
        j
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b.get(i) != Some(&b'_') {
            i += 1;
            continue;
        }
        let first = underscores(i);
        // Groups of underscores joined by `/` (digits such as `20` may open a group).
        let mut end = i + first;
        let mut groups = 1;
        loop {
            let j = spaces(end);
            if b.get(j) != Some(&b'/') {
                break;
            }
            let j = spaces(j + 1);
            let digits = b.get(j..).map_or(0, |r| r.iter().take_while(|c| c.is_ascii_digit()).count()).min(4);
            let n = underscores(j + digits);
            if n < 2 {
                break;
            }
            end = j + digits + n;
            groups += 1;
        }
        if ((groups >= 2 && first >= 2) || first >= MIN_BLANK) && !filled_in(b, i, end) {
            out.push((i, end));
        }
        i = end.max(i + 1);
    }
    out
}

/// How far [`filled_in`] looks for the underscores on the other side of a value.
const FILLED_REACH: usize = 40;

/// Whether the underscores at `start..end` are the leftover of a blank already filled in by hand,
/// `__JOHN SMITH______`: a value touching them that touches more underscores on its other side.
/// The search stops after [`FILLED_REACH`] bytes and at a `:`, a tab or two spaces, which part
/// labels and blanks (`Ref. No. ____    Code ____` is two blanks).
fn filled_in(b: &[u8], start: usize, end: usize) -> bool {
    let touches = |c: Option<&u8>| c.is_some_and(|c| !matches!(c, b' ' | b'\t' | b'_'));
    let reaches = |bytes: &mut dyn Iterator<Item = &u8>| {
        let mut prev = 0u8;
        for &c in bytes.take(FILLED_REACH) {
            match c {
                // The value must touch these underscores too (`20__. alle ore __/__` is two blanks).
                b'_' => return prev != b' ',
                b':' | b'\t' => return false,
                b' ' if prev == b' ' => return false,
                _ => {}
            }
            prev = c;
        }
        false
    };
    let before = start.checked_sub(1).and_then(|j| b.get(j));
    (touches(before) && reaches(&mut b.get(..start).unwrap_or(&[]).iter().rev()))
        || (touches(b.get(end)) && reaches(&mut b.get(end..).unwrap_or(&[]).iter()))
}

/// A field name from the end of `before`: its last word or two (`Ref. No.` → `Ref_No`), or
/// `Field` when there are none.
fn name_before(before: &str) -> String {
    // Only the words since the previous blank on the line.
    let before = before.rsplit('_').next().unwrap_or(before);
    // Articles, prepositions and one-letter elisions (`on the`, `il giorno`, `d'`) add nothing.
    const SMALL: [&str; 30] = [
        "the", "a", "an", "of", "on", "at", "to", "by", "for", "in", "il", "lo", "la", "i", "gli", "le", "un", "una", "di", "del", "della", "dei",
        "al", "alla", "alle", "ai", "nel", "per", "con", "da",
    ];
    let words: Vec<&str> = before
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && w.chars().any(char::is_alphabetic))
        .filter(|w| !SMALL.contains(&w.to_lowercase().as_str()))
        .filter(|w| w.chars().count() > 1 || w.chars().all(char::is_uppercase))
        .collect();
    let tail = words.get(words.len().saturating_sub(2)..).unwrap_or(&[]);
    // Two words make a label when the first is capitalised (`Ref. No.`, `Tax code`); in running
    // text (`no later than the day`) the last word names the blank.
    let tail = match tail {
        [a, b] if !a.chars().next().is_some_and(char::is_uppercase) => vec![*b],
        t => t.to_vec(),
    };
    let name = tail.join("_");
    if name.is_empty() { "Field".to_string() } else { name.chars().take(40).collect() }
}

/// The band from `a` to `b` on one line: (page, [x, top, width, height]); `None` when they are
/// on different pages or lines, or not laid out.
fn span(lay: &DocLayout, a: &Pos, b: &Pos) -> Option<(usize, [f32; 4])> {
    let ca = lay.caret(a)?;
    let cb = lay.caret(b)?;
    if ca.page != cb.page || (ca.top - cb.top).abs() > 2.0 {
        // Wrapped onto another line: the first line's part, to the line's end is unknown — use a
        // minimum width from the start.
        return Some((ca.page, [ca.x, ca.top, 36.0, ca.height.max(8.0)]));
    }
    let (x0, x1) = (ca.x.min(cb.x), ca.x.max(cb.x));
    let top = ca.top.min(cb.top);
    let h = ca.height.max(cb.height).max(8.0);
    Some((ca.page, [x0, top, (x1 - x0).max(12.0), h]))
}

/// The check box squares the layout draws on `page` (cached).
fn page_squares<'a>(doc: &Document, lay: &DocLayout, page: usize, cache: &'a mut [Option<Vec<[f32; 4]>>]) -> &'a [[f32; 4]] {
    let Some(slot) = cache.get_mut(page) else {
        return &[];
    };
    if slot.is_none() {
        let draws = lay.pages.get(page).map(|pg| display::page_display(doc, pg, &DisplayOptions::default())).unwrap_or_default();
        let mut found = Vec::new();
        collect_squares(&draws, &mut found);
        *slot = Some(found);
    }
    slot.as_deref().unwrap_or(&[])
}

fn collect_squares(draws: &[Draw], out: &mut Vec<[f32; 4]>) {
    for d in draws {
        match d {
            Draw::Shape { rect, kind: ShapeKind::Rectangle, fill: None, stroke: Some(_), stroke_width, .. }
                if (stroke_width - 0.72).abs() < 0.01 && (rect.w - rect.h).abs() < 0.05 =>
            {
                out.push([rect.x, rect.y, rect.w, rect.h])
            }
            Draw::Clip { items, .. } | Draw::Turned { items, .. } | Draw::Rotated { items, .. } => collect_squares(items, out),
            _ => {}
        }
    }
}

/// Text without object markers and with line breaks as spaces.
fn clean(s: &str) -> String {
    s.chars().filter(|c| *c != OBJ).map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim_end().to_string()
}

fn default_name(kind: &FieldKind) -> String {
    match kind {
        FieldKind::CheckBox { .. } => "Check".into(),
        FieldKind::Text { .. } => "Text".into(),
        FieldKind::Choice { .. } => "Choice".into(),
    }
}

/// Field names must be unique (one field per name): later duplicates get `_2`, `_3`….
pub(crate) fn unique_names(fields: &mut [FormField]) {
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for f in fields.iter_mut() {
        // PDF partial names may not contain periods.
        let base = f.name.replace('.', "_");
        let n = seen.entry(base.clone()).or_insert(0);
        *n += 1;
        f.name = if *n == 1 { base } else { format!("{base}_{n}") };
    }
}

/// `pdf` with an AcroForm holding `fields` (none: unchanged).
pub(crate) fn add_acroform(pdf: Vec<u8>, fields: &[FormField]) -> Result<Vec<u8>, PdfError> {
    if fields.is_empty() {
        return Ok(pdf);
    }
    let mut doc = lopdf::Document::load_mem(&pdf).map_err(|e| PdfError::Write(format!("form fields: reading the PDF: {e}")))?;
    let pages = doc.get_pages();
    let helv = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    let mut refs = Vec::new();
    for f in fields {
        let Some(&page_id) = u32::try_from(f.page + 1).ok().and_then(|n| pages.get(&n)) else {
            continue;
        };
        let page_h = page_height(&doc, page_id).unwrap_or(842.0);
        // A check box's rect is its square; the widget adds a white margin around it.
        let pad = if matches!(f.kind, FieldKind::CheckBox { .. }) { CHECK_BOX_PAD } else { 0.0 };
        let [x, y, w, h] = [f.rect[0] - pad, f.rect[1] - pad, f.rect[2] + 2.0 * pad, f.rect[3] + 2.0 * pad];
        let rect = vec![Object::Real(x), Object::Real(page_h - y - h), Object::Real(x + w), Object::Real(page_h - y)];
        let mut d = dictionary! {
            "Type" => "Annot",
            "Subtype" => "Widget",
            "T" => Object::String(pdf_text(&f.name), StringFormat::Literal),
            "Rect" => rect,
            "F" => 4,
            "P" => page_id,
        };
        match &f.kind {
            FieldKind::CheckBox { checked } => {
                let on = appearance(&mut doc, w, h, &check_box_stream(w, h, true), None);
                let off = appearance(&mut doc, w, h, &check_box_stream(w, h, false), None);
                let state = if *checked { "Yes" } else { "Off" };
                d.set("FT", "Btn");
                d.set("V", Object::Name(state.into()));
                d.set("AS", Object::Name(state.into()));
                d.set("AP", dictionary! { "N" => dictionary! { "Yes" => on, "Off" => off } });
                d.set("MK", dictionary! { "BG" => vec![1.into(), 1.into(), 1.into()] });
            }
            FieldKind::Text { value, max_len } => {
                let size = fit_size(w, h, value);
                d.set("FT", "Tx");
                d.set("V", Object::String(pdf_text(value), StringFormat::Literal));
                // Auto size while filling in; a white background when the viewer redraws it.
                d.set("DA", Object::String(b"/Helv 0 Tf 0 g".to_vec(), StringFormat::Literal));
                if f.opaque || f.cover_when_filled {
                    d.set("MK", dictionary! { "BG" => vec![1.into(), 1.into(), 1.into()] });
                }
                if let Some(n) = max_len {
                    d.set("MaxLen", i64::from(*n));
                }
                let ap = appearance(&mut doc, w, h, &text_stream(w, h, size, value, f.opaque), Some(helv));
                d.set("AP", dictionary! { "N" => ap });
            }
            FieldKind::Choice { options, selected, editable } => {
                let shown = selected.and_then(|i| options.get(i)).cloned().unwrap_or_default();
                let size = fit_size(w, h, &shown);
                d.set("FT", "Ch");
                // Combo box (bit 18), editable (bit 19).
                d.set("Ff", if *editable { (1i64 << 17) | (1 << 18) } else { 1i64 << 17 });
                d.set("Opt", options.iter().map(|o| Object::String(pdf_text(o), StringFormat::Literal)).collect::<Vec<_>>());
                d.set("V", Object::String(pdf_text(&shown), StringFormat::Literal));
                d.set("DA", Object::String(b"/Helv 0 Tf 0 g".to_vec(), StringFormat::Literal));
                d.set("MK", dictionary! { "BG" => vec![1.into(), 1.into(), 1.into()] });
                let ap = appearance(&mut doc, w, h, &text_stream(w, h, size, &shown, f.opaque), Some(helv));
                d.set("AP", dictionary! { "N" => ap });
            }
        }
        let id = doc.add_object(d);
        add_annot(&mut doc, page_id, id).map_err(|e| PdfError::Write(format!("form fields: {e}")))?;
        refs.push(Object::Reference(id));
    }
    let root = doc.trailer.get(b"Root").and_then(Object::as_reference).map_err(|e| PdfError::Write(format!("form fields: no catalog: {e}")))?;
    let form = dictionary! {
        "Fields" => refs,
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
        "DA" => Object::String(b"/Helv 0 Tf 0 g".to_vec(), StringFormat::Literal),
    };
    doc.get_object_mut(root)
        .and_then(Object::as_dict_mut)
        .map_err(|e| PdfError::Write(format!("form fields: bad catalog: {e}")))?
        .set("AcroForm", form);
    let mut out = Vec::with_capacity(pdf.len() + 4096);
    doc.save_to(&mut out).map_err(|e| PdfError::Write(format!("form fields: writing the PDF: {e}")))?;
    Ok(out)
}

fn page_height(doc: &lopdf::Document, page: ObjectId) -> Option<f32> {
    let dict = doc.get_dictionary(page).ok()?;
    let mb = dict.get(b"MediaBox").ok()?;
    let arr = match mb {
        Object::Reference(r) => doc.get_object(*r).ok()?.as_array().ok()?,
        o => o.as_array().ok()?,
    };
    let num = |o: &Object| o.as_float().ok().or_else(|| o.as_i64().ok().map(|v| v as f32));
    Some(num(arr.get(3)?)? - num(arr.get(1)?)?)
}

fn add_annot(doc: &mut lopdf::Document, page: ObjectId, annot: ObjectId) -> Result<(), String> {
    // `/Annots` may be an inline array or a reference to one.
    let existing = doc.get_dictionary(page).map_err(|e| e.to_string())?.get(b"Annots").ok().cloned();
    match existing {
        Some(Object::Reference(r)) => {
            doc.get_object_mut(r).and_then(Object::as_array_mut).map_err(|e| e.to_string())?.push(Object::Reference(annot));
        }
        Some(Object::Array(mut a)) => {
            a.push(Object::Reference(annot));
            doc.get_dictionary_mut(page).map_err(|e| e.to_string())?.set("Annots", a);
        }
        _ => {
            doc.get_dictionary_mut(page).map_err(|e| e.to_string())?.set("Annots", vec![Object::Reference(annot)]);
        }
    }
    // Tab order follows the structure (PDF/UA asks for it on pages with annotations).
    doc.get_dictionary_mut(page).map_err(|e| e.to_string())?.set("Tabs", "S");
    Ok(())
}

/// A form XObject `w` × `h` drawing `content`, with Helvetica as `/Helv` when given.
fn appearance(doc: &mut lopdf::Document, w: f32, h: f32, content: &str, font: Option<ObjectId>) -> ObjectId {
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => vec![Object::Real(0.0), Object::Real(0.0), Object::Real(w), Object::Real(h)],
    };
    if let Some(f) = font {
        dict.set("Resources", dictionary! { "Font" => dictionary! { "Helv" => f } });
    }
    doc.add_object(Stream::new(dict, content.as_bytes().to_vec()))
}

/// Word's check box look: white square, 0.72 pt border, an X in 0.48 pt lines when checked.
fn check_box_stream(w: f32, h: f32, checked: bool) -> String {
    // The box is the square grown by the pad: border and X on the square, as drawn below.
    let i = CHECK_BOX_PAD;
    let (r, t) = (w - i, h - i);
    let mut s = format!("q 1 g 0 0 {w:.2} {h:.2} re f 0 G {CHECK_BOX_STROKE} w {i} {i} {:.2} {:.2} re S", w - 2.0 * i, h - 2.0 * i);
    if checked {
        s.push_str(&format!(" 0.48 w {i} {i} m {r:.2} {t:.2} l S {i} {t:.2} m {r:.2} {i} l S"));
    }
    s.push_str(" Q");
    s
}

/// `value` in Helvetica `size` on one line, clipped to the box, on a white background if `opaque`.
fn text_stream(w: f32, h: f32, size: f32, value: &str, opaque: bool) -> String {
    // On the text's baseline: the box is the line's ascent and descent, the descent ~21 % of it.
    let base = (h * 0.21).max(1.0);
    let text = pdf_escape(&win_ansi(value));
    let bg = if opaque { format!("1 g 0 0 {w:.2} {h:.2} re f ") } else { String::new() };
    format!(
        "/Tx BMC q {bg}1 1 {:.2} {:.2} re W n BT /Helv {size:.1} Tf 0 g 1 {base:.2} Td ({text}) Tj ET Q EMC",
        (w - 2.0).max(0.0),
        (h - 2.0).max(0.0)
    )
}

/// A Helvetica size for `text` in a `w` × `h` box: 70 % of the height (at most 12 pt), smaller
/// when the text would not fit across (down to 4 pt).
fn fit_size(w: f32, h: f32, text: &str) -> f32 {
    let by_height = (h * 0.7).clamp(4.0, 12.0);
    let em = helvetica_width(text);
    if em <= 0.0 { by_height } else { by_height.min((w - 2.0).max(1.0) / em).max(4.0) }
}

/// Width of `text` in Helvetica, in ems (the standard AFM widths for ASCII; 0.556 otherwise).
fn helvetica_width(text: &str) -> f32 {
    const W: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278,
        584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944,
        667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500,
        278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
    ];
    text.chars().map(|c| (c as u32).checked_sub(32).and_then(|i| W.get(i as usize)).copied().unwrap_or(556) as f32 / 1000.0).sum()
}

/// A PDF text string: PDFDocEncoding when it fits (Latin-1 range), else UTF-16BE with a BOM.
fn pdf_text(s: &str) -> Vec<u8> {
    if s.chars().all(|c| (c as u32) < 0x80 || ((c as u32) >= 0xA0 && (c as u32) <= 0xFF)) {
        s.chars().map(|c| c as u32 as u8).collect()
    } else {
        let mut v = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        v
    }
}

/// `s` in WinAnsiEncoding for a Type 1 appearance: Latin-1 plus the usual punctuation; anything
/// else becomes `?`.
fn win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            '\u{20}'..='\u{7E}' | '\u{A0}'..='\u{FF}' => c as u32 as u8,
            '€' => 0x80,
            '‚' => 0x82,
            '„' => 0x84,
            '…' => 0x85,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '\u{2002}' | '\u{2003}' => b' ',
            _ => b'?',
        })
        .collect()
}

fn pdf_escape(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(b as char);
            }
            0x20..=0x7E => s.push(b as char),
            _ => s.push_str(&format!("\\{b:03o}")),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_without_periods() {
        let f = |n: &str| FormField {
            name: n.into(),
            page: 0,
            rect: [0.0; 4],
            kind: FieldKind::CheckBox { checked: false },
            opaque: true,
            cover_when_filled: false,
        };
        let mut v = vec![f("Check"), f("Check"), f("a.b"), f("Check")];
        unique_names(&mut v);
        let names: Vec<&str> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Check", "Check_2", "a_b", "Check_3"]);
    }

    #[test]
    fn blanks_and_their_names() {
        let t = "Ref. No. __________    Code _______ on the day __/__/20__. at __/__ and _ x__y";
        let runs: Vec<&str> = blank_runs(t).into_iter().map(|(a, b)| &t[a..b]).collect();
        assert_eq!(runs, ["__________", "_______", "__/__/20__", "__/__"]);
        assert_eq!(blank_runs("__ / __ / ____"), [(0, 14)]);
        // Blanks already filled in by hand are not blanks.
        assert!(blank_runs("Employee name: __JOHN SMITH______").is_empty());
        assert!(blank_runs("Date filled in __04/05/2025_____").is_empty());
        assert!(blank_runs("Name: ______JOHN__").is_empty());
        assert_eq!(blank_runs("Name: JOHN SMITH______").len(), 1);
        assert_eq!(blank_runs("____ to ____").len(), 2);
        assert_eq!(name_before("Ref. No. "), "Ref_No");
        assert_eq!(name_before("Ref. No. __________    Code "), "Code");
        assert_eq!(name_before("no later than the day "), "day");
        assert_eq!(name_before(""), "Field");
        assert_eq!(name_before("Tax code: "), "Tax_code");
        assert_eq!(name_before("l'importo d'"), "importo");
    }

    #[test]
    fn text_encodings() {
        assert_eq!(pdf_text("Città"), b"Citt\xe0".to_vec());
        assert_eq!(&pdf_text("Ω")[..2], &[0xFE, 0xFF]);
        assert_eq!(pdf_escape(&win_ansi("a(b)\\ è")), "a\\(b\\)\\\\ \\350");
    }
}
