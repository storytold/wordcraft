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
                json!({"page": st.page, "x0": st.from.0, "y0": st.from.1, "x1": st.to.0, "y1": st.to.1})
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
}
