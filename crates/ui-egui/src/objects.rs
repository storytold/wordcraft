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
    /// A handle of a turned text box being edited (drawn straight, Word check P): the drag works
    /// on the straight frame, a turn adds to the box's real angle (`base`), and the text stays
    /// edited (`keep`, the selection to go back to).
    base: f32,
    keep: Option<wordcraft_engine::Selection>,
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

/// A turned text box whose text is being edited, so drawn straight.
fn straightened(o: &ObjectHit, editing: bool) -> bool {
    editing && crate::canvas::turned(o)
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
            if d.drag.grab == Grab::Rotate && d.keep.is_some() {
                // A straightened frame's turn adds to the box's angle: Shift snaps the sum.
                d.drag.turn_to(at, pages, scale, false);
                if shift {
                    let step = frame::SNAP_DEG;
                    d.drag.deg = wordcraft_geom::normalize_degrees(((d.base + d.drag.deg) / step).round() * step - d.base);
                }
            } else if d.drag.grab == Grab::Rotate {
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
            base: 0.0,
            keep: None,
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
    if pressed && adding && grab == Grab::Move && selected(app).is_some() && (ui.input(|i| i.modifiers.shift) || !again) {
        let _ = app.run("select.addObject", json!({"pos": object.pos()}));
        return true;
    }
    // A handle of a turned text box being edited: the text stays edited and the box straight.
    let straight_handle = grab != Grab::Move && active(app, layout).is_some_and(|(o, editing)| straightened(&o, editing) && o.pos() == object.pos());
    if pressed && straight_handle {
        let min = app
            .session
            .doc
            .para(object.story, &object.path)
            .and_then(|p| p.object_at(object.off))
            .map_or(0.0, wordcraft_engine::cmd::objects::min_size);
        let drag = Drag::new(grab, object.page, object.rect, 0.0, at);
        let (base, keep) = (object.spin.deg, Some(app.session.sel.clone()));
        app.canvas.obj_drag = Some(ObjectDrag {
            drag,
            object,
            min,
            picture: false,
            pending_behind: false,
            end: None,
            base,
            keep,
            guides: Vec::new(),
            preview: None,
            copy: false,
            drop_at: None,
        });
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
            base: 0.0,
            keep: None,
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
                base: 0.0,
                keep: None,
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
    // A turned text box being edited is drawn straight, without handles (its handles would
    // turn and size it from the straight frame); its border still moves it.
    // A turned text box being edited is drawn straight, its handles on the straight frame.
    let handle = active(app, layout).filter(|(o, _)| o.story == story && !fixed(app, o)).and_then(|(o, editing)| {
        let f = screen(pages, scale, o.page, o.rect)?;
        let deg = if straightened(&o, editing) { 0.0 } else { o.spin.deg };
        if !size_only(app, &o) && frame::rotate_handle_at(f, deg, at) {
            return Some((o, Grab::Rotate));
        }
        frame::handle_at(f, deg, at).map(|h| (o, Grab::Resize(h)))
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
        let straight = crate::canvas::straight_box(app, layout);
        layout.object_at_shown(page, x, y, EDGE / scale.max(0.01), story, straight).map(|o| (o, Grab::Move))
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
    let img = wordcraft_render::render_items(
        &app.session.doc,
        &items,
        wordcraft_geom::Rect::new(b.x, b.y, b.w, b.h),
        scale * ctx.pixels_per_point(),
        &opts,
    );
    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.to_straight());
    Some(ctx.load_texture("object-drag", ci, egui::TextureOptions::LINEAR))
}

/// Commit a finished drag as one command.
fn drop(app: &mut WordApp, d: &ObjectDrag) {
    if !d.drag.moved {
        return;
    }
    // A handle of an edited turned box: the commands act on the box, then the text is edited
    // again.
    if let Some(keep) = &d.keep {
        let pos = d.object.pos();
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
        commit(app, d);
        app.session.sel = keep.clone();
        return;
    }
    commit(app, d);
}

/// Run a finished drag's command.
fn commit(app: &mut WordApp, d: &ObjectDrag) {
    let mut r = d.drag.rect;
    // A chart or SmartArt takes a new size, never a new place.
    if matches!(d.drag.grab, Grab::Resize(_)) && size_only(app, &d.object) {
        let _ = app.run("picture.size", json!({"width": r.w, "height": r.h, "lockAspect": false}));
        return;
    }
    // A resize of an edited turned box ran on its straight frame: once it turns back about its
    // new centre, the edge held still would move by R·Δc − Δc. Move the frame back by that, so
    // the held edge stays at the same page point (as a turned frame's resize does).
    if d.keep.is_some() && matches!(d.drag.grab, Grab::Resize(_)) {
        let s = d.drag.start;
        let (dx, dy) = (r.x + r.w / 2.0 - (s.x + s.w / 2.0), r.y + r.h / 2.0 - (s.y + s.h / 2.0));
        let (tx, ty) = wordcraft_geom::Spin::new(d.base, false, false).apply(0.0, 0.0, dx, dy);
        if (tx - dx).is_finite() && (ty - dy).is_finite() {
            r.x += tx - dx;
            r.y += ty - dy;
        }
    }
    let params = match d.drag.grab {
        Grab::Rotate => {
            let deg = wordcraft_geom::normalize_degrees(d.base + d.drag.deg);
            let _ = app.run("arrange.rotation", json!({"degrees": deg}));
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
            frame::paint_turning(painter, t, to, d.drag.deg, d.base + d.drag.deg);
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
        // A turned text box being edited is drawn straight, and so is its frame (Word check P).
        let deg = if straightened(&o, editing) { 0.0 } else { o.spin.deg };
        frame::paint(painter, t, f, deg, editing, handles, handles && !size_only(app, &o));
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
pub(crate) mod tests;
