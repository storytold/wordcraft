use super::state::DefaultApp;
use super::{FileDialogAnswer, FileDialogRequest, async_saves, paste_picture, printing};
use crate::secrets;

/// Platform services injected by the host (file dialogs, file I/O).
#[derive(Default)]
pub struct Services {
    /// Desktop: discover configured printers on a worker.
    pub printers: Option<printing::Discover>,
    /// Desktop: submit the PDF with the chosen settings on a worker.
    pub print_native: Option<printing::Submit>,
    /// Pick a file to open; the argument says what for (`document`, `picture`).
    pub pick_open: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// Pick a path to save to, given a suggested name.
    pub pick_save: Option<Box<dyn Fn(&str) -> Option<String>>>,
    /// Desktop: show a native file dialog without blocking the UI thread (#94); used instead of
    /// `pick_open` / `pick_save` when set. The host sends the picked path (`Ok(None)` when
    /// cancelled) through the returned channel once the user answers, and wakes the UI; a dropped
    /// sender counts as cancelled. An `Err` says no system dialog could be shown, and the app shows
    /// its built-in file browser instead (#576). The app shows one dialog at a time ([`file_dialogs`]).
    pub file_dialog: Option<Box<dyn Fn(FileDialogRequest) -> std::sync::mpsc::Receiver<FileDialogAnswer>>>,
    /// Web: open a file picker; the file arrives later through `inbox`.
    pub open_async: Option<Box<dyn Fn(&str)>>,
    /// Web: files (name, bytes) delivered asynchronously (picker, drag and drop).
    pub inbox: Option<Inbox>,
    /// Web: pictures pasted from the browser's clipboard (the bytes; the name is ignored). Kept
    /// apart from `inbox` because a paste only lands in the document when the page is the edit
    /// target (#1349), where a dropped or opened file always lands.
    pub pasted: Option<Inbox>,
    /// Web: hand bytes to the browser as a download. An error means no download started (the
    /// browser can't tell the page whether the user then kept the file).
    pub download: Option<Box<dyn Fn(&str, &[u8]) -> Result<(), String>>>,
    /// Web: saves that `download` started and that finish later (the browser's save picker,
    /// #1315). The hook starts a ticket for such a save and reports how it ended; until then the
    /// document's save isn't confirmed, and a New/Open held back by the Save Changes prompt waits.
    pub async_saves: Option<async_saves::AsyncSaves>,
    /// Web, framed by a page that keeps the files (`?host=parent`): set alongside `download`,
    /// Save, Save As and Export hand the bytes to that page instead, with whether it was Save As
    /// ([`HostSaveFn`]: true for a `file.saveAs` in a format that keeps everything, which File ›
    /// Export to such a format runs too, and for a Save under a name the page never sent; false
    /// for Save and for copies: the PDF and PNG exports and Save As in a format that doesn't keep
    /// everything). Save then writes the opened file
    /// under its own name and format, as on the desktop, and a Save or Save As that keeps
    /// everything makes that name the document's file, so the next Save goes to it.
    pub host_save: Option<HostSaveFn>,
    /// Web: told whether the document has unsaved changes after each pass, for the browser's
    /// leave-page guard (`beforeunload` runs between frames and can't ask the app).
    pub on_dirty: Option<Box<dyn Fn(bool)>>,
    /// Web: hand PDF bytes to the browser's print dialog (no download or intermediate file).
    /// Desktop uses `print_native` instead. An error means nothing was opened.
    pub print: Option<Box<dyn Fn(&[u8]) -> Result<(), String>>>,
    /// Desktop: the picture on the system clipboard, if any — egui's paste only carries text
    /// (#45). Read when Paste finds no text; see [`paste_picture`].
    pub clipboard_picture: Option<Box<dyn Fn() -> Option<paste_picture::ClipboardPicture>>>,
    /// Whether the system has animation effects on (Windows: Animation effects; macOS: Reduce
    /// motion; Linux and the BSDs: the desktop's setting; browsers: `prefers-reduced-motion`), or
    /// `None` when it doesn't say. What "Use system setting" follows ([`theme::Animations`]).
    /// Asked every frame, so a change in the system's settings shows at once: the host answers
    /// from memory and does any slow asking elsewhere. Without the hook, animation effects stay on.
    pub animations_enabled: Option<Box<dyn Fn() -> Option<bool>>>,
    /// Desktop: the text on the system clipboard, if any. Ctrl+V gets it from egui's paste event;
    /// the ribbon and context-menu Paste read it here (#584).
    pub clipboard_text: Option<Box<dyn Fn() -> Option<String>>>,
    /// Desktop: the HTML on the system clipboard — another program's formatted copy (a Word
    /// document, a web page) — if any. Paste keeps its formatting (#687); see [`system_clipboard`].
    pub clipboard_html: Option<Box<dyn Fn() -> Option<String>>>,
    /// Desktop: put HTML on the system clipboard with its plain-text alternative (html, text, and
    /// for a copy of one equation alone its MathML, else empty), so other programs keep the
    /// formatting (#1523) and equations (#816). The MathML goes in the platform's MathML formats
    /// as well, which Word reads a pasted equation from (#1297). `true` when the HTML and text are
    /// there; otherwise the copy's text goes through egui as usual.
    pub set_clipboard_html: Option<Box<dyn Fn(&str, &str, &str) -> bool>>,
    /// Desktop: the RTF on the system clipboard, where the platform offers it (#687).
    pub clipboard_rtf: Option<Box<dyn Fn() -> Option<String>>>,
    /// Desktop: the names of the formats on the system clipboard (MIME types, Windows format
    /// names), where the platform lists them, so a paste that finds nothing it can use says what
    /// is there instead of "empty" (#833).
    pub clipboard_formats: Option<Box<dyn Fn() -> Vec<String>>>,
    /// Web: keep a font the user added (stored file name, original bytes) for the next visit
    /// (#582). Desktop fonts are stored by the font database itself, in the app's font folder.
    pub store_font: Option<Box<dyn Fn(&str, &[u8])>>,
    /// Web: forget a stored font (its stored file name) once the user removes it.
    pub forget_font: Option<Box<dyn Fn(&str)>>,
    /// Web: request browser to query and load local installed fonts (`window.queryLocalFonts()`).
    pub load_system_fonts: Option<Box<dyn Fn()>>,
    /// Desktop: File › Options › General › Make WordCraft the default for Word documents (#295),
    /// only from that button (never a programmatic command). Windows opens the Settings app's
    /// Default apps page (apps can't set defaults themselves); Linux and BSD set it with
    /// `xdg-mime`. `None` (web, macOS) hides the button; macOS shows how to do it in Finder
    /// instead. An error means nothing changed.
    pub make_default_app: Option<Box<dyn Fn() -> Result<DefaultApp, String>>>,
    /// Desktop: the system's credential store, where the LanguageTool API key is kept instead of
    /// `ui.json` (#764). `None` (web, no store) keeps it in `ui.json`.
    pub secrets: Option<Box<dyn secrets::SecretStore>>,
    /// Desktop: the app's own user templates folder (`Templates` in the settings folder, #2852),
    /// used while File › Options names none. File › New lists personal templates from it and Save
    /// as Word Template starts in it. `None` (web) hides personal templates.
    pub templates_dir: Option<std::path::PathBuf>,
}

/// Files delivered asynchronously.
pub type Inbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// [`Services::host_save`]: `(file name, bytes, save_as)`. An error means the page didn't get
/// the file, and the save failed.
pub type HostSaveFn = Box<dyn Fn(&str, &[u8], bool) -> Result<(), String>>;
