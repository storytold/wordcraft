//! One native window per open document. The app's windows are a [`craft_window::WindowList`]: one
//! entry is the root OS window (drawn by the app's own shell), every other entry is shown as a
//! [`DocumentWindow`] in an OS window of its own. Each draws the whole interface (title bar,
//! ribbon, page, status bar, dialogs) for its own document. [`WordApp`]'s fields hold the drawn
//! window's state ([`crate::window_ui`]); the other windows keep theirs in their [`Window`].
//! Between frames the fields hold the active window's (the one focused last), so the control
//! channel, the MCP bridge and the host's menus act on it.
//!
//! A frame: [`WordApp::logic`] does the app-wide work once (the control channel, recovery,
//! timers, printing). [`WordApp::ui`] then draws the windows in turn: for each it swaps the
//! window's state in, makes its document the session's active one, does the window's own work
//! (its file dialog, AutoSave, dropped files, Read Aloud, requests queued for it) and draws it.
//! Last, the active window's state comes back.
//!
//! Which window shows what follows the session ([`WordApp::sync_document`]): a document made
//! active that no window shows (New, Open) gets a new window, focused and cascaded from the last
//! one; a document a window shows brings that window to the front; a window whose document closed
//! closes, and the window focused before it comes forward. Opening a file replaces a blank
//! document nobody used in its own window, and the last window stays open with no document (the
//! empty window the Start screen also leaves). The web app has one window and switches documents
//! in it ([`crate::documents`]).
//!
//! View › New Window opens another window on a document. Each window on it has its own view of
//! it (selection, zoom, view mode, panes: [`WindowView`]); the session holds the drawn window's,
//! and the others keep theirs in their [`Window`]. What the app keeps per document
//! (AutoSave's file, comment drafts) stays in one place, shared by its windows. Closing one of a
//! document's windows leaves the document open and asks nothing; closing its last window closes
//! it.

use craft::ui::contract::{BodyCx, Props as _, ShowContainer as _, SizeLimits};
// PLAN: assumed `craft::ui::runtime::env::WindowKey` is the facade path of ui-contract's `runtime/env/shell.rs` `WindowKey`.
use craft::ui::runtime::env::WindowKey;
use craft::ui::runtime::windows::{WindowAction, WindowHost};
use craft::ui::shell::DocumentWindow;
use craft::ui::style::StyleCx;
use craft::window::{Document, WindowId, WindowList, WindowMenu};
use egui::ViewportId;
use serde_json::{Value, json};
use wordcraft_engine::WindowView;
use wordcraft_engine::cmd::view::OtherWindow;

use crate::{WordApp, documents, window_geometry::MIN_SIZE, window_ui::WindowUi};

pub(crate) mod arrange;

/// The window title for a document called `name`.
pub(crate) fn title(name: &str, dirty: bool) -> String {
    format!("{name}{} - WordCraft", if dirty { " •" } else { "" })
}

/// The id of the container that shows window `id`; the window host names its viewport by it.
fn window_container_id(id: WindowId) -> egui::Id {
    egui::Id::unique(("wordcraft.window", id.get()))
}

/// An open window.
pub(crate) struct Window {
    /// The document it shows.
    doc: u64,
    /// Its state while another window's is in [`WordApp`]'s fields; `None` for that window.
    ui: Option<Box<WindowUi>>,
    /// Requests commands made for its document while another window was drawn
    /// ([`WordApp::handle_request`]), handled when it draws next.
    requests: Vec<Value>,
    /// Its view of the document (caret, zoom, view mode…) while another window on the same
    /// document is drawn. `None` while the session holds it: for the drawn window, and for the
    /// window on each other document whose view is parked with that document.
    view: Option<WindowView>,
    /// Brings its OS window forward in the next pass that shows it; sent once.
    raise: bool,
    /// Its OS window opens cascaded from the window before it; used once, when it first shows.
    place: bool,
}

impl Window {
    /// A window on `doc`; `place` cascades its OS window from the others when it first shows.
    fn new(doc: u64, ui: Option<Box<WindowUi>>, view: Option<WindowView>, place: bool) -> Window {
        Window { doc, ui, requests: Vec::new(), view, raise: false, place }
    }
}

/// The app's windows. See the module documentation.
pub struct Windows {
    /// Separate native windows (desktop). Off, one window shows every document in turn (web).
    native: bool,
    /// Every window, in opening and focus order.
    list: WindowList<Window>,
    /// The entry the root OS window shows.
    root: WindowId,
    /// The window whose state is in [`WordApp`]'s fields.
    shown: WindowId,
    /// View › Window: Side by Side and Synchronous Scrolling (#322).
    arranged: arrange::State,
    /// Whether the root OS window had focus in the last pass; a [`DocumentWindow`] reports its own gain
    /// (`focus_gained`), the root's is read from the window host's facts.
    root_focused: bool,
}

impl Windows {
    /// One window, showing document `doc`.
    pub(crate) fn new(doc: u64) -> Windows {
        let mut list = WindowList::new();
        let id = list.open(Window::new(doc, None, None, false));
        Windows { native: cfg!(not(target_arch = "wasm32")), list, root: id, shown: id, arranged: arrange::State::default(), root_focused: false }
    }

    /// Whether documents get native windows of their own.
    pub(crate) fn native(&self) -> bool {
        self.native
    }

    /// One window for every document, as in the web app.
    #[cfg(test)]
    pub(crate) fn single(&mut self) {
        self.native = false;
    }

    /// The key of the window whose state is in [`WordApp`]'s fields.
    pub(crate) fn shown(&self) -> u64 {
        self.shown.get()
    }

    /// The key of the entry the root OS window shows.
    pub(crate) fn root_key(&self) -> u64 {
        self.root.get()
    }

    /// The key of the window that has focus (the one focused last), if any.
    pub(crate) fn active(&self) -> Option<u64> {
        self.list.active().map(WindowId::get)
    }

    /// The key of the window drawn now: `main` for the root entry, else one of its own, so two
    /// windows never share a placement or shell key.
    pub(crate) fn window_key(&self) -> WindowKey {
        if self.shown == self.root { WindowKey::main() } else { WindowKey::new(format!("window-{}", self.shown.get())) }
    }

    /// The viewports whose OS window is open: the root's and every other window's.
    pub(crate) fn viewports(&self, ctx: &egui::Context) -> Vec<ViewportId> {
        if !self.native {
            return vec![ViewportId::ROOT];
        }
        self.list.ids().filter_map(|id| self.viewport_of(ctx, id)).collect()
    }

    /// The list entry with the app's key `key` (the number control and menu entries carry).
    fn id_of(&self, key: u64) -> Option<WindowId> {
        self.list.ids().find(|id| id.get() == key)
    }

    fn get(&self, key: u64) -> Option<&Window> {
        self.list.get(self.id_of(key)?)
    }

    fn get_mut(&mut self, key: u64) -> Option<&mut Window> {
        let id = self.id_of(key)?;
        self.list.get_mut(id)
    }

    /// The window showing document `doc`, the most recently focused if several do.
    fn of_doc(&self, doc: u64) -> Option<u64> {
        self.list.recent().find(|&id| self.list.get(id).is_some_and(|w| w.doc == doc)).map(WindowId::get)
    }

    /// The document of the window focused last before `key`'s, if any.
    fn next_doc(&self, key: u64) -> Option<u64> {
        if !self.native {
            return None;
        }
        self.list.recent().find(|id| id.get() != key).and_then(|id| self.list.get(id)).map(|w| w.doc)
    }

    /// The viewport window `id` draws in, while its OS window is open: the root's for the root
    /// entry, else the one the window host names for the window's container.
    fn viewport_of(&self, ctx: &egui::Context, id: WindowId) -> Option<ViewportId> {
        if id == self.root {
            return Some(ViewportId::ROOT);
        }
        WindowHost::of(ctx).viewport_of(window_container_id(id))
    }

    /// [`Windows::viewport_of`] for the window with the app's key `key`.
    fn viewport(&self, ctx: &egui::Context, key: u64) -> Option<ViewportId> {
        self.viewport_of(ctx, self.id_of(key)?)
    }
}

impl WordApp {
    /// How many windows are open.
    pub fn window_count(&self) -> usize {
        if self.windows.native { self.windows.list.len() } else { 1 }
    }

    /// The egui viewport of the active window, the one focused last: control input,
    /// `ui.screenshot` and `ui.resize` are its. The root viewport when the windows don't have
    /// viewports of their own (the web app, embedded viewports) or the window has none yet.
    pub fn active_viewport(&self) -> ViewportId {
        let Some(ctx) = self.ctx.as_ref().filter(|_| self.windows.native) else { return ViewportId::ROOT };
        let id = self.windows.list.active().unwrap_or(self.windows.shown);
        self.windows.viewport_of(ctx, id).unwrap_or(ViewportId::ROOT)
    }

    /// Whether a close request for the root viewport closes only the window drawn there: true
    /// while other windows are open. The root adopts another entry then, and the host keeps the
    /// app running.
    pub fn closes_a_window(&self) -> bool {
        self.window_count() > 1 && !self.quit_requested
    }

    /// Quit the app (the app menu's Quit): ask about every document with unsaved work, each in
    /// its window, as [`WordApp::close_requested`] does. True when the app may quit now; the
    /// host closes it once `quit_requested` is set.
    pub fn quit(&mut self) -> bool {
        let go = self.close_requested();
        if go {
            self.quit_requested = true;
        }
        go
    }

    /// Put the drawn window's state away and draw window `key`, with its document active. False
    /// when there is no such window, or its document has closed ([`WordApp::prune_windows`]
    /// closes it then); the drawn window stays.
    pub(crate) fn enter_window(&mut self, key: u64) -> bool {
        let Some(id) = self.windows.id_of(key) else { return false };
        if id == self.windows.shown {
            return true;
        }
        let session = &self.session;
        let Some(w) = self.windows.list.get_mut(id).filter(|w| w.ui.as_ref().is_some_and(|ui| session.is_open(ui.shown_document))) else {
            return false;
        };
        let Some(mut state) = w.ui.take() else { return false };
        let view = w.view.take();
        let from = self.shown_document;
        self.swap_window(&mut state);
        let left = std::mem::replace(&mut self.windows.shown, id);
        if let Some(w) = self.windows.list.get_mut(left) {
            w.doc = state.shown_document;
            w.ui = Some(state);
        }
        let to = self.shown_document;
        self.carry_document_state(from, to);
        if let Some(view) = view {
            self.swap_view_in(key, to, view);
        }
        true
    }

    /// Window `key`, on document `doc` (now the session's active one), brings its own view of the
    /// document: the view the session holds goes to the window on `doc` it belongs to (the one
    /// without a view of its own), and `view` takes its place, made valid for edits made since.
    fn swap_view_in(&mut self, key: u64, doc: u64, view: WindowView) {
        let held = self.session.take_view();
        if let Some((_, owner)) = self.windows.list.iter_mut().find(|(id, w)| id.get() != key && w.doc == doc && w.view.is_none()) {
            owner.view = Some(held);
        }
        self.session.put_view(view);
    }

    /// Open a new window for open document `doc`, focused, and draw it. The window that was drawn
    /// leaves the File page: whatever it was opening is open. `view` is the new window's view of
    /// a document another window shows (View › New Window); `None` for a document no window
    /// shows yet. Its OS window shows from the next pass.
    fn open_window(&mut self, doc: u64, view: Option<WindowView>) {
        self.ui.backstage = false;
        let mut state = Box::new(WindowUi::new(doc));
        // A hidden ribbon stays hidden in the new window, as in Word; the tab still opens on Home.
        state.ribbon_collapsed = self.ui.ribbon_collapsed;
        let id = self.windows.list.open(Window::new(doc, Some(state), view, true));
        self.enter_window(id.get());
        self.focus_window(id.get());
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }

    /// View › New Window: another window on the drawn window's document, with the same caret,
    /// zoom and view, focused and cascaded from the others. Each window then has its own caret
    /// and view; the document, its undo history and its save state are shared. In one window
    /// (the web app) and with no document open, nothing happens.
    pub(crate) fn new_window(&mut self) {
        if !self.windows.native || !self.has_document() {
            return;
        }
        let view = self.session.copy_view();
        self.open_window(self.shown_document, Some(view));
    }

    /// The window that comes forward when the drawn window closes and other windows show its
    /// document, which then stays open: the one of all the others focused last. `None` when the
    /// drawn window is its document's only one.
    pub(crate) fn other_window_on_document(&self) -> Option<u64> {
        let shown = self.windows.shown;
        let doc = self.windows.list.get(shown)?.doc;
        if !self.windows.native || !self.windows.list.iter().any(|(id, w)| id != shown && w.doc == doc) {
            return None;
        }
        self.windows.list.recent().find(|&id| id != shown).map(WindowId::get)
    }

    /// Whether command `id` (File › Close) closes the drawn window alone, asking nothing: other
    /// windows show its document. Quitting closes documents, not windows.
    pub(crate) fn closes_only_its_window(&self, id: &str, params: &Value) -> bool {
        id == "file.close" && params.get("quit").and_then(Value::as_bool) != Some(true) && self.other_window_on_document().is_some()
    }

    /// Close the drawn window alone: other windows show its document, which stays open as it is.
    pub(crate) fn close_window_only(&mut self) -> bool {
        let Some(next) = self.other_window_on_document() else { return false };
        self.close_shown_window(next);
        true
    }

    /// The drawn window's document has closed: draw window `to` and close this one.
    fn close_shown_window(&mut self, to: u64) {
        let closing = self.windows.shown;
        if !self.enter_window(to) {
            return;
        }
        self.windows.list.close(closing);
        self.focus_window(to);
    }

    /// Make window `key` the active one and bring it to the front in the next pass that shows it.
    pub(crate) fn focus_window(&mut self, key: u64) {
        if let Some(id) = self.windows.id_of(key) {
            self.windows.list.focus(id);
            if let Some(w) = self.windows.list.get_mut(id) {
                w.raise = true;
            }
        }
        self.settle_windows();
    }

    /// The root OS window shows an entry that is open: when its own closed, the active one moves
    /// in. Each window's page state is its own, so the page keeps its place.
    fn settle_windows(&mut self) {
        if !self.windows.list.contains(self.windows.root)
            && let Some(next) = self.windows.list.active()
        {
            self.windows.root = next;
        }
    }

    /// Keep a window for every open document: windows whose document closed (an agent closed it)
    /// close, and documents with no window (an agent made one while another was active) get one
    /// behind the others. The active document is left to [`WordApp::sync_document`].
    pub(crate) fn prune_windows(&mut self) {
        let shown = self.windows.shown;
        let session = &self.session;
        let gone: Vec<WindowId> = self.windows.list.iter().filter(|(id, w)| *id != shown && !session.is_open(w.doc)).map(|(id, _)| id).collect();
        let mut changed = !gone.is_empty();
        for id in gone {
            self.windows.list.close(id);
        }
        let behind = self.windows.list.active();
        let active = self.session.document_id();
        for d in self.session.documents() {
            if d.id == active || Some(d.id) == self.no_document || d.id == self.shown_document || self.windows.of_doc(d.id).is_some() {
                continue;
            }
            self.windows.list.open(Window::new(d.id, Some(Box::new(WindowUi::new(d.id))), None, true));
            changed = true;
        }
        if let Some(behind) = behind {
            self.windows.list.focus(behind);
        }
        if changed {
            self.settle_windows();
            if let Some(ctx) = &self.ctx {
                ctx.request_repaint();
            }
        }
    }

    /// [`WordApp::sync_document`] with native windows: the session's active document `id` isn't
    /// the drawn window's. Another open document shows in its window, which comes to the front,
    /// or in a new window; when the drawn window's document has closed, its window closes and the
    /// one showing `id` comes forward. The Start screen's empty window stays beside the window
    /// that shows `id`. False when the window itself is to show `id` instead: its document was
    /// replaced, or it is the Start screen's empty window and no other window shows `id`.
    pub(crate) fn follow_in_windows(&mut self, id: u64) -> bool {
        let left = self.shown_document;
        let stand_in = self.no_document == Some(left);
        let other = self.windows.of_doc(id).filter(|&k| k != self.windows.shown());
        if self.session.is_open(left) {
            match other {
                Some(key) if self.enter_window(key) => self.focus_window(key),
                _ if stand_in => return false,
                _ => self.open_window(id, None),
            }
            return true;
        }
        if let Some(key) = other {
            self.close_shown_window(key);
            return true;
        }
        false
    }

    /// The drawn window now shows document `id` (its own was replaced).
    pub(crate) fn window_shows(&mut self, id: u64) {
        let shown = self.windows.shown;
        if let Some(w) = self.windows.list.get_mut(shown) {
            w.doc = id;
        }
    }

    /// Close the active document. With windows, its window closes and the window focused before
    /// it comes forward; else the window shows the next document. The session leaves a new blank
    /// document when it was the last.
    pub(crate) fn close_shown_document(&mut self) -> Result<(), String> {
        let doc = self.session.document_id();
        let next = self.windows.next_doc(self.windows.shown());
        self.session.close_document(doc).map_err(|e| e.to_string())?;
        if let Some(next) = next
            && self.session.is_open(next)
            && let Err(e) = self.session.switch_document(next)
        {
            log::warn!("closing document {doc}: couldn't show document {next}: {e}");
        }
        self.sync_document();
        Ok(())
    }

    /// A request made for open document `document`, which another window shows: that window
    /// handles it when it draws next (a dialog opens there). The active window and document stay
    /// as they are, so the control channel and agents go on working where they were. False when
    /// no other window shows the document (or windows aren't native).
    pub(crate) fn queue_for_window(&mut self, document: u64, req: &Value) -> bool {
        if !self.windows.native {
            return false;
        }
        let Some(key) = self.windows.of_doc(document).filter(|&k| k != self.windows.shown()) else { return false };
        if let Some(w) = self.windows.get_mut(key) {
            w.requests.push(req.clone());
        }
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
        true
    }

    /// View › Window › Switch Windows' entries and the window titles, built from the windows as
    /// they are now.
    fn window_menu(&self) -> WindowMenu {
        let docs = self.session.documents();
        WindowMenu::new(&self.windows.list, |_, w| match docs.iter().find(|d| d.id == w.doc) {
            Some(d) => Document { key: w.doc, name: documents::display_title(d), dirty: d.dirty },
            None => Document { key: w.doc, name: String::new(), dirty: false },
        })
    }

    /// View › Window › Switch Windows with native windows: every window by its title (two
    /// windows on one document as `Name:1`, `Name:2`) with its key, the active one marked,
    /// unsaved ones with the title bar's dot. Choosing one brings it to the front
    /// ([`WordApp::show_window`]).
    pub(crate) fn switch_windows_entries(&self) -> Vec<(u64, String, bool)> {
        let shown = self.windows.shown;
        self.window_menu().entries().iter().map(|e| (e.id.get(), format!("{}{}", e.title, if e.dirty { " •" } else { "" }), e.id == shown)).collect()
    }

    /// Bring window `key` to the front and make it the active one, with its document.
    pub(crate) fn show_window(&mut self, key: u64) {
        if self.enter_window(key) {
            self.focus_window(key);
        }
    }

    /// The drawn window's title: its document's name as View › Window › Switch Windows gives it.
    pub(crate) fn window_title(&self) -> String {
        if !self.has_document() {
            return "WordCraft".to_string();
        }
        let name = self.window_menu().title(self.windows.shown).map(str::to_string).unwrap_or_else(|| self.title_stem());
        title(&name, self.session.dirty)
    }

    /// The title of window `id` as `menu` gives it; `WordCraft` for a window with no document.
    fn title_in(menu: &WindowMenu, id: WindowId) -> String {
        match menu.get(id).filter(|e| !e.title.is_empty()) {
            Some(e) => title(&e.title, e.dirty),
            None => "WordCraft".to_string(),
        }
    }

    /// Where a window opening now cascades from: the outer rect of the window focused before it
    /// that has reported one, moved one step. `None` where no window reports its place (the web,
    /// an embedded shell); the overlay places the window then.
    fn cascade_from(&self, ui: &egui::Ui, new: WindowId, step: f32) -> Option<egui::Rect> {
        let host = WindowHost::of(ui.ctx());
        let previous =
            self.windows.list.recent().filter(|&id| id != new).find_map(|id| host.facts(self.windows.viewport_of(ui.ctx(), id)?)?.outer)?;
        Some(DocumentWindow::cascaded(ui, previous, previous.size(), step))
    }

    /// Draw every window (see the module documentation): the root entry in the root viewport,
    /// every other one in its [`DocumentWindow`].
    pub(crate) fn windows_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.prune_windows();
        self.settle_windows();
        // Where OS windows can't open (egui embeds them in the root: tests, `ui_shot`; the web) the
        // active window is shown alone: every window would draw the same widgets in one viewport.
        if !WindowHost::of(&ctx).opens_windows() {
            let active = self.windows.list.active().unwrap_or(self.windows.shown);
            self.draw_entry(ui, active.get());
            self.after_windows();
            return;
        }
        let raise_root = self.windows.list.get_mut(self.windows.root).is_some_and(|w| std::mem::take(&mut w.raise));
        if raise_root {
            WindowHost::of(&ctx).act(ViewportId::ROOT, WindowAction::Focus);
        }
        self.draw_entry(ui, self.windows.root.get());
        let root = self.windows.root;
        // The root window gaining OS focus makes its entry the active one, as a child's `focus_gained` does.
        let root_focused = WindowHost::of(&ctx).facts(ViewportId::ROOT).is_some_and(|facts| facts.focused);
        if root_focused && !std::mem::replace(&mut self.windows.root_focused, root_focused) {
            self.windows.list.focus(root);
        }
        self.windows.root_focused = root_focused;
        let menu = self.window_menu();
        let step = StyleCx::with_root(&ctx, |cx| cx.tokens.size.window.cascade);
        let ids: Vec<WindowId> = self.windows.list.ids().filter(|&id| id != root).collect();
        let mut closes = Vec::new();
        for id in ids {
            let Some(w) = self.windows.list.get_mut(id) else { continue };
            let (raise, place) = (std::mem::take(&mut w.raise), std::mem::take(&mut w.place));
            let mut window = DocumentWindow::new(Self::title_in(&menu, id))
                .id(window_container_id(id))
                .overlay(|overlay| overlay.size_limits(SizeLimits::at_least(MIN_SIZE)));
            if raise {
                window = window.raise();
            }
            if place && let Some(rect) = self.cascade_from(ui, id, step) {
                window = window.first_placed(rect);
            }
            let out = window.show(ui, |ui, cx| self.draw_window(ui, cx, id)).output;
            if out.focus_gained {
                self.windows.list.focus(id);
            }
            if out.close_requested.is_some() {
                closes.push(id);
            }
        }
        self.window_events(&closes);
        self.after_windows();
    }

    /// After the frame's drawing: windows opened or closed meanwhile settle, and the active
    /// window's state comes back.
    fn after_windows(&mut self) {
        self.prune_windows();
        self.settle_windows();
        if let Some(active) = self.windows.list.active().map(WindowId::get) {
            self.enter_window(active);
            self.session.windows.current = Some(active);
            if self.session.windows.available {
                self.session.windows.others = self
                    .switch_windows_entries()
                    .into_iter()
                    .filter(|(k, ..)| *k != active)
                    .map(|(id, title, _)| OtherWindow { id, title })
                    .collect();
            }
        }
    }

    /// Draw window `id` into its [`DocumentWindow`]'s body.
    fn draw_window(&mut self, ui: &mut egui::Ui, _cx: &mut BodyCx<'_>, id: WindowId) {
        self.draw_entry(ui, id.get());
    }

    /// Draw window `key`: its state comes in, its queued requests run, then its own work, input
    /// and interface.
    fn draw_entry(&mut self, ui: &mut egui::Ui, key: u64) {
        if !self.enter_window(key) {
            return;
        }
        let ctx = ui.ctx().clone();
        let queued = self.windows.get_mut(key).map(|w| std::mem::take(&mut w.requests)).unwrap_or_default();
        for req in queued {
            self.handle_request(&req);
        }
        self.window_work(&ctx);
        if self.windows.shown() != key {
            return;
        }
        self.window_ui(ui);
        arrange::forward_scroll(self, &ctx);
    }

    /// The close requests of the windows' close buttons from this frame: File › Close in that
    /// window, which asks about unsaved changes first. The last window's close quits, which the
    /// host asks about ([`WordApp::close_requested`]).
    fn window_events(&mut self, closes: &[WindowId]) {
        for &id in closes {
            if self.quit_requested || self.windows.list.len() <= 1 || !self.windows.list.contains(id) {
                continue;
            }
            self.close_window(id.get());
        }
    }

    /// The close button of window `key` (not the last one): File › Close in that window, which
    /// asks about unsaved changes first.
    pub(crate) fn close_window(&mut self, key: u64) {
        if !self.enter_window(key) {
            return;
        }
        self.focus_window(key);
        if !self.has_document() {
            // An empty window beside others: nothing to ask about.
            let _ = self.close_shown_document();
            return;
        }
        if let Err(e) = self.run("file.close", json!({})) {
            log::warn!("closing a window: {e}");
        }
    }

    /// The id of the drawn window's own `name` in egui's memory, which every viewport shares: a
    /// search box's text, a gallery's first tile.
    pub(crate) fn window_id(&self, name: &'static str) -> egui::Id {
        egui::Id::unique(name).with(self.windows.shown())
    }
}

#[cfg(test)]
mod tests;
