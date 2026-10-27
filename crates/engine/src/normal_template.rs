//! The Normal template (#4237, ADR 0001): the saved defaults every new blank document starts
//! from. It is a WordCraft-written `Normal.dotx` (in the settings folder on the desktop, under
//! its own `localStorage` key on the web), read with the same docx reader as any document.
//!
//! Set As Default re-reads the template, changes only what was set and writes it back whole
//! ([`NormalStore::save`] replaces it atomically), so two windows lose nothing but the same
//! property. A missing template means WordCraft's built-in formatting; a damaged or oversized
//! one too, with a status message, never a panic. Text language is never stored: Proofing
//! Language › Set As Default (`prefs.default_language`) owns it.
//!
//! Which parts a template carries is recorded in a custom document property
//! ([`PARTS_PROPERTY`]): a template that only saved page setup leaves the styles of a new
//! document built in, and one that never saved page setup leaves its paper to the region.

use std::sync::Arc;

use wordcraft_doc::{Document, StyleSheet};

/// Largest template read or written, in bytes. A template holds styles, lists, a theme and page
/// setup, not content: a few tens of kilobytes. Anything bigger is not ours.
pub const MAX_TEMPLATE_BYTES: usize = 8 << 20;

/// The template's file name in the settings folder.
pub const FILE_NAME: &str = "Normal.dotx";

/// Custom document property naming the parts the template carries, space-separated
/// (`styles`, `theme`, `page`).
pub const PARTS_PROPERTY: &str = "WordCraftNormalParts";

/// A part of the formatting the template can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The style sheet (document defaults and every style) and the lists the styles use.
    Styles,
    /// Theme fonts, theme colours and the theme's name.
    Theme,
    /// Page setup: paper, orientation, margins, header and footer distance, vertical alignment,
    /// different first page, different odd and even pages, mirrored margins.
    Page,
}

impl Part {
    fn word(self) -> &'static str {
        match self {
            Part::Styles => "styles",
            Part::Theme => "theme",
            Part::Page => "page",
        }
    }
}

/// Where the template is kept. The front end provides it ([`crate::Session::normal_store`]);
/// headless sessions (`wordcraft-cli mcp`, `wordcraft-cli run`, tests) have none, so they never
/// change the person's defaults. An agent driving the running app (`--control`, `mcp --connect`)
/// uses the app's store, like the saved proofing language.
pub trait NormalStore: Send + Sync {
    /// The template's bytes; `Ok(None)` when there is none. Bytes over [`MAX_TEMPLATE_BYTES`]
    /// may be returned as an error instead.
    fn load(&self) -> Result<Option<Vec<u8>>, String>;
    /// Replace the template with `bytes`, all at once.
    fn save(&self, bytes: &[u8]) -> Result<(), String>;
    /// Delete the template; deleting a missing one is not an error.
    fn reset(&self) -> Result<(), String>;
}

/// The template in memory: for tests and hosts with nowhere to keep it.
#[derive(Default)]
pub struct MemoryStore(std::sync::Mutex<Option<Vec<u8>>>);

impl MemoryStore {
    pub fn new() -> Arc<MemoryStore> {
        Arc::new(MemoryStore::default())
    }

    /// Put `bytes` in place, as if a template had been saved (a damaged one in tests).
    pub fn put(&self, bytes: Option<Vec<u8>>) {
        if let Ok(mut b) = self.0.lock() {
            *b = bytes;
        }
    }

    /// The bytes held.
    pub fn get(&self) -> Option<Vec<u8>> {
        self.0.lock().ok().and_then(|b| b.clone())
    }
}

impl NormalStore for MemoryStore {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.get())
    }

    fn save(&self, bytes: &[u8]) -> Result<(), String> {
        self.put(Some(bytes.to_vec()));
        Ok(())
    }

    fn reset(&self) -> Result<(), String> {
        self.put(None);
        Ok(())
    }
}

/// `Normal.dotx` in a folder (the desktop app's settings folder). Written through a sibling
/// temporary file and a rename, like a document save.
#[cfg(not(target_arch = "wasm32"))]
pub struct FileStore {
    pub path: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileStore {
    /// The template in `dir` ([`FILE_NAME`]).
    pub fn in_dir(dir: &std::path::Path) -> Arc<FileStore> {
        Arc::new(FileStore { path: dir.join(FILE_NAME) })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl NormalStore for FileStore {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        crate::io::read_app_file(&self.path, u64::try_from(MAX_TEMPLATE_BYTES).unwrap_or(u64::MAX))
    }

    fn save(&self, bytes: &[u8]) -> Result<(), String> {
        crate::io::write_app_file(&self.path, bytes)
    }

    fn reset(&self) -> Result<(), String> {
        crate::io::remove_app_file(&self.path)
    }
}

/// Read a template: at most [`MAX_TEMPLATE_BYTES`], an unencrypted Word document, read by the
/// docx reader with its own caps. Its text language is dropped.
pub fn parse(bytes: &[u8]) -> Result<Document, String> {
    if bytes.len() > MAX_TEMPLATE_BYTES {
        return Err(format!("it is larger than {} MB", MAX_TEMPLATE_BYTES >> 20));
    }
    if wordcraft_docx::is_encrypted(bytes) {
        return Err("it is encrypted".into());
    }
    let mut doc = wordcraft_docx::read(bytes).map_err(|e| e.to_string())?;
    strip_language(&mut doc.styles);
    Ok(doc)
}

/// The bytes of template `doc`, with no text language.
pub fn to_bytes(doc: &Document) -> Result<Vec<u8>, String> {
    let mut doc = doc.clone();
    strip_language(&mut doc.styles);
    let bytes = crate::io::save_bytes(FILE_NAME, &doc)?;
    if bytes.len() > MAX_TEMPLATE_BYTES {
        return Err(format!("the Normal template would be larger than {} MB", MAX_TEMPLATE_BYTES >> 20));
    }
    Ok(bytes)
}

/// A template with nothing saved in it yet: built-in formatting, no parts.
pub fn empty() -> Document {
    let mut d = Document::new();
    d.core.title = "Normal".into();
    d
}

/// Text language isn't part of the template (Proofing Language › Set As Default owns it).
fn strip_language(sheet: &mut StyleSheet) {
    let clear = |c: &mut wordcraft_doc::CharProps| {
        c.lang = None;
        c.lang_ea = None;
        c.lang_bidi = None;
    };
    clear(&mut sheet.default_chr);
    for st in &mut sheet.styles {
        clear(&mut st.chr);
    }
}

/// The parts template `t` carries.
pub fn parts(t: &Document) -> Vec<Part> {
    let words = t.custom_prop(PARTS_PROPERTY).unwrap_or("");
    [Part::Styles, Part::Theme, Part::Page].into_iter().filter(|p| words.split_whitespace().any(|w| w == p.word())).collect()
}

/// Record that template `t` carries `part`.
pub fn mark(t: &mut Document, part: Part) {
    let mut have = parts(t);
    if !have.contains(&part) {
        have.push(part);
    }
    let words: Vec<&str> = have.iter().map(|p| p.word()).collect();
    t.set_custom_prop(PARTS_PROPERTY, &words.join(" "));
}

/// Make blank document `d` start from template `t`: the parts it carries replace `d`'s.
pub fn apply_to_blank(d: &mut Document, t: &Document) {
    let parts = parts(t);
    if parts.contains(&Part::Styles) {
        d.styles = t.styles.clone();
        d.numbering = t.numbering.clone();
    }
    if parts.contains(&Part::Theme) {
        copy_theme(&mut d.settings, &t.settings);
    }
    if parts.contains(&Part::Page) {
        copy_page(d, t);
    }
}

/// Make a document read from plain text or Markdown take template `t`'s formatting: its
/// defaults, its styles over same-named ones (the import's own others stay), its theme and page
/// setup.
pub fn apply_to_import(d: &mut Document, t: &Document) {
    let parts = parts(t);
    if parts.contains(&Part::Styles) {
        crate::cmd::templates::apply_template_styles(d, t);
    }
    if parts.contains(&Part::Theme) {
        copy_theme(&mut d.settings, &t.settings);
    }
    if parts.contains(&Part::Page) {
        copy_page(d, t);
    }
}

/// Theme fonts, colours and name from `from`.
/// Drops lists no style numbers with, so saving the same styles again (which copies their lists
/// afresh) doesn't grow the template.
// ponytail: only styles are checked; the template's body is never used, so its paragraphs don't count.
pub(crate) fn drop_unused_lists(t: &mut Document) {
    let used: std::collections::HashSet<u32> = t.styles.styles.iter().filter_map(|st| st.para.numbering.map(|n| n.num)).collect();
    t.numbering.nums.retain(|n| used.contains(&n.id));
    let abstracts: std::collections::HashSet<u32> = t.numbering.nums.iter().map(|n| n.abstract_id).collect();
    t.numbering.abstracts.retain(|a| abstracts.contains(&a.id));
}

pub(crate) fn copy_theme(to: &mut wordcraft_doc::Settings, from: &wordcraft_doc::Settings) {
    to.major_font = from.major_font.clone();
    to.minor_font = from.minor_font.clone();
    to.theme_colors = from.theme_colors.clone();
    to.theme_name = from.theme_name.clone();
}

/// Page setup of `from`'s last section onto every section of `to` (a new document has one), and
/// the document-wide odd/even headers and mirrored margins. Columns, headers and footers'
/// content, numbering and the rest stay.
pub(crate) fn copy_page(to: &mut Document, from: &Document) {
    to.settings.even_odd_headers = from.settings.even_odd_headers;
    to.settings.mirror_margins = from.settings.mirror_margins;
    let src = &from.last_section;
    let set = |sp: &mut wordcraft_doc::SectionProps| page_setup_onto(sp, src);
    let ends: Vec<usize> = to.sections().iter().map(|(end, _)| *end).collect();
    for end in ends {
        set(to.section_mut(end));
    }
    set(&mut to.last_section);
}

/// The page setup fields Page Setup › Set As Default saves, from `src` onto `sp`.
pub(crate) fn page_setup_onto(sp: &mut wordcraft_doc::SectionProps, src: &wordcraft_doc::SectionProps) {
    sp.page_w = src.page_w;
    sp.page_h = src.page_h;
    sp.landscape = src.landscape;
    sp.margin_top = src.margin_top;
    sp.margin_bottom = src.margin_bottom;
    sp.margin_left = src.margin_left;
    sp.margin_right = src.margin_right;
    sp.header = src.header;
    sp.footer = src.footer;
    sp.gutter = src.gutter;
    sp.valign = src.valign;
    sp.title_page = src.title_page;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_round_trips_its_parts_without_language() {
        let mut t = empty();
        t.styles.default_chr.font = Some("Georgia".into());
        t.styles.default_chr.size = Some(13.0);
        t.styles.default_chr.lang = Some("fr-FR".into());
        t.last_section.margin_left = 100.0;
        mark(&mut t, Part::Styles);
        mark(&mut t, Part::Page);
        mark(&mut t, Part::Page);
        let back = parse(&to_bytes(&t).unwrap()).unwrap();
        assert_eq!(parts(&back), vec![Part::Styles, Part::Page]);
        assert_eq!(back.styles.default_chr.font.as_deref(), Some("Georgia"));
        assert_eq!(back.styles.default_chr.size, Some(13.0));
        assert_eq!(back.styles.default_chr.lang, None, "language is not stored");
        assert_eq!(back.last_section.margin_left, 100.0);
    }

    #[test]
    fn the_template_is_saved_as_a_word_template_not_a_document() {
        use std::io::Read;
        assert_eq!(FILE_NAME, "Normal.dotx");
        let bytes = to_bytes(&empty()).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut types = String::new();
        zip.by_name("[Content_Types].xml").unwrap().read_to_string(&mut types).unwrap();
        assert!(types.contains("wordprocessingml.template.main+xml"), "a .dotx package, which Word opens as a template");
        assert!(!types.contains("wordprocessingml.document.main+xml"));
    }

    #[test]
    fn damaged_or_oversized_templates_are_refused() {
        assert!(parse(b"not a docx at all").is_err());
        assert!(parse(&[]).is_err());
        let mut zip_head = b"PK\x03\x04".to_vec();
        zip_head.extend(std::iter::repeat_n(0u8, 300));
        assert!(parse(&zip_head).is_err());
        assert!(parse(&vec![0u8; MAX_TEMPLATE_BYTES + 1]).unwrap_err().contains("larger"));
    }

    #[test]
    fn a_blank_document_takes_only_the_parts_carried() {
        let mut t = empty();
        t.last_section.page_w = 999.0;
        t.styles.default_chr.font = Some("Georgia".into());
        mark(&mut t, Part::Styles);
        let mut d = Document::new();
        d.last_section.page_w = 595.0;
        apply_to_blank(&mut d, &t);
        assert_eq!(d.styles.default_chr.font.as_deref(), Some("Georgia"));
        assert_eq!(d.last_section.page_w, 595.0, "page setup wasn't saved: the region's paper stays");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_file_store_writes_reads_and_resets() {
        let dir = std::env::temp_dir().join(format!("wc-normal-{}-{:?}", std::process::id(), std::thread::current().id()));
        let store = FileStore::in_dir(&dir.join("nested"));
        assert_eq!(store.load().unwrap(), None);
        store.save(b"abc").unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some(&b"abc"[..]));
        store.reset().unwrap();
        store.reset().unwrap();
        assert_eq!(store.load().unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
