//! The built-in file browser (#576): Open, Save As, Export, Insert Picture and recipient lists
//! when the system's file dialog can't be shown — a Linux desktop with neither a file chooser
//! portal nor zenity (labwc, sway… with only xdg-desktop-portal-wlr) — or when the user asks for
//! it (File › Open › Browse in WordCraft…).
//!
//! A folder path field with Up and Home folder, the folder's subfolders and the files of the
//! chosen type, a file name field and the file types. It only lists folders the user navigates to
//! and never reads a file: the picked path goes to the same follow-up as the system dialog's.

use std::path::{Path, PathBuf};

use egui::{Align2, Rect, Sense, Ui, pos2, vec2};

use crate::file_dialogs::{AfterPick, FileDialogRequest, open_filters};
use crate::theme::{APP_COLOR, Tokens, regular};
use crate::{WordApp, icons};

/// At most this many entries are listed; a bigger folder shows the first ones and says so.
const MAX_LISTED: usize = 5000;
/// At most this many folder entries are read, so a huge folder can't stall the window.
const MAX_SCANNED: usize = 100_000;
const ROW: f32 = 24.0;

/// The browser while it is open, and the folder it showed last (where it opens next time).
#[derive(Default)]
pub(crate) struct State {
    pub(crate) open: Option<Box<FileBrowser>>,
    last_dir: Option<PathBuf>,
}

/// One subfolder or file in the listing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    /// The name as shown (a name that isn't valid UTF-8 is shown with replacement characters).
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) dir: bool,
}

/// A folder's subfolders (first) and files, by name.
#[derive(Debug, Default)]
pub(crate) struct Listing {
    pub(crate) entries: Vec<Entry>,
    /// The folder holds more than was listed.
    pub(crate) truncated: bool,
}

/// Whether `path` has one of `exts` (case-insensitive); `*` takes any file.
pub(crate) fn matches(path: &Path, exts: &[String]) -> bool {
    exts.iter().any(|e| e == "*") || path.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// The subfolders and the files with one of `exts` in `dir`, at most `cap` of them, folders first
/// and each by name. Hidden entries (a leading dot) are left out; entries that can't be read are
/// skipped.
pub(crate) fn list_dir(dir: &Path, exts: &[String], cap: usize) -> std::io::Result<Listing> {
    let mut listing = Listing::default();
    for (i, item) in std::fs::read_dir(dir)?.enumerate() {
        if i >= MAX_SCANNED {
            listing.truncated = true;
            break;
        }
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = item.path();
        // A symbolic link to a folder is a folder.
        let is_dir = match item.file_type() {
            Ok(t) if t.is_symlink() => std::fs::metadata(&path).is_ok_and(|m| m.is_dir()),
            Ok(t) => t.is_dir(),
            Err(_) => continue,
        };
        if !is_dir && !matches(&path, exts) {
            continue;
        }
        if listing.entries.len() >= cap {
            listing.truncated = true;
            break;
        }
        listing.entries.push(Entry { name, path, dir: is_dir });
    }
    listing.entries.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase()));
    Ok(listing)
}

/// The user's home folder.
pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// What the user typed in the file name field, as a path: relative to `dir`, absolute, or under
/// the home folder (`~/…`). `None` when nothing was typed.
pub(crate) fn resolve(dir: &Path, typed: &str) -> Option<PathBuf> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    if typed == "~" {
        return home();
    }
    if let Some(rest) = typed.strip_prefix("~/") {
        return home().map(|h| h.join(rest));
    }
    Some(dir.join(typed))
}

/// What OK (or Enter, or a double click) did.
#[derive(Debug, PartialEq)]
pub(crate) enum Step {
    /// The browser stays open (it moved to a folder, or has something to say).
    Stay,
    /// The file to open or save to.
    Pick(PathBuf),
}

/// A message under the listing.
#[derive(Clone, Debug, PartialEq)]
enum Note {
    None,
    NotFound(String),
    Replace(PathBuf),
}

/// The open browser.
pub(crate) struct FileBrowser {
    req: FileDialogRequest,
    pub(crate) after: AfterPick,
    /// Opened because the system dialog couldn't be shown: say why, and how to get it.
    unavailable: bool,
    dir: PathBuf,
    /// The folder field (edited by the user, applied on Enter).
    location: String,
    /// The file name field.
    pub(crate) name: String,
    /// The file types, as (label, extensions).
    filters: Vec<(String, Vec<String>)>,
    filter: usize,
    listing: Result<Listing, String>,
    /// The highlighted entry.
    selected: Option<PathBuf>,
    note: Note,
    focus_name: bool,
}

impl FileBrowser {
    pub(crate) fn new(req: FileDialogRequest, after: AfterPick, start: PathBuf, unavailable: bool) -> Self {
        let (name, filters) = match &req {
            FileDialogRequest::Open { purpose } | FileDialogRequest::OpenMany { purpose } => {
                (String::new(), open_filters(purpose).iter().map(|(l, e)| ((*l).to_string(), e.iter().map(|x| (*x).to_string()).collect())).collect())
            }
            FileDialogRequest::Save { name } => {
                let ext = Path::new(name).extension().and_then(|e| e.to_str()).filter(|e| !e.is_empty());
                let mut f: Vec<(String, Vec<String>)> = ext.map(|e| (format!("*.{e}"), vec![e.to_string()])).into_iter().collect();
                f.push(("All files".into(), vec!["*".into()]));
                (name.clone(), f)
            }
        };
        let mut b = FileBrowser {
            req,
            after,
            unavailable,
            dir: start.clone(),
            location: String::new(),
            name,
            filters,
            filter: 0,
            listing: Ok(Listing::default()),
            selected: None,
            note: Note::None,
            focus_name: true,
        };
        b.navigate(start);
        b
    }

    fn exts(&self) -> &[String] {
        self.filters.get(self.filter).map_or(&[], |(_, e)| e.as_slice())
    }

    /// Show the folder `dir` (an unreadable folder shows why, and Up still works).
    pub(crate) fn navigate(&mut self, dir: PathBuf) {
        self.listing = list_dir(&dir, self.exts(), MAX_LISTED).map_err(|e| e.to_string());
        self.location = dir.to_string_lossy().into_owned();
        self.dir = dir;
        self.selected = None;
        self.note = Note::None;
    }

    fn up(&mut self) {
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            self.navigate(parent);
        }
    }

    /// OK: a highlighted folder, or a typed folder, is opened; otherwise the typed name is the
    /// pick. Save adds the chosen type's extension to a name without one and asks once more
    /// before replacing a file; Open wants a file that exists.
    pub(crate) fn accept(&mut self) -> Step {
        if let Some(dir) = self.selected.take().filter(|p| p.is_dir()) {
            self.navigate(dir);
            return Step::Stay;
        }
        let Some(mut path) = resolve(&self.dir, &self.name) else { return Step::Stay };
        if path.is_dir() {
            self.name.clear();
            self.navigate(path);
            return Step::Stay;
        }
        match self.req {
            FileDialogRequest::Open { .. } | FileDialogRequest::OpenMany { .. } => {
                if path.is_file() {
                    return Step::Pick(path);
                }
                self.note = Note::NotFound(self.name.trim().to_string());
                Step::Stay
            }
            FileDialogRequest::Save { .. } => {
                if path.extension().is_none()
                    && let Some(ext) = self.exts().first().filter(|e| e.as_str() != "*")
                {
                    path.set_extension(ext);
                }
                if path.exists() && self.note != Note::Replace(path.clone()) {
                    self.note = Note::Replace(path);
                    return Step::Stay;
                }
                Step::Pick(path)
            }
        }
    }

    /// A click on an entry: highlight it, and a file's name goes into the file name field.
    fn select(&mut self, e: &Entry) {
        self.selected = Some(e.path.clone());
        if !e.dir {
            self.name = e.name.clone();
        }
        self.note = Note::None;
    }

    fn title(&self) -> &'static str {
        match self.req {
            FileDialogRequest::Open { .. } | FileDialogRequest::OpenMany { .. } => "Open",
            FileDialogRequest::Save { .. } => "Save As",
        }
    }

    fn ok_label(&self) -> &'static str {
        match self.req {
            FileDialogRequest::Open { .. } | FileDialogRequest::OpenMany { .. } => "Open",
            FileDialogRequest::Save { .. } => "Save",
        }
    }
}

impl WordApp {
    /// Ask for a file in the built-in browser and do `after` with the answer when it closes.
    pub(crate) fn show_file_browser(&mut self, req: FileDialogRequest, after: AfterPick, unavailable: bool) {
        let start = self
            .file_browser
            .last_dir
            .clone()
            .filter(|d| d.is_dir())
            .or_else(|| self.session.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf).filter(|d| d.is_dir()))
            .or_else(home)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("/"));
        self.file_browser.open = Some(Box::new(FileBrowser::new(req, after, start, unavailable)));
    }

    /// `ui.fileBrowser`: the built-in browser for `purpose` (document, picture, recipients, save),
    /// even when a system dialog was asked for and never showed up (it is given up as cancelled).
    pub(crate) fn browse_builtin(&mut self, purpose: &str) {
        self.give_up_file_dialog();
        if self.file_browser.open.is_some() {
            return;
        }
        let (req, after) = match purpose {
            "picture" => (FileDialogRequest::Open { purpose: "picture".into() }, AfterPick::Picture),
            "recipients" => (FileDialogRequest::Open { purpose: "recipients".into() }, AfterPick::Recipients),
            "save" => (FileDialogRequest::Save { name: self.default_save_name() }, AfterPick::SaveAs(crate::file_dialogs::AfterSave::Nothing)),
            _ => (FileDialogRequest::Open { purpose: "document".into() }, AfterPick::OpenDocument),
        };
        self.show_file_browser(req, after, false);
    }

    /// Close the browser with this answer (`None`: cancelled) and finish what it was for.
    pub(crate) fn close_file_browser(&mut self, picked: Option<PathBuf>) {
        let Some(b) = self.file_browser.open.take() else { return };
        self.file_browser.last_dir = Some(b.dir.clone());
        let picked = picked.map(|p| p.to_string_lossy().into_owned());
        if let Err(e) = self.finish_pick(b.after, picked) {
            log::warn!("file browser: {e}");
        }
        self.canvas.want_focus = true;
    }
}

/// Draw the browser, if it is open.
pub(crate) fn show(app: &mut WordApp, ctx: &egui::Context) {
    let Some(mut b) = app.file_browser.open.take() else { return };
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut done: Option<Option<PathBuf>> = None;
    egui::Window::new(tl!(b.title()))
        .id(egui::Id::new("builtin_file_browser"))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -20.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(620.0);
            done = body(ui, &mut b, &t);
        });
    if !open {
        done = Some(None);
    }
    app.file_browser.open = Some(b);
    if let Some(picked) = done {
        app.close_file_browser(picked);
    }
}

/// The browser's contents; `Some` once it closes (`Some(None)`: cancelled).
fn body(ui: &mut Ui, b: &mut FileBrowser, t: &Tokens) -> Option<Option<PathBuf>> {
    if b.unavailable {
        ui.label(
            egui::RichText::new(tl!(
                "The system file dialog isn't available, so WordCraft shows its own. To get the system dialog, install a file chooser portal (such as xdg-desktop-portal-gtk) or zenity."
            ))
            .color(t.text_dim),
        );
        ui.add_space(6.0);
    }
    let mut location_done = false;
    ui.horizontal(|ui| {
        if icon_button(ui, t, "chevronUp", tl!("Up one level"), b.dir.parent().is_some()) {
            b.up();
        }
        if icon_button(ui, t, "homeFolder", tl!("Home folder"), true)
            && let Some(h) = home()
        {
            b.navigate(h);
        }
        let r = ui.add(egui::TextEdit::singleline(&mut b.location).desired_width(f32::INFINITY));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            location_done = true;
            let typed = b.location.clone();
            if let Some(dir) = resolve(&b.dir, &typed) {
                b.navigate(dir);
            }
        }
    });
    ui.add_space(4.0);
    let mut pick = false;
    let mut go: Option<PathBuf> = None;
    let mut clicked: Option<Entry> = None;
    egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.input_border)).inner_margin(2.0).show(ui, |ui| {
        let entries: &[Entry] = b.listing.as_ref().map(|l| l.entries.as_slice()).unwrap_or(&[]);
        egui::ScrollArea::vertical().max_height(300.0).min_scrolled_height(300.0).auto_shrink([false, false]).show_rows(
            ui,
            ROW,
            entries.len(),
            |ui, range| {
                for e in entries.get(range).unwrap_or(&[]) {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                    if b.selected.as_ref() == Some(&e.path) {
                        ui.painter().rect_filled(r, 2.0, t.checked);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, 2.0, t.hover);
                    }
                    let icon = Rect::from_center_size(pos2(r.min.x + 14.0, r.center().y), vec2(18.0, 18.0));
                    icons::paint(ui.painter(), icon, if e.dir { "folder" } else { "file" }, t.icon, t.accent);
                    ui.painter().with_clip_rect(r).text(pos2(r.min.x + 30.0, r.center().y), Align2::LEFT_CENTER, &e.name, regular(13.0), t.text);
                    if resp.double_clicked() {
                        if e.dir {
                            go = Some(e.path.clone());
                        } else {
                            clicked = Some(e.clone());
                            pick = true;
                        }
                    } else if resp.clicked() {
                        clicked = Some(e.clone());
                    }
                }
            },
        );
    });
    if let Some(e) = clicked {
        b.select(&e);
    }
    if let Some(dir) = go {
        b.navigate(dir);
    }
    let message = match (&b.listing, &b.note) {
        (_, Note::NotFound(name)) => Some((crate::i18n::fmt(tl!("{name} wasn't found."), &[("name", name)]), t.red)),
        (_, Note::Replace(p)) => {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Some((crate::i18n::fmt(tl!("{name} already exists. Choose Save again to replace it."), &[("name", &name)]), t.red))
        }
        (Err(e), _) => Some((crate::i18n::fmt(tl!("This folder can't be read: {error}"), &[("error", e)]), t.red)),
        (Ok(l), _) if l.truncated => {
            Some((crate::i18n::fmt(tl!("Showing the first {count} items."), &[("count", &l.entries.len().to_string())]), t.text_dim))
        }
        _ => None,
    };
    if let Some((text, color)) = message {
        ui.label(egui::RichText::new(text).color(color));
    }
    ui.add_space(6.0);
    let mut refilter = false;
    ui.horizontal(|ui| {
        let label = ui.label(tl!("File name:"));
        // The name field takes what the label (longer in some languages) and the type list leave.
        let width = (ui.available_width() - 170.0).max(120.0);
        let r = ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(width)).labelled_by(label.id);
        if b.focus_name {
            r.request_focus();
            b.focus_name = false;
        }
        if r.changed() {
            b.selected = None;
            b.note = Note::None;
        }
        let before = b.filter;
        let current = b.filters.get(b.filter).map(|(l, _)| tl!(l.as_str()).to_string()).unwrap_or_default();
        egui::ComboBox::from_id_salt("builtin_file_browser_type").selected_text(current).width(150.0).show_ui(ui, |ui| {
            for (i, (l, _)) in b.filters.iter().enumerate() {
                ui.selectable_value(&mut b.filter, i, tl!(l.as_str()));
            }
        });
        refilter = b.filter != before;
    });
    if refilter {
        b.navigate(b.dir.clone());
    }
    ui.add_space(8.0);
    let mut cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let enter = !location_done && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel |= ui.button(tl!("Cancel")).clicked();
            let ok = ui.add(egui::Button::new(egui::RichText::new(tl!(b.ok_label())).color(egui::Color32::WHITE)).fill(APP_COLOR));
            pick |= ok.clicked() || enter;
        });
    });
    if cancel {
        return Some(None);
    }
    if pick && let Step::Pick(path) = b.accept() {
        return Some(Some(path));
    }
    None
}

/// A small button with a drawn icon and a tooltip; true when clicked.
fn icon_button(ui: &mut Ui, t: &Tokens, icon: &str, tip: &str, enabled: bool) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    let c = if enabled { t.icon } else { t.text_disabled };
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(18.0, 18.0)), icon, c, if enabled { t.accent } else { c });
    resp.on_hover_text(tip).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use serde_json::json;
    use std::sync::mpsc::channel;
    use wordcraft_engine::Session;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wordcraft-ui-browser-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn exts(e: &[&str]) -> Vec<String> {
        e.iter().map(|x| x.to_string()).collect()
    }

    /// Folders come first, then the files of the chosen type (any case), each by name; hidden
    /// entries and other types are left out, and a big folder is cut off and says so.
    #[test]
    fn listing_filters_sorts_and_caps() {
        let dir = scratch("list");
        for f in ["b.docx", "A.DOCX", "notes.txt", ".hidden.docx", "c.docx"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        std::fs::create_dir(dir.join("zeta")).unwrap();
        std::fs::create_dir(dir.join(".git")).unwrap();
        let l = list_dir(&dir, &exts(&["docx"]), 100).unwrap();
        let names: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["zeta", "A.DOCX", "b.docx", "c.docx"]);
        assert!(!l.truncated);
        assert_eq!(list_dir(&dir, &exts(&["*"]), 100).unwrap().entries.len(), 5);
        let capped = list_dir(&dir, &exts(&["docx"]), 2).unwrap();
        assert_eq!(capped.entries.len(), 2);
        assert!(capped.truncated);
        assert!(list_dir(&dir.join("missing"), &exts(&["*"]), 10).is_err(), "an unreadable folder is an error, not a panic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Save As in the browser: a typed folder is opened, a bare name gets the type's extension,
    /// and an existing file is replaced only on the second OK.
    #[test]
    fn save_adds_the_extension_and_asks_before_replacing() {
        let dir = scratch("save");
        std::fs::create_dir(dir.join("drafts")).unwrap();
        std::fs::write(dir.join("drafts").join("old.docx"), b"x").unwrap();
        let req = FileDialogRequest::Save { name: "Document1.docx".into() };
        let mut b = FileBrowser::new(req, AfterPick::OpenDocument, dir.clone(), false);
        b.name = "drafts".into();
        assert_eq!(b.accept(), Step::Stay);
        assert_eq!(b.dir, dir.join("drafts"));
        b.name = "chapter".into();
        assert_eq!(b.accept(), Step::Pick(dir.join("drafts").join("chapter.docx")));
        b.name = "old".into();
        assert_eq!(b.accept(), Step::Stay, "asks before replacing");
        assert_eq!(b.accept(), Step::Pick(dir.join("drafts").join("old.docx")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The host can't show a system dialog (#576): Save As carries on in the built-in browser,
    /// and its answer saves the document.
    #[test]
    fn no_system_dialog_falls_back_to_the_built_in_browser() {
        let dir = scratch("fallback");
        let services = Services {
            file_dialog: Some(Box::new(|_req| {
                let (tx, rx) = channel();
                tx.send(Err("no portal".to_string())).unwrap();
                rx
            })),
            ..Default::default()
        };
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), services);
        a.run("text.insert", json!({"text": "Saved without a portal"})).unwrap();
        a.save_as_dialog();
        assert!(a.file_dialog_open());
        let b = a.file_browser.open.as_mut().expect("the built-in browser is open");
        assert!(b.unavailable);
        b.navigate(dir.clone());
        b.name = "novel".into();
        let Step::Pick(path) = b.accept() else { panic!("picked") };
        a.close_file_browser(Some(path));
        assert!(!a.file_dialog_open());
        assert!(!a.session.dirty);
        let saved = wordcraft_engine::io::open_path(&dir.join("novel.docx")).unwrap();
        assert!(saved.plain_text(wordcraft_doc::StoryRef::Body).contains("Saved without a portal"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
