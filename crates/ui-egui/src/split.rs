//! View › Window › Split (#3254): the window shows its document in two panes, one above the
//! other, with a bar between them, so two parts of a long document can be seen and edited at
//! once, as in Word.
//!
//! Each pane scrolls on its own and has its own caret, zoom and view mode, as each of Word's
//! panes has its own selection and view: the pane being worked in draws from the session and
//! [`WordApp::canvas`], and the other keeps its page state and its [`WindowView`] in a
//! [`SplitPane`] held by that canvas state. The other pane is drawn by swapping them in for it,
//! with no caret and without the keyboard. A press in it makes it the pane worked in before the
//! press is handled, so the press lands in it as in a window with one pane. The task panes, the
//! ruler and the window's other settings stay the window's ([`ViewState::with_pane_settings`]).
//!
//! As in Word: Split divides the window in the middle and the new, lower pane is worked in;
//! Remove Split (the button again, or a double-click on the bar) closes the pane being worked in
//! and leaves the other one. Dragging the bar moves it; dragged to the window's top or bottom
//! edge, it closes the pane it squeezes out. F6 and Shift+F6 move between the panes ([`f6`]).
//! Splitting is not an edit: there is nothing to undo, and an edit made in either pane is undone
//! from either.
//!
//! [`ViewState::with_pane_settings`]: wordcraft_engine::ViewState::with_pane_settings

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use wordcraft_engine::WindowView;

use crate::WordApp;
use crate::canvas::{self, CanvasState};
use crate::theme::Tokens;

/// Height of the bar between the panes.
pub(crate) const BAR: f32 = 7.0;

/// The least height a pane is drawn with. The bar dragged closer than this to the window's top
/// or bottom edge closes the pane it squeezes out; a window too short for two panes shows one.
pub(crate) const MIN_PANE: f32 = 56.0;

/// The pane of a split window not being worked in, and how the window is divided.
pub(crate) struct SplitPane {
    /// Its page state: scroll position, rendered pages, rulers.
    canvas: CanvasState,
    /// Its selection and view; `None` only while it is drawn or worked in.
    view: Option<WindowView>,
    /// The upper pane's share of the room the panes have (0–1), kept as the window resizes.
    fraction: f32,
    /// The pane being worked in is the lower one.
    active_bottom: bool,
    /// Where the upper pane, the bar and the lower pane were drawn last; `None` while the window
    /// shows one pane (Read Mode, Focus, a window too short for two).
    rects: Option<[Rect; 3]>,
}

impl SplitPane {
    /// The pane not being worked in: its page state (screen geometry, scroll position).
    #[cfg(test)]
    pub(crate) fn other_canvas(&self) -> &CanvasState {
        &self.canvas
    }

    /// The pane not being worked in: its selection and view.
    #[cfg(test)]
    pub(crate) fn other_view(&self) -> Option<&WindowView> {
        self.view.as_ref()
    }

    /// The pane being worked in is the lower one.
    #[cfg(test)]
    pub(crate) fn active_bottom(&self) -> bool {
        self.active_bottom
    }

    /// Where the upper pane, the bar and the lower pane were drawn last.
    #[cfg(test)]
    pub(crate) fn rects(&self) -> Option<[Rect; 3]> {
        self.rects
    }
}

/// What the bar was asked to do.
#[derive(Clone, Copy, PartialEq, Debug)]
enum BarAction {
    /// Remove Split: close the pane being worked in (a double-click).
    Remove,
    /// Close the upper (`false`) or the lower (`true`) pane: the bar was dragged onto its edge.
    Close(bool),
}

/// Draw the document area: two panes and the bar between them while View › Split is on, else
/// the page alone. `allowed`: the window shows its normal chrome (Focus and Full Screen View
/// show one pane, and so does Read Mode, keeping the split for when they end).
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, allowed: bool) {
    if !app.session.view.split {
        remove(app);
    }
    let full = ui.available_rect_before_wrap();
    let fits = full.height() >= 2.0 * MIN_PANE + BAR && full.width() > 0.0;
    if !allowed || !app.session.view.split || app.session.view.read_mode || !fits {
        // The split is kept for when it can be drawn again, but its panes aren't on screen.
        if let Some(split) = app.canvas.split.as_mut() {
            split.rects = None;
        }
        canvas::show(app, ui);
        return;
    }
    let document = app.session.document_id();
    let mut split = match app.canvas.split.take() {
        Some(s) if s.view.as_ref().is_some_and(|v| v.document_id() == document) => s,
        _ => open(app),
    };
    let [top, bar, bottom] = divide(full, split.fraction);
    split.rects = Some([top, bar, bottom]);
    // A press in the other pane makes it the one worked in before anything is drawn, so the
    // press then places the caret, selects or drags there.
    let other = if split.active_bottom { top } else { bottom };
    if !app.modal_open() && pressed_in(ui, other) {
        switch(app, &mut split);
    }
    if let Some(action) = bar_input(ui, &mut split, bar, full) {
        finish(app, ui, split, action);
        return;
    }
    for (lower, rect) in [(false, top), (true, bottom)] {
        let mut pane = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("split_pane", lower)));
        pane.set_clip_rect(rect.intersect(ui.clip_rect()));
        if lower == split.active_bottom {
            canvas::show(app, &mut pane);
        } else {
            draw_other(app, &mut pane, &mut split);
        }
    }
    paint_bar(ui, bar);
    // A command run on the page may have shown another document (its state started afresh) or
    // turned the split off: the other pane goes with it.
    if app.session.document_id() == document && app.canvas.split.is_none() {
        app.canvas.split = Some(split);
    }
}

/// Split: the window's pane becomes the upper one, and a new lower pane on the same place, with
/// the same caret and view, is worked in, as in Word.
fn open(app: &mut WordApp) -> Box<SplitPane> {
    let fresh = app.canvas.split_off();
    // The window's pane is drawn inside the split now, whose scroll memory is its own.
    keep_place(app);
    let mut split = Box::new(SplitPane { canvas: fresh, view: Some(app.session.copy_view()), fraction: 0.5, active_bottom: false, rects: None });
    switch(app, &mut split);
    split
}

/// A window that shows one document at a time shows a document again whose window was split
/// (#3254): its other pane comes back from where it was put aside ([`crate::documents`]), and
/// the pane worked in takes the other pane number, so each keeps its own scroll position and
/// keyboard focus. Nothing when it had none.
pub(crate) fn resume(app: &mut WordApp, split: Option<Box<SplitPane>>) {
    let Some(split) = split else { return };
    // Each pane's scroll area keeps its place in egui's memory, by document and pane number.
    app.canvas.pane = if split.canvas.pane == 0 { 1 } else { 0 };
    app.canvas.split = Some(split);
}

/// Work in the other pane: its view and page state come in, and the pane worked in until now
/// keeps its own, without the keyboard.
fn switch(app: &mut WordApp, split: &mut SplitPane) {
    let Some(view) = split.view.take() else { return };
    let held = app.session.swap_pane_view(view);
    split.view = Some(held);
    std::mem::swap(&mut app.canvas, &mut split.canvas);
    let parked = &mut split.canvas;
    parked.focused = false;
    parked.want_focus = false;
    parked.balloon = None;
    parked.context_menu_open = false;
    parked.mini_anchor = None;
    app.canvas.want_focus = true;
    split.active_bottom = !split.active_bottom;
}

/// Remove Split: the pane being worked in closes and the other one fills the window, as Word's
/// Remove Split does. Nothing when the window has one pane.
fn remove(app: &mut WordApp) {
    let Some(mut split) = app.canvas.split.take() else { return };
    if split.view.as_ref().is_some_and(|v| v.document_id() == app.session.document_id()) {
        switch(app, &mut split);
    }
    keep_place(app);
}

/// The pane left after a split ends is drawn in the whole window, whose scroll memory isn't the
/// pane's: it asks for the place it showed.
fn keep_place(app: &mut WordApp) {
    app.canvas.scroll_request = Some(app.canvas.scroll_offset);
}

/// Act on the bar's double-click or drag to an edge, then draw the pane that is left.
fn finish(app: &mut WordApp, ui: &mut Ui, mut split: Box<SplitPane>, action: BarAction) {
    match action {
        BarAction::Remove => app.canvas.split = Some(split),
        BarAction::Close(lower) => {
            // The pane that stays is the one worked in, and no pane is left to remove.
            if lower == split.active_bottom {
                switch(app, &mut split);
            }
            keep_place(app);
        }
    }
    // Through the command, so the ribbon, the control channel and scripts see the split end.
    // Should it be refused, the next frame splits the window again; this one draws one pane
    // rather than handling the same press twice.
    let _ = app.run("view.split", json!({"value": false}));
    remove(app);
    canvas::show(app, ui);
}

/// F6 and Shift+F6 with the document's keyboard in a split window (#3254): F6 moves from the
/// upper pane to the lower one and Shift+F6 back, as Word's F6 goes through a split window's
/// panes before the task panes. Past the lower pane (or, backwards, the upper one) the keys go
/// on to the task panes; with none open they go round the two panes. True when the key was used
/// here.
pub(crate) fn f6(app: &mut WordApp, reverse: bool, task_panes: bool) -> bool {
    if !app.canvas.focused {
        return false;
    }
    // Only while both panes are on screen.
    let Some(mut split) = app.canvas.split.take_if(|s| s.rects.is_some()) else { return false };
    // Forward from the upper pane, or backwards from the lower one, stays in the document.
    let within = split.active_bottom == reverse;
    let current = split.view.as_ref().is_some_and(|v| v.document_id() == app.session.document_id());
    let used = current && (within || !task_panes);
    if used {
        switch(app, &mut split);
    }
    app.canvas.split = Some(split);
    used
}

/// The upper pane, the bar and the lower pane in `full`, the upper pane taking `fraction` of the
/// room, each pane at least [`MIN_PANE`] tall.
fn divide(full: Rect, fraction: f32) -> [Rect; 3] {
    let room = (full.height() - BAR).max(0.0);
    let upper = (room * fraction.clamp(0.0, 1.0)).round().clamp(MIN_PANE.min(room), (room - MIN_PANE).max(MIN_PANE.min(room)));
    let top = Rect::from_min_size(full.min, vec2(full.width(), upper));
    let bar = Rect::from_min_size(pos2(full.min.x, top.max.y), vec2(full.width(), BAR));
    let bottom = Rect::from_min_max(pos2(full.min.x, bar.max.y), full.max);
    [top, bar, bottom]
}

/// A press (any button) this frame inside `rect`, on the window's own layer: not on a menu or
/// dialog over it.
fn pressed_in(ui: &Ui, rect: Rect) -> bool {
    let presses: Vec<egui::Pos2> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerButton { pos, pressed: true, .. } if rect.contains(*pos) => Some(*pos),
                _ => None,
            })
            .collect()
    });
    presses.into_iter().any(|p| ui.ctx().layer_id_at(p).is_none_or(|l| l == ui.layer_id()))
}

/// The bar: dragged, it moves; dragged onto the window's top or bottom edge, it closes the pane
/// it squeezes out; double-clicked, it removes the split.
fn bar_input(ui: &Ui, split: &mut SplitPane, bar: Rect, full: Rect) -> Option<BarAction> {
    let resp = ui.interact(bar, ui.scope_id().with("split_bar"), Sense::click_and_drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if resp.double_clicked() {
        return Some(BarAction::Remove);
    }
    let y = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos())).map(|p| p.y)?;
    if resp.dragged() {
        let room = full.height() - BAR;
        if room > 0.0 {
            split.fraction = ((y - full.min.y - BAR / 2.0) / room).clamp(0.0, 1.0);
        }
    }
    if resp.drag_stopped() {
        if y < full.min.y + MIN_PANE {
            return Some(BarAction::Close(false));
        }
        if y > full.max.y - MIN_PANE {
            return Some(BarAction::Close(true));
        }
    }
    None
}

/// The bar between the panes: a band in the window's chrome colour with a line along each edge.
fn paint_bar(ui: &Ui, bar: Rect) {
    let t = Tokens::get(ui.ctx());
    let painter = ui.painter_at(bar);
    painter.rect_filled(bar, 0.0, t.panel);
    let line = Stroke::new(1.0, t.border_strong);
    for y in [bar.min.y + 0.5, bar.max.y - 0.5] {
        painter.hline(bar.x_range(), y, line);
    }
}

/// Draw the pane not being worked in: its view and page state are swapped in for it, and back
/// out after.
fn draw_other(app: &mut WordApp, ui: &mut Ui, split: &mut SplitPane) {
    let Some(view) = split.view.take() else { return };
    let document = app.session.document_id();
    if view.document_id() != document {
        split.view = Some(view);
        return;
    }
    std::mem::swap(&mut app.canvas, &mut split.canvas);
    let held = app.session.swap_pane_view(view);
    app.canvas.passive = true;
    canvas::show(app, ui);
    app.canvas.passive = false;
    split.view = Some(app.session.swap_pane_view(held));
    std::mem::swap(&mut app.canvas, &mut split.canvas);
}

#[cfg(test)]
mod tests;
