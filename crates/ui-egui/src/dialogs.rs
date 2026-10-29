//! Dialogs: Font, Paragraph, Find & Replace, Go To, Insert Table, Page Setup, Link, Bookmark,
//! Word Count, Zoom, Watermark, New/Modify Style, Manage Styles, New/Modify Table Style, Table
//! Properties, Enclose Characters, Command search, Icons, Paste Special, About, Save Changes, the password to open a
//! document and Encrypt with Password, and the mail-merge Recipient List, Insert Merge Field, Find
//! Recipient, merge rules, Match Fields, Check for Errors and Select Table (a workbook's sheet) (Customize Keyboard is in
//! `dialogs_keyboard`, Customize Ribbon in `dialogs_ribbon`). Every dialog ends by running a
//! command (or shows one's result), so agents get the same result without the dialog.

use std::collections::BTreeSet;

use egui::{Sense, Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};

use wordcraft_engine::Password;
use wordcraft_engine::cmd::mailings::{FilterOp, FilterRule, SortKey};

use crate::WordApp;
use crate::options::MeasureExt;
use crate::theme::{Tokens, semibold};

mod bookmarks;
mod chrome;
mod dispatch;
mod find;
mod font;
mod forms;
mod mailings;
mod page_setup;
mod paragraph;
mod styles;

use bookmarks::bookmark_at_caret;
#[cfg(test)]
use bookmarks::bookmark_names;
pub use chrome::show;
pub(crate) use chrome::{buttons, cancel_key_pressed, color_menu, ok_cancel_responses};
pub use find::{FindExtra, FindTab, GoToWhat};
#[cfg(test)]
use find::{command_matches, goto_page, swallow_shortcut_text};
#[cfg(test)]
use font::{apply_font, preview_size};
use font::{east_asian_char, font_props};
#[cfg(test)]
use forms::custom_value_preview;
pub use forms::{CellMarginsForm, CustomPropertiesForm, TableForm, table_grid_picker};
pub use mailings::{ADDRESS_PARTS, FilterRow, NEW_LIST_FIELDS, RecipientFilter, recipient_table, rule_params};
#[cfg(test)]
pub(crate) use mailings::{hold_rows, merge_order, recipient_list, visible_recipients};
pub(crate) use page_setup::PAPER_LABELS;
#[cfg(test)]
pub(crate) use page_setup::paper_entry;
#[cfg(test)]
use paragraph::apply_paragraph;
use paragraph::{para_fields, paragraph_dialog};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "dialog", rename_all = "camelCase")]
pub enum Dialog {
    RenameDocument {
        name: String,
        message: String,
        focus: bool,
    },
    Font {
        /// The font (Latin text) and the Asian text font, as the font lists name them, and their
        /// values when the dialog opened: OK changes only the ones changed (#505).
        font: String,
        font_was: String,
        font_ea: String,
        font_ea_was: String,
        /// Show the Asian text font box: a Chinese or Japanese interface, or East Asian text or
        /// an Asian font in the selection.
        asian: bool,
        size: String,
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
        sup: bool,
        sub: bool,
        small_caps: bool,
        caps: bool,
        hidden: bool,
        color: String,
        spacing: f32,
        /// Kerning for fonts (`w:kern`), from the Advanced tab: `None` while the selected text
        /// differs, shown neither on nor off; `kern_at` is its Points and above size.
        kern: Option<bool>,
        kern_at: f32,
        /// The properties the fields stood for when the dialog opened: OK sends only what the
        /// user changed, so formatting the dialog can't show (a double underline) or left
        /// alone stays as it is.
        #[serde(skip)]
        start: serde_json::Map<String, Value>,
    },
    Paragraph {
        /// Right-to-left reading order (Direction in Word's Paragraph dialog).
        rtl: bool,
        /// Alignment and indents as seen on the page: `left` is the left side whatever the
        /// direction (the model stores them from the paragraph's start edge).
        align: String,
        left: f32,
        right: f32,
        first: f32,
        before: f32,
        after: f32,
        /// Line spacing as a multiple; Exactly and At Least show as 1.0, and OK keeps them
        /// unless this changes (#467).
        line: f32,
        keep_next: bool,
        keep_lines: bool,
        page_break: bool,
        widow: bool,
        /// Page: 0 Indents and Spacing, 1 Line and Page Breaks, 2 Asian Typography.
        tab: u8,
        /// Asian Typography as the model has it (`para.asianTypography`): kinsoku, word wrap,
        /// hanging punctuation, compress punctuation at line start, space between Asian and
        /// Latin text, space between Asian text and numbers.
        asian: [bool; 6],
        /// Mirror indents, don't add space between paragraphs of the same style, suppress line
        /// numbers, don't hyphenate (#512).
        opts: [bool; 4],
        /// The fields every selected paragraph shares, as `para_fields` names them, from when
        /// the dialog opened: OK sends only the fields that differ from these, so an untouched
        /// tab adds no direct formatting (#1346).
        #[serde(skip)]
        start: serde_json::Map<String, Value>,
        /// The fields the selected paragraphs differ in that the user hasn't set: they show
        /// blank, and OK leaves each paragraph's own value (#1346).
        mixed: BTreeSet<String>,
    },
    /// Tabs (#320).
    Tabs {
        form: Box<crate::dialogs_para::TabsForm>,
    },
    /// Borders and Shading (#320).
    Borders {
        form: Box<crate::dialogs_para::BordersForm>,
    },
    Find {
        query: String,
        replace: String,
        match_case: bool,
        whole_word: bool,
        regex: bool,
        /// Search options under "Search Options" (#543).
        extra: FindExtra,
        /// Find, Replace or Go To, one dialog as in Word (#712).
        tab: FindTab,
        goto_what: GoToWhat,
        /// Go To's box: a page number (`+2`, `-1` move from the current page) or a bookmark.
        goto_target: String,
        message: String,
        /// Put the caret in Find what on the first frame, so typing searches (#772).
        focus: bool,
    },
    InsertTable {
        rows: u32,
        cols: u32,
    },
    PageSetup {
        top: f32,
        bottom: f32,
        left: f32,
        right: f32,
        landscape: bool,
        /// Mirrored margins (the whole document's): left and right become inside and outside.
        mirror: bool,
        /// Document grid: kind (0 none, 1 lines, 2 lines and characters, 3 snap to characters),
        /// lines per page and characters per line; `grid_was` as opened (applied only if changed).
        grid: (u8, u32, u32),
        grid_was: (u8, u32, u32),
        /// Section direction: right to left (`layout.sectionRtl`).
        rtl: bool,
        /// Paper width and height in the measurement unit as shown (landscape: the wide side
        /// first); `paper_was` as opened (`layout.size` runs only if it changed, #1433).
        paper: (f32, f32),
        paper_was: (f32, f32),
        /// Custom size chosen in Paper size: shown even while the numbers match a named size.
        paper_custom: bool,
        /// Why OK didn't apply (the margins don't fit the paper).
        message: String,
        /// Vertical alignment (`layout.verticalAlign`, applied only if changed).
        valign: wordcraft_doc::section::PageVAlign,
        /// Headers and footers: different odd and even pages (the document's) and a different
        /// first page (the section's), applied only if changed (#1230).
        odd_even: bool,
        first_page: bool,
    },
    Link {
        url: String,
        text: String,
        /// Focus and select the address when the dialog first opens.
        focus: bool,
        /// "Place in This Document" instead of "Existing File or Web Page".
        place: bool,
        /// The document's places (`link.targets`), read when that tab is first shown.
        targets: Option<Value>,
        /// The chosen place: the `insert.link` parameters, and the name it shows.
        pick: Option<(Value, String)>,
    },
    Bookmark {
        name: String,
        /// Listed in document order (Word's Sort by Location) rather than by name.
        by_location: bool,
        /// Hidden bookmarks (`_Toc…`, `_Ref…`, made for tables of contents and cross-references) listed too.
        hidden: bool,
        /// A bookmark was deleted, so Cancel reads Close (as in Word).
        deleted: bool,
    },
    WordCount {
        stats: Value,
    },
    Zoom {
        percent: f32,
    },
    Watermark {
        text: String,
        diagonal: bool,
    },
    /// Home › Font › Phonetic Guide: ruby text over the selected text (`format.phonetic`).
    Phonetic {
        base: String,
        ruby: String,
        /// OOXML alignment name (`center`, `distributeLetter`…).
        align: String,
        size: f32,
        offset: f32,
        /// The selection already has a phonetic guide (Remove is offered).
        existing: bool,
        /// Ask the Ruby text box for the keyboard on the first frame (once, so Tab and clicks still move it).
        focus: bool,
    },
    NewStyle {
        name: String,
        based_on: String,
        /// Opened from Manage Styles, which comes back when this closes.
        #[serde(skip)]
        back: bool,
    },
    ModifyStyle {
        #[serde(skip)]
        back: bool,
        /// The dialog that comes back when this closes (Table of Contents › Modify…).
        #[serde(skip)]
        resume: Option<Box<Dialog>>,
        id: String,
        name: String,
        font: String,
        size: f32,
        bold: bool,
        italic: bool,
        color: String,
        before: f32,
        after: f32,
    },
    /// Manage Styles: every style with its preview and description; modify, create, delete and
    /// show or hide them.
    ManageStyles {
        alphabetical: bool,
        selected: String,
    },
    /// Table Design › New / Modify Table Style: one table style part at a time (#714).
    TableStyle {
        /// The style being modified; `None` creates one.
        id: Option<String>,
        name: String,
        /// Style id.
        based_on: String,
        /// Index into `regions` (see `dialogs_table_style::PARTS`).
        region: usize,
        regions: Box<[crate::dialogs_table_style::TableRegion; 13]>,
        /// What the parts showed when the dialog opened (or the base style changed): only
        /// changes are sent, so everything else stays inherited.
        #[serde(skip)]
        basis: Box<[crate::dialogs_table_style::TableRegion; 13]>,
        /// The line the border choices draw.
        pen: crate::dialogs_table_style::BorderPen,
    },
    Commands {
        query: String,
        /// Opened by a shortcut that may also send its key's text (Alt+Q's "q", in the same frame
        /// or the next): that text isn't typed into the box; the next key pressed ends this (#819).
        #[serde(skip)]
        swallow: bool,
    },
    /// Columns, Symbol and Field ([`crate::dialogs_insert`], #321).
    Insert(Box<crate::dialogs_insert::InsertDialog>),
    /// Caption, Index, Mark Index Entry, Table of Contents, Source Manager and Create / Edit
    /// Source ([`crate::dialogs_refs`], #402).
    Refs(Box<crate::dialogs_refs::RefsDialog>),
    /// Insert › Icons: the icon library, searchable and filtered by category (empty for all).
    Icons {
        query: String,
        category: String,
        selected: String,
    },
    /// Paste Special: the clipboard's formats and the chosen one. The clipboard payload stays out
    /// of the serialized dialog state (it can be megabytes).
    PasteSpecial {
        formats: Vec<String>,
        choice: usize,
        #[serde(skip)]
        payload: Value,
        /// Shown when nothing can be pasted: what the clipboard holds, or that it is empty (#833).
        #[serde(skip)]
        note: String,
    },
    /// Tabs: 0 About, 1 Contributors, 2 Models.
    About {
        tab: u8,
    },
    /// Confirm before deleting a recovery copy.
    DiscardRecovery {
        id: String,
        name: String,
    },
    /// "Do you want to save changes?" before a user's New, Open, Close, Envelopes, Labels or
    /// Finish & Merge replaces the document (see [`WordApp::run`]); `then` runs once it is
    /// answered (`ui.saveChanges`), and only while the document it asked about
    /// (`Session::document_id`) is still the one open. `message`: why the last Save failed, shown
    /// in the prompt when it comes back after that failure (#1445); empty otherwise.
    SaveChanges {
        name: String,
        then: String,
        #[serde(skip)]
        params: Value,
        #[serde(skip)]
        document: u64,
        #[serde(default)]
        message: String,
    },
    /// Mailings › Select Recipients / Edit Recipient List: a table of recipients to type or
    /// edit (field names on top), or a file to load instead (#240).
    RecipientList {
        fields: Vec<String>,
        rows: Vec<Vec<String>>,
        message: String,
        /// Filter and Sort (#778): applied with `mailings.filterRecipients` on OK.
        filter: RecipientFilter,
    },
    /// Mailings › Select Recipients › Use an Existing List… picked a workbook with several sheets
    /// (#334): which sheet holds the recipients, and whether its first row names the fields.
    /// OK runs `mailings.recipients` with `params` (the file) plus `sheet` and `headers`.
    SelectTable {
        sheets: Vec<String>,
        selected: usize,
        headers: bool,
        message: String,
        #[serde(skip)]
        params: Value,
    },
    /// Mailings › Insert Merge Field: one of the recipient list's fields (or a typed name).
    InsertMergeField {
        fields: Vec<String>,
        field: String,
    },
    /// A field's code: a new field (Ctrl+F9, `edit` false) or the selected field's code to edit
    /// (double-click a field showing its code, right-click › Edit Field…).
    FieldCode {
        code: String,
        edit: bool,
        message: String,
    },
    /// Mailings › Finish & Merge › Edit Individual Documents… or Print Documents… (`print`):
    /// which records to merge (0 all, 1 the previewed one, 2 the From and To range).
    MergeRecords {
        print: bool,
        choice: u8,
        from: String,
        to: String,
        message: String,
    },
    /// Mailings › Find Recipient.
    FindRecipient {
        text: String,
        message: String,
    },
    /// Mailings › Rules › If…Then…Else… (`IF`) or Skip Record If… (`SKIPIF`): the condition, and
    /// for If the text to insert either way.
    MergeRule {
        rule: String,
        fields: Vec<String>,
        field: String,
        value: String,
        then: String,
        els: String,
    },
    /// Mailings › Match Fields: which recipient-list field fills each address part.
    MatchFields {
        fields: Vec<String>,
        address: Value,
    },
    /// Mailings › Check for Errors: merge fields the recipient list doesn't have.
    CheckErrors {
        unknown: Vec<String>,
        records: u64,
    },
    /// Table Layout › Properties: the table, the caret's row, column and cell. Only what changed
    /// from `basis` (the values when it opened) is applied, as one `table.properties` step.
    TableProperties {
        form: Box<TableForm>,
        #[serde(skip)]
        basis: Box<TableForm>,
    },
    /// Table Layout › Cell Margins: edits the table's default margins.
    CellMargins {
        form: Box<CellMarginsForm>,
        #[serde(skip)]
        basis: Box<CellMarginsForm>,
    },
    /// Opening a password-protected document (#55): the password, then `file.open` with
    /// `params` again. The password is never serialised or printed.
    Password {
        name: String,
        #[serde(skip)]
        params: Value,
        #[serde(skip)]
        password: Password,
        message: String,
    },
    /// File › Info › Protect Document › Encrypt with Password: a password typed twice
    /// (`file.encrypt`); left empty, it removes the password.
    EncryptPassword {
        #[serde(skip)]
        password: Password,
        #[serde(skip)]
        confirm: Password,
        message: String,
    },
    /// Home › Paragraph › Multilevel List › Define New Multilevel List (#328).
    DefineList {
        form: Box<crate::dialogs_lists::ListForm>,
    },
    /// Review › Tracking › Track Changes Options (#328).
    TrackOptions {
        form: Box<crate::dialogs_lists::TrackForm>,
    },
    /// Compare / Combine Documents and AutoCorrect Options ([`crate::dialogs_review`], #412).
    Review(Box<crate::dialogs_review::ReviewDialog>),
    /// Tools › Customize Keyboard (#368).
    CustomizeKeyboard {
        form: Box<crate::dialogs_keyboard::KeyboardForm>,
    },
    /// Tools › Customize Ribbon and the Quick Access Toolbar (#379).
    CustomizeRibbon {
        form: Box<crate::dialogs_ribbon::RibbonForm>,
    },
    /// Developer › Controls › Properties.
    Control {
        form: Box<crate::dialogs_controls::ControlForm>,
    },
    /// Home › Font › Enclose Characters: the style (`none` takes the enclosure away, `shrink`,
    /// `enlarge`), the text and the shape (`circle`, `square`, `triangle`, `diamond`).
    Enclose {
        style: String,
        text: String,
        shape: String,
    },
    /// References › Footnotes › Footnote and Endnote (#385).
    NoteOptions {
        form: Box<crate::dialogs_notes::NoteForm>,
    },
    /// View › Templates › Templates and Add-ins (#377).
    Templates {
        form: Box<crate::dialogs_templates::TemplatesForm>,
    },
    /// Templates and Add-ins › Organizer: copy, delete and rename styles (#377).
    Organizer {
        form: Box<crate::dialogs_templates::OrganizerForm>,
    },
    /// Line Numbers, Hyphenation, Language, Envelopes and Labels (#407).
    Layout(Box<crate::dialogs_layout::LayoutDialog>),
    /// File › Info › Properties › Advanced Properties › Custom: the document's custom
    /// properties (#2982).
    CustomProperties(Box<CustomPropertiesForm>),
}

impl Dialog {
    pub fn name(&self) -> &'static str {
        match self {
            Dialog::RenameDocument { .. } => "renameDocument",
            Dialog::Font { .. } => "font",
            Dialog::Paragraph { .. } => "paragraph",
            Dialog::Tabs { .. } => "tabs",
            Dialog::Borders { .. } => "borders",
            Dialog::Find { tab: FindTab::Find, .. } => "find",
            Dialog::Find { tab: FindTab::Replace, .. } => "replace",
            Dialog::Find { tab: FindTab::GoTo, .. } => "goto",
            Dialog::InsertTable { .. } => "insertTable",
            Dialog::PageSetup { .. } => "pageSetup",
            Dialog::Link { .. } => "link",
            Dialog::Bookmark { .. } => "bookmark",
            Dialog::WordCount { .. } => "wordCount",
            Dialog::Zoom { .. } => "zoom",
            Dialog::Watermark { .. } => "watermark",
            Dialog::Phonetic { .. } => "phonetic",
            Dialog::NewStyle { .. } => "newStyle",
            Dialog::ModifyStyle { .. } => "modifyStyle",
            Dialog::ManageStyles { .. } => "manageStyles",
            Dialog::TableStyle { id: None, .. } => "newTableStyle",
            Dialog::TableStyle { .. } => "modifyTableStyle",
            Dialog::Commands { .. } => "commands",
            Dialog::Insert(d) => d.name(),
            Dialog::Refs(d) => d.name(),
            Dialog::Icons { .. } => "icons",
            Dialog::PasteSpecial { .. } => "pasteSpecial",
            Dialog::About { .. } => "about",
            Dialog::SaveChanges { .. } => "saveChanges",
            Dialog::DiscardRecovery { .. } => "discardRecovery",
            Dialog::RecipientList { .. } => "recipientList",
            Dialog::SelectTable { .. } => "selectTable",
            Dialog::InsertMergeField { .. } => "insertMergeField",
            Dialog::FindRecipient { .. } => "findRecipient",
            Dialog::MergeRecords { print: false, .. } => "mergeRecords",
            Dialog::MergeRecords { .. } => "printRecords",
            Dialog::FieldCode { edit: false, .. } => "fieldCode",
            Dialog::FieldCode { .. } => "editFieldCode",
            Dialog::MergeRule { rule, .. } if rule == "SKIPIF" => "ruleSkipIf",
            Dialog::MergeRule { .. } => "ruleIf",
            Dialog::MatchFields { .. } => "matchFields",
            Dialog::CheckErrors { .. } => "checkErrors",
            Dialog::TableProperties { .. } => "tableProperties",
            Dialog::CellMargins { .. } => "tableCellMargins",
            Dialog::Password { .. } => "password",
            Dialog::EncryptPassword { .. } => "encryptPassword",
            Dialog::DefineList { .. } => "defineList",
            Dialog::TrackOptions { .. } => "trackChangesOptions",
            Dialog::Review(d) => d.name(),
            Dialog::CustomizeKeyboard { .. } => "customizeKeyboard",
            Dialog::CustomizeRibbon { .. } => "customizeRibbon",
            Dialog::Control { .. } => "controlProperties",
            Dialog::Enclose { .. } => "enclose",
            Dialog::NoteOptions { .. } => "noteOptions",
            Dialog::Templates { .. } => "templates",
            Dialog::Organizer { .. } => "organizer",
            Dialog::CustomProperties(..) => "customProperties",
            Dialog::Layout(d) => d.name(),
        }
    }

    /// The dialog that shows a report command's result: Match Fields, Check for Errors.
    pub fn report(id: &str, v: &Value) -> Option<Dialog> {
        let strings = |k: &str| -> Vec<String> {
            v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
        };
        match id {
            "mailings.matchFields" => {
                Some(Dialog::MatchFields { fields: strings("fields"), address: v.get("address").cloned().unwrap_or(Value::Null) })
            }
            "mailings.checkErrors" => {
                Some(Dialog::CheckErrors { unknown: strings("unknownFields"), records: v.get("records").and_then(Value::as_u64).unwrap_or(0) })
            }
            _ => None,
        }
    }

    pub fn open(name: &str, app: &mut WordApp) -> Option<Dialog> {
        if let Some(d) = crate::dialogs_insert::open(name, app) {
            return Some(Dialog::Insert(Box::new(d)));
        }
        if let Some(d) = crate::dialogs_refs::open(name, app) {
            return Some(Dialog::Refs(Box::new(d)));
        }
        if let Some(d) = crate::dialogs_layout::open(name, app) {
            return Some(Dialog::Layout(Box::new(d)));
        }
        if matches!(name, "compare" | "combine" | "autoCorrect" | "restrictEditing") {
            return crate::dialogs_review::open(name, app).map(|d| Dialog::Review(Box::new(d)));
        }
        let st = app.session.run("format.state", &json!({})).unwrap_or_default();
        let s = |k: &str| st.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let b = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
        Some(match name {
            "font" => {
                let font = app.previews.font_shown(&s("font"));
                let font_ea = app.previews.font_shown(&s("fontEastAsia"));
                let lang = crate::i18n::current().code();
                let text = app.session.selected_text();
                let text = if text.is_empty() {
                    app.session.doc.para_at(&app.session.sel.focus).map(|p| p.plain_text().to_string()).unwrap_or_default()
                } else {
                    text
                };
                let asian = ["zh", "ja", "ko"].iter().any(|l| lang.starts_with(l)) || !font_ea.is_empty() || text.chars().any(east_asian_char);
                let kern = st.get("kern").and_then(Value::as_f64);
                let mut d = Dialog::Font {
                    font_was: font.clone(),
                    font,
                    font_ea_was: font_ea.clone(),
                    font_ea,
                    asian,
                    size: st.get("size").and_then(Value::as_f64).map(|v| v.to_string()).unwrap_or_default(),
                    bold: b("bold"),
                    italic: b("italic"),
                    underline: b("underline"),
                    strike: b("strike"),
                    sup: b("superscript"),
                    sub: b("subscript"),
                    small_caps: b("smallCaps"),
                    caps: b("caps"),
                    hidden: b("hidden"),
                    color: s("color"),
                    spacing: st.get("spacing").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    kern: kern.map(|k| k > 0.0),
                    kern_at: kern.filter(|k| *k > 0.0).unwrap_or(12.0) as f32,
                    start: serde_json::Map::new(),
                };
                let props = font_props(&d);
                if let Dialog::Font { start, .. } = &mut d {
                    *start = props;
                }
                d
            }
            "paragraph" | "asianTypography" => {
                let tab = if name == "asianTypography" { 2 } else { 0 };
                let s = &app.session;
                let story = s.sel.focus.story;
                let of = |props: &wordcraft_doc::props::ParaProps| paragraph_dialog(&s.doc.styles.resolve_para(props), tab);
                // Every selected paragraph as the dialog would show it: what they all share is
                // shown, and what OK compares with; the rest shows blank (#1346).
                let mut each = wordcraft_engine::cmd::selected_paths(s).into_iter().filter_map(|path| s.doc.para(story, &path)).map(|p| of(&p.props));
                let mut d = each.next().or_else(|| s.doc.para_at(&s.sel.focus).map(|p| of(&p.props)))?;
                let fields = para_fields(&d);
                let mut shared = fields.clone();
                for other in each {
                    if shared.is_empty() {
                        break;
                    }
                    let other = para_fields(&other);
                    shared.retain(|k, v| other.get(k) == Some(&*v));
                }
                if let Dialog::Paragraph { start, mixed, .. } = &mut d {
                    *mixed = fields.into_iter().map(|(k, _)| k).filter(|k| !shared.contains_key(k)).collect();
                    *start = shared;
                }
                d
            }
            "tabs" => Dialog::Tabs { form: Box::new(crate::dialogs_para::TabsForm::read(app)?) },
            "borders" | "pageBorders" | "shading" => {
                let tab = match name {
                    "pageBorders" => 1,
                    "shading" => 2,
                    _ => 0,
                };
                Dialog::Borders { form: Box::new(crate::dialogs_para::BordersForm::read(app, tab)?) }
            }
            "find" | "replace" | "goto" => Dialog::Find {
                query: if app.session.sel.is_collapsed() { app.session.find.query.clone() } else { app.session.selected_text() },
                replace: app.session.find.replace.clone(),
                match_case: app.session.find.match_case,
                whole_word: app.session.find.whole_word,
                regex: app.session.find.regex,
                extra: FindExtra {
                    wildcards: app.session.find.wildcards,
                    prefix: app.session.find.match_prefix,
                    suffix: app.session.find.match_suffix,
                    ignore_punct: app.session.find.ignore_punct,
                    ignore_space: app.session.find.ignore_space,
                    more: app.session.find.wildcards
                        || app.session.find.match_prefix
                        || app.session.find.match_suffix
                        || app.session.find.ignore_punct
                        || app.session.find.ignore_space,
                    to_replace: false,
                },
                tab: match name {
                    "replace" => FindTab::Replace,
                    "goto" => FindTab::GoTo,
                    _ => FindTab::Find,
                },
                goto_what: GoToWhat::Page,
                goto_target: String::new(),
                message: String::new(),
                focus: true,
            },
            "insertTable" => Dialog::InsertTable { rows: 2, cols: 5 },
            "pageSetup" => {
                let sp = wordcraft_engine::cmd::page::sect(&app.session);
                use wordcraft_doc::section::DocGridType;
                let kind = match sp.doc_grid.map(|g| g.kind) {
                    Some(DocGridType::Lines) => 1,
                    Some(DocGridType::LinesAndChars) => 2,
                    Some(DocGridType::SnapToChars) => 3,
                    _ => 0,
                };
                let size = app.session.doc.styles.resolve_char(Some("Normal"), &Default::default()).size;
                let grid = (kind, sp.grid_lines_per_page().min(1000), sp.grid_chars_per_line(size).min(1000));
                let paper = (sp.page_w / crate::options::unit().pt_per_unit(), sp.page_h / crate::options::unit().pt_per_unit());
                Dialog::PageSetup {
                    paper,
                    paper_was: paper,
                    paper_custom: false,
                    message: String::new(),
                    top: sp.margin_top / crate::options::unit().pt_per_unit(),
                    bottom: sp.margin_bottom / crate::options::unit().pt_per_unit(),
                    left: sp.margin_left / crate::options::unit().pt_per_unit(),
                    right: sp.margin_right / crate::options::unit().pt_per_unit(),
                    landscape: sp.landscape,
                    mirror: app.session.doc.settings.mirror_margins,
                    grid,
                    grid_was: grid,
                    rtl: sp.rtl,
                    valign: sp.valign,
                    odd_even: app.session.doc.settings.even_odd_headers,
                    first_page: sp.title_page,
                }
            }
            "link" => {
                Dialog::Link { url: "https://".into(), text: app.session.selected_text(), place: false, targets: None, pick: None, focus: true }
            }
            "bookmark" => Dialog::Bookmark { name: bookmark_at_caret(&app.session), by_location: false, hidden: false, deleted: false },
            "wordCount" => Dialog::WordCount { stats: app.session.run("review.wordCount", &json!({})).unwrap_or_default() },
            "zoom" => Dialog::Zoom { percent: (app.session.view.zoom * 100.0).round() },
            "watermark" => Dialog::Watermark { text: "CONFIDENTIAL".into(), diagonal: true },
            "phonetic" => {
                let v = wordcraft_engine::cmd::phonetic::state(&app.session);
                let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                Dialog::Phonetic {
                    base: s("base"),
                    ruby: s("ruby"),
                    align: s("align"),
                    size: f("size"),
                    offset: f("offset"),
                    existing: v.get("existing").and_then(Value::as_bool).unwrap_or(false),
                    focus: true,
                }
            }
            "newStyle" => Dialog::NewStyle { name: "Style1".into(), based_on: "Normal".into(), back: false },
            "manageStyles" => Dialog::ManageStyles { alphabetical: false, selected: s("style") },
            "newTableStyle" | "modifyTableStyle" => crate::dialogs_table_style::open(app, name)?,
            "commands" => Dialog::Commands { query: String::new(), swallow: true },
            "icons" => Dialog::Icons { query: String::new(), category: String::new(), selected: String::new() },
            "tableProperties" => {
                let form = Box::new(TableForm::read(app)?);
                Dialog::TableProperties { basis: form.clone(), form }
            }
            "tableCellMargins" => {
                let form = Box::new(CellMarginsForm::read(app)?);
                Dialog::CellMargins { basis: form.clone(), form }
            }
            "enclose" => {
                // The selection's text and its enclosure, if it has one.
                let t = wordcraft_engine::cmd::format::enclose_target(&app.session).ok();
                let cur = t.as_ref().and_then(|t| t.current.clone());
                Dialog::Enclose {
                    style: cur.as_ref().map_or("shrink", |e| e.style.name()).into(),
                    shape: cur.as_ref().map_or("circle", |e| e.shape.name()).into(),
                    text: t.map(|t| t.text).unwrap_or_default(),
                }
            }
            "pasteSpecial" => Dialog::paste_special(app, &json!({})),
            "renameDocument" => Dialog::RenameDocument { name: app.title_stem(), message: String::new(), focus: true },
            "about" => Dialog::About { tab: 0 },
            "contributors" => Dialog::About { tab: 1 },
            "models" => Dialog::About { tab: 2 },
            // The current list to edit, or a new one to type.
            "recipientList" | "newRecipientList" => {
                let m = &app.session.merge;
                if name == "recipientList" && !m.headers.is_empty() {
                    // Every record (not only the filtered ones), with the filter to edit.
                    Dialog::RecipientList {
                        fields: m.headers.clone(),
                        rows: m.source.clone(),
                        message: String::new(),
                        filter: RecipientFilter::from_session(m),
                    }
                } else {
                    let fields: Vec<String> = NEW_LIST_FIELDS.iter().map(ToString::to_string).collect();
                    let rows = vec![vec![String::new(); fields.len()]];
                    Dialog::RecipientList { fields, rows, message: String::new(), filter: RecipientFilter::default() }
                }
            }
            "insertMergeField" => {
                let fields = app.session.merge.headers.clone();
                Dialog::InsertMergeField { field: fields.first().cloned().unwrap_or_default(), fields }
            }
            "findRecipient" => Dialog::FindRecipient { text: String::new(), message: String::new() },
            "mergeRecords" | "printRecords" => Dialog::MergeRecords {
                print: name == "printRecords",
                choice: 0,
                from: "1".into(),
                to: app.session.merge.rows.len().to_string(),
                message: String::new(),
            },
            "fieldCode" => Dialog::FieldCode { code: String::new(), edit: false, message: String::new() },
            "editFieldCode" => {
                let field = wordcraft_engine::cmd::fields::selected(&app.session).into_iter().next()?;
                Dialog::FieldCode { code: field.instr, edit: true, message: String::new() }
            }
            "encryptPassword" => Dialog::EncryptPassword { password: Password::default(), confirm: Password::default(), message: String::new() },
            "ruleIf" | "ruleSkipIf" => {
                let fields = app.session.merge.headers.clone();
                Dialog::MergeRule {
                    rule: if name == "ruleSkipIf" { "SKIPIF" } else { "IF" }.into(),
                    field: fields.first().cloned().unwrap_or_default(),
                    fields,
                    value: String::new(),
                    then: String::new(),
                    els: String::new(),
                }
            }
            "matchFields" | "checkErrors" => {
                let id = if name == "matchFields" { "mailings.matchFields" } else { "mailings.checkErrors" };
                return Self::report(id, &app.session.run(id, &json!({})).unwrap_or_default());
            }
            "defineList" => Dialog::DefineList { form: Box::new(crate::dialogs_lists::ListForm::read(app)) },
            "trackChangesOptions" => Dialog::TrackOptions { form: Box::new(crate::dialogs_lists::TrackForm::read(app)) },
            "customizeKeyboard" => Dialog::CustomizeKeyboard { form: Box::new(crate::dialogs_keyboard::KeyboardForm::read(app)) },
            "customizeRibbon" => {
                Dialog::CustomizeRibbon { form: Box::new(crate::dialogs_ribbon::RibbonForm::read(app, crate::dialogs_ribbon::Page::Ribbon)) }
            }
            "customizeQuickAccess" => {
                Dialog::CustomizeRibbon { form: Box::new(crate::dialogs_ribbon::RibbonForm::read(app, crate::dialogs_ribbon::Page::QuickAccess)) }
            }
            "noteOptions" => Dialog::NoteOptions { form: Box::new(crate::dialogs_notes::NoteForm::read(app)) },
            "templates" => Dialog::Templates { form: Box::new(crate::dialogs_templates::TemplatesForm::read(app)) },
            "organizer" => Dialog::Organizer { form: Box::new(crate::dialogs_templates::OrganizerForm::read(app, None)) },
            "customProperties" => Dialog::CustomProperties(Box::new(CustomPropertiesForm::read(app))),
            "controlProperties" => match crate::dialogs_controls::ControlForm::read(app) {
                Some(form) => Dialog::Control { form: Box::new(form) },
                None => {
                    app.status(tl!("Put the caret in a content control first."));
                    return None;
                }
            },
            _ => return None,
        })
    }

    /// Paste Special for a clipboard payload (`text`, `html`, `rtf`; empty = WordCraft's own copy).
    pub fn paste_special(app: &WordApp, payload: &Value) -> Dialog {
        let formats: Vec<String> = wordcraft_engine::cmd::paste::available(&app.session, payload).into_iter().map(str::to_string).collect();
        let mut kept = json!({});
        for k in ["text", "html", "rtf", "picture"] {
            if let Some(t) = payload.get(k).and_then(Value::as_str) {
                kept[k] = json!(t);
            }
        }
        let note = if formats.is_empty() { crate::system_clipboard::nothing_to_paste(app) } else { String::new() };
        Dialog::PasteSpecial { formats, choice: 0, payload: kept, note }
    }

    pub fn modify_style(app: &WordApp, id: &str) -> Option<Dialog> {
        let st = app.session.doc.styles.get(id)?;
        let rc = app.session.doc.styles.resolve_char(
            if st.kind == wordcraft_doc::StyleKind::Paragraph { Some(id) } else { None },
            &wordcraft_doc::CharProps {
                style: if st.kind == wordcraft_doc::StyleKind::Character { Some(id.into()) } else { None },
                ..Default::default()
            },
        );
        let rp = app.session.doc.styles.resolve_para(&wordcraft_doc::ParaProps { style: Some(id.into()), ..Default::default() });
        Some(Dialog::ModifyStyle {
            back: false,
            resume: None,
            id: id.into(),
            name: st.name.clone(),
            font: rc.font,
            size: rc.size,
            bold: rc.bold,
            italic: rc.italic,
            color: match rc.color {
                wordcraft_doc::TextColor::Rgb(c) => c.hex(),
                wordcraft_doc::TextColor::Auto => "auto".into(),
            },
            before: rp.space_before,
            after: rp.space_after,
        })
    }
}

pub mod dialogs_chart;
pub mod dialogs_controls;
pub mod dialogs_insert;
pub mod dialogs_keyboard;
pub mod dialogs_layout;
pub mod dialogs_lists;
pub mod dialogs_notes;
pub mod dialogs_para;
pub mod dialogs_refs;
pub mod dialogs_review;
pub mod dialogs_ribbon;
pub mod dialogs_smart_art;
pub mod dialogs_table_style;
pub mod dialogs_templates;
#[cfg(test)]
mod tests;
