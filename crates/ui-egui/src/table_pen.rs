//! Draw Table and Eraser as canvas modes (Insert › Tables, Table Layout › Draw). The buttons
//! toggle a pen or an eraser; while one is on, presses on the page draw (or erase) instead of
//! moving the caret, a preview follows the pointer, and each finished stroke or click runs
//! `table.draw` / `table.eraser` with page coordinates (one undo step each). Esc or the button
//! again turns it off. The geometry decisions are the engine's (`cmd::table_draw`).

use craft::behavior::kurbo::Point;
use craft::behavior::{CancelReason, Cursor, IconId, PaintedCursor, PointerGlyph};
use craft::pages::ui::{PageBody, PageRegion};
use craft::ui::canvas::{CanvasTool, ToolAction, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::primitives::{Draw, PaintShape, StrokeColor, StrokeWidth};
use egui::Pos2;
use serde_json::{Value, json};
use wordcraft_engine::cmd::table_draw::{self, Stroke as Kind};
use wordcraft_layout::DocLayout;

use crate::WordApp;
use crate::ink::{page_under, page_xy, placed};

/// Which table tool is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableTool {
    Draw,
    Erase,
}

impl TableTool {
    fn id(self) -> &'static str {
        match self {
            TableTool::Draw => "table.draw",
            TableTool::Erase => "table.eraser",
        }
    }
}

/// Table Design › Borders › Line Style: the styles the pen offers, as `(label, style)`.
pub const LINE_STYLES: [(&str, &str); 8] = [
    ("Single", "single"),
    ("Dotted", "dotted"),
    ("Dashed", "dashed"),
    ("Dot Dash", "dotDash"),
    ("Double", "double"),
    ("Triple", "triple"),
    ("Thick", "thick"),
    ("Wave", "wave"),
];

/// Table Design › Borders › Line Weight: Word's weights, in points.
pub const LINE_WEIGHTS: [f32; 9] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0];

/// The pen of Table Design › Borders (Line Style, Line Weight, Pen Color, #4589): what Draw
/// Table, the Borders menu and Border Painter draw with. It belongs to the window, not the
/// document, like the colours last applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    pub style: &'static str,
    pub width: f32,
    /// `RRGGBB`, or `None` for Automatic.
    pub color: Option<String>,
}

impl Default for Pen {
    /// Word's: a single ½ pt line in the automatic colour.
    fn default() -> Self {
        Pen { style: "single", width: 0.5, color: None }
    }
}

impl Pen {
    /// The pen as the params `table.draw`, `table.borders` and `table.borderPainter` take.
    pub fn params(&self) -> Value {
        json!({"style": self.style, "width": self.width, "color": self.color})
    }

    /// The weight as Word lists it (½ pt, 2¼ pt…).
    pub fn weight_label(width: f32) -> String {
        let whole = width.trunc();
        let frac = match ((width - whole) * 4.0).round() as u8 {
            1 => "\u{bc}",
            2 => "\u{bd}",
            3 => "\u{be}",
            _ => "",
        };
        let n = match (whole as u32, frac) {
            (0, "") => "0".to_string(),
            (0, f) => f.to_string(),
            (w, f) => format!("{w}{f}"),
        };
        crate::i18n::fmt(tl!("{n} pt"), &[("n", &n)])
    }
}

/// `ui.tablePen {"style"?, "width"?, "color"?}`: set the pen (`color` "auto" or null is
/// Automatic). A style or colour it doesn't know, or a width that isn't a number, is refused
/// and the pen stays as it was. Returns the pen.
pub fn set_pen(app: &mut WordApp, p: &Value) -> Result<Value, String> {
    let mut pen = app.canvas.table_pen.clone();
    if let Some(v) = p.get("style").filter(|v| !v.is_null()) {
        let name = v.as_str().unwrap_or_default();
        pen.style =
            LINE_STYLES.iter().find(|(_, s)| s.eq_ignore_ascii_case(name)).map(|(_, s)| *s).ok_or_else(|| format!("unknown line style `{name}`"))?;
    }
    if let Some(v) = p.get("width").filter(|v| !v.is_null()) {
        let w = v.as_f64().filter(|w| w.is_finite()).ok_or("`width` must be a number of points")?;
        pen.width = (w as f32).clamp(0.25, 6.0);
    }
    if let Some(v) = p.get("color") {
        pen.color = match v {
            Value::Null => None,
            Value::String(c) if c == "auto" => None,
            Value::String(c) => Some(wordcraft_doc::Rgb::parse(c).map(|c| c.hex()).ok_or_else(|| format!("unknown colour `{c}`"))?),
            other => return Err(format!("`color` must be \"RRGGBB\", \"auto\" or null, not {other}")),
        };
    }
    app.canvas.table_pen = pen;
    Ok(app.canvas.table_pen.params())
}

/// `table.draw`'s params for a finished stroke, drawn with `pen`.
fn draw_params(pen: &Pen, st: &PenStroke) -> Value {
    let mut params = pen.params();
    for (k, v) in [("page", json!(st.page)), ("x0", json!(st.from.0)), ("y0", json!(st.from.1)), ("x1", json!(st.to.0)), ("y1", json!(st.to.1))] {
        params[k] = v;
    }
    params
}

/// A stroke in progress: its page and its start and current end (page points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PenStroke {
    page: usize,
    from: (f32, f32),
    to: (f32, f32),
}

/// The ribbon buttons (no page coordinates): turn the tool on, or off when it already is.
/// `{"on": bool}` sets it. With coordinates the command goes to the engine instead (`None`).
pub fn toggle(app: &mut WordApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let tool = match id {
        "table.draw" => TableTool::Draw,
        "table.eraser" => TableTool::Erase,
        _ => return None,
    };
    if p.get("page").is_some() {
        return None;
    }
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(app.canvas.table_tool != Some(tool));
    app.canvas.table_tool = on.then_some(tool);
    app.canvas.table_stroke = None;
    if on {
        // PLAN: assumed `CanvasState::focus_page(&mut self, &egui::Context)` (cv.A10, canvas-w9).
        if let Some(ctx) = app.ctx.clone() {
            app.canvas.focus_page(&ctx);
        }
        app.status(match tool {
            TableTool::Draw => tl!("Drag a rectangle to draw a table, or a line across cells to split them. Esc stops."),
            TableTool::Erase => tl!("Click a border between two cells to merge them. Esc stops."),
        });
    }
    Some(Ok(json!({"tool": app.canvas.table_tool.map(TableTool::id)})))
}

/// Turn the tool off (Esc).
pub fn stop(app: &mut WordApp) -> bool {
    let was = app.canvas.table_tool.is_some();
    app.canvas.table_tool = None;
    app.canvas.table_stroke = None;
    was
}

/// Draw Table or the table eraser as the canvas's tool for one pass. The stroke in progress lives
/// in `app.canvas.table_stroke`.
pub(crate) struct TablePenCanvasTool<'a> {
    app: &'a mut WordApp,
    body: &'a PageBody<'a>,
    /// The layout of this pass, for the preview's geometry.
    layout: Option<std::sync::Arc<DocLayout>>,
}

/// The table tool for this pass while Draw Table or the eraser is on; `body` is this pass's page body.
pub(crate) fn active_tool<'a>(app: &'a mut WordApp, body: &'a PageBody<'a>) -> Option<TablePenCanvasTool<'a>> {
    app.canvas.table_tool?;
    Some(TablePenCanvasTool { app, body, layout: None })
}

/// The areas where a table tool takes a press over every canvas control: every page in full while Draw
/// Table or the eraser is on, so a stroke starts over a content-control button too; none otherwise.
pub(crate) fn regions(app: &WordApp, layout: &DocLayout) -> Vec<PageRegion> {
    if app.canvas.table_tool.is_none() {
        return Vec::new();
    }
    // The whole page, as a polygon in the page's own points from its top left corner.
    layout
        .pages
        .iter()
        .enumerate()
        .map(|(i, pg)| {
            let (w, h) = (f64::from(pg.w), f64::from(pg.h));
            PageRegion::polygon(i, vec![Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)])
        })
        .collect()
}

impl CanvasTool for TablePenCanvasTool<'_> {
    fn each_pass(&mut self, _cx: &mut ToolContext<'_>) {
        self.layout = Some(self.app.session.layout());
    }

    fn press(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(pg) = page_under(self.body.pages, pointer.pos) else { return ToolAction::Ignored };
        let [x, y] = page_xy(pg, pointer.pos);
        self.app.canvas.table_stroke = Some(PenStroke { page: pg.page, from: (x, y), to: (x, y) });
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        let pages = self.body.pages;
        if let Some(st) = self.app.canvas.table_stroke.as_mut()
            && let Some(pg) = placed(pages, st.page)
        {
            let [x, y] = page_xy(pg, pointer.pos);
            st.to = (x, y);
        }
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        self.drag(pointer, cx);
        let Some(st) = self.app.canvas.table_stroke.take() else { return };
        let Some(tool) = self.app.canvas.table_tool else { return };
        let params = match tool {
            TableTool::Draw => {
                // A click without a drag draws nothing.
                if (st.to.0 - st.from.0).abs() < 2.0 && (st.to.1 - st.from.1).abs() < 2.0 {
                    return;
                }
                draw_params(&self.app.canvas.table_pen, &st)
            }
            TableTool::Erase => json!({"page": st.page, "x": st.to.0, "y": st.to.1}),
        };
        if let Err(e) = self.app.run(tool.id(), params) {
            self.app.status(e);
        }
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.app.canvas.table_stroke = None;
    }

    fn key(&mut self, key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        matches!(key, ToolKey::Cancel) && stop(self.app)
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        use craft::theme::variant::Tone;
        let (Some(layout), Some(style), Some(tool)) = (&self.layout, cx.style(), self.app.canvas.table_tool) else { return };
        let accent = StrokeColor::Theme(style.tone(Tone::Accent));
        let screen =
            |page: usize, x: f32, y: f32| -> Option<Pos2> { Some(placed(self.body.pages, page)?.to_screen(Point::new(f64::from(x), f64::from(y)))) };
        let line = |a: Option<Pos2>, b: Option<Pos2>, width: StrokeWidth, color: StrokeColor| {
            if let (Some(a), Some(b)) = (a, b) {
                draw.stroke(&PaintShape::Path(vec![a, b]), width, color);
            }
        };
        match (tool, self.app.canvas.table_stroke) {
            (TableTool::Draw, Some(st)) => match table_draw::stroke(layout, st.page, st.from.0, st.from.1, st.to.0, st.to.1) {
                Some(Kind::Rect(r)) => {
                    let corners = [(r.x, r.y), (r.right(), r.y), (r.right(), r.bottom()), (r.x, r.bottom()), (r.x, r.y)];
                    let pts: Option<Vec<Pos2>> = corners.iter().map(|(x, y)| screen(st.page, *x, *y)).collect();
                    if let Some(pts) = pts {
                        draw.stroke(&PaintShape::Path(pts), StrokeWidth::Medium, accent);
                    }
                }
                Some(Kind::Vertical { x, y0, y1 }) => line(screen(st.page, x, y0), screen(st.page, x, y1), StrokeWidth::Medium, accent),
                Some(Kind::Horizontal { y, x0, x1 }) => line(screen(st.page, x0, y), screen(st.page, x1, y), StrokeWidth::Medium, accent),
                // Not a table or a line yet: the drag's box, dashed and dim.
                None => {
                    let corners = [(st.from.0, st.from.1), (st.to.0, st.from.1), (st.to.0, st.to.1), (st.from.0, st.to.1), (st.from.0, st.from.1)];
                    let pts: Option<Vec<Pos2>> = corners.iter().map(|(x, y)| screen(st.page, *x, *y)).collect();
                    if let Some(pts) = pts {
                        draw.dashed(&pts, draw.dashes(), StrokeWidth::Thin, StrokeColor::Theme(style.tone(Tone::Neutral)));
                    }
                }
            },
            (TableTool::Erase, _) => {
                if let Some(p) = cx.pointer.map(|p| p.pos)
                    && let Some(pg) = page_under(self.body.pages, p)
                {
                    let [x, y] = page_xy(pg, p);
                    if let Some(e) = table_draw::erase_target(layout, pg.page, x, y) {
                        line(
                            screen(pg.page, e.from.0, e.from.1),
                            screen(pg.page, e.to.0, e.to.1),
                            StrokeWidth::Heavy,
                            StrokeColor::Theme(style.tone(Tone::Bad)),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// A crosshair with the tool's icon riding beside it.
    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        let icon =
            if self.app.canvas.table_tool == Some(TableTool::Erase) { IconId::new("wordcraft.eraser") } else { IconId::new("wordcraft.pencil") };
        Cursor::Painted(PaintedCursor::pointer_tool(PointerGlyph::Crosshair, icon))
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the table.draw and table.eraser commands")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deleting a table or removing its last cells turns off the eraser.
    #[test]
    fn deleting_table_turns_off_the_eraser() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        a.run("insert.table", json!({"rows": 2, "cols": 2})).unwrap();
        a.run("table.eraser", json!({"on": true})).unwrap();
        assert_eq!(a.canvas.table_tool, Some(TableTool::Erase));
        a.run("table.deleteTable", json!({})).unwrap();
        assert_eq!(a.canvas.table_tool, None);

        // Deleting the only row also removes the table and stops the eraser.
        a.run("insert.table", json!({"rows": 1, "cols": 1})).unwrap();
        a.run("table.eraser", json!({"on": true})).unwrap();
        assert_eq!(a.canvas.table_tool, Some(TableTool::Erase));
        a.run("table.deleteRow", json!({})).unwrap();
        assert_eq!(a.canvas.table_tool, None);
    }

    /// #4589: Line Style, Line Weight and Pen Color set the pen (`ui.tablePen`); junk is refused
    /// and leaves it as it was; a stroke draws with it; it outlasts a switch to another document.
    #[test]
    fn the_pen_is_set_from_the_ribbon_and_strokes_draw_with_it() {
        let mut a = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        assert_eq!(a.canvas.table_pen, Pen::default());
        a.run("ui.tablePen", json!({"style": "double", "width": 2.25})).unwrap();
        // Pen Color goes through the colour menu's target.
        let (cmd, p) = crate::colors::Target::cmd("ui.tablePen").command("c00000").unwrap();
        a.run(&cmd, p).unwrap();
        let red = Pen { style: "double", width: 2.25, color: Some("C00000".into()) };
        assert_eq!(a.canvas.table_pen, red);
        for junk in [json!({"style": "zigzag"}), json!({"width": "thick"}), json!({"color": "purple"}), json!({"color": false}), json!({"color": 12})]
        {
            assert!(a.run("ui.tablePen", junk.clone()).is_err(), "{junk}");
        }
        assert_eq!(a.canvas.table_pen, red, "refused values change nothing");
        a.run("ui.tablePen", json!({"width": 1e9})).unwrap();
        assert_eq!(a.canvas.table_pen.width, 6.0, "clamped to Word's heaviest");
        a.run("ui.tablePen", json!({"width": 2.25, "color": "auto"})).unwrap();
        assert_eq!(a.canvas.table_pen.color, None);
        let st = PenStroke { page: 0, from: (10.0, 20.0), to: (110.0, 80.0) };
        let p = draw_params(&a.canvas.table_pen, &st);
        assert_eq!((&p["style"], &p["width"], &p["color"], &p["x1"]), (&json!("double"), &json!(2.25), &Value::Null, &json!(110.0)));
        assert_eq!(table_draw::pen(&p).map(|b| (b.style, b.width)), Some((wordcraft_doc::props::BorderStyle::Double, 2.25)));
        // The weights as Word lists them.
        let labels: Vec<String> = LINE_WEIGHTS.iter().map(|&w| Pen::weight_label(w)).collect();
        assert_eq!(labels, ["\u{bc} pt", "\u{bd} pt", "\u{be} pt", "1 pt", "1\u{bd} pt", "2\u{bc} pt", "3 pt", "4\u{bd} pt", "6 pt"]);
        // Another document keeps the pen.
        a.canvas.show_other_document(false);
        assert_eq!(a.canvas.table_pen.style, "double");
    }
}
