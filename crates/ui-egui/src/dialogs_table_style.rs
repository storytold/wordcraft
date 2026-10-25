//! Table Design › New / Modify Table Style (#714): one table style part at a time ("Apply
//! formatting to"), with its font, size, bold, italic, underline and colour, cell shading, borders
//! (line style, width and colour, and which edges) and cell alignment, over a live preview. OK
//! runs `table.newStyle` / `table.modifyStyle` with only what changed.

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, TableLook, Underline, VAlign};
use wordcraft_doc::styles::{PartFormat, PartSet, TablePart, TableStyleProps};
use wordcraft_doc::{CharProps, Rgb, TextColor};
use wordcraft_engine::cmd::table_style::CELL_ALIGNS;

use crate::WordApp;
use crate::dialogs::{Dialog, buttons, color_menu};
use crate::theme::Tokens;

/// One table style part in the dialog, as it resolves through the based-on chain: colours are
/// hex, empty for none.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRegion {
    pub font: String,
    /// Points as typed; empty when the style doesn't set one.
    pub size: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub color: String,
    pub fill: String,
    pub borders: Borders,
    /// One of the `cellAlign` names (`topLeft`…), empty when the part doesn't set one.
    pub align: String,
}

/// The line the border buttons draw.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BorderPen {
    pub style: BorderStyle,
    pub width: f32,
    /// Hex, empty for automatic.
    pub color: String,
}

impl Default for BorderPen {
    fn default() -> Self {
        BorderPen { style: BorderStyle::Single, width: 0.5, color: String::new() }
    }
}

impl BorderPen {
    fn line(&self) -> Border {
        Border { style: self.style, width: self.width, color: Rgb::parse(&self.color), space: 0.0 }
    }
}

/// The parts "Apply formatting to" offers, in its order.
pub const PARTS: [(TablePart, &str); 13] = [
    (TablePart::WholeTable, "Whole Table"),
    (TablePart::FirstRow, "Header Row"),
    (TablePart::LastRow, "Total Row"),
    (TablePart::FirstCol, "First Column"),
    (TablePart::LastCol, "Last Column"),
    (TablePart::Band1Horz, "Odd Banded Rows"),
    (TablePart::Band2Horz, "Even Banded Rows"),
    (TablePart::Band1Vert, "Odd Banded Columns"),
    (TablePart::Band2Vert, "Even Banded Columns"),
    (TablePart::NwCell, "Top Left Cell"),
    (TablePart::NeCell, "Top Right Cell"),
    (TablePart::SwCell, "Bottom Left Cell"),
    (TablePart::SeCell, "Bottom Right Cell"),
];

/// The index in [`PARTS`] of `part`.
pub fn part_index(part: TablePart) -> usize {
    PARTS.iter().position(|(p, _)| *p == part).unwrap_or(0)
}

/// The border picker's choices: label and the `edges` param.
const EDGES: [(&str, &str); 10] = [
    ("All Borders", "all"),
    ("No Border", "none"),
    ("Outside Borders", "outside"),
    ("Inside Borders", "inside"),
    ("Top Border", "top"),
    ("Bottom Border", "bottom"),
    ("Left Border", "left"),
    ("Right Border", "right"),
    ("Inside Horizontal Border", "insideH"),
    ("Inside Vertical Border", "insideV"),
];

/// The alignment grid's tooltips, in `CELL_ALIGNS` order.
const ALIGN_TIPS: [&str; 9] = [
    "Align Top Left",
    "Align Top Center",
    "Align Top Right",
    "Align Center Left",
    "Align Center",
    "Align Center Right",
    "Align Bottom Left",
    "Align Bottom Center",
    "Align Bottom Right",
];

/// The `cellAlign` name for a part's alignments, empty when it sets neither.
fn align_name(h: Option<Align>, v: Option<VAlign>) -> String {
    if h.is_none() && v.is_none() {
        return String::new();
    }
    let (h, v) = (h.unwrap_or(Align::Left), v.unwrap_or(VAlign::Top));
    let h = if matches!(h, Align::Justify | Align::Distribute) { Align::Left } else { h };
    CELL_ALIGNS.iter().find(|(_, va, ha)| *va == v && *ha == h).map(|(n, _, _)| n.to_string()).unwrap_or_default()
}

/// A size in points as the box shows it.
fn size_text(pt: f32) -> String {
    if (pt - pt.round()).abs() < 0.01 { format!("{}", pt.round() as i64) } else { format!("{pt:.1}") }
}

/// The parts of table style `id` as resolved through its based-on chain, in [`PARTS`] order. A
/// part's text shows the whole table's formatting with its own over it.
pub fn table_regions(app: &WordApp, id: &str) -> Box<[TableRegion; 13]> {
    let Some(st) = app.session.doc.styles.table_style(id) else { return Default::default() };
    Box::new(PARTS.map(|(part, _)| region_of(&st, part)))
}

fn region_of(st: &TableStyleProps, part: TablePart) -> TableRegion {
    let f = st.part(part);
    let mut c = st.chr.clone();
    c.overlay(&f.chr);
    TableRegion {
        font: c.font.clone().unwrap_or_default(),
        size: c.size.map(size_text).unwrap_or_default(),
        bold: c.bold.unwrap_or(false),
        italic: c.italic.unwrap_or(false),
        underline: c.underline.is_some_and(|u| u != Underline::None),
        color: match c.color {
            Some(TextColor::Rgb(c)) => c.hex(),
            _ => String::new(),
        },
        fill: f.fill.map(Rgb::hex).unwrap_or_default(),
        borders: f.borders.unwrap_or_default(),
        align: align_name(f.align, f.v_align),
    }
}

/// The params `table.newStyle` / `table.modifyStyle` take for the dialog's state: only what
/// changed from `basis`, part by part, so the rest stays inherited.
pub fn table_style_params(id: Option<&str>, name: &str, based_on: &str, regions: &[TableRegion; 13], basis: &[TableRegion; 13]) -> Value {
    let mut v = json!({"name": name.trim(), "basedOn": based_on});
    for (((part, _), r), b) in PARTS.iter().zip(regions.iter()).zip(basis.iter()) {
        let ch = region_changes(r, b);
        if ch.as_object().is_some_and(|o| !o.is_empty()) {
            v[part.ooxml()] = ch;
        }
    }
    if let Some(id) = id {
        v["style"] = json!(id);
    }
    v
}

/// The params for what changed in one part.
pub fn region_changes(r: &TableRegion, basis: &TableRegion) -> Value {
    let color = |c: &str| if c.is_empty() { Value::Null } else { json!(c) };
    let mut v = json!({});
    if r.font.trim() != basis.font.trim() {
        // An emptied box makes the part's font inherited again.
        v["font"] = if r.font.trim().is_empty() { Value::Null } else { json!(r.font.trim()) };
    }
    if r.size.trim() != basis.size.trim()
        && let Ok(pt) = r.size.trim().parse::<f32>()
        && pt.is_finite()
        && pt > 0.0
    {
        v["size"] = json!(pt);
    }
    for (k, a, b) in [("bold", r.bold, basis.bold), ("italic", r.italic, basis.italic), ("underline", r.underline, basis.underline)] {
        if a != b {
            v[k] = json!(a);
        }
    }
    if r.color != basis.color {
        v["color"] = color(&r.color);
    }
    if r.fill != basis.fill {
        v["fill"] = color(&r.fill);
    }
    if r.align != basis.align {
        v["cellAlign"] = if r.align.is_empty() { Value::Null } else { json!(r.align) };
    }
    let edges =
        |b: &Borders| [("top", b.top), ("bottom", b.bottom), ("left", b.left), ("right", b.right), ("insideH", b.between), ("insideV", b.inside_v)];
    let changed: Vec<Value> = edges(&r.borders)
        .into_iter()
        .zip(edges(&basis.borders))
        // An explicit "no line" edge differs from an unset one: it hides the lines a lower part
        // (or the based-on style) draws.
        .filter(|((_, a), (_, b))| a != b)
        .filter_map(|((e, a), _)| match a {
            Some(l) if l.is_visible() => Some(json!({"edges": e, "style": l.style.ooxml(), "width": l.width, "color": l.color.map(Rgb::hex)})),
            Some(_) => Some(json!({"edges": e, "style": "none"})),
            // The picker never unsets an edge.
            None => None,
        })
        .collect();
    if !changed.is_empty() {
        v["borders"] = Value::Array(changed);
    }
    v
}

/// Apply the border picker's `edges` choice to `b` with `pen`: All, None, Outside and Inside set
/// their edges; a single edge toggles.
pub fn pick_edges(b: &mut Borders, edges: &str, pen: &BorderPen) {
    let none = Border { style: BorderStyle::None, width: 0.0, color: None, space: 0.0 };
    let line = pen.line();
    let toggle = |e: &mut Option<Border>| *e = Some(if e.is_some_and(|x| x.is_visible()) { none } else { line });
    match edges {
        "all" => *b = Borders::all(line),
        "none" => *b = Borders::all(none),
        "outside" => (b.top, b.bottom, b.left, b.right) = (Some(line), Some(line), Some(line), Some(line)),
        "inside" => (b.between, b.inside_v) = (Some(line), Some(line)),
        "top" => toggle(&mut b.top),
        "bottom" => toggle(&mut b.bottom),
        "left" => toggle(&mut b.left),
        "right" => toggle(&mut b.right),
        "insideH" => toggle(&mut b.between),
        "insideV" => toggle(&mut b.inside_v),
        _ => {}
    }
}

/// Whether `edges` are all drawn in `b` (the picker shows those choices checked).
fn edges_on(b: &Borders, edges: &str) -> bool {
    let on = |e: Option<Border>| e.is_some_and(|x| x.is_visible());
    match edges {
        "all" => [b.top, b.bottom, b.left, b.right, b.between, b.inside_v].into_iter().all(on),
        // Only explicit "no line" edges: unset ones still show what the part inherits.
        "none" => [b.top, b.bottom, b.left, b.right, b.between, b.inside_v].into_iter().all(|e| e.is_some_and(|x| !x.is_visible())),
        "outside" => [b.top, b.bottom, b.left, b.right].into_iter().all(on),
        "inside" => on(b.between) && on(b.inside_v),
        "top" => on(b.top),
        "bottom" => on(b.bottom),
        "left" => on(b.left),
        "right" => on(b.right),
        "insideH" => on(b.between),
        "insideV" => on(b.inside_v),
        _ => false,
    }
}

/// A small square showing which edges a border choice draws: the drawn edges in the text colour,
/// the others as faint dotted guides.
fn edge_glyph(ui: &mut Ui, edges: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
    let p = ui.painter_at(r);
    let r = r.shrink(2.0);
    let c = r.center();
    let segs = [
        ("top", [r.left_top(), r.right_top()]),
        ("bottom", [r.left_bottom(), r.right_bottom()]),
        ("left", [r.left_top(), r.left_bottom()]),
        ("right", [r.right_top(), r.right_bottom()]),
        ("insideH", [pos2(r.left(), c.y), pos2(r.right(), c.y)]),
        ("insideV", [pos2(c.x, r.top()), pos2(c.x, r.bottom())]),
    ];
    for (e, seg) in segs {
        let on = match edges {
            "all" => true,
            "none" => false,
            "outside" => matches!(e, "top" | "bottom" | "left" | "right"),
            "inside" => matches!(e, "insideH" | "insideV"),
            x => x == e,
        };
        if on {
            p.line_segment(seg, Stroke::new(1.5, t.icon));
        } else {
            p.extend(egui::Shape::dotted_line(&seg, t.text_disabled, 2.5, 0.6));
        }
    }
}

/// One button of the alignment grid: a square with three short bars where the text sits.
fn align_button(ui: &mut Ui, (v, h): (VAlign, Align), on: bool, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    let resp = resp.on_hover_text(tip);
    let p = ui.painter_at(r);
    let bg = if on {
        t.checked
    } else if resp.hovered() {
        t.hover
    } else {
        t.input
    };
    p.rect(r, 2.0, bg, Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    let inner = r.shrink(4.0);
    let y0 = match v {
        VAlign::Top => inner.top(),
        VAlign::Center => inner.center().y - 4.0,
        VAlign::Bottom => inner.bottom() - 8.0,
    };
    for (i, w) in [10.0f32, 6.0, 10.0].into_iter().enumerate() {
        let x0 = match h {
            Align::Center => inner.center().x - w / 2.0,
            Align::Right => inner.right() - w,
            _ => inner.left(),
        };
        p.rect_filled(Rect::from_min_size(pos2(x0, y0 + i as f32 * 3.0), vec2(w, 1.6)), 0.0, t.icon);
    }
    resp
}

/// The table style options that show `part` in the preview.
fn look_for(mut look: TableLook, part: TablePart) -> TableLook {
    match part {
        TablePart::FirstRow => look.header_row = true,
        TablePart::LastRow => look.total_row = true,
        TablePart::FirstCol => look.first_column = true,
        TablePart::LastCol => look.last_column = true,
        TablePart::Band1Horz | TablePart::Band2Horz => look.banded_rows = true,
        TablePart::Band1Vert | TablePart::Band2Vert => look.banded_columns = true,
        TablePart::NwCell => (look.header_row, look.first_column) = (true, true),
        TablePart::NeCell => (look.header_row, look.last_column) = (true, true),
        TablePart::SwCell => (look.total_row, look.first_column) = (true, true),
        TablePart::SeCell => (look.total_row, look.last_column) = (true, true),
        TablePart::WholeTable => {}
    }
    look
}

/// A small sample table drawn with `style` (5 columns, a header, three body rows and a total
/// row), showing the parts `look` turns on: the dialog's live preview.
pub fn table_style_preview(ui: &mut Ui, style: Option<&TableStyleProps>, look: TableLook) {
    const COLS: usize = 5;
    const ROWS: usize = 5;
    const TEXT: [[&str; COLS]; ROWS] = [
        ["", "Mon", "Tue", "Wed", "Sum"],
        ["North", "4", "7", "2", "13"],
        ["South", "6", "1", "5", "12"],
        ["West", "3", "8", "4", "15"],
        ["Total", "13", "16", "11", "40"],
    ];
    let (rect, _) = ui.allocate_exact_size(vec2(360.0, 130.0), Sense::hover());
    let painter = ui.painter_at(rect);
    // The page is white whatever the interface theme.
    painter.rect_filled(rect, 2.0, egui::Color32::WHITE);
    let t = Tokens::get(ui.ctx());
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let Some(st) = style else { return };
    let formats: Vec<PartFormat> = TablePart::ALL.iter().map(|p| st.part(*p)).collect();
    let table = rect.shrink2(vec2(14.0, 10.0));
    let (cw, rh) = (table.width() / COLS as f32, table.height() / ROWS as f32);
    let rgb = |c: Rgb| egui::Color32::from_rgb(c.0, c.1, c.2);
    let tb = st.parts.borders.unwrap_or_default();
    let band = (st.parts.band_size.unwrap_or(0), st.parts.col_band_size.unwrap_or(0));
    for (ri, row) in TEXT.iter().enumerate() {
        for (ci, txt) in row.iter().enumerate() {
            // Sample words in the interface language; numbers as they are.
            let txt = if txt.chars().any(char::is_alphabetic) { tl!(*txt) } else { *txt };
            let cell = Rect::from_min_size(table.min + vec2(ci as f32 * cw, ri as f32 * rh), vec2(cw, rh));
            let mut fill = None;
            let mut chr = CharProps::default();
            let (mut h, mut v) = (st.para.align, None);
            let mut b = Borders {
                top: if ri == 0 { tb.top } else { tb.between },
                bottom: if ri + 1 == ROWS { tb.bottom } else { tb.between },
                left: if ci == 0 { tb.left } else { tb.inside_v },
                right: if ci + 1 == COLS { tb.right } else { tb.inside_v },
                between: None,
                inside_v: None,
            };
            // Parts from lowest to highest precedence, as layout applies them.
            for part in PartSet::of_cell(&look, band, (ri, ROWS), (ci, 1, COLS)).iter() {
                let Some(f) = TablePart::ALL.iter().position(|p| *p == part).and_then(|i| formats.get(i)) else { continue };
                fill = f.fill.or(fill);
                chr.overlay(&f.chr);
                h = f.align.or(h);
                v = f.v_align.or(v);
                if part != TablePart::WholeTable
                    && let Some(rb) = f.borders
                {
                    let rb = if part.is_row() {
                        Borders {
                            left: if ci == 0 { rb.left } else { rb.inside_v },
                            right: if ci + 1 == COLS { rb.right } else { rb.inside_v },
                            ..rb
                        }
                    } else if part.is_column() {
                        Borders { top: if ri == 0 { rb.top } else { rb.between }, bottom: if ri + 1 == ROWS { rb.bottom } else { rb.between }, ..rb }
                    } else {
                        rb
                    };
                    b.overlay(&Borders { between: None, inside_v: None, ..rb });
                }
            }
            if let Some(f) = fill {
                painter.rect_filled(cell, 0.0, rgb(f));
            }
            let edge = |from: egui::Pos2, to: egui::Pos2, e: Option<Border>| {
                if let Some(e) = e.filter(Border::is_visible) {
                    let color = e.color.map_or(egui::Color32::BLACK, rgb);
                    let stroke = Stroke::new(e.width.clamp(0.5, 3.0) * 1.3, color);
                    if matches!(e.style, BorderStyle::Dotted | BorderStyle::Dashed | BorderStyle::DotDash) {
                        painter.extend(egui::Shape::dashed_line(&[from, to], stroke, 3.0, 2.0));
                    } else {
                        painter.line_segment([from, to], stroke);
                    }
                }
            };
            edge(cell.left_top(), cell.right_top(), b.top);
            edge(cell.left_bottom(), cell.right_bottom(), b.bottom);
            edge(cell.left_top(), cell.left_bottom(), b.left);
            edge(cell.right_top(), cell.right_bottom(), b.right);
            let color = match chr.color {
                Some(TextColor::Rgb(c)) => rgb(c),
                _ => egui::Color32::BLACK,
            };
            let font = egui::FontId::proportional(11.0);
            let (ax, x) = match h {
                Some(Align::Center) => (egui::Align::Center, cell.center().x),
                Some(Align::Right) => (egui::Align::Max, cell.right() - 4.0),
                _ => (egui::Align::Min, cell.left() + 4.0),
            };
            let (ay, y) = match v {
                Some(VAlign::Center) => (egui::Align::Center, cell.center().y),
                Some(VAlign::Bottom) => (egui::Align::Max, cell.bottom() - 2.0),
                _ => (egui::Align::Min, cell.top() + 2.0),
            };
            let pos = pos2(x, y);
            let r = painter.text(pos, egui::Align2([ax, ay]), txt, font.clone(), color);
            if chr.bold.unwrap_or(false) {
                // A second pass a hair to the right reads as bold at this size.
                painter.text(pos + vec2(0.6, 0.0), egui::Align2([ax, ay]), txt, font, color);
            }
            if chr.underline.is_some_and(|u| u != Underline::None) && !txt.is_empty() {
                painter.line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(0.8, color));
            }
        }
    }
}

/// The dialog's state.
pub struct Form<'a> {
    pub id: &'a Option<String>,
    pub name: &'a mut String,
    pub based_on: &'a mut String,
    pub region: &'a mut usize,
    pub regions: &'a mut Box<[TableRegion; 13]>,
    pub basis: &'a mut Box<[TableRegion; 13]>,
    pub pen: &'a mut BorderPen,
}

/// The dialog body. Returns true to close.
pub fn body(app: &mut WordApp, ui: &mut Ui, f: Form) -> bool {
    let Form { id, name, based_on, region, regions, basis, pen } = f;
    let styles: Vec<(String, String)> = app
        .session
        .doc
        .styles
        .styles
        .iter()
        .filter(|s| s.kind == wordcraft_doc::StyleKind::Table && Some(&s.id) != id.as_ref())
        .map(|s| (s.id.clone(), s.name.clone()))
        .collect();
    let base_name = styles.iter().find(|(i, _)| i == based_on).map(|(_, n)| n.clone()).unwrap_or_else(|| based_on.clone());
    let mut rebased = false;
    egui::Grid::new("tstyle").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let name_label = ui.label(tl!("Name:"));
        ui.text_edit_singleline(name).labelled_by(name_label.id);
        ui.end_row();
        let base_label = ui.label(tl!("Style based on:"));
        let base = egui::ComboBox::from_id_salt("tstyle_base").width(200.0).selected_text(base_name).show_ui(ui, |ui| {
            for (sid, sname) in &styles {
                if ui.selectable_label(sid == based_on, sname).clicked() && sid != based_on {
                    *based_on = sid.clone();
                    rebased = true;
                }
            }
        });
        base.response.labelled_by(base_label.id);
        ui.end_row();
        let region_label = ui.label(tl!("Apply formatting to:"));
        let part = egui::ComboBox::from_id_salt("tstyle_region")
            .width(200.0)
            .selected_text(tl!(PARTS.get(*region).map_or("Whole Table", |(_, l)| *l)))
            .show_ui(ui, |ui| {
                for (i, (_, n)) in PARTS.iter().enumerate() {
                    ui.selectable_value(region, i, tl!(*n));
                }
            });
        part.response.labelled_by(region_label.id);
        ui.end_row();
    });
    // A new style shows its new base's formatting; a modified one keeps its own.
    if rebased && id.is_none() {
        *regions = table_regions(app, based_on);
        *basis = regions.clone();
    }
    ui.separator();
    let theme = app.session.doc.settings.theme_colors.clone();
    let fams = app.previews.families();
    let preview_fn = app.previews.font_preview_fn();
    if let Some(r) = regions.get_mut(*region) {
        ui.label(egui::RichText::new(tl!("Formatting")).font(crate::theme::semibold(12.5)));
        // Font, size, bold, italic, underline and text colour.
        ui.horizontal(|ui| {
            if let Some(f) = crate::widgets::font_combo(ui, "tstyle_font", 170.0, &r.font, &fams, Some(&*preview_fn))
                && f != crate::widgets::ADD_FONT
            {
                r.font = f;
            }
            let size = ui.add(egui::TextEdit::singleline(&mut r.size).desired_width(36.0));
            size.on_hover_text(tl!("Size:")).accessible_name(tl!("Size:"));
            ui.toggle_value(&mut r.bold, egui::RichText::new("B").strong()).on_hover_text(tl!("Bold"));
            ui.toggle_value(&mut r.italic, egui::RichText::new("I").italics()).on_hover_text(tl!("Italic"));
            ui.toggle_value(&mut r.underline, egui::RichText::new("U").underline()).on_hover_text(tl!("Underline"));
            ui.push_id("tstyle_color", |ui| {
                ui.label(tl!("Font color:"));
                color_menu(ui, &theme, &mut r.color);
            });
        });
        // Borders: the pen (style, width, colour) and which edges; then the cell shading.
        ui.horizontal(|ui| {
            let current = LINE_STYLES.iter().find(|(s, _)| *s == pen.style).map_or("Single", |(_, n)| *n);
            let style_label = ui.label(tl!("Style:"));
            let style = egui::ComboBox::from_id_salt("tstyle_pen_style").selected_text(tl!(current)).width(90.0).show_ui(ui, |ui| {
                for (s, n) in LINE_STYLES {
                    ui.selectable_value(&mut pen.style, s, tl!(n));
                }
            });
            style.response.labelled_by(style_label.id);
            let width_label = ui.label(tl!("Width:"));
            let width = egui::ComboBox::from_id_salt("tstyle_pen_width")
                .selected_text(crate::i18n::fmt(tl!("{n} pt"), &[("n", &pen.width.to_string())]))
                .width(70.0)
                .show_ui(ui, |ui| {
                    for w in WIDTHS {
                        ui.selectable_value(&mut pen.width, w, crate::i18n::fmt(tl!("{n} pt"), &[("n", &w.to_string())]));
                    }
                });
            width.response.labelled_by(width_label.id);
            ui.push_id("tstyle_pen_color", |ui| {
                ui.label(tl!("Color:"));
                color_menu(ui, &theme, &mut pen.color);
            });
            ui.menu_button(format!("{} ▾", tl!("Borders")), |ui| {
                for (label, edges) in EDGES {
                    ui.horizontal(|ui| {
                        edge_glyph(ui, edges);
                        if ui.selectable_label(edges_on(&r.borders, edges), tl!(label)).clicked() {
                            pick_edges(&mut r.borders, edges, pen);
                            ui.close();
                        }
                    });
                }
            });
        });
        ui.horizontal(|ui| {
            ui.push_id("tstyle_fill", |ui| {
                ui.label(tl!("Fill color:"));
                color_menu(ui, &theme, &mut r.fill);
            });
            ui.add_space(12.0);
            ui.label(tl!("Alignment:"));
            egui::Grid::new("tstyle_align").spacing(vec2(1.0, 1.0)).show(ui, |ui| {
                for (i, ((n, v, h), tip)) in CELL_ALIGNS.iter().zip(ALIGN_TIPS).enumerate() {
                    let on = r.align == *n;
                    if align_button(ui, (*v, *h), on, tl!(tip)).clicked() {
                        r.align = if on { String::new() } else { n.to_string() };
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        });
    }
    // Live preview: the style as OK would leave it, on the current table's options with the
    // part being edited turned on.
    let v = table_style_params(id.as_deref(), name, based_on, regions, basis);
    let look = app
        .session
        .sel
        .focus
        .path
        .cell()
        .and_then(|(tp, _, _)| app.session.doc.table(app.session.sel.focus.story, &tp).map(|t| t.props.look))
        .unwrap_or_default();
    let look = look_for(look, PARTS.get(*region).map_or(TablePart::WholeTable, |(p, _)| *p));
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tl!("Preview")).small().weak());
    table_style_preview(ui, wordcraft_engine::cmd::table_style::preview(&app.session, &v).as_ref(), look);
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok {
        let cmd = if id.is_some() { "table.modifyStyle" } else { "table.newStyle" };
        if let Err(e) = app.run(cmd, v) {
            app.status(e);
            return false;
        }
    }
    ok || cancel
}

/// The border styles the pen offers (the Borders and Shading dialog's list).
const LINE_STYLES: [(BorderStyle, &str); 8] = [
    (BorderStyle::Single, "Single"),
    (BorderStyle::Dotted, "Dotted"),
    (BorderStyle::Dashed, "Dashed"),
    (BorderStyle::DotDash, "Dot dash"),
    (BorderStyle::Double, "Double"),
    (BorderStyle::Triple, "Triple"),
    (BorderStyle::Thick, "Thick"),
    (BorderStyle::Wave, "Wave"),
];

const WIDTHS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0];

/// The dialog for `name` (`newTableStyle` / `modifyTableStyle`), `None` when there's no table
/// style to modify.
pub fn open(app: &WordApp, name: &str) -> Option<Dialog> {
    let current = current_table_style(app);
    let (id, name, based_on, regions) = if name == "newTableStyle" {
        let based_on = current.unwrap_or_else(|| "TableGrid".into());
        let styles = &app.session.doc.styles;
        let name = (1..1000).map(|i| format!("Table Style {i}")).find(|n| styles.find(n).is_none()).unwrap_or_default();
        let regions = table_regions(app, &based_on);
        (None, name, based_on, regions)
    } else {
        let id = current?;
        let st = app.session.doc.styles.get(&id)?;
        let (name, based_on) = (st.name.clone(), st.based_on.clone().unwrap_or_default());
        let regions = table_regions(app, &id);
        (Some(id), name, based_on, regions)
    };
    Some(Dialog::TableStyle { id, name, based_on, region: 0, basis: regions.clone(), regions, pen: BorderPen::default() })
}

/// The style of the table at the caret, when it is a table style.
fn current_table_style(app: &WordApp) -> Option<String> {
    let s = &app.session;
    let (tp, _, _) = s.sel.focus.path.cell()?;
    let id = s.doc.table(s.sel.focus.story, &tp)?.props.style.clone()?;
    s.doc.styles.get(&id).filter(|st| st.kind == wordcraft_doc::StyleKind::Table).map(|st| st.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use wordcraft_engine::Session;

    fn steps(h: &mut egui_kittest::Harness<'_, WordApp>) {
        for _ in 0..4 {
            h.step();
        }
    }

    /// #714: picking a part under "Apply formatting to" and changing it edits that part (here the
    /// even banded rows) and no other.
    #[test]
    fn the_part_picker_edits_the_part_it_shows() {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("insert.table", json!({"rows": 4, "cols": 3})).unwrap();
        app.run("table.newStyle", json!({"name": "Parts"})).unwrap();
        app.dialog = Dialog::open("modifyTableStyle", &mut app);
        assert!(matches!(app.dialog, Some(Dialog::TableStyle { .. })));
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        steps(&mut h);
        // The picker is a combo box showing the part being edited.
        h.get_by(|n| n.role() == egui::accesskit::Role::ComboBox && n.value().as_deref() == Some("Whole Table")).click();
        steps(&mut h);
        h.get_by_label("Even Banded Rows").click();
        steps(&mut h);
        assert!(matches!(&h.state().dialog, Some(Dialog::TableStyle { region, .. }) if *region == part_index(TablePart::Band2Horz)));
        h.get_by_label("B").click();
        steps(&mut h);
        h.get_by_label("OK").click();
        steps(&mut h);
        assert!(h.state().dialog.is_none(), "OK closes the dialog");
        let st = h.state().session.doc.styles.get("Parts").cloned().unwrap();
        assert_eq!(st.table_part(TablePart::Band2Horz).chr.bold, Some(true));
        for other in [TablePart::WholeTable, TablePart::Band1Horz, TablePart::FirstRow, TablePart::Band2Vert] {
            assert_ne!(st.table_part(other).chr.bold, Some(true), "{}", other.ooxml());
        }
    }

    /// The border picker sets and toggles edges with the pen, and only changed edges are sent.
    #[test]
    fn border_picker_sends_only_changed_edges() {
        let pen = BorderPen { style: BorderStyle::Double, width: 1.5, color: "FF0000".into() };
        let basis = TableRegion::default();
        let mut r = basis.clone();
        pick_edges(&mut r.borders, "outside", &pen);
        pick_edges(&mut r.borders, "left", &pen);
        assert!(edges_on(&r.borders, "top") && !edges_on(&r.borders, "left") && !edges_on(&r.borders, "inside"));
        r.align = "bottomRight".into();
        let v = region_changes(&r, &basis);
        let edges: Vec<&str> = v["borders"].as_array().unwrap().iter().map(|e| e["edges"].as_str().unwrap()).collect();
        // Left was turned off again: an explicit "no line" now, not the inherited one.
        assert_eq!(edges, ["top", "bottom", "left", "right"]);
        assert_eq!(v["borders"][0]["style"], "double");
        assert_eq!(v["borders"][2]["style"], "none");
        assert_eq!(v["cellAlign"], "bottomRight");
        assert_eq!(align_name(Some(Align::Center), Some(VAlign::Bottom)), "bottomCenter");
        assert_eq!(align_name(None, None), "");
    }

    /// #1288: "No Border" on a part that inherits its lines (the header row of a style based on
    /// Table Grid) sends explicit "no line" edges, and the header row draws no lines.
    #[test]
    fn no_border_hides_inherited_lines() {
        use wordcraft_layout::Placed;
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("insert.table", json!({"rows": 2, "cols": 3})).unwrap();
        app.run("table.look", json!({"headerRow": true})).unwrap();
        app.run("table.newStyle", json!({"name": "Parts", "basedOn": "Table Grid"})).unwrap();
        // The table's horizontal rules, by y.
        let rules = |app: &mut WordApp| -> Vec<f32> {
            app.session.layout().pages[0]
                .items
                .iter()
                .filter_map(|i| match i {
                    Placed::Rule { y0, y1, border, .. } if border.is_visible() && (y0 - y1).abs() < 0.01 => Some(*y0),
                    _ => None,
                })
                .collect()
        };
        let before = rules(&mut app);
        let top = before.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(top.is_finite(), "Table Grid draws the table's top edge");
        app.dialog = Dialog::open("modifyTableStyle", &mut app);
        let mut r = table_regions(&app, "Parts")[part_index(TablePart::FirstRow)].clone();
        assert!(!edges_on(&r.borders, "none"), "inherited lines aren't \"No Border\"");
        pick_edges(&mut r.borders, "none", &BorderPen::default());
        assert!(edges_on(&r.borders, "none"));
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        steps(&mut h);
        h.get_by(|n| n.role() == egui::accesskit::Role::ComboBox && n.value().as_deref() == Some("Whole Table")).click();
        steps(&mut h);
        h.get_by_label("Header Row").click();
        steps(&mut h);
        h.get_by_label("Borders ▾").click();
        steps(&mut h);
        h.get_by_label("No Border").click();
        steps(&mut h);
        h.get_by_label("OK").click();
        steps(&mut h);
        assert!(h.state().dialog.is_none(), "OK closes the dialog");
        let head = h.state().session.doc.styles.get("Parts").unwrap().table_part(TablePart::FirstRow);
        let b = head.borders.unwrap();
        for e in [b.top, b.bottom, b.left, b.right, b.between, b.inside_v] {
            assert_eq!(e.map(|x| x.style), Some(BorderStyle::None));
        }
        let after = rules(h.state_mut());
        assert!(after.iter().all(|y| (y - top).abs() > 0.01), "the header row's top edge is gone: {after:?}");
        assert!(after.len() < before.len());
    }
}
