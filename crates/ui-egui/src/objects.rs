//! Pictures, shapes and text boxes on the canvas: click one to select it, drag it to move it,
//! drag a handle of its frame to resize it, drag the rotation handle to turn it (Shift: 15° steps),
//! arrow keys nudge a floating one. The frame is a `TransformBox` ([`show`]); a move is an
//! [`ObjectGesture`] of the page tool. Changes go through `arrange.bounds` / `arrange.rotation` /
//! `arrange.nudge`, so they're undoable and scriptable like everything else.
//!
//! While dragging, nothing is laid out: the preview is the object drawn alone, once, when the
//! drag starts (#1010: a cut of the page would carry along whatever is behind it). The page it
//! was on is drawn again without it (see [`dragged_off_page`]), so it is not left behind where it
//! was. The drop is one command, one relayout.
//!
//! Dragging an In Line with Text object moves it within the text, as in Word: a bold insertion
//! cursor follows the pointer through the text (#2233, #4195) and the drop puts it there, still inline
//! (`arrange.bounds` / `arrange.copyTo` with `to`, #2234). A floating one goes where it's dropped.
//!
//! Moving a floating one snaps it to alignment guides ([`snap`]): the page's edges and centre, the
//! margins and the other floating objects on the page. Alt moves freely; File › Options › Advanced
//! turns them off (`ui.alignmentGuides`).
//!
//! A selected connector shows a handle at each end instead of a frame: dragging one onto a shape
//! glues it to the nearest connection site there (`shape.connect`), elsewhere leaves it free.
//!
//! Inside a Drawing Canvas, a press on a member picks that member (`select.member`, #2637): it
//! gets the frame and handles, inside the canvas's frame, and is dragged, resized and turned on
//! its own; a press in a text box member's text puts the caret there. The canvas itself is
//! picked where no member is.

use std::sync::Arc;

use craft::behavior::geometry::{BoxAllow, BoxRules, BoxState, ScaleRule, SnapKind, SnapOptions, SnapSuspend, SnapTarget};
use craft::behavior::kurbo::{BezPath, Point, Rect as DocRect, Vec2};
use craft::behavior::{Axis, CanvasView, Cursor, CursorIcon, GesturePhase, Input, Metrics, Sense};
use craft::draw::ui::transform::{LineEnd, LineEnds, LineLook, RotateZone, TransformBox};
use craft::keymap::Mods;
use craft::pages::ui::{PageBody, PageSnap, PlacedPage};
use craft::ui::canvas::{ToolContext, ToolPointer};
use craft::ui::canvas_controls::SelectionOutline;
use craft::ui::contract::{Bind, Common, Component, Cx, MeasureCx, PaintUi, Props, Role, Show as _};
use craft::ui::primitives::{Draw, PaintShape, StrokeWidth, TextureCache};
use craft::ui::runtime::env::Label;
use craft::ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Pos2, Rect, Ui};
use serde_json::json;
use wordcraft_doc::para::{InlineObject, OBJ, Wrap};
use wordcraft_doc::{PartKind, Pos, StoryRef};
use wordcraft_layout::DocLayout;
use wordcraft_layout::hit::ObjectHit;

use crate::WordApp;

/// The band along a text box's border that grabs the box rather than its text, screen points.
const EDGE: f32 = 5.0;
/// How opaque the drag preview of an object's pixels is (0–1).
const PREVIEW_SHARE: f32 = 170.0 / 255.0;
/// Arrow-key nudge, points (Ctrl/Alt: fine).
const NUDGE: f32 = 6.0;
const NUDGE_FINE: f32 = 1.0;

/// Where an In Line with Text object dragged through the text would land.
#[derive(Clone)]
struct Drop {
    /// The text position under the pointer.
    pos: Pos,
    /// The colour of the bold insertion cursor drawn there.
    color: Color32,
    /// The page the cursor is on and its box there, points.
    page: usize,
    caret: wordcraft_geom::Rect,
}

/// The object drawn alone over its rotated bounds, made once a move starts.
struct Preview {
    /// The bits of the pixels-per-page-point it was drawn at, which with the object names its texture.
    scale_bits: u32,
    image: Arc<egui::ColorImage>,
}

/// The object the frame's turn handle is turning: its pixels, drawn once, turned live by the box's angle.
pub(crate) struct Turning {
    /// The object turned.
    pos: Pos,
    preview: Preview,
    /// The turn so far beyond the object's own angle, degrees.
    delta: f32,
}

/// An object being moved by the page tool.
pub(crate) struct ObjectGesture {
    /// The object pressed; `None` for a press that was taken as a click (a link followed, an
    /// object added to the selection) and has nothing to drag.
    object: Option<ObjectHit>,
    /// The press, screen.
    press: Pos2,
    /// The grabbed point within the frame, points.
    grab: (f32, f32),
    /// The page and rect the object started on, points.
    page: usize,
    start: wordcraft_geom::Rect,
    /// Where it would land: page and rect, points.
    to_page: usize,
    rect: wordcraft_geom::Rect,
    /// Past the drag threshold (a press that never gets there is a click).
    moved: bool,
    /// A selected Behind Text picture overlaps text: the press also goes to that text until the
    /// drag takes the object, so a simple click can still put the caret there.
    pending_behind: bool,
    /// Ctrl/⌘ is held: dropping leaves the original and puts a copy at the new place.
    copy: bool,
    /// The (Shift, Ctrl/⌘) state at the last move: a change by the release re-places the object (an aligned
    /// copy's axis lock). Alt is not part of it: letting Alt go just before the release must not snap an
    /// object that was placed freely while Alt was held.
    last_mods: (bool, bool),
    /// Moving an In Line with Text object: where it lands.
    drop_at: Option<Drop>,
    preview: Option<Preview>,
}

/// The connection sites of the shapes near a dragged connector end: each a screen point, and whether it is the
/// one the end would glue to (filled).
#[derive(Component, Props)]
#[component(canvas_control)]
struct GlueSites {
    #[common]
    common: Common,
    sites: Vec<(Pos2, bool)>,
}

impl GlueSites {
    /// The `sites` (screen points, and whether each is the glue target), named `label`.
    fn new(label: impl Into<Label>, sites: Vec<(Pos2, bool)>) -> Self {
        GlueSites { common: Common::new(), sites }.label(label)
    }
}

/// The look of the glue sites: the handle's size and overlay colours.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct GlueSitesStyle {
    #[style(metrics)]
    metrics: Metrics,
    radius: f32,
    edge: StyleColor,
    fill: StyleColor,
}

impl Component for GlueSites {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = GlueSitesStyle;
    type Output = egui::Response;

    fn style(cx: &StyleCx<'_>, _: GlueSitesStyleKey) -> GlueSitesStyle {
        let colors = &cx.tokens.color.overlay;
        GlueSitesStyle {
            metrics: Metrics::new(),
            radius: cx.tokens.size.handle.sm / 2.0,
            edge: cx.color(colors.handle_stroke),
            fill: cx.color(colors.handle_fill),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = &cx.style;
        let draw = ui.draw();
        for &(at, glued) in &self.sites {
            let site = PaintShape::Circle(at, style.radius);
            draw.fill(&site, if glued { style.edge } else { style.fill });
            draw.stroke(&site, StrokeWidth::Thin, style.edge);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

/// The turning object's pixels on the clockwise screen quad `corners`, as the move preview shows them.
#[derive(Component, Props)]
#[component(canvas_control)]
struct TurnPreview {
    #[common]
    common: Common,
    corners: [Pos2; 4],
    texture: egui::TextureId,
}

impl TurnPreview {
    /// The pixels `texture` on `corners`, named `label`.
    fn new(label: impl Into<Label>, corners: [Pos2; 4], texture: egui::TextureId) -> Self {
        TurnPreview { common: Common::new(), corners, texture }.label(label)
    }
}

/// The look of the turn preview: it has no colours of its own.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct TurnPreviewStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl Component for TurnPreview {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = TurnPreviewStyle;
    type Output = egui::Response;

    fn style(_: &StyleCx<'_>, _: TurnPreviewStyleKey) -> TurnPreviewStyle {
        TurnPreviewStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.draw().faded(PREVIEW_SHARE, |d| d.texture_quad(self.corners, self.texture, uv));
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

/// Keeps the pixels of the object `o` the frame is turning, turned to `live`, and shows them. A box that is not a
/// pure turn (a resize) has no pixel preview; the frame's outline alone shows it.
fn show_turning(app: &mut WordApp, _ui: &mut Ui, layout: &DocLayout, body: &PageBody, pp: &PlacedPage, o: &ObjectHit, live: &BoxState) {
    let c = live.quad.map(|q| pp.from_doc(&body.view, q));
    let (w, h) = ((c[1] - c[0]).hypot() as f32, (c[3] - c[0]).hypot() as f32);
    if (w - o.rect.w).abs() > 0.5 || (h - o.rect.h).abs() > 0.5 {
        app.canvas.turning = None;
        return;
    }
    let deg = wordcraft_geom::normalize_degrees((c[1].y - c[0].y).atan2(c[1].x - c[0].x).to_degrees() as f32);
    let delta = wordcraft_geom::normalize_degrees(deg - o.spin.deg);
    if app.canvas.turning.as_ref().is_none_or(|t| t.pos != o.pos()) {
        app.canvas.turning = preview(app, layout, o, pp.scale as f32, body).map(|preview| Turning { pos: o.pos(), preview, delta });
    }
    if let Some(t) = app.canvas.turning.as_mut() {
        t.delta = delta;
    }
}

/// Paints the pixels of the turning object, if the frame is turning one.
fn paint_turning(app: &WordApp, ui: &mut Ui, body: &PageBody, pp: &PlacedPage, o: &ObjectHit) {
    let (Some(t), Some(ctx)) = (app.canvas.turning.as_ref().filter(|t| t.pos == o.pos()), app.ctx.as_ref()) else { return };
    let tex = TextureCache::texture(ctx, ("object-turn", t.pos.clone(), t.preview.scale_bits), || (*t.preview.image).clone());
    let s = screen_rect(pp, preview_bounds(o));
    // The cached pixels already hold the object's own turn and flips: only the turn beyond it is added, about the
    // centre of the bounds.
    let spin = egui::emath::Rot2::from_angle(t.delta.to_radians());
    let centre = s.center();
    let corners = [s.left_top(), s.right_top(), s.right_bottom(), s.left_bottom()].map(|p| centre + spin * (p - centre));
    TurnPreview::new(tl!("Turning object").to_string(), corners, tex).show_at(ui, body.rect);
}

/// The page `page` of `body`, as placed on screen this pass.
fn placed<'a>(body: &'a PageBody<'_>, page: usize) -> Option<&'a PlacedPage> {
    body.pages.iter().find(|p| p.page == page)
}

/// The page under (or, outside every page, nearest to) the screen point `at`, and the point on it
/// in page points.
fn page_at<'a>(body: &'a PageBody<'_>, layout: &DocLayout, at: Pos2) -> Option<(&'a PlacedPage, f32, f32)> {
    let (pp, p) = body.page_at_doc(body.view.to_doc(at))?;
    layout.pages.get(pp.page)?;
    Some((pp, p.x as f32, p.y as f32))
}

/// The width of the text-box edge band on `pp`, page points.
fn edge_of(pp: &PlacedPage) -> f32 {
    EDGE / (pp.scale as f32).max(0.01)
}

fn pt(x: f32, y: f32) -> Point {
    Point::new(f64::from(x), f64::from(y))
}

/// `r` on screen, on page `pp`.
fn screen_rect(pp: &PlacedPage, r: wordcraft_geom::Rect) -> Rect {
    Rect::from_two_pos(pp.to_screen(pt(r.x, r.y)), pp.to_screen(pt(r.right(), r.bottom())))
}

/// The selected floating connector's start and end on its page (points), and its layout; or a
/// floating line's, whose ends move the same way (they just don't glue); both in the body text,
/// where `shape.connect` moves them. Such a line shows
/// and takes handles at its ends only, as Word's do: the frame's handles would sit on a level
/// line, so pressing it there would resize it into a slant rather than move it.
fn connector_ends(app: &WordApp, layout: &DocLayout) -> Option<(ObjectHit, (f32, f32), (f32, f32))> {
    let (pos, InlineObject::Shape { kind, float, .. }) = selected(app)? else { return None };
    // Where `shape.connect` can't move its ends (In Line with Text, a header or footer), a line or
    // connector keeps its frame's handles instead.
    if !kind.is_open() || pos.story != StoryRef::Body || float.wrap == Wrap::Inline {
        return None;
    }
    let (flip_h, flip_v) = (float.flip_h, float.flip_v);
    let o = layout.object(&pos, app.session.page_hint)?;
    let (a, b) = wordcraft_engine::cmd::connectors::page_ends(o.rect, flip_h, flip_v, o.spin.deg);
    Some((o, a, b))
}

/// The selected object: the selection is exactly one picture, shape or text box, in the body
/// or in a header/footer.
pub fn selected(app: &WordApp) -> Option<(Pos, &InlineObject)> {
    wordcraft_engine::cmd::objects::object_selection(&app.session).filter(|(p, _)| selectable_story(app, p.story))
}

/// Whether objects of `story` can be selected and dragged on the canvas: the body and
/// headers/footers.
fn selectable_story(app: &WordApp, story: StoryRef) -> bool {
    match story {
        StoryRef::Body => true,
        StoryRef::Part(id) => app.session.doc.parts.get(&id).is_some_and(|p| matches!(p.kind, PartKind::Header | PartKind::Footer)),
    }
}

/// The story whose objects the canvas handles: the header/footer being edited, else the body.
fn active_story(app: &WordApp, layout: &DocLayout) -> StoryRef {
    let story = crate::canvas::text_box_host(app, layout).unwrap_or(app.session.sel.focus.story);
    match story {
        StoryRef::Body => StoryRef::Body,
        StoryRef::Part(id) if selectable_story(app, StoryRef::Part(id)) => StoryRef::Part(id),
        // Notes and text-box text: no top-level objects live here, so grabs find nothing and
        // the press falls through to text handling.
        other => other,
    }
}

/// Whether the object under `hit` keeps its place and size: a chart or diagram WordCraft has no
/// model for can be selected and deleted, not moved or resized, so it gets no handles, no move
/// cursor and no drag.
fn fixed(app: &WordApp, hit: &ObjectHit) -> bool {
    matches!(hit_object(app, hit), Some(InlineObject::Graphic { .. })) && !size_only(app, hit)
}

/// Whether the object under `hit` is a chart or SmartArt that WordCraft can edit: it takes
/// resize handles and is laid out again at the new size, but it can't be moved or turned.
fn size_only(app: &WordApp, hit: &ObjectHit) -> bool {
    matches!(
        hit_object(app, hit),
        Some(InlineObject::Graphic { graphic, .. }) if graphic.chart.is_some() || graphic.smart_art.is_some()
    )
}

/// The object a hit is: the object at its position, or the canvas member it names.
fn hit_object<'a>(app: &'a WordApp, hit: &ObjectHit) -> Option<&'a InlineObject> {
    let pos = hit.pos();
    let o = app.session.doc.para_at(&pos).and_then(|p| p.object_at(pos.off))?;
    match (hit.member, o) {
        (None, _) => Some(o),
        (Some(i), InlineObject::Group { children, canvas: Some(_), .. }) => children.get(i).map(|c| &c.obj),
        _ => None,
    }
}

/// Member `index` of the canvas laid out at `canvas`, as a hit of its own: its frame where the
/// canvas draws it, its own turn. `None` when it isn't drawn, or the canvas is turned (its
/// members aren't picked on their own yet).
fn member_hit(app: &WordApp, canvas: &ObjectHit, index: usize) -> Option<ObjectHit> {
    let (o, r) = (pickable_canvas(app, canvas)?, canvas.rect);
    let (_, frame) = o.member_rects(r.x, r.y, r.w, r.h).into_iter().find(|(i, _)| *i == index)?;
    member_with_frame(app, canvas, o, index, frame)
}

/// The canvas object of hit `canvas` when its members can be picked: a canvas, hit as a whole,
/// not turned.
fn pickable_canvas<'a>(app: &'a WordApp, canvas: &ObjectHit) -> Option<&'a InlineObject> {
    let pos = canvas.pos();
    let o = app.session.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).filter(|o| o.is_canvas())?;
    (canvas.member.is_none() && canvas.spin.is_identity()).then_some(o)
}

/// [`member_hit`] for member `index` of canvas `o`, whose frame on the page (`[x, y, w, h]`) is
/// already worked out.
fn member_with_frame(app: &WordApp, canvas: &ObjectHit, o: &InlineObject, index: usize, [x, y, w, h]: [f32; 4]) -> Option<ObjectHit> {
    let InlineObject::Group { children, .. } = o else { return None };
    let member = children.get(index).filter(|c| wordcraft_engine::cmd::canvas::pickable(c))?;
    let text_box = match &member.obj {
        InlineObject::Shape { story: Some(id), .. } if app.session.doc.parts.get(id).is_some_and(|p| p.kind == PartKind::TextBox) => Some(*id),
        _ => None,
    };
    Some(ObjectHit {
        rect: wordcraft_geom::Rect::new(x, y, w, h),
        spin: member.obj.frame().map(|(_, _, f)| f.spin()).unwrap_or_default(),
        text_box,
        // Placed by the canvas, not the text: dragging any handle may move its top-left.
        wrap: Wrap::InFrontOfText,
        member: Some(index),
        ..canvas.clone()
    })
}

/// The member of the canvas hit `canvas` under the page point (x, y), topmost first; a text box
/// member is grabbed anywhere on it too (inside, [`in_text`] tells its text from its border).
/// The members' frames are worked out once for all of them.
fn member_at(app: &WordApp, canvas: &ObjectHit, x: f32, y: f32, edge: f32) -> Option<ObjectHit> {
    let (o, r) = (pickable_canvas(app, canvas)?, canvas.rect);
    // Members are drawn clipped to the canvas: only what shows is grabbed.
    if !r.contains(wordcraft_geom::Point::new(x, y)) {
        return None;
    }
    o.member_rects(r.x, r.y, r.w, r.h)
        .into_iter()
        .rev()
        .filter_map(|(i, frame)| member_with_frame(app, canvas, o, i, frame))
        .find(|m| m.rect.expand(edge).contains(m.unspin(x, y)))
}

/// Whether the page point (x, y) is in a text box member's text rather than on its border.
fn in_text(m: &ObjectHit, x: f32, y: f32, edge: f32) -> bool {
    m.text_box.is_some() && m.rect.w.min(m.rect.h) > edge * 3.0 && m.rect.expand(-edge).contains(m.unspin(x, y))
}

/// The story of the selected text box (a picked canvas member too).
fn selected_text_box(app: &WordApp) -> Option<u32> {
    if let Some((_, _, InlineObject::Shape { story: Some(id), .. })) = wordcraft_engine::cmd::canvas::selected_member(&app.session) {
        return app.session.doc.parts.get(id).is_some_and(|p| p.kind == PartKind::TextBox).then_some(*id);
    }
    match selected(app)? {
        (_, InlineObject::Shape { story: Some(id), .. }) if app.session.doc.parts.get(id).is_some_and(|p| p.kind == PartKind::TextBox) => Some(*id),
        _ => None,
    }
}

/// The object showing a frame, and whether its text is being edited (caret in a text box).
pub fn active(app: &WordApp, layout: &DocLayout) -> Option<(ObjectHit, bool)> {
    let hint = app.session.page_hint;
    if let Some((pos, _)) = selected(app) {
        let canvas = layout.object(&pos, hint)?;
        if let Some((_, i, _)) = wordcraft_engine::cmd::canvas::selected_member(&app.session) {
            return member_hit(app, &canvas, i).or(Some(canvas)).map(|o| (o, false));
        }
        return Some((canvas, false));
    }
    match app.session.sel.focus.story {
        StoryRef::Part(id) if crate::canvas::in_text_box(app) => layout.text_box(id, hint).map(|o| (o, true)),
        _ => None,
    }
}

impl ObjectGesture {
    /// A press that was a click on an object and has nothing to drag.
    fn inert() -> Self {
        ObjectGesture {
            object: None,
            press: Pos2::ZERO,
            grab: (0.0, 0.0),
            page: 0,
            start: wordcraft_geom::Rect::new(0.0, 0.0, 0.0, 0.0),
            to_page: 0,
            rect: wordcraft_geom::Rect::new(0.0, 0.0, 0.0, 0.0),
            moved: false,
            pending_behind: false,
            copy: false,
            last_mods: (false, false),
            drop_at: None,
            preview: None,
        }
    }

    /// A move of `object`, grabbed at the screen point `at`.
    fn start(body: &PageBody, object: ObjectHit, at: Pos2) -> Option<Self> {
        let g = placed(body, object.page)?.to_page(at);
        let r = object.rect;
        Some(ObjectGesture {
            press: at,
            grab: (g.x as f32 - r.x, g.y as f32 - r.y),
            page: object.page,
            start: r,
            to_page: object.page,
            rect: r,
            object: Some(object),
            ..Self::inert()
        })
    }

    /// The press began on a Behind Text object that has not become a drag: the text under it gets the press as well.
    pub(crate) fn pending_behind(&self) -> bool {
        self.pending_behind
    }

    /// A press at `p` on the page tool's canvas: on an object it selects it and, where it can
    /// move, starts a move; `None` where the press is not about an object, so the text gets it.
    /// A gesture with nothing to drag stands for a press taken as a click (a link followed, an
    /// object added to the selection), so the text does not also get it. With
    /// `pending_behind` the text under the object gets the press as well, until the drag
    /// takes the object ([`release`](Self::release) says `false` for a click). A press that is
    /// not on the cropped picture ends cropping first.
    pub(crate) fn press(app: &mut WordApp, layout: &DocLayout, body: &PageBody, p: &ToolPointer, _cx: &mut ToolContext) -> Option<Self> {
        let (at, mods) = (p.pos, p.mods);
        if app.canvas.link_from.is_some() {
            link_click(app, layout, body, at);
            return Some(Self::inert());
        }
        let front = grab_at(app, layout, body, at);
        let on_selected = front.as_ref().is_some_and(|o| selected(app).is_some_and(|(pos, _)| pos == o.pos()));
        if !on_selected {
            // PLAN: assumed `pub fn leave(app: &mut WordApp)` in crop.rs (canvas-w12)
            crate::crop::leave(app);
        }
        // Ctrl/⌘+click inside a linked text box follows the text box's link (its text is not grabbed
        // as the object, so the object branch below can't see it).
        if front.is_none()
            && mods.cmd
            && !mods.shift
            && let Some(link) = text_box_at(layout, body, at).and_then(|object| object_link(app, &object))
        {
            crate::canvas::follow_link(app, link);
            return Some(Self::inert());
        }
        // A Behind Text object reached only for dragging lies under the text: text over it keeps its click.
        let hit = front.is_some();
        let object = front.or_else(|| behind_drag_at(app, layout, body, at))?;
        // Double-click a SmartArt graphic (selected by the first click) to edit its text.
        if p.clicks >= 2 && wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).is_some() {
            let _ = app.run("smartArt.textPane", json!({"value": true}));
            return Some(Self::inert());
        }
        // Ctrl/⌘+click on a linked picture or shape follows its link, as on linked text (#5267).
        if hit
            && mods.cmd
            && !mods.shift
            && let Some(link) = object_link(app, &object)
        {
            crate::canvas::follow_link(app, link);
            return Some(Self::inert());
        }
        // Shift/Ctrl+click adds an object to the selected ones (or takes it out), for Group.
        let adding = mods.shift || mods.cmd;
        if let Some(index) = object.member.filter(|_| !adding) {
            return press_member(app, layout, body, at, object, index);
        }
        // Adding to the selection takes the whole canvas.
        let object = match object.member {
            Some(_) => layout.object(&object.pos(), object.page)?,
            None => object,
        };
        // Ctrl on the object that is already selected starts a copy-drag instead, including
        // Ctrl+Shift for an aligned copy. Shift alone keeps its additive-selection behavior.
        let again = selected(app).is_some_and(|(pos, _)| pos == object.pos())
            || (mods.cmd && mods.shift && wordcraft_engine::cmd::objects::picked_objects(&app.session).contains(&object.pos()));
        let shift_only = mods.shift && !mods.cmd;
        if adding && selected(app).is_some() && (shift_only || !again) {
            let _ = app.run("select.addObject", json!({"pos": object.pos()}));
            return Some(Self::inert());
        }
        // While editing a header/footer, a click on its Behind Text picture where no header/footer
        // text covers it (the plain hit test picked it: down over the body, or clear of the text in
        // the band) selects it, as in Word; only over text does the click defer to that text.
        // ponytail: headers/footers only; body Behind Text pictures clear of text could do the same.
        let clear_of_text = matches!(object.story, StoryRef::Part(_))
            && page_at(body, layout, at)
                .and_then(|(pp, x, y)| layout.object_at_in(pp.page, x, y, edge_of(pp), object.story))
                .is_some_and(|o| o.pos() == object.pos());
        if object.behind() && !clear_of_text && behind_draggable(app, &object) {
            let mut g = Self::start(body, object, at)?;
            g.pending_behind = true;
            return Some(g);
        }
        let pos = object.pos();
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
        // A header/footer picture repeats on every page: select the copy clicked, as `select_at` does.
        app.session.page_hint = object.page;
        if selected(app).is_none() {
            return Some(Self::inert());
        }
        // Objects in table cells can be selected and resized, not moved; charts neither.
        if fixed(app, &object) || !movable(app, &object) || size_only(app, &object) {
            return Some(Self::inert());
        }
        let mut g = Self::start(body, object, at)?;
        g.copy = mods.cmd;
        Some(g)
    }

    /// The pointer moved to `p` with the button held: the object follows it, keeping the grabbed
    /// point under it across pages, snapped to the alignment guides (Alt: freely).
    pub(crate) fn drag(&mut self, app: &mut WordApp, layout: &DocLayout, body: &PageBody, p: &ToolPointer, cx: &mut ToolContext) {
        let Some(object) = self.object.clone() else { return };
        if self.pending_behind {
            let pos = object.pos();
            let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
            let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
            // The text click path has been tracking the pointer until it became a drag.
            app.canvas.cancel_text_drag();
            self.pending_behind = false;
        }
        self.moved = true;
        self.last_mods = (p.mods.shift, p.mods.cmd);
        self.copy = p.mods.cmd && object.member.is_none();
        // Ctrl+Shift moves a copy along the dominant axis only.
        let lock = (self.copy && p.mods.shift && object.floating()).then(|| {
            let d = p.pos - self.press;
            d.x.abs() >= d.y.abs()
        });
        let found = match lock {
            Some(_) => placed(body, self.page).map(|pp| {
                let g = pp.to_page(p.pos);
                (pp, g.x as f32, g.y as f32)
            }),
            None => page_at(body, layout, p.pos),
        };
        let Some((pp, x, y)) = found else { return };
        self.to_page = pp.page;
        self.rect = wordcraft_geom::Rect::new(x - self.grab.0, y - self.grab.1, self.start.w, self.start.h);
        match lock {
            Some(true) => self.rect.y = self.start.y,
            Some(false) => self.rect.x = self.start.x,
            None => {}
        }
        if !object.floating() {
            // An inline one lands in the text: the insertion cursor follows the pointer.
            self.drop_at = drop_point(app, layout, body, p.pos, self, &object);
        } else if object.member.is_none() {
            self.snap_move(body, cx, &object, lock);
        }
        if self.preview.is_none() {
            self.preview = preview(app, layout, &object, pp.scale as f32, body);
        }
    }

    /// Snap the moved object's bounds (rotated ones by their turned box) to the guides of the page
    /// it's over. `lock` (`Some(horizontal)`) keeps an aligned copy on its axis.
    fn snap_move(&mut self, body: &PageBody, cx: &mut ToolContext, object: &ObjectHit, lock: Option<bool>) {
        let Some(pp) = placed(body, self.to_page) else { return };
        let Some(b) = finite_spin_bounds(self.rect, object.spin) else { return };
        let a = pp.to_doc(&body.view, pt(b.x, b.y));
        let c = pp.to_doc(&body.view, pt(b.right(), b.bottom()));
        if !a.x.is_finite() || !a.y.is_finite() || !c.x.is_finite() || !c.y.is_finite() {
            return;
        }
        let off = cx.snap_rect(DocRect::from_points(a, c));
        let off = match lock {
            Some(true) => Vec2::new(off.x, 0.0),
            Some(false) => Vec2::new(0.0, off.y),
            None => off,
        };
        if !off.x.is_finite() || !off.y.is_finite() || off == Vec2::ZERO {
            return;
        }
        let at = pp.from_doc(&body.view, a + off);
        let dx = (at.x - f64::from(b.x)) as f32;
        let dy = (at.y - f64::from(b.y)) as f32;
        if dx.is_finite() && dy.is_finite() {
            self.rect.x += dx;
            self.rect.y += dy;
        }
    }

    /// The button went up. Commits a move as one command; false for a press on a Behind Text
    /// object that never became a drag, which is a click on the text under it.
    pub(crate) fn release(mut self, app: &mut WordApp, layout: &DocLayout, body: &PageBody, p: &ToolPointer, cx: &mut ToolContext) -> bool {
        if self.pending_behind {
            // No drag threshold was crossed: let the text click finish normally.
            app.canvas.cancel_text_drag();
            return false;
        }
        // The canvas calls `drag` only when the pointer moves, so a modifier changed since then (Shift up
        // frees an aligned copy's axis, Shift down locks it) is not yet in the placement. The release carries
        // the modifiers live at that moment: the object is placed once more with them.
        if self.moved && self.last_mods != (p.mods.shift, p.mods.cmd) {
            self.drag(app, layout, body, p, cx);
        }
        drop(app, &self);
        true
    }

    /// Escape: the drag is dropped without a change.
    pub(crate) fn cancel(self, _app: &mut WordApp) {}

    /// The drag preview: the object's pixels where it would land, and the insertion cursor of an
    /// In Line with Text object. The snap lines are the host's to draw.
    pub(crate) fn paint(&self, app: &WordApp, draw: &Draw, body: &PageBody) {
        let Some(object) = self.object.as_ref().filter(|_| self.moved) else { return };
        if let (Some(pv), Some(ctx)) = (&self.preview, app.ctx.as_ref())
            && let Some(pp) = placed(body, self.to_page)
        {
            let tex = TextureCache::texture(ctx, ("object-drag", object.pos(), pv.scale_bits), || (*pv.image).clone());
            // A turned object's pixels fill its rotated bounds, which move with it.
            let r = preview_bounds(object);
            let moved = wordcraft_geom::Rect::new(r.x + self.rect.x - self.start.x, r.y + self.rect.y - self.start.y, r.w, r.h);
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            let s = screen_rect(pp, moved);
            let corners = [s.left_top(), s.right_top(), s.right_bottom(), s.left_bottom()];
            draw.faded(PREVIEW_SHARE, |d| d.texture_quad(corners, tex, uv));
        }
        // The insertion cursor where a dragged In Line with Text object will land: solid and twice as
        // wide as the caret, so it stands out as Word's does (#4195).
        if let Some(drop) = &self.drop_at
            && let Some(pp) = placed(body, drop.page)
        {
            let r = screen_rect(pp, drop.caret);
            let bar = Rect::from_min_max(egui::pos2(r.left() - 1.5, r.top()), egui::pos2(r.left() + 1.5, r.bottom()));
            draw.data_fill(&PaintShape::Rect(bar, CornerRadius::ZERO), DataColor::new(drop.color));
        }
    }

    /// The pointer over an object (not a handle of the frame, which is the box's): its cursor.
    pub(crate) fn cursor(app: &WordApp, layout: &DocLayout, body: &PageBody, at: Pos2, mods: Mods) -> Option<Cursor> {
        let o = grab_at(app, layout, body, at)?;
        let (pp, x, y) = page_at(body, layout, at)?;
        let in_member_text = o.member.is_some() && in_text(&o, x, y, edge_of(pp));
        let icon = if in_member_text && !fixed(app, &o) {
            CursorIcon::Text
        } else if fixed(app, &o) || size_only(app, &o) {
            CursorIcon::Default
        } else if mods.cmd && selected(app).is_some_and(|(pos, _)| pos == o.pos()) {
            CursorIcon::Copy
        } else {
            CursorIcon::Move
        };
        Some(Cursor::from(icon))
    }
}

/// A press on member `index` of a canvas (`object` is the member's hit): in a text box member's
/// text it puts the caret there; elsewhere it picks the member (selecting its canvas first) and
/// starts dragging it.
fn press_member(app: &mut WordApp, layout: &DocLayout, body: &PageBody, at: Pos2, object: ObjectHit, index: usize) -> Option<ObjectGesture> {
    if let (Some(id), Some((pp, x, y))) = (object.text_box, page_at(body, layout, at))
        && in_text(&object, x, y, edge_of(pp))
    {
        // A box linked from another shows (and edits) that box's story.
        let id = app.session.doc.text_box_chain(id).and_then(|c| c.first().copied()).unwrap_or(id);
        if let Some(pos) = layout.hit(pp.page, x, y, StoryRef::Part(id)) {
            let _ = app.run("caret.set", json!({"pos": pos}));
        }
        return Some(ObjectGesture::inert());
    }
    let pos = object.pos();
    let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
    let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
    if app.run("select.member", json!({"index": index})).is_err() {
        return Some(ObjectGesture::inert());
    }
    ObjectGesture::start(body, object, at)
}

/// The object a move has taken off its page (which page, and where it is in the document): the
/// page is rendered again without it while the preview follows the pointer, so the object is not
/// left behind where it was. Not for a member of a drawing canvas: its position is the canvas's,
/// and the canvas stays.
pub(crate) fn dragged_off_page(drag: Option<&ObjectGesture>) -> Option<(usize, Pos)> {
    let d = drag.filter(|d| d.moved)?;
    let object = d.object.as_ref().filter(|o| o.member.is_none())?;
    Some((object.page, object.pos()))
}

/// Where an In Line with Text object dragged to `at` (screen) lands: the nearest position of its
/// own story's text there, with the insertion cursor's colour (the same contrast as the caret's).
fn drop_point(app: &WordApp, layout: &DocLayout, body: &PageBody, at: Pos2, g: &ObjectGesture, object: &ObjectHit) -> Option<Drop> {
    let (pp, x, y) = page_at(body, layout, at)?;
    let pos = layout.hit(pp.page, x, y, object.story)?;
    if let Some(was) = &g.drop_at
        && was.pos == pos
    {
        return Some(was.clone()); // the paper under it is worked out once per position
    }
    let c = layout.caret_on(&pos, pp.page)?;
    let color = crate::canvas::caret_color_on(app, layout, c.page, c.x, c.top + c.height / 2.0);
    Some(Drop { pos, color, page: c.page, caret: wordcraft_geom::Rect::new(c.x, c.top, 0.0, c.height) })
}

/// The snapping of an object's move (View › Snap To, as the alignment guides): the page's edges and
/// centre, its margins and the other floating objects on it, for every page, each catching only
/// along its own page. Alt suspends it; File › Options › Advanced turns it off.
pub(crate) fn snap(app: &WordApp, layout: &DocLayout) -> Option<PageSnap> {
    let me = selected(app).and_then(|(pos, _)| layout.object(&pos, app.session.page_hint));
    let pages: Vec<Vec<SnapTarget>> = layout.pages.iter().map(|pg| snap_targets(pg, me.as_ref())).collect();
    let mut options = SnapOptions::default();
    options.on = app.ui.alignment_guides;
    options.suspend = SnapSuspend::Alt;
    options.grid = false;
    options.layers = false;
    // Each page's targets in its own page points, which the page view maps to document space.
    Some(pages.into_iter().enumerate().fold(PageSnap::new(options), |snap, (page, targets)| snap.page(page, targets)))
}

/// Axis-aligned bounds of `rect` turned by `spin`, or `None` when any input or bound is non-finite.
fn finite_spin_bounds(rect: wordcraft_geom::Rect, spin: wordcraft_geom::Spin) -> Option<wordcraft_geom::Rect> {
    if !rect.x.is_finite() || !rect.y.is_finite() || !rect.w.is_finite() || !rect.h.is_finite() || !spin.deg.is_finite() {
        return None;
    }
    let (bw, bh) = spin.extent(rect.w, rect.h);
    let x = rect.x + (rect.w - bw) / 2.0;
    let y = rect.y + (rect.h - bh) / 2.0;
    (x.is_finite() && y.is_finite() && bw.is_finite() && bh.is_finite()).then(|| wordcraft_geom::Rect::new(x, y, bw, bh))
}

/// The snap targets of `pg` in its own page points, each spanning the page's own range:
/// its edges and centre, its margins, and the floating objects on it that `me` can snap to (all but
/// itself and, for a text box, the objects in its text: they move with it). The page's own width
/// leaves out the comment balloons' markup area.
fn snap_targets(pg: &wordcraft_layout::Page, me: Option<&ObjectHit>) -> Vec<SnapTarget> {
    if !pg.w.is_finite() || !pg.h.is_finite() || !pg.markup.is_finite() {
        return Vec::new();
    }
    let w = (f64::from(pg.w) - f64::from(pg.markup)).max(0.0);
    let h = f64::from(pg.h).max(0.0);
    let mut targets = Vec::new();
    let vertical = |targets: &mut Vec<SnapTarget>, x: f64, kind: SnapKind| {
        if !x.is_finite() {
            return;
        }
        let mut t = SnapTarget::new(Axis::Y, x, kind);
        t.span = Some((0.0, h));
        targets.push(t);
    };
    for (x, kind) in [(0.0, SnapKind::Bounds), (w / 2.0, SnapKind::Center), (w, SnapKind::Bounds)] {
        vertical(&mut targets, x, kind);
    }
    if pg.body.x.is_finite() && pg.body.w.is_finite() {
        vertical(&mut targets, f64::from(pg.body.x), SnapKind::Guide);
        vertical(&mut targets, f64::from(pg.body.x) + f64::from(pg.body.w), SnapKind::Guide);
    }
    let horizontal = |targets: &mut Vec<SnapTarget>, y: f64, kind: SnapKind| {
        if !y.is_finite() {
            return;
        }
        let mut t = SnapTarget::new(Axis::X, y, kind);
        t.span = Some((0.0, w));
        targets.push(t);
    };
    for (y, kind) in [(0.0, SnapKind::Bounds), (h / 2.0, SnapKind::Center), (h, SnapKind::Bounds)] {
        horizontal(&mut targets, y, kind);
    }
    if pg.body.y.is_finite() && pg.body.h.is_finite() {
        horizontal(&mut targets, f64::from(pg.body.y), SnapKind::Guide);
        horizontal(&mut targets, f64::from(pg.body.y) + f64::from(pg.body.h), SnapKind::Guide);
    }
    let pos = me.map(ObjectHit::pos);
    let own = me.and_then(|m| m.text_box).map(StoryRef::Part);
    for it in pg.items.iter().chain(&pg.header).chain(&pg.footer) {
        let wordcraft_layout::Placed::Object { rect, spin, story, path, off, wrap, .. } = it else { continue };
        let is_me = pos.as_ref().is_some_and(|p| *story == p.story && *path == p.path && *off == p.off);
        if *wrap == Wrap::Inline || is_me || Some(*story) == own {
            continue;
        }
        let Some(b) = finite_spin_bounds(*rect, *spin) else {
            continue;
        };
        let x0 = f64::from(b.x);
        let x1 = x0 + f64::from(b.w);
        for (x, kind) in [(x0, SnapKind::LayerEdge), ((x0 + x1) / 2.0, SnapKind::LayerCenter), (x1, SnapKind::LayerEdge)] {
            vertical(&mut targets, x, kind);
        }
        let y0 = f64::from(b.y);
        let y1 = y0 + f64::from(b.h);
        for (y, kind) in [(y0, SnapKind::LayerEdge), ((y0 + y1) / 2.0, SnapKind::LayerCenter), (y1, SnapKind::LayerEdge)] {
            horizontal(&mut targets, y, kind);
        }
    }
    targets
}

/// Create Link's click: link the box it started from to the text box under `at` (anywhere on it);
/// either way, linking is over.
fn link_click(app: &mut WordApp, layout: &DocLayout, body: &PageBody, at: Pos2) {
    let Some(from) = app.canvas.link_from.take() else { return };
    let target = page_at(body, layout, at).and_then(|(pp, x, y)| {
        let page = pp.page;
        layout.find_object(page, |o| o.page == page && o.text_box.is_some() && o.rect.contains(wordcraft_geom::Point::new(x, y)))
    });
    let Some(target) = target else {
        app.status(tl!("Not linked: click a text box"));
        return;
    };
    let end = Pos { off: from.off + OBJ.len_utf8(), ..from.clone() };
    let _ = app.run("select.range", json!({"anchor": from, "focus": end}));
    if let Err(e) = app.run("shape.link", json!({"target": target.pos()})) {
        app.status(e);
    }
}

/// Right-click at `at`: the picture, shape or text box under it becomes the selection (as a click
/// would, without a drag), so the context menu acts on it. Returns whether there was one.
pub fn select_at(app: &mut WordApp, layout: &DocLayout, body: &PageBody, at: Pos2) -> bool {
    // While editing a header/footer, `grab_at` looks at its own objects, so its picture is picked
    // too and the menu's Reset Picture or Wrap Text acts on it.
    let Some(object) = grab_at(app, layout, body, at) else { return false };
    let pos = object.pos();
    // Already selected (alone or among several picked for Group): keep the selection.
    if !wordcraft_engine::cmd::objects::picked_objects(&app.session).contains(&pos) {
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
    }
    // A header/footer picture repeats on every page: its frame belongs on the page clicked, not
    // the one last edited.
    app.session.page_hint = object.page;
    true
}

/// The hyperlink of the picture, shape or text box under `at` (Insert › Link with it selected,
/// or a link read from the file): what hovering shows and Ctrl/⌘+click
/// follows, as for linked text (#5267).
pub fn link_at(app: &WordApp, layout: &DocLayout, body: &PageBody, at: Pos2) -> Option<String> {
    match grab_at(app, layout, body, at) {
        Some(object) => object_link(app, &object),
        // Inside a text box its text is what a click reaches, but a linked text box's link
        // comes before its text's own, as for a picture or shape.
        None => text_box_at(layout, body, at).and_then(|object| object_link(app, &object)),
    }
}

/// The text box whose text is under `at`, if any.
fn text_box_at(layout: &DocLayout, body: &PageBody, at: Pos2) -> Option<ObjectHit> {
    let (pp, x, y) = page_at(body, layout, at)?;
    let StoryRef::Part(id) = layout.story_at(pp.page, x, y)? else { return None };
    layout.text_box(id, pp.page)
}

/// The hyperlink on `object`'s character, if it has one.
fn object_link(app: &WordApp, object: &ObjectHit) -> Option<String> {
    let pos = object.pos();
    app.session.doc.para_at(&pos)?.props_of_char(pos.off).link.clone().filter(|l| !l.is_empty())
}

/// The object of the story being edited (body, or the header/footer) that a press at `at` would
/// grab. The handles of a shown frame are the box's, taken before the tool.
fn grab_at(app: &WordApp, layout: &DocLayout, body: &PageBody, at: Pos2) -> Option<ObjectHit> {
    let story = active_story(app, layout);
    let (pp, x, y) = page_at(body, layout, at)?;
    let (page, edge) = (pp.page, edge_of(pp));
    // A Behind Text picture is normally hit only where no body text is present, so text above
    // it remains easy to edit. Once the picture is selected (for example with the arrow keys
    // or the Selection Pane), let its own frame win inside the frame so the user can drag it
    // directly without first finding an uncovered patch.
    let selected_picture = selected(app)
        .filter(|(_, obj)| behind_body_drag(obj))
        .and_then(|(pos, _)| layout.object(&pos, app.session.page_hint))
        .filter(|o| o.behind() && o.page == page && movable(app, o) && o.rect.expand(edge).contains(o.unspin(x, y)));
    if selected_picture.is_some() {
        return selected_picture;
    }
    // Selectable: body objects (table cells too) or the header/footer's own objects;
    // notes' objects aren't yet. Inside a canvas, the member there.
    let o = layout.object_at_in(page, x, y, edge, story)?;
    Some(member_at(app, &o, x, y, edge).unwrap_or(o))
}

/// Objects that can be dragged to a new place: those in the own paragraphs of the body or of a
/// header/footer.
fn movable(app: &WordApp, o: &ObjectHit) -> bool {
    // A canvas member moves inside its canvas, wherever the canvas is.
    (o.member.is_some() || o.path.depth() == 0) && selectable_story(app, o.story)
}

/// Whether a Behind Text object of this kind is grabbed by pressing its body. Pictures, shapes and
/// groups yes; a text box no (its inside is its text, so only its border grabs it). Used so one can
/// be dragged over the text it sits behind.
fn behind_body_drag(obj: &InlineObject) -> bool {
    matches!(obj, InlineObject::Image { .. } | InlineObject::Group { .. } | InlineObject::Shape { story: None, .. })
}

/// Whether the object at `o` is a Behind Text picture, shape or group that grabbing its body moves.
fn behind_draggable(app: &WordApp, o: &ObjectHit) -> bool {
    app.session.doc.para_at(&o.pos()).and_then(|p| p.object_at(o.pos().off)).is_some_and(behind_body_drag)
}

/// The topmost Behind Text picture, shape or group of the story being edited under `at`, even
/// where text covers it. A press over one couldn't grab it before (text won), so a drag did
/// nothing; with this it starts a frame drag once the pointer moves, while a plain click still
/// edits the text it sits behind (see `pending_behind`).
fn behind_drag_at(app: &WordApp, layout: &DocLayout, body: &PageBody, at: Pos2) -> Option<ObjectHit> {
    let story = active_story(app, layout);
    let (pp, x, y) = page_at(body, layout, at)?;
    let (page, edge) = (pp.page, edge_of(pp));
    layout.find_object(page, |o| {
        o.page == page
            && o.story == story
            && o.behind()
            && movable(app, o)
            && behind_draggable(app, o)
            && o.rect.expand(edge).contains(o.unspin(x, y))
    })
}

/// The pixels-per-page-point for a `w` x `h` preview, lowered so neither side exceeds the GPU's
/// `max_side` (the texture is painted into the object's rect, so a smaller one is only stretched).
fn preview_scale(px: f32, w: f32, h: f32, max_side: usize) -> f32 {
    let room = (max_side.min(wordcraft_render::MAX_SIDE as usize).max(2) - 1) as f32;
    let px = if px.is_finite() { px } else { 1.0 };
    px.min(room / w.max(h).max(1.0))
}

/// The thinnest a drag preview is, in page points: a level line has no height, and a texture over a
/// rect with none paints nothing.
const MIN_PREVIEW_SIDE: f32 = 2.0;

/// `o`'s rotated bounds for its drag preview, grown about their centre to at least
/// [`MIN_PREVIEW_SIDE`] each way.
fn preview_bounds(o: &ObjectHit) -> wordcraft_geom::Rect {
    let b = o.bounds();
    let (w, h) = (b.w.max(MIN_PREVIEW_SIDE), b.h.max(MIN_PREVIEW_SIDE));
    wordcraft_geom::Rect::new(b.x - (w - b.w) / 2.0, b.y - (h - b.h) / 2.0, w, h)
}

/// `o` drawn alone over its rotated bounds, at `scale` screen points per page point.
fn preview(app: &WordApp, layout: &DocLayout, o: &ObjectHit, scale: f32, body: &PageBody) -> Option<Preview> {
    let obj = hit_object(app, o)?;
    let page = layout.pages.get(o.page)?;
    let mut opts = crate::canvas::screen_render_options();
    opts.dark = crate::canvas::dark_page(app);
    let items = wordcraft_layout::display::object_display(&app.session.doc, page, obj, o.rect, o.text_box, &opts.display);
    let b = preview_bounds(o);
    let max_side = app.ctx.as_ref().map_or(2048, |c| c.input(|i| i.max_texture_side));
    let ppp = body.pass.and_then(|p| p.space.as_ref()).map_or(1.0, |s| s.pixels_per_point);
    let px = preview_scale(scale * ppp, b.w, b.h, max_side);
    let img = wordcraft_render::render_items(&app.session.doc, &items, wordcraft_geom::Rect::new(b.x, b.y, b.w, b.h), px, &opts);
    // A texture over the device limit panics wgpu outside `Session::run`; fall back to no preview.
    if img.width as usize > max_side || img.height as usize > max_side {
        return None;
    }
    let size = [img.width as usize, img.height as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &img.to_straight());
    Some(Preview { scale_bits: px.to_bits(), image: Arc::new(image) })
}

/// Commit a finished move as one command.
fn drop(app: &mut WordApp, d: &ObjectGesture) {
    let Some(object) = d.object.as_ref().filter(|_| d.moved) else { return };
    let r = d.rect;
    // An In Line with Text object stays in the text, at the insertion cursor; dropped on
    // itself, it doesn't move.
    if !object.floating() {
        let Some(Drop { pos: to, .. }) = &d.drop_at else { return };
        let pos = object.pos();
        let on_itself = to.story == pos.story && to.path == pos.path && (to.off == pos.off || to.off == pos.off + OBJ.len_utf8());
        if d.copy {
            let _ = app.run("arrange.copyTo", json!({"to": to}));
        } else if !on_itself {
            let _ = app.run("arrange.bounds", json!({"to": to}));
        }
        return;
    }
    // A click that never moved the object changes nothing.
    if d.to_page == d.page && r == d.start {
        return;
    }
    if d.copy {
        let _ = app.run("arrange.copyTo", json!({"page": d.to_page, "x": r.x, "y": r.y}));
    } else {
        let _ = app.run("arrange.bounds", json!({"page": d.to_page, "x": r.x, "y": r.y}));
    }
}

/// The corners of `o`'s frame in document space, clockwise from the top left, turned by its spin.
fn box_quad(pp: &PlacedPage, view: &CanvasView, o: &ObjectHit) -> [Point; 4] {
    let r = o.rect;
    let (s, c) = f64::from(o.spin.deg).to_radians().sin_cos();
    let (cx, cy) = (f64::from(r.x + r.w / 2.0), f64::from(r.y + r.h / 2.0));
    let (hw, hh) = (f64::from(r.w / 2.0), f64::from(r.h / 2.0));
    [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].map(|(dx, dy)| pp.to_doc(view, Point::new(cx + dx * c - dy * s, cy + dx * s + dy * c)))
}

/// `l` page points as document units, along the page's x axis.
fn doc_len(pp: &PlacedPage, view: &CanvasView, l: f32) -> f64 {
    pp.to_doc(view, pt(l, 0.0)).x - pp.to_doc(view, pt(0.0, 0.0)).x
}

/// `o`'s turned frame as a closed path in document units.
fn outline_path(pp: &PlacedPage, view: &CanvasView, o: &ObjectHit) -> BezPath {
    let q = box_quad(pp, view, o);
    let mut path = BezPath::new();
    path.move_to(q[0]);
    for p in &q[1..] {
        path.line_to(*p);
    }
    path.close_path();
    path
}

/// The frame of the selected object, as a box: its handles resize it, the rotation handle turns it
/// (Shift: 15° steps); a connector or line shows a handle at each end instead, and dragging one
/// shows the connection sites it would glue to. The frame of a text box being edited is dashed, that
/// of a fixed graphic solid without handles, and the other objects picked for Group are outlined.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, layout: &DocLayout, body: &PageBody) {
    // Set again below while the frame's box has a press open.
    app.canvas.frame_held = false;
    let active = active(app, layout);
    let others: Vec<BezPath> = wordcraft_engine::cmd::objects::picked_objects(&app.session)
        .iter()
        .filter(|pos| active.as_ref().is_none_or(|(o, _)| o.pos() != **pos))
        .filter_map(|pos| layout.object(pos, app.session.page_hint))
        .filter_map(|h| placed(body, h.page).map(|pp| outline_path(pp, &body.view, &h)))
        .collect();
    if !others.is_empty() {
        SelectionOutline::solid(tl!("Picked objects").to_string(), &others).show_at(ui, body.rect);
    }
    let Some((o, editing)) = active else { return };
    if !selectable_story(app, o.story) {
        return;
    }
    let Some(pp) = placed(body, o.page) else { return };
    // The picture in crop mode has the crop overlay's frame and handles on its corners: the object's own
    // resize handles there would take the press first and resize the picture instead of cropping it.
    if crate::crop::active(app).is_some() {
        return;
    }
    let view = &body.view;
    if editing || fixed(app, &o) {
        let frame = [outline_path(pp, view, &o)];
        let outline = if editing {
            SelectionOutline::dashed(tl!("Text box frame").to_string(), &frame)
        } else {
            SelectionOutline::solid(tl!("Object frame").to_string(), &frame)
        };
        outline.show_at(ui, body.rect);
        return;
    }
    let ends = connector_ends(app, layout);
    let size_only = size_only(app, &o);
    let (min, picture) = match hit_object(app, &o) {
        Some(obj) => (wordcraft_engine::cmd::objects::min_size(obj), matches!(obj, InlineObject::Image { .. })),
        None => return,
    };
    // The framework turns a box about its pivot: the centre of the object's (possibly turned) bounds, not the origin.
    let quad = box_quad(pp, view, &o);
    let pivot = quad[0].midpoint(quad[2]);
    let state = BoxState::new(quad, pivot);
    let start = DocRect::from_points(pp.to_doc(view, pt(o.rect.x, o.rect.y)), pp.to_doc(view, pt(o.rect.right(), o.rect.bottom())));
    let scale = if picture { ScaleRule::Corners } else { ScaleRule::Free };
    let rules = BoxRules::default().with_scale(scale).with_allow(BoxAllow::new(!size_only, false, false)).with_min_side(doc_len(pp, view, min));
    let mut tbox =
        TransformBox::new(tl!("Object frame").to_string(), Bind::given(state), start, 0.0).rules(rules).inside_moves(false).reference_point(false);
    if !size_only {
        tbox = tbox.rotate_zone(RotateZone::Handle);
    }
    if let Some((_, a, b)) = ends {
        tbox = tbox.line(LineLook::new(pp.to_doc(view, pt(a.0, a.1)), pp.to_doc(view, pt(b.0, b.1))).ends(LineEnds::Free));
    }
    let out = tbox.show_at(ui, body.rect);
    app.canvas.frame_held = out.held.is_some();
    // The object's pixels follow the turn handle live (a text box's text does not turn with its frame yet, so it
    // keeps the outline until the release). The release and Escape end the preview.
    let ended = out.edit.events.committed().is_some() || matches!(out.edit.events.last(), Some(craft::behavior::EditEvent::Cancel { .. }));
    if ended || out.held.is_none() || o.text_box.is_some() {
        app.canvas.turning = None;
    } else {
        if let Some(live) = out.edit.events.value() {
            show_turning(app, ui, layout, body, pp, &o, live);
        }
        paint_turning(app, ui, body, pp, &o);
    }
    if let Some(after) = out.edit.events.committed() {
        commit_box(app, &o, pp, view, after);
    }
    if let (Some(drag), Some(_)) = (out.line_end, ends) {
        let at = pp.from_doc(view, drag.at);
        let at = (at.x as f32, at.y as f32);
        match drag.phase {
            GesturePhase::Commit => {
                let key = if drag.end == LineEnd::From { "start" } else { "end" };
                let _ = app.run("shape.connect", json!({key: {"x": at.0, "y": at.1, "page": o.page}}));
            }
            GesturePhase::Begin | GesturePhase::Update => {
                let sites = glue_sites(app, layout, pp, &o, at);
                if !sites.is_empty() {
                    GlueSites::new(tl!("Connection sites").to_string(), sites).show_at(ui, body.rect);
                }
            }
            _ => {}
        }
    }
}

/// Apply a committed box to object `o`: a turn is `arrange.rotation`, a new size (and place, for a
/// floating one that can move: its far edges stay put) is `arrange.bounds`, or `picture.size` for a
/// chart or SmartArt. A box that ended where it began changes nothing.
fn commit_box(app: &mut WordApp, o: &ObjectHit, pp: &PlacedPage, view: &CanvasView, after: &BoxState) {
    let c = after.quad.map(|q| pp.from_doc(view, q));
    let (w, h) = ((c[1] - c[0]).hypot() as f32, (c[3] - c[0]).hypot() as f32);
    let centre = ((c[0].x + c[2].x) / 2.0, (c[0].y + c[2].y) / 2.0);
    let deg = wordcraft_geom::normalize_degrees((c[1].y - c[0].y).atan2(c[1].x - c[0].x).to_degrees() as f32);
    let turned = (deg - o.spin.deg + 540.0).rem_euclid(360.0) - 180.0;
    if turned.abs() > 0.05 {
        let _ = app.run("arrange.rotation", json!({"degrees": deg}));
        return;
    }
    let r = wordcraft_geom::Rect::new((centre.0 - f64::from(w) / 2.0) as f32, (centre.1 - f64::from(h) / 2.0) as f32, w, h);
    let same = |a: f32, b: f32| (a - b).abs() < 0.01;
    if same(r.x, o.rect.x) && same(r.y, o.rect.y) && same(r.w, o.rect.w) && same(r.h, o.rect.h) {
        return;
    }
    // A chart or SmartArt takes a new size, never a new place.
    if size_only(app, o) {
        let _ = app.run("picture.size", json!({"width": r.w, "height": r.h, "lockAspect": false}));
    } else if o.floating() && movable(app, o) {
        let _ = app.run("arrange.bounds", json!({"width": r.w, "height": r.h, "page": o.page, "x": r.x, "y": r.y}));
    } else {
        let _ = app.run("arrange.bounds", json!({"width": r.w, "height": r.h}));
    }
}

/// The connection sites, on screen, of the shapes near a connector end dragged to `at` (page points),
/// and whether each is the one the end would glue to. A line's end doesn't glue, so it has none.
fn glue_sites(app: &WordApp, layout: &DocLayout, pp: &PlacedPage, o: &ObjectHit, at: (f32, f32)) -> Vec<(Pos2, bool)> {
    let snap = wordcraft_engine::cmd::connectors::SNAP;
    let glues = matches!(selected(app), Some((_, InlineObject::Shape { kind, .. })) if kind.is_connector());
    let mut sites: Vec<((f32, f32), f32)> = Vec::new();
    if glues && let Some(pg) = layout.pages.get(o.page) {
        for it in &pg.items {
            let wordcraft_layout::Placed::Object { rect, spin, story: StoryRef::Body, path, off, .. } = it else { continue };
            let pos = Pos { story: StoryRef::Body, path: path.clone(), off: *off };
            let Some(InlineObject::Shape { kind, .. }) = app.session.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)) else { continue };
            if kind.is_connector() || pos == o.pos() {
                continue;
            }
            let r = [rect.x, rect.y, rect.w, rect.h];
            // Sites of shapes the pointer is near.
            if at.0 < rect.x - 4.0 * snap || at.0 > rect.right() + 4.0 * snap || at.1 < rect.y - 4.0 * snap || at.1 > rect.bottom() + 4.0 * snap {
                continue;
            }
            for (i, _) in wordcraft_doc::connector::sites(*kind, rect.w, rect.h).iter().enumerate() {
                let Some(p) = wordcraft_doc::connector::site_point(*kind, r, *spin, i as u32) else { continue };
                let dist = ((p.0 - at.0).powi(2) + (p.1 - at.1).powi(2)).sqrt();
                sites.push((p, dist));
            }
        }
    }
    let best = sites.iter().enumerate().filter(|(_, s)| s.1 <= snap).min_by(|a, b| a.1.1.total_cmp(&b.1.1)).map(|(i, _)| i);
    sites.iter().enumerate().map(|(i, (p, _))| (pp.to_screen(pt(p.0, p.1)), best == Some(i))).collect()
}

/// Keys for a selected object: arrows nudge a floating one; typing or Enter goes into a text box;
/// Escape ends Create Link. Returns true if handled.
pub fn key(app: &mut WordApp, key: egui::Key, m: egui::Modifiers) -> bool {
    if key == egui::Key::Escape && app.canvas.link_from.take().is_some() {
        return true;
    }
    if selected(app).is_none() {
        return false;
    }
    let step = if m.ctrl || m.alt || m.command { NUDGE_FINE } else { NUDGE };
    let (dx, dy) = match key {
        egui::Key::ArrowLeft => (-step, 0.0),
        egui::Key::ArrowRight => (step, 0.0),
        egui::Key::ArrowUp => (0.0, -step),
        egui::Key::ArrowDown => (0.0, step),
        egui::Key::Enter => return enter_text_box(app),
        _ => return false,
    };
    // Inline objects move with the text: arrows move the caret as usual.
    app.run("arrange.nudge", json!({"dx": dx, "dy": dy})).is_ok()
}

/// With a text box selected, put the caret at the end of its text (typing goes there).
pub fn enter_text_box(app: &mut WordApp) -> bool {
    let Some(id) = selected_text_box(app) else { return false };
    // A box linked from another shows (and edits) the first box's story.
    let id = app.session.doc.text_box_chain(id).and_then(|c| c.first().copied()).unwrap_or(id);
    let end = app.session.doc.end_of(StoryRef::Part(id));
    app.run("caret.set", json!({"pos": end})).is_ok()
}

#[cfg(test)]
mod tests;
