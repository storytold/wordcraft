//! Table Design › New / Modify Table Style (#714): one table style part at a time ("Apply
//! formatting to"), with its font, size, bold, italic, underline and colour, cell shading, borders
//! (line style, width and colour, and which edges) and cell alignment, over a live preview. OK
//! runs `table.newStyle` / `table.modifyStyle` with only what changed.

use craft::behavior::Anchor9;
use craft::color_ui::ui::ColorPickerTab;
use craft::font_manager::picker::FontPickerSettings;
use craft::ui::buttons::{Button, ControlPlace, GroupLevel, ToggleButton};
use craft::ui::contract::{Bind, Entry, Gap, Props as _, Show as _, ShowContainer as _, Sizing, Trigger};
use craft::ui::controls::disclosure::DisclosureGlyph;
use craft::ui::dialog::DialogId;
use craft::ui::display::{Picture, PictureSize, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::Row;
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::menu::Menu;
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::{ComboBox, ReferencePoint, ReferencePointKind, Select};
use craft::ui::popover::Popover;
use craft::ui::primitives::{ContentPainterCx, Glyph, IconSpec};
use craft::ui::runtime::env::Label;
use craft::ui::sections::Section;
use craft::ui::style::{UiTokens, rgba_color};
use craft::ui::text::TextField;
use egui::{Color32, Painter, Rect, Shape, Stroke, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::props::{Align, Border, BorderStyle, Borders, TableLook, Underline, VAlign};
use wordcraft_doc::styles::{PartFormat, PartSet, TablePart, TableStyleProps};
use wordcraft_doc::{CharProps, Rgb, TextColor};
use wordcraft_engine::cmd::table_style::{self, CELL_ALIGNS};

use crate::WordApp;
use crate::colors::{ColorFieldEmpty, Palette, color_field};
use crate::dialogs::host::{Asks, ChildAnswer, WordForm, open_form};
use crate::widgets::font_box::{CatalogRef, font_picker}; // PLAN: assumed CatalogRef / WordApp::font_* (ribbon-w11, S-L284)

/// One table style part in the dialog, as it resolves through the based-on chain: colours are
/// hex, empty for none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

/// The alignment picker's cells in the order of [`CELL_ALIGNS`] (row by row from the top left).
const ANCHORS: [Anchor9; 9] = [
    Anchor9::TopLeft,
    Anchor9::Top,
    Anchor9::TopRight,
    Anchor9::Left,
    Anchor9::Centre,
    Anchor9::Right,
    Anchor9::BottomLeft,
    Anchor9::Bottom,
    Anchor9::BottomRight,
];

/// The alignment labels shown on the closed Alignment button, in [`CELL_ALIGNS`] order.
const ALIGN_LABELS: [&str; 9] = ["Top left", "Top", "Top right", "Left", "Center", "Right", "Bottom left", "Bottom", "Bottom right"];

/// The closed Alignment button's visible label for `align` (`"Auto"` when inherited).
fn align_label(align: &str) -> Label {
    let text = CELL_ALIGNS.iter().position(|(n, _, _)| *n == align).and_then(|i| ALIGN_LABELS.get(i).copied()).unwrap_or("Auto");
    lab(text)
}

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
    let mut v = json!({"name": name.trim()});
    if !based_on.trim().is_empty() {
        v["basedOn"] = json!(based_on.trim());
    }
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

/// A square showing which edges a border choice draws: the drawn edges in the icon colour, the
/// others as faint dotted guides.
#[derive(craft::ui::contract::MemoProps)]
struct EdgeGlyph {
    edges: &'static str,
}

/// One glyph per [`EDGES`] choice, `'static` so a menu entry can borrow it as its icon.
static EDGE_GLYPHS: [EdgeGlyph; 10] = [
    EdgeGlyph { edges: "all" },
    EdgeGlyph { edges: "none" },
    EdgeGlyph { edges: "outside" },
    EdgeGlyph { edges: "inside" },
    EdgeGlyph { edges: "top" },
    EdgeGlyph { edges: "bottom" },
    EdgeGlyph { edges: "left" },
    EdgeGlyph { edges: "right" },
    EdgeGlyph { edges: "insideH" },
    EdgeGlyph { edges: "insideV" },
];

impl Glyph for EdgeGlyph {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, color: Color32, _mirrored: bool) {
        let stroke = Stroke::new(tokens.stroke.medium, color);
        let guide = rgba_color(tokens.color.text.faint);
        let r = rect.shrink(rect.width() * 0.125);
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
            let on = match self.edges {
                "all" => true,
                "none" => false,
                "outside" => matches!(e, "top" | "bottom" | "left" | "right"),
                "inside" => matches!(e, "insideH" | "insideV"),
                x => x == e,
            };
            if on {
                painter.line_segment(seg, stroke);
            } else {
                painter.extend(Shape::dotted_line(&seg, guide, tokens.stroke.medium * 2.0, tokens.stroke.medium * 0.5));
            }
        }
    }
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
fn paint_preview(cx: &ContentPainterCx<'_>, rect: Rect, style: Option<&TableStyleProps>, look: &TableLook) {
    const COLS: usize = 5;
    const ROWS: usize = 5;
    const TEXT: [[&str; COLS]; ROWS] = [
        ["", "Mon", "Tue", "Wed", "Sum"],
        ["North", "4", "7", "2", "13"],
        ["South", "6", "1", "5", "12"],
        ["West", "3", "8", "4", "15"],
        ["Total", "13", "16", "11", "40"],
    ];
    // The page is white whatever the interface theme; the style's colours are data, so they go through
    // `data_shapes` and dim with a disabled picture.
    cx.data_shapes([Shape::rect_filled(rect, 2.0, Color32::WHITE)]);
    cx.painter.rect_stroke(rect, 2.0, Stroke::new(cx.stroke_width(craft::ui::primitives::StrokeWidth::Thin), cx.ink()), egui::StrokeKind::Inside);
    let Some(st) = style else { return };
    let mut shapes: Vec<Shape> = Vec::new();
    let formats: Vec<PartFormat> = TablePart::ALL.iter().map(|p| st.part(*p)).collect();
    let pad_x = (rect.width() * 0.035).clamp(6.0, 14.0);
    let pad_y = (rect.height() * 0.06).clamp(4.0, 10.0);
    let table = rect.shrink2(vec2(pad_x, pad_y));
    let (cw, rh) = (table.width() / COLS as f32, table.height() / ROWS as f32);
    let rgb = |c: Rgb| Color32::from_rgb(c.0, c.1, c.2);
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
            for part in PartSet::of_cell(look, band, (ri, ROWS), (ci, 1, COLS)).iter() {
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
                shapes.push(Shape::rect_filled(cell, 0.0, rgb(f)));
            }
            let mut edge = |from: egui::Pos2, to: egui::Pos2, e: Option<Border>| {
                if let Some(e) = e.filter(Border::is_visible) {
                    let color = e.color.map_or(Color32::BLACK, rgb);
                    let stroke = Stroke::new(e.width.clamp(0.5, 3.0) * 1.3, color);
                    if matches!(e.style, BorderStyle::Dotted | BorderStyle::Dashed | BorderStyle::DotDash) {
                        shapes.extend(Shape::dashed_line(&[from, to], stroke, 3.0, 2.0));
                    } else {
                        shapes.push(Shape::line_segment([from, to], stroke));
                    }
                }
            };
            edge(cell.left_top(), cell.right_top(), b.top);
            edge(cell.left_bottom(), cell.right_bottom(), b.bottom);
            edge(cell.left_top(), cell.left_bottom(), b.left);
            edge(cell.right_top(), cell.right_bottom(), b.right);
            let color = match chr.color {
                Some(TextColor::Rgb(c)) => rgb(c),
                _ => Color32::BLACK,
            };
            let font = egui::FontId::proportional((rh * 0.65).clamp(8.5, 11.0));
            let inset_x = (cw * 0.06).clamp(2.0, 4.0);
            let inset_y = (rh * 0.1).clamp(1.0, 2.0);
            let (ax, x) = match h {
                Some(Align::Center) => (egui::Align::Center, cell.center().x),
                Some(Align::Right) => (egui::Align::Max, cell.right() - inset_x),
                _ => (egui::Align::Min, cell.left() + inset_x),
            };
            let (ay, y) = match v {
                Some(VAlign::Center) => (egui::Align::Center, cell.center().y),
                Some(VAlign::Bottom) => (egui::Align::Max, cell.bottom() - inset_y),
                _ => (egui::Align::Min, cell.top() + inset_y),
            };
            let galley = cx.painter.layout_no_wrap(txt.to_string(), font, color);
            let r = egui::Align2([ax, ay]).anchor_size(pos2(x, y), galley.size());
            if chr.bold.unwrap_or(false) {
                // A second pass a hair to the right reads as bold at this size.
                shapes.push(Shape::galley(r.min + vec2(0.6, 0.0), galley.clone(), color));
            }
            shapes.push(Shape::galley(r.min, galley, color));
            if chr.underline.is_some_and(|u| u != Underline::None) && !txt.is_empty() {
                shapes.push(Shape::line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(0.8, color)));
            }
        }
    }
    cx.data_shapes(shapes);
}

/// A table style the form can be based on, with the parts it resolves to: a new style takes these
/// as its starting point when the base changes, without the form reaching into the session.
#[derive(Clone, Debug, Default, PartialEq)]
struct Base {
    id: String,
    name: String,
    regions: Box<[TableRegion; 13]>,
}

/// The form's label for `text`, translated.
fn lab(text: &str) -> Label {
    tl!(text).to_string().into()
}

/// The dialog's state.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableStyleForm {
    /// The style being modified; `None` creates one.
    id: Option<String>,
    name: String,
    /// Style id.
    based_on: String,
    /// Index into `regions` (see [`PARTS`]).
    region: usize,
    regions: Box<[TableRegion; 13]>,
    /// What the parts showed when the dialog opened (or the base style changed): only changes are
    /// sent, so everything else stays inherited.
    basis: Box<[TableRegion; 13]>,
    /// The line the border choices draw.
    pen: BorderPen,
    /// The table styles this one can be based on, read at open.
    #[serde(skip)]
    bases: Vec<Base>,
    /// The colour lists the colour fields offer, read at open.
    #[serde(skip)]
    palette: Palette,
    /// The installed fonts the font box lists, read at open.
    #[serde(skip)]
    catalog: CatalogRef,
    /// Recent fonts and the picker's remembered state, read at open.
    #[serde(skip)]
    font_settings: FontPickerSettings,
    /// The options of the table at the caret, which the preview shows with the edited part on.
    #[serde(skip)]
    look: TableLook,
    /// The style as OK would leave it, copied in by [`prepare_preview`] before each pass.
    #[serde(skip)]
    preview: Option<TableStyleProps>,
}

impl TableStyleForm {
    /// The params of the command OK runs.
    fn params(&self) -> Value {
        table_style_params(self.id.as_deref(), &self.name, &self.based_on, &self.regions, &self.basis)
    }

    /// The part being edited.
    fn part(&self) -> TablePart {
        PARTS.get(self.region).map_or(TablePart::WholeTable, |(p, _)| *p)
    }

    /// Name and base style.
    fn identity(&mut self, ui: &mut Ui) {
        FieldGroup::new(lab("Table style")).show(ui, |ui, _| {
            let name_label = lab("Name:");
            Field::new(name_label.clone()).show(ui, |ui, _| {
                TextField::new(name_label.clone(), Bind::bound(&mut self.name)).width_chars(30).show(ui);
            });
            let entries: Vec<Entry<usize>> = self.bases.iter().enumerate().map(|(i, b)| Entry::new(i, b.name.clone())).collect();
            let current = self.bases.iter().position(|b| b.id == self.based_on).unwrap_or(usize::MAX);
            if let Some(i) = pick_index(ui, "Style based on:", &entries, current)
                && let Some(base) = self.bases.get(i)
            {
                self.based_on = base.id.clone();
                // A new style shows its new base's formatting; a modified one keeps its own.
                if self.id.is_none() {
                    self.regions = base.regions.clone();
                    self.basis = self.regions.clone();
                }
            }
            let part_entries: Vec<Entry<usize>> = PARTS.iter().enumerate().map(|(i, (_, n))| Entry::new(i, lab(n))).collect();
            if let Some(i) = pick_index(ui, "Apply formatting to:", &part_entries, self.region) {
                self.region = i;
            }
        });
    }

    /// The edited part's font, size, toggles, colours, borders and alignment.
    fn formatting(&mut self, ui: &mut Ui, asks: &mut Vec<Asks>) {
        let Self { region, regions, pen, palette, catalog, font_settings, .. } = self;
        let Some(r) = regions.get_mut(*region) else { return };
        Section::fixed(lab("Formatting")).show(ui, |ui, _| {
            // Row 1: Font, size, and B / I / U toggles.
            Row::new().gap(Gap::Inline).wrap(true).show(ui, |ui, _| {
                let sizes: Vec<Entry<String>> = wordcraft_engine::cmd::format::SIZES
                    .iter()
                    .map(|s| {
                        let s = size_text(*s);
                        Entry::new(s.clone(), s)
                    })
                    .collect();
                if let Some(catalog) = catalog.get() {
                    Row::new().gap(Gap::Inline).show(ui, |ui, _| {
                        let font_label = lab("Font:");
                        Text::new(font_label.clone()).show(ui);
                        craft::ui::runtime::env::with_env(
                            ui,
                            |overrides| {
                                overrides.values.set(ControlPlace::Ribbon);
                            },
                            |ui| {
                                font_picker(font_label, Bind::bound(&mut r.font), catalog, Bind::bound(&mut *font_settings), &[], None).show(ui);
                            },
                        );
                    });
                }
                Row::new().gap(Gap::Inline).show(ui, |ui, _| {
                    let size_label = lab("Size:");
                    Text::new(size_label.clone()).show(ui);
                    craft::ui::runtime::env::with_env(
                        ui,
                        |overrides| {
                            overrides.values.set(ControlPlace::Ribbon);
                        },
                        |ui| {
                            ComboBox::new(size_label, Bind::bound(&mut r.size), &sizes).show(ui);
                        },
                    );
                });

                // Compact icon-only labels inside standard secondary (framed) toggle buttons so B / I / U
                // keep full icon size and visible button outlines.
                craft::ui::runtime::env::with_env(
                    ui,
                    |overrides| {
                        overrides.values.set(GroupLevel::Compact);
                    },
                    |ui| {
                        Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                            ToggleButton::new(lab("Bold"), Bind::bound(&mut r.bold)).icon(crate::icons::id("bold")).tooltip(lab("Bold")).show(ui);
                            ToggleButton::new(lab("Italic"), Bind::bound(&mut r.italic))
                                .icon(crate::icons::id("italic"))
                                .tooltip(lab("Italic"))
                                .show(ui);
                            ToggleButton::new(lab("Underline"), Bind::bound(&mut r.underline))
                                .icon(crate::icons::id("underline"))
                                .tooltip(lab("Underline"))
                                .show(ui);
                        });
                    },
                );
            });

            // Row 2: Font color and Fill color.
            Row::new().gap(Gap::Section).wrap(true).show(ui, |ui, _| {
                Row::new().gap(Gap::Inline).show(ui, |ui, _| {
                    Text::new(lab("Font color:")).show(ui);
                    if color_field(ui, &lab("Font color:"), &mut r.color, &ColorFieldEmpty::NoColor, palette).more_colors {
                        asks.push(Asks::Color { field: "color", current: Rgb::parse(&r.color), tab: ColorPickerTab::Standard });
                    }
                });
                Row::new().gap(Gap::Inline).show(ui, |ui, _| {
                    Text::new(lab("Fill color:")).show(ui);
                    if color_field(ui, &lab("Fill color:"), &mut r.fill, &ColorFieldEmpty::NoColor, palette).more_colors {
                        asks.push(Asks::Color { field: "fill", current: Rgb::parse(&r.fill), tab: ColorPickerTab::Standard });
                    }
                });
            });

            // Row 3: Border pen style, width, and color.
            Row::new().gap(Gap::Inline).wrap(true).show(ui, |ui, _| {
                let styles: Vec<Entry<usize>> = LINE_STYLES.iter().enumerate().map(|(i, (_, n))| Entry::new(i, lab(n))).collect();
                let current = LINE_STYLES.iter().position(|(s, _)| *s == pen.style).unwrap_or(0);
                Row::new().gap(Gap::Inline).show(ui, |ui, _| {
                    Text::new(lab("Style:")).show(ui);
                    if let Some((s, _)) = Select::new(lab("Style:"), Bind::given(current), &styles)
                        .sizing(Sizing::Content)
                        .show(ui)
                        .value()
                        .copied()
                        .and_then(|i| LINE_STYLES.get(i))
                    {
                        pen.style = *s;
                    }
                });

                let widths: Vec<Entry<usize>> =
                    WIDTHS.iter().enumerate().map(|(i, w)| Entry::new(i, crate::i18n::fmt(tl!("{n} pt"), &[("n", &w.to_string())]))).collect();
                let current = WIDTHS.iter().position(|w| *w == pen.width).unwrap_or(usize::MAX);
                Row::new().gap(Gap::Inline).show(ui, |ui, _| {
                    Text::new(lab("Width:")).show(ui);
                    if let Some(w) = Select::new(lab("Width:"), Bind::given(current), &widths)
                        .sizing(Sizing::Content)
                        .show(ui)
                        .value()
                        .copied()
                        .and_then(|i| WIDTHS.get(i))
                    {
                        pen.width = *w;
                    }
                });

                Row::new().gap(Gap::Inline).show(ui, |ui, _| {
                    Text::new(lab("Color:")).show(ui);
                    if color_field(ui, &lab("Color:"), &mut pen.color, &ColorFieldEmpty::NoColor, palette).more_colors {
                        asks.push(Asks::Color { field: "pen", current: Rgb::parse(&pen.color), tab: ColorPickerTab::Standard });
                    }
                });
            });

            // Row 4: Borders menu and compact Alignment dropdown opening the 3×3 alignment grid.
            Row::new().gap(Gap::Section).wrap(true).show(ui, |ui, _| {
                let button = Button::new(lab("Borders:")).painted(lab("Borders")).trailing_icon(DisclosureGlyph::menu_button()).show(ui);
                let entries: Vec<Entry<usize>> = EDGES
                    .iter()
                    .zip(EDGE_GLYPHS.iter())
                    .enumerate()
                    .map(|(i, ((label, edges), glyph))| Entry::new(i, lab(label)).icon(IconSpec::glyph(glyph)).checked(edges_on(&r.borders, edges)))
                    .collect();
                let picked = Menu::new(lab("Borders")).opened_by(&button, Trigger::Press).pick(ui, &entries).picked;
                if let Some((_, edges)) = picked.and_then(|i| EDGES.get(i)) {
                    pick_edges(&mut r.borders, edges, pen);
                }

                Row::new().gap(Gap::Inline).show(ui, |ui, _| {
                    Text::new(lab("Alignment:")).show(ui);
                    let open_id = ui.make_persistent_id("tstyle_align_open");
                    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
                    let align_text = align_label(&r.align);
                    let align_spoken = Label::from(format!("{} {}", tl!("Alignment:"), align_text.as_str()));
                    let button = Button::new(align_spoken)
                        .painted(align_text)
                        .icon(crate::icons::id("cellAlign"))
                        .trailing_icon(DisclosureGlyph::menu_button())
                        .show(ui);
                    if button.clicked() {
                        open = !open;
                    }
                    Popover::new(lab("Alignment"), Bind::bound(&mut open)).opened_by(&button).show(ui, |ui, cx| {
                        // The grid is row by row from the top left, as `CELL_ALIGNS` is.
                        let current = CELL_ALIGNS.iter().position(|(n, _, _)| *n == r.align).and_then(|i| ANCHORS.get(i)).copied();
                        let picked = ReferencePoint::new(lab("Alignment:"), Bind::given(current))
                            .kind(ReferencePointKind::Alignment)
                            .show(ui)
                            .value()
                            .copied();
                        if let Some(anchor) = picked {
                            // Picking the selected cell again clears the alignment.
                            r.align = anchor
                                .and_then(|a| ANCHORS.iter().position(|x| *x == a))
                                .and_then(|i| CELL_ALIGNS.get(i))
                                .map(|(n, _, _)| n.to_string())
                                .unwrap_or_default();
                            cx.request_close();
                        }
                    });
                    ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
                });
            });
        });
    }
}

/// A named `Select` over `entries` in a `Field`: the index picked in this pass.
fn pick_index(ui: &mut Ui, label: &str, entries: &[Entry<usize>], current: usize) -> Option<usize> {
    let label = lab(label);
    Field::new(label.clone()).show(ui, |ui, _| Select::new(label.clone(), Bind::given(current), entries).show(ui).value().copied()).inner.flatten()
}

impl WordForm for TableStyleForm {
    fn key(&self) -> &'static str {
        if self.id.is_none() { "newTableStyle" } else { "modifyTableStyle" }
    }

    fn title(&self) -> Label {
        if self.id.is_none() { lab("New Table Style") } else { lab("Modify Table Style") }
    }

    fn prepare(&mut self, app: &WordApp) {
        // PLAN: assumed `WordForm::prepare(&mut self, &WordApp)` and `Palette::of` (dialogs-w13 S-L283, ribbon-w11 S-L284)
        self.palette = Palette::of(app);
        self.catalog = CatalogRef(Some(app.font_catalog()));
        self.font_settings = app.font_picker_settings();
        // The style as OK would leave it, redone each pass so nothing outlives it.
        self.preview = table_style::preview(&app.session, &self.params());
    }

    fn write_back(&self, app: &mut WordApp) {
        app.set_font_picker_settings(self.font_settings.clone());
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        self.identity(ui);
        self.formatting(ui, asks);
        // Live preview: the style as OK would leave it, on the current table's options with the
        // part being edited turned on.
        let look = look_for(self.look, self.part());
        let preview = self.preview.as_ref();
        Section::fixed(lab("Preview")).show(ui, |ui, _| {
            Picture::painter(lab("Table style preview"), ThumbAspect::new(36, 13), |cx, rect| paint_preview(cx, rect, preview, &look))
                .extent(PictureSize::Sample(ThumbSize::Large))
                .show(ui);
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let cmd = if self.id.is_some() { "table.modifyStyle" } else { "table.newStyle" };
        vec![(cmd.to_string(), self.params())]
    }

    fn answer(&mut self, answer: ChildAnswer) {
        let ChildAnswer::Color { field, hex } = answer else { return };
        match field {
            "color" => {
                if let Some(r) = self.regions.get_mut(self.region) {
                    r.color = hex;
                }
            }
            "fill" => {
                if let Some(r) = self.regions.get_mut(self.region) {
                    r.fill = hex;
                }
            }
            "pen" => self.pen.color = hex,
            _ => {}
        }
    }
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

/// Opens the dialog for `name` (`newTableStyle` / `modifyTableStyle`), `None` when there's no table
/// style to modify. `params["part"]` opens it on that part (`"firstRow"`…).
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let current = current_table_style(app);
    let (id, style_name, based_on, regions) = if name == "newTableStyle" {
        let based_on = current.unwrap_or_else(|| "TableGrid".into());
        let styles = &app.session.doc.styles;
        let style_name = (1..1000).map(|i| format!("Table Style {i}")).find(|n| styles.find(n).is_none()).unwrap_or_default();
        let regions = table_regions(app, &based_on);
        (None, style_name, based_on, regions)
    } else {
        let id = current?;
        let st = app.session.doc.styles.get(&id)?;
        let (style_name, based_on) = (st.name.clone(), st.based_on.clone().unwrap_or_default());
        let regions = table_regions(app, &id);
        (Some(id), style_name, based_on, regions)
    };
    let bases: Vec<Base> = app
        .session
        .doc
        .styles
        .styles
        .iter()
        .filter(|s| s.kind == wordcraft_doc::StyleKind::Table && Some(&s.id) != id.as_ref())
        .map(|s| Base { id: s.id.clone(), name: s.name.clone(), regions: table_regions(app, &s.id) })
        .collect();
    let look = app
        .session
        .sel
        .focus
        .path
        .cell()
        .and_then(|(tp, _, _)| app.session.doc.table(app.session.sel.focus.story, &tp).map(|t| t.props.look))
        .unwrap_or_default();
    let region = params.get("part").and_then(Value::as_str).and_then(table_style::part_named).map_or(0, part_index);
    let form = TableStyleForm {
        id,
        name: style_name,
        based_on,
        region,
        basis: regions.clone(),
        regions,
        pen: BorderPen::default(),
        bases,
        look,
        ..Default::default()
    };
    Some(open_form(app, parent, form))
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

    /// The app, drawn whole, with the dialog host running after its UI.
    fn harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        crate::test_harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        )
    }

    /// Alignment choices expose their names and current selection, and can be toggled through the dialog.
    #[test]
    fn alignment_buttons_expose_names_selection_and_clicks() {
        use crate::dialogs::host::FormHost;
        use egui::accesskit::{Role, Toggled};
        use egui_kittest::kittest::NodeT;

        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("insert.table", json!({"rows": 4, "cols": 3})).unwrap();
        app.run("table.newStyle", json!({"name": "AlignTest"})).unwrap();
        let dialog_id = crate::dialogs::open(&mut app, "modifyTableStyle", &json!({})).expect("dialog opened");
        // Keep the background ribbon's character controls out of the dialog's label queries.
        app.ui.tab = "Insert".into();
        let mut h = harness(app);
        steps(&mut h);

        // Open the Alignment popover, verify labels and unchecked state, then pick "Bottom right".
        h.get_by_role_and_label(Role::Button, "Alignment: Auto").click();
        steps(&mut h);
        assert_eq!(h.get_by_role_and_label(Role::RadioButton, "Bottom right").accesskit_node().toggled(), Some(Toggled::False));
        assert!(!h.get_by_role_and_label(Role::RadioButton, "Center").accesskit_node().is_disabled());

        // Click "Bottom right", check align == "bottomRight" and button exposes the value.
        h.get_by_role_and_label(Role::RadioButton, "Bottom right").click();
        steps(&mut h);
        {
            let host = h.state().dialog_host.get_by_id::<FormHost<TableStyleForm>>(dialog_id).expect("form is hosted");
            let r = &host.form().regions[host.form().region];
            assert_eq!(r.align, "bottomRight");
        }

        // Reopen Alignment via its updated accessible name, verify "Bottom right" is checked, click again to clear it.
        h.get_by_role_and_label(Role::Button, "Alignment: Bottom right").click();
        steps(&mut h);
        assert_eq!(h.get_by_role_and_label(Role::RadioButton, "Bottom right").accesskit_node().toggled(), Some(Toggled::True));
        h.get_by_role_and_label(Role::RadioButton, "Bottom right").click();
        steps(&mut h);
        {
            let host = h.state().dialog_host.get_by_id::<FormHost<TableStyleForm>>(dialog_id).expect("form is hosted");
            let r = &host.form().regions[host.form().region];
            assert_eq!(r.align, "");
        }
        assert!(h.query_by_role_and_label(Role::Button, "Alignment: Auto").is_some());
    }

    /// #7692: empty or whitespace basedOn must be omitted so build_modified does not reject unbased styles.
    #[test]
    fn table_style_params_omits_empty_based_on() {
        let regions: [TableRegion; 13] = std::array::from_fn(|_| TableRegion::default());
        let basis: [TableRegion; 13] = std::array::from_fn(|_| TableRegion::default());
        let params = table_style_params(Some("tableGrid"), "Table Grid", "", &regions, &basis);
        assert!(!params.as_object().unwrap().contains_key("basedOn"));

        let params_whitespace = table_style_params(Some("tableGrid"), "Table Grid", "   ", &regions, &basis);
        assert!(!params_whitespace.as_object().unwrap().contains_key("basedOn"));

        let params_with_base = table_style_params(None, "New Style", "tableGrid", &regions, &basis);
        assert_eq!(params_with_base.get("basedOn").and_then(|v| v.as_str()), Some("tableGrid"));
    }

    /// Choosing the document default font or size on a sub-part after changing Whole Table must
    /// still emit that part's explicit override instead of treating the default as unchanged.
    #[test]
    fn choosing_default_font_and_size_on_sub_part_overrides_whole_table() {
        use crate::dialogs::host::FormHost;
        use egui::accesskit::Role;
        use wordcraft_doc::CharProps;

        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
        let default_chr = app.session.doc.styles.resolve_char(None, &CharProps::default());
        let default_size = size_text(default_chr.size);
        let other_size = if (default_chr.size - 9.0).abs() < 0.1 { "14".to_string() } else { "9".to_string() };
        let other_font = "Georgia".to_string();

        let dialog_id = crate::dialogs::open(&mut app, "modifyTableStyle", &json!({})).expect("dialog opened");
        app.ui.tab = "Insert".into();
        let mut h = harness(app);
        steps(&mut h);

        // Set Whole Table font and size to non-default values, then switch to Header Row and pick
        // the document's own default font and size explicitly.
        {
            let host = h.state_mut().dialog_host.get_mut_by_id::<FormHost<TableStyleForm>>(dialog_id).expect("form is hosted");
            host.form_mut().regions[part_index(TablePart::WholeTable)].font = other_font.clone();
            host.form_mut().regions[part_index(TablePart::WholeTable)].size = other_size.clone();
        }
        steps(&mut h);
        h.get_by_label("Apply formatting to:").click();
        steps(&mut h);
        h.get_by_label("Header Row").click();
        steps(&mut h);
        h.get_by_role_and_label(Role::ComboBox, "Size:").click();
        steps(&mut h);
        for _ in 0..default_size.len().max(4) {
            h.key_press(egui::Key::Backspace);
        }
        h.event(egui::Event::Text(default_size));
        h.key_press(egui::Key::Enter);
        steps(&mut h);
        {
            let host = h.state_mut().dialog_host.get_mut_by_id::<FormHost<TableStyleForm>>(dialog_id).expect("form is hosted");
            host.form_mut().regions[part_index(TablePart::FirstRow)].font = default_chr.font.clone();
        }
        steps(&mut h);

        h.get_by_label("OK").click();
        steps(&mut h);
        assert!(h.state().dialog_host.open_dialogs().is_empty(), "OK closes the dialog");

        let st = h.state().session.doc.styles.get("TableGrid").cloned().unwrap();
        assert_eq!(st.table_part(TablePart::WholeTable).chr.font.as_deref(), Some(other_font.as_str()));
        assert_eq!(st.table_part(TablePart::FirstRow).chr.font.as_deref(), Some(default_chr.font.as_str()));
        assert_eq!(st.table_part(TablePart::FirstRow).chr.size, Some(default_chr.size));
    }

    /// #7692: modifying the default Table Grid style (which has no base style) sends no empty basedOn
    /// and OK closes successfully without a blank preview or command rejection.
    #[test]
    fn modify_table_grid_style_omits_empty_based_on_and_does_not_fail() {
        use crate::dialogs::host::FormHost;

        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
        let dialog_id = crate::dialogs::open(&mut app, "modifyTableStyle", &json!({})).expect("dialog opened");
        app.ui.tab = "Insert".into();
        let mut h = harness(app);
        steps(&mut h);
        {
            let host = h.state().dialog_host.get_by_id::<FormHost<TableStyleForm>>(dialog_id).expect("form is hosted");
            assert!(host.form().based_on.is_empty(), "Table Grid has no base style");
            assert!(host.form().preview.is_some(), "Table Grid preview must build without a base style");
            let r = &host.form().regions[host.form().region];
            assert!(r.font.is_empty(), "unset font stays empty (inherited) until edited");
            assert!(r.size.is_empty(), "unset size stays empty (inherited) until edited");
            let params = host.form().params();
            let obj = params.as_object().unwrap();
            assert!(!obj.contains_key("basedOn"), "empty basedOn must be omitted");
            assert!(!obj.contains_key("wholeTable"), "untouched style sends no region changes");
        }
        {
            use egui::accesskit::Role;

            let dialog = h.get_by_role_and_label(Role::Dialog, "Modify Table Style").rect();
            assert!(h.query_by_role_and_label(Role::Heading, "Formatting").is_some(), "Formatting section heading is exposed");
            assert!(h.query_by_role_and_label(Role::Label, "Font:").is_some(), "Font visible label is exposed");
            assert!(h.query_by_role_and_label(Role::Label, "Size:").is_some(), "Size visible label is exposed");
            let font = h.get_by_role_and_label(Role::ComboBox, "Font:").rect();
            let size = h.get_by_role_and_label(Role::ComboBox, "Size:").rect();
            let underline = h.get_by_role_and_label(Role::Button, "Underline").rect();
            let font_color = h.query_all_by_label("Font color: No Color").next().expect("font color field").rect();
            let fill = h.query_all_by_label("Fill color: No Color").next().expect("fill color field").rect();
            let borders = h.get_by_role_and_label(Role::Button, "Borders:").rect();
            let align = h.get_by_role_and_label(Role::Button, "Alignment: Auto").rect();
            assert!(size.width() < font.width(), "Size box should be narrower than Font box: size={size:?} font={font:?}");
            for (name, rect) in
                [("Underline", underline), ("Font color", font_color), ("Fill color", fill), ("Borders", borders), ("Alignment", align)]
            {
                assert!(rect.right() <= dialog.right(), "{name} must fit inside the dialog without overflowing: rect={rect:?} dialog={dialog:?}");
            }
            assert!(
                (borders.center().y - align.center().y).abs() < (borders.height() + align.height()) * 0.25
                    && (borders.height() - align.height()).abs() < borders.height() * 0.25,
                "Borders and Alignment should line up compactly on one row: borders={borders:?} align={align:?}"
            );
            assert!(
                (font_color.center().y - fill.center().y).abs() < (font_color.height() + fill.height()) * 0.25,
                "Font color and Fill color should share one row: font_color={font_color:?} fill={fill:?}"
            );
            let preview = h.get_by_role_and_label(Role::Image, "Table style preview").rect();
            let controls_height = align.bottom() - font.top();
            assert!(
                preview.height() > font.height() * 2.0 && preview.height() < controls_height && preview.width() > preview.height() * 2.0,
                "preview should be a landscape compact sample taller than a control row and shorter than the formatting controls: preview={preview:?} font={font:?} controls_height={controls_height}"
            );
            assert!(
                preview.right() <= dialog.right() && preview.bottom() <= dialog.bottom(),
                "preview must stay inside the dialog: preview={preview:?} dialog={dialog:?}"
            );
        }
        h.get_by_label("OK").click();
        steps(&mut h);
        assert!(h.state().dialog_host.open_dialogs().is_empty(), "OK closes the dialog");
    }

    /// #714: picking a part under "Apply formatting to" and changing it edits that part (here the
    /// even banded rows) and no other.
    #[test]
    fn the_part_picker_edits_the_part_it_shows() {
        use egui::accesskit::Toggled;
        use egui_kittest::kittest::NodeT;
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("insert.table", json!({"rows": 4, "cols": 3})).unwrap();
        app.run("table.newStyle", json!({"name": "Parts"})).unwrap();
        assert!(crate::dialogs::open(&mut app, "modifyTableStyle", &json!({})).is_some());
        // Keep the background ribbon's character controls out of the dialog's label queries.
        app.ui.tab = "Insert".into();
        let mut h = harness(app);
        steps(&mut h);
        h.get_by_label("Apply formatting to:").click();
        steps(&mut h);
        h.get_by_label("Even Banded Rows").click();
        steps(&mut h);
        for label in ["Bold", "Italic", "Underline"] {
            // An off toggle button reports no toggled state; only an on one says `True`.
            assert_ne!(
                h.get_by_role_and_label(egui::accesskit::Role::Button, label).accesskit_node().toggled(),
                Some(Toggled::True),
                "{label} starts off"
            );
            h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
            steps(&mut h);
            assert_eq!(h.get_by_role_and_label(egui::accesskit::Role::Button, label).accesskit_node().toggled(), Some(Toggled::True));
        }
        h.get_by_label("OK").click();
        steps(&mut h);
        assert!(h.state().dialog_host.open_dialogs().is_empty(), "OK closes the dialog");
        let st = h.state().session.doc.styles.get("Parts").cloned().unwrap();
        let chr = st.table_part(TablePart::Band2Horz).chr;
        assert_eq!(chr.bold, Some(true));
        assert_eq!(chr.italic, Some(true));
        assert!(chr.underline.is_some_and(|u| u != Underline::None));
        for other in [TablePart::WholeTable, TablePart::Band1Horz, TablePart::FirstRow, TablePart::Band2Vert] {
            let chr = st.table_part(other).chr;
            assert_ne!(chr.bold, Some(true), "{}", other.ooxml());
            assert_ne!(chr.italic, Some(true), "{}", other.ooxml());
            assert!(!chr.underline.is_some_and(|u| u != Underline::None), "{}", other.ooxml());
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
        let r = table_regions(&app, "Parts")[part_index(TablePart::FirstRow)].clone();
        assert!(!edges_on(&r.borders, "none"), "inherited lines aren't \"No Border\"");
        let mut picked = r;
        pick_edges(&mut picked.borders, "none", &BorderPen::default());
        assert!(edges_on(&picked.borders, "none"));
        assert!(crate::dialogs::open(&mut app, "modifyTableStyle", &json!({})).is_some());
        app.ui.tab = "Insert".into();
        let mut h = harness(app);
        steps(&mut h);
        h.get_by_label("Apply formatting to:").click();
        steps(&mut h);
        h.get_by_label("Header Row").click();
        steps(&mut h);
        // The Borders: field names its button by the field's label.
        h.get_by_label("Borders:").click();
        steps(&mut h);
        h.get_by_label("No Border").click();
        steps(&mut h);
        h.get_by_label("OK").click();
        steps(&mut h);
        assert!(h.state().dialog_host.open_dialogs().is_empty(), "OK closes the dialog");
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
