//! View › Window › Split (#3254): the window shows its document in two panes, one above the
//! other, so two parts of a long document can be seen and edited at once, as in Word.
//!
//! The panes, the divider between them and the choice of the pane worked in are craft-ui's
//! [`SplitViews`]: its inactive pane is a Tab stop over its view that takes a press and reports it
//! ([`SplitViewsOutput::activated`](craft::ui::canvas::SplitViewsOutput)), so the view under it
//! gets neither the press, the keyboard nor the input method. Each pane is a page view shown with
//! [`PageView::show_at`](craft::pages::ui::PageView::show_at) ([`canvas::show_pane`]).
//!
//! Each pane scrolls on its own and has its own caret, zoom and view mode, as each of Word's panes
//! has its own selection and view: the pane worked in draws from the session and
//! [`WordApp::canvas`], and the other keeps its page state and its [`WindowView`] in a
//! [`SplitPane`] held by that canvas state. The other pane is drawn by swapping them in for it.
//! The task panes, the ruler and the window's other settings stay the window's
//! ([`ViewState::with_pane_settings`]).
//!
//! As in Word: Split divides the window in the middle and the new, lower pane is worked in;
//! Remove Split closes the pane being worked in and leaves the other one. Splitting is not an
//! edit: there is nothing to undo, and an edit made in either pane is undone from either.
//!
//! [`ViewState::with_pane_settings`]: wordcraft_engine::ViewState::with_pane_settings

use craft::ui::canvas::{SplitAt, SplitLayout, SplitViews};
use craft::ui::contract::{Bind, ShowContainer as _};
use egui::{Rect, Ui};
use wordcraft_engine::WindowView;

use crate::WordApp;
use crate::canvas::{self, CanvasState, Pane};

/// The panes lie one above the other: pane 0 is the upper one.
const LAYOUT: SplitLayout = SplitLayout::TwoUpHorizontal;

/// The least height a window's document area needs for two panes; a shorter one shows one.
pub(crate) const MIN_PANES: f32 = 120.0;

/// The pane of a split window not being worked in, and how the window is divided.
pub(crate) struct SplitPane {
    /// Its page state: the page view's place, the pages it showed, its rulers.
    canvas: CanvasState,
    /// Its selection and view; `None` only while it is drawn or worked in.
    view: Option<WindowView>,
    /// Where the divider lies (the upper pane's share of the height), kept as the window resizes.
    at: SplitAt,
    /// The pane being worked in is the lower one.
    active_bottom: bool,
    /// Where the upper and the lower pane were drawn last; `None` while the window shows one pane
    /// (Read Mode, Focus, a window too short for two).
    rects: Option<[Rect; 2]>,
}

#[cfg(test)]
impl SplitPane {
    /// The pane not being worked in: its page state (scroll position, pages shown).
    pub(crate) fn other_canvas(&self) -> &CanvasState {
        &self.canvas
    }

    /// The pane not being worked in: its selection and view.
    pub(crate) fn other_view(&self) -> Option<&WindowView> {
        self.view.as_ref()
    }

    /// The pane being worked in is the lower one.
    pub(crate) fn active_bottom(&self) -> bool {
        self.active_bottom
    }

    /// Where the upper and the lower pane were drawn last.
    pub(crate) fn rects(&self) -> Option<[Rect; 2]> {
        self.rects
    }
}

/// Draw the document area: two panes while View › Split is on, else the page alone. `allowed`:
/// the window shows its normal chrome (Focus and Full Screen View show one pane, and so does Read
/// Mode, keeping the split for when they end).
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui, allowed: bool) {
    if !app.session.view.split {
        remove(app);
    }
    let full = ui.available_rect_before_wrap();
    let fits = full.height() >= MIN_PANES && full.width() > 0.0;
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
    let active = usize::from(split.active_bottom);
    let divider = split.at.clone();
    let mut at = split.at.clone();
    let mut rects = None;
    let out = SplitViews::new(tl!("Document"), LAYOUT, Bind::bound(&mut at)).active(active).show(ui, |ui, body| {
        let placed = divider.placed(LAYOUT, 2, body.rect, body.env.direction().is_rtl());
        let [Some(&upper), Some(&lower)] = [placed.first(), placed.get(1)] else { return };
        for (index, rect) in [(0, upper), (1, lower)] {
            // Each pane's page view has ids of its own, kept by its place in the window.
            ui.scope_builder(egui::UiBuilder::new().id_salt(("split_pane", index)).max_rect(rect), |ui| {
                ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                if index == active {
                    canvas::show_pane(app, ui, Pane::Active(rect));
                } else {
                    draw_other(app, ui, &mut split, rect);
                }
            });
        }
        rects = Some([upper, lower]);
    });
    split.at = at;
    split.rects = rects;
    // A press in the other pane makes it the one worked in; the press itself goes to no page, as craft-ui's split
    // views have it.
    if out.output.activated.is_some_and(|pane| pane != active) && app.session.document_id() == document {
        switch(app, &mut split);
        // The press focused the pane's own stop at the pass's end: the page takes the keyboard after it.
        app.canvas.focus_page_after_pass();
    }
    // A command run on the page may have shown another document (its state started afresh): the other pane goes
    // with it.
    if app.session.document_id() == document && app.canvas.split.is_none() {
        app.canvas.split = Some(split);
    }
}

/// Split: the window's pane becomes the upper one, and a new lower pane on the same place, with
/// the same caret and view, is worked in, as in Word.
fn open(app: &mut WordApp) -> Box<SplitPane> {
    let fresh = app.canvas.split_off();
    let mut split =
        Box::new(SplitPane { canvas: fresh, view: Some(app.session.copy_view()), at: SplitAt::default(), active_bottom: false, rects: None });
    switch(app, &mut split);
    split
}

/// A window that shows one document at a time shows a document again whose window was split
/// (#3254): its other pane comes back from where it was put aside ([`crate::documents`]). Nothing
/// when it had none.
pub(crate) fn resume(app: &mut WordApp, split: Option<Box<SplitPane>>) {
    if split.is_some() {
        app.canvas.split = split;
    }
}

/// Work in the other pane: its view and page state come in, and the pane worked in until now
/// keeps its own, without the keyboard.
fn switch(app: &mut WordApp, split: &mut SplitPane) {
    let Some(view) = split.view.take() else { return };
    let held = app.session.swap_pane_view(view);
    split.view = Some(held);
    std::mem::swap(&mut app.canvas, &mut split.canvas);
    app.canvas.take_window_state(&mut split.canvas);
    split.active_bottom = !split.active_bottom;
}

/// Remove Split: the pane being worked in closes and the other one fills the window, as Word's
/// Remove Split does. Nothing when the window has one pane.
fn remove(app: &mut WordApp) {
    let Some(mut split) = app.canvas.split.take() else { return };
    if split.view.as_ref().is_some_and(|v| v.document_id() == app.session.document_id()) {
        switch(app, &mut split);
    }
}

/// Draw the pane not being worked in at `rect`: its view and page state are swapped in for it,
/// and back out after.
fn draw_other(app: &mut WordApp, ui: &mut Ui, split: &mut SplitPane, rect: Rect) {
    let Some(view) = split.view.take() else { return };
    if view.document_id() != app.session.document_id() {
        split.view = Some(view);
        return;
    }
    std::mem::swap(&mut app.canvas, &mut split.canvas);
    let held = app.session.swap_pane_view(view);
    canvas::show_pane(app, ui, Pane::Other(rect));
    split.view = Some(app.session.swap_pane_view(held));
    std::mem::swap(&mut app.canvas, &mut split.canvas);
}

#[cfg(test)]
mod tests;
