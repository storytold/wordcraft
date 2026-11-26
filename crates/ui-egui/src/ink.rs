//! Drawing on the page with the Draw tab's pens, eraser and lasso, and Ink Replay. While a pen
//! is in use, a drag draws a live stroke and its release commits it with `draw.stroke` (one
//! command, one undo step); with the eraser, a click or drag over a stroke deletes it with
//! `draw.erase` (one undo step per drag). With the lasso, a drag draws a loop whose release
//! selects the ink inside (`draw.lasso`); dragging the selection moves it (`draw.lassoMove`),
//! Delete deletes it and Esc lets it go. Select (or Esc) goes back to ordinary selection.
//!
//! Ink Replay (`draw.replay`) hides the ink on the pages and draws the strokes again here, one
//! after another in the order they were drawn, with a bar of controls at the bottom.

use craft::behavior::kurbo::{BezPath, Point};
use craft::behavior::overlay::{Align, Side};
use craft::behavior::{CancelReason, Cursor, CursorIcon, IconId, Metrics, Sense};
use craft::pages::ui::{PageBody, PlacedPage};
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::IconButton;
use craft::ui::canvas::{CanvasTool, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPath, ToolPointer};
use craft::ui::canvas_controls::{Marquee, MarqueeLook, SelectionOutline};
use craft::ui::contract::overlay::{Anchor, Overlay, Place};
use craft::ui::contract::{
    Common, Component, Container, ContainerProps, Cx, MeasureCx, Mount, NoValue, Outcome, Pad, PaintUi, Props, Role, Show as _, ShowContainer as _,
    Sizing,
};
use craft::ui::primitives::{DataStrokeWidth, DataWidth, Draw, PaintShape, StrokeColor};
use craft::ui::runtime::motion::Animate;
use craft::ui::style::{ComponentStyle, DataColor, StyleCx, SurfaceKind};
use egui::{Align2, Color32, Pos2, Response, Ui, Vec2};
use serde_json::json;
use wordcraft_doc::freeform::{InkTool, MAX_POINTS};
use wordcraft_engine::cmd::draw::DrawMode;
use wordcraft_layout::DocLayout;

use crate::WordApp;

/// A stroke being drawn: its page and points (page points).
pub struct InkDrag {
    page: usize,
    tool: InkTool,
    pts: Vec<[f32; 2]>,
}

/// The lasso in use: a loop being drawn, or the selection being dragged (page points).
pub enum LassoDrag {
    /// A loop being drawn.
    Loop {
        /// The page it is drawn on.
        page: usize,
        /// Its points.
        pts: Vec<[f32; 2]>,
    },
    /// The selection being dragged.
    Move {
        /// The page of the selection.
        page: usize,
        /// Where the drag began.
        from: [f32; 2],
        /// Where the pointer is now.
        to: [f32; 2],
    },
}

/// Ink Replay in progress: the strokes in drawing order and the two clock anchors that give how
/// far it has got. A replay is playing while `playing_from` is set; otherwise it holds at
/// `paused_at`. With neither set (a replay just started by a command) the next pass of the
/// replay's bar starts it from the beginning at that pass's time.
pub struct Replay {
    strokes: Vec<ReplayStroke>,
    /// The pass time at which the replay's position was 0, while it plays.
    playing_from: Option<f64>,
    /// Seconds into the replay, while it is held.
    paused_at: Option<f32>,
}

struct ReplayStroke {
    page: usize,
    paths: Vec<Vec<[f32; 2]>>,
    color: Color32,
    width: f32,
    /// When it starts being drawn and how long it takes, seconds.
    start: f32,
    dur: f32,
}

/// The Draw tab's pointer state beyond the pens (kept on the canvas).
#[derive(Default)]
pub struct InkUi {
    pub(crate) lasso: Option<LassoDrag>,
    pub(crate) replay: Option<Replay>,
}

/// Smallest move that adds a point to a live stroke, screen points.
const MIN_STEP_PX: f32 = 1.5;
/// How fast replayed ink is drawn, page points per second, and the pause between strokes.
const REPLAY_SPEED: f32 = 400.0;
const REPLAY_GAP: f32 = 0.12;
/// How close to the lasso selection's frame a press still grabs it, page points.
const GRAB_SLOP: f32 = 4.0;
/// Seconds between the passes a playing replay asks for.
const REPLAY_FRAME: f64 = 1.0 / 60.0;

// --- Pages ----------------------------------------------------------------------------------

/// The placed page `page` of this pass.
pub(crate) fn placed(pages: &[PlacedPage], page: usize) -> Option<&PlacedPage> {
    pages.iter().find(|p| p.page == page)
}

/// The page under (or nearest to) screen point `p`. Pages overlap in the margins Hide White Space
/// cuts: the one whose middle is nearer wins.
pub(crate) fn page_under(pages: &[PlacedPage], p: Pos2) -> Option<&PlacedPage> {
    let key = |pg: &PlacedPage| {
        if pg.rect.contains(p) { (0.0, (pg.rect.center().y - p.y).abs()) } else { (pg.rect.distance_to_pos(p), 0.0) }
    };
    pages.iter().filter(|pg| pg.scale > 0.0).min_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    })
}

/// The page point under screen point `p` on `pg`.
pub(crate) fn page_xy(pg: &PlacedPage, p: Pos2) -> [f32; 2] {
    let q = pg.to_page(p);
    [q.x as f32, q.y as f32]
}

fn screen_of(pg: &PlacedPage, p: [f32; 2]) -> Pos2 {
    pg.to_screen(Point::new(f64::from(p[0]), f64::from(p[1])))
}

// --- The tool -------------------------------------------------------------------------------

/// The pen, eraser and lasso as the canvas's tool for one pass. The gesture in progress lives in
/// `app.canvas` (`ink`, `erasing`, `ink_ui.lasso`), so nothing outlives the pass but those fields.
pub(crate) struct InkCanvasTool<'a> {
    app: &'a mut WordApp,
    body: &'a PageBody<'a>,
}

/// The ink tool for this pass while a pen, the eraser or the lasso is in use; `body` is this
/// pass's page body.
pub(crate) fn active_tool<'a>(app: &'a mut WordApp, body: &'a PageBody<'a>) -> Option<InkCanvasTool<'a>> {
    (app.session.view.draw.mode != DrawMode::Select).then_some(InkCanvasTool { app, body })
}

impl InkCanvasTool<'_> {
    /// Erases the stroke under `at` on `page`. The first stroke a drag erases starts an undo step;
    /// the rest join it.
    fn erase_at(&mut self, page: usize, at: [f32; 2]) {
        let Some(erased) = self.app.canvas.erasing else { return };
        if wordcraft_engine::cmd::draw::ink_at(&mut self.app.session, page, at[0], at[1]).is_none() {
            return;
        }
        if erased {
            self.app.session.join_next_undo();
        }
        if self.app.run("draw.erase", json!({"page": page, "x": at[0], "y": at[1]})).is_ok() {
            self.app.canvas.erasing = Some(true);
        }
    }

    fn clear_gesture(&mut self) {
        self.app.canvas.ink = None;
        self.app.canvas.erasing = None;
        self.app.canvas.ink_ui.lasso = None;
    }
}

impl CanvasTool for InkCanvasTool<'_> {
    fn press(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) -> ToolAction {
        let Some(pg) = page_under(self.body.pages, pointer.pos) else { return ToolAction::Ignored };
        let (page, at) = (pg.page, page_xy(pg, pointer.pos));
        match self.app.session.view.draw.mode {
            DrawMode::Pen(tool) => self.app.canvas.ink = Some(InkDrag { page, tool, pts: vec![at] }),
            DrawMode::Eraser => {
                self.app.canvas.erasing = Some(false);
                self.erase_at(page, at);
            }
            DrawMode::Lasso => {
                // A press on the selection drags it; anywhere else starts a new loop.
                let layout = self.app.session.layout();
                let grab = wordcraft_engine::cmd::draw::lasso_bounds(&self.app.session, &layout).is_some_and(|(bp, r)| {
                    bp == page
                        && at[0] >= r.x - GRAB_SLOP
                        && at[0] <= r.right() + GRAB_SLOP
                        && at[1] >= r.y - GRAB_SLOP
                        && at[1] <= r.bottom() + GRAB_SLOP
                });
                self.app.canvas.ink_ui.lasso =
                    Some(if grab { LassoDrag::Move { page, from: at, to: at } } else { LassoDrag::Loop { page, pts: vec![at] } });
            }
            DrawMode::Select => return ToolAction::Ignored,
        }
        ToolAction::Claimed
    }

    fn drag(&mut self, pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        let pages = self.body.pages;
        if let Some(d) = self.app.canvas.ink.as_mut() {
            if let Some(pg) = placed(pages, d.page) {
                let at = page_xy(pg, pointer.pos);
                let far = d.pts.last().is_none_or(|[lx, ly]| ((at[0] - lx).powi(2) + (at[1] - ly).powi(2)).sqrt() * pg.scale as f32 >= MIN_STEP_PX);
                if far && d.pts.len() < MAX_POINTS {
                    d.pts.push(at);
                }
            }
            return;
        }
        if self.app.canvas.erasing.is_some() {
            if let Some(pg) = page_under(pages, pointer.pos) {
                let (page, at) = (pg.page, page_xy(pg, pointer.pos));
                self.erase_at(page, at);
            }
            return;
        }
        match self.app.canvas.ink_ui.lasso.as_mut() {
            Some(LassoDrag::Loop { page, pts }) => {
                if let Some(pg) = placed(pages, *page) {
                    let at = page_xy(pg, pointer.pos);
                    let far =
                        pts.last().is_none_or(|[lx, ly]| ((at[0] - lx).powi(2) + (at[1] - ly).powi(2)).sqrt() * pg.scale as f32 >= MIN_STEP_PX * 2.0);
                    if far && pts.len() < MAX_POINTS {
                        pts.push(at);
                    }
                }
            }
            Some(LassoDrag::Move { page, to, .. }) => {
                if let Some(pg) = placed(pages, *page) {
                    *to = page_xy(pg, pointer.pos);
                }
            }
            None => {}
        }
    }

    fn release(&mut self, _pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {
        if let Some(d) = self.app.canvas.ink.take() {
            let _ = self.app.run("draw.stroke", json!({"page": d.page, "points": d.pts, "tool": d.tool.name()}));
        }
        self.app.canvas.erasing = None;
        match self.app.canvas.ink_ui.lasso.take() {
            Some(LassoDrag::Loop { page, pts }) if pts.len() >= 3 => {
                let _ = self.app.run("draw.lasso", json!({"page": page, "points": pts}));
            }
            // A click without a loop lets the selection go.
            Some(LassoDrag::Loop { .. }) => {
                let _ = self.app.run("draw.lasso", json!({"clear": true}));
            }
            Some(LassoDrag::Move { from, to, .. }) => {
                let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                if dx.abs() + dy.abs() > 0.25 {
                    let _ = self.app.run("draw.lassoMove", json!({"dx": dx, "dy": dy}));
                }
            }
            None => {}
        }
    }

    fn cancel(&mut self, _reason: CancelReason, _cx: &mut ToolContext<'_>) {
        self.clear_gesture();
    }

    fn claims(&self) -> ToolClaims {
        // Delete is the tool's only while a lasso selection exists to delete.
        if wordcraft_engine::cmd::draw::lasso_selection(&self.app.session).is_some() {
            ToolClaims::default().deletes()
        } else {
            ToolClaims::default()
        }
    }

    fn key(&mut self, key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        let selected = wordcraft_engine::cmd::draw::lasso_selection(&self.app.session).is_some();
        match key {
            ToolKey::Cancel if selected => {
                let _ = self.app.run("draw.lasso", json!({"clear": true}));
                true
            }
            ToolKey::Delete { .. } if selected => {
                let _ = self.app.run("draw.lassoDelete", json!({}));
                true
            }
            _ => false,
        }
    }

    fn paint(&self, draw: &Draw<'_>, _cx: &ToolContext<'_>) {
        let Some(d) = &self.app.canvas.ink else { return };
        let Some(pg) = placed(self.body.pages, d.page) else { return };
        let set = self.app.session.view.draw.settings(d.tool);
        let alpha = (d.tool.alpha() * 255.0).round() as u8;
        let color = screen_ink(crate::canvas::dark_page(self.app), set.color, alpha);
        let width = (set.width * pg.scale as f32).max(1.0);
        let pts: Vec<Pos2> = d.pts.iter().map(|p| screen_of(pg, *p)).collect();
        if let [only] = pts.as_slice() {
            draw.data_fill(&PaintShape::Circle(*only, width / 2.0), DataColor::new(color));
        } else {
            draw.stroke(&PaintShape::Path(pts), DataStrokeWidth::Data(DataWidth::points(width)), StrokeColor::Data(DataColor::new(color)));
        }
    }

    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        if matches!(self.app.canvas.ink_ui.lasso, Some(LassoDrag::Move { .. })) {
            Cursor::System(CursorIcon::Grabbing)
        } else {
            Cursor::System(CursorIcon::Crosshair)
        }
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the draw.stroke, draw.erase and draw.lasso commands take the same points")
    }
}

/// The on-screen colour of ink: on a dark page (View > Switch Modes) it is mapped like every other
/// document colour, so a stroke looks the same while it is drawn as after it is saved.
fn screen_ink(dark: bool, c: wordcraft_doc::Rgb, alpha: u8) -> Color32 {
    let opts = wordcraft_render::RenderOptions { dark, ..Default::default() };
    let wordcraft_doc::Rgb(r, g, b) = opts.ink(c);
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

// --- Overlay --------------------------------------------------------------------------------

/// The ink overlay of a pass: the lasso loop and its selection outline, and Ink Replay with its
/// bar. Called from the canvas body after the tool ran.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody) {
    let mode = app.session.view.draw.mode;
    if mode != DrawMode::Lasso {
        app.canvas.ink_ui.lasso = None;
        if app.session.view.draw.lasso.is_some() {
            let _ = app.run("draw.lasso", json!({"clear": true}));
        }
    }
    if mode == DrawMode::Select {
        app.canvas.ink = None;
        app.canvas.erasing = None;
    }
    show_lasso(app, ui, layout, body);
    show_replay(app, ui, body);
}

fn show_lasso(app: &WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody) {
    let to_doc = |pg: &PlacedPage, p: [f32; 2]| pg.to_doc(&body.view, Point::new(f64::from(p[0]), f64::from(p[1])));
    if let Some(LassoDrag::Loop { page, pts }) = &app.canvas.ink_ui.lasso
        && let Some(pg) = placed(body.pages, *page)
    {
        let doc: Vec<Point> = pts.iter().map(|p| to_doc(pg, *p)).collect();
        let _ = Marquee::polygon(tl!("Lasso").to_string(), doc).look(MarqueeLook::Drawing).show(ui);
    }
    let Some((page, r)) = wordcraft_engine::cmd::draw::lasso_bounds(&app.session, layout) else { return };
    let Some(pg) = placed(body.pages, page) else { return };
    let (dx, dy) = match &app.canvas.ink_ui.lasso {
        Some(LassoDrag::Move { from, to, .. }) => (to[0] - from[0], to[1] - from[1]),
        _ => (0.0, 0.0),
    };
    let (x0, y0, x1, y1) = (r.x + dx, r.y + dy, r.right() + dx, r.bottom() + dy);
    let mut path = BezPath::new();
    path.move_to(to_doc(pg, [x0, y0]));
    path.line_to(to_doc(pg, [x1, y0]));
    path.line_to(to_doc(pg, [x1, y1]));
    path.line_to(to_doc(pg, [x0, y1]));
    path.close_path();
    let paths = [path];
    let _ = SelectionOutline::dashed(tl!("Lasso selection").to_string(), &paths).show(ui);
}

// --- Ink Replay -----------------------------------------------------------------------------

/// Starts Ink Replay's state from the session's ink when `draw.replay` is on and none exists;
/// `false` (and the switch turned off) when there is no ink to replay.
fn ensure_replay(app: &mut WordApp) -> bool {
    if app.canvas.ink_ui.replay.is_some() {
        return true;
    }
    let mut start = 0.0;
    let strokes: Vec<ReplayStroke> = wordcraft_engine::cmd::draw::strokes_in_drawing_order(&mut app.session)
        .into_iter()
        .map(|(page, k)| {
            let paths = k.paths();
            let len: f32 = paths.iter().map(|p| path_len(p)).sum();
            let dur = (len / REPLAY_SPEED).clamp(0.15, 2.5);
            let alpha = (k.freeform.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            let c = k.color.unwrap_or_default();
            let width = if k.width.is_finite() { k.width.clamp(0.25, 72.0) } else { 1.0 };
            let st = ReplayStroke { page, paths, color: Color32::from_rgba_unmultiplied(c.0, c.1, c.2, alpha), width, start, dur };
            start += dur + REPLAY_GAP;
            st
        })
        .collect();
    if strokes.is_empty() {
        stop_replay(app);
        return false;
    }
    app.canvas.ink_ui.replay = Some(Replay { strokes, playing_from: None, paused_at: None });
    true
}

impl Replay {
    fn end(&self) -> f32 {
        self.strokes.last().map_or(0.0, |s| s.start + s.dur)
    }

    /// Seconds into the replay at pass time `now`.
    fn at(&self, now: f64) -> f32 {
        let t = match self.playing_from {
            Some(from) => (now - from) as f32,
            None => self.paused_at.unwrap_or(0.0),
        };
        t.clamp(0.0, self.end())
    }

    /// Holds the replay at `t` seconds.
    fn pause_at(&mut self, t: f32) {
        self.paused_at = Some(t);
        self.playing_from = None;
    }

    /// The position at `now`, with a replay that ran to its end held there.
    fn settle(&mut self, now: f64) -> f32 {
        let t = self.at(now);
        if self.playing_from.is_some() && t >= self.end() {
            self.pause_at(self.end());
        }
        t
    }

    /// Strokes started by `t`.
    fn started(&self, t: f32) -> usize {
        self.strokes.iter().filter(|s| s.start <= t).count()
    }
}

/// Holds Ink Replay at the fraction `at` (0 to 1, clamped) of its length; what `ui.replayAt` runs.
pub(crate) fn replay_at(app: &mut WordApp, at: f32) {
    if !app.session.view.draw.replay || !ensure_replay(app) {
        return;
    }
    let at = if at.is_finite() { at.clamp(0.0, 1.0) } else { 0.0 };
    if let Some(r) = app.canvas.ink_ui.replay.as_mut() {
        let t = at * r.end();
        r.pause_at(t);
    }
}

fn path_len(p: &[[f32; 2]]) -> f32 {
    p.windows(2).filter_map(|w| Some((w.first()?, w.get(1)?))).map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1])).sum()
}

/// Stops Ink Replay; `true` when one was running (Escape then goes no further).
pub(crate) fn stop_replay(app: &mut WordApp) -> bool {
    let was = app.session.view.draw.replay || app.canvas.ink_ui.replay.is_some();
    app.canvas.ink_ui.replay = None;
    if was {
        let _ = app.run("draw.replay", json!({"value": false}));
    }
    was
}

fn show_replay(app: &mut WordApp, ui: &mut Ui, body: &PageBody) {
    if !app.session.view.draw.replay {
        app.canvas.ink_ui.replay = None;
        return;
    }
    if !ensure_replay(app) {
        return;
    }
    let Some(r) = app.canvas.ink_ui.replay.as_mut() else { return };
    let overlay = Overlay::floating().modeless().place(Place::anchored(Anchor::Corner(body.rect, Align2::CENTER_BOTTOM), Side::Above, Align::Center));
    let mut close = false;
    // A pass whose layer body does not run (a settled layer) keeps the held position; a playing
    // replay wakes its body every frame.
    let mut t = r.paused_at.unwrap_or(0.0);
    let _ = InkReplay::new(overlay).show(ui, |ui, cx| {
        let now = cx.now();
        // A replay started outside a pass begins at this pass's time.
        if r.playing_from.is_none() && r.paused_at.is_none() {
            r.playing_from = Some(now);
        }
        let label =
            crate::i18n::fmt(tl!("Stroke {n} of {total}"), &[("n", &r.started(r.at(now)).to_string()), ("total", &r.strokes.len().to_string())]);
        let _ = Bar::new(tl!("Ink Replay").to_string(), BarKind::Tool).show(ui, |ui, _| {
            if IconButton::new(IconId::new("replayRewind")).label(tl!("Rewind").to_string()).show(ui).clicks > 0 {
                // Rewind keeps the play state: a playing replay starts again from the first stroke.
                if r.playing_from.is_some() {
                    r.playing_from = Some(now);
                } else {
                    r.pause_at(0.0);
                }
            }
            let playing = r.playing_from.is_some();
            let (icon, name) = if playing { ("replayPause", tl!("Pause")) } else { ("replayPlay", tl!("Play")) };
            if IconButton::new(crate::icons::id(icon)).label(name.to_string()).show(ui).clicks > 0 {
                let here = r.at(now);
                if playing {
                    r.pause_at(here);
                } else {
                    // Play again from the start once it has finished.
                    let from = if here >= r.end() { 0.0 } else { here };
                    r.playing_from = Some(now - f64::from(from));
                    r.paused_at = None;
                }
            }
            if IconButton::new(IconId::new("replayForward")).label(tl!("Forward").to_string()).show(ui).clicks > 0 {
                // Forward: finish the stroke being drawn, or the next one between strokes.
                let here = r.at(now);
                let next = r.strokes.iter().map(|s| s.start + s.dur).find(|e| *e > here + 1e-3).unwrap_or(r.end());
                if r.playing_from.is_some() {
                    r.playing_from = Some(now - f64::from(next));
                } else {
                    r.pause_at(next);
                }
            }
            let _ = craft::ui::display::Text::new(label.clone()).show(ui);
            if IconButton::new(IconId::new("wordcraft.close")).label(tl!("Close").to_string()).show(ui).clicks > 0 {
                close = true;
            }
        });
        t = r.settle(now);
        if r.playing_from.is_some() {
            cx.wake_at(now + REPLAY_FRAME);
        }
    });
    if close {
        stop_replay(app);
        return;
    }
    let dark = crate::canvas::dark_page(app);
    if let Some(r) = &app.canvas.ink_ui.replay {
        let _ = ReplayInk { common: Common::new(), pages: body.pages, replay: r, t, dark }.show_at(ui, body.rect);
    }
}

/// The replayed ink so far: finished strokes whole, the current one up to where it has got. A canvas
/// control with no parts, so it takes no input and paints through its `Draw`.
#[derive(Component, Props)]
#[component(canvas_control)]
struct ReplayInk<'a> {
    #[common]
    common: Common,
    pages: &'a [PlacedPage],
    replay: &'a Replay,
    /// Seconds into the replay.
    t: f32,
    /// The page is shown dark, so the ink is mapped like the saved one.
    dark: bool,
}

/// The replay ink takes its colours from the strokes' data; only metrics come from the style.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct ReplayInkStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl Component for ReplayInk<'_> {
    const ROLE: Role = Role::None;
    const OVERLAYS: bool = false;
    const STOPS: bool = false;
    const SHOWS_LABEL: bool = false;
    type Style = ReplayInkStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ReplayInkStyleKey) -> ReplayInkStyle {
        ReplayInkStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, _cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let (r, pages, t, dark) = (self.replay, self.pages, self.t, self.dark);
        paint_strokes(&draw, r, pages, t, dark);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

fn paint_strokes(draw: &Draw<'_>, r: &Replay, pages: &[PlacedPage], t: f32, dark: bool) {
    for k in r.strokes.iter().filter(|k| k.start <= t) {
        let Some(pg) = placed(pages, k.page) else { continue };
        let frac = if k.dur > 0.0 { ((t - k.start) / k.dur).clamp(0.0, 1.0) } else { 1.0 };
        let total: f32 = k.paths.iter().map(|p| path_len(p)).sum();
        let [cr, cg, cb, ca] = k.color.to_srgba_unmultiplied();
        let color = screen_ink(dark, wordcraft_doc::Rgb(cr, cg, cb), ca);
        let mut left = total * frac;
        let width = (k.width * pg.scale as f32).max(1.0);
        for path in &k.paths {
            let mut pts: Vec<Pos2> = Vec::new();
            for w in path.windows(2) {
                let (Some(a), Some(b)) = (w.first(), w.get(1)) else { continue };
                if pts.is_empty() {
                    pts.push(screen_of(pg, *a));
                }
                let seg = (b[0] - a[0]).hypot(b[1] - a[1]);
                if seg <= left {
                    pts.push(screen_of(pg, *b));
                    left -= seg;
                } else {
                    let f = if seg > 0.0 { left / seg } else { 0.0 };
                    pts.push(screen_of(pg, [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]));
                    left = 0.0;
                    break;
                }
            }
            match (pts.as_slice(), path.as_slice()) {
                (_, [only]) => {
                    draw.data_fill(&PaintShape::Circle(screen_of(pg, *only), width / 2.0), DataColor::new(color));
                }
                ([], _) => {}
                _ => {
                    draw.stroke(&PaintShape::Path(pts), DataStrokeWidth::Data(DataWidth::points(width)), StrokeColor::Data(DataColor::new(color)));
                }
            }
            if left <= 0.0 {
                break;
            }
        }
    }
}

/// The floating layer Ink Replay's bar sits in, at the bottom of the document area. Its body is
/// the bar; the layer reads the pass clock through its body context.
#[derive(Component)]
#[component(container, unregistered)]
struct InkReplay {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl InkReplay {
    fn new(overlay: Overlay) -> Self {
        let props = ContainerProps::new().mount(Mount::Layer(overlay)).surface(SurfaceKind::Floating).pad(Pad::None).sizing(Sizing::Content);
        InkReplay { common: Common::new(), props }.label(tl!("Ink Replay").to_string()).id_salt("ink-replay")
    }
}

/// The style of the replay layer: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct InkReplayStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What the replay layer reports: only its response.
#[derive(Debug)]
struct InkReplayOutput {
    response: Response,
}

impl Outcome for InkReplayOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl Component for InkReplay {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = InkReplayStyle;
    type Output = InkReplayOutput;

    fn style(_: &StyleCx<'_>, _: InkReplayStyleKey) -> InkReplayStyle {
        InkReplayStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> InkReplayOutput {
        InkReplayOutput { response: cx.response.clone() }
    }
}

impl Container for InkReplay {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_ink_follows_the_dark_page() {
        let black = wordcraft_doc::Rgb(0, 0, 0);
        assert_eq!(screen_ink(false, black, 255), Color32::from_rgba_unmultiplied(0, 0, 0, 255));
        // Black pen on a dark page turns light, like the saved stroke.
        let dark = screen_ink(true, black, 128);
        assert_eq!(dark, Color32::from_rgba_unmultiplied(255, 255, 255, 128));
    }
}
