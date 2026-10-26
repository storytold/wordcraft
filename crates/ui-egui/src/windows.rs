//! One native window per open document. Each window is an egui viewport, driven by
//! [`craft_window::Viewports`]: one window draws in the root viewport, every other one in an
//! immediate viewport of its own, and each draws the whole interface (title bar, ribbon, page,
//! status bar, dialogs) for its own document. [`WordApp`]'s fields hold the drawn window's state
//! ([`crate::window_ui`]); the other windows keep theirs in their [`Window`]. Between frames the
//! fields hold the active window's (the one focused last), so the control channel, the MCP
//! bridge and the host's menus act on it.
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

use std::collections::HashMap;

use craft_window::{Document, Frame, Viewports, WindowEvent, WindowId, WindowList, WindowMenu};
use egui::{ViewportCommand, ViewportId};
use serde_json::{Value, json};
use wordcraft_engine::WindowView;

use crate::{WordApp, documents, window_ui::WindowUi};

/// The window title for a document called `name`.
pub(crate) fn title(name: &str, dirty: bool) -> String {
    format!("{name}{} - WordCraft", if dirty { " •" } else { "" })
}

/// An open window.
pub(crate) struct Window {
    /// The app's key for the window. A window made while the windows are being drawn gets its
    /// [`WindowId`] once the frame's drawing ends.
    key: u64,
    id: Option<WindowId>,
    /// The document it shows.
    doc: u64,
    /// Its state while another window's is in [`WordApp`]'s fields; `None` for that window.
    ui: Option<Box<WindowUi>>,
    /// Requests commands made for its document while another window was drawn
    /// ([`WordApp::handle_request`]), handled when it draws next.
    requests: Vec<Value>,
    /// The title it last drew, for its viewport.
    title: String,
    /// Its view of the document (caret, zoom, view mode…) while another window on the same
    /// document is drawn. `None` while the session holds it: for the drawn window, and for the
    /// window on each other document whose view is parked with that document.
    view: Option<WindowView>,
}

/// The app's windows. See the module documentation.
pub struct Windows {
    /// Separate native windows (desktop). Off, one window shows every document in turn (web).
    native: bool,
    /// Every window, with its key, for the viewport driver: opening and focus order, positions.
    /// Lent to the driver while the windows draw.
    list: WindowList<u64>,
    viewports: Option<Viewports>,
    /// Open windows in opening order.
    open: Vec<Window>,
    /// The window whose state is in [`WordApp`]'s fields.
    shown: u64,
    next_key: u64,
    /// The windows are being drawn: `list` is lent out, so windows opened or closed now join or
    /// leave it when the drawing ends.
    showing: bool,
    /// Window keys, most recently focused first, as the drawing started.
    recent: Vec<u64>,
    /// A window to bring to the front when the drawing ends.
    focus: Option<u64>,
    /// View › Window › Switch Windows and the window titles, as the frame's drawing started.
    menu: WindowMenu,
    /// What a new window's viewport starts from: size, icon, app id ([`WordApp::set_window_builder`]).
    builder: egui::ViewportBuilder,
    /// [`ViewportInput`] is registered with egui.
    input_plugin: bool,
    /// The viewport driver draws the windows this frame, and titles them.
    driven: bool,
}

impl Windows {
    /// One window, showing document `doc`.
    pub(crate) fn new(doc: u64) -> Windows {
        let mut list = WindowList::new();
        let id = list.open(1);
        let mut viewports = Viewports::new(Frame::Desktop);
        viewports.set_root(Some(id));
        Windows {
            native: cfg!(not(target_arch = "wasm32")),
            list,
            viewports: Some(viewports),
            open: vec![Window { key: 1, id: Some(id), doc, ui: None, requests: Vec::new(), title: String::new(), view: None }],
            shown: 1,
            next_key: 2,
            showing: false,
            recent: vec![1],
            focus: None,
            menu: WindowMenu::default(),
            builder: egui::ViewportBuilder::default().with_inner_size([1440.0, 920.0]).with_min_inner_size([760.0, 480.0]),
            input_plugin: false,
            driven: false,
        }
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

    /// Whether the viewport driver titles the windows (else the drawn window sends its title).
    pub(crate) fn driven(&self) -> bool {
        self.driven
    }

    /// The key of the window whose state is in [`WordApp`]'s fields.
    pub(crate) fn shown(&self) -> u64 {
        self.shown
    }

    fn get(&self, key: u64) -> Option<&Window> {
        self.open.iter().find(|w| w.key == key)
    }

    fn get_mut(&mut self, key: u64) -> Option<&mut Window> {
        self.open.iter_mut().find(|w| w.key == key)
    }

    /// The window showing document `doc`, the most recently focused if several do.
    fn of_doc(&self, doc: u64) -> Option<u64> {
        self.recent_keys().into_iter().find(|&k| self.get(k).is_some_and(|w| w.doc == doc))
    }

    /// Open windows, most recently focused first; windows opened during this frame's drawing
    /// come first.
    fn recent_keys(&self) -> Vec<u64> {
        let mut keys: Vec<u64> = self.open.iter().rev().filter(|w| w.id.is_none()).map(|w| w.key).collect();
        let listed: Vec<u64> =
            if self.showing { self.recent.clone() } else { self.list.recent().filter_map(|id| self.list.get(id).copied()).collect() };
        keys.extend(listed.into_iter().filter(|&k| self.get(k).is_some()));
        for w in &self.open {
            if !keys.contains(&w.key) {
                keys.push(w.key);
            }
        }
        keys
    }

    /// The document of the window focused last before `key`'s, if any.
    fn next_doc(&self, key: u64) -> Option<u64> {
        if !self.native {
            return None;
        }
        self.recent_keys().into_iter().find(|&k| k != key).and_then(|k| self.get(k)).map(|w| w.doc)
    }

    /// The window's viewport.
    fn viewport(&self, key: u64) -> ViewportId {
        match (self.get(key).and_then(|w| w.id), &self.viewports) {
            (Some(id), Some(v)) => v.viewport_id(id),
            _ => ViewportId::ROOT,
        }
    }

    /// The title View › Window › Switch Windows gives the window, as the frame started.
    fn menu_title(&self, key: u64) -> Option<&str> {
        self.menu.title(self.get(key)?.id?)
    }

    /// Bring the list up to date with the windows opened, closed and focused since it was last
    /// lent out, and give the root viewport a window when its own closed.
    fn settle(&mut self, ctx: Option<&egui::Context>) {
        if self.showing {
            return;
        }
        let gone: Vec<WindowId> = self.list.ids().filter(|id| !self.open.iter().any(|w| w.id == Some(*id))).collect();
        for id in gone {
            self.list.close(id);
        }
        let active = self.list.active();
        let mut opened = false;
        for w in self.open.iter_mut().filter(|w| w.id.is_none()) {
            w.id = Some(self.list.open(w.key));
            opened = true;
        }
        let focus = self.focus.take().and_then(|k| self.get(k)).and_then(|w| w.id);
        // A window opened behind the others (nothing asked to focus it) leaves the active one so.
        if let Some(id) = focus.or(active.filter(|_| opened)) {
            self.list.focus(id);
        }
        let Some(viewports) = self.viewports.as_mut() else { return };
        if !viewports.root().is_some_and(|r| self.list.contains(r))
            && let Some(next) = self.list.active()
        {
            match ctx {
                Some(ctx) => viewports.adopt(ctx, &self.list, next),
                None => viewports.set_root(Some(next)),
            }
        }
        if let (Some(id), Some(ctx)) = (focus, ctx) {
            viewports.focus(ctx, id);
        }
        if opened && let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    }
}

/// Control-channel input for a window in a viewport of its own: eframe's raw-input hook
/// ([`WordApp::raw_input_hook`]) sees only the root viewport's input, and egui runs a plugin's
/// input hook for every viewport. On macOS it also reads Control-click as the secondary button
/// in those windows, as the raw-input hook does in the root's.
#[derive(Default)]
pub(crate) struct ViewportInput {
    /// The viewport the queued events go to, on its next pass.
    target: Option<ViewportId>,
    events: Vec<egui::Event>,
    /// A viewport whose pass got keyboard input this way, until its window has looked.
    keys_for: Option<ViewportId>,
    control_click: HashMap<ViewportId, bool>,
}

impl egui::Plugin for ViewportInput {
    fn debug_name(&self) -> &'static str {
        "wordcraft_viewport_input"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        let viewport = input.viewport_id;
        if viewport == ViewportId::ROOT {
            return;
        }
        if cfg!(target_os = "macos") {
            crate::control_click_as_secondary(&mut input.events, self.control_click.entry(viewport).or_default());
        }
        if self.target != Some(viewport) || self.events.is_empty() {
            return;
        }
        self.target = None;
        let events = std::mem::take(&mut self.events);
        if events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))) {
            input.focused = true;
            self.keys_for = Some(viewport);
        }
        input.events.extend(events);
    }
}

impl WordApp {
    /// How many windows are open.
    pub fn window_count(&self) -> usize {
        if self.windows.native { self.windows.open.len() } else { 1 }
    }

    /// The egui viewport of the active window, the one focused last: control input,
    /// `ui.screenshot`, `ui.resize` and the saved window geometry are its. The root viewport
    /// when the windows don't have viewports of their own (the web app, embedded viewports).
    pub fn active_viewport(&self) -> ViewportId {
        if !self.windows.native || !self.windows.driven {
            return ViewportId::ROOT;
        }
        let key = self.windows.recent_keys().first().copied().unwrap_or(self.windows.shown);
        self.windows.viewport(key)
    }

    /// The active window's viewport and surface, for `ui.screenshot`: the root's from this
    /// frame's raw input, another window's from the input it last drew with (the host's raw-input
    /// hook sees only the root's).
    pub(crate) fn active_surface_geometry(&self, ctx: &egui::Context) -> (ViewportId, Option<crate::control::SurfaceGeometry>) {
        let viewport = self.active_viewport();
        if viewport == ViewportId::ROOT {
            return (viewport, self.surface_geometry);
        }
        let (info, screen_rect, max_texture_side) = ctx.input_for(viewport, |i| (i.viewport().clone(), i.raw.screen_rect, i.raw.max_texture_side));
        (viewport, crate::control::SurfaceGeometry::from_viewport(&info, screen_rect, max_texture_side, ctx))
    }

    /// The viewport new windows start from: the host's size, icon and app id. Their title and
    /// place are the app's.
    pub fn set_window_builder(&mut self, builder: egui::ViewportBuilder) {
        self.windows.builder = builder;
    }

    /// Whether a close request for the root viewport closes only the window drawn there: true
    /// while other windows are open. [`WordApp::ui`] closes it (a [`WindowEvent::CloseRequested`]),
    /// and the host keeps the app running.
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
        if key == self.windows.shown {
            return true;
        }
        let session = &self.session;
        let Some(w) = self.windows.get_mut(key).filter(|w| w.ui.as_ref().is_some_and(|ui| session.is_open(ui.shown_document))) else {
            return false;
        };
        let Some(mut state) = w.ui.take() else { return false };
        let view = w.view.take();
        let from = self.shown_document;
        self.swap_window(&mut state);
        let left = std::mem::replace(&mut self.windows.shown, key);
        if let Some(w) = self.windows.get_mut(left) {
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
        if let Some(owner) = self.windows.open.iter_mut().find(|w| w.key != key && w.doc == doc && w.view.is_none()) {
            owner.view = Some(held);
        }
        self.session.put_view(view);
    }

    /// Open a new window for open document `doc`, focused, and draw it. The window that was drawn
    /// leaves the File page: whatever it was opening is open. `view` is the new window's view of
    /// a document another window shows (View › New Window); `None` for a document no window
    /// shows yet.
    fn open_window(&mut self, doc: u64, view: Option<WindowView>) {
        let key = self.windows.next_key;
        self.windows.next_key = self.windows.next_key.saturating_add(1);
        let mut state = Box::new(WindowUi::new(doc));
        // Faces borrowed for the interface (non-Latin file names) stay installed.
        state.canvas.ime_faces = self.canvas.ime_faces.clone();
        self.ui.backstage = false;
        self.windows.open.push(Window { key, id: None, doc, ui: Some(state), requests: Vec::new(), title: String::new(), view });
        self.enter_window(key);
        self.focus_window(key);
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
        let doc = self.windows.get(shown)?.doc;
        if !self.windows.native || !self.windows.open.iter().any(|w| w.key != shown && w.doc == doc) {
            return None;
        }
        self.windows.recent_keys().into_iter().find(|&k| k != shown)
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
        self.windows.open.retain(|w| w.key != closing);
        self.focus_window(to);
    }

    /// Bring window `key` to the front (now, or once the frame's drawing ends) and make it the
    /// active one.
    pub(crate) fn focus_window(&mut self, key: u64) {
        self.windows.focus = Some(key);
        self.settle_windows();
    }

    /// [`Windows::settle`]; a window that moved into the root viewport keeps its place on the
    /// page (the page's scroll state is the viewport's).
    fn settle_windows(&mut self) {
        let root = |w: &Windows| w.viewports.as_ref().and_then(Viewports::root);
        let before = root(&self.windows);
        self.windows.settle(self.ctx.as_ref());
        let Some(adopted) = root(&self.windows).filter(|r| before.is_some_and(|b| b != *r)) else { return };
        let Some(key) = self.windows.list.get(adopted).copied() else { return };
        if key == self.windows.shown {
            self.canvas.scroll_request = Some(self.canvas.scroll_offset);
        } else if let Some(w) = self.windows.get_mut(key).and_then(|w| w.ui.as_mut()) {
            w.canvas.scroll_request = Some(w.canvas.scroll_offset);
        }
    }

    /// Keep a window for every open document: windows whose document closed (an agent closed it)
    /// close, and documents with no window (an agent made one while another was active) get one
    /// behind the others. The active document is left to [`WordApp::sync_document`].
    pub(crate) fn prune_windows(&mut self) {
        let shown = self.windows.shown;
        let session = &self.session;
        let before = self.windows.open.len();
        self.windows.open.retain(|w| w.key == shown || session.is_open(w.doc));
        let mut changed = self.windows.open.len() != before;
        let active = self.session.document_id();
        for d in self.session.documents() {
            if d.id == active || Some(d.id) == self.no_document || d.id == self.shown_document || self.windows.of_doc(d.id).is_some() {
                continue;
            }
            let key = self.windows.next_key;
            self.windows.next_key = self.windows.next_key.saturating_add(1);
            let mut state = Box::new(WindowUi::new(d.id));
            state.canvas.ime_faces = self.canvas.ime_faces.clone();
            self.windows.open.push(Window { key, id: None, doc: d.id, ui: Some(state), requests: Vec::new(), title: String::new(), view: None });
            changed = true;
        }
        if changed {
            self.settle_windows();
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
        let other = self.windows.of_doc(id).filter(|&k| k != self.windows.shown);
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
        if let Some(w) = self.windows.get_mut(shown) {
            w.doc = id;
        }
    }

    /// Close the active document. With windows, its window closes and the window focused before
    /// it comes forward; else the window shows the next document. The session leaves a new blank
    /// document when it was the last.
    pub(crate) fn close_shown_document(&mut self) -> Result<(), String> {
        let doc = self.session.document_id();
        let next = self.windows.next_doc(self.windows.shown);
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
        let Some(key) = self.windows.of_doc(document).filter(|&k| k != self.windows.shown) else { return false };
        if let Some(w) = self.windows.get_mut(key) {
            w.requests.push(req.clone());
        }
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
        true
    }

    /// View › Window › Switch Windows with native windows: every window by its title (two
    /// windows on one document as `Name:1`, `Name:2`) with its key, the active one marked,
    /// unsaved ones with the title bar's dot. Choosing one brings it to the front
    /// ([`WordApp::show_window`]).
    pub(crate) fn switch_windows_entries(&self) -> Vec<(u64, String, bool)> {
        self.windows
            .menu
            .entries()
            .iter()
            .filter_map(|e| {
                let w = self.windows.open.iter().find(|w| w.id == Some(e.id))?;
                Some((w.key, format!("{}{}", e.title, if e.dirty { " •" } else { "" }), w.key == self.windows.shown))
            })
            .collect()
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
        let name = self.windows.menu_title(self.windows.shown).map(str::to_string).unwrap_or_else(|| self.title_stem());
        title(&name, self.session.dirty)
    }

    /// Draw every window, each in its viewport (see the module documentation).
    pub(crate) fn windows_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.prune_windows();
        self.settle_windows();
        self.windows.menu = self.window_menu(&self.windows.list);
        // A backend that can't open native windows (egui embeds them in the root: tests,
        // `ui_shot`) shows the active window alone. Embedded, every window would draw the same
        // widgets in one viewport.
        self.windows.driven = !ctx.embed_viewports() && self.windows.viewports.is_some();
        if !self.windows.driven {
            let active = self.windows.recent_keys().first().copied().unwrap_or(self.windows.shown);
            self.draw_window(ui, active);
            self.after_windows();
            return;
        }
        let Some(mut viewports) = self.windows.viewports.take() else { return };
        let mut list = std::mem::take(&mut self.windows.list);
        self.windows.recent = list.recent().filter_map(|id| list.get(id).copied()).collect();
        let titles: HashMap<u64, String> = self.windows.open.iter().map(|w| (w.key, w.title.clone())).collect();
        let base = self.windows.builder.clone();
        self.windows.showing = true;
        let events = viewports.show(
            ui,
            &mut list,
            |_, key| match titles.get(key).filter(|t| !t.is_empty()) {
                Some(t) => base.clone().with_title(t.clone()),
                None => base.clone().with_title("WordCraft"),
            },
            |ui, _, key| self.draw_window(ui, *key),
        );
        self.windows.showing = false;
        self.windows.list = list;
        self.windows.viewports = Some(viewports);
        self.window_events(&ctx, &events);
        self.after_windows();
    }

    /// After the frame's drawing: windows opened, closed or focused meanwhile settle, and the
    /// active window's state comes back.
    fn after_windows(&mut self) {
        self.prune_windows();
        self.settle_windows();
        if let Some(&active) = self.windows.recent_keys().first() {
            self.enter_window(active);
        }
    }

    /// View › Window › Switch Windows for the windows in `list`.
    fn window_menu(&self, list: &WindowList<u64>) -> WindowMenu {
        let docs = self.session.documents();
        let doc_of: HashMap<u64, u64> = self.windows.open.iter().map(|w| (w.key, w.doc)).collect();
        WindowMenu::new(list, |_, key| {
            let doc = doc_of.get(key).copied().unwrap_or_default();
            match docs.iter().find(|d| d.id == doc) {
                Some(d) => Document { key: doc, name: documents::display_title(d), dirty: d.dirty },
                None => Document { key: doc, name: String::new(), dirty: false },
            }
        })
    }

    /// Draw window `key` into its viewport's `ui`.
    fn draw_window(&mut self, ui: &mut egui::Ui, key: u64) {
        if !self.enter_window(key) {
            return;
        }
        let ctx = ui.ctx().clone();
        let queued = self.windows.get_mut(key).map(|w| std::mem::take(&mut w.requests)).unwrap_or_default();
        for req in queued {
            self.handle_request(&req);
        }
        self.control_keys_arrived(&ctx);
        self.window_work(&ctx);
        self.window_input(&ctx);
        if self.windows.shown != key {
            return;
        }
        self.window_ui(ui);
        // Drawing can close the window or bring another forward: the fields then hold another
        // window's state.
        if self.windows.shown != key {
            return;
        }
        let title = self.sent_title.clone();
        if let Some(w) = self.windows.get_mut(key) {
            w.title = title;
        }
    }

    /// The windows' focus changes and close requests from this frame.
    fn window_events(&mut self, ctx: &egui::Context, events: &[WindowEvent]) {
        for event in events {
            let WindowEvent::CloseRequested(id) = *event else { continue };
            let Some(key) = self.windows.list.get(id).copied() else { continue };
            // The last window's close button quits: the host asks first ([`WordApp::close_requested`]).
            if self.windows.open.len() <= 1 || self.quit_requested {
                continue;
            }
            if self.windows.viewports.as_ref().and_then(Viewports::root) == Some(id) {
                ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::CancelClose);
            }
            self.close_window(key);
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

    /// The control channel's next input, for the active window when it has a viewport of its own:
    /// handed to [`ViewportInput`], which gives it to that viewport's next pass.
    pub(crate) fn send_control_input(&mut self, ctx: &egui::Context) {
        if self.synthetic.is_empty() || self.control_input_is_roots(ctx) {
            return;
        }
        if !self.windows.input_plugin {
            ctx.add_plugin(ViewportInput::default());
            self.windows.input_plugin = true;
        }
        self.settle_windows();
        let target = self.active_viewport();
        // Input queued for a window that has closed since (or moved into the root viewport)
        // would wait forever.
        let live: Vec<ViewportId> = self.windows.open.iter().map(|w| self.windows.viewport(w.key)).filter(|&v| v != ViewportId::ROOT).collect();
        ctx.with_plugin(|p: &mut ViewportInput| {
            if p.target.is_some_and(|t| !live.contains(&t)) {
                p.target = None;
                p.events.clear();
            }
        });
        let waiting = ctx.with_plugin(|p: &mut ViewportInput| !p.events.is_empty()).unwrap_or(true);
        if waiting {
            return;
        }
        let Some(events) = self.next_synthetic() else { return };
        ctx.with_plugin(|p: &mut ViewportInput| {
            p.target = Some(target);
            p.events = events;
        });
        ctx.request_repaint_of(target);
    }

    /// Whether control input goes in through the root viewport's raw input: one window, the
    /// active window drawn there, or viewports embedded in the root (tests).
    pub(crate) fn control_input_is_roots(&self, ctx: &egui::Context) -> bool {
        !self.windows.native || ctx.embed_viewports() || self.active_viewport() == ViewportId::ROOT
    }

    /// Keys from the control channel reached this window's viewport: the page counts as focused
    /// when its widget has the keyboard, as [`WordApp::raw_input_hook`] does for the root.
    fn control_keys_arrived(&mut self, ctx: &egui::Context) {
        if !self.windows.input_plugin {
            return;
        }
        let viewport = ctx.viewport_id();
        let arrived = ctx
            .with_plugin(|p: &mut ViewportInput| {
                let hit = p.keys_for == Some(viewport);
                if hit {
                    p.keys_for = None;
                }
                hit
            })
            .unwrap_or(false);
        if arrived && let Some((id, _)) = self.canvas.widget_id {
            self.canvas.focused = ctx.memory(|m| m.has_focus(id));
        }
    }

    /// The id of the drawn window's own `name` in egui's memory, which every viewport shares: a
    /// search box's text, a gallery's first tile.
    pub(crate) fn window_id(&self, name: &'static str) -> egui::Id {
        egui::Id::unique(name).with(self.windows.shown)
    }

    /// The windows other than the drawn one, for state every window contributes to (the faces
    /// borrowed for the interface).
    pub(crate) fn other_windows(&self) -> impl Iterator<Item = &WindowUi> {
        self.windows.open.iter().filter_map(|w| w.ui.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use egui::{RawInput, ViewportCommand, ViewportEvent, ViewportId, ViewportIdMap, ViewportInfo};
    use serde_json::json;
    use wordcraft_doc::StoryRef;
    use wordcraft_engine::Session;

    use crate::{Services, WordApp};

    thread_local! {
        /// What the test backend tells each viewport about itself (focus, close requests).
        static INFOS: RefCell<ViewportIdMap<ViewportInfo>> = RefCell::new(ViewportIdMap::default());
        /// Input events for each viewport's next pass.
        static EVENTS: RefCell<HashMap<ViewportId, Vec<egui::Event>>> = RefCell::new(HashMap::new());
        /// The viewports that drew in the last frame, in order.
        static DRAWN: RefCell<Vec<ViewportId>> = const { RefCell::new(Vec::new()) };
        /// Viewport commands the app sent, by viewport.
        static COMMANDS: RefCell<HashMap<ViewportId, Vec<ViewportCommand>>> = RefCell::new(HashMap::new());
        /// Textures uploaded (page rasters, the font atlas) since the counter was reset.
        static UPLOADS: RefCell<usize> = const { RefCell::new(0) };
        /// Each viewport's last input-method output.
        static IME: RefCell<HashMap<ViewportId, Option<egui::output::IMEOutput>>> = RefCell::new(HashMap::new());
    }

    fn input(id: ViewportId) -> RawInput {
        let mut raw = RawInput {
            viewport_id: id,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))),
            ..Default::default()
        };
        raw.viewports = INFOS.with(|i| i.borrow().clone());
        raw.focused = raw.viewports.entry(id).or_default().focused.unwrap_or(false);
        raw.events = EVENTS.with(|e| e.borrow_mut().remove(&id).unwrap_or_default());
        raw
    }

    fn keep(out: egui::FullOutput, id: ViewportId) {
        DRAWN.with(|d| d.borrow_mut().push(id));
        UPLOADS.with(|u| *u.borrow_mut() += out.textures_delta.set.len());
        IME.with(|i| i.borrow_mut().insert(id, out.platform_output.ime.clone()));
        for (v, o) in &out.viewport_output {
            COMMANDS.with(|c| c.borrow_mut().entry(*v).or_default().extend(o.commands.iter().cloned()));
        }
        out.drop_without_applying_deltas();
    }

    /// A backend with native viewports, as eframe's: each immediate viewport runs a pass of its
    /// own, with its own input.
    fn backend() -> egui::Context {
        INFOS.with(|i| i.borrow_mut().clear());
        EVENTS.with(|e| e.borrow_mut().clear());
        COMMANDS.with(|c| c.borrow_mut().clear());
        UPLOADS.with(|u| *u.borrow_mut() = 0);
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        egui::Context::set_immediate_viewport_renderer(|ctx, viewport| {
            let id = viewport.ids.this;
            let mut raw = input(id);
            raw.viewports.entry(id).or_default().parent = Some(viewport.ids.parent);
            let mut cb = viewport.viewport_ui_cb;
            keep(ctx.run_ui(raw, |ui| cb(ui)), id);
        });
        ctx
    }

    /// One frame of every window.
    fn frame(ctx: &egui::Context, a: &mut WordApp) {
        DRAWN.with(|d| d.borrow_mut().clear());
        let mut raw = input(ViewportId::ROOT);
        a.raw_input_hook(ctx, &mut raw);
        let out = ctx.run_ui(raw, |ui| {
            a.logic(ui.ctx());
            a.ui(ui);
        });
        keep(out, ViewportId::ROOT);
        INFOS.with(|i| i.borrow_mut().values_mut().for_each(|v| v.events.clear()));
    }

    fn frames(ctx: &egui::Context, a: &mut WordApp, n: usize) {
        for _ in 0..n {
            frame(ctx, a);
        }
    }

    /// Focus viewport `id` alone, as the desktop does when a window is clicked.
    fn focus(id: ViewportId) {
        INFOS.with(|i| {
            let mut i = i.borrow_mut();
            for v in i.values_mut() {
                v.focused = Some(false);
            }
            i.entry(id).or_default().focused = Some(true);
        });
    }

    fn sent(id: ViewportId, cmd: &ViewportCommand) -> bool {
        COMMANDS.with(|c| c.borrow().get(&id).is_some_and(|v| v.contains(cmd)))
    }

    fn app(text: &str) -> WordApp {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        a.run("text.insert", json!({"text": text})).unwrap();
        a
    }

    fn text(a: &WordApp) -> String {
        a.session.doc.plain_text(StoryRef::Body).trim().to_string()
    }

    fn prompt(a: &WordApp) -> Option<&'static str> {
        a.dialog.as_ref().map(|d| d.name())
    }

    /// New opens the document in a second native window, focused, with a fresh interface; the
    /// first keeps its document, its ribbon tab and its open dialog, and gets them back when it
    /// is focused again.
    #[test]
    fn new_opens_a_second_window_and_each_keeps_its_state() {
        let ctx = backend();
        let mut a = app("First");
        let first = a.session.document_id();
        frames(&ctx, &mut a, 3);
        a.ui.tab = "Insert".into();
        a.run("ui.dialog", json!({"name": "paragraph"})).unwrap();
        assert_eq!(prompt(&a), Some("paragraph"));

        a.run("file.new", json!({})).unwrap();
        let second = a.session.document_id();
        assert_ne!(second, first);
        assert_eq!(a.window_count(), 2);
        assert!(a.dialog.is_none() && a.ui.tab == crate::DEFAULT_TAB, "the new window starts afresh");
        frames(&ctx, &mut a, 3);
        let child = a.active_viewport();
        assert_ne!(child, ViewportId::ROOT, "the new window has a viewport of its own");
        assert_eq!(DRAWN.with(|d| d.borrow().clone()), [child, ViewportId::ROOT], "both windows draw each frame");
        frames(&ctx, &mut a, 3);
        assert!(UPLOADS.with(|u| *u.borrow()) > 0, "the pages were rendered");
        UPLOADS.with(|u| *u.borrow_mut() = 0);
        frames(&ctx, &mut a, 2);
        assert_eq!(UPLOADS.with(|u| *u.borrow()), 0, "a window whose document didn't change renders no page again");
        assert!(a.sent_title.starts_with("Document2"), "{}", a.sent_title);
        assert_eq!(a.session.document_id(), second);
        a.run("text.insert", json!({"text": "Second"})).unwrap();

        focus(ViewportId::ROOT);
        frames(&ctx, &mut a, 2);
        assert_eq!(a.active_viewport(), ViewportId::ROOT);
        assert_eq!(a.session.document_id(), first);
        assert_eq!((text(&a), a.ui.tab.as_str(), prompt(&a)), ("First".into(), "Insert", Some("paragraph")));
        focus(child);
        frames(&ctx, &mut a, 2);
        assert_eq!((text(&a), a.ui.tab.as_str(), prompt(&a)), ("Second".into(), crate::DEFAULT_TAB, None));
    }

    fn docx_on_disk(name: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-windows-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.docx"));
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": body})).unwrap();
        s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
        path
    }

    /// Open of a file a window already shows brings that window to the front; Open with only an
    /// untouched blank document open replaces it in its own window.
    #[test]
    fn open_focuses_the_files_window_or_reuses_a_blank_one() {
        let path = docx_on_disk("report", "Report");
        let ctx = backend();
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        frames(&ctx, &mut a, 2);
        let window = a.windows.shown();
        a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!((a.window_count(), a.windows.shown(), text(&a).as_str()), (1, window, "Report"), "the blank document's window shows it");
        let report = a.session.document_id();

        a.run("file.new", json!({})).unwrap();
        frames(&ctx, &mut a, 2);
        assert_eq!(a.window_count(), 2);
        assert_ne!(a.session.document_id(), report);
        COMMANDS.with(|c| c.borrow_mut().clear());
        let r = a.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["alreadyOpen"], true);
        assert_eq!((a.window_count(), a.windows.shown(), a.session.document_id()), (2, window, report));
        frames(&ctx, &mut a, 1);
        assert!(sent(ViewportId::ROOT, &ViewportCommand::Focus), "the file's window comes to the front");
        assert_eq!(a.session.document_id(), report);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    /// A window's close button closes it, asking first about unsaved changes; after Don't Save the
    /// other window comes forward. When the root viewport's window closes, the other window moves
    /// into the root viewport.
    #[test]
    fn closing_a_window_asks_first_and_the_root_adopts_another() {
        let ctx = backend();
        let mut a = app("Kept");
        a.run("file.save", json!({"path": docx_on_disk("kept", "Kept").to_string_lossy()})).unwrap();
        let kept = a.session.document_id();
        frames(&ctx, &mut a, 2);
        a.run("file.new", json!({})).unwrap();
        a.run("text.insert", json!({"text": "Dropped"})).unwrap();
        frames(&ctx, &mut a, 2);
        let child = a.active_viewport();

        INFOS.with(|i| i.borrow_mut().entry(child).or_default().events.push(ViewportEvent::Close));
        frame(&ctx, &mut a);
        assert_eq!(prompt(&a), Some("saveChanges"), "the dirty window asks");
        assert_eq!((a.window_count(), a.active_viewport()), (2, child));
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert_eq!((a.window_count(), a.session.document_id(), text(&a).as_str()), (1, kept, "Kept"));
        assert_eq!(a.active_viewport(), ViewportId::ROOT);
        frames(&ctx, &mut a, 2);
        assert_eq!(DRAWN.with(|d| d.borrow().clone()), [ViewportId::ROOT], "the closed window's viewport is gone");

        a.run("file.new", json!({})).unwrap();
        let other = a.windows.shown();
        frames(&ctx, &mut a, 2);
        COMMANDS.with(|c| c.borrow_mut().clear());
        INFOS.with(|i| i.borrow_mut().entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close));
        assert!(a.closes_a_window(), "the host leaves the root's close to the app");
        frame(&ctx, &mut a);
        assert!(sent(ViewportId::ROOT, &ViewportCommand::CancelClose), "the app keeps running");
        assert!(!a.quit_requested);
        assert_eq!((a.window_count(), a.windows.shown()), (1, other));
        assert_eq!(a.active_viewport(), ViewportId::ROOT, "the remaining window moved into the root viewport");
        assert_eq!(a.canvas.scroll_request, Some(a.canvas.scroll_offset), "its page stays where it was");
        frames(&ctx, &mut a, 2);
        assert_eq!(DRAWN.with(|d| d.borrow().clone()), [ViewportId::ROOT]);
        assert!(a.has_document() && a.session.documents().len() == 1);
    }

    /// A request an agent's command makes for the document of another window is that window's:
    /// Print opens the Print page there when it draws, and the active window and document stay
    /// as they are, so the agent's next command works where it did.
    #[test]
    fn a_request_for_another_windows_document_is_handled_there() {
        let ctx = backend();
        let mut a = app("First");
        let first = a.session.document_id();
        let first_window = a.windows.shown();
        frames(&ctx, &mut a, 2);
        a.run("file.new", json!({})).unwrap();
        let second = a.session.document_id();
        frames(&ctx, &mut a, 2);
        let child = a.active_viewport();
        a.session.with_document(first, |s| s.run("file.print", &json!({}))).unwrap().unwrap();
        for req in std::mem::take(&mut a.session.ui_requests) {
            assert_eq!(req["document"], first);
            a.handle_request(&req);
        }
        assert_eq!((a.session.document_id(), a.active_viewport()), (second, child), "the active document stays");
        assert!(!a.ui.backstage);
        frames(&ctx, &mut a, 2);
        assert_eq!((a.session.document_id(), a.active_viewport()), (second, child));
        assert!(!a.ui.backstage, "the active window keeps its page");
        let other = a.windows.get(first_window).and_then(|w| w.ui.as_ref()).map(|w| (w.backstage, w.backstage_page.as_str()));
        assert_eq!(other, Some((true, "print")), "the Print page opened in the document's window");
    }

    /// Quitting from the Start screen's empty window asks about the documents other windows
    /// show (one an agent made and typed in).
    #[test]
    fn quitting_from_the_empty_window_asks_about_the_other_windows() {
        let ctx = backend();
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        frames(&ctx, &mut a, 2);
        a.run("file.close", json!({})).unwrap();
        assert!(!a.has_document());
        let empty = a.windows.shown();
        let placeholder = a.session.document_id();
        let other = a.session.with_document(placeholder, |s| s.add_document(wordcraft_doc::Document::new())).unwrap();
        a.session.with_document(other, |s| s.run("text.insert", &json!({"text": "Agent"}))).unwrap().unwrap();
        frames(&ctx, &mut a, 2);
        assert_eq!((a.window_count(), a.windows.shown()), (2, empty), "the agent's document has a window behind");
        assert!(!a.has_document());

        assert!(!a.quit());
        assert_eq!((a.session.document_id(), prompt(&a), text(&a).as_str()), (other, Some("saveChanges"), "Agent"));
        assert_ne!(a.windows.shown(), empty, "the document's own window asks");
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(a.quit_requested);
    }

    /// Mod+W in a window with no changes closes it; the keys typed after it in the same frame
    /// don't reach the window that comes forward.
    #[test]
    fn keys_after_closing_a_window_go_nowhere() {
        let ctx = backend();
        let mut a = app("First");
        frames(&ctx, &mut a, 2);
        a.run("file.new", json!({})).unwrap();
        let child = a.active_viewport();
        focus(child);
        frames(&ctx, &mut a, 8);
        assert!(a.canvas.focused);
        let mac = cfg!(target_os = "macos");
        let modifiers = egui::Modifiers { ctrl: !mac, mac_cmd: mac, command: true, ..Default::default() };
        EVENTS.with(|e| {
            e.borrow_mut().entry(child).or_default().extend([
                egui::Event::Key { key: egui::Key::W, physical_key: None, pressed: true, repeat: false, modifiers },
                egui::Event::Text("x".into()),
            ])
        });
        frame(&ctx, &mut a);
        assert_eq!(a.window_count(), 1, "Mod+W closed the window");
        frames(&ctx, &mut a, 2);
        assert_eq!(text(&a), "First");
    }

    /// A window whose document closed without the windows following (an agent closed it) isn't
    /// drawn or made active; the next frame closes it.
    #[test]
    fn a_window_whose_document_closed_is_not_entered() {
        let ctx = backend();
        let mut a = app("First");
        let first = a.session.document_id();
        let first_window = a.windows.shown();
        frames(&ctx, &mut a, 2);
        a.run("file.new", json!({})).unwrap();
        let (second, second_window) = (a.session.document_id(), a.windows.shown());
        frames(&ctx, &mut a, 2);
        a.session.close_document(first).unwrap();
        assert!(!a.enter_window(first_window));
        assert_eq!((a.windows.shown(), a.session.document_id()), (second_window, second));
        frames(&ctx, &mut a, 2);
        assert_eq!((a.window_count(), a.session.document_id()), (1, second));
    }

    /// A composition in one window shows and commits only there: the other window draws no
    /// composition and its document gets none of the text.
    #[test]
    fn a_composition_stays_in_its_window() {
        let ctx = backend();
        let mut a = app("A");
        frames(&ctx, &mut a, 2);
        a.run("file.new", json!({})).unwrap();
        a.run("text.insert", json!({"text": "B"})).unwrap();
        let b = a.active_viewport();
        focus(ViewportId::ROOT);
        frames(&ctx, &mut a, 8);
        assert_eq!(a.active_viewport(), ViewportId::ROOT);
        assert!(a.canvas.focused, "the page has the keyboard");
        EVENTS.with(|e| {
            e.borrow_mut()
                .entry(ViewportId::ROOT)
                .or_default()
                .push(egui::Event::Ime(egui::ImeEvent::Preedit { text: "にほん".into(), active_range_chars: None }))
        });
        frames(&ctx, &mut a, 2);
        assert_eq!(a.canvas.ime_preedit, "にほん");
        let other = a.windows.open.iter().find_map(|w| w.ui.as_ref());
        assert!(other.is_some_and(|w| w.canvas.ime_preedit.is_empty()), "the other window composes nothing");
        assert!(IME.with(|i| i.borrow().get(&ViewportId::ROOT).cloned().flatten()).is_some(), "the composing window places the candidate window");

        focus(b);
        frames(&ctx, &mut a, 2);
        assert!(a.canvas.ime_preedit.is_empty(), "the focused window has no composition");
        EVENTS.with(|e| e.borrow_mut().entry(ViewportId::ROOT).or_default().push(egui::Event::Ime(egui::ImeEvent::Commit("にほん".into()))));
        frames(&ctx, &mut a, 2);
        assert_eq!(text(&a), "B", "a commit in the other window doesn't land here");
    }

    /// Control-channel typing goes to the active window when it has a viewport of its own (the
    /// root's raw-input hook doesn't see that window's input), once.
    #[test]
    fn control_typing_reaches_the_active_window() {
        let ctx = backend();
        let mut a = app("First");
        frames(&ctx, &mut a, 2);
        a.run("file.new", json!({})).unwrap();
        let child = a.active_viewport();
        focus(child);
        frames(&ctx, &mut a, 8);
        assert!(a.canvas.focused, "the new window's page has the keyboard");
        let (req, _reply) = crate::ControlRequest::new("ui.text", json!({"text": "Typed"}));
        assert!(matches!(crate::control::handle(&mut a, &ctx, &req), crate::control::Outcome::Done(_)));
        frames(&ctx, &mut a, 4);
        assert_eq!(text(&a), "Typed");
        assert!(a.synthetic.is_empty());
        focus(ViewportId::ROOT);
        frames(&ctx, &mut a, 2);
        assert_eq!(text(&a), "First", "the other window got none of it");
    }

    /// `ui.screenshot` captures the active window: the capture is sized from its viewport and the
    /// command goes to that viewport. A capture still pending when another window comes forward
    /// fails with an error instead of capturing that window.
    #[test]
    fn screenshot_captures_the_active_window() {
        let ctx = backend();
        let mut a = app("First");
        frames(&ctx, &mut a, 2);
        a.run("file.new", json!({})).unwrap();
        let child = a.active_viewport();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        INFOS.with(|i| i.borrow_mut().entry(child).or_default().inner_rect = Some(rect));
        focus(child);
        frames(&ctx, &mut a, 4);

        let (reply, replies) = std::sync::mpsc::channel();
        crate::control::admit_screenshot(&mut a, &ctx, None, reply);
        assert!(replies.try_recv().is_err(), "the capture is admitted");
        assert_eq!(a.shots.requests.iter().map(|s| (s.viewport, s.expected)).collect::<Vec<_>>(), [(child, [800, 600])]);
        a.shots.requests.iter_mut().for_each(|s| s.ready_at = 0.0);
        COMMANDS.with(|c| c.borrow_mut().clear());
        frames(&ctx, &mut a, 4);
        let shot_to =
            |id: ViewportId| COMMANDS.with(|c| c.borrow().get(&id).is_some_and(|v| v.iter().any(|c| matches!(c, ViewportCommand::Screenshot(_)))));
        assert!(shot_to(child), "the active window is captured");
        assert!(!shot_to(ViewportId::ROOT), "the first window is not");

        let (reply, later) = std::sync::mpsc::channel();
        crate::control::admit_screenshot(&mut a, &ctx, None, reply);
        focus(ViewportId::ROOT);
        frames(&ctx, &mut a, 2);
        for r in [replies, later] {
            let v = r.try_recv().expect("the capture was answered");
            assert_eq!(v["ok"], false);
            assert!(v["error"].as_str().is_some_and(|e| e.contains("another window")), "{v}");
        }
        assert!(a.shots.requests.is_empty());
    }

    /// Quitting with two documents of unsaved changes asks about each in its window, the window
    /// coming to the front; Cancel keeps both.
    #[test]
    fn quitting_asks_about_each_document_in_its_window() {
        let ctx = backend();
        let mut a = app("One");
        let one = a.session.document_id();
        frames(&ctx, &mut a, 2);
        a.run("file.new", json!({})).unwrap();
        a.run("text.insert", json!({"text": "Two"})).unwrap();
        let two = a.session.document_id();
        let child = a.active_viewport();
        frames(&ctx, &mut a, 2);

        assert!(!a.quit());
        assert_eq!((a.session.document_id(), prompt(&a)), (two, Some("saveChanges")), "the active window asks first");
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();
        assert!(!a.quit_requested);
        assert_eq!((a.window_count(), a.session.documents().len()), (2, 2), "Cancel keeps both");

        assert!(!a.quit());
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert_eq!((a.session.document_id(), prompt(&a)), (one, Some("saveChanges")), "then the other window asks");
        assert_eq!(a.window_count(), 1);
        COMMANDS.with(|c| c.borrow_mut().clear());
        frame(&ctx, &mut a);
        assert!(sent(ViewportId::ROOT, &ViewportCommand::Focus), "its window comes to the front");
        assert!(!DRAWN.with(|d| d.borrow().contains(&child)));
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(a.quit_requested);
    }

    fn window_labels(a: &WordApp) -> Vec<String> {
        let mut l: Vec<String> = a.switch_windows_entries().into_iter().map(|(_, label, _)| label).collect();
        l.sort();
        l
    }

    /// View › New Window opens a second window on the document, titled `Name:1` / `Name:2`. Text
    /// typed in one shows in the other; each keeps its own caret and zoom. Typing in a window
    /// whose caret was in text the other window deleted lands at the end of what is left, and
    /// Undo in that window undoes its own typing, then the other window's delete.
    #[test]
    fn new_window_shows_the_document_twice_each_with_its_own_caret_and_zoom() {
        let ctx = backend();
        let mut a = app("Hello world");
        let doc = a.session.document_id();
        frames(&ctx, &mut a, 2);
        a.run("view.newWindow", json!({})).unwrap();
        assert_eq!((a.window_count(), a.session.document_id()), (2, doc));
        frames(&ctx, &mut a, 3);
        let b = a.active_viewport();
        assert_ne!(b, ViewportId::ROOT);
        assert_eq!(window_labels(&a), ["Document1:1 •", "Document1:2 •"]);
        assert_eq!(a.window_title(), super::title("Document1:2", true));

        a.run("view.zoom", json!({"value": 150})).unwrap();
        a.run("text.insert", json!({"text": "!"})).unwrap();
        focus(ViewportId::ROOT);
        frames(&ctx, &mut a, 2);
        assert_eq!(a.active_viewport(), ViewportId::ROOT);
        assert_eq!(text(&a), "Hello world!", "the edit shows in the other window");
        assert_eq!((a.session.view.zoom, a.session.sel.focus.off), (1.0, 11), "window A keeps its zoom and caret");

        a.run("select.text", json!({"text": " world!"})).unwrap();
        a.run("text.delete", json!({})).unwrap();
        focus(b);
        frames(&ctx, &mut a, 2);
        assert_eq!(a.active_viewport(), b);
        assert!((a.session.view.zoom - 1.5).abs() < 1e-6, "window B keeps its zoom");
        a.run("text.insert", json!({"text": "?"})).unwrap();
        assert_eq!(text(&a), "Hello?", "B's caret was clamped into what is left");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(text(&a), "Hello");
        a.run("edit.undo", json!({})).unwrap();
        assert_eq!(text(&a), "Hello world!", "Undo in B undoes A's delete");
        frames(&ctx, &mut a, 2);
        assert_eq!(a.session.documents().len(), 1);
    }

    /// Closing one of two windows on a document asks nothing, even with unsaved changes, and the
    /// other window goes back to the plain name; closing the last one asks. Quitting with a
    /// document in two windows asks once.
    #[test]
    fn closing_one_of_two_windows_on_a_document_asks_nothing() {
        let ctx = backend();
        let mut a = app("Kept");
        let doc = a.session.document_id();
        frames(&ctx, &mut a, 2);
        a.run("view.newWindow", json!({})).unwrap();
        frames(&ctx, &mut a, 2);
        let child = a.active_viewport();
        INFOS.with(|i| i.borrow_mut().entry(child).or_default().events.push(ViewportEvent::Close));
        frame(&ctx, &mut a);
        assert_eq!(prompt(&a), None, "no prompt while another window shows the document");
        assert_eq!((a.window_count(), a.session.document_id(), text(&a).as_str()), (1, doc, "Kept"));
        assert!(a.session.dirty);
        frames(&ctx, &mut a, 2);
        assert_eq!(a.window_title(), super::title("Document1", true));
        a.run("file.close", json!({})).unwrap();
        assert_eq!(prompt(&a), Some("saveChanges"), "the last window asks");
        a.run("ui.saveChanges", json!({"answer": "cancel"})).unwrap();

        a.run("view.newWindow", json!({})).unwrap();
        frames(&ctx, &mut a, 2);
        assert_eq!(a.window_count(), 2);
        assert!(!a.quit());
        assert_eq!(prompt(&a), Some("saveChanges"));
        a.run("ui.saveChanges", json!({"answer": "dontSave"})).unwrap();
        assert!(a.quit_requested, "one prompt for the document");
    }

    /// Two windows, the second (focused) on an untouched blank document: an app whose host
    /// answers file dialogs only when the test does, plus the dialog's answer channel.
    fn blank_beside_a_document() -> (egui::Context, WordApp, Shown, u64) {
        let shown: Shown = std::rc::Rc::default();
        let log = shown.clone();
        let services = Services {
            file_dialog: Some(Box::new(move |_| {
                let (tx, rx) = std::sync::mpsc::channel();
                log.borrow_mut().push(tx);
                rx
            })),
            ..Default::default()
        };
        let ctx = backend();
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        frames(&ctx, &mut a, 2);
        a.run("file.open", json!({"path": docx_on_disk("first", "First").to_string_lossy()})).unwrap();
        a.run("file.new", json!({})).unwrap();
        frames(&ctx, &mut a, 2);
        assert!(a.window_count() == 2 && a.session.is_untouched_blank());
        let blank = a.windows.shown();
        (ctx, a, shown, blank)
    }

    type Shown = std::rc::Rc<RefCell<Vec<std::sync::mpsc::Sender<crate::file_dialogs::FileDialogAnswer>>>>;

    /// Each way of opening a file replaces an untouched blank document in its own window, even
    /// when the file arrives while another window draws: File › Open, ⌘O and the File page's
    /// Browse (an asynchronous dialog), the File page's and Start screen's recent list, and
    /// Finder (between frames, in the focused window).
    #[test]
    fn every_open_replaces_an_untouched_blank_document_in_its_window() {
        let second = docx_on_disk("second", "Second");
        let path = second.to_string_lossy().to_string();
        let opens: [(&str, fn(&mut WordApp, &egui::Context, &Shown, &str)); 3] = [
            ("dialog", |a, ctx, shown, path| {
                a.run("ui.backstage", json!({"value": true, "page": "open"})).unwrap();
                a.run("ui.openFileDialog", json!({})).unwrap();
                frames(ctx, a, 2);
                let tx = shown.borrow_mut().pop().unwrap();
                tx.send(Ok(vec![path.to_string()])).unwrap();
                frames(ctx, a, 3);
            }),
            ("recent", |a, ctx, _, path| {
                crate::backstage::open_recent(a, path).unwrap();
                frames(ctx, a, 2);
            }),
            ("finder", |a, ctx, _, path| {
                a.open_file(path, None).unwrap();
                frames(ctx, a, 2);
            }),
        ];
        for (how, open) in opens {
            let (ctx, mut a, shown, blank) = blank_beside_a_document();
            open(&mut a, &ctx, &shown, &path);
            assert_eq!(a.window_count(), 2, "{how}: no third window");
            assert!(a.enter_window(blank), "{how}: the blank document's window is still open");
            assert_eq!(text(&a), "Second", "{how}: it shows the opened file");
        }
        std::fs::remove_dir_all(second.parent().unwrap()).ok();
    }
}
