/// Platform services injected by the host. Files, the clipboard, animations and preferences
/// reach the app through the framework's host, not through here.
#[derive(Default)]
pub struct Services {
    /// Printing: the print job layer over the host's jobs (desktop printers, the browser's print
    /// dialog on the web).
    pub print: Option<craft::print::spool::Spool>,
    /// Web: told whether the document has unsaved changes after each pass, for the browser's
    /// leave-page guard (`beforeunload` runs between frames and can't ask the app).
    pub on_dirty: Option<Box<dyn Fn(bool)>>,
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
    #[cfg(not(target_arch = "wasm32"))]
    pub make_default_app: Option<Box<dyn Fn() -> Result<craft::platform::default_app::DefaultApp, craft::platform::default_app::DefaultAppError>>>,
    /// Desktop: the app's own user templates folder (`Templates` in the settings folder, #2852),
    /// used while File › Options names none. File › New lists personal templates from it and Save
    /// as Word Template starts in it. `None` (web) hides personal templates.
    pub templates_dir: Option<std::path::PathBuf>,
    /// Files the system asked the app to open, and files dropped on a window, read once per frame.
    pub intake: Option<craft::platform::open::FileIntake>,
}
