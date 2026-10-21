//! Fillable form fields (AcroForm) for [`crate::PdfOptions::form_fields`].
//!
//! krilla writes no interactive forms, so they are added to the finished PDF: the document's
//! fields are found where the same layout put them on the page, and each becomes a widget with
//! its own appearance, written in the standard PDF font closest to the document's text there.
//!
//! - **Form fields:** legacy form fields (`FORMCHECKBOX`, `FORMTEXT`, `FORMDROPDOWN`) and content
//!   controls that may be edited (plain and rich text, date, check box, drop-down, combo box),
//!   named after the field (`w:name`, tag or title), with the document's value as the page shows
//!   it (no hidden text or tracked deletions). Their widgets are opaque: they cover the field drawn
//!   below, so a printed or flattened copy shows the value filled in, not the one under it.
//!
//! The PDF is read and written again with `lopdf`; the page content, fonts and structure tree are
//! kept as they are.

use lopdf::{Object, ObjectId, Stream, StringFormat, dictionary};
use wordcraft_doc::control::ControlKind;
use wordcraft_doc::legacy_form::LegacyForm;
use wordcraft_doc::para::{InlineObject, OBJ, ShapeKind};
use wordcraft_doc::{Document, Paragraph, Pos, Rgb, StoryRef, TextColor};
use wordcraft_layout::DocLayout;
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

/// The font a field's text is written in: the standard PDF font closest to the document's text
/// there, with its size and colour. Standard fonts need no embedding and every viewer has them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FieldFont {
    /// Resource name (`/TiRo`) and base font (`Times-Roman`).
    pub(crate) key: &'static str,
    pub(crate) base: &'static str,
    /// The text's size in points; `None`: fitted to the box.
    pub(crate) size: Option<f32>,
    /// The text's colour; `None`: black.
    pub(crate) color: Option<[f32; 3]>,
}

impl FieldFont {
    /// Helvetica, fitted to the box, black.
    pub(crate) const DEFAULT: FieldFont = FieldFont { key: "Helv", base: "Helvetica", size: None, color: None };

    /// Font, size and colour for a field's default appearance (`DA`); size 0 is auto size.
    fn da(&self) -> String {
        format!("/{} {} Tf {}", self.key, self.size.map_or("0".to_string(), |s| format!("{s:.1}")), self.fill())
    }

    /// The fill colour operator for the text: `r g b rg`, or black.
    fn fill(&self) -> String {
        match self.color {
            Some([r, g, b]) => format!("{r:.3} {g:.3} {b:.3} rg"),
            None => "0 g".to_string(),
        }
    }

    /// Width of `text` in this font, in ems (close enough to fit a value in its box).
    fn width(&self, text: &str) -> f32 {
        match self.base.split('-').next() {
            Some("Courier") => text.chars().count() as f32 * 0.6,
            Some("Times") => helvetica_width(text) * 0.9,
            _ => helvetica_width(text),
        }
    }
}

/// The standard PDF font for a document font: monospaced ones are Courier, serif ones Times,
/// the rest (Arial, Calibri, Aptos…) Helvetica; with bold and italic.
fn standard_font(name: &str, bold: bool, italic: bool) -> (&'static str, &'static str) {
    const MONO: [&str; 7] = ["mono", "courier", "consolas", "lucida console", "menlo", "code", "typewriter"];
    const SERIF: [&str; 13] = [
        "serif",
        "times",
        "georgia",
        "garamond",
        "cambria",
        "palatino",
        "book antiqua",
        "bookman",
        "century",
        "constantia",
        "baskerville",
        "didot",
        "roman",
    ];
    let n = name.to_lowercase();
    let family = if MONO.iter().any(|m| n.contains(m)) {
        2
    } else if !n.contains("sans") && SERIF.iter().any(|m| n.contains(m)) {
        1
    } else {
        0
    };
    const FONTS: [[(&str, &str); 4]; 3] = [
        [("Helv", "Helvetica"), ("HeBo", "Helvetica-Bold"), ("HeOb", "Helvetica-Oblique"), ("HeBO", "Helvetica-BoldOblique")],
        [("TiRo", "Times-Roman"), ("TiBo", "Times-Bold"), ("TiIt", "Times-Italic"), ("TiBI", "Times-BoldItalic")],
        [("Cour", "Courier"), ("CoBo", "Courier-Bold"), ("CoOb", "Courier-Oblique"), ("CoBO", "Courier-BoldOblique")],
    ];
    FONTS[family][usize::from(bold) + 2 * usize::from(italic)]
}

/// The font of the document's text at `off` in `p`: family, bold, italic, size and colour.
fn font_at(doc: &Document, p: &Paragraph, off: usize) -> FieldFont {
    let rc = doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(off));
    let (key, base) = standard_font(&rc.font, rc.bold, rc.italic);
    let color = match rc.color {
        TextColor::Rgb(Rgb(r, g, b)) if (r, g, b) != (0, 0, 0) => Some([f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0]),
        _ => None,
    };
    FieldFont { key, base, size: Some(rc.size).filter(|s| s.is_finite() && *s > 0.0).map(|s| s.clamp(4.0, 72.0)), color }
}

/// A fillable field where the layout put it: `rect` is (x, y, w, h) in points from the page's
/// top-left corner, on 0-based layout page `page`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FormField {
    pub(crate) name: String,
    pub(crate) page: usize,
    pub(crate) rect: [f32; 4],
    pub(crate) kind: FieldKind,
    /// The font its text is written in.
    pub(crate) font: FieldFont,
    /// An opaque white background covering what is drawn below (a field's static value).
    pub(crate) opaque: bool,
}

impl FormField {
    /// A field with an opaque background, its text left-aligned and free.
    pub(crate) fn new(name: String, page: usize, rect: [f32; 4], kind: FieldKind, font: FieldFont) -> FormField {
        FormField { name, page, rect, kind, font, opaque: true }
    }
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
            if omitted(doc, p, off) {
                continue;
            }
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
            out.push(FormField::new(base, page, rect, kind, font_at(doc, p, off)));
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
        if omitted(doc, p, r.start.off) {
            continue;
        }
        let from = Pos::new(story, r.start.path.clone(), r.start.off + OBJ.len_utf8());
        let Some((page, rect)) = span(lay, &from, &r.end) else {
            continue;
        };
        let text = clean(&visible_text(doc, p, from.off, r.end.off));
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
        out.push(FormField::new(name, page, rect, kind, font_at(doc, p, from.off)));
    }
    unique_names(&mut out);
    out
}

/// Width of the border the layout strokes around a check box's square.
const CHECK_BOX_STROKE: f32 = 0.72;

/// White margin around a check box widget, covering the drawn square should a viewer round its
/// position differently.
const CHECK_BOX_PAD: f32 = 1.0 + CHECK_BOX_STROKE / 2.0;

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

/// The check box squares among `draws` (and the groups inside them): 0.72 pt stroked squares.
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

/// Whether the text at `off` in `p` is left out of the page: hidden, or a tracked deletion (the
/// export shows the final text). Such a field is not made fillable.
fn omitted(doc: &Document, p: &Paragraph, off: usize) -> bool {
    let rc = doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(off));
    rc.hidden || rc.del.is_some()
}

/// The text of `p` in `from..to` as the page shows it: without hidden text and tracked
/// deletions, which must not reach a field's value.
fn visible_text(doc: &Document, p: &Paragraph, from: usize, to: usize) -> String {
    let style = p.props.style.as_deref();
    let mut out = String::new();
    for (r, props) in p.run_ranges() {
        let (a, b) = (r.start.max(from), r.end.min(to));
        if a >= b {
            continue;
        }
        let rc = doc.styles.resolve_char(style, props);
        if rc.hidden || rc.del.is_some() {
            continue;
        }
        out.push_str(p.text.get(a..b).unwrap_or(""));
    }
    out
}

/// Text without object markers and with line breaks as spaces.
fn clean(s: &str) -> String {
    s.chars().filter(|c| *c != OBJ).map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().trim_end().to_string()
}

/// A field's name when the document gives none: its kind.
fn default_name(kind: &FieldKind) -> String {
    match kind {
        FieldKind::CheckBox { .. } => "Check".into(),
        FieldKind::Text { .. } => "Text".into(),
        FieldKind::Choice { .. } => "Choice".into(),
    }
}

/// Field names must be unique (one field per name: viewers join fields sharing a name, so typing
/// in one fills the other): a name already given gets the first free `_2`, `_3`… suffix, a
/// suffix never matching another field's name.
pub(crate) fn unique_names(fields: &mut [FormField]) {
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut next: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for f in fields.iter_mut() {
        // PDF partial names may not contain periods.
        let base = f.name.replace('.', "_");
        if used.insert(base.clone()) {
            f.name = base;
            continue;
        }
        let n = next.entry(base.clone()).or_insert(1);
        loop {
            *n += 1;
            let name = format!("{base}_{n}");
            if used.insert(name.clone()) {
                f.name = name;
                break;
            }
        }
    }
}

/// `pdf` with an AcroForm holding `fields` (none: unchanged).
pub(crate) fn add_acroform(pdf: Vec<u8>, fields: &[FormField]) -> Result<Vec<u8>, String> {
    if fields.is_empty() {
        return Ok(pdf);
    }
    let mut doc = lopdf::Document::load_mem(&pdf).map_err(|e| format!("reading the exported PDF: {e}"))?;
    let pages = doc.get_pages();
    let mut fonts: std::collections::BTreeMap<&'static str, ObjectId> = std::collections::BTreeMap::new();
    // Helvetica always: the form's own default appearance names it.
    font_obj(&mut doc, &mut fonts, &FieldFont::DEFAULT);
    let mut refs = Vec::new();
    for f in fields {
        let Some(&page_id) = u32::try_from(f.page + 1).ok().and_then(|n| pages.get(&n)) else {
            continue;
        };
        let page_h = page_height(&doc, page_id).unwrap_or(842.0);
        // A check box's rect is its square; the widget adds a white margin around it.
        let (px, py) = if matches!(f.kind, FieldKind::CheckBox { .. }) { (CHECK_BOX_PAD, CHECK_BOX_PAD) } else { (0.0, 0.0) };
        let [x, y, w, h] = [f.rect[0] - px, f.rect[1] - py, f.rect[2] + 2.0 * px, f.rect[3] + 2.0 * py];
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
                let size = fit_size(w, h, value, &f.font);
                let font = (f.font.key, font_obj(&mut doc, &mut fonts, &f.font));
                d.set("FT", "Tx");
                d.set("V", Object::String(pdf_text(value), StringFormat::Literal));
                // Auto size while filling in; a white background when the viewer redraws it.
                d.set("DA", Object::String(f.font.da().into_bytes(), StringFormat::Literal));
                let mut mk = lopdf::Dictionary::new();
                if f.opaque {
                    mk.set("BG", vec![1.into(), 1.into(), 1.into()]);
                }
                if !mk.is_empty() {
                    d.set("MK", mk);
                }
                if let Some(n) = max_len {
                    d.set("MaxLen", i64::from(*n));
                }
                let ap = appearance(&mut doc, w, h, &text_stream(w, h, size, value, f.opaque, &f.font), Some(font));
                d.set("AP", dictionary! { "N" => ap });
            }
            FieldKind::Choice { options, selected, editable } => {
                let shown = selected.and_then(|i| options.get(i)).cloned().unwrap_or_default();
                let size = fit_size(w, h, &shown, &f.font);
                let font = (f.font.key, font_obj(&mut doc, &mut fonts, &f.font));
                d.set("FT", "Ch");
                // Combo box (bit 18), editable (bit 19).
                d.set("Ff", if *editable { (1i64 << 17) | (1 << 18) } else { 1i64 << 17 });
                d.set("Opt", options.iter().map(|o| Object::String(pdf_text(o), StringFormat::Literal)).collect::<Vec<_>>());
                d.set("V", Object::String(pdf_text(&shown), StringFormat::Literal));
                d.set("DA", Object::String(f.font.da().into_bytes(), StringFormat::Literal));
                d.set("MK", dictionary! { "BG" => vec![1.into(), 1.into(), 1.into()] });
                let ap = appearance(&mut doc, w, h, &text_stream(w, h, size, &shown, f.opaque, &f.font), Some(font));
                d.set("AP", dictionary! { "N" => ap });
            }
        }
        let id = doc.add_object(d);
        add_annot(&mut doc, page_id, id)?;
        refs.push(Object::Reference(id));
    }
    let root = doc.trailer.get(b"Root").and_then(Object::as_reference).map_err(|e| format!("no catalog: {e}"))?;
    let form = dictionary! {
        "Fields" => refs,
        "DR" => dictionary! { "Font" => lopdf::Dictionary::from_iter(fonts.iter().map(|(k, id)| (k.as_bytes().to_vec(), Object::Reference(*id)))) },
        "DA" => Object::String(b"/Helv 0 Tf 0 g".to_vec(), StringFormat::Literal),
    };
    doc.get_object_mut(root).and_then(Object::as_dict_mut).map_err(|e| format!("bad catalog: {e}"))?.set("AcroForm", form);
    let mut out = Vec::with_capacity(pdf.len() + 4096);
    doc.save_to(&mut out).map_err(|e| format!("writing the PDF: {e}"))?;
    Ok(out)
}

/// The height of page `page` from its media box, in points.
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

/// Add widget `annot` to page `page`'s annotations, the page's tab order following its structure.
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

/// A form XObject `w` × `h` drawing `content`, with a font under its resource name when given.
fn appearance(doc: &mut lopdf::Document, w: f32, h: f32, content: &str, font: Option<(&str, ObjectId)>) -> ObjectId {
    let mut dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => vec![Object::Real(0.0), Object::Real(0.0), Object::Real(w), Object::Real(h)],
    };
    if let Some((key, f)) = font {
        let mut fd = lopdf::Dictionary::new();
        fd.set(key, f);
        dict.set("Resources", dictionary! { "Font" => fd });
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

/// `value` in `font` at `size` on one line, clipped to the box, on a white background if `opaque`.
fn text_stream(w: f32, h: f32, size: f32, value: &str, opaque: bool, font: &FieldFont) -> String {
    // On the text's baseline: the box is the line's ascent and descent, the descent ~21 % of it.
    let (key, fill) = (font.key, font.fill());
    let base = (h * 0.21).max(1.0);
    let text = pdf_escape(&win_ansi(value));
    let bg = if opaque { format!("1 g 0 0 {w:.2} {h:.2} re f ") } else { String::new() };
    format!(
        "/Tx BMC q {bg}1 1 {:.2} {:.2} re W n BT /{key} {size:.1} Tf {fill} 1 {base:.2} Td ({text}) Tj ET Q EMC",
        (w - 2.0).max(0.0),
        (h - 2.0).max(0.0)
    )
}

/// A size for `text` in `font` in a `w` × `h` box: the font's own size (the document's), else 70 %
/// of the height (at most 12 pt); smaller when the text would not fit across (down to 4 pt).
fn fit_size(w: f32, h: f32, text: &str, font: &FieldFont) -> f32 {
    let wanted = font.size.unwrap_or((h * 0.7).clamp(4.0, 12.0));
    let em = font.width(text);
    if em <= 0.0 { wanted } else { wanted.min((w - 2.0).max(1.0) / em).max(4.0) }
}

/// The font object for `font` (a standard Type 1 font, WinAnsi), added once.
fn font_obj(doc: &mut lopdf::Document, fonts: &mut std::collections::BTreeMap<&'static str, ObjectId>, font: &FieldFont) -> ObjectId {
    *fonts.entry(font.key).or_insert_with(|| {
        doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => font.base, "Encoding" => "WinAnsiEncoding" })
    })
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

/// `bytes` as the inside of a PDF literal string: parentheses and backslashes escaped, bytes
/// outside printable ASCII as octal escapes.
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
        let f = |n: &str| FormField::new(n.into(), 0, [0.0; 4], FieldKind::CheckBox { checked: false }, FieldFont::DEFAULT);
        let mut v = vec![f("Check"), f("Check"), f("a.b"), f("Check")];
        unique_names(&mut v);
        let names: Vec<&str> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Check", "Check_2", "a_b", "Check_3"]);
        // A suffix never takes a name a field already has, nor one given before.
        let mut v = vec![f("A"), f("A_2"), f("A_2_2"), f("A"), f("A_2")];
        unique_names(&mut v);
        let names: Vec<&str> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["A", "A_2", "A_2_2", "A_3", "A_2_3"]);
        let mut v = vec![f("Name_2"), f("Name"), f("Name")];
        unique_names(&mut v);
        let names: Vec<&str> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Name_2", "Name", "Name_3"]);
    }

    #[test]
    fn standard_fonts_for_document_fonts() {
        let base = |n: &str, b: bool, i: bool| standard_font(n, b, i).1;
        assert_eq!(base("Aptos", false, false), "Helvetica");
        assert_eq!(base("Calibri", true, false), "Helvetica-Bold");
        assert_eq!(base("Microsoft Sans Serif", false, false), "Helvetica");
        assert_eq!(base("Times New Roman", false, true), "Times-Italic");
        assert_eq!(base("Aptos Serif", true, true), "Times-BoldItalic");
        assert_eq!(base("Consolas", false, false), "Courier");
        assert_eq!(base("Aptos Mono", true, false), "Courier-Bold");
        assert_eq!(base("", false, false), "Helvetica");
        let f = FieldFont { size: Some(10.0), color: Some([1.0, 0.0, 0.0]), ..FieldFont::DEFAULT };
        assert_eq!(f.da(), "/Helv 10.0 Tf 1.000 0.000 0.000 rg");
        assert_eq!(FieldFont::DEFAULT.da(), "/Helv 0 Tf 0 g");
    }

    #[test]
    fn hidden_and_deleted_text_stays_out_of_fields() {
        let mut d = Document::from_text("ab secret cd");
        let story = StoryRef::Body;
        let path = d.para_paths(story)[0].clone();
        let at = |o: usize| Pos::new(story, path.clone(), o);
        d.format_range(&at(3), &at(10), &|c| c.hidden = Some(true)).unwrap();
        let p = d.para(story, &path).unwrap();
        assert_eq!(visible_text(&d, p, 0, p.text.len()), "ab cd");
        assert!(omitted(&d, p, 4) && !omitted(&d, p, 0));
        d.format_range(&at(0), &at(3), &|c| c.del = Some(1)).unwrap();
        let p = d.para(story, &path).unwrap();
        assert_eq!(visible_text(&d, p, 0, p.text.len()), "cd");
    }

    #[test]
    fn text_encodings() {
        assert_eq!(pdf_text("Città"), b"Citt\xe0".to_vec());
        assert_eq!(&pdf_text("Ω")[..2], &[0xFE, 0xFF]);
        assert_eq!(pdf_escape(&win_ansi("a(b)\\ è")), "a\\(b\\)\\\\ \\350");
    }
}
