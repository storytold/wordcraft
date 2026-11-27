//! File › Print (#641): the print settings and a preview of the pages.
//!
//! Page range and parity shape the PDF. Native printer settings and copy collation go to
//! the shared spooler; browser printing and saved print PDFs retain copies in the PDF.

mod preview;

use std::sync::Arc;

use craft::theme::Variant;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, FieldLayout, Gap, LeastWidth, Props as _, Scroll, Show as _, ShowContainer as _, Sizing, WrapWidth, clicked};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::PreviewView;
use craft::ui::pickers::Select;
use craft::ui::primitives::HAlign;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::memory::Remember as _;
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Value, json};
use wordcraft_layout::DocLayout;

use crate::WordApp;

/// The Print page's settings. Not saved with the document or the UI state.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintSettings {
    /// `all`, `current`, `selection` or `custom`.
    pub range: String,
    /// The custom range text, like `1-3, 5`.
    pub pages: String,
    /// `all`, `odd` or `even`.
    pub parity: String,
    pub copies: u32,
    pub collate: bool,
    /// The physical sheet side shown in the planned print job (0-based).
    pub preview: usize,
    /// `auto`, `portrait` or `landscape`.
    pub orientation: String,
    /// Scale to paper size (#2108): `none` prints the document's page sizes, a Layout › Size
    /// name scales every page onto that paper. The document itself is unchanged.
    pub paper: String,
    /// Physical grid size or booklet binding, independent of document page setup.
    pub pages_per_sheet: String,
}

impl Default for PrintSettings {
    /// Start with one page per sheet and the document's paper size.
    fn default() -> Self {
        PrintSettings {
            range: "all".into(),
            pages: String::new(),
            parity: "all".into(),
            copies: 1,
            collate: true,
            preview: 0,
            orientation: "auto".into(),
            paper: NO_SCALING.into(),
            pages_per_sheet: "1".into(),
        }
    }
}

impl PrintSettings {
    /// The params of `ui.print` and `file.printPdf` for these settings.
    pub fn params(&self) -> Value {
        let mut v = json!({
            "range": self.range,
            "parity": self.parity,
            "copies": self.copies,
            "collate": self.collate,
            "orientation": self.orientation,
            "pagesPerSheet": self.pages_per_sheet.parse::<u32>().map_or_else(|_| json!(self.pages_per_sheet), |n| json!(n)),
        });
        if self.range == "custom" {
            v["pages"] = json!(self.pages);
        }
        if self.paper != NO_SCALING {
            v["paper"] = json!(self.paper);
        }
        v
    }

    /// The paper pages are scaled to, as (width, height) in points, or `None` for their own size.
    pub fn paper_size(&self) -> Option<(f32, f32)> {
        wordcraft_geom::paper_by_name(&self.paper).map(|(_, w, h)| (w, h))
    }
}

/// The Print page's state, per window: the settings, what was said of the last job, and the preview's view.
pub struct PrintPage {
    /// What gets printed.
    pub settings: PrintSettings,
    /// The generation (`WordApp::print_gen`) of the job result this window has already said, in
    /// its status bar or on this page: a result of the same text from a later job is another one.
    pub(crate) said: Option<u64>,
    /// The generation of the result held back on this opening of the page.
    pub(crate) hidden: Option<u64>,
    /// The frame whose first pass decided what this opening holds back: the page counts as appearing through every
    /// pass of that frame, and the later passes must not read the page's own "said" mark of the first one.
    opened_in: Option<u64>,
    /// The preview's zoom and pan; it follows the pane until the user zooms or pans.
    pub(crate) view: PreviewView,
    /// The output layout, kept while what it is made from ([`LayoutKey`]) is unchanged.
    layout: Option<(LayoutKey, Arc<DocLayout>)>,
}

impl PrintPage {
    /// The preview's zoom range: 10% to 500%, as the status bar's.
    const ZOOM: std::ops::RangeInclusive<f32> = 0.1..=5.0;

    /// A view that follows the preview pane.
    pub(crate) fn fit_view() -> PreviewView {
        PreviewView::fitting().zoom_range(Self::ZOOM)
    }
}

impl Default for PrintPage {
    fn default() -> Self {
        PrintPage { settings: PrintSettings::default(), said: None, hidden: None, opened_in: None, view: PrintPage::fit_view(), layout: None }
    }
}

pub(crate) const RANGES: [(&str, &str); 4] =
    [("all", "All pages"), ("current", "Current page"), ("selection", "Selected pages"), ("custom", "Custom range")];
pub(crate) const PARITIES: [(&str, &str); 3] = [("all", "All pages in the range"), ("odd", "Odd pages only"), ("even", "Even pages only")];
/// The Scale to paper size choice that keeps each page's own size.
pub(crate) const NO_SCALING: &str = "none";
pub(crate) const ORIENTATIONS: [(&str, &str); 3] = [("auto", "Auto orientation"), ("portrait", "Portrait"), ("landscape", "Landscape")];
/// The Pages per sheet choices: grids and the two booklet bindings.
pub(crate) const PAGES_PER_SHEET: [(&str, &str); 8] = [
    ("1", "1"),
    ("2", "2"),
    ("4", "4"),
    ("6", "6"),
    ("9", "9"),
    ("16", "16"),
    ("booklet", "Booklet (left binding)"),
    ("bookletReverse", "Booklet (right binding)"),
];

/// The settings that decide how tracked changes and comments look in the print preview:
/// Print Markup and Track Changes Options.
type MarkupKey = (bool, wordcraft_layout::display::MarkupOptions);

/// The [`MarkupKey`] now.
fn markup_key(app: &WordApp) -> MarkupKey {
    (app.session.prefs.print_markup, app.session.prefs.markup.clone())
}

/// What the preview's layout is made from besides the document's text: the document id, the
/// revision, Print Markup and Track Changes Options, the file name and path FILENAME fields show
/// (#6282: Save As and Rename change them without an edit), and the font database generation the
/// app has seen (#6283: `WordApp::watch_fonts` moves it when fonts are added or removed).
type LayoutKey = (u64, u64, MarkupKey, (String, String), u64);

/// The [`LayoutKey`] now.
fn layout_key(app: &WordApp) -> LayoutKey {
    (app.session.document_id(), app.session.rev(), markup_key(app), app.session.resolved_filenames(), app.font_generation)
}

/// The output layout, kept in `cache` while what it is made from ([`LayoutKey`]) is unchanged.
// lean: keyed on the app's own inputs until the engine has an export/layout generation (#6282, #6283).
fn layout_of(app: &WordApp, cache: &mut Option<(LayoutKey, Arc<DocLayout>)>) -> Arc<DocLayout> {
    let key = layout_key(app);
    if let Some((k, l)) = cache
        && *k == key
    {
        return l.clone();
    }
    let l = app.session.print_layout(app.session.prefs.print_markup);
    *cache = Some((key, l.clone()));
    l
}

/// How the preview draws a page: tracked changes marked as the print job marks them, headers and
/// footers at full strength (the preview is the printed page, not the editing view).
pub(super) fn preview_options(app: &WordApp) -> wordcraft_render::RenderOptions {
    let mut opts = wordcraft_render::RenderOptions::default();
    opts.display.markup = app.session.prefs.print_markup;
    opts.display.revisions = app.session.prefs.markup.clone();
    opts.display.dim_header = false;
    opts
}

/// One choice list over `(id, label)` pairs, named `label`; true when the choice changed.
fn choice(ui: &mut Ui, label: &str, salt: &str, current: &mut String, items: &[(&str, &str)]) -> bool {
    let mut changed = false;
    // The name above its list, as production's settings column.
    Field::new(label.to_owned()).id_salt(salt).layout(FieldLayout::Above).show(ui, |ui, _| {
        changed = unlabelled_choice(ui, label, salt, current, items);
    });
    changed
}

/// The list of `choice` with no name above it, `label` only its accessible name: production's second Pages list
/// (Pages in the range) sits under the first, which the one "Pages" heads.
fn unlabelled_choice(ui: &mut Ui, label: &str, salt: &str, current: &mut String, items: &[(&str, &str)]) -> bool {
    let entries: Vec<Entry<String>> = items.iter().map(|(id, name)| Entry::new((*id).to_owned(), tl!(name).to_owned())).collect();
    let picked = Select::new(label.to_owned(), Bind::given(current.clone()), &entries).id_salt(salt).show(ui);
    match picked.value() {
        Some(value) if value != current => {
            *current = value.clone();
            true
        }
        _ => false,
    }
}

/// The Print page: settings on the left, the preview on the right.
pub fn show(app: &mut WordApp, ui: &mut Ui) {
    let mut cache = app.print.layout.take();
    let lay = layout_of(app, &mut cache);
    app.print.layout = cache;
    let total = lay.pages.len();
    app.print.settings.preview = app.print.settings.preview.min(total.saturating_sub(1));
    let chooses = app.services.print.as_ref().is_some_and(|spool| spool.chooses_printer());
    let can_print = app.services.print.is_some();
    Stack::new().fill().show(ui, |ui, cx| {
        // Opened afresh, the page doesn't repeat how the last job went when this window already said
        // it (its status bar, or this page): `printing.message` itself stays, as `ui.printSettings`
        // and other windows still report it. What the printers are doing (finding them, none found)
        // always shows.
        // `appeared` holds through every pass of the opening frame: the first pass decides.
        let frame = ui.ctx().cumulative_frame_nr();
        let opened = cx.appeared() && app.print.opened_in != Some(frame);
        if opened {
            app.print.opened_in = Some(frame);
        }
        let result = |m: &Option<crate::printing::Feedback>| {
            matches!(
                m,
                Some(crate::printing::Feedback::Submitted(_) | crate::printing::Feedback::Failed(_) | crate::printing::Feedback::WorkerStopped)
            )
        };
        let message = app.printing.message.clone();
        let generation = app.print_gen;
        if opened {
            app.print.hidden = (result(&message) && app.print.said == Some(generation)).then_some(generation);
        }
        // A later message (a job started again, then failed the same way) is shown afresh.
        if app.print.hidden != Some(generation) {
            app.print.hidden = None;
        }
        let hide_message = app.print.hidden.is_some();
        if !hide_message {
            // Shown here, so the next opening needn't repeat it.
            app.print.said = result(&message).then_some(generation);
        }
        let ready = !chooses || app.printing.ready();
        // The row takes the page, so the preview beside the settings column fills the room left (production: the
        // preview takes the page's height beside a 380 pt column).
        Row::new().fill().gap(Gap::Section).show(ui, |ui, _| {
            // Production's settings column is 380 pt (a dialog's least content width), with no frame of its own on the
            // page, scrolling down the page's height; its paragraphs wrap inside it: the column's wrap width keeps the
            // long notes from widening it over the preview.
            let _ = Stack::new()
                .sizing(Sizing::FillHeight)
                .least_width(LeastWidth::Dialog)
                .wrap_width(WrapWidth::Column)
                .scroll(Scroll::vertical())
                .label(Label::from(tl!("Print").to_owned()))
                .gap(Gap::Stack)
                .show(ui, |ui, _| settings(app, ui, &lay, hide_message, chooses, can_print, ready));
            preview::show(app, ui, &lay);
        });
    });
}

/// Enter on the Print page: prints with the page's settings when a job can start. The File page's key scope
/// routes the key here while nothing has focus; a focused control takes Enter itself.
pub(crate) fn print_requested(app: &mut WordApp, _ctx: &egui::Context) {
    let chooses = app.services.print.as_ref().is_some_and(|spool| spool.chooses_printer());
    let can_print = app.services.print.is_some();
    if can_print && (!chooses || app.printing.ready()) {
        let params = app.print.settings.params();
        let _ = app.run("ui.print", params);
    }
}

/// The settings column: the page's action first, the printer controls, the choices and the export formats.
fn settings(app: &mut WordApp, ui: &mut Ui, lay: &Arc<DocLayout>, hide_message: bool, chooses: bool, can_print: bool, ready: bool) {
    let total = lay.pages.len();
    let current = app.session.current_pages(lay);
    // #2958: as in Word, the page's action leads the settings, in the accent of a dialog's primary button.
    if can_print && clicked(&Button::new(tl!("Print")).variant(Variant::PRIMARY).enabled(ready).id_salt("print").show(ui)) {
        let params = app.print.settings.params();
        let _ = app.run("ui.print", params);
    }
    if chooses {
        if let Some(discover) = app.services.print.as_ref().map(|spool| spool.discoverer()) {
            app.printing.run(ui, &discover);
        }
        if let Some(message) = app.printing.message.as_ref().filter(|_| !hide_message) {
            Text::new(crate::printing::feedback(message, |label| crate::i18n::t(label).to_owned())).show(ui);
        }
        // A job can wait long on a sleeping printer's driver; Cancel frees the page.
        if app.printing.busy() && clicked(&Button::new(tl!("Cancel printing")).id_salt("cancel").show(ui)) {
            let _ = app.run("ui.printCancel", json!({}));
        }
        app.print.settings.copies = app.printing.settings.copies;
    } else if can_print {
        Text::new(tl!("Send the document straight to the system print dialog, or save a copy in another format below.")).show(ui);
    } else {
        Text::new(tl!("Save a copy in another format. Printing goes through a PDF you can print from any viewer.")).show(ui);
    }
    if clicked(&Button::new(tl!("Save these pages as PDF…")).id_salt("save-pdf").sizing(Sizing::FillWidth).show(ui)) {
        app.print_pdf_dialog();
    }

    let print = &mut app.print;
    if chooses {
        if print.settings.copies > 1 {
            collate(ui, &mut print.settings.collate, !app.printing.busy());
        }
    } else {
        Field::new(tl!("Copies")).id_salt("copies").layout(FieldLayout::Above).show(ui, |ui, _| {
            let field = NumberField::new(tl!("Copies"), Bind::given(Some(f64::from(print.settings.copies))))
                .step(1.0)
                .range(1.0..=f64::from(wordcraft_engine::print::MAX_COPIES))
                .commit_decimals(0);
            if let Some(n) = field.show(ui).value().copied().flatten() {
                print.settings.copies = (n.round() as u32).clamp(1, wordcraft_engine::print::MAX_COPIES);
            }
        });
        if print.settings.copies > 1 {
            collate(ui, &mut print.settings.collate, true);
        }
    }

    let mut restart = choice(ui, tl!("Pages"), "print_range", &mut print.settings.range, &RANGES);
    if print.settings.range == "custom" {
        let edited = TextField::new(tl!("Pages"), Bind::given(print.settings.pages.clone()))
            .placeholder(tl!("Pages like 1-3, 5"))
            .limit(wordcraft_engine::print::MAX_SPEC_CHARS)
            .show(ui);
        if let Some(text) = edited.value() {
            print.settings.pages.clone_from(text);
            restart = true;
        }
    }
    restart |= unlabelled_choice(ui, tl!("Pages in the range"), "print_parity", &mut print.settings.parity, &PARITIES);
    if restart {
        // Show the first page that will print.
        print.settings.preview = 0;
    }
    let sheet = &mut print.settings;
    if sheet.pages_per_sheet == "2" || sheet.pages_per_sheet.starts_with("booklet") {
        Field::new(tl!("Orientation")).id_salt("print_orientation").layout(FieldLayout::Above).show(ui, |ui, _| {
            Text::new(tl!("Landscape")).show(ui);
        });
    } else {
        choice(ui, tl!("Orientation"), "print_orientation", &mut sheet.orientation, &ORIENTATIONS);
    }
    choice(ui, tl!("Pages per sheet"), "print_pages_per_sheet", &mut sheet.pages_per_sheet, &PAGES_PER_SHEET);
    if sheet.pages_per_sheet.starts_with("booklet") {
        Text::new(tl!("Booklets: duplex on the short edge. Blank pages are added at the end.")).show(ui);
    }
    // Word: Pages per sheet › Scale to paper size. Only the printout changes.
    let papers: Vec<(&str, &str)> =
        std::iter::once((NO_SCALING, "No Scaling")).chain(wordcraft_geom::PAPER_SIZES.iter().map(|(n, _, _)| (*n, *n))).collect();
    choice(ui, tl!("Scale to paper size"), "print_paper", &mut sheet.paper, &papers);

    match wordcraft_engine::print::plan(&app.print.settings.params(), total, current) {
        Ok(plan) => {
            let msg =
                crate::i18n::fmt(tl!("{count} of {total} pages will print"), &[("count", &plan.chosen.to_string()), ("total", &total.to_string())]);
            // Production's count is dim, a note under the choices rather than one of them.
            Text::new(msg).shade(Shade::Muted).show(ui);
        }
        Err(e) => {
            Text::new(e.to_string()).tone(Tone::Bad).show(ui);
        }
    }
    // A rule sets the export formats apart from the print settings, as production's.
    Divider::horizontal().show(ui);
    if can_print {
        Text::new(tl!("Or save a copy in another format:")).show(ui);
    }
    export_formats(app, ui);
}

/// The Collate copies check box.
fn collate(ui: &mut Ui, on: &mut bool, enabled: bool) {
    let checked = Checkbox::new(Bind::given(*on)).label(tl!("Collate copies")).enabled(enabled).id_salt("collate").show(ui);
    if let Some(value) = checked.value().copied() {
        *on = value;
    }
}

/// The export "format" of a Word 2003 XML Document: an `.xml` file saved with `format: "wordml2003"`.
pub(crate) const EXPORT_WORD_2003: &str = "wordml2003";

/// The "save a copy" formats as (label, format), also the built-in browser's types when exporting.
pub(crate) const EXPORT_FORMATS: &[(&str, &str)] = &[
    ("PDF document (*.pdf)", "pdf"),
    ("Word document (*.docx)", "docx"),
    ("Word XML Document (*.xml)", "xml"),
    ("Word 2003 XML Document (*.xml)", EXPORT_WORD_2003),
    ("OpenDocument Text (*.odt)", "odt"),
    ("OpenDocument Text Template (*.ott)", "ott"),
    ("Flat OpenDocument Text (*.fodt)", "fodt"),
    ("Rich Text Format (*.rtf)", "rtf"),
    ("Web page (*.html)", "html"),
    ("Markdown (*.md)", "md"),
    ("LaTeX (*.tex)", "tex"),
    ("Plain text (*.txt)", "txt"),
    ("Page image (*.png)", "png"),
];

/// The file extension an export format writes.
pub(crate) fn export_file_ext(format: &str) -> &str {
    if format == EXPORT_WORD_2003 { "xml" } else { format }
}

/// Is `ext` an extension of a document or export type (the Export types, plus the older `.doc`
/// and `.htm`)? Such an extension on a typed name gives way to the chosen type's; any other
/// (`notes.v2`) is part of the name.
pub(crate) fn is_document_ext(ext: &str) -> bool {
    ["doc", "htm"].iter().any(|e| e.eq_ignore_ascii_case(ext)) || EXPORT_FORMATS.iter().any(|&(_, f)| export_file_ext(f).eq_ignore_ascii_case(ext))
}

/// The name an export to `format` is saved under: the chosen type's extension replaces a known
/// document extension (`notes.docx` as plain text is `notes.txt`) and is added to any other name.
pub(crate) fn export_name(name: &str, format: &str) -> String {
    let want = export_file_ext(format);
    let path = std::path::Path::new(name);
    // A dotfile name (`.docx`) has no extension for `Path::extension`, which the engine's save also
    // reads, so it is a stem and gets one (`.docx.xml`); a bare extension would be refused.
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case(want) => name.to_string(),
        Some(e) if is_document_ext(e) => path.with_extension(want).to_string_lossy().into_owned(),
        // `a.` has an empty extension: its dot is dropped before the type's is added.
        _ => format!("{}.{want}", name.trim_end_matches('.')),
    }
}

/// The "save a copy" formats (File › Export, and below the settings on the Print page).
pub fn export_formats(app: &mut WordApp, ui: &mut Ui) {
    // A refused or failed export, said here because Backstage hides the status bar (Export and Print pages both draw this).
    if !app.export_error.is_empty() {
        Text::new(app.export_error.clone()).show(ui);
    }
    // Each format is a button across the column, its name at the reading start, as production's 320 pt rows.
    for &(label, ext) in EXPORT_FORMATS {
        if clicked(&Button::new(tl!(label)).id_salt(ext).sizing(Sizing::FillWidth).align(HAlign::Start).show(ui)) {
            app.export_dialog(ext);
        }
    }
    // Linux and the BSDs (#3535): WordCraft's own browser when the system dialog doesn't appear.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    {
        if clicked(&Button::new(tl!("Browse in WordCraft…")).id_salt("browse").sizing(Sizing::FillWidth).align(HAlign::Start).show(ui)) {
            let _ = app.run("ui.fileBrowser", json!({"purpose": "export"}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use wordcraft_engine::Session;

    /// Print Markup changes layout and pixels without changing the document revision.
    #[test]
    fn preview_refreshes_layout_and_texture_when_print_markup_toggles() {
        let mut session = Session::new(wordcraft_doc::Document::from_text("Synthetic print preview"));
        session.run("select.text", &json!({"text": "preview"})).unwrap();
        session.run("review.newComment", &json!({"text": "Synthetic printed comment"})).unwrap();
        session.prefs.markup.comments = true;
        let mut app = WordApp::new(session, Services::default());
        let mut cache = None;
        let rev = app.session.rev();
        let dirty = app.session.dirty;
        let mut previous = None;
        for markup in [false, true, false] {
            app.session.run("file.printMarkup", &json!({"value": markup})).unwrap();
            let layout = layout_of(&app, &mut cache);
            assert_eq!(layout.pages[0].markup > 0.0, markup);
            assert!(Arc::ptr_eq(&layout, &layout_of(&app, &mut cache)), "unchanged layout is reused");
            let revision = preview::inputs(&app);
            assert_eq!(revision, preview::inputs(&app));
            if let Some((old_layout, old_revision)) = previous.take() {
                assert!(!Arc::ptr_eq(&layout, &old_layout));
                assert_ne!(revision, old_revision, "the sheet is drawn again");
            }
            previous = Some((layout, revision));
            assert_eq!(app.session.rev(), rev);
            assert_eq!(app.session.dirty, dirty);
        }
    }

    /// A one-page synthetic document in a fresh preview.
    fn synthetic_app() -> (WordApp, egui::Context) {
        let session = Session::new(wordcraft_doc::Document::from_text("Synthetic print preview"));
        (WordApp::new(session, Services::default()), egui::Context::default())
    }

    /// Rename (and Save As) change what a FILENAME field shows without an edit (#6282): the
    /// preview lays the page out and draws it again.
    #[test]
    fn preview_follows_the_name_a_filename_field_shows() {
        let mut doc = wordcraft_doc::Document::new();
        let mut p = wordcraft_doc::Paragraph::new();
        let field = wordcraft_doc::InlineObject::Field {
            instr: "FILENAME".into(),
            result: String::new(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        };
        p.insert_object(0, field, &Default::default()).unwrap();
        doc.body = vec![wordcraft_doc::para_block(p)];
        let mut app = WordApp::new(Session::new(doc), Services::default());
        let mut cache = None;
        let shown = |l: &DocLayout| -> String {
            l.pages[0]
                .items
                .iter()
                .filter_map(|it| {
                    if let wordcraft_layout::Placed::Lines { para, .. } = it {
                        Some(para.shown.iter().map(|s| s.1.as_str()).collect::<String>())
                    } else {
                        None
                    }
                })
                .collect()
        };
        let before = layout_of(&app, &mut cache);
        assert_eq!(shown(&before), app.session.resolved_filenames().0, "the untitled document's name");
        let revision = preview::inputs(&app);
        let rev = app.session.rev();
        app.session.run("file.rename", &json!({"name": "Quarterly report"})).unwrap();
        assert_eq!(app.session.rev(), rev, "renaming is no edit");
        let after = layout_of(&app, &mut cache);
        assert_eq!(shown(&after), "Quarterly report");
        assert_ne!(preview::inputs(&app), revision);
    }

    /// Fonts added or removed change the layout without an edit (#6283): once the app has seen the
    /// font database move, the preview lays the page out and draws it again.
    #[test]
    fn preview_follows_the_fonts() {
        let (mut app, _) = synthetic_app();
        let mut cache = None;
        let before = layout_of(&app, &mut cache);
        let revision = preview::inputs(&app);
        // What `watch_fonts` records when a font is added: not the shared database itself, which
        // tests running alongside change.
        app.font_generation = app.font_generation.wrapping_add(1);
        let after = layout_of(&app, &mut cache);
        assert!(!Arc::ptr_eq(&before, &after), "laid out again with the new fonts");
        assert!(Arc::ptr_eq(&after, &layout_of(&app, &mut cache)), "and kept while the fonts stay");
        assert_ne!(preview::inputs(&app), revision);
    }

    /// The Print page's preview shown in a harness at `ppp` pixels per point over the synthetic page.
    fn harness(ppp: f32) -> egui_kittest::Harness<'static, WordApp> {
        let (app, _) = synthetic_app();
        crate::test_harness::builder()
            .with_size(egui::vec2(1000.0, 800.0))
            .with_pixels_per_point(ppp)
            .build_ui_state(|ui, app: &mut WordApp| show(app, ui), app)
    }

    /// The texture bytes the preview holds in the store of the context's current texture limit.
    fn stored_bytes(ctx: &egui::Context) -> usize {
        preview::thumbnails(ctx).cache().statistics(ctx).bytes
    }

    /// The GPU's texture limit changes with the GPU, as when the window moves to a display on
    /// another one (#6284): after the limit drops, the sheet is drawn again within it.
    #[test]
    fn preview_texture_follows_the_gpu_texture_limit() {
        const LIMIT: usize = 1024;
        let mut h = harness(4.0);
        h.input_mut().max_texture_side = Some(4096);
        h.run_steps(6);
        let roomy = stored_bytes(&h.ctx);
        assert!(roomy > LIMIT * LIMIT * 4, "the pane's sheet outgrows the lower limit: {roomy}");
        h.input_mut().max_texture_side = Some(LIMIT);
        h.run_steps(6);
        let limited = stored_bytes(&h.ctx);
        assert!(limited > 0 && limited <= LIMIT * LIMIT * 4, "{limited}");
    }

    /// The sheet follows the pane's shown size times the pixels per point: a denser display holds a
    /// bigger picture of the same pane.
    #[test]
    fn preview_texture_follows_the_exact_width_and_pixels_per_point() {
        let at = |ppp: f32| {
            let mut h = harness(ppp);
            h.run_steps(6);
            stored_bytes(&h.ctx)
        };
        let (sparse, dense) = (at(1.0), at(2.0));
        assert!(sparse > 0 && dense > sparse * 2, "{dense} {sparse}");
    }

    /// The preview pages as the print job does (#5269 review): without Print Markup tracked
    /// deletions take no space, whatever Track Changes Options mark them with.
    #[test]
    fn the_preview_pages_as_the_print_job() {
        let text: Vec<String> = (0..200).map(|i| format!("Line {i} of the old text")).collect();
        let doc = wordcraft_doc::Document::from_text(&text.join("\n"));
        let mut app = crate::WordApp::new(wordcraft_engine::Session::new(doc), Default::default());
        let mut cache = None;
        app.run("review.trackChanges", json!({"value": true})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("text.insert", json!({"text": "Kept"})).unwrap();
        // The preview is the printed page: headers and footers draw at full strength (#6267).
        assert!(!preview_options(&app).display.dim_header);
        let mut pages = |app: &crate::WordApp| (layout_of(app, &mut cache).pages.len(), app.session.print_pdf(&json!({})).unwrap().1.chosen);
        app.run("file.printMarkup", json!({"value": true})).unwrap();
        let (marked, job) = pages(&app);
        assert!(marked > 1 && marked == job, "{marked} vs {job}");
        app.run("file.printMarkup", json!({"value": false})).unwrap();
        assert_eq!(pages(&app), (1, 1));
        // The cached layout follows the Delete mark option too (it changes the layout with markup).
        app.run("file.printMarkup", json!({"value": true})).unwrap();
        app.run("review.trackingOptions", json!({"deleteMark": "hidden"})).unwrap();
        assert_eq!(pages(&app), (1, 1));
        // Changing revision colours refreshes the preview revision even on the same page count.
        let rev_before = preview::inputs(&app);
        app.run("review.trackingOptions", json!({"insertColor": "0000FF", "deleteColor": "FF0000"})).unwrap();
        let rev_after = preview::inputs(&app);
        assert_ne!(rev_before, rev_after, "tracking colours refresh preview revision");
    }

    /// Two documents at the same revision drawn in one window's context (#5270 review): each
    /// preview gets its own layout, not the other's cached one.
    #[test]
    fn the_preview_cache_tells_documents_apart() {
        let long: Vec<String> = (0..200).map(|i| format!("Line {i}")).collect();
        let app = |text: &str| crate::WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::from_text(text)), Default::default());
        let (long, short) = (app(&long.join("\n")), app("One line"));
        assert_eq!(long.session.rev(), short.session.rev(), "the test needs equal revisions");
        let mut cache = None;
        assert!(layout_of(&long, &mut cache).pages.len() > 1);
        assert_eq!(layout_of(&short, &mut cache).pages.len(), 1);
    }
}
