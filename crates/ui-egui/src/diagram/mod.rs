//! A draw.io picture, edited in place.
//!
//! A PNG exported from draw.io carries the diagram's own XML ([`wordcraft_diagram::png`]), so it
//! can be opened again where it is and edited — the way Word opens a Visio object in place.
//! Double-click it (or pick Edit Diagram from its context menu) and draw.io runs in a child view
//! of the app's window, over the document; Save puts the new picture back with its XML inside it.
//!
//! The editor is [`editor::Editor`]: a view the host's window has to be there for, which is why
//! opening takes two steps. The UI asks ([`Diagram::request`]); the host opens it on its next
//! frame ([`Diagram::open`]), because only the host holds the window.

use egui::{self, Rect, Ui};
use serde_json::json;

use crate::WordApp;
use editor::{Editor, Event, Window};

/// macOS and Windows: draw.io in a child view of the app's window.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[path = "editor_native.rs"]
pub mod editor;

/// Everywhere else the same picture, read-only.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[path = "editor_absent.rs"]
pub mod editor;

/// Whether a picture can be edited here: the editor needs a native window (macOS, Windows).
pub const CAN_EDIT: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// Why there is no editor here (`None` where there is one): what `ui.editDiagram` answers when a
/// draw.io picture can't be opened.
pub fn unsupported() -> Option<&'static str> {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Some(editor::Editor::WHY)
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        None
    }
}

/// The strip above the editor that says what is being edited and how to leave it, in points.
const BAR: f32 = 30.0;

/// Is the selected picture a draw.io diagram (a PNG with the diagram's XML inside it)?
pub fn is_drawio_selected(app: &WordApp) -> bool {
    let Some(media) = app.selected_picture_media() else { return false };
    app.session.doc.media.get(&media).is_some_and(|b| wordcraft_diagram::png::is_drawio(b))
}

/// The editor over one picture, and what it has said.
#[derive(Default)]
pub struct Diagram {
    /// A picture the user asked to edit: opened once the host hands over its window.
    request: Option<String>,
    editor: Option<Editor>,
    /// Whether it has been put in place yet. It starts hidden, so it never flashes at a spot it
    /// isn't meant to be at.
    placed: bool,
    /// Save was pressed: waiting for the PNG draw.io answers with.
    saving: bool,
    /// Where to put it, in points, written by [`show`] and read by [`WordApp::diagram_frame`].
    rect: Option<Rect>,
}

impl Diagram {
    /// An editor is open over a picture.
    pub fn is_open(&self) -> bool {
        self.editor.is_some()
    }
    /// The media key of a picture waiting to be opened.
    pub fn pending(&self) -> Option<&str> {
        self.request.as_deref()
    }

    /// Edit the picture whose media key is `media`, on the frame the window is at hand.
    pub fn request(&mut self, media: String) {
        self.request = Some(media);
    }
    /// Drop a request that can't be served (the picture is gone).
    pub fn cancel(&mut self) {
        self.request = None;
    }

    /// Open draw.io over `png`, inside `window`.
    pub fn open(&mut self, window: &Window<'_>, png: &[u8]) -> Result<(), String> {
        self.request = None;
        self.saving = false;
        self.placed = false;
        self.editor = Some(Editor::open(window, png)?);
        Ok(())
    }

    /// What draw.io has said since the last frame.
    pub fn poll(&mut self) -> Vec<Event> {
        let Some(editor) = self.editor.as_mut() else { return Vec::new() };
        let events = editor.poll();
        // The diagram is on screen: give it the keyboard.
        if events.iter().any(|e| matches!(e, Event::Ready)) {
            editor.focus();
        }
        events
    }

    /// Ask draw.io for the picture: the answer comes back as an `Exported` event.
    pub fn save(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            self.saving = true;
            editor.save();
        }
    }

    /// Put the editor away.
    pub fn close(&mut self) {
        self.editor = None;
        self.saving = false;
        self.placed = false;
    }

    /// Where the editor goes, in points, written by [`show`] and read by [`WordApp::diagram_frame`].
    pub fn placement(&self) -> Option<Rect> {
        self.rect
    }

    /// Put the editor over `rect` (egui points, top-left origin) and show it. wry turns the
    /// top-left y into whatever the platform's native views count from (on macOS it flips the
    /// y itself, see its `window_position`), so no flipping here — doing it here too put the
    /// view upside down over the strip that says how to leave the editor.
    pub fn place(&mut self, rect: Rect) {
        let Some(editor) = self.editor.as_ref() else { return };
        editor.set_bounds(f64::from(rect.min.x), f64::from(rect.min.y.max(0.0)), f64::from(rect.width()), f64::from(rect.height()));
        if !self.placed {
            self.placed = true;
            editor.set_visible(true);
            editor.focus();
        }
    }
}

/// The document area, minus a strip for the bar while the editor is open: drawn after the
/// canvas, so the page can't paint over the strip that says how to leave the editor.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let full = ui.clip_rect();
    let d = &mut app.diagram;
    if !d.is_open() {
        d.rect = Some(full);
        return;
    }
    let (bar, rest) = full.split_top_bottom_at_y(full.min.y + BAR);
    d.rect = Some(rest);
    // Over the editor, so it stays reachable while draw.io has the keyboard.
    ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
        let t = crate::theme::Tokens::get(ui.ctx());
        egui::Frame::NONE.fill(t.canvas).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new(tl!("Editing diagram")).strong());
                ui.label(tl!("Save puts it back into the document; Cancel leaves it as it was."));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tl!("Cancel")).clicked() {
                        app.diagram.close();
                        app.status(tl!("The diagram was left as it was."));
                    }
                    let saving = app.diagram.saving;
                    if ui.add_enabled(!saving, egui::Button::new(tl!("Save"))).clicked() {
                        app.diagram.save();
                    }
                });
            });
        });
    });
    // Escape leaves as well: it commits, as clicking away from an activated object does.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.diagram.save();
    }
    // The view is a native child, so its place has to be kept up with the window: draw as long
    // as it is open.
    ui.ctx().request_repaint();
}

/// An `Exported` PNG, put back where the picture is (base64, as every command carries bytes).
pub fn apply(app: &mut WordApp, png: Vec<u8>) {
    let data = wordcraft_engine::cmd::insert::base64_encode(&png);
    match app.run("diagram.update", json!({"data": data})) {
        Ok(_) => app.status(tl!("The diagram was updated.")),
        Err(e) => app.status(e),
    }
}
