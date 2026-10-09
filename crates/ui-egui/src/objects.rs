//! Pictures, shapes and text boxes on the canvas: click one to select it, drag it to move it,
//! drag a handle to resize it, arrow keys nudge a floating one. The frame itself is
//! [`crate::frame`]; changes go through `arrange.bounds` / `arrange.nudge`, so they're undoable
//! and scriptable like everything else.
//!
//! While dragging, nothing is laid out or rendered: the preview is the object's own pixels taken
//! from its page's cached texture. The drop is one command, one relayout.

use egui::{Color32, Painter, Pos2, Rect, Ui};
use serde_json::json;
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{PartKind, Pos, StoryRef};
use wordcraft_layout::DocLayout;
use wordcraft_layout::hit::ObjectHit;

use crate::WordApp;
use crate::frame::{self, Drag, Grab};
use crate::theme::Tokens;

/// The band along a text box's border that grabs the box rather than its text, screen points.
const EDGE: f32 = 5.0;
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
}

/// The selected object: the selection is exactly one picture, shape or text box in the body.
pub fn selected(app: &WordApp) -> Option<(Pos, &InlineObject)> {
    wordcraft_engine::cmd::objects::object_selection(&app.session).filter(|(p, _)| p.story == StoryRef::Body)
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
    if let Some(mut d) = app.canvas.obj_drag.take() {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            return true; // cancelled: dropped without a change
        }
        if ui.input(|i| i.pointer.primary_down()) {
            let shift = ui.input(|i| i.modifiers.shift);
            // Pictures keep their proportions from a corner unless Shift; shapes only with Shift.
            d.drag.update(at, pages, scale, d.min, d.picture != shift);
            app.canvas.obj_drag = Some(d);
        } else {
            drop(app, &d);
        }
        return true;
    }
    let pressed = ui.input(|i| i.pointer.primary_pressed()) && resp.contains_pointer();
    let multi = resp.double_clicked() || resp.triple_clicked();
    if !(pressed || multi) || crate::canvas::editing_header_footer(app, layout) {
        return false;
    }
    let Some((object, grab)) = grab_at(app, layout, pages, scale, at) else { return false };
    if pressed {
        let pos = object.pos();
        let end = Pos { off: pos.off + OBJ.len_utf8(), ..pos.clone() };
        let _ = app.run("select.range", json!({"anchor": pos, "focus": end}));
        let (min, picture) = match selected(app) {
            Some((_, o)) => (wordcraft_engine::cmd::objects::min_size(o), matches!(o, InlineObject::Image { .. })),
            None => return true,
        };
        // Objects in table cells can be selected and resized, not moved.
        if grab != Grab::Move || movable(&object) {
            app.canvas.obj_drag = Some(ObjectDrag { drag: Drag::new(grab, object.page, object.rect, at), object, min, picture });
        }
    }
    true
}

/// What a press at `at` would grab: a handle of the shown frame, else an object.
fn grab_at(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<(ObjectHit, Grab)> {
    let handle = active(app, layout).filter(|(o, _)| o.story == StoryRef::Body).and_then(|(o, _)| {
        let f = screen(pages, scale, o.page, o.rect)?;
        frame::handle_at(f, at).map(|h| (o, Grab::Resize(h)))
    });
    handle.or_else(|| {
        let (page, x, y) = crate::canvas::page_at(pages, scale, at)?;
        // Selectable: body objects (table cells too); notes' objects aren't yet.
        layout.object_at(page, x, y, EDGE / scale.max(0.01)).filter(|o| o.story == StoryRef::Body).map(|o| (o, Grab::Move))
    })
}

/// Objects that can be dragged to a new place: those in the body's own paragraphs.
fn movable(o: &ObjectHit) -> bool {
    o.story == StoryRef::Body && o.path.depth() == 0
}

/// Commit a finished drag as one command.
fn drop(app: &mut WordApp, d: &ObjectDrag) {
    if !d.drag.moved {
        return;
    }
    let r = d.drag.rect;
    let params = match d.drag.grab {
        Grab::Move => json!({"page": d.drag.to_page, "x": r.x, "y": r.y}),
        // An inline object stays in the text; a floating one keeps its far edges put.
        Grab::Resize(_) if d.object.floating() && movable(&d.object) => json!({"width": r.w, "height": r.h, "page": d.drag.page, "x": r.x, "y": r.y}),
        Grab::Resize(_) => json!({"width": r.w, "height": r.h}),
    };
    let _ = app.run("arrange.bounds", params);
}

/// Draw the frame of the active object, or the drag preview.
pub fn paint(app: &WordApp, painter: &Painter, t: &Tokens, layout: &DocLayout, pages: &[Rect], scale: f32) {
    if let Some(d) = app.canvas.obj_drag.as_ref().filter(|d| d.drag.moved) {
        let Some(to) = screen(pages, scale, d.drag.to_page, d.drag.rect) else { return };
        // The object's own pixels, from its page's cached texture: moved, or scaled for a picture.
        // Resizing reflows a text box (and redraws a shape), so that's just the new outline.
        let pixels = d.picture || d.drag.grab == Grab::Move;
        if !pixels {
            painter.rect_filled(to, 0.0, t.accent.gamma_multiply(0.08));
        } else if let (Some(tex), Some(pg)) = (app.canvas.page_texture(d.object.page), layout.pages.get(d.object.page)) {
            let r = d.object.rect;
            let (w, h) = (pg.w.max(1.0), pg.h.max(1.0));
            let uv = Rect::from_min_max(egui::pos2(r.x / w, r.y / h), egui::pos2(r.right() / w, r.bottom() / h));
            painter.image(tex.id(), to, uv, Color32::from_white_alpha(170));
        }
        frame::paint(painter, t, to, false, false);
        return;
    }
    if let Some((o, editing)) = active(app, layout)
        && let Some(f) = screen(pages, scale, o.page, o.rect)
    {
        // Handles where it can be resized (body objects).
        frame::paint(painter, t, f, editing, o.story == StoryRef::Body);
    }
}

/// The pointer over an object or a handle (or dragging): its cursor.
pub fn cursor(app: &WordApp, layout: &DocLayout, pages: &[Rect], scale: f32, at: Pos2) -> Option<egui::CursorIcon> {
    if let Some(d) = &app.canvas.obj_drag {
        return Some(d.drag.grab.cursor());
    }
    if crate::canvas::editing_header_footer(app, layout) {
        return None;
    }
    grab_at(app, layout, pages, scale, at).map(|(_, g)| g.cursor())
}

/// Keys for a selected object: arrows nudge a floating one; typing or Enter goes into a text box.
/// Returns true if handled.
pub fn key(app: &mut WordApp, key: egui::Key, m: egui::Modifiers) -> bool {
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
    let end = app.session.doc.end_of(StoryRef::Part(id));
    app.run("caret.set", json!({"pos": end})).is_ok()
}
