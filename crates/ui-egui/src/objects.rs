//! Pictures, shapes and text boxes on the canvas: click one to select it, drag it to move it,
//! drag a handle to resize it, drag the rotation handle to turn it (Shift: 15° steps), arrow keys
//! nudge a floating one. The frame itself is [`crate::frame`]; changes go through
//! `arrange.bounds` / `arrange.rotation` / `arrange.nudge`, so they're undoable and scriptable like
//! everything else.
//!
//! While dragging, nothing is laid out: the preview is the object drawn alone, once, when the
//! drag starts (#1010: a cut of the page would carry along whatever is behind it). The page it
//! was on is drawn again without it (see [`dragged_off_page`]), so it is not left behind where it
//! was. The drop is one command, one relayout.
//!
//! Dragging an In Line with Text object moves it within the text, as in Word: a dashed insertion
//! cursor follows the pointer through the text (#2233) and the drop puts it there, still inline
//! (`arrange.bounds` / `arrange.copyTo` with `to`, #2234). A floating one goes where it's dropped.
//!
//! Moving a floating one snaps it to alignment guides ([`crate::guides`]): the page's edges and
//! centre, the margins and the other floating objects on the page, shown as thin lines while
//! dragging. Alt moves freely; File › Options › Advanced turns them off (`ui.alignmentGuides`).
//!
//! A selected connector shows a handle at each end instead of a frame: dragging one onto a shape
//! glues it to the nearest connection site there (`shape.connect`), elsewhere leaves it free.

use egui::{Color32, Painter, Pos2, Rect, Ui};
use serde_json::json;
use wordcraft_doc::para::{InlineObject, OBJ, Wrap};
use wordcraft_doc::{PartKind, Pos, StoryRef};
use wordcraft_layout::DocLayout;
use wordcraft_layout::hit::ObjectHit;

use crate::WordApp;
use crate::frame::{self, Drag, Grab};
use crate::guides;
use crate::theme::Tokens;

/// The band along a text box's border that grabs the box rather than its text, screen points.
const EDGE: f32 = 5.0;
/// How opaque the drag preview of an object's pixels is (0–255).
const PREVIEW_OPACITY: u8 = 170;
/// Arrow-key nudge, points (Ctrl/Alt: fine).
const NUDGE: f32 = 6.0;
const NUDGE_FINE: f32 = 1.0;

/// An object being dragged by its frame.
pub struct ObjectDrag {
    object: ObjectHit,
    drag: Drag,
    /// Smallest size it can be resized to, points.
    min: f32,
    picture: bool,
    /// A selected Behind Text picture overlaps text; wait for a drag threshold before taking the
    /// gesture so a simple click can still put the caret in that text.
    pending_behind: bool,
    /// Dragging a connector's end (false: its start, true: its end) to this page point.
    end: Option<(bool, (f32, f32))>,
    /// Ctrl/⌘ is held: dropping leaves the original and puts a copy at the new place.
    copy: bool,
    /// The alignment guides a move snapped to, shown while dragging ([`crate::guides`]).
    guides: Vec<crate::guides::Guide>,
    /// The object drawn alone over its rotated bounds, for the preview: made once it moves.
    preview: Option<egui::TextureHandle>,
    /// Moving an In Line with Text object: the text position under the pointer, where it lands,
    /// and the colour of the dashed insertion cursor drawn there.
    drop_at: Option<(Pos, Color32)>,
}

/// How near (screen points) a press must be to a connector's end to grab it.
const END_GRAB: f32 = 7.0;

/// The selected connector's start and end on its page (points), and its layout.
fn connector_ends(app: &WordApp, layout: &DocLayout) -> Option<(ObjectHit, (f32, f32), (f32, f32))> {
    let (pos, InlineObject::Shape { kind, float, .. }) = selected(app)? else { return None };
    if !kind.is_connector() {
        return None;
    }
    let (flip_h, flip_v) = (float.flip_h, float.flip_v);
    let o = layout.object(&pos, app.session.page_hint)?;
    let (a, b) = wordcraft_doc::connector::ends(o.rect.x, o.rect.y, o.rect.w, o.rect.h, flip_h, flip_v);
    Some((o, a, b))
}

fn point_on_screen(pages: &[Rect], scale: f32, page: usize, (x, y): (f32, f32)) -> Option<Pos2> {
    pages.get(page).map(|p| egui::pos2(p.min.x + x * scale, p.min.y + y * scale))
}

/// The selected connector's end under `at` (screen): which (false: start), and the connector.
fn connector_end_at(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<(ObjectHit, bool)> {
    let (o, a, b) = connector_ends(app, layout)?;
    let near = |p: (f32, f32)| point_on_screen(pages, scale, o.page, p).is_some_and(|s| s.distance(at) <= END_GRAB);
    if near(b) {
        Some((o, true))
    } else if near(a) {
        Some((o, false))
    } else {
        None
    }
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
    let pos = hit.pos();
    matches!(app.session.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)), Some(InlineObject::Graphic { .. })) && !size_only(app, hit)
}

/// Whether the object under `hit` is a chart or SmartArt that WordCraft can edit: it takes
/// resize handles and is laid out again at the new size, but it can't be moved or turned.
fn size_only(app: &WordApp, hit: &ObjectHit) -> bool {
    let pos = hit.pos();
    matches!(
        app.session.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)),
        Some(InlineObject::Graphic { graphic, .. }) if graphic.chart.is_some() || graphic.smart_art.is_some()
    )
}

/// The story of the selected text box.
fn selected_text_box(app: &WordApp) -> Option<u32> {
    match selected(app)? {
        (_, InlineObject::Shape { story: Some(id), .. }) if app.session.doc.parts.get(id).is_some_and(|p| p.kind == PartKind::TextBox) => Some(*id),
        _ => None,
    }
}

/// The object showing a frame, and whether its text is being edited (caret in a text box).
pub fn active(app: &WordApp, layout: &DocLayout) -> Option<(ObjectHit, bool)> {
    let hint = app.session.page_hint;
    if let Some((pos, _)) = selected(app) {
        return layout.object(&pos, hint).map(|o| (o, false));
    }
    match app.session.sel.focus.story {
        StoryRef::Part(id) if crate::canvas::in_text_box(app) => layout.text_box(id, hint).map(|o| (o, true)),
        _ => None,
    }
}

fn screen(pages: &[Rect], scale: f32, page: usize, r: wordcraft_geom::Rect) -> Option<Rect> {
    pages.get(page).map(|p| frame::to_screen(*p, scale, r))
}

/// Pressing, dragging, releasing or double-clicking on an object (or a frame handle). Returns
/// true when it was about an object, so the text doesn't also get it.
pub fn pointer(app: &mut WordApp, ui: &Ui, resp: &egui::Response, pages: &[Rect], layout: &DocLayout, scale: f32, at: Pos2) -> bool {
    if let Some(taken) = crate::crop::pointer(app, ui, resp, pages, layout, scale, at) {
        return taken;
    }
    if let Some(mut d) = app.canvas.obj_drag.take() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return true; // cancelled: dropped without a change
        }
        if d.pending_behind {
            if !ui.input(|i| i.pointer.primary_down()) {
                // No drag threshold was crossed: let the text click finish normally.
                app.canvas.cancel_text_drag();
                return false;
            }
            let shift = ui.input(|i| i.modifiers.shift);
            d.drag.update(at, pages, scale, d.min, d.picture != shift);
            if !d.drag.moved {
                app.canvas.obj_drag = Some(d);
                return false;
            }
            let pos = d.object.pos();
            let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
            let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
            // The text click path has been tracking the pointer until it became a drag.
            app.canvas.cancel_text_drag();
            d.pending_behind = false;
        }
        if let Some((which, _)) = d.end {
            if ui.input(|i| i.pointer.primary_down()) {
                if let Some((_, x, y)) = crate::canvas::page_at(pages, layout, scale, at).filter(|(p, ..)| *p == d.object.page) {
                    d.end = Some((which, (x, y)));
                    d.drag.moved = true;
                }
                app.canvas.obj_drag = Some(d);
            } else if d.drag.moved
                && let Some((_, (x, y))) = d.end
            {
                let key = if which { "end" } else { "start" };
                let _ = app.run("shape.connect", json!({key: {"x": x, "y": y, "page": d.object.page}}));
            }
            return true;
        }
        if ui.input(|i| i.pointer.primary_down()) {
            let shift = ui.input(|i| i.modifiers.shift);
            d.copy = ui.input(|i| i.modifiers.command) && d.drag.grab == Grab::Move;
            if d.drag.grab == Grab::Rotate {
                d.drag.turn_to(at, pages, scale, shift);
            } else {
                // Pictures keep their proportions from a corner unless Shift; shapes only with Shift.
                d.drag.update(at, pages, scale, d.min, d.picture != shift);
                // Alignment guides: a move snaps to the page, the margins and the other objects
                // (Alt: freely).
                d.guides.clear();
                if d.drag.grab == Grab::Move && d.drag.moved && !d.object.floating() {
                    // An inline one lands in the text: the insertion cursor follows the pointer.
                    d.drop_at = drop_point(app, layout, pages, scale, at, &d);
                } else if d.drag.grab == Grab::Move && d.drag.moved && app.ui.alignment_guides && !ui.input(|i| i.modifiers.alt) {
                    snap_move(layout, &mut d, scale);
                }
            }
            if d.drag.moved && d.preview.is_none() && shows_pixels(&d) {
                d.preview = preview(app, ui.ctx(), layout, &d.object, scale);
            }
            app.canvas.obj_drag = Some(d);
        } else {
            drop(app, &d);
        }
        return true;
    }
    let pressed = ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer();
    if app.canvas.link_from.is_some() && (pressed || ui.input(|i| i.key_pressed(egui::Key::Escape))) {
        link_click(app, layout, pages, scale, at, pressed);
        return true;
    }
    let multi = resp.double_clicked() || resp.triple_clicked();
    if !(pressed || multi) {
        return false;
    }
    // A selected connector's end: drag it to glue it.
    if pressed && let Some((object, which)) = connector_end_at(app, layout, pages, scale, at) {
        let drag = Drag::new(Grab::Move, object.page, object.rect, 0.0, at);
        app.canvas.obj_drag = Some(ObjectDrag {
            drag,
            object,
            min: 0.0,
            picture: false,
            pending_behind: false,
            end: Some((which, (0.0, 0.0))),
            guides: Vec::new(),
            preview: None,
            copy: false,
            drop_at: None,
        });
        return true;
    }
    let grabbed = grab_at(app, layout, pages, scale, at).or_else(|| pressed.then(|| behind_drag_at(app, layout, pages, scale, at)).flatten());
    let Some((object, grab)) = grabbed else { return false };
    // Double-click a SmartArt graphic (selected by the first click) to edit its text.
    if resp.double_clicked() && wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).is_some() {
        let _ = app.run("smartArt.textPane", json!({"value": true}));
        return true;
    }
    // Shift/Ctrl+click adds an object to the selected ones (or takes it out), for Group.
    let adding = ui.input(|i| i.modifiers.shift || i.modifiers.command);
    // Ctrl on the object that is already selected starts a copy-drag instead.
    let again = selected(app).is_some_and(|(p, _)| p == object.pos());
    if pressed && adding && grab == Grab::Move && selected(app).is_some() && (!again || ui.input(|i| i.modifiers.shift)) {
        let _ = app.run("select.addObject", json!({"pos": object.pos()}));
        return true;
    }
    if pressed && grab == Grab::Move && object.behind() && behind_draggable(app, &object) {
        let (min, picture) = match app.session.doc.para_at(&object.pos()).and_then(|p| p.object_at(object.pos().off)) {
            Some(o) => (wordcraft_engine::cmd::objects::min_size(o), matches!(o, InlineObject::Image { .. })),
            None => return false,
        };
        app.canvas.obj_drag = Some(ObjectDrag {
            drag: Drag::new(grab, object.page, object.rect, object.spin.deg, at),
            object,
            min,
            picture,
            pending_behind: true,
            end: None,
            guides: Vec::new(),
            preview: None,
            copy: false,
            drop_at: None,
        });
        return false;
    }
    if pressed {
        let pos = object.pos();
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
        let (min, picture) = match selected(app) {
            Some((_, o)) => (wordcraft_engine::cmd::objects::min_size(o), matches!(o, InlineObject::Image { .. })),
            None => return true,
        };
        // Objects in table cells can be selected and resized, not moved; charts neither.
        let size_only = size_only(app, &object);
        if !fixed(app, &object) && (grab != Grab::Move || (movable(app, &object) && !size_only)) {
            let copy = grab == Grab::Move && ui.input(|i| i.modifiers.command);
            app.canvas.obj_drag = Some(ObjectDrag {
                drag: Drag::new(grab, object.page, object.rect, object.spin.deg, at),
                object,
                min,
                picture,
                pending_behind: false,
                end: None,
                guides: Vec::new(),
                preview: None,
                copy,
                drop_at: None,
            });
        }
    }
    true
}

/// The object a move has taken off its page (which page, and where it is in the document): the
/// page is rendered again without it while the preview follows the pointer, so the object is not
/// left behind where it was. Only for a move — resizing and turning show the object itself, and
/// dragging a connector's end onto a shape still shows that connector.
pub(crate) fn dragged_off_page(drag: Option<&ObjectDrag>) -> Option<(usize, Pos)> {
    let d = drag?;
    if !d.drag.moved || d.end.is_some() || d.drag.grab != Grab::Move {
        return None;
    }
    Some((d.object.page, d.object.pos()))
}

/// Where an In Line with Text object dragged to `at` (screen) lands: the nearest position of its
/// own story's text there, with the insertion cursor's colour (the same contrast as the caret's).
fn drop_point(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2, d: &ObjectDrag) -> Option<(Pos, Color32)> {
    let (page, x, y) = crate::canvas::page_at(pages, layout, scale, at)?;
    let pos = layout.hit(page, x, y, d.object.story)?;
    if let Some((was, color)) = &d.drop_at
        && *was == pos
    {
        return Some((pos, *color)); // the paper under it is worked out once per position
    }
    let c = layout.caret_on(&pos, page)?;
    Some((pos, crate::canvas::caret_color_on(app, layout, c.page, c.x, c.top + c.height / 2.0)))
}

/// Snap a moved object's bounds (rotated ones by their turned box) to the guides of the page
/// it's over: the page's edges and centre, its margins, the other floating objects there.
fn snap_move(layout: &DocLayout, d: &mut ObjectDrag, scale: f32) {
    let Some(pg) = layout.pages.get(d.drag.to_page) else { return };
    let candidates = guides::candidates(paper_width(pg), pg.h, pg.body, snap_targets(pg, &d.object));
    let s = guides::snap(d.object.spin.bounds(d.drag.rect), &candidates, guides::SNAP_SCREEN / scale.max(0.01));
    d.drag.rect.x += s.dx;
    d.drag.rect.y += s.dy;
    d.guides = s.lines;
}

/// The page's own width, without the comment balloons' markup area.
fn paper_width(pg: &wordcraft_layout::Page) -> f32 {
    (pg.w - pg.markup).max(0.0)
}

/// The bounds of the floating objects on `pg` that `me` can snap to: all but itself and, for a
/// text box, the objects in its text (they move with it).
fn snap_targets(pg: &wordcraft_layout::Page, me: &ObjectHit) -> Vec<wordcraft_geom::Rect> {
    let pos = me.pos();
    let own = me.text_box.map(StoryRef::Part);
    pg.items
        .iter()
        .chain(&pg.header)
        .chain(&pg.footer)
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Object { rect, spin, story, path, off, wrap, .. }
                if *wrap != Wrap::Inline && !(*story == pos.story && *path == pos.path && *off == pos.off) && Some(*story) != own =>
            {
                Some(spin.bounds(*rect))
            }
            _ => None,
        })
        .collect()
}

/// The guides a move snapped to: thin lines across the page (not into the balloon area).
fn paint_guides(d: &ObjectDrag, painter: &Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) {
    let Some(p) = pages.get(d.drag.to_page) else { return };
    let right = layout.pages.get(d.drag.to_page).map_or(p.max.x, |pg| (p.min.x + paper_width(pg) * scale).min(p.max.x));
    let stroke = egui::Stroke::new(1.0, t.guide);
    for g in &d.guides {
        let line = match g.axis {
            guides::Axis::X => [egui::pos2(p.min.x + g.at * scale, p.min.y), egui::pos2(p.min.x + g.at * scale, p.max.y)],
            guides::Axis::Y => [egui::pos2(p.min.x, p.min.y + g.at * scale), egui::pos2(right, p.min.y + g.at * scale)],
        };
        painter.line_segment(line, stroke);
    }
}

/// Create Link's click: link the box it started from to the text box under `at` (anywhere on it),
/// if `pressed`; either way, linking is over.
fn link_click(app: &mut WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2, pressed: bool) {
    let Some(from) = app.canvas.link_from.take() else { return };
    if !pressed {
        return;
    }
    let target = crate::canvas::page_at(pages, layout, scale, at).and_then(|(page, x, y)| {
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
pub fn select_at(app: &mut WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> bool {
    if crate::canvas::editing_header_footer(app, layout) {
        return false;
    }
    let Some((object, _)) = grab_at(app, layout, pages, scale, at) else { return false };
    let pos = object.pos();
    // Already selected (alone or among several picked for Group): keep the selection.
    if !wordcraft_engine::cmd::objects::picked_objects(&app.session).contains(&pos) {
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
    }
    true
}

/// What a press at `at` would grab: a handle of the shown frame, else an object of the story
/// being edited (body, or the header/footer).
fn grab_at(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<(ObjectHit, Grab)> {
    let story = active_story(app, layout);
    let handle = active(app, layout).filter(|(o, _)| o.story == story && !fixed(app, o)).and_then(|(o, _)| {
        let f = screen(pages, scale, o.page, o.rect)?;
        if !size_only(app, &o) && frame::rotate_handle_at(f, o.spin.deg, at) {
            return Some((o, Grab::Rotate));
        }
        frame::handle_at(f, o.spin.deg, at).map(|h| (o, Grab::Resize(h)))
    });
    handle.or_else(|| {
        let (page, x, y) = crate::canvas::page_at(pages, layout, scale, at)?;
        // A Behind Text picture is normally hit only where no body text is present, so text above
        // it remains easy to edit. Once the picture is selected (for example with the arrow keys
        // or the Selection Pane), let its own frame win inside the frame so the user can drag it
        // directly without first finding an uncovered patch.
        let selected_picture = selected(app)
            .filter(|(_, obj)| behind_body_drag(obj))
            .and_then(|(pos, _)| layout.object(&pos, app.session.page_hint))
            .filter(|o| o.behind() && o.page == page && movable(app, o) && o.rect.expand(EDGE / scale.max(0.01)).contains(o.unspin(x, y)))
            .map(|o| (o, Grab::Move));
        if selected_picture.is_some() {
            return selected_picture;
        }
        // Selectable: body objects (table cells too) or the header/footer's own objects;
        // notes' objects aren't yet.
        layout.object_at_in(page, x, y, EDGE / scale.max(0.01), story).map(|o| (o, Grab::Move))
    })
}

/// Objects that can be dragged to a new place: those in the own paragraphs of the body or of a
/// header/footer.
fn movable(app: &WordApp, o: &ObjectHit) -> bool {
    o.path.depth() == 0 && selectable_story(app, o.story)
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
fn behind_drag_at(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<(ObjectHit, Grab)> {
    let story = active_story(app, layout);
    let (page, x, y) = crate::canvas::page_at(pages, layout, scale, at)?;
    let edge = EDGE / scale.max(0.01);
    layout
        .find_object(page, |o| {
            o.page == page
                && o.story == story
                && o.behind()
                && movable(app, o)
                && behind_draggable(app, o)
                && o.rect.expand(edge).contains(o.unspin(x, y))
        })
        .map(|o| (o, Grab::Move))
}

/// Whether a drag shows the object's pixels rather than an outline. Resizing reflows a text
/// box (and redraws a shape). Text-box text doesn't yet follow its frame's committed rotation,
/// so turning one keeps the outline preview rather than promising rotated text on release.
fn shows_pixels(d: &ObjectDrag) -> bool {
    (d.picture && d.object.spin.is_identity()) || d.drag.grab == Grab::Move || (d.drag.grab == Grab::Rotate && d.object.text_box.is_none())
}

/// `o` drawn alone over its rotated bounds, as a texture, at the canvas's `scale` (screen points
/// per page point).
fn preview(app: &WordApp, ctx: &egui::Context, layout: &DocLayout, o: &ObjectHit, scale: f32) -> Option<egui::TextureHandle> {
    let obj = app.session.doc.para_at(&o.pos()).and_then(|p| p.object_at(o.off))?;
    let page = layout.pages.get(o.page)?;
    let mut opts = crate::canvas::screen_render_options();
    opts.dark = app.session.view.dark_mode;
    let items = wordcraft_layout::display::object_display(&app.session.doc, page, obj, o.rect, o.text_box, &opts.display);
    let b = o.bounds();
    let img = wordcraft_render::render_items(&app.session.doc, &items, b.x, b.y, b.w, b.h, scale * ctx.pixels_per_point(), &opts);
    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_straight());
    Some(ctx.load_texture("object-drag", ci, egui::TextureOptions::LINEAR))
}

/// Commit a finished drag as one command.
fn drop(app: &mut WordApp, d: &ObjectDrag) {
    if !d.drag.moved {
        return;
    }
    let r = d.drag.rect;
    // A chart or SmartArt takes a new size, never a new place.
    if matches!(d.drag.grab, Grab::Resize(_)) && size_only(app, &d.object) {
        let _ = app.run("picture.size", json!({"width": r.w, "height": r.h, "lockAspect": false}));
        return;
    }
    let params = match d.drag.grab {
        Grab::Rotate => {
            let _ = app.run("arrange.rotation", json!({"degrees": d.drag.deg}));
            return;
        }
        // An In Line with Text object stays in the text, at the insertion cursor; dropped on
        // itself, it doesn't move.
        Grab::Move if !d.object.floating() => {
            let Some((to, _)) = &d.drop_at else { return };
            let pos = d.object.pos();
            let on_itself = to.story == pos.story && to.path == pos.path && (to.off == pos.off || to.off == pos.off + OBJ.len_utf8());
            if d.copy {
                let _ = app.run("arrange.copyTo", json!({"to": to}));
            } else if !on_itself {
                let _ = app.run("arrange.bounds", json!({"to": to}));
            }
            return;
        }
        Grab::Move if d.copy => {
            let _ = app.run("arrange.copyTo", json!({"page": d.drag.to_page, "x": r.x, "y": r.y}));
            return;
        }
        Grab::Move => json!({"page": d.drag.to_page, "x": r.x, "y": r.y}),
        // An inline object stays in the text; a floating one keeps its far edges put.
        Grab::Resize(_) if d.object.floating() && movable(app, &d.object) => {
            json!({"width": r.w, "height": r.h, "page": d.drag.page, "x": r.x, "y": r.y})
        }
        Grab::Resize(_) => json!({"width": r.w, "height": r.h}),
    };
    let _ = app.run("arrange.bounds", params);
}

/// Draw the frame of the active object, or the drag preview.
pub fn paint(app: &WordApp, painter: &Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) {
    if crate::crop::paint(app, painter, t, layout, pages, scale) {
        return;
    }
    if let Some(d) = app.canvas.obj_drag.as_ref().filter(|d| d.end.is_some()) {
        paint_end_drag(app, d, painter, t, layout, pages, scale);
        return;
    }
    if let Some((o, a, b)) = connector_ends(app, layout) {
        // A connector: a handle at each end (filled when glued).
        let glued = match selected(app) {
            Some((_, InlineObject::Shape { extra, .. })) => (extra.start.is_some(), extra.end.is_some()),
            _ => (false, false),
        };
        for (p, on) in [(a, glued.0), (b, glued.1)] {
            if let Some(c) = point_on_screen(pages, scale, o.page, p) {
                painter.circle(c, 4.5, if on { t.accent } else { Color32::WHITE }, egui::Stroke::new(1.2, t.accent));
            }
        }
        return;
    }
    if let Some(d) = app.canvas.obj_drag.as_ref().filter(|d| d.drag.moved) {
        let Some(to) = screen(pages, scale, d.drag.to_page, d.drag.rect) else { return };
        if d.drag.grab == Grab::Rotate {
            if let Some(tex) = &d.preview
                && let Some(bounds) = screen(pages, scale, d.drag.page, d.object.bounds())
            {
                // The cached pixels already contain the starting rotation and flips. Turn
                // only by the additional angle; changing UVs would mirror them a second time.
                let mut mesh = egui::Mesh::with_texture(tex.id());
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                mesh.add_rect_with_uv(bounds, uv, Color32::from_white_alpha(PREVIEW_OPACITY));
                let delta = wordcraft_geom::normalize_degrees(d.drag.deg - d.object.spin.deg);
                mesh.rotate(egui::emath::Rot2::from_angle(delta.to_radians()), to.center());
                painter.add(egui::Shape::mesh(mesh));
            }
            frame::paint_turning(painter, t, to, d.drag.deg);
            return;
        }
        if !shows_pixels(d) {
            frame::paint_outline(painter, t, to, d.drag.deg);
        } else if let Some(tex) = &d.preview {
            // A turned object's pixels fill its rotated bounds, which move with it.
            let r = d.object.bounds();
            let (dx, dy) = (d.drag.rect.x - d.drag.start.x, d.drag.rect.y - d.drag.start.y);
            let moved = wordcraft_geom::Rect::new(r.x + dx, r.y + dy, r.w, r.h);
            let to = if d.object.spin.is_identity() { to } else { screen(pages, scale, d.drag.to_page, moved).unwrap_or(to) };
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(tex.id(), to, uv, Color32::from_white_alpha(PREVIEW_OPACITY));
        }
        frame::paint(painter, t, to, d.drag.deg, false, false, false);
        paint_guides(d, painter, t, layout, pages, scale);
        paint_drop_caret(d, painter, layout, pages, scale);
        return;
    }
    // The others selected along with it (Shift+click): their outlines.
    for p in &app.session.also_selected {
        if let Some(o) = layout.object(p, app.session.page_hint)
            && let Some(f) = screen(pages, scale, o.page, o.rect)
        {
            frame::paint(painter, t, f, o.spin.deg, false, false, false);
        }
    }
    if let Some((o, editing)) = active(app, layout)
        && let Some(f) = screen(pages, scale, o.page, o.rect)
    {
        // Handles where it can be resized and turned (body and header/footer objects,
        // not charts): else a plain frame.
        let handles = selectable_story(app, o.story) && !fixed(app, &o);
        frame::paint(painter, t, f, o.spin.deg, editing, handles, handles && !size_only(app, &o));
    }
}

/// The dashed insertion cursor where a dragged In Line with Text object will land.
fn paint_drop_caret(d: &ObjectDrag, painter: &Painter, layout: &DocLayout, pages: &[Rect], scale: f32) {
    let Some((pos, color)) = &d.drop_at else { return };
    let Some(c) = layout.caret_on(pos, d.drag.to_page) else { return };
    let Some(r) = screen(pages, scale, c.page, wordcraft_geom::Rect::new(c.x, c.top, 0.0, c.height)) else { return };
    painter.extend(egui::Shape::dashed_line(&[r.left_top(), r.left_bottom()], egui::Stroke::new(1.5, *color), 2.0, 2.0));
}

/// A connector's end being dragged: a line from its other end to the pointer, and the connection
/// sites of the shapes near the pointer (the one it would glue to, filled).
fn paint_end_drag(app: &WordApp, d: &ObjectDrag, painter: &Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) {
    let Some((which, at)) = d.end.filter(|_| d.drag.moved) else { return };
    let Some((_, a, b)) = connector_ends(app, layout) else { return };
    let fixed = if which { a } else { b };
    let snap = wordcraft_engine::cmd::connectors::SNAP;
    let mut best: Option<((f32, f32), f32)> = None;
    if let Some(pg) = layout.pages.get(d.object.page) {
        for it in &pg.items {
            let wordcraft_layout::Placed::Object { rect, spin, story: StoryRef::Body, path, off, .. } = it else { continue };
            let pos = Pos { story: StoryRef::Body, path: path.clone(), off: *off };
            let Some(InlineObject::Shape { kind, .. }) = app.session.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)) else { continue };
            if kind.is_connector() || pos == d.object.pos() {
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
                if let Some(c) = point_on_screen(pages, scale, d.object.page, p) {
                    painter.circle_stroke(c, 3.5, egui::Stroke::new(1.0, t.accent));
                }
                if dist <= snap && best.is_none_or(|b| dist < b.1) {
                    best = Some((p, dist));
                }
            }
        }
    }
    let to = best.map_or(at, |b| b.0);
    if let (Some(f), Some(e)) = (point_on_screen(pages, scale, d.object.page, fixed), point_on_screen(pages, scale, d.object.page, to)) {
        painter.line_segment([f, e], egui::Stroke::new(1.5, t.accent));
        painter.circle(e, 4.5, if best.is_some() { t.accent } else { Color32::WHITE }, egui::Stroke::new(1.2, t.accent));
    }
}

/// The pointer over an object or a handle (or dragging): its cursor.
pub fn cursor(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<egui::CursorIcon> {
    if let Some(c) = crate::crop::cursor(app, layout, pages, scale, at) {
        return Some(c);
    }
    if let Some(d) = &app.canvas.obj_drag {
        return Some(if d.end.is_some() {
            egui::CursorIcon::Crosshair
        } else if d.copy && d.drag.moved {
            egui::CursorIcon::Copy
        } else {
            d.drag.grab.cursor()
        });
    }
    if connector_end_at(app, layout, pages, scale, at).is_some() {
        return Some(egui::CursorIcon::Crosshair);
    }
    grab_at(app, layout, pages, scale, at).map(|(o, g)| hover_cursor(app, &o, g))
}

/// The cursor over part `g` of object `o`: none of a drag for an object that won't move there (a
/// fixed graphic, or the body of a chart or SmartArt, which only resizes).
fn hover_cursor(app: &WordApp, o: &ObjectHit, g: Grab) -> egui::CursorIcon {
    if fixed(app, o) || (g == Grab::Move && size_only(app, o)) { egui::CursorIcon::Default } else { g.cursor() }
}

/// Keys for a selected object: arrows nudge a floating one; typing or Enter goes into a text box.
/// Returns true if handled.
pub fn key(app: &mut WordApp, key: egui::Key, m: egui::Modifiers) -> bool {
    // Crop mode takes Esc and Enter (leaving it, the crop kept).
    if crate::crop::key(app, key) {
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
mod tests {
    use std::sync::Arc;

    use wordcraft_doc::para::{InlineObject, ShapeKind};
    use wordcraft_doc::{Document, Pos};

    use super::*;

    /// An app whose first paragraph holds `obj`, and where it's laid out.
    fn app_with(obj: InlineObject) -> (WordApp, ObjectHit) {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        app.session.doc.insert_object(&Pos::body(0, 0), obj, &Default::default()).unwrap();
        let hit = app.session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
        (app, hit)
    }

    #[test]
    fn charts_get_no_handles_or_drag_but_shapes_do() {
        let chart =
            InlineObject::Graphic { w: 200.0, h: 100.0, alt: String::new(), float: Default::default(), graphic: Arc::new(Default::default()) };
        let (app, hit) = app_with(chart);
        assert!(fixed(&app, &hit));
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 50.0,
            h: 50.0,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float: Default::default(),
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        let (app, hit) = app_with(shape);
        assert!(!fixed(&app, &hit));
    }

    #[test]
    fn editable_smart_art_takes_resize_handles_but_not_a_move() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        app.run("insert.smartArt", json!({"layout": "pyramid", "items": ["a", "b"]})).unwrap();
        let hit = app.session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
        assert!(!fixed(&app, &hit));
        assert!(size_only(&app, &hit));
        // Its body shows no move cursor; its handles show theirs.
        assert_eq!(hover_cursor(&app, &hit, Grab::Move), egui::CursorIcon::Default);
        assert_ne!(hover_cursor(&app, &hit, Grab::Resize(frame::Handle { x: 1, y: 0 })), egui::CursorIcon::Default);
        let (w0, h0) = (hit.rect.w, hit.rect.h);
        assert!(app.run("arrange.bounds", json!({"width": w0 / 2.0, "height": h0 / 2.0})).is_err());
        app.run("picture.size", json!({"width": w0 / 2.0, "height": h0 / 2.0, "lockAspect": false})).unwrap();
        let hit = app.session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
        assert!((hit.rect.w - w0 / 2.0).abs() < 1.0);
    }
    /// Alignment guides: dragging a floating shape to within a few points of the page's centre
    /// snaps its centre exactly onto it and shows the guide while dragging; the drop lands there,
    /// and the guides go. Holding Alt drags freely.
    #[test]
    fn dragging_a_shape_snaps_to_the_page_centre_unless_alt() {
        use egui::{Event, Modifiers, PointerButton};
        let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        a.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 50})).unwrap();
        a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let page = h.state_mut().session.layout().pages.first().map(|p| p.w - p.markup).unwrap();
        let centre = page / 2.0;
        let shape = |h: &mut egui_kittest::Harness<'_, WordApp>| h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
        let button =
            |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        // Press on the shape's centre and drag it so its centre ends 1.5 pt right of the page's.
        let drag = |h: &mut egui_kittest::Harness<'_, WordApp>, alt: bool| {
            let r = shape(h);
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let from = crate::canvas::page_to_screen(h.state_mut(), 0, cx, cy).unwrap();
            let to = crate::canvas::page_to_screen(h.state_mut(), 0, centre + 1.5, cy).unwrap();
            h.event(Event::ModifiersChanged(if alt { Modifiers::ALT } else { Modifiers::default() }));
            h.event(Event::PointerMoved(from));
            h.event(button(from, true));
            h.step();
            // Out past the drag threshold, then to the target.
            for at in [from + egui::vec2(0.0, 40.0), to + egui::vec2(0.0, 40.0), to] {
                h.event(Event::PointerMoved(at));
                h.step();
            }
            let d = h.state().canvas.obj_drag.as_ref().expect("dragging");
            let mid = (d.drag.rect, d.guides.clone());
            h.event(button(to, false));
            for _ in 0..3 {
                h.step();
            }
            h.event(Event::ModifiersChanged(Modifiers::default()));
            h.step();
            mid
        };
        let (rect, guides) = drag(&mut h, false);
        assert!((rect.x + rect.w / 2.0 - centre).abs() < 1e-3, "snapped onto the page's centre while dragging: {rect:?}");
        assert!(guides.contains(&crate::guides::Guide::x(centre)), "the centre guide shows: {guides:?}");
        assert!(h.state().canvas.obj_drag.is_none(), "dropped: no guides any more");
        let r = shape(&mut h);
        assert!((r.x + r.w / 2.0 - centre).abs() < 0.01, "dropped on the centre: {r:?}");
        // Alt: no snapping, no guides; it lands where the pointer put it.
        let (rect, guides) = drag(&mut h, true);
        assert!((rect.x + rect.w / 2.0 - centre - 1.5).abs() < 0.01, "free with Alt: {rect:?}");
        assert!(guides.is_empty(), "{guides:?}");
        let r = shape(&mut h);
        assert!((r.x + r.w / 2.0 - centre - 1.5).abs() < 0.01, "{r:?}");
    }
    /// #1738: a floating shape dragged from page 1 onto page 2 leaves the view where it is, even
    /// though the command that drops it (`arrange.bounds`) is not a view command.
    #[test]
    fn dropping_a_shape_on_another_page_keeps_the_view_where_it_is() {
        use egui::{Event, PointerButton};
        let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        a.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 50})).unwrap();
        a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
        let end = Pos::body(0, wordcraft_doc::para::OBJ.len_utf8());
        a.run("caret.set", json!({"pos": end})).unwrap();
        a.run("text.insert", json!({"text": "one"})).unwrap();
        a.run("text.newParagraph", json!({})).unwrap();
        a.run("para.pageBreakBefore", json!({})).unwrap();
        a.run("text.insert", json!({"text": "two"})).unwrap();
        a.run("view.zoom", json!({"value": 30})).unwrap();
        a.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": end})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let r = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
        let from = crate::canvas::page_to_screen(h.state_mut(), 0, r.x + r.w / 2.0, r.y + r.h / 2.0).unwrap();
        let to = crate::canvas::page_to_screen(h.state_mut(), 1, 200.0, 300.0).unwrap();
        let button =
            |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        h.event(Event::PointerMoved(from));
        h.event(button(from, true));
        h.step();
        for k in 1..=4 {
            h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 4.0)));
            h.step();
        }
        h.state_mut().canvas.scroll_to_caret = false;
        h.event(button(to, false));
        h.step();
        h.step();
        let landed = h.state_mut().session.layout().object_at(1, 200.0, 300.0, 0.0).is_some();
        assert!(landed, "the shape landed on page 2");
        assert!(!h.state().canvas.scroll_to_caret, "the drop scrolled to the caret");
    }

    /// A floating rectangle shape, `w` × `h`.
    fn floating_rect(w: f32, h: f32) -> InlineObject {
        InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w,
            h,
            fill: None,
            stroke: None,
            stroke_width: 1.0,
            float: wordcraft_doc::para::Float { wrap: Wrap::InFrontOfText, x: 10.0, y: 10.0, ..Default::default() },
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        }
    }

    /// A text box doesn't snap to the objects inside it (they move with it).
    #[test]
    fn a_text_box_does_not_snap_to_its_own_contents() {
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        app.run("insert.textBox", json!({"text": "Box"})).unwrap();
        let bx = app.session.layout().find_object(0, |o| o.text_box.is_some()).expect("the box");
        let end = Pos { off: bx.off + OBJ.len_utf8(), ..bx.pos() };
        app.run("select.range", json!({"anchor": bx.pos(), "focus": end})).unwrap();
        app.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 200, "width": 200, "height": 150})).unwrap();
        let id = app.session.layout().find_object(0, |o| o.text_box.is_some()).and_then(|o| o.text_box).unwrap();
        let inner = Pos { story: StoryRef::Part(id), path: wordcraft_doc::Path::top(0), off: 0 };
        app.session.doc.insert_object(&inner, floating_rect(30.0, 20.0), &Default::default()).unwrap();
        app.session.touch();
        let layout = app.session.layout();
        let bx = layout.find_object(0, |o| o.text_box.is_some()).unwrap();
        let pg = layout.pages.first().unwrap();
        let inside = layout.find_object(0, |o| o.story == StoryRef::Part(id)).expect("the box's own object is laid out");
        let targets = snap_targets(pg, &bx);
        assert!(!targets.contains(&inside.bounds()), "{targets:?}");
        // Another floating object on the page is still a target.
        let other = snap_targets(pg, &inside);
        assert!(other.contains(&bx.bounds()), "{other:?}");
    }

    /// Pressing Alt in the middle of a drag stops the snapping, and letting it go again doesn't
    /// turn keytips on (the Alt was part of the drag, not a tap).
    #[test]
    fn alt_pressed_during_a_drag_unsnaps_and_keeps_keytips_off() {
        use egui::{Event, Modifiers, PointerButton};
        let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        a.run("insert.shape", json!({"kind": "rectangle", "width": 100, "height": 50})).unwrap();
        a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let centre = h.state_mut().session.layout().pages.first().map(|p| (p.w - p.markup) / 2.0).unwrap();
        let r = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap().rect;
        let cy = r.y + r.h / 2.0;
        let from = crate::canvas::page_to_screen(h.state_mut(), 0, r.x + r.w / 2.0, cy).unwrap();
        let to = crate::canvas::page_to_screen(h.state_mut(), 0, centre + 1.5, cy).unwrap();
        let button =
            |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        h.event(Event::PointerMoved(from));
        h.event(button(from, true));
        h.step();
        for at in [from + egui::vec2(0.0, 40.0), to] {
            h.event(Event::PointerMoved(at));
            h.step();
        }
        assert!(!h.state().canvas.obj_drag.as_ref().expect("dragging").guides.is_empty(), "snapped before Alt");
        // Alt down mid-drag, as a key and a modifier change (backends send both).
        let alt_key =
            |pressed: bool, modifiers: Modifiers| Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed, repeat: false, modifiers };
        h.event(alt_key(true, Modifiers::ALT));
        h.event(Event::ModifiersChanged(Modifiers::ALT));
        h.event(Event::PointerMoved(to + egui::vec2(0.0, 1.0)));
        h.step();
        let d = h.state().canvas.obj_drag.as_ref().expect("still dragging");
        assert!(d.guides.is_empty() && (d.drag.rect.x + d.drag.rect.w / 2.0 - centre - 1.5).abs() < 0.01, "free with Alt");
        // Alt up, then the release.
        h.event(alt_key(false, Modifiers::default()));
        h.event(Event::ModifiersChanged(Modifiers::default()));
        h.event(button(to, false));
        for _ in 0..3 {
            h.step();
        }
        assert!(h.state().canvas.obj_drag.is_none());
        assert_eq!(h.state().ui.keytips, crate::keytips::Phase::Off, "Alt during a drag isn't a keytip tap");
    }

    /// #2233, #2234: dragging an In Line with Text picture into another paragraph shows a dashed
    /// insertion cursor at the text under the pointer (no alignment guides), and the drop puts the
    /// picture there, still inline and selected. Dropped back where it was, nothing changes.
    #[test]
    fn an_inline_picture_drags_to_the_insertion_cursor_and_stays_inline() {
        use crate::table_resize::tests::{H, button, steps};
        let mut a = WordApp::new(
            wordcraft_engine::Session::new(Document::from_text("The first paragraph of text.\nThe second paragraph, where the picture starts.")),
            Default::default(),
        );
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        let picture =
            InlineObject::Image { media: String::new(), w: 80.0, h: 40.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
        a.session.doc.insert_object(&Pos::body(1, 0), picture, &Default::default()).unwrap();
        a.session.touch();
        let mut h: H = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        steps(&mut h, 6);
        let o = h.state_mut().session.layout().object(&Pos::body(1, 0), 0).expect("laid out");
        assert_eq!(o.wrap, Wrap::Inline);
        // Aim between "first" and " paragraph" in the first paragraph.
        let target = Pos::body(0, "The first".len());
        let c = h.state_mut().session.layout().caret(&target).expect("caret");
        let at = |h: &mut H, x: f32, y: f32| crate::canvas::page_to_screen(h.state_mut(), 0, x, y).unwrap();
        let start = at(&mut h, o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
        let end = at(&mut h, c.x + 0.5, c.top + c.height / 2.0);
        h.event(egui::Event::PointerMoved(start));
        h.step();
        button(&h, start, true);
        h.step();
        for k in 1..=5 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
            h.step();
        }
        let d = h.state().canvas.obj_drag.as_ref().expect("dragging");
        assert_eq!(d.drop_at.as_ref().map(|(p, _)| p.clone()), Some(target.clone()), "the insertion cursor is at the text under the pointer");
        assert!(d.guides.is_empty(), "an inline object doesn't snap to guides");
        button(&h, end, false);
        steps(&mut h, 3);
        assert!(h.state().canvas.obj_drag.is_none());
        let (pos, obj) = selected(h.state()).expect("still selected");
        assert_eq!(pos, target, "landed at the insertion cursor");
        assert!(!obj.is_floating(), "still In Line with Text");
        assert_eq!(h.state().session.doc.para_at(&Pos::body(1, 0)).map(|p| p.objects.len()), Some(0), "left its old place");
        // Back onto itself: no change, no undo step.
        let undo = h.state().session.undo_depth();
        let o = h.state_mut().session.layout().object(&target, 0).expect("laid out");
        let start = at(&mut h, o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
        button(&h, start, true);
        h.step();
        for v in [egui::vec2(12.0, 0.0), egui::vec2(0.0, 0.0)] {
            h.event(egui::Event::PointerMoved(start + v));
            h.step();
        }
        button(&h, start, false);
        steps(&mut h, 3);
        assert_eq!(selected(h.state()).map(|(p, _)| p), Some(target));
        assert_eq!(h.state().session.undo_depth(), undo);
    }

    /// #1010: dragging a shape off text, the preview is the shape alone, not the text that was
    /// behind it. An unfilled red rectangle in front of black text is dragged down into the empty
    /// page: its preview has its red outline and nothing dark inside.
    #[test]
    fn a_dragged_shape_leaves_the_text_behind_it() {
        use crate::table_resize::tests::{H, button, steps};
        let text = "MMMMMMMMMMMM ".repeat(12);
        let mut a = WordApp::new(wordcraft_engine::Session::new(Document::from_text(&text)), Default::default());
        let shape = InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 120.0,
            h: 60.0,
            fill: None,
            stroke: Some(wordcraft_doc::Rgb(0xFF, 0, 0)),
            stroke_width: 3.0,
            float: wordcraft_doc::para::Float { wrap: wordcraft_doc::para::Wrap::InFrontOfText, ..Default::default() },
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        };
        a.session.doc.insert_object(&Pos::body(0, 0), shape, &Default::default()).unwrap();
        let mut h: H = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        steps(&mut h, 6);
        let o = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).expect("laid out");
        let at = |h: &mut H, x: f32, y: f32| crate::canvas::page_to_screen(h.state_mut(), 0, x, y).unwrap();
        let down = 200.0;
        let start = at(&mut h, o.rect.x + o.rect.w / 2.0, o.rect.y + 4.0);
        let end = at(&mut h, o.rect.x + o.rect.w / 2.0, o.rect.y + 4.0 + down);
        h.event(egui::Event::PointerMoved(start));
        h.step();
        button(&h, start, true);
        h.step();
        for k in 1..=5 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 5.0)));
            h.step();
        }
        h.step(); // the preview is painted a frame after the move
        assert!(h.state().canvas.obj_drag.as_ref().is_some_and(|d| d.drag.moved), "a drag is in progress");
        let (tl, br) = (at(&mut h, o.rect.x, o.rect.y + down), at(&mut h, o.rect.right(), o.rect.bottom() + down));
        let img = h.render().unwrap();
        let ppp = h.ctx.pixels_per_point();
        // Pixels in a screen area, but for the mouse pointer drawn at `end`.
        let count = |x0: f32, y0: f32, x1: f32, y1: f32, f: &dyn Fn([u8; 4]) -> bool| {
            let (x0, y0, x1, y1) = ((x0 * ppp).ceil() as u32, (y0 * ppp).ceil() as u32, (x1 * ppp) as u32, (y1 * ppp) as u32);
            let pointer = Rect::from_min_size(end - egui::vec2(2.0, 2.0), egui::vec2(20.0, 26.0));
            (y0..y1)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|&(x, y)| !pointer.contains(egui::pos2(x as f32 / ppp, y as f32 / ppp)) && f(img.get_pixel(x, y).0))
                .count()
        };
        let red = count(tl.x - 4.0, tl.y - 4.0, br.x + 4.0, br.y + 4.0, &|[r, g, b, _]| r > 200 && g < 180 && b < 180);
        assert!(red > 50, "the preview shows the shape's outline ({red} red pixels)");
        let dark = count(tl.x + 8.0, tl.y + 8.0, br.x - 8.0, br.y - 8.0, &|[r, g, b, _]| r < 180 && g < 180 && b < 180);
        assert_eq!(dark, 0, "no text from behind the shape in its preview");
        button(&h, end, false);
        steps(&mut h, 3);
    }

    /// A 100×60 pt rectangle filled E2EFDA: [`fill_pixels`] counts the page's pixels of that colour.
    fn filled_shape() -> InlineObject {
        InlineObject::Shape {
            kind: ShapeKind::Rectangle,
            w: 100.0,
            h: 60.0,
            fill: Some(wordcraft_doc::fill::Fill::solid(wordcraft_doc::props::Rgb(0xE2, 0xEF, 0xDA))),
            stroke: None,
            stroke_width: 0.0,
            float: Default::default(),
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        }
    }

    /// How many pixels of the first page have the fill colour: how many copies of the shape the
    /// page shows.
    fn fill_pixels(app: &mut WordApp, opts: &wordcraft_render::RenderOptions) -> usize {
        let layout = app.session.layout();
        let img = wordcraft_render::render_page(&app.session.doc, &layout.pages[0], 1.0, opts);
        img.to_straight().as_chunks::<4>().0.iter().filter(|p| (p[0], p[1], p[2]) == (0xE2, 0xEF, 0xDA)).count()
    }

    /// A shape dragged on the page is left out of it: the preview under the pointer stands in for
    /// it, so the page must not draw it where it was as well (`DisplayOptions::hide_object`).
    ///
    /// Judged by pixels: the page raster has none of its fill colour once it is left out, and a
    /// thousand or so while it isn't. Without this, "it stays where it was while it is dragged"
    /// would come back with nothing to catch it.
    #[test]
    fn a_dragged_inline_object_is_left_out_of_its_page() {
        let (mut app, hit) = app_with(filled_shape());
        let pos = hit.pos();
        let on = wordcraft_render::RenderOptions::default();
        let before = fill_pixels(&mut app, &on);
        assert!(before > 1000, "the shape is on its page to begin with: {before} px");
        let mut hidden = on.clone();
        hidden.display.hide_object = Some(pos);
        assert_eq!(fill_pixels(&mut app, &hidden), 0, "a dragged object is left out of its page");
    }

    /// The same for an object that is already floating: it is drawn as a `Placed::Shape` of its
    /// own rather than inside a line, so leaving it out goes by the object the shape carries.
    #[test]
    fn a_dragged_floating_shape_is_left_out_of_its_page() {
        let (mut app, hit) = app_with(filled_shape());
        let pos = hit.pos();
        let end = Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", serde_json::json!({"anchor": pos, "focus": end}));
        app.run("arrange.bounds", serde_json::json!({"page": 0, "x": 100.0, "y": 150.0})).expect("floats it");
        let hit = app.session.layout().object(&pos, 0).expect("laid out");
        assert!(hit.floating(), "arrange.bounds makes it float");

        let on = wordcraft_render::RenderOptions::default();
        let before = fill_pixels(&mut app, &on);
        assert!(before > 1000, "the floating shape is on its page to begin with: {before} px");
        let mut hidden = on.clone();
        hidden.display.hide_object = Some(pos);
        assert_eq!(fill_pixels(&mut app, &hidden), 0, "a dragged floating shape is left out of its page");
    }

    /// A floating shape dragged onto the next page lands there — the page the preview shows —
    /// even dropped at the very top of it, where the anchor is the page's first paragraph. The
    /// shape's wrap holds that paragraph on the old page, so once the shape moves the paragraph
    /// can slide back and take the shape with it; the drop must re-anchor until it stays.
    #[test]
    fn a_shape_dragged_onto_the_next_page_is_dropped_there() {
        use egui::{Event, PointerButton};
        let mut a = WordApp::new(wordcraft_engine::Session::new(Document::new()), crate::Services::default());
        a.run("view.commentsPane", json!({"value": false})).unwrap();
        for i in 0..40 {
            if i > 0 {
                a.run("text.newParagraph", json!({})).unwrap();
            }
            a.run("text.insert", json!({"text": format!("Paragraph {i} has words enough to run over two or three lines of the page.")})).unwrap();
        }
        a.run("caret.docStart", json!({})).unwrap();
        a.run("insert.shape", json!({"kind": "rectangle"})).unwrap();
        a.run("arrange.bounds", json!({"page": 0, "x": 100, "y": 250, "width": 100, "height": 50})).unwrap();
        assert!(a.session.layout().pages.len() > 1, "a second page to drag onto");
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 1800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            a,
        );
        for _ in 0..6 {
            h.step();
        }
        let shape =
            |h: &mut egui_kittest::Harness<'_, WordApp>| h.state_mut().session.layout().find_object(0, |o| o.text_box.is_none()).expect("a shape");
        let r = shape(&mut h).rect;
        let from = crate::canvas::page_to_screen(h.state_mut(), 0, r.x + r.w / 2.0, r.y + r.h / 2.0).expect("the shape's page is on screen");
        let to = crate::canvas::page_to_screen(h.state_mut(), 1, 150.0, 120.0).expect("the next page is on screen");
        let button =
            |at: Pos2, pressed: bool| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        h.event(Event::PointerMoved(from));
        h.event(button(from, true));
        h.step();
        for at in [from + egui::vec2(0.0, 40.0), to] {
            h.event(Event::PointerMoved(at));
            h.step();
        }
        assert_eq!(h.state().canvas.obj_drag.as_ref().expect("dragging").drag.to_page, 1, "the preview is on the page under the pointer");
        h.event(button(to, false));
        for _ in 0..3 {
            h.step();
        }
        let o = shape(&mut h);
        assert_eq!(o.page, 1, "dropped on the page under the pointer, not thrown back");
        assert!((o.rect.x - 100.0).abs() < 0.5, "kept its place across: {:?}", o.rect);
    }

    /// A settled canvas for the rotation regressions, without taking focus on the desktop.
    fn rotation_window(app: WordApp) -> crate::table_resize::tests::H {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(0.25).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        crate::table_resize::tests::steps(&mut h, 6);
        h
    }

    /// #2277: the actual picture pixels follow the rotation handle before release, including
    /// pictures whose cached preview already contains a rotation and either reflection.
    #[test]
    fn rotation_drag_turns_picture_pixels_live_and_commits_once_or_cancels() {
        use crate::table_resize::tests::{H, button, steps};
        for (initial, flip_h, flip_v) in [(0.0, false, false), (30.0, true, false), (90.0, false, true), (315.0, true, true)] {
            // Four original colour quadrants make both accidental extra rotation and reflected
            // UVs observable. Sample near the long ends after a quarter turn, outside the old
            // picture, so the stationary page image cannot satisfy the pixel assertions.
            let colours = [[230, 20, 20, 255], [20, 200, 20, 255], [20, 20, 230, 255], [230, 200, 20, 255]];
            let image = image::RgbaImage::from_fn(80, 40, |x, y| {
                let quadrant = match (x < 40, y < 20) {
                    (true, true) => 0,
                    (false, true) => 1,
                    (false, false) => 2,
                    (true, false) => 3,
                };
                image::Rgba(colours[quadrant])
            });
            let mut bytes = Vec::new();
            image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
            let picture = InlineObject::Image {
                media: "rotation-test".into(),
                w: 200.0,
                h: 100.0,
                alt: String::new(),
                float: wordcraft_doc::para::Float {
                    wrap: Wrap::InFrontOfText,
                    x: 150.0,
                    y: 180.0,
                    rot: initial,
                    flip_h,
                    flip_v,
                    ..Default::default()
                },
                crop: [0.0; 4],
                ole: None,
            };
            let (mut app, _) = app_with(picture);
            app.session.doc.media.insert("rotation-test".into(), Arc::new(bytes));
            app.session.touch();
            app.run("view.commentsPane", json!({"value": false})).unwrap();
            app.run("select.range", json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, OBJ.len_utf8())})).unwrap();
            let mut h: H = rotation_window(app);
            let depth = h.state().session.undo_depth();
            let original = serde_json::to_value(&h.state().session.doc).unwrap();
            let o = h.state_mut().session.layout().object(&Pos::body(0, 0), 0).unwrap();
            let (cx, cy) = (o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
            let center = crate::canvas::page_to_screen(h.state_mut(), 0, cx, cy).unwrap();
            let top = crate::canvas::page_to_screen(h.state_mut(), 0, cx, o.rect.y).unwrap();
            let radius = center.distance(top) + 22.0;
            let handle = |degrees: f32| center + egui::emath::Rot2::from_angle(degrees.to_radians()) * egui::vec2(0.0, -radius);
            let target = wordcraft_geom::normalize_degrees(initial + 90.0);
            let start = handle(initial);
            let end = handle(target);
            h.event(egui::Event::PointerMoved(start));
            button(&h, start, true);
            h.step();
            h.event(egui::Event::PointerMoved(end));
            steps(&mut h, 2);
            let d = h.state().canvas.obj_drag.as_ref().expect("rotating");
            assert_eq!(d.drag.grab, Grab::Rotate);
            assert_eq!(d.drag.deg, target);
            let texture = d.preview.as_ref().expect("cached picture preview").id();
            assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original, "dragging must not edit the document");
            assert_eq!(h.state().session.undo_depth(), depth);
            let rendered = h.render().unwrap();
            if initial == 0.0
                && let Ok(path) = std::env::var("WORDCRAFT_ROTATION_SHOT")
            {
                rendered.save(path).unwrap();
            }
            let ppp = h.ctx.pixels_per_point();
            let spin = wordcraft_geom::Spin::new(target, flip_h, flip_v);
            for ((x, y), colour) in [(-75.0, -20.0), (75.0, -20.0), (75.0, 20.0), (-75.0, 20.0)].into_iter().zip(colours) {
                let (x, y) = spin.apply(cx, cy, cx + x, cy + y);
                let at = crate::canvas::page_to_screen(h.state_mut(), 0, x, y).unwrap();
                let pixel = rendered.get_pixel((at.x * ppp).round() as u32, (at.y * ppp).round() as u32).0;
                for (actual, expected) in pixel.into_iter().zip(colour).take(3) {
                    if expected > 150 {
                        assert!(actual > 170, "live quadrant missing: {initial}/{flip_h}/{flip_v} {colour:?} -> {pixel:?}");
                    } else {
                        assert!(actual < 150, "live quadrant missing: {initial}/{flip_h}/{flip_v} {colour:?} -> {pixel:?}");
                    }
                }
            }
            // A second angle reuses the raster rather than regenerating it on pointer moves.
            let next = handle(target + 15.0);
            h.event(egui::Event::PointerMoved(next));
            steps(&mut h, 2);
            assert_eq!(h.state().canvas.obj_drag.as_ref().unwrap().preview.as_ref().unwrap().id(), texture);
            let final_angle = h.state().canvas.obj_drag.as_ref().unwrap().drag.deg;
            button(&h, next, false);
            steps(&mut h, 3);
            assert!(h.state().canvas.obj_drag.is_none());
            assert_eq!(h.state().session.undo_depth(), depth + 1, "one command on release");
            assert_eq!(selected(h.state()).unwrap().1.frame().unwrap().2.rot, final_angle);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            steps(&mut h, 3);
            assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original, "one undo restores everything");

            h.event(egui::Event::PointerMoved(start));
            button(&h, start, true);
            h.step();
            h.event(egui::Event::PointerMoved(end));
            steps(&mut h, 2);
            assert!(h.state().canvas.obj_drag.as_ref().unwrap().preview.is_some());
            h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() });
            h.step();
            button(&h, end, false);
            steps(&mut h, 3);
            assert!(h.state().canvas.obj_drag.is_none());
            assert_eq!(h.state().session.undo_depth(), depth, "Escape adds no undo step");
            assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original, "Escape cancels without an edit");
        }
    }

    #[test]
    fn rotation_drag_keeps_text_boxes_outline_only_until_their_text_can_rotate() {
        use crate::table_resize::tests::{button, steps};
        let mut app = WordApp::new(wordcraft_engine::Session::new(Document::new()), Default::default());
        app.run("insert.textBox", json!({"text": "Text stays upright on release"})).unwrap();
        let bx = app.session.layout().find_object(0, |o| o.text_box.is_some()).unwrap();
        app.run("select.range", json!({"anchor": bx.pos(), "focus": Pos { off: bx.off + OBJ.len_utf8(), ..bx.pos() }})).unwrap();
        app.run("arrange.bounds", json!({"page": 0, "x": 150, "y": 180, "width": 200, "height": 100})).unwrap();
        let mut h = rotation_window(app);
        let original = serde_json::to_value(&h.state().session.doc).unwrap();
        let depth = h.state().session.undo_depth();
        let bx = h.state_mut().session.layout().find_object(0, |o| o.text_box.is_some()).unwrap();
        let (cx, cy) = (bx.rect.x + bx.rect.w / 2.0, bx.rect.y + bx.rect.h / 2.0);
        let center = crate::canvas::page_to_screen(h.state_mut(), 0, cx, cy).unwrap();
        let top = crate::canvas::page_to_screen(h.state_mut(), 0, cx, bx.rect.y).unwrap();
        let radius = center.distance(top) + 22.0;
        let (start, end) = (center - egui::vec2(0.0, radius), center + egui::vec2(radius, 0.0));
        h.event(egui::Event::PointerMoved(start));
        button(&h, start, true);
        h.step();
        h.event(egui::Event::PointerMoved(end));
        steps(&mut h, 2);
        let d = h.state().canvas.obj_drag.as_ref().expect("turning the text box frame");
        assert_eq!((d.drag.grab, d.drag.deg), (Grab::Rotate, 90.0));
        assert!(d.preview.is_none(), "do not preview rotated text that would snap upright on release");
        assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), original);
        assert_eq!(h.state().session.undo_depth(), depth);
        button(&h, end, false);
        steps(&mut h, 3);
        assert_eq!(selected(h.state()).unwrap().1.frame().unwrap().2.rot, 90.0);
        assert_eq!(h.state().session.undo_depth(), depth + 1);
    }
}
