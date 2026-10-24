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

/// The Find and Replace dialog's search options beyond the first row (#543).
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindExtra {
    pub wildcards: bool,
    pub prefix: bool,
    pub suffix: bool,
    pub ignore_punct: bool,
    pub ignore_space: bool,
    /// The search options are shown.
    pub more: bool,
    /// Special ▾ inserts into Replace with (it last had the focus), not Find what.
    pub to_replace: bool,
}

/// The tabs of the Find and Replace dialog (#712).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FindTab {
    #[default]
    Find,
    Replace,
    GoTo,
}

/// What Go To moves to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GoToWhat {
    #[default]
    Page,
    Bookmark,
}

/// The Special ▾ menu: (name, code) for Find what, without and with wildcards.
const SPECIAL_FIND: &[(&str, &str)] = &[
    ("Paragraph Mark", "^p"),
    ("Tab Character", "^t"),
    ("Any Character", "^?"),
    ("Any Digit", "^#"),
    ("Any Letter", "^$"),
    ("Caret Character", "^^"),
    ("Section Character", "^%"),
    ("Paragraph Character", "^v"),
    ("Column Break", "^n"),
    ("Em Dash", "^+"),
    ("En Dash", "^="),
    ("Graphic", "^g"),
    ("Manual Line Break", "^l"),
    ("Manual Page Break", "^m"),
    ("Nonbreaking Hyphen", "^~"),
    ("Nonbreaking Space", "^s"),
    ("Optional Hyphen", "^-"),
    ("Section Break", "^b"),
    ("White Space", "^w"),
];
const SPECIAL_FIND_WILD: &[(&str, &str)] = &[
    ("Any Character", "?"),
    ("Character in Range", "[-]"),
    ("Beginning of Word", "<"),
    ("End of Word", ">"),
    ("Expression", "()"),
    ("Not", "[!]"),
    ("Num Occurrences", "{,}"),
    ("Previous 1 or More", "@"),
    ("0 or More Characters", "*"),
    ("Paragraph Mark", "^13"),
    ("Tab Character", "^t"),
    ("Caret Character", "^^"),
    ("Section Character", "^%"),
    ("Paragraph Character", "^v"),
    ("Column Break", "^n"),
    ("Em Dash", "^+"),
    ("En Dash", "^="),
    ("Graphic", "^g"),
    ("Manual Line Break", "^l"),
    ("Manual Page Break", "^m"),
    ("Nonbreaking Hyphen", "^~"),
    ("Nonbreaking Space", "^s"),
    ("Optional Hyphen", "^-"),
];
/// Replace with; with wildcards, "Find What Expression" (`\1`) comes first.
const SPECIAL_REPLACE: &[(&str, &str)] = &[
    ("Paragraph Mark", "^p"),
    ("Tab Character", "^t"),
    ("Caret Character", "^^"),
    ("Section Character", "^%"),
    ("Paragraph Character", "^v"),
    ("Clipboard Contents", "^c"),
    ("Column Break", "^n"),
    ("Em Dash", "^+"),
    ("En Dash", "^="),
    ("Find What Text", "^&"),
    ("Manual Line Break", "^l"),
    ("Manual Page Break", "^m"),
    ("Nonbreaking Hyphen", "^~"),
    ("Nonbreaking Space", "^s"),
    ("Optional Hyphen", "^-"),
];

/// The Special ▾ menu button; returns the code picked.
fn special_menu(ui: &mut Ui, extra: &FindExtra) -> Option<&'static str> {
    let mut picked = None;
    ui.menu_button(format!("{} ▾", tl!("Special")), |ui| {
        let wild_replace: &[(&str, &str)] = &[("Find What Expression", "\\1")];
        let lists: [&[(&str, &str)]; 2] = match (extra.to_replace, extra.wildcards) {
            (false, false) => [SPECIAL_FIND, &[]],
            (false, true) => [SPECIAL_FIND_WILD, &[]],
            (true, false) => [SPECIAL_REPLACE, &[]],
            (true, true) => [wild_replace, SPECIAL_REPLACE],
        };
        for (name, code) in lists.into_iter().flatten() {
            if ui.button(format!("{}   {code}", tl!(name))).clicked() {
                picked = Some(*code);
                ui.close();
            }
        }
    });
    picked
}

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
    /// Design › Customize Colors… and Customize Fonts… (#1869).
    Themes(Box<crate::dialogs_themes::ThemeDialog>),
}

/// The Table Properties dialog's fields. Lengths are in the interface unit
/// ([`wordcraft_geom::Unit`], inches by default).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableForm {
    pub width_on: bool,
    pub width: f32,
    /// `left`, `center` or `right`.
    pub align: String,
    pub indent: f32,
    /// Table direction: right to left (`w:bidiVisual`).
    pub rtl: bool,
    pub row_height_on: bool,
    pub row_height: f32,
    pub row_exact: bool,
    pub allow_break: bool,
    pub header_row: bool,
    pub column_width: f32,
    pub cell_width_on: bool,
    pub cell_width: f32,
    /// `top`, `center` or `bottom`.
    pub valign: String,
}

/// The Cell Margins dialog's values, in the interface unit.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellMarginsForm {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

impl CellMarginsForm {
    pub(crate) fn read(app: &WordApp) -> Option<Self> {
        let (tp, _, _) = app.session.sel.focus.path.cell()?;
        let table = app.session.doc.table(app.session.sel.focus.story, &tp)?;
        let scale = crate::options::unit().pt_per_unit();
        let style = table.props.style.as_deref().and_then(|id| app.session.doc.styles.table_style(id));
        let margins = table.props.cell_margins.or_else(|| style.and_then(|s| s.parts.cell_margins)).unwrap_or([0.0, 5.4, 0.0, 5.4]);
        let field = |value: f32, fallback: f32| if value.is_finite() { value.clamp(0.0, 144.0) / scale } else { fallback / scale };
        Some(Self { top: field(margins[0], 0.0), left: field(margins[1], 5.4), bottom: field(margins[2], 0.0), right: field(margins[3], 5.4) })
    }

    /// Send all four values together: changing one side must preserve the other three.
    pub(crate) fn changes(&self, basis: &CellMarginsForm) -> Value {
        if [self.top, self.left, self.bottom, self.right]
            .into_iter()
            .zip([basis.top, basis.left, basis.bottom, basis.right])
            .all(|(a, b)| (a - b).abs() <= 1e-4)
        {
            return json!({});
        }
        let scale = crate::options::unit().pt_per_unit();
        let points = |v: f32, old: f32| if v.is_finite() { (v * scale).clamp(0.0, 144.0) } else { old * scale };
        json!({
            "top": points(self.top, basis.top),
            "left": points(self.left, basis.left),
            "bottom": points(self.bottom, basis.bottom),
            "right": points(self.right, basis.right),
        })
    }
}

impl TableForm {
    /// The caret's table, row, column and cell as the dialog shows them.
    pub fn read(app: &WordApp) -> Option<TableForm> {
        use wordcraft_doc::props::{Align, HeightRule, VAlign};
        let s = &app.session;
        let (tp, r, c) = s.sel.focus.path.cell()?;
        let t = s.doc.table(s.sel.focus.story, &tp)?;
        let k = crate::options::unit().pt_per_unit();
        let row = t.rows.get(r).map(|x| x.props.clone()).unwrap_or_default();
        let cell = t.rows.get(r).and_then(|x| x.cells.get(c)).map(|x| x.props.clone()).unwrap_or_default();
        let tw = wordcraft_engine::cmd::page::sect(s).text_width();
        let width = t.props.width_pct.filter(|p| *p > 0.0).map(|p| tw * p.min(100.0) / 100.0).or(t.props.width.filter(|w| *w > 0.0));
        let fin = |v: f32| if v.is_finite() { v } else { 0.0 };
        Some(TableForm {
            width_on: width.is_some(),
            width: fin(width.unwrap_or(tw)) / k,
            align: match t.props.align {
                Some(Align::Center) => "center",
                Some(Align::Right) => "right",
                _ => "left",
            }
            .into(),
            indent: fin(t.props.indent.unwrap_or(0.0)) / k,
            rtl: t.props.bidi_visual,
            row_height_on: row.height.is_some() && row.height_rule != HeightRule::Auto,
            row_height: fin(row.height.unwrap_or(18.0)) / k,
            row_exact: row.height_rule == HeightRule::Exact,
            allow_break: !row.cant_split,
            header_row: row.header,
            column_width: fin(t.grid.get(t.grid_col(r, c)).copied().unwrap_or(72.0)) / k,
            cell_width_on: cell.width.is_some(),
            cell_width: fin(cell.width.unwrap_or(72.0)) / k,
            valign: match cell.valign {
                Some(VAlign::Center) => "center",
                Some(VAlign::Bottom) => "bottom",
                _ => "top",
            }
            .into(),
        })
    }

    /// `table.properties` parameters for what differs from `basis` (empty when nothing does).
    pub fn changes(&self, basis: &TableForm) -> Value {
        let k = crate::options::unit().pt_per_unit();
        let moved = |a: f32, b: f32| (a - b).abs() > 1e-4;
        let mut v = serde_json::Map::new();
        if self.width_on != basis.width_on || (self.width_on && moved(self.width, basis.width)) {
            v.insert("width".into(), if self.width_on { json!(self.width * k) } else { Value::Null });
        }
        if self.align != basis.align {
            v.insert("align".into(), json!(self.align));
        }
        if moved(self.indent, basis.indent) {
            v.insert("indent".into(), json!(self.indent * k));
        }
        if self.rtl != basis.rtl {
            v.insert("rtl".into(), json!(self.rtl));
        }
        if self.row_height_on != basis.row_height_on
            || (self.row_height_on && (moved(self.row_height, basis.row_height) || self.row_exact != basis.row_exact))
        {
            v.insert("rowHeight".into(), if self.row_height_on { json!(self.row_height * k) } else { Value::Null });
            v.insert("rowHeightRule".into(), json!(if self.row_exact { "exact" } else { "atLeast" }));
        }
        if self.allow_break != basis.allow_break {
            v.insert("allowBreak".into(), json!(self.allow_break));
        }
        if self.header_row != basis.header_row {
            v.insert("headerRow".into(), json!(self.header_row));
        }
        if moved(self.column_width, basis.column_width) {
            v.insert("columnWidth".into(), json!(self.column_width * k));
        }
        if self.cell_width_on != basis.cell_width_on || (self.cell_width_on && moved(self.cell_width, basis.cell_width)) {
            v.insert("cellWidth".into(), if self.cell_width_on { json!(self.cell_width * k) } else { Value::Null });
        }
        if self.valign != basis.valign {
            v.insert("valign".into(), json!(self.valign));
        }
        Value::Object(v)
    }
}

/// The address parts Match Fields lists, with their keys in `mailings.matchFields`' `address`.
pub const ADDRESS_PARTS: [(&str, &str); 9] = [
    ("Courtesy Title", "title"),
    ("First Name", "firstName"),
    ("Last Name", "lastName"),
    ("Company", "company"),
    ("Address", "address"),
    ("City", "city"),
    ("State", "state"),
    ("Postal Code", "postal"),
    ("Country", "country"),
];

/// The `mailings.rules` parameters a Merge Rule dialog stands for, once it names a field.
pub fn rule_params(d: &Dialog) -> Option<Value> {
    let Dialog::MergeRule { rule, field, value, then, els, .. } = d else { return None };
    let field = field.trim();
    if field.is_empty() {
        return None;
    }
    Some(if rule == "SKIPIF" {
        json!({"rule": "SKIPIF", "field": field, "value": value})
    } else {
        json!({"rule": "IF", "field": field, "value": value, "then": then, "else": els})
    })
}

/// The fields a new recipient list starts with.
pub const NEW_LIST_FIELDS: [&str; 5] = ["First Name", "Last Name", "Address", "City", "Postal Code"];

/// A typed recipient list ready for `mailings.recipients`: field names trimmed, blank rows
/// dropped. An error says what to fix.
pub fn recipient_table(fields: &[String], rows: &[Vec<String>]) -> Result<Value, &'static str> {
    let fields: Vec<&str> = fields.iter().map(|f| f.trim()).collect();
    if fields.is_empty() {
        return Err("Add at least one field.");
    }
    if fields.iter().any(|f| f.is_empty()) {
        return Err("Every field needs a name.");
    }
    for (i, f) in fields.iter().enumerate() {
        if fields.iter().skip(i + 1).any(|g| g.eq_ignore_ascii_case(f)) {
            return Err("Field names must be different.");
        }
    }
    let rows: Vec<&Vec<String>> = rows.iter().filter(|r| r.iter().any(|c| !c.trim().is_empty())).collect();
    if rows.is_empty() {
        return Err("Type at least one recipient.");
    }
    Ok(json!({"fields": fields, "rows": rows}))
}

/// The Recipient List dialog's Filter and Sort section (#778).
#[derive(Clone, Debug, Default, Serialize)]
pub struct RecipientFilter {
    /// Whether the section is shown.
    pub open: bool,
    pub rules: Vec<FilterRow>,
    /// Sort by, then by, then by: a field (empty: none) and descending.
    pub sort: [(String, bool); 3],
    /// Find in list (#1576): only rows with a cell containing this (any case) are shown.
    /// View only, like `only_merged`: neither changes the list or the merge.
    pub find: String,
    /// Show only the records the filter keeps (#1576).
    pub only_merged: bool,
    /// The grid's rows held while one of its cells is edited (see [`hold_rows`]).
    #[serde(skip)]
    pub(crate) held: Option<Vec<usize>>,
    /// The grid cell that had the keyboard last frame.
    #[serde(skip)]
    pub(crate) editing: Option<egui::Id>,
    /// The last merge order worked out, keyed by a hash of the list, filter and sort, so a long
    /// list isn't filtered and sorted again on every frame.
    #[serde(skip)]
    pub(crate) order_cache: Option<(u64, Vec<Option<usize>>)>,
}

/// One filter condition row: `field op value`, joined to the row above with And or Or.
#[derive(Clone, Debug, Serialize)]
pub struct FilterRow {
    pub field: String,
    #[serde(serialize_with = "op_id")]
    pub op: FilterOp,
    pub value: String,
    pub or: bool,
}

fn op_id<S: serde::Serializer>(op: &FilterOp, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(op.id())
}

impl RecipientFilter {
    /// The session's current filter and sort.
    pub fn from_session(m: &wordcraft_engine::cmd::mailings::MergeState) -> Self {
        let mut sort: [(String, bool); 3] = Default::default();
        for (slot, k) in sort.iter_mut().zip(&m.sort) {
            *slot = (k.field.clone(), k.descending);
        }
        let rules: Vec<FilterRow> =
            m.filter.iter().map(|r| FilterRow { field: r.field.clone(), op: r.op, value: r.value.clone(), or: r.or }).collect();
        RecipientFilter { open: !rules.is_empty() || !m.sort.is_empty(), rules, sort, ..Default::default() }
    }

    /// The rules and sort keys on `fields` (a rule on a field renamed or removed in the table
    /// drops, as the engine drops it when the list is edited).
    fn engine(&self, fields: &[String]) -> (Vec<FilterRule>, Vec<SortKey>) {
        let known = |f: &str| !f.trim().is_empty() && fields.iter().any(|h| h.trim().eq_ignore_ascii_case(f.trim()));
        let rules = self
            .rules
            .iter()
            .filter(|r| known(&r.field))
            .map(|r| FilterRule { field: r.field.clone(), op: r.op, value: r.value.clone(), or: r.or })
            .collect();
        let sort = self.sort.iter().filter(|(f, _)| known(f)).map(|(f, d)| SortKey { field: f.clone(), descending: *d }).collect();
        (rules, sort)
    }

    /// Each record's place in the merge (None: filtered out), from the cache when the list,
    /// filter and sort haven't changed.
    fn order(&mut self, fields: &[String], rows: &[Vec<String>]) -> &[Option<usize>] {
        use std::hash::{Hash, Hasher};
        let (rules, sort) = self.engine(fields);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        fields.hash(&mut h);
        rows.hash(&mut h);
        for r in &rules {
            (&r.field, r.op.id(), &r.value, r.or).hash(&mut h);
        }
        for k in &sort {
            (&k.field, k.descending).hash(&mut h);
        }
        let key = h.finish();
        if self.order_cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let mut order = vec![None; rows.len()];
            for (n, i) in wordcraft_engine::cmd::mailings::select_records(fields, rows, &rules, &sort).into_iter().enumerate() {
                if let Some(slot) = order.get_mut(i) {
                    *slot = Some(n + 1);
                }
            }
            self.order_cache = Some((key, order));
        }
        self.order_cache.as_ref().map_or(&[], |(_, o)| o.as_slice())
    }

    fn is_empty(&self) -> bool {
        self.rules.iter().all(|r| r.field.is_empty()) && self.sort.iter().all(|(f, _)| f.is_empty())
    }

    /// `mailings.filterRecipients` params for a list with `fields`.
    pub fn params(&self, fields: &[String]) -> Value {
        let (rules, sort) = self.engine(fields);
        json!({
            "rules": rules.iter().map(|r| json!({"field": r.field, "op": r.op.id(), "value": r.value, "join": if r.or { "or" } else { "and" }})).collect::<Vec<_>>(),
            "sort": sort.iter().map(|k| json!({"field": k.field, "descending": k.descending})).collect::<Vec<_>>(),
        })
    }
}

/// Each record's place in the merge (None: filtered out), when `filter` filters or sorts.
#[cfg(test)]
pub(crate) fn merge_order(fields: &[String], rows: &[Vec<String>], filter: &RecipientFilter) -> Option<Vec<Option<usize>>> {
    (!filter.is_empty()).then(|| {
        let (rules, sort) = filter.engine(fields);
        let mut order = vec![None; rows.len()];
        for (k, i) in wordcraft_engine::cmd::mailings::select_records(fields, rows, &rules, &sort).into_iter().enumerate() {
            if let Some(slot) = order.get_mut(i) {
                *slot = Some(k + 1);
            }
        }
        order
    })
}

/// The Recipient List grid's rows (#1576): real indices into `rows`, in list order, matching
/// `find` and, with `only_merged`, kept by the filter (`order` from [`merge_order`]).
pub(crate) fn visible_recipients(rows: &[Vec<String>], order: Option<&[Option<usize>]>, find: &str, only_merged: bool) -> Vec<usize> {
    let needle: Vec<char> = find.trim().chars().flat_map(fold_char).collect();
    let kept = |i: usize| !only_merged || order.is_none_or(|o| o.get(i).copied().flatten().is_some());
    let found = |row: &Vec<String>| needle.is_empty() || row.iter().any(|c| contains_folded(c, &needle));
    rows.iter().enumerate().filter(|(i, row)| kept(*i) && found(row)).map(|(i, _)| i).collect()
}

/// The rows the grid shows (#1576): `fresh`, except while a grid cell is being edited, when the
/// rows shown as the edit began stay put, so a row the edit stops matching doesn't vanish (or
/// slide another record under the cursor) mid-edit. `rows` is the list's length.
pub(crate) fn hold_rows(fresh: Vec<usize>, held: &mut Option<Vec<usize>>, editing: bool, rows: usize) -> Vec<usize> {
    if !editing {
        *held = None;
        return fresh;
    }
    match held {
        Some(h) => h.iter().copied().filter(|&r| r < rows).collect(),
        None => {
            *held = Some(fresh.clone());
            fresh
        }
    }
}

impl RecipientFilter {
    /// New Entry: show every row again, so the new, empty entry can't be hidden by the view.
    pub(crate) fn show_all(&mut self) {
        self.find.clear();
        self.only_merged = false;
        self.held = None;
    }
}

/// Lower-cases a char for searching; final sigma `ς` counts as `σ`, so `ΟΣ` matches `ος`.
fn fold_char(c: char) -> impl Iterator<Item = char> {
    c.to_lowercase().map(|l| if l == 'ς' { 'σ' } else { l })
}

/// Whether `hay` contains `needle` (folded chars) in any case, without allocating.
fn contains_folded(hay: &str, needle: &[char]) -> bool {
    hay.char_indices().any(|(i, _)| {
        let mut h = hay.get(i..).unwrap_or_default().chars().flat_map(fold_char);
        needle.iter().all(|n| h.next() == Some(*n))
    })
}

/// A colour menu: a swatch with the colour grid and No Color. `value` is hex, empty for none.
pub(crate) fn color_menu(ui: &mut Ui, theme: &[wordcraft_doc::Rgb], value: &mut String) {
    color_field(ui, theme, value, "No Color", "");
}

/// A colour field: a swatch and a drop-down with `none_label` (which sets `none_value`), the
/// colour grid, Recent Colors and More Colors… (#606). `value` is hex or `none_value`. An empty
/// `none_label` leaves the none option out: the field always holds a colour (#1869).
pub(crate) fn color_field(ui: &mut Ui, theme: &[wordcraft_doc::Rgb], value: &mut String, none_label: &str, none_value: &str) {
    ui.horizontal(|ui| {
        let id = ui.id().with("colorField");
        crate::colors::take_field(ui.ctx(), id, value);
        let c = wordcraft_doc::Rgb::parse(value);
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        let t = Tokens::get(ui.ctx());
        match c {
            Some(c) => {
                ui.painter().rect_filled(r, 2.0, crate::theme::c32(c));
            }
            None => {
                ui.painter().line_segment([r.left_bottom(), r.right_top()], egui::Stroke::new(1.0, t.text_dim));
            }
        }
        ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
        let label = if c.is_some() || none_label.is_empty() { value.clone() } else { tl!(none_label).to_string() };
        ui.menu_button(label, |ui| {
            if !none_label.is_empty() && ui.button(tl!(none_label)).clicked() {
                *value = none_value.to_string();
                ui.close();
            }
            crate::colors::field_menu(ui, theme, id, value);
        });
    });
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
            Dialog::Layout(d) => d.name(),
            Dialog::Themes(d) => d.name(),
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
        if let Some(d) = crate::dialogs_themes::open(name, app) {
            return Some(Dialog::Themes(Box::new(d)));
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

/// Grid picker for Insert › Table (hover to size and preview on the page, click to insert).
pub fn table_grid_picker(ui: &mut Ui, app: &mut WordApp) {
    let t = Tokens::get(ui.ctx());
    let id = egui::Id::new("table_picker_hover");
    let hover: (usize, usize) = ui.data(|d| d.get_temp(id)).unwrap_or((0, 0));
    ui.label(if hover.0 > 0 {
        crate::i18n::fmt(tl!("{cols}×{rows} Table"), &[("cols", &hover.1.to_string()), ("rows", &hover.0.to_string())])
    } else {
        tl!("Insert Table").to_string()
    });
    let mut new_hover = (0, 0);
    let mut clicked = None;
    egui::Grid::new("tgp").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
        for r in 1..=8 {
            for c in 1..=10 {
                let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                let on = r <= hover.0 && c <= hover.1;
                ui.painter().rect(
                    rect,
                    1.0,
                    if on { t.checked } else { t.input },
                    egui::Stroke::new(1.0, if on { t.accent } else { t.border_strong }),
                    egui::StrokeKind::Inside,
                );
                if resp.hovered() {
                    new_hover = (r, c);
                }
                if resp.clicked() {
                    clicked = Some((r, c));
                }
            }
            ui.end_row();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, new_hover));
    if new_hover != hover {
        // The page may already be drawn this frame (Focus drops the ribbon over it afterwards).
        ui.ctx().request_repaint();
    }
    if let Some((r, c)) = clicked {
        let _ = app.run("insert.table", json!({"rows": r, "cols": c}));
        ui.close();
    } else if new_hover.0 > 0 {
        // Word's Live Preview: the page shows the hovered table until a square is clicked.
        app.canvas.table_preview = Some(new_hover);
    }
}

/// The open dialog, as (window id, frames it has been centred for).
fn dialog_top_key() -> egui::Id {
    egui::Id::new("dialog_top")
}

/// Frames a dialog stays centred: the first one measures it, the second centres that size.
const DIALOG_CENTRED_FRAMES: u8 = 2;

/// Places a dialog: centred for its first frames, then left where it is with its top-centre point
/// as the pivot, so a taller or shorter tab (Paragraph, Font, Borders and Shading…) only moves
/// the bottom edge and the tab strip stays under the mouse (#507). It is no longer anchored after
/// that, so dragging the title bar moves it (#732). egui keeps it on screen if it grows past the
/// bottom.
fn place_dialog<'a>(ctx: &egui::Context, id: egui::Id, w: egui::Window<'a>) -> egui::Window<'a> {
    let w = w.pivot(egui::Align2::CENTER_TOP);
    match ctx.data(|d| d.get_temp::<(egui::Id, u8)>(dialog_top_key())) {
        Some((open, frames)) if open == id && frames >= DIALOG_CENTRED_FRAMES => w,
        _ => w.anchor(egui::Align2::CENTER_CENTER, vec2(0.0, -40.0)),
    }
}

/// After a frame of a dialog: counts the frames it has been centred for.
fn count_dialog_frame(ctx: &egui::Context, id: egui::Id) {
    let frames = match ctx.data(|d| d.get_temp::<(egui::Id, u8)>(dialog_top_key())) {
        Some((open, frames)) if open == id => frames,
        _ => 0,
    };
    if frames < DIALOG_CENTRED_FRAMES {
        ctx.data_mut(|d| d.insert_temp(dialog_top_key(), (id, frames + 1)));
    }
}

pub fn show(app: &mut WordApp, ctx: &egui::Context) {
    let Some(mut d) = app.dialog.take() else {
        ctx.data_mut(|d| d.remove::<(egui::Id, u8)>(dialog_top_key()));
        return;
    };
    let mut close = false;
    let title = match &d {
        Dialog::RenameDocument { .. } => "Rename Document",
        Dialog::Font { .. } => "Font",
        Dialog::Paragraph { .. } => "Paragraph",
        Dialog::Tabs { .. } => "Tabs",
        Dialog::Borders { .. } => "Borders and Shading",
        Dialog::Find { .. } => "Find and Replace",
        Dialog::InsertTable { .. } => "Insert Table",
        Dialog::PageSetup { .. } => "Page Setup",
        Dialog::Link { .. } => "Insert Hyperlink",
        Dialog::Bookmark { .. } => "Bookmark",
        Dialog::WordCount { .. } => "Word Count",
        Dialog::Zoom { .. } => "Zoom",
        Dialog::Watermark { .. } => "Custom Watermark",
        Dialog::Phonetic { .. } => "Phonetic Guide",
        Dialog::NewStyle { .. } => "Create New Style",
        Dialog::ModifyStyle { .. } => "Modify Style",
        Dialog::ManageStyles { .. } => "Manage Styles",
        Dialog::TableStyle { id: None, .. } => "New Table Style",
        Dialog::TableStyle { .. } => "Modify Table Style",
        Dialog::Commands { .. } => "Search Commands",
        Dialog::Insert(d) => d.title(),
        Dialog::Refs(d) => d.title(),
        Dialog::Icons { .. } => "Icons",
        Dialog::PasteSpecial { .. } => "Paste Special",
        Dialog::About { .. } => "About WordCraft",
        Dialog::SaveChanges { .. } => "WordCraft",
        Dialog::DiscardRecovery { .. } => "Discard recovery copy",
        Dialog::RecipientList { .. } => "Recipient List",
        Dialog::SelectTable { .. } => "Select Table",
        Dialog::InsertMergeField { .. } => "Insert Merge Field",
        Dialog::FindRecipient { .. } => "Find Recipient",
        Dialog::MergeRecords { .. } => "Finish & Merge",
        Dialog::FieldCode { edit: false, .. } => "Field",
        Dialog::FieldCode { .. } => "Edit Field Code",
        Dialog::MergeRule { rule, .. } if rule == "SKIPIF" => "Skip Record If",
        Dialog::MergeRule { .. } => "If…Then…Else",
        Dialog::MatchFields { .. } => "Match Fields",
        Dialog::CheckErrors { .. } => "Check for Errors",
        Dialog::TableProperties { .. } => "Table Properties",
        Dialog::CellMargins { .. } => "Cell Margins",
        Dialog::Password { .. } => "Password",
        Dialog::EncryptPassword { .. } => "Encrypt with Password",
        Dialog::DefineList { .. } => "Define New Multilevel List",
        Dialog::TrackOptions { .. } => "Track Changes Options",
        Dialog::Review(d) => d.title(),
        Dialog::CustomizeKeyboard { .. } => "Customize Keyboard",
        Dialog::CustomizeRibbon { .. } => "Customize Ribbon",
        Dialog::Control { .. } => "Content Control Properties",
        Dialog::Enclose { .. } => "Enclose Characters",
        Dialog::NoteOptions { .. } => "Footnote and Endnote",
        Dialog::Templates { .. } => "Templates and Add-ins",
        Dialog::Organizer { .. } => "Organizer",
        Dialog::Layout(d) => d.title(),
        Dialog::Themes(d) => d.title(),
    };
    let id = egui::Id::new(("dialog", title));
    let window = crate::window_close::window(ctx, egui::Window::new(tl!(title)).id(id).collapsible(false).resizable(false));
    let shown = place_dialog(ctx, id, window).show(ctx, |ui| {
        ui.set_min_width(340.0);
        close = body(app, ui, &mut d);
    });
    if let Some(shown) = &shown {
        if let Dialog::SaveChanges { name, message, .. } = &d {
            let question = crate::i18n::fmt(tl!("Do you want to save changes to {name}?"), &[("name", name)]);
            let mut description = tl!("Your changes will be lost if you don't save them.").to_string();
            if !message.is_empty() {
                description.push('\n');
                description.push_str(message);
            }
            // This is an urgent decision, not another application window. Keep its context with
            // the focused choice, including when a failed Save brings the same prompt back.
            ctx.accesskit_node_builder(shown.response.id, |node| {
                node.set_role(egui::accesskit::Role::AlertDialog);
                node.set_label(question);
                node.set_description(description);
            });
        }
        count_dialog_frame(ctx, id);
        close |= crate::window_close::show(ctx, &shown.response, &Tokens::get(ctx));
    }
    if close {
        app.canvas.want_focus = true;
    } else {
        app.dialog = Some(d);
    }
}

/// Labels of Page Setup's paper size (#1433), in one place in case Word's wording differs.
pub(crate) const PAPER_LABELS: PaperLabels =
    PaperLabels { group: "Paper", size: "Paper size:", custom: "Custom size", width: "Width:", height: "Height:", more: "More Paper Sizes…" };

pub(crate) struct PaperLabels {
    pub group: &'static str,
    pub size: &'static str,
    pub custom: &'static str,
    pub width: &'static str,
    pub height: &'static str,
    /// The Size menu's last item, opening Page Setup.
    pub more: &'static str,
}

/// The named paper size `paper` (in the measurement unit, either orientation) is, or None for a
/// custom size.
pub(crate) fn paper_entry(paper: (f32, f32)) -> Option<&'static str> {
    let ppu = crate::options::unit().pt_per_unit();
    wordcraft_geom::paper_name(paper.0 * ppu, paper.1 * ppu)
}

/// The commands Page Setup's OK runs for the paper, orientation and margins (#1433), all in the
/// measurement unit as the dialog shows them. Refuses when the margins don't fit the paper.
/// Otherwise it orders them so that none is refused on the way: the size first
/// when the section's margins fit it, else the margins first, else margins of 0 in between.
/// `layout.size` keeps the section's orientation; `layout.orientation` then turns the page and
/// its margins to what the dialog shows. A size equal to the opened one (either way round,
/// within 0.01 pt) doesn't run `layout.size`.
fn page_layout_commands(
    app: &WordApp,
    (paper, was): ((f32, f32), (f32, f32)),
    landscape: bool,
    (top, bottom, left, right): (f32, f32, f32, f32),
) -> Result<Vec<(&'static str, Value)>, &'static str> {
    use wordcraft_engine::cmd::page::{MARGINS_DONT_FIT, margins_fit, sect};
    let ppu = crate::options::unit().pt_per_unit();
    let (paper, was) = ((paper.0 * ppu, paper.1 * ppu), (was.0 * ppu, was.1 * ppu));
    let m = (top * ppu, bottom * ppu, left * ppu, right * ppu);
    let cur = sect(&app.session);
    if !margins_fit(paper, m, cur.gutter) {
        return Err(MARGINS_DONT_FIT);
    }
    let turned = |on: bool, (a, b): (f32, f32)| if on { (a.max(b), a.min(b)) } else { (a.min(b), a.max(b)) };
    let ((w0, h0), (w1, h1)) = (turned(false, paper), turned(false, was));
    let new_size = (w0 - w1).abs() >= 0.01 || (h0 - h1).abs() >= 0.01;
    let size = ("layout.size", json!({"width": paper.0, "height": paper.1}));
    let orientation = ("layout.orientation", json!({"value": if landscape { "landscape" } else { "portrait" }}));
    let margins = ("layout.margins", json!({"top": m.0, "bottom": m.1, "left": m.2, "right": m.3}));
    let old_margins = (cur.margin_top, cur.margin_bottom, cur.margin_left, cur.margin_right);
    Ok(if !new_size || margins_fit(turned(cur.landscape, paper), old_margins, cur.gutter) {
        let mut v = if new_size { vec![size] } else { Vec::new() };
        v.extend([orientation, margins]);
        v
    } else if margins_fit(turned(landscape, (cur.page_w, cur.page_h)), m, cur.gutter) {
        vec![orientation, margins, size]
    } else {
        vec![orientation, ("layout.margins", json!({"top": 0.0, "bottom": 0.0, "left": 0.0, "right": 0.0})), size, margins]
    })
}

/// Run a dialog's commands as one undo step. If one fails, put the document and the undo stack
/// back as they were (nothing applied) and return its error.
fn run_as_one_step(app: &mut WordApp, commands: Vec<(&str, Value)>) -> Result<(), String> {
    let snapshot = app.session.edit_snapshot();
    for (i, (id, params)) in commands.into_iter().enumerate() {
        if i > 0 {
            app.session.join_next_undo();
        }
        if let Err(e) = app.run(id, params) {
            app.session.restore(snapshot);
            return Err(e);
        }
    }
    Ok(())
}

/// Page Setup's Paper group (#1433): a Paper size list (the named sizes and Custom size) and
/// the width and height. The fields follow the orientation (the wide side is the width in
/// landscape), and a typed size that is wider than tall turns the orientation to landscape.
fn page_paper_group(ui: &mut Ui, paper: &mut (f32, f32), landscape: &mut bool, custom: &mut bool) {
    if (*landscape && paper.0 < paper.1) || (!*landscape && paper.0 > paper.1) {
        *paper = (paper.1, paper.0);
    }
    let unit = crate::options::unit();
    let ppu = unit.pt_per_unit();
    let l = &PAPER_LABELS;
    ui.label(egui::RichText::new(tl!(l.group)).font(semibold(12.5)));
    ui.horizontal(|ui| {
        let label = ui.label(tl!(l.size));
        let entry = paper_entry(*paper).filter(|_| !*custom);
        let shown = tl!(entry.unwrap_or(l.custom));
        let combo = egui::ComboBox::from_id_salt("ps_paper_size").width(180.0).height(400.0).selected_text(shown).show_ui(ui, |ui| {
            for (n, w, h) in wordcraft_geom::PAPER_SIZES {
                if ui.selectable_label(entry == Some(*n), tl!(n)).clicked() {
                    let (w, h) = (w / ppu, h / ppu);
                    *paper = if *landscape { (w.max(h), w.min(h)) } else { (w.min(h), w.max(h)) };
                    *custom = false;
                }
            }
            // Custom size keeps the numbers to edit, and stays chosen while they match a name.
            if ui.selectable_label(entry.is_none(), tl!(l.custom)).clicked() {
                *custom = true;
            }
        });
        combo.response.labelled_by(label.id);
    });
    egui::Grid::new("ps_paper").num_columns(4).show(ui, |ui| {
        use wordcraft_engine::cmd::page::{MAX_PAGE_SIDE, MIN_PAGE_SIDE};
        let range = MIN_PAGE_SIDE / ppu..=MAX_PAGE_SIDE / ppu;
        ui.label(tl!(l.width));
        let w = ui.add(egui::DragValue::new(&mut paper.0).speed(3.6 / ppu).range(range.clone()).measure(unit).max_decimals(2));
        ui.label(tl!(l.height));
        let h = ui.add(egui::DragValue::new(&mut paper.1).speed(3.6 / ppu).range(range).measure(unit).max_decimals(2));
        if (w.changed() || h.changed()) && paper.0 != paper.1 {
            *landscape = paper.0 > paper.1;
        }
        ui.end_row();
    });
}

/// Cancel key for dialogs: Escape everywhere, plus ⌘. (Command+Period) on macOS.
pub(crate) fn cancel_key_pressed(ui: &Ui) -> bool {
    ui.input(|i| i.key_pressed(egui::Key::Escape) || (cfg!(target_os = "macos") && i.modifiers.command && i.key_pressed(egui::Key::Period)))
}

/// Lays out an OK and a Cancel button right-to-left, in the platform's order:
/// macOS puts Cancel left of the default OK button; Windows and Linux put OK left of Cancel.
/// Returns `(ok, cancel)` responses. Call inside a right-to-left layout.
pub(crate) fn ok_cancel_responses(ui: &mut Ui, draw_ok: impl FnOnce(&mut Ui) -> egui::Response, cancel: &str) -> (egui::Response, egui::Response) {
    if cfg!(target_os = "macos") {
        let okb = draw_ok(ui);
        (okb, ui.button(cancel))
    } else {
        let cancelb = ui.button(cancel);
        (draw_ok(ui), cancelb)
    }
}

pub(crate) fn buttons(ui: &mut Ui, ok: &str) -> (bool, bool) {
    let mut r = (false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Enter and Escape belong to the Colors dialog while it is open above this one.
            let keys = !crate::colors::is_open(ui.ctx());
            let (okb, cancelb) = ok_cancel_responses(ui, |ui| ui.add(crate::widgets::primary_button(ui, ok)), tl!("Cancel"));
            if cancelb.clicked() || (keys && cancel_key_pressed(ui)) {
                r.1 = true;
            }
            if okb.clicked() || (keys && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                r.0 = true;
            }
        });
    });
    r
}

/// The Font dialog preview's size: the Size box's points between 6 and 72, else 12. `"NaN"` and
/// `"inf"` parse as floats too, and a NaN font size trips epaint's glyph assertion (#3663).
fn preview_size(size: &str) -> f32 {
    size.trim().parse::<f32>().ok().filter(|pt| pt.is_finite()).unwrap_or(12.0).clamp(6.0, 72.0)
}

/// The character properties the Font dialog's fields stand for.
fn font_props(d: &Dialog) -> serde_json::Map<String, Value> {
    let Dialog::Font {
        font, font_ea, size, bold, italic, underline, strike, sup, sub, small_caps, caps, hidden, color, spacing, kern, kern_at, ..
    } = d
    else {
        return serde_json::Map::new();
    };
    let mut props = serde_json::Map::new();
    props.insert("bold".into(), json!(*bold));
    props.insert("italic".into(), json!(*italic));
    props.insert("strike".into(), json!(*strike));
    props.insert("underline".into(), json!(if *underline { "single" } else { "none" }));
    let vert = if *sup {
        "superscript"
    } else if *sub {
        "subscript"
    } else {
        "baseline"
    };
    props.insert("vertAlign".into(), json!(vert));
    props.insert("smallCaps".into(), json!(*small_caps));
    props.insert("caps".into(), json!(*caps));
    props.insert("hidden".into(), json!(*hidden));
    props.insert("spacing".into(), json!(*spacing));
    match kern {
        Some(true) => {
            props.insert("kern".into(), json!(kern_at.clamp(1.0, 1638.0)));
        }
        Some(false) => {
            props.insert("kern".into(), json!(0.0));
        }
        None => {}
    }
    if !font.trim().is_empty() {
        props.insert("font".into(), json!(font.trim()));
    }
    if !font_ea.trim().is_empty() {
        props.insert("fontEa".into(), json!(font_ea.trim()));
    }
    if let Ok(s) = size.trim().parse::<f64>() {
        props.insert("size".into(), json!(s));
    }
    if let Some(c) = wordcraft_doc::Rgb::parse(color) {
        props.insert("color".into(), json!({"Rgb": [c.0, c.1, c.2]}));
    } else if color.eq_ignore_ascii_case("auto") {
        // Automatic, from the colour field's drop-down (#606).
        props.insert("color".into(), json!("Auto"));
    }
    props
}

/// Is `c` Chinese, Japanese or Korean text (ideographs, kana, Hangul, CJK punctuation, fullwidth
/// forms)?
fn east_asian_char(c: char) -> bool {
    matches!(u32::from(c), 0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// The Font dialog's OK: one formatting step that sets only the properties the user changed, so
/// formatting the dialog can't show or left alone stays as it is. Fonts are set by their stored
/// (English) names, the Latin font for Persian/Arabic text too (as `format.font`).
fn apply_font(app: &mut WordApp, d: &Dialog) {
    let Dialog::Font { start, .. } = d else { return };
    let mut changed: serde_json::Map<String, Value> = font_props(d).into_iter().filter(|(k, v)| start.get(k) != Some(v)).collect();
    if let Some(f) = changed.get("font").and_then(Value::as_str).map(str::to_string) {
        let f = app.previews.font_stored(&f);
        changed.insert("font".into(), json!(f));
        changed.insert("fontCs".into(), json!(f));
    }
    if let Some(f) = changed.get("fontEa").and_then(Value::as_str).map(str::to_string) {
        changed.insert("fontEa".into(), json!(app.previews.font_stored(&f)));
    }
    if !changed.is_empty() {
        let _ = app.run("format.set", json!({"props": changed}));
    }
}

/// Returns true to close.
fn body(app: &mut WordApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    match d {
        Dialog::RenameDocument { name, message, focus } => {
            ui.label(tl!("Document name"));
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(320.0).char_limit(200));
            if *focus {
                r.request_focus();
                *focus = false;
            }
            if !message.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, message.as_str());
            }
            let (ok, cancel) = buttons(ui, tl!("Rename"));
            if cancel {
                return true;
            }
            if ok && name.trim() == app.title_stem() {
                return true;
            }
            if ok {
                match app.run("file.rename", json!({"name": name})) {
                    Ok(_) => return true,
                    Err(e) => *message = e,
                }
            }
            false
        }

        Dialog::Font {
            font,
            font_ea,
            asian,
            size,
            bold,
            italic,
            underline,
            strike,
            sup,
            sub,
            small_caps,
            caps,
            hidden,
            color,
            spacing,
            kern,
            kern_at,
            ..
        } => {
            // Build the sample text format for the preview pane below.
            let preview_size = preview_size(size);
            let preview_color = wordcraft_doc::Rgb::parse(color).map(crate::theme::c32).unwrap_or_else(|| ui.visuals().text_color());
            let mut fmt = egui::TextFormat {
                font_id: egui::FontId::new(preview_size, egui::FontFamily::Proportional),
                color: preview_color,
                ..Default::default()
            };
            if *bold {
                fmt.font_id = crate::theme::semibold(preview_size);
            }
            if *italic {
                fmt.italics = true;
            }
            if *underline {
                fmt.underline = egui::Stroke::new(1.0, preview_color);
            }
            if *strike {
                fmt.strikethrough = egui::Stroke::new(1.0, preview_color);
            }
            egui::Grid::new("fontdlg").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                // Word's two font boxes when East Asian text is in use: the Latin text font and
                // the Asian text font, each with its own list (type to find a font).
                let prev = app.previews.font_preview_fn();
                if *asian {
                    ui.label(tl!("Asian text font:"));
                    let fams = app.previews.east_asian_families();
                    if let Some(f) = crate::widgets::font_combo(ui, "fontdlg_asian", 220.0, font_ea, &fams, Some(&*prev)) {
                        if f == crate::widgets::ADD_FONT {
                            let _ = app.run("font.add", json!({}));
                        } else if f == crate::widgets::LOAD_SYSTEM_FONTS {
                            let _ = app.run("font.loadSystem", json!({}));
                        } else {
                            *font_ea = f;
                        }
                    }
                    ui.end_row();
                }
                ui.label(if *asian { tl!("Latin text font:") } else { tl!("Font:") });
                let fams = app.previews.families();
                if let Some(f) = crate::widgets::font_combo(ui, "fontdlg_font", 220.0, font, &fams, Some(&*prev)) {
                    if f == crate::widgets::ADD_FONT {
                        let _ = app.run("font.add", json!({}));
                    } else if f == crate::widgets::LOAD_SYSTEM_FONTS {
                        let _ = app.run("font.loadSystem", json!({}));
                    } else {
                        *font = f;
                    }
                }
                ui.end_row();
                ui.label(tl!("Size:"));
                ui.text_edit_singleline(size);
                ui.end_row();
                ui.label(tl!("Font color:"));
                let theme = app.session.doc.settings.theme_colors.clone();
                color_field(ui, &theme, color, "Automatic", "auto");
                ui.end_row();
                ui.label(tl!("Spacing (pt):"));
                ui.add(egui::DragValue::new(spacing).speed(0.1).range(-20.0..=20.0));
                ui.end_row();
                let label = tl!("Kerning for fonts:");
                match kern {
                    Some(on) => {
                        ui.checkbox(on, label);
                    }
                    None => {
                        if ui.add(egui::Checkbox::new(&mut false, label).indeterminate(true)).clicked() {
                            *kern = Some(true);
                        }
                    }
                }
                ui.horizontal(|ui| {
                    ui.add_enabled(*kern == Some(true), egui::DragValue::new(kern_at).speed(0.5).range(1.0..=1638.0).max_decimals(1));
                    ui.label(tl!("Points and above"));
                });
                ui.end_row();
            });
            ui.separator();
            ui.label(egui::RichText::new(tl!("Effects")).font(semibold(12.5)));
            egui::Grid::new("fx").num_columns(2).show(ui, |ui| {
                ui.checkbox(bold, tl!("Bold"));
                ui.checkbox(italic, tl!("Italic"));
                ui.end_row();
                ui.checkbox(underline, tl!("Underline"));
                ui.checkbox(strike, tl!("Strikethrough"));
                ui.end_row();
                ui.checkbox(sup, tl!("Superscript"));
                ui.checkbox(sub, tl!("Subscript"));
                ui.end_row();
                ui.checkbox(small_caps, tl!("Small caps"));
                ui.checkbox(caps, tl!("All caps"));
                ui.end_row();
                ui.checkbox(hidden, tl!("Hidden"));
                ui.end_row();
            });
            // Live preview of the chosen font/size/colour/effects.
            ui.separator();
            ui.label(egui::RichText::new(tl!("Preview")).small().weak());
            let sample = if *caps { tl!("AaBbYyZz").to_uppercase() } else { tl!("AaBbYyZz").to_string() };
            let mut job = egui::text::LayoutJob::single_section(sample, fmt);
            job.wrap.max_width = ui.available_width().min(400.0);
            ui.label(job);
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                apply_font(app, d);
            }
            ok || cancel
        }
        Dialog::Paragraph {
            rtl,
            align,
            left,
            right,
            first,
            before,
            after,
            line,
            keep_next,
            keep_lines,
            page_break,
            widow,
            tab,
            asian,
            opts,
            mixed,
            ..
        } => {
            let names = ["Indents and Spacing", "Line and Page Breaks", "Asian Typography"].map(|n| tl!(n).to_string());
            if let Some(k) = crate::widgets::dialog_tabs(ui, &names, *tab as usize) {
                *tab = k as u8;
            }
            ui.add_space(6.0);
            match *tab {
                0 => {
                    ui.label(egui::RichText::new(tl!("General")).font(semibold(12.5)));
                    ui.horizontal(|ui| {
                        ui.label(tl!("Direction:"));
                        // Neither while the selected paragraphs differ.
                        let blank = mixed.contains("bidi");
                        for (value, text) in [(true, tl!("Right-to-left")), (false, tl!("Left-to-right"))] {
                            if ui.radio(!blank && *rtl == value, text).clicked() {
                                *rtl = value;
                                mixed.remove("bidi");
                            }
                        }
                    });
                    const ALIGNS: [(&str, &str); 4] = [("left", "Left"), ("center", "Center"), ("right", "Right"), ("justify", "Justify")];
                    ui.horizontal(|ui| {
                        ui.label(tl!("Alignment:"));
                        let shown = if mixed.contains("align") {
                            ""
                        } else {
                            ALIGNS.iter().find(|(v, _)| v == align).map_or(align.as_str(), |(_, l)| tl!(l))
                        };
                        egui::ComboBox::from_id_salt("para_align").selected_text(shown).show_ui(ui, |ui| {
                            for (v, l) in ALIGNS {
                                if ui.selectable_value(align, v.to_string(), tl!(l)).clicked() {
                                    mixed.remove("align");
                                }
                            }
                        });
                    });
                    ui.label(egui::RichText::new(tl!("Indentation")).font(semibold(12.5)));
                    // Mirrored indents are the inside and outside ones (they swap on even pages).
                    let (l_label, r_label) = if opts[0] { ("Inside:", "Outside:") } else { ("Left:", "Right:") };
                    egui::Grid::new("ind").num_columns(4).show(ui, |ui| {
                        ui.label(tl!(l_label));
                        para_number(ui, left, ParaNumber::Length, "left", mixed);
                        ui.label(tl!(r_label));
                        para_number(ui, right, ParaNumber::Length, "right", mixed);
                        ui.end_row();
                        ui.label(tl!("First line:"));
                        para_number(ui, first, ParaNumber::Length, "indentFirst", mixed);
                        ui.end_row();
                    });
                    para_check(ui, &mut opts[0], tl!("Mirror indents"), "mirrorIndents", mixed);
                    ui.label(egui::RichText::new(tl!("Spacing")).font(semibold(12.5)));
                    egui::Grid::new("sp").num_columns(4).show(ui, |ui| {
                        ui.label(tl!("Before:"));
                        para_number(ui, before, ParaNumber::Points, "spaceBefore", mixed);
                        ui.label(tl!("Line spacing:"));
                        para_number(ui, line, ParaNumber::Lines, "lineSpacing", mixed);
                        ui.end_row();
                        ui.label(tl!("After:"));
                        para_number(ui, after, ParaNumber::Points, "spaceAfter", mixed);
                        ui.end_row();
                    });
                    para_check(ui, &mut opts[1], tl!("Don't add space between paragraphs of the same style"), "contextualSpacing", mixed);
                }
                1 => {
                    ui.label(egui::RichText::new(tl!("Line and Page Breaks")).font(semibold(12.5)));
                    para_check(ui, widow, tl!("Widow/Orphan control"), "widowControl", mixed);
                    para_check(ui, keep_next, tl!("Keep with next"), "keepNext", mixed);
                    para_check(ui, keep_lines, tl!("Keep lines together"), "keepLines", mixed);
                    para_check(ui, page_break, tl!("Page break before"), "pageBreakBefore", mixed);
                    ui.label(egui::RichText::new(tl!("Formatting exceptions")).font(semibold(12.5)));
                    para_check(ui, &mut opts[2], tl!("Suppress line numbers"), "suppressLineNumbers", mixed);
                    para_check(ui, &mut opts[3], tl!("Don't hyphenate"), "suppressHyphens", mixed);
                }
                _ => {
                    let [kinsoku, word_wrap, overflow, top_line, de, dn] = asian;
                    ui.label(egui::RichText::new(tl!("Line breaking")).font(semibold(12.5)));
                    para_check(ui, kinsoku, tl!("Apply Asian line-breaking rules (kinsoku)"), "kinsoku", mixed);
                    // The model's flag is "wrap whole words"; the box offers the opposite.
                    let mut mid_word = !*word_wrap;
                    para_check(ui, &mut mid_word, tl!("Allow Latin text to wrap in the middle of a word"), "wordWrap", mixed);
                    *word_wrap = !mid_word;
                    para_check(ui, overflow, tl!("Allow hanging punctuation"), "overflowPunct", mixed);
                    ui.label(egui::RichText::new(tl!("Character Spacing")).font(semibold(12.5)));
                    para_check(ui, top_line, tl!("Compress punctuation at the start of a line"), "topLinePunct", mixed);
                    para_check(ui, de, tl!("Add space between Asian and Latin text"), "autoSpaceDE", mixed);
                    para_check(ui, dn, tl!("Add space between Asian text and numbers"), "autoSpaceDN", mixed);
                }
            }
            ui.add_space(6.0);
            let tabs = ui.button(tl!("Tabs…")).clicked();
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok || tabs {
                apply_paragraph(app, d);
            }
            // Tabs… keeps what was set here and moves on to the Tabs dialog.
            if tabs {
                app.dialog = Dialog::open("tabs", app);
            }
            ok || cancel || tabs
        }
        Dialog::Tabs { form } => crate::dialogs_para::tabs(app, ui, form),
        Dialog::Borders { form } => crate::dialogs_para::borders(app, ui, form),
        Dialog::Find { query, replace, match_case, whole_word, regex, extra, tab, goto_what, goto_target, message, focus } => {
            const FIND_TABS: [(FindTab, &str); 3] = [(FindTab::Find, "Find"), (FindTab::Replace, "Replace"), (FindTab::GoTo, "Go To")];
            let names = FIND_TABS.map(|(_, l)| tl!(l).to_string());
            let current = FIND_TABS.iter().position(|(t, _)| t == tab).unwrap_or(0);
            if let Some((t, _)) = crate::widgets::dialog_tabs(ui, &names, current).and_then(|k| FIND_TABS.get(k)) {
                *tab = *t;
                message.clear();
            }
            ui.add_space(6.0);
            if *tab == FindTab::GoTo {
                return goto_body(app, ui, goto_what, goto_target, message);
            }
            let replace_mode = *tab == FindTab::Replace;
            ui.horizontal(|ui| {
                ui.label(tl!("Find what:"));
                let r = ui.add(egui::TextEdit::singleline(query).id(egui::Id::new("find_what")).desired_width(240.0));
                if std::mem::take(focus) {
                    r.request_focus();
                }
                if r.gained_focus() {
                    extra.to_replace = false;
                }
            });
            if replace_mode {
                ui.horizontal(|ui| {
                    ui.label(tl!("Replace with:"));
                    if ui.add(egui::TextEdit::singleline(replace).desired_width(228.0)).gained_focus() {
                        extra.to_replace = true;
                    }
                });
            } else {
                extra.to_replace = false;
            }
            // Word greys out Match case and the word options with wildcards (they always match case).
            let plain = !extra.wildcards;
            ui.horizontal(|ui| {
                ui.add_enabled(plain, egui::Checkbox::new(match_case, tl!("Match case")));
                ui.add_enabled(plain, egui::Checkbox::new(whole_word, tl!("Whole words")));
                ui.checkbox(&mut extra.wildcards, tl!("Use wildcards"));
            });
            let arrow = if extra.more { "▾" } else { "▸" };
            if ui.selectable_label(extra.more, format!("{arrow} {}", tl!("Search Options"))).clicked() {
                extra.more = !extra.more;
            }
            if extra.more {
                ui.horizontal(|ui| {
                    ui.add_enabled(plain, egui::Checkbox::new(&mut extra.prefix, tl!("Match prefix")));
                    ui.add_enabled(plain, egui::Checkbox::new(&mut extra.suffix, tl!("Match suffix")));
                });
                ui.horizontal(|ui| {
                    ui.add_enabled(plain, egui::Checkbox::new(&mut extra.ignore_punct, tl!("Ignore punctuation characters")));
                    ui.add_enabled(plain, egui::Checkbox::new(&mut extra.ignore_space, tl!("Ignore white-space characters")));
                });
                ui.horizontal(|ui| {
                    ui.checkbox(regex, tl!("Regular expressions"));
                    if let Some(code) = special_menu(ui, extra) {
                        if extra.to_replace { replace.push_str(code) } else { query.push_str(code) }
                    }
                });
            }
            let opts = json!({
                "text": query,
                "with": replace,
                "matchCase": *match_case,
                "wholeWord": *whole_word,
                "regex": *regex,
                "wildcards": extra.wildcards,
                "matchPrefix": extra.prefix,
                "matchSuffix": extra.suffix,
                "ignorePunct": extra.ignore_punct,
                "ignoreSpace": extra.ignore_space,
            });
            if !replace_mode {
                ui.horizontal(|ui| {
                    let lit = app.session.find.highlight;
                    if ui.selectable_label(lit, tl!("Reading Highlight")).clicked() {
                        let mut v = opts.clone();
                        v["highlight"] = json!(!lit);
                        match app.run("edit.advancedFind", v) {
                            Ok(r) if !lit => *message = crate::i18n::fmt(tl!("{count} items highlighted"), &[("count", &r["count"].to_string())]),
                            Ok(_) => message.clear(),
                            Err(e) => *message = e,
                        }
                    }
                });
            }
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).weak());
            }
            let mut close = false;
            ui.horizontal(|ui| {
                if replace_mode {
                    if ui.button(tl!("Replace All")).clicked() {
                        match app.run("edit.replaceAll", opts.clone()) {
                            Ok(r) => *message = format!("All done. We made {} replacements.", r["replaced"]),
                            Err(e) => *message = e,
                        }
                    }
                    if ui.button(tl!("Replace")).clicked() {
                        let _ = app.run("edit.replace", opts.clone());
                    }
                }
                if ui.button(tl!("Find Next")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    // One step per click: `edit.find` would select a match itself, and the step after it skip one.
                    match app.run("edit.findNext", opts.clone()) {
                        Ok(r) => *message = format!("Match {} of {}", r["index"].as_u64().unwrap_or(0) + 1, r["count"]),
                        Err(e) => *message = e,
                    }
                }
                if ui.button(tl!("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
            close
        }
        Dialog::Enclose { style, text, shape } => {
            ui.label(egui::RichText::new(tl!("Style")).font(semibold(12.5)));
            ui.horizontal(|ui| {
                for (v, l) in [("none", "None"), ("shrink", "Shrink text"), ("enlarge", "Enlarge symbol")] {
                    ui.radio_value(style, v.to_string(), tl!(l));
                }
            });
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.label(tl!("Text:"));
                    ui.add(egui::TextEdit::singleline(text).desired_width(110.0).char_limit(2));
                    ui.add_space(6.0);
                    enclose_preview(ui, style, text, shape);
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(tl!("Enclosure"));
                    for (v, l) in [("circle", "Circle"), ("square", "Square"), ("triangle", "Triangle"), ("diamond", "Diamond")] {
                        ui.add_enabled_ui(style != "none", |ui| ui.selectable_value(shape, v.to_string(), tl!(l)));
                    }
                });
            });
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                let params = if style == "none" { json!({"shape": null}) } else { json!({"shape": shape, "style": style, "text": text}) };
                let _ = app.run("format.enclose", params);
            }
            ok || cancel
        }
        Dialog::TableProperties { form, basis } => {
            table_properties(ui, form);
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                let changes = form.changes(basis);
                if changes.as_object().is_some_and(|m| !m.is_empty()) {
                    let _ = app.run("table.properties", changes);
                }
            }
            ok || cancel
        }
        Dialog::CellMargins { form, basis } => {
            cell_margins_dialog(ui, form);
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                let changes = form.changes(basis);
                if changes.as_object().is_some_and(|m| !m.is_empty()) {
                    let _ = app.run("table.cellMargins", changes);
                }
            }
            ok || cancel
        }
        Dialog::InsertTable { rows, cols } => {
            egui::Grid::new("it").num_columns(2).show(ui, |ui| {
                ui.label(tl!("Number of columns:"));
                ui.add(egui::DragValue::new(cols).range(1..=63));
                ui.end_row();
                ui.label(tl!("Number of rows:"));
                ui.add(egui::DragValue::new(rows).range(1..=1000));
                ui.end_row();
            });
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                let _ = app.run("insert.table", json!({"rows": *rows, "cols": *cols}));
            }
            ok || cancel
        }
        Dialog::PageSetup {
            top,
            bottom,
            left,
            right,
            landscape,
            mirror,
            rtl,
            grid,
            grid_was,
            paper,
            paper_was,
            paper_custom,
            message,
            valign,
            odd_even,
            first_page,
        } => {
            ui.label(egui::RichText::new(tl!("Margins")).font(semibold(12.5)));
            egui::Grid::new("ps").num_columns(4).show(ui, |ui| {
                ui.label(tl!("Top:"));
                ui.add(egui::DragValue::new(top).speed(3.6 / crate::options::unit().pt_per_unit()).measure(crate::options::unit()).max_decimals(2));
                ui.label(tl!("Bottom:"));
                ui.add(
                    egui::DragValue::new(bottom).speed(3.6 / crate::options::unit().pt_per_unit()).measure(crate::options::unit()).max_decimals(2),
                );
                ui.end_row();
                ui.label(if *mirror { tl!("Inside:") } else { tl!("Left:") });
                ui.add(egui::DragValue::new(left).speed(3.6 / crate::options::unit().pt_per_unit()).measure(crate::options::unit()).max_decimals(2));
                ui.label(if *mirror { tl!("Outside:") } else { tl!("Right:") });
                ui.add(egui::DragValue::new(right).speed(3.6 / crate::options::unit().pt_per_unit()).measure(crate::options::unit()).max_decimals(2));
                ui.end_row();
            });
            ui.label(egui::RichText::new(tl!("Orientation")).font(semibold(12.5)));
            ui.horizontal(|ui| {
                ui.radio_value(landscape, false, tl!("Portrait"));
                ui.radio_value(landscape, true, tl!("Landscape"));
            });
            ui.label(egui::RichText::new(tl!("Pages")).font(semibold(12.5)));
            ui.checkbox(mirror, tl!("Mirror margins"));
            page_paper_group(ui, paper, landscape, paper_custom);
            ui.label(egui::RichText::new(tl!("Document Grid")).font(semibold(12.5)));
            ui.horizontal_wrapped(|ui| {
                ui.radio_value(&mut grid.0, 0, tl!("No grid"));
                ui.radio_value(&mut grid.0, 1, tl!("Lines only"));
                ui.radio_value(&mut grid.0, 2, tl!("Lines and characters"));
                ui.radio_value(&mut grid.0, 3, tl!("Text snaps to characters"));
            });
            egui::Grid::new("ps_grid").num_columns(4).show(ui, |ui| {
                ui.add_enabled(grid.0 >= 2, egui::Label::new(tl!("Characters per line:")));
                ui.add_enabled(grid.0 >= 2, egui::DragValue::new(&mut grid.2).range(1..=1000));
                ui.add_enabled(grid.0 >= 1, egui::Label::new(tl!("Lines per page:")));
                ui.add_enabled(grid.0 >= 1, egui::DragValue::new(&mut grid.1).range(1..=1000));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.label(tl!("Section direction:"));
                ui.radio_value(rtl, false, tl!("Left-to-right"));
                ui.radio_value(rtl, true, tl!("Right-to-left"));
            });
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).small().color(ui.visuals().error_fg_color));
            }
            page_valign_combo(ui, valign);
            // Word's Page Setup › Layout: a title page without the page number (#1230).
            ui.label(egui::RichText::new(tl!("Headers and footers")).font(semibold(12.5)));
            ui.checkbox(odd_even, tl!("Different Odd & Even Pages"));
            ui.checkbox(first_page, tl!("Different First Page"));
            let (mut ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                // Everything OK changes is one undo step; a refused command applies nothing.
                let result = page_layout_commands(app, (*paper, *paper_was), *landscape, (*top, *bottom, *left, *right))
                    .map_err(|e| tl!(e).to_string())
                    .and_then(|mut commands| {
                        // Mirror margins go with the margins (#1181).
                        if let Some((_, v)) = commands.iter_mut().rev().find(|(id, _)| *id == "layout.margins") {
                            v["mirror"] = json!(*mirror);
                        }
                        if grid != grid_was {
                            let kind = ["none", "lines", "linesAndChars", "snapToChars"].get(grid.0 as usize).copied().unwrap_or("none");
                            let mut v = json!({"type": kind});
                            if grid.1 != grid_was.1 {
                                v["linesPerPage"] = json!(grid.1);
                            }
                            if grid.2 != grid_was.2 {
                                v["charsPerLine"] = json!(grid.2);
                            }
                            commands.push(("layout.documentGrid", v));
                        }
                        if *rtl != wordcraft_engine::cmd::page::sect(&app.session).rtl {
                            commands.push(("layout.sectionRtl", json!({"value": *rtl})));
                        }
                        if *valign != wordcraft_engine::cmd::page::sect(&app.session).valign {
                            commands.push(("layout.verticalAlign", json!({"value": page_valign_value(*valign)})));
                        }
                        if *odd_even != app.session.doc.settings.even_odd_headers {
                            commands.push(("layout.differentOddEven", json!({"value": *odd_even})));
                        }
                        if *first_page != wordcraft_engine::cmd::page::sect(&app.session).title_page {
                            commands.push(("layout.differentFirstPage", json!({"value": *first_page})));
                        }
                        run_as_one_step(app, commands)
                    });
                if let Err(e) = result {
                    *message = e;
                    ok = false;
                }
            }
            ok || cancel
        }
        Dialog::Link { url, text, place, targets, pick, focus } => {
            // Word's "Link to:" choices; the places are read once, when that tab is first shown.
            ui.horizontal(|ui| {
                ui.label(tl!("Link to:"));
                ui.selectable_value(place, false, tl!("Existing File or Web Page"));
                if ui.selectable_value(place, true, tl!("Place in This Document")).clicked() && targets.is_none() {
                    *targets = app.run("link.targets", json!({})).ok();
                }
            });
            ui.add_space(4.0);
            egui::Grid::new("lnk").num_columns(2).show(ui, |ui| {
                ui.label(tl!("Text to display:"));
                ui.text_edit_singleline(text);
                ui.end_row();
                if !*place {
                    ui.label(tl!("Address:"));
                    let mut out = egui::TextEdit::singleline(url).id(egui::Id::new("link_address")).show(ui);
                    if std::mem::take(focus) {
                        out.response.request_focus();
                        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(url.chars().count()));
                        out.state.cursor.set_char_range(Some(all));
                        out.state.store(ui.ctx(), out.response.id);
                    }
                    ui.end_row();
                }
            });
            if *place {
                ui.add_space(4.0);
                ui.label(tl!("Select a place in this document:"));
                let mut chosen: Option<(Value, String)> = None;
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(220.0).min_scrolled_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
                        let is = |pick: &Option<(Value, String)>, key: &str, val: &Value| pick.as_ref().is_some_and(|(v, _)| v.get(key) == Some(val));
                        let top = tl!("Top of the Document").to_string();
                        if ui.selectable_label(is(pick, "top", &json!(true)), &top).clicked() {
                            chosen = Some((json!({"top": true}), top));
                        }
                        let headings = targets.as_ref().and_then(|t| t.get("headings")).and_then(Value::as_array).cloned().unwrap_or_default();
                        if !headings.is_empty() {
                            ui.label(egui::RichText::new(tl!("Headings")).font(semibold(12.5)));
                        }
                        for h in &headings {
                            let (block, level) = (h.get("block").cloned().unwrap_or_default(), h.get("level").and_then(Value::as_u64).unwrap_or(1));
                            let name = h.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
                            ui.horizontal(|ui| {
                                ui.add_space(14.0 * level.saturating_sub(1).min(8) as f32 + 8.0);
                                if ui.selectable_label(is(pick, "heading", &block), &name).clicked() {
                                    chosen = Some((json!({"heading": block}), name));
                                }
                            });
                        }
                        let marks = targets.as_ref().and_then(|t| t.get("bookmarks")).and_then(Value::as_array).cloned().unwrap_or_default();
                        if !marks.is_empty() {
                            ui.label(egui::RichText::new(tl!("Bookmarks")).font(semibold(12.5)));
                        }
                        for m in marks.iter().filter_map(Value::as_str) {
                            ui.horizontal(|ui| {
                                ui.add_space(8.0);
                                if ui.selectable_label(is(pick, "bookmark", &json!(m)), m).clicked() {
                                    chosen = Some((json!({"bookmark": m}), m.to_string()));
                                }
                            });
                        }
                    });
                });
                if let Some(c) = chosen {
                    // The text shown follows the choice until it is typed over (as in Word).
                    if text.is_empty() || pick.as_ref().is_some_and(|(_, old)| old == text) {
                        *text = c.1.clone();
                    }
                    *pick = Some(c);
                }
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                if *place {
                    let Some((mut params, _)) = pick.clone() else { return false };
                    if !text.is_empty() {
                        params["text"] = json!(text);
                    }
                    let _ = app.run("insert.link", params);
                } else {
                    let _ = app.run("insert.link", json!({"url": url, "text": text}));
                }
            }
            ok || cancel
        }
        Dialog::Bookmark { name, by_location, hidden, deleted } => bookmark(app, ui, name, by_location, hidden, deleted),
        Dialog::WordCount { stats } => {
            egui::Grid::new("wc").num_columns(2).spacing(vec2(30.0, 6.0)).show(ui, |ui| {
                for (l, k) in [
                    ("Pages", "pages"),
                    ("Words", "words"),
                    ("Characters (no spaces)", "characters"),
                    ("Characters (with spaces)", "charactersWithSpaces"),
                    ("Paragraphs", "paragraphs"),
                    ("Lines", "lines"),
                ] {
                    ui.label(tl!(l));
                    ui.label(stats.get(k).map(|v| v.to_string()).unwrap_or_default());
                    ui.end_row();
                }
            });
            let mut include = stats.get("includeTextBoxes").and_then(Value::as_bool).unwrap_or(true);
            if ui.checkbox(&mut include, tl!("Count words in text boxes and notes")).changed()
                && let Ok(v) = app.run("review.wordCount", json!({"includeTextBoxes": include}))
            {
                *stats = v;
            }
            let (ok, cancel) = buttons(ui, tl!("Close"));
            ok || cancel
        }
        Dialog::Zoom { percent } => {
            ui.horizontal(|ui| {
                for p in [200.0, 100.0, 75.0] {
                    ui.radio_value(percent, p, format!("{p}%"));
                }
            });
            // Logarithmic like the status bar slider, and wide enough to aim: the default 100 pt
            // linear slider jumped from 100% to 480% within a short drag (issue #67).
            ui.spacing_mut().slider_width = 220.0;
            ui.add(egui::Slider::new(percent, 10.0..=500.0).logarithmic(true).step_by(1.0).suffix("%"));
            let mut fit = None;
            ui.horizontal(|ui| {
                if ui.button(tl!("Page width")).clicked() {
                    fit = Some("pageWidth");
                }
                if ui.button(tl!("Whole page")).clicked() {
                    fit = Some("onePage");
                }
                if ui.button(tl!("Many pages")).clicked() {
                    fit = Some("multiplePages");
                }
            });
            if let Some(f) = fit {
                let _ = app.run("view.zoom", json!({"value": f}));
                return true;
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                let _ = app.run("view.zoom", json!({"value": *percent}));
            }
            ok || cancel
        }
        Dialog::Watermark { text, diagonal } => {
            ui.horizontal(|ui| {
                ui.label(tl!("Text:"));
                ui.text_edit_singleline(text);
            });
            ui.horizontal(|ui| {
                ui.radio_value(diagonal, true, tl!("Diagonal"));
                ui.radio_value(diagonal, false, tl!("Horizontal"));
            });
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                let _ = app.run("design.watermark", json!({"text": text, "diagonal": *diagonal}));
            }
            ok || cancel
        }
        Dialog::Phonetic { base, ruby, align, size, offset, existing, focus } => {
            // Base and ruby text are usually Chinese or Japanese: fonts for them load from the next frame.
            app.want_system_cjk = true;
            const ALIGNS: [(&str, &str); 6] = [
                ("center", "Center"),
                ("distributeLetter", "Distribute letters"),
                ("distributeSpace", "Distribute spacing"),
                ("left", "Left"),
                ("right", "Right"),
                ("rightVertical", "Right (vertical)"),
            ];
            egui::Grid::new("phonetic").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tl!("Base text:"));
                ui.label(egui::RichText::new(base.as_str()).strong());
                ui.end_row();
                ui.label(tl!("Ruby text:"));
                let r = ui.text_edit_singleline(ruby);
                // Typing goes to the Ruby text box, not the selected text in the document (#3914).
                if std::mem::take(focus) {
                    r.request_focus();
                }
                ui.end_row();
                ui.label(tl!("Alignment:"));
                let shown = ALIGNS.iter().find(|(k, _)| k == align).map_or("Center", |(_, l)| l);
                egui::ComboBox::from_id_salt("phonetic_align").selected_text(tl!(shown)).show_ui(ui, |ui| {
                    for (k, l) in ALIGNS {
                        ui.selectable_value(align, k.to_string(), tl!(l));
                    }
                });
                ui.end_row();
                ui.label(tl!("Offset (pt):"));
                ui.add(egui::DragValue::new(offset).speed(0.5).range(-100.0..=100.0));
                ui.end_row();
                ui.label(tl!("Size (pt):"));
                ui.add(egui::DragValue::new(size).speed(0.5).range(1.0..=200.0));
                ui.end_row();
            });
            let mut remove = false;
            if *existing {
                ui.add_space(4.0);
                remove = ui.button(tl!("Remove")).clicked();
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if remove {
                let _ = app.run("format.phonetic", json!({"ruby": null}));
            } else if ok {
                let _ = app.run("format.phonetic", json!({"ruby": ruby, "align": align, "size": *size, "offset": *offset}));
            }
            ok || cancel || remove
        }
        Dialog::NewStyle { name, based_on, back } => {
            egui::Grid::new("ns").num_columns(2).show(ui, |ui| {
                ui.label(tl!("Name:"));
                ui.text_edit_singleline(name);
                ui.end_row();
                ui.label(tl!("Style based on:"));
                ui.text_edit_singleline(based_on);
                ui.end_row();
            });
            ui.label(egui::RichText::new(tl!("The new style takes the formatting of the current paragraph.")).small().weak());
            let (ok, cancel) = buttons(ui, tl!("OK"));
            let mut created = None;
            if ok {
                created =
                    app.run("styles.create", json!({"name": name, "basedOn": based_on})).ok().and_then(|v| v.get("id")?.as_str().map(str::to_string));
            }
            if (ok || cancel) && *back {
                app.dialog = Some(Dialog::ManageStyles { alphabetical: false, selected: created.unwrap_or_default() });
            }
            ok || cancel
        }
        Dialog::ModifyStyle { back, resume, id, name, font, size, bold, italic, color, before, after } => {
            egui::Grid::new("ms").num_columns(2).show(ui, |ui| {
                ui.label(tl!("Name:"));
                ui.text_edit_singleline(name);
                ui.end_row();
                ui.label(tl!("Font:"));
                let shown = app.previews.font_shown(font);
                let fams = app.previews.families();
                let prev = app.previews.font_preview_fn();
                if let Some(f) = crate::widgets::font_combo(ui, "ms_font", 220.0, &shown, &fams, Some(&*prev)) {
                    if f == crate::widgets::ADD_FONT {
                        let _ = app.run("font.add", json!({}));
                    } else {
                        *font = app.previews.font_stored(&f);
                    }
                }
                ui.end_row();
                ui.label(tl!("Size:"));
                ui.add(egui::DragValue::new(size).range(1.0..=1638.0).speed(0.5));
                ui.end_row();
                ui.label(tl!("Color:"));
                ui.text_edit_singleline(color);
                ui.end_row();
                ui.label(tl!("Space before/after:"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(before).range(0.0..=1584.0));
                    ui.add(egui::DragValue::new(after).range(0.0..=1584.0));
                });
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.checkbox(bold, tl!("Bold"));
                ui.checkbox(italic, tl!("Italic"));
            });
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                let mut chr = json!({"font": font, "size": *size, "bold": *bold, "italic": *italic});
                if let Some(c) = wordcraft_doc::Rgb::parse(color) {
                    chr["color"] = json!({"Rgb": [c.0, c.1, c.2]});
                }
                let _ =
                    app.run("styles.modify", json!({"style": id, "name": name, "chr": chr, "para": {"spaceBefore": *before, "spaceAfter": *after}}));
            }
            if (ok || cancel) && *back {
                app.dialog = Some(Dialog::ManageStyles { alphabetical: false, selected: id.clone() });
            }
            if (ok || cancel)
                && let Some(r) = resume.take()
            {
                app.dialog = Some(*r);
            }
            ok || cancel
        }
        Dialog::ManageStyles { alphabetical, selected } => manage_styles(app, ui, alphabetical, selected),
        Dialog::TableStyle { id, name, based_on, region, regions, basis, pen } => {
            crate::dialogs_table_style::body(app, ui, crate::dialogs_table_style::Form { id, name, based_on, region, regions, basis, pen })
        }
        Dialog::Insert(f) => crate::dialogs_insert::body(app, ui, f),
        Dialog::Refs(f) => crate::dialogs_refs::body(app, ui, f),
        Dialog::Commands { query, swallow } => {
            if *swallow {
                ui.ctx().input_mut(|i| i.events.retain(|e| !swallow_shortcut_text(swallow, e)));
            }
            let r = ui.add(egui::TextEdit::singleline(query).hint_text(tl!("Type a command, e.g. \"insert table\"")).desired_width(380.0));
            // Only when it lacks focus: `request_focus` always interrupts IME, and on X11 that
            // recreates the XIM context, so asking every frame left fcitx taking no keys (#2986).
            if !r.has_focus() {
                r.request_focus();
            }
            let reg = app.session.registry.clone();
            // An empty box suggests a few common commands, as Word's Search does.
            let mut hits: Vec<&wordcraft_engine::CommandSpec> = if query.trim().is_empty() {
                ui.label(egui::RichText::new(tl!("Suggested")).font(semibold(12.5)));
                SEARCH_SUGGESTIONS.iter().filter_map(|id| reg.get(id)).collect()
            } else {
                reg.all().iter().filter(|c| command_matches(c, query)).collect()
            };
            hits.truncate(14);
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let mut run: Option<&str> = enter.then(|| hits.first().map(|c| c.id)).flatten();
            for c in &hits {
                let sc = crate::widgets::shortcut_text(app, c.id);
                let label = format!("{}   —   {}", tl!(c.label), crate::i18n::location(c.location));
                if ui.add(egui::Button::new(label).shortcut_text(sc).min_size(vec2(380.0, 0.0))).clicked() {
                    run = Some(c.id);
                }
            }
            // Search also finds the words in the document (the Navigation pane's results).
            let text = query.trim().to_string();
            let mut find = false;
            if !text.is_empty() {
                ui.separator();
                let label = crate::i18n::fmt(tl!("Find “{text}” in the document"), &[("text", &text)]);
                find = ui.add(egui::Button::new(label).min_size(vec2(380.0, 0.0))).clicked() || (enter && hits.is_empty());
            }
            if let Some(id) = run {
                let _ = app.run(id, json!({}));
            } else if find {
                find_in_navigation_pane(app, ui.ctx(), &text);
            }
            run.is_some() || find || ui.input(|i| i.key_pressed(egui::Key::Escape))
        }
        Dialog::Icons { query, category, selected } => icons_body(app, ui, query, category, selected),
        Dialog::PasteSpecial { formats, choice, payload, note } => {
            ui.label(egui::RichText::new(tl!("Paste as:")).font(semibold(12.5)));
            if formats.is_empty() {
                // The engine's wording, sentence-cased; the format names in it aren't translated.
                let text = if note.is_empty() || note == "the clipboard is empty" {
                    tl!("The clipboard is empty.").to_string()
                } else {
                    let mut c = note.chars();
                    c.next().map(|f| f.to_uppercase().chain(c).collect::<String>() + ".").unwrap_or_default()
                };
                ui.label(egui::RichText::new(text).weak());
            }
            for (i, f) in formats.iter().enumerate() {
                let label = wordcraft_engine::cmd::paste::FORMATS.iter().find(|(id, _)| id == f).map(|(_, l)| *l).unwrap_or(f.as_str());
                ui.radio_value(choice, i, tl!(label));
            }
            if let Some(f) = formats.get(*choice) {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl!("Result")).font(semibold(12.5)));
                let hint = match f.as_str() {
                    "formatted" => "Keeps the fonts and formatting of the copied text.",
                    "rtf" => "Reads the clipboard's rich text and keeps its formatting.",
                    "html" => "Reads the clipboard's web content and keeps its formatting.",
                    "picture" => "Pastes the clipboard's picture.",
                    _ => "Drops all formatting and pastes plain text.",
                };
                ui.label(egui::RichText::new(tl!(hint)).weak());
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok && let Some(f) = formats.get(*choice) {
                let mut params = payload.clone();
                params["as"] = json!(f);
                let _ = app.run("edit.pasteSpecial", params);
            }
            ok || cancel
        }
        Dialog::About { tab } => {
            ui.set_width(660.0);
            let names = ["About", "Contributors", "Models"].map(|n| tl!(n).to_string());
            if let Some(i) = crate::widgets::dialog_tabs(ui, &names, *tab as usize) {
                *tab = i as u8;
            }
            ui.add_space(6.0);
            match *tab {
                1 => crate::credits::contributors_ui(ui),
                2 => crate::credits::models_ui(ui),
                _ => {
                    ui.label(egui::RichText::new(tl!("WordCraft")).font(semibold(22.0)));
                    let build = option_env!("WORDCRAFT_BUILD_SHA").map(|s| s.get(..8).unwrap_or(s)).unwrap_or(tl!("development build"));
                    ui.label(crate::i18n::fmt(tl!("Version {version} ({build})"), &[("version", env!("CARGO_PKG_VERSION")), ("build", build)]));
                    ui.label(tl!(
                        "A free, open-source word processor written from scratch in Rust.\nPart of the Crafting Apps from the ArtCraft team."
                    ));
                    ui.add_space(6.0);
                    ui.hyperlink_to(tl!("getartcraft.com/apps/wordcraft"), "https://getartcraft.com/apps/wordcraft");
                    ui.hyperlink_to(tl!("Join us on Discord: discord.gg/artcraft"), "https://discord.gg/artcraft");
                    ui.hyperlink_to(tl!("Source code: github.com/storytold/wordcraft"), "https://github.com/storytold/wordcraft");
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(tl!("MIT OR Apache-2.0. Copyright (c) 2026 ArtCraft Team and the WordCraft contributors."))
                            .small()
                            .weak(),
                    );
                }
            }
            let (ok, cancel) = buttons(ui, tl!("Close"));
            ok || cancel
        }
        Dialog::DiscardRecovery { name, .. } => {
            ui.label(crate::i18n::fmt(tl!("Discard the recovery copy of {name}?"), &[("name", name)]));
            ui.label(tl!("This cannot be undone. Your original file stays unchanged."));
            let answer = ui
                .horizontal(|ui| {
                    let cancel = ui.button(tl!("Cancel"));
                    let discard = ui.button(tl!("Discard"));
                    if cancel.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        Some("cancel")
                    } else if discard.clicked() {
                        Some("discard")
                    } else {
                        None
                    }
                })
                .inner;
            if let Some(answer) = answer {
                app.dialog = Some(d.clone());
                let result = app.run("ui.recoveryDiscard", json!({"answer": answer}));
                if let Err(e) = &result {
                    app.status(e.clone());
                }
                return result.is_ok();
            }
            false
        }
        Dialog::SaveChanges { name, message, .. } => {
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Do you want to save changes to {name}?"), &[("name", name)])).font(semibold(15.0)));
            ui.label(tl!("Your changes will be lost if you don't save them."));
            // Why the last Save failed (#1445): the prompt came back so the user can't miss it.
            if !message.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(message.as_str()).color(ui.visuals().error_fg_color));
            }
            ui.add_space(8.0);
            let buttons = ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let cancel = ui.button(tl!("Cancel"));
                    let dont_save = ui.button(tl!("Don't Save"));
                    let save = ui.add(crate::widgets::primary_button(ui, tl!("Save")));
                    [(cancel, "cancel"), (dont_save, "dontSave"), (save, "save")]
                })
                .inner
            });
            let buttons = buttons.inner;
            // The keyboard alone answers, as in Word (#861): Save has focus when the prompt opens,
            // so Enter saves, and the arrow keys (egui's focus navigation) or Tab move between the
            // buttons, shown as Save, Don't Save, Cancel. The focus is taken from whatever held it,
            // not only from nothing: the page keeps it while typing and claims Tab and the arrows
            // (#1597).
            if !buttons.iter().any(|(b, _)| b.has_focus())
                && let Some((save, _)) = buttons.get(2)
            {
                save.request_focus();
            }
            // Enter clicks the focused button; it means Save only when none of them has focus.
            let focused = buttons.iter().any(|(b, _)| b.has_focus());
            let answer = if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                Some("cancel")
            } else if let Some((_, a)) = buttons.iter().find(|(b, _)| b.clicked()) {
                Some(*a)
            } else if !focused && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                Some("save")
            } else {
                None
            };
            let Some(answer) = answer else { return false };
            // `ui.saveChanges` reads the prompt from `app.dialog`, which `show` has taken out.
            app.dialog = Some(d.clone());
            let _ = app.run("ui.saveChanges", json!({"answer": answer}));
            true
        }
        Dialog::RecipientList { fields, rows, message, filter } => recipient_list(app, ui, fields, rows, message, filter),
        Dialog::InsertMergeField { fields, field } => {
            if fields.is_empty() {
                ui.label(egui::RichText::new(tl!("Select recipients first to choose from their fields.")).small().weak());
            } else {
                ui.label(tl!("Fields:"));
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for f in fields.iter() {
                        let resp = ui.selectable_label(field == f, f);
                        if resp.clicked() {
                            field.clone_from(f);
                        }
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label(tl!("Field:"));
                ui.text_edit_singleline(field);
            });
            let (ok, cancel) = buttons(ui, tl!("Insert"));
            if ok && !field.trim().is_empty() {
                let _ = app.run("mailings.insertField", json!({"field": field.trim()}));
                return true;
            }
            cancel
        }
        Dialog::Password { name, params, password, message } => {
            ui.label(crate::i18n::fmt(tl!("{name} is protected with a password."), &[("name", name)]));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(tl!("Password:"));
                let r = ui.add(egui::TextEdit::singleline(password.as_mut_string()).password(true).desired_width(220.0));
                if password.is_empty() && !r.has_focus() {
                    r.request_focus();
                }
            });
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).small().color(ui.visuals().error_fg_color));
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok && !password.is_empty() {
                let mut p = params.clone();
                if let Some(o) = p.as_object_mut() {
                    o.insert("password".into(), json!(password.as_str()));
                }
                // `execute`, not `run`: Save Changes was asked when the open began.
                match app.execute("file.open", p) {
                    Ok(_) => {
                        app.ui.backstage = false;
                        return true;
                    }
                    Err(e) if wordcraft_engine::io::wrong_password(&e) => {
                        *message = tl!("That password isn't right. Check it (passwords are case-sensitive) and try again.").to_string();
                        password.as_mut_string().clear();
                    }
                    Err(e) => *message = e,
                }
            }
            cancel
        }
        Dialog::EncryptPassword { password, confirm, message } => {
            ui.label(tl!("Anyone who opens the document will need this password."));
            ui.add_space(4.0);
            egui::Grid::new("encrypt_password").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tl!("Password:"));
                let r = ui.add(egui::TextEdit::singleline(password.as_mut_string()).password(true).desired_width(220.0));
                if password.is_empty() && confirm.is_empty() && !r.has_focus() {
                    r.request_focus();
                }
                ui.end_row();
                ui.label(tl!("Confirm password:"));
                ui.add(egui::TextEdit::singleline(confirm.as_mut_string()).password(true).desired_width(220.0));
                ui.end_row();
            });
            ui.label(
                egui::RichText::new(tl!(
                    "Keep the password somewhere safe: without it nobody, WordCraft included, can open the document. Leave both boxes empty to remove the password."
                ))
                .small()
                .weak(),
            );
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).small().color(ui.visuals().error_fg_color));
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok {
                if password != confirm {
                    *message = tl!("The passwords don't match.").to_string();
                    confirm.as_mut_string().clear();
                    return false;
                }
                let pw = if password.is_empty() { Value::Null } else { json!(password.as_str()) };
                match app.run("file.encrypt", json!({"password": pw})) {
                    Ok(_) => {
                        app.status(if password.is_empty() {
                            tl!("The password is removed. Save the document to keep the change.")
                        } else {
                            tl!("The document will be saved with a password. Save it to protect it.")
                        });
                        return true;
                    }
                    Err(e) => *message = e,
                }
            }
            cancel
        }
        Dialog::DefineList { form } => crate::dialogs_lists::define_list(app, ui, form),
        Dialog::TrackOptions { form } => crate::dialogs_lists::track_options(app, ui, form),
        Dialog::Review(d) => crate::dialogs_review::body(app, ui, d),
        Dialog::CustomizeKeyboard { form } => crate::dialogs_keyboard::customize_keyboard(app, ui, form),
        Dialog::CustomizeRibbon { form } => crate::dialogs_ribbon::customize_ribbon(app, ui, form),
        Dialog::FieldCode { code, edit, message } => {
            ui.label(tl!("Field codes:"));
            let r = ui.add(egui::TextEdit::singleline(code).desired_width(320.0).font(egui::TextStyle::Monospace));
            if !r.has_focus() && code.is_empty() {
                r.request_focus();
            }
            ui.label(egui::RichText::new("PAGE · NUMPAGES · DATE \\@ \"MMMM d, yyyy\" · AUTHOR · TITLE · SEQ Figure").small().weak());
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).small().weak());
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok && !code.trim().is_empty() {
                let r = if *edit {
                    app.run("fields.setCode", json!({"code": code.trim()}))
                } else {
                    app.run("insert.field", json!({"instr": code.trim()}))
                };
                match r {
                    Ok(_) => return true,
                    Err(e) => *message = e,
                }
            }
            cancel
        }
        Dialog::Control { form } => crate::dialogs_controls::body(app, ui, form),
        Dialog::SelectTable { sheets, selected, headers, message, params } => {
            ui.label(tl!("Choose the sheet that holds the recipients."));
            ui.add_space(4.0);
            // A double-click picks the sheet as OK does.
            let mut picked = false;
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(300.0);
                egui::ScrollArea::vertical().id_salt("select_table").max_height(220.0).show(ui, |ui| {
                    for (i, name) in sheets.iter().enumerate() {
                        let r = ui.selectable_label(*selected == i, name.as_str());
                        if r.clicked() || r.double_clicked() {
                            *selected = i;
                        }
                        picked |= r.double_clicked();
                    }
                });
            });
            ui.checkbox(headers, tl!("First row of data contains column headers"));
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).color(Tokens::get(ui.ctx()).red));
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok || picked {
                let mut p = params.clone();
                if let Some(o) = p.as_object_mut() {
                    o.insert("sheet".into(), json!(*selected + 1));
                    o.insert("headers".into(), json!(*headers));
                }
                match app.load_recipients(p) {
                    Ok(_) => return true,
                    Err(e) => *message = e,
                }
            }
            cancel
        }
        Dialog::NoteOptions { form } => crate::dialogs_notes::note_options(app, ui, form),
        Dialog::Templates { form } => crate::dialogs_templates::templates(app, ui, form),
        Dialog::Organizer { form } => crate::dialogs_templates::organizer(app, ui, form),
        Dialog::Layout(d) => crate::dialogs_layout::body(app, ui, d),
        Dialog::Themes(d) => crate::dialogs_themes::body(app, ui, d),
        Dialog::FindRecipient { text, message } => {
            ui.horizontal(|ui| {
                ui.label(tl!("Find:"));
                ui.text_edit_singleline(text);
            });
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).small().weak());
            }
            let (ok, cancel) = buttons(ui, tl!("Find"));
            if ok && !text.trim().is_empty() {
                match app.run("mailings.findRecipient", json!({"text": text.trim()})) {
                    Ok(_) => return true,
                    Err(e) => *message = e,
                }
            }
            cancel
        }
        Dialog::MergeRecords { print, choice, from, to, message } => {
            ui.radio_value(choice, 0, tl!("All"));
            ui.radio_value(choice, 1, tl!("Current record"));
            ui.horizontal(|ui| {
                ui.radio_value(choice, 2, tl!("From:"));
                // Typing a number picks the range, as in Word.
                if ui.add(egui::TextEdit::singleline(from).desired_width(60.0)).changed() {
                    *choice = 2;
                }
                ui.label(tl!("To:"));
                if ui.add(egui::TextEdit::singleline(to).desired_width(60.0)).changed() {
                    *choice = 2;
                }
            });
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).small().weak());
            }
            let (ok, cancel) = buttons(ui, tl!(if *print { "Print" } else { "OK" }));
            if ok {
                let params = match *choice {
                    0 => Some(json!({"records": "all"})),
                    1 => Some(json!({"records": "current"})),
                    _ => match (from.trim().parse::<u64>(), to.trim().parse::<u64>()) {
                        (Ok(f), Ok(t)) => Some(json!({"from": f, "to": t})),
                        _ => None,
                    },
                };
                let id = if *print { "mailings.printDocuments" } else { "mailings.finish" };
                // An empty range is reported here, before Save Changes asks about the document.
                let run = params.ok_or_else(|| tl!("Enter a number.").to_string()).and_then(|p| {
                    wordcraft_engine::cmd::mailings::merge_range(&app.session, &p).map_err(|e| e.to_string())?;
                    app.run(id, p)
                });
                match run {
                    Ok(_) => return true,
                    Err(e) => *message = e,
                }
            }
            cancel
        }
        Dialog::MergeRule { rule, fields, field, value, then, els } => {
            let skip = rule == "SKIPIF";
            ui.label(tl!(if skip {
                "Leave a recipient out of the merge when a field has this value."
            } else {
                "Insert one text or another, depending on a field's value."
            }));
            ui.add_space(4.0);
            egui::Grid::new("merge_rule").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label(tl!("Field:"));
                if fields.is_empty() {
                    ui.text_edit_singleline(field);
                } else {
                    egui::ComboBox::from_id_salt("merge_rule_field").selected_text(field.as_str()).width(200.0).show_ui(ui, |ui| {
                        for f in fields.iter() {
                            ui.selectable_value(field, f.clone(), f);
                        }
                    });
                }
                ui.end_row();
                ui.label(tl!("Equals:"));
                ui.text_edit_singleline(value);
                ui.end_row();
                if !skip {
                    ui.label(tl!("Then insert:"));
                    ui.text_edit_singleline(then);
                    ui.end_row();
                    ui.label(tl!("Otherwise insert:"));
                    ui.text_edit_singleline(els);
                    ui.end_row();
                }
            });
            let (ok, cancel) = buttons(ui, tl!("OK"));
            let params = rule_params(d);
            if ok && let Some(params) = params {
                let _ = app.run("mailings.rules", params);
                return true;
            }
            cancel
        }
        Dialog::MatchFields { fields, address } => {
            if fields.is_empty() {
                ui.label(egui::RichText::new(tl!("Select recipients first to match their fields.")).small().weak());
            } else {
                ui.label(tl!("The recipient-list field that fills each part of an address block and greeting line."));
                ui.add_space(4.0);
                egui::Grid::new("match_fields").num_columns(2).spacing(vec2(24.0, 6.0)).striped(true).show(ui, |ui| {
                    for (part, key) in ADDRESS_PARTS {
                        ui.label(tl!(part));
                        match address.get(key).and_then(Value::as_str) {
                            Some(f) => ui.label(egui::RichText::new(f).strong()),
                            None => ui.label(egui::RichText::new(tl!("(not matched)")).weak()),
                        };
                        ui.end_row();
                    }
                });
            }
            let (ok, cancel) = buttons(ui, tl!("Close"));
            ok || cancel
        }
        Dialog::CheckErrors { unknown, records } => {
            if unknown.is_empty() {
                ui.label(tl!("No errors found: every merge field is in the recipient list."));
            } else {
                ui.label(tl!("These merge fields aren't in the recipient list:"));
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for f in unknown.iter() {
                        ui.label(egui::RichText::new(format!("• {f}")).color(Tokens::get(ui.ctx()).red));
                    }
                });
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Recipients: {count}"), &[("count", &records.to_string())])).weak());
            let (ok, cancel) = buttons(ui, tl!("Close"));
            ok || cancel
        }
    }
}

/// The Recipient List dialog's body: a file button, the editable table, OK/Cancel. True to close.
pub(crate) fn recipient_list(
    app: &mut WordApp,
    ui: &mut Ui,
    fields: &mut Vec<String>,
    rows: &mut Vec<Vec<String>>,
    message: &mut String,
    filter: &mut RecipientFilter,
) -> bool {
    const CELL_W: f32 = 120.0;
    const X_W: f32 = 18.0;
    const ROW_H: f32 = 24.0;
    const NUM_W: f32 = 34.0;
    // Each record's place in the merge (None: filtered out), when there is a filter or sort.
    let order: Option<Vec<Option<usize>>> = (!filter.is_empty()).then(|| filter.order(fields, rows).to_vec());
    // Wide enough for every field (up to a limit; more scroll sideways).
    let gap = ui.spacing().item_spacing.x;
    // Plus the rows' scroll bar, so each row's remove button stays in view (#1576).
    let scroll = ui.spacing().scroll;
    let bar = scroll.bar_width + scroll.bar_inner_margin + scroll.bar_outer_margin;
    let table_w = NUM_W + gap + fields.len() as f32 * (CELL_W + X_W + 2.0 * gap) + X_W + 2.0 * gap + bar + X_W;
    ui.set_min_width(table_w.clamp(340.0, 720.0));
    ui.label(tl!("Type the recipients, or load them from a file."));
    if ui.button(tl!("Use an Existing List…")).clicked() {
        // The picker takes over; the list it loads replaces this one.
        let _ = app.run("ui.openRecipientList", json!({}));
        return true;
    }
    ui.add_space(6.0);
    // Find in list and the merged-only view (#1576) hide rows from the grid, never change them.
    // Nothing to leave out without a filter: the box shows unticked.
    if order.is_none() {
        filter.only_merged = false;
    }
    // A grid cell still has the keyboard: its rows stay put (see `hold_rows`).
    let editing = filter.editing.is_some_and(|id| ui.memory(|m| m.has_focus(id)));
    if !editing {
        filter.editing = None;
    }
    let merged_only = egui::Checkbox::new(&mut filter.only_merged, tl!("Show only records that will be merged"));
    ui.add_enabled(order.is_some(), merged_only);
    let visible = ui
        .horizontal(|ui| {
            ui.label(tl!("Find in list:"));
            ui.add(egui::TextEdit::singleline(&mut filter.find).desired_width(180.0).hint_text(tl!("Any field")));
            // The rows the grid shows, by real index: edits and removes go to the right record.
            let fresh = visible_recipients(rows, order.as_deref(), &filter.find, filter.only_merged);
            let visible = hold_rows(fresh, &mut filter.held, editing, rows.len());
            if !filter.find.trim().is_empty() {
                let count = visible.len().to_string();
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Matches: {count}"), &[("count", &count)])).weak());
            }
            visible
        })
        .inner;
    ui.add_space(4.0);
    let mut remove_field = None;
    let can_remove = fields.len() > 1;
    let mut remove_row = None;
    let mut x_w = X_W;
    let mut focused = None;
    egui::ScrollArea::horizontal().id_salt("recipients_h").max_width(720.0).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.allocate_exact_size(vec2(NUM_W, ROW_H - 4.0), Sense::hover());
                for (i, f) in fields.iter_mut().enumerate() {
                    ui.add_sized([CELL_W, ROW_H - 4.0], egui::TextEdit::singleline(f).font(egui::TextStyle::Button).hint_text(tl!("Field name")));
                    let x = ui.add_enabled(can_remove, egui::Button::new("×").small().min_size(vec2(X_W, X_W))).on_hover_text(tl!("Remove field"));
                    x_w = x.rect.width();
                    if x.clicked() {
                        remove_field = Some(i);
                    }
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("recipients_v").max_height(260.0).show_rows(ui, ROW_H, visible.len(), |ui, range| {
                for k in range {
                    let Some(&r) = visible.get(k) else { continue };
                    let Some(row) = rows.get_mut(r) else { continue };
                    // Striped rows, like a data grid: painted under the row once its size is known.
                    let stripe = ui.painter().add(egui::Shape::Noop);
                    let line = ui.horizontal(|ui| {
                        // The record's number in the merge, a dash when the filter leaves it out.
                        let (num, kept) = match order.as_ref().map(|o| o.get(r).copied().flatten()) {
                            None => ((r + 1).to_string(), true),
                            Some(Some(n)) => (n.to_string(), true),
                            Some(None) => ("–".to_string(), false),
                        };
                        let num = egui::RichText::new(num).weak();
                        let (rect, _) = ui.allocate_exact_size(vec2(NUM_W, ROW_H - 4.0), Sense::hover());
                        ui.put(rect, egui::Label::new(if kept { num } else { num.strikethrough() }));
                        for (j, c) in row.iter_mut().enumerate() {
                            // Keyed on the record, not its place in the grid: when the view hides
                            // or moves rows, the keyboard can't land in another record (#1576).
                            let cell = ui.add_sized([CELL_W, ROW_H - 4.0], egui::TextEdit::singleline(c).id_salt(("recipient_cell", r, j)));
                            if cell.has_focus() {
                                focused = Some(cell.id);
                            }
                            // Under the field's remove button.
                            ui.allocate_exact_size(vec2(x_w, X_W), Sense::hover());
                        }
                        if ui.add(egui::Button::new("×").small().min_size(vec2(X_W, X_W))).on_hover_text(tl!("Remove recipient")).clicked() {
                            remove_row = Some(r);
                        }
                    });
                    if k % 2 == 1 {
                        let half = ui.spacing().item_spacing.y / 2.0;
                        let rect = line.response.rect.expand2(vec2(0.0, half));
                        ui.painter().set(stripe, egui::Shape::rect_filled(rect, 2.0, ui.visuals().faint_bg_color));
                    }
                }
            });
        });
    });
    if focused.is_some() {
        filter.editing = focused;
    }
    if remove_field.is_some() || remove_row.is_some() {
        // Row numbers move: hold nothing, and let go of the focused cell (its id names a row).
        filter.held = None;
        if let Some(id) = filter.editing.take() {
            ui.memory_mut(|m| m.surrender_focus(id));
        }
    }
    if let Some(i) = remove_field.filter(|_| fields.len() > 1) {
        fields.remove(i);
        for r in rows.iter_mut() {
            if i < r.len() {
                r.remove(i);
            }
        }
    }
    if let Some(r) = remove_row.filter(|r| *r < rows.len()) {
        rows.remove(r);
    }
    ui.horizontal(|ui| {
        if ui.button(tl!("New Entry")).clicked() {
            // The view would hide the new, empty entry.
            filter.show_all();
            rows.push(vec![String::new(); fields.len()]);
        }
        if ui.button(tl!("Add Field")).clicked() {
            fields.push(String::new());
            for r in rows.iter_mut() {
                r.push(String::new());
            }
        }
    });
    ui.add_space(4.0);
    let toggle = egui::Button::new(tl!("Filter and Sort")).selected(filter.open);
    if ui.add(toggle).clicked() {
        filter.open = !filter.open;
    }
    if filter.open {
        recipient_filter(ui, fields, filter);
    }
    if let Some(order) = &order {
        let count = order.iter().flatten().count().to_string();
        let total = rows.len().to_string();
        ui.label(
            egui::RichText::new(crate::i18n::fmt(tl!("{count} of {total} recipients will be merged."), &[("count", &count), ("total", &total)]))
                .weak(),
        );
    }
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).color(Tokens::get(ui.ctx()).red));
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if !ok {
        return cancel;
    }
    let r = recipient_table(fields, rows).map_err(|e| tl!(e).to_string()).and_then(|params| app.load_recipients(params));
    // A new list starts unfiltered; the dialog's filter and sort apply to it.
    let r = r.and_then(|_| if filter.is_empty() { Ok(Value::Null) } else { app.run("mailings.filterRecipients", filter.params(fields)) });
    match r {
        Ok(_) => true,
        Err(e) => {
            *message = e;
            false
        }
    }
}

/// Filter and Sort: condition rows (And/Or, field, comparison, value) and three sort keys.
fn recipient_filter(ui: &mut Ui, fields: &[String], filter: &mut RecipientFilter) {
    let names: Vec<String> = fields.iter().map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect();
    let field_combo = |ui: &mut Ui, id: (&str, usize), value: &mut String, none: bool| {
        let shown = if value.is_empty() { tl!("(none)").to_string() } else { value.clone() };
        egui::ComboBox::from_id_salt(id).width(130.0).selected_text(shown).show_ui(ui, |ui| {
            if none {
                ui.selectable_value(value, String::new(), tl!("(none)"));
            }
            for n in &names {
                ui.selectable_value(value, n.clone(), n.as_str());
            }
        });
    };
    let mut remove = None;
    egui::Grid::new("recipient_filter").num_columns(5).spacing(vec2(6.0, 4.0)).show(ui, |ui| {
        ui.label("");
        ui.label(egui::RichText::new(tl!("Field")).weak());
        ui.label(egui::RichText::new(tl!("Comparison")).weak());
        ui.label(egui::RichText::new(tl!("Compare to")).weak());
        ui.end_row();
        for (i, r) in filter.rules.iter_mut().enumerate() {
            if i == 0 {
                ui.label("");
            } else {
                egui::ComboBox::from_id_salt(("rf_join", i)).width(60.0).selected_text(tl!(if r.or { "Or" } else { "And" })).show_ui(ui, |ui| {
                    ui.selectable_value(&mut r.or, false, tl!("And"));
                    ui.selectable_value(&mut r.or, true, tl!("Or"));
                });
            }
            field_combo(ui, ("rf_field", i), &mut r.field, false);
            egui::ComboBox::from_id_salt(("rf_op", i)).width(150.0).selected_text(tl!(r.op.label())).show_ui(ui, |ui| {
                for op in FilterOp::ALL {
                    ui.selectable_value(&mut r.op, op, tl!(op.label()));
                }
            });
            ui.add_enabled(r.op.has_value(), egui::TextEdit::singleline(&mut r.value).desired_width(130.0));
            if ui.add(egui::Button::new("×").small()).on_hover_text(tl!("Remove condition")).clicked() {
                remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove.filter(|i| *i < filter.rules.len()) {
        filter.rules.remove(i);
    }
    ui.horizontal(|ui| {
        if ui.button(tl!("Add Condition")).clicked() && filter.rules.len() < 64 {
            let field = names.first().cloned().unwrap_or_default();
            filter.rules.push(FilterRow { field, op: FilterOp::Equal, value: String::new(), or: false });
        }
        if ui.button(tl!("Clear All")).clicked() {
            filter.rules.clear();
            filter.sort = Default::default();
        }
    });
    ui.add_space(4.0);
    egui::Grid::new("recipient_sort").num_columns(3).spacing(vec2(6.0, 4.0)).show(ui, |ui| {
        for (i, (field, desc)) in filter.sort.iter_mut().enumerate() {
            ui.label(tl!(if i == 0 { "Sort by" } else { "Then by" }));
            field_combo(ui, ("rs_field", i), field, true);
            ui.add_enabled_ui(!field.is_empty(), |ui| {
                egui::ComboBox::from_id_salt(("rs_dir", i)).width(110.0).selected_text(tl!(if *desc { "Descending" } else { "Ascending" })).show_ui(
                    ui,
                    |ui| {
                        ui.selectable_value(desc, false, tl!("Ascending"));
                        ui.selectable_value(desc, true, tl!("Descending"));
                    },
                );
            });
            ui.end_row();
        }
    });
}

/// The Table Properties dialog's fields: Table, Row, Column and Cell sections.
fn table_properties(ui: &mut Ui, f: &mut TableForm) {
    let unit = crate::options::unit();
    fn len(v: &mut f32, lo: f32, unit: wordcraft_geom::Unit) -> egui::DragValue<'_> {
        egui::DragValue::new(v).speed(0.01).range(lo..=22.0).measure(unit).max_decimals(2)
    }
    let heading = |ui: &mut Ui, s: &str| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
    };
    heading(ui, "Table");
    egui::Grid::new("tp_table").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.checkbox(&mut f.width_on, tl!("Preferred width:"));
        ui.add_enabled(f.width_on, len(&mut f.width, 0.1, unit));
        ui.end_row();
        ui.label(tl!("Alignment:"));
        ui.horizontal(|ui| {
            for (v, l) in [("left", "Left"), ("center", "Center"), ("right", "Right")] {
                ui.radio_value(&mut f.align, v.to_string(), tl!(l));
            }
        });
        ui.end_row();
        ui.label(tl!("Indent from left:"));
        ui.add_enabled(f.align == "left", egui::DragValue::new(&mut f.indent).speed(0.01).range(-11.0..=22.0).measure(unit).max_decimals(2));
        ui.end_row();
        ui.label(tl!("Table direction:"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut f.rtl, false, tl!("Left-to-right"));
            ui.radio_value(&mut f.rtl, true, tl!("Right-to-left"));
        });
        ui.end_row();
    });
    heading(ui, "Row");
    egui::Grid::new("tp_row").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.checkbox(&mut f.row_height_on, tl!("Specify height:"));
        ui.horizontal(|ui| {
            ui.add_enabled(f.row_height_on, len(&mut f.row_height, 0.02, unit));
            ui.add_enabled_ui(f.row_height_on, |ui| {
                egui::ComboBox::from_id_salt("tp_rule").selected_text(tl!(if f.row_exact { "Exactly" } else { "At least" })).show_ui(ui, |ui| {
                    ui.selectable_value(&mut f.row_exact, false, tl!("At least"));
                    ui.selectable_value(&mut f.row_exact, true, tl!("Exactly"));
                });
            });
        });
        ui.end_row();
    });
    ui.checkbox(&mut f.allow_break, tl!("Allow row to break across pages"));
    ui.checkbox(&mut f.header_row, tl!("Repeat as header row on each page"));
    heading(ui, "Column");
    egui::Grid::new("tp_col").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Preferred width:"));
        ui.add(len(&mut f.column_width, 0.1, unit));
        ui.end_row();
    });
    heading(ui, "Cell");
    egui::Grid::new("tp_cell").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.checkbox(&mut f.cell_width_on, tl!("Preferred width:"));
        ui.add_enabled(f.cell_width_on, len(&mut f.cell_width, 0.1, unit));
        ui.end_row();
        ui.label(tl!("Vertical alignment:"));
        ui.horizontal(|ui| {
            for (v, l) in [("top", "Top"), ("center", "Center"), ("bottom", "Bottom")] {
                ui.radio_value(&mut f.valign, v.to_string(), tl!(l));
            }
        });
        ui.end_row();
    });
}

fn cell_margins_dialog(ui: &mut Ui, f: &mut CellMarginsForm) {
    let unit = crate::options::unit();
    let max = 144.0 / unit.pt_per_unit();
    egui::Grid::new("cell_margins").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        for (label, value) in [("Top:", &mut f.top), ("Left:", &mut f.left), ("Bottom:", &mut f.bottom), ("Right:", &mut f.right)] {
            ui.label(tl!(label));
            ui.add(egui::DragValue::new(value).speed(0.01).range(0.0..=max).measure(unit).max_decimals(2));
            ui.end_row();
        }
    });
}

/// The Paragraph dialog as one paragraph's formatting fills it, on page `tab`.
fn paragraph_dialog(rp: &wordcraft_doc::resolve::ResolvedPara, tab: u8) -> Dialog {
    let line = match rp.line_spacing {
        wordcraft_doc::props::LineSpacing::Multiple(m) => m,
        _ => 1.0,
    };
    let (vl, vr) = rp.visual_indents();
    let unit = crate::options::unit().pt_per_unit();
    Dialog::Paragraph {
        rtl: rp.bidi,
        align: format!("{:?}", rp.align.visual(rp.bidi)).to_lowercase(),
        left: vl / unit,
        right: vr / unit,
        first: rp.indent_first / unit,
        before: rp.space_before,
        after: rp.space_after,
        line,
        keep_next: rp.keep_next,
        keep_lines: rp.keep_lines,
        page_break: rp.page_break_before,
        widow: rp.widow_control,
        tab,
        asian: [rp.kinsoku, rp.word_wrap, rp.overflow_punct, rp.top_line_punct, rp.auto_space_de, rp.auto_space_dn],
        opts: [rp.mirror_indents, rp.contextual_spacing, rp.suppress_line_numbers, rp.suppress_hyphens],
        start: serde_json::Map::new(),
        mixed: BTreeSet::new(),
    }
}

/// The Paragraph dialog's fields as `para.set` takes them: `align`, `left` and `right` as seen
/// on the page (its `onPage`), the rest as paragraph properties, line spacing as a multiple.
fn para_fields(d: &Dialog) -> serde_json::Map<String, Value> {
    let Dialog::Paragraph { rtl, align, left, right, first, before, after, line, keep_next, keep_lines, page_break, widow, asian, opts, .. } = d
    else {
        return serde_json::Map::new();
    };
    let unit = crate::options::unit().pt_per_unit();
    let [kinsoku, word_wrap, overflow, top_line, de, dn] = asian;
    let [mirror, contextual, no_numbers, no_hyphens] = opts;
    let fields = json!({
        "bidi": rtl, "align": align, "left": left * unit, "right": right * unit, "indentFirst": first * unit,
        "spaceBefore": before, "spaceAfter": after, "lineSpacing": {"rule": "multiple", "value": line},
        "keepNext": keep_next, "keepLines": keep_lines, "pageBreakBefore": page_break, "widowControl": widow,
        "mirrorIndents": mirror, "contextualSpacing": contextual, "suppressLineNumbers": no_numbers, "suppressHyphens": no_hyphens,
        "kinsoku": kinsoku, "wordWrap": word_wrap, "overflowPunct": overflow, "topLinePunct": top_line, "autoSpaceDE": de, "autoSpaceDN": dn,
    });
    match fields {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    }
}

/// The Paragraph dialog's number boxes: a length in the measurement unit (indents), points
/// (spacing) or lines (line spacing).
#[derive(Clone, Copy)]
enum ParaNumber {
    Length,
    Points,
    Lines,
}

/// A Paragraph dialog number box for the field `key`. While the selected paragraphs differ in
/// it, it shows nothing, not even the unit, until a value is typed or dragged in.
fn para_number(ui: &mut Ui, value: &mut f32, kind: ParaNumber, key: &str, mixed: &mut BTreeSet<String>) {
    let unit = crate::options::unit();
    let blank = mixed.contains(key);
    // A value from the document outside the range (line spacing 6) stays until edited.
    let field = match kind {
        ParaNumber::Length => egui::DragValue::new(value).speed(3.6 / unit.pt_per_unit()).max_decimals(2),
        ParaNumber::Points => egui::DragValue::new(value).speed(1.0).range(0.0..=1584.0).clamp_existing_to_range(false),
        ParaNumber::Lines => egui::DragValue::new(value).speed(0.05).range(0.5..=5.0).clamp_existing_to_range(false),
    };
    let field = match kind {
        ParaNumber::Length if blank => field.measure_blank(unit),
        ParaNumber::Length => field.measure(unit),
        _ if blank => field.custom_formatter(|_, _| String::new()),
        ParaNumber::Points => field.suffix(format!(" {}", tl!("pt"))),
        ParaNumber::Lines => field,
    };
    if ui.add(field).changed() {
        mixed.remove(key);
    }
}

/// A Paragraph dialog check box for the field `key`. While the selected paragraphs differ in
/// it, it shows as neither on nor off, and a click turns it on for all of them.
fn para_check(ui: &mut Ui, on: &mut bool, text: &str, key: &str, mixed: &mut BTreeSet<String>) {
    if !mixed.contains(key) {
        ui.checkbox(on, text);
    } else if ui.add(egui::Checkbox::new(&mut false, text).indeterminate(true)).clicked() {
        *on = true;
        mixed.remove(key);
    }
}

/// The Paragraph dialog's OK, as one `para.set` and so one undo step (#1346): the fields that
/// differ from what every selected paragraph had when it opened, not the blank ones, which
/// stay as each paragraph has them. The alignment and indents are as seen on the page, so a
/// new direction keeps them where the dialog shows them.
fn apply_paragraph(app: &mut WordApp, d: &Dialog) {
    let Dialog::Paragraph { start, mixed, .. } = d else { return };
    let fields = para_fields(d);
    let turned = !mixed.contains("bidi") && fields.get("bidi") != start.get("bidi");
    let (mut props, mut page) = (serde_json::Map::new(), serde_json::Map::new());
    for (k, v) in fields {
        let on_page = matches!(k.as_str(), "align" | "left" | "right");
        if mixed.contains(&k) || (start.get(&k) == Some(&v) && !(turned && on_page)) {
            continue;
        }
        if on_page {
            page.insert(k, v);
        } else {
            props.insert(k, v);
        }
    }
    if !props.is_empty() || !page.is_empty() {
        let _ = app.run("para.set", json!({"props": props, "onPage": page}));
    }
}

/// The innermost bookmark around the caret, which the Bookmark dialog opens on (as Word does);
/// empty when the caret is in none.
fn bookmark_at_caret(s: &wordcraft_engine::Session) -> String {
    let at = &s.sel.focus;
    s.doc
        .bookmarks()
        .into_iter()
        .rev()
        .find(|(name, start)| !name.starts_with('_') && start <= at && *at <= wordcraft_engine::cmd::insert::bookmark_span(&s.doc, name, start).1)
        .map(|(name, _)| name)
        .unwrap_or_default()
}

/// The bookmarks the Bookmark dialog lists: by name or in document order, the hidden ones
/// (`_Toc…`, `_Ref…`) only when asked for.
fn bookmark_names(doc: &wordcraft_doc::Document, by_location: bool, hidden: bool) -> Vec<String> {
    let mut names: Vec<String> = doc.bookmarks().into_iter().map(|(n, _)| n).filter(|n| hidden || !n.starts_with('_')).collect();
    if !by_location {
        names.sort_by_key(|n| n.to_lowercase());
    }
    names
}

/// Insert › Bookmark, laid out as Word's (#1236): the name box over the list, Add / Delete / Go To
/// beside them, Sort by and Hidden bookmarks below. Add (Enter) closes the dialog; Delete and Go
/// To leave it open, and once a bookmark is deleted Cancel reads Close.
fn bookmark(app: &mut WordApp, ui: &mut Ui, name: &mut String, by_location: &mut bool, hidden: &mut bool, deleted: &mut bool) -> bool {
    let marks = bookmark_names(&app.session.doc, *by_location, *hidden);
    let t = Tokens::get(ui.ctx());
    let (mut add, mut delete, mut go_to) = (false, false, false);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(tl!("Bookmark name:"));
            ui.add(egui::TextEdit::singleline(name).desired_width(240.0));
            egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.border)).inner_margin(4).show(ui, |ui| {
                ui.set_width(232.0);
                ui.set_height(150.0);
                egui::ScrollArea::vertical().id_salt("bookmark_list").max_height(150.0).auto_shrink([false, false]).show(ui, |ui| {
                    if marks.is_empty() {
                        // #1904: a blank box says nothing; reuse the Field dialog's wording.
                        ui.label(egui::RichText::new(tl!("The document has no bookmarks yet.")).small().color(t.text_dim));
                    }
                    for n in &marks {
                        if ui.selectable_label(*n == *name, n.as_str()).clicked() {
                            *name = n.clone();
                        }
                    }
                });
            });
        });
        ui.vertical(|ui| {
            ui.add_space(18.0);
            let listed = marks.iter().any(|n| n == name.trim());
            let w = vec2(90.0, 0.0);
            add = ui.add_enabled(wordcraft_engine::cmd::insert::bookmark_name_ok(name.trim()), egui::Button::new(tl!("Add")).min_size(w)).clicked();
            delete = ui.add_enabled(listed, egui::Button::new(tl!("Delete")).min_size(w)).clicked();
            go_to = ui.add_enabled(listed, egui::Button::new(tl!("Go To")).min_size(w)).clicked();
        });
    });
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        ui.label(tl!("Sort by:"));
        ui.vertical(|ui| {
            ui.radio_value(by_location, false, tl!("Name"));
            ui.radio_value(by_location, true, tl!("Location"));
        });
    });
    ui.checkbox(hidden, tl!("Hidden bookmarks"));
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    if add || (enter && wordcraft_engine::cmd::insert::bookmark_name_ok(name.trim())) {
        let _ = app.run("insert.bookmark", json!({"name": name.trim()}));
        return true;
    }
    if delete && app.run("insert.deleteBookmark", json!({"name": name.trim()})).is_ok() {
        name.clear();
        *deleted = true;
    }
    if go_to {
        let _ = app.run("edit.goto", json!({"bookmark": name.trim()}));
    }
    ui.add_space(8.0);
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close |= ui.button(if *deleted { tl!("Close") } else { tl!("Cancel") }).clicked();
        });
    });
    close
}

/// Manage Styles' body. Reads the list from `styles.manage`'s data (never re-running the command,
/// which would ask to open this dialog again) and acts through commands.
fn manage_styles(app: &mut WordApp, ui: &mut Ui, alphabetical: &mut bool, selected: &mut String) -> bool {
    let list = wordcraft_engine::cmd::para::manage_list(&app.session, *alphabetical);
    let list = list.as_array().cloned().unwrap_or_default();
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    if !list.iter().any(|x| str_of(x, "id") == *selected) {
        *selected = list.first().map(|x| str_of(x, "id")).unwrap_or_default();
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Sort order:"));
        egui::ComboBox::from_id_salt("manage_styles_sort")
            .selected_text(if *alphabetical { tl!("Alphabetical") } else { tl!("As Recommended") })
            .show_ui(ui, |ui| {
                ui.selectable_value(alphabetical, false, tl!("As Recommended"));
                ui.selectable_value(alphabetical, true, tl!("Alphabetical"));
            });
    });
    let t = Tokens::get(ui.ctx());
    let shown = egui::Id::new("manage_styles_shown");
    egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.border)).inner_margin(4).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("manage_styles_list").max_height(200.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(380.0);
            for st in &list {
                let id = str_of(st, "id");
                let glyph = match st.get("type").and_then(Value::as_str) {
                    Some("character") => "a",
                    Some("linked") => "¶a",
                    Some("table") => "⊞",
                    _ => "¶",
                };
                let hidden = st.get("hidden").and_then(Value::as_bool).unwrap_or(false);
                let mut name = egui::RichText::new(format!("{glyph}  {}", str_of(st, "name")));
                if hidden {
                    name = name.weak();
                }
                let r = ui.selectable_label(*selected == id, name);
                // Bring the selection into view once, when it changes (opening, New Style…).
                if *selected == id && ui.data(|d| d.get_temp::<String>(shown)).as_deref() != Some(id.as_str()) {
                    r.scroll_to_me(Some(egui::Align::Center));
                    ui.data_mut(|d| d.insert_temp(shown, id.clone()));
                }
                if r.clicked() {
                    ui.data_mut(|d| d.insert_temp(shown, id.clone()));
                    *selected = id.clone();
                }
                if r.double_clicked() {
                    *selected = id;
                    if let Some(Dialog::ModifyStyle { id, name, font, size, bold, italic, color, before, after, .. }) =
                        Dialog::modify_style(app, selected)
                    {
                        app.dialog = Some(Dialog::ModifyStyle { back: true, resume: None, id, name, font, size, bold, italic, color, before, after });
                    }
                }
            }
        });
    });
    let leave = |ui: &mut Ui| {
        ui.data_mut(|d| d.remove::<String>(shown));
        true
    };
    if app.dialog.is_some() {
        return leave(ui);
    }
    let Some(cur) = list.iter().find(|x| str_of(x, "id") == *selected).cloned() else { return close_button(ui) && leave(ui) };
    let ty = str_of(&cur, "type");
    ui.add_space(6.0);
    crate::previews::style_preview(app, ui, selected, &ty, vec2(388.0, if ty == "table" { 64.0 } else { 40.0 }));
    ui.add_space(4.0);
    ui.add(egui::Label::new(egui::RichText::new(describe(&cur)).small()).wrap());
    ui.add_space(6.0);
    let builtin = cur.get("builtIn").and_then(Value::as_bool).unwrap_or(true);
    if ty != "table" {
        let mut gallery = cur.get("inGallery").and_then(Value::as_bool).unwrap_or(false);
        if ui.checkbox(&mut gallery, tl!("Show in the Styles gallery")).changed() {
            let _ = app.run("styles.setVisibility", json!({"style": selected, "gallery": gallery}));
        }
    }
    let mut hidden = cur.get("hidden").and_then(Value::as_bool).unwrap_or(false);
    if ui.checkbox(&mut hidden, tl!("Hide from style lists")).changed() {
        let _ = app.run("styles.setVisibility", json!({"style": selected, "hidden": hidden}));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // Table styles are edited from Table Design.
        if ui.add_enabled(ty != "table", egui::Button::new(tl!("Modify…"))).clicked()
            && let Some(Dialog::ModifyStyle { id, name, font, size, bold, italic, color, before, after, .. }) = Dialog::modify_style(app, selected)
        {
            app.dialog = Some(Dialog::ModifyStyle { back: true, resume: None, id, name, font, size, bold, italic, color, before, after });
        }
        if ui.button(tl!("New Style…")).clicked() {
            app.dialog = Some(Dialog::NewStyle { name: "Style1".into(), based_on: str_of(&cur, "name"), back: true });
        }
        let del = ui.add_enabled(!builtin, egui::Button::new(tl!("Delete")));
        let del = if builtin { del.on_disabled_hover_text(tl!("Built-in styles can't be deleted.")) } else { del };
        if del.clicked() {
            let _ = app.run("styles.delete", json!({"style": selected}));
            selected.clear();
        }
    });
    (app.dialog.is_some() || close_button(ui)) && leave(ui)
}

/// A dialog's single Close button (Escape too).
fn close_button(ui: &mut Ui) -> bool {
    ui.add_space(8.0);
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        close |= ui.add(crate::widgets::primary_button(ui, tl!("Close"))).clicked();
    });
    close
}

/// A style's description: its type, the formatting it sets and what it is based on.
fn describe(st: &Value) -> String {
    let f = st.get("format").cloned().unwrap_or_default();
    let fs = |k: &str| f.get(k).and_then(Value::as_str);
    let fnum = |k: &str| f.get(k).and_then(Value::as_f64);
    let pt = |x: f64| format!("{}", (x * 10.0).round() / 10.0);
    let mut parts: Vec<String> = vec![
        match st.get("type").and_then(Value::as_str) {
            Some("character") => tl!("Character style"),
            Some("linked") => tl!("Linked style (paragraph and character)"),
            Some("table") => tl!("Table style"),
            _ => tl!("Paragraph style"),
        }
        .to_string(),
    ];
    if st.get("builtIn").and_then(Value::as_bool).unwrap_or(false) {
        parts.push(tl!("Built-in style").to_string());
    }
    if let Some(x) = fs("font") {
        parts.push(crate::i18n::fmt(tl!("Font: {font}"), &[("font", x)]));
    }
    if let Some(x) = fnum("size") {
        parts.push(crate::i18n::fmt(tl!("{size} pt"), &[("size", &pt(x))]));
    }
    if f.get("bold").and_then(Value::as_bool) == Some(true) {
        parts.push(tl!("Bold").to_string());
    }
    if f.get("italic").and_then(Value::as_bool) == Some(true) {
        parts.push(tl!("Italic").to_string());
    }
    if let Some(x) = fs("color") {
        parts.push(crate::i18n::fmt(tl!("Color: {color}"), &[("color", &format!("#{x}"))]));
    }
    if let Some(x) = fnum("spaceBefore") {
        parts.push(crate::i18n::fmt(tl!("Space before: {pt} pt"), &[("pt", &pt(x))]));
    }
    if let Some(x) = fnum("spaceAfter") {
        parts.push(crate::i18n::fmt(tl!("Space after: {pt} pt"), &[("pt", &pt(x))]));
    }
    if let Some(b) = st.get("basedOn").and_then(Value::as_str) {
        parts.push(crate::i18n::fmt(tl!("Based on: {style}"), &[("style", b)]));
    }
    parts.join(", ")
}

/// What Search lists before anything is typed: common commands (#819).
const SEARCH_SUGGESTIONS: [&str; 3] = ["review.spelling", "review.wordCount", "file.print"];

/// Search's "Find … in the document": the Navigation pane opens on its results for `text`.
fn find_in_navigation_pane(app: &mut WordApp, ctx: &egui::Context, text: &str) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("nav_query"), text.to_string()));
    app.ui.nav_tab = "results".into();
    let _ = app.run("view.navigationPane", json!({"value": true}));
    let _ = app.run("edit.find", json!({"text": text}));
}

/// Whether `e` is the text of the shortcut that opened a box (see `Dialog::Commands::swallow`):
/// a plain key press first means the user is typing, and one swallowed text is all there is.
fn swallow_shortcut_text(swallow: &mut bool, e: &egui::Event) -> bool {
    match e {
        egui::Event::Key { pressed: true, modifiers, .. } if !modifiers.alt => {
            *swallow = false;
            false
        }
        egui::Event::Text(_) if *swallow => {
            *swallow = false;
            true
        }
        _ => false,
    }
}

/// Search's filter: every word of the query is in the command's label, location or id, in English
/// or in the interface language, so "insert table" finds Insert › Tables › Table (#819).
fn command_matches(c: &wordcraft_engine::CommandSpec, query: &str) -> bool {
    if query.trim().is_empty() {
        return false;
    }
    let hay = format!(
        "{} {} {} {} {}",
        c.label.to_lowercase(),
        c.location.to_lowercase(),
        c.id.to_lowercase(),
        crate::i18n::lowercase(tl!(c.label)),
        crate::i18n::lowercase(&crate::i18n::location(c.location))
    );
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()) || hay.contains(&crate::i18n::lowercase(w)))
}

/// The Find and Replace dialog's Go To tab (#712): a page (a number, or `+2` / `-1` from the
/// current page; empty, Next and Previous step a page) or a bookmark. Returns true to close.
fn goto_body(app: &mut WordApp, ui: &mut Ui, what: &mut GoToWhat, target: &mut String, message: &mut String) -> bool {
    // The document's bookmarks, only while they are listed (finding them walks the document).
    let bookmarks: Vec<String> = if *what == GoToWhat::Bookmark {
        app.session.doc.bookmarks().into_iter().map(|(n, _)| n).filter(|n| !n.starts_with('_')).collect()
    } else {
        Vec::new()
    };
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(tl!("Go to what:"));
            for (w, label) in [(GoToWhat::Page, "Page"), (GoToWhat::Bookmark, "Bookmark")] {
                if ui.selectable_label(*what == w, tl!(label)).clicked() && *what != w {
                    *what = w;
                    target.clear();
                    message.clear();
                }
            }
        });
        ui.add_space(16.0);
        ui.vertical(|ui| match what {
            GoToWhat::Page => {
                ui.label(tl!("Enter page number:"));
                let r = ui.add(egui::TextEdit::singleline(target).desired_width(220.0));
                if !r.has_focus() {
                    r.request_focus();
                }
                ui.label(egui::RichText::new(tl!("Type +2 or -1 to move from the page you are on.")).weak());
            }
            GoToWhat::Bookmark => {
                ui.label(tl!("Enter bookmark name:"));
                egui::ComboBox::from_id_salt("goto_bookmark").width(220.0).selected_text(target.as_str()).show_ui(ui, |ui| {
                    for b in &bookmarks {
                        ui.selectable_value(target, b.clone(), b.as_str());
                    }
                });
                if bookmarks.is_empty() {
                    ui.label(egui::RichText::new(tl!("This document has no bookmarks.")).weak());
                }
            }
        });
    });
    if !message.is_empty() {
        ui.label(egui::RichText::new(message.as_str()).weak());
    }
    let mut close = false;
    let mut go: Option<Value> = None;
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let page_step = *what == GoToWhat::Page && target.trim().is_empty();
        if page_step && ui.button(tl!("Previous")).clicked() {
            go = Some(json!({"pageBy": -1}));
        }
        let label = if page_step { tl!("Next") } else { tl!("Go To") };
        if ui.button(label).clicked() || enter {
            go = match what {
                GoToWhat::Page => match goto_page(target) {
                    Some(p) => Some(p),
                    None => {
                        *message = tl!("Type a page number, or +2 / -1.").to_string();
                        None
                    }
                },
                GoToWhat::Bookmark if !target.is_empty() => Some(json!({"bookmark": target.clone()})),
                GoToWhat::Bookmark => None,
            };
        }
        if ui.button(tl!("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
    });
    if let Some(params) = go {
        match app.run("edit.goto", params) {
            Ok(_) => message.clear(),
            Err(e) => *message = e,
        }
    }
    close
}

/// Go To's page box as `edit.goto` params: empty steps one page on, `+n` / `-n` move from the
/// current page, a number is that page. `None` for anything else.
fn goto_page(text: &str) -> Option<Value> {
    let t = text.trim().replace('\u{2212}', "-");
    if t.is_empty() {
        return Some(json!({"pageBy": 1}));
    }
    if let Some(rest) = t.strip_prefix('+') {
        return rest.parse::<i64>().ok().filter(|n| *n >= 0).map(|n| json!({"pageBy": n}));
    }
    if t.starts_with('-') {
        return t.parse::<i64>().ok().map(|n| json!({"pageBy": n}));
    }
    t.parse::<u64>().ok().filter(|n| *n >= 1).map(|n| json!({"page": n}))
}

/// path data; Insert (or a double click) runs `insert.icon`. Returns true to close.
fn icons_body(app: &mut WordApp, ui: &mut Ui, query: &mut String, category: &mut String, selected: &mut String) -> bool {
    use wordcraft_render::icon_lib::{self, Category};
    let t = Tokens::get(ui.ctx());
    ui.set_width(560.0);
    // Typing goes to the search box (never to the document behind the dialog).
    let r = ui.add(egui::TextEdit::singleline(query).hint_text(tl!("Search icons")).desired_width(560.0));
    if !r.has_focus() {
        r.request_focus();
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        if ui.selectable_label(category.is_empty(), tl!("All")).clicked() {
            category.clear();
        }
        for c in Category::ALL {
            if ui.selectable_label(*category == c.name(), tl!(c.name())).clicked() {
                *category = c.name().to_string();
            }
        }
    });
    ui.add_space(4.0);
    let hits = icon_lib::search(query, Category::parse(category));
    let mut insert = false;
    egui::ScrollArea::vertical().max_height(300.0).min_scrolled_height(300.0).auto_shrink([true, false]).show(ui, |ui| {
        if hits.is_empty() {
            ui.label(egui::RichText::new(tl!("No icons match your search.")).color(t.text_dim));
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for icon in &hits {
                let (rect, resp) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::click());
                let on = *selected == icon.id;
                let fill = if on {
                    t.checked
                } else if resp.hovered() {
                    t.hover
                } else {
                    egui::Color32::TRANSPARENT
                };
                let edge = if on { egui::Stroke::new(1.5, t.accent) } else { egui::Stroke::new(1.0, t.border) };
                ui.painter().rect(rect, 4.0, fill, edge, egui::StrokeKind::Inside);
                crate::icons::library(ui.painter(), rect.shrink(12.0), icon, t.text);
                let resp = resp.on_hover_text(icon.name);
                if resp.clicked() {
                    *selected = icon.id.to_string();
                }
                if resp.double_clicked() {
                    *selected = icon.id.to_string();
                    insert = true;
                }
            }
        });
    });
    let name = icon_lib::find(selected).map(|i| i.name).unwrap_or("");
    ui.add_space(4.0);
    ui.label(egui::RichText::new(name).color(t.text_dim));
    let (ok, cancel) = buttons(ui, tl!("Insert"));
    if (ok || insert) && !selected.is_empty() {
        let _ = app.run("insert.icon", json!({"id": selected.as_str()}));
        return true;
    }
    cancel
}

/// Enclose Characters' preview: the text in the chosen shape, shrunk or with the shape enlarged
/// (drawn, so it doesn't depend on the interface font having ○ □ △ ◇).
fn enclose_preview(ui: &mut Ui, style: &str, text: &str, shape: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(110.0, 64.0), Sense::hover());
    let p = ui.painter_at(r);
    p.rect_filled(r, 2.0, t.input);
    p.rect_stroke(r, 2.0, egui::Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    let ink = ui.visuals().text_color();
    let c = r.center();
    let (box_h, text_px) = match style {
        "none" => (0.0, 26.0),
        "enlarge" => (48.0, 26.0),
        _ => (34.0, 20.0),
    };
    let h = box_h / 2.0;
    let stroke = egui::Stroke::new(1.5, ink);
    match (shape, box_h > 0.0) {
        (_, false) => {}
        ("square", _) => {
            p.rect_stroke(egui::Rect::from_center_size(c, vec2(box_h, box_h)), 0.0, stroke, egui::StrokeKind::Middle);
        }
        ("triangle", _) => {
            let pts = vec![egui::pos2(c.x, c.y - h), egui::pos2(c.x + h * 1.1, c.y + h), egui::pos2(c.x - h * 1.1, c.y + h)];
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        ("diamond", _) => {
            let pts = vec![egui::pos2(c.x, c.y - h), egui::pos2(c.x + h, c.y), egui::pos2(c.x, c.y + h), egui::pos2(c.x - h, c.y)];
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        _ => {
            p.circle_stroke(c, h, stroke);
        }
    }
    let px = match (style, shape) {
        ("shrink", "triangle" | "diamond") => text_px * 0.75,
        _ => text_px,
    };
    let dy = if shape == "triangle" && box_h > 0.0 { h * 0.3 } else { 0.0 };
    p.text(c + vec2(0.0, dy), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(px), ink);
}

/// Page Setup's "Vertical alignment:" box (Word's Layout tab).
fn page_valign_combo(ui: &mut egui::Ui, valign: &mut wordcraft_doc::section::PageVAlign) {
    use wordcraft_doc::section::PageVAlign;
    const CHOICES: [(PageVAlign, &str); 4] =
        [(PageVAlign::Top, "Top"), (PageVAlign::Center, "Center"), (PageVAlign::Justify, "Justified"), (PageVAlign::Bottom, "Bottom")];
    ui.horizontal(|ui| {
        ui.label(tl!("Vertical alignment:"));
        let shown = CHOICES.iter().find(|(v, _)| v == valign).map_or("", |(_, l)| tl!(l));
        egui::ComboBox::from_id_salt("ps_valign").selected_text(shown).show_ui(ui, |ui| {
            for (v, l) in CHOICES {
                ui.selectable_value(valign, v, tl!(l));
            }
        });
    });
}

/// `layout.verticalAlign`'s name for a value.
fn page_valign_value(v: wordcraft_doc::section::PageVAlign) -> &'static str {
    use wordcraft_doc::section::PageVAlign;
    match v {
        PageVAlign::Top => "top",
        PageVAlign::Center => "center",
        PageVAlign::Justify => "justify",
        PageVAlign::Bottom => "bottom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::Pos;
    use wordcraft_doc::props::Align;
    use wordcraft_engine::Session;

    /// #3931: hovering Insert › Table's grid asks the page for a preview of that table (rows by
    /// columns) and changes nothing; leaving the grid drops it; a click inserts the table with no
    /// preview left over for the page to show on top of it.
    #[test]
    fn the_table_grid_previews_the_hovered_table_until_the_click() {
        use egui_kittest::kittest::Queryable;
        let a = WordApp::new(Session::new(wordcraft_doc::Document::from_text("Lanterns")), crate::Services::default());
        let mut h =
            egui_kittest::Harness::builder().with_size(vec2(400.0, 400.0)).build_ui_state(|ui, app: &mut WordApp| table_grid_picker(ui, app), a);
        h.run_steps(2);
        let spacing = h.ctx.global_style().spacing.clone();
        let top = h.get_by_label("Insert Table").rect();
        // Squares are 16 points with 2 between, in grid columns at least `interact_size` wide:
        // the middle of row 3, column 4.
        let col = spacing.interact_size.x.max(16.0) + 2.0;
        let at = egui::pos2(top.left() + 3.0 * col + 8.0, top.bottom() + spacing.item_spacing.y + 2.0 * 18.0 + 8.0);
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(2);
        assert_eq!(h.state().canvas.table_preview, Some((3, 4)), "three rows of four columns");
        assert!(h.query_by_label("4×3 Table").is_some(), "the label reads columns × rows");
        assert!(h.state().session.doc.body.iter().all(|b| b.as_table().is_none()) && !h.state().session.can_undo());
        h.event(egui::Event::PointerMoved(egui::pos2(390.0, 390.0)));
        h.state_mut().canvas.table_preview = None;
        h.run_steps(2);
        assert_eq!(h.state().canvas.table_preview, None, "off the grid nothing is previewed");
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        // The page takes the request every frame; here nothing does.
        h.state_mut().canvas.table_preview = None;
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
        let t = h.state().session.doc.body.iter().find_map(|b| b.as_table()).expect("the click inserts the table");
        assert_eq!((t.rows.len(), t.cols()), (3, 4));
        assert_eq!(h.state().canvas.table_preview, None, "the inserted table isn't previewed again");
    }

    /// Go To's page box (#712): a number, +n / -n from the current page (a typographic minus
    /// too), empty for the next page; anything else is refused.
    #[test]
    fn go_to_reads_its_page_box() {
        assert_eq!(goto_page("7"), Some(json!({"page": 7})));
        assert_eq!(goto_page(" +2 "), Some(json!({"pageBy": 2})));
        assert_eq!(goto_page("-1"), Some(json!({"pageBy": -1})));
        assert_eq!(goto_page("\u{2212}3"), Some(json!({"pageBy": -3})));
        assert_eq!(goto_page(""), Some(json!({"pageBy": 1})));
        for junk in ["0", "abc", "+x", "1.5", "99999999999999999999999"] {
            assert_eq!(goto_page(junk), None, "{junk}");
        }
    }

    /// Insert › Bookmark (#1236), as in Word: it opens on the bookmark around the caret, lists by
    /// name, Go To selects a bookmark's text and Delete removes one — both leaving the dialog
    /// open, Cancel then reading Close — and one undo brings the bookmark back.
    #[test]
    fn the_bookmark_dialog_goes_to_and_deletes_bookmarks() {
        fn press(h: &mut egui_kittest::Harness<'_, WordApp>, label: &str) {
            use egui_kittest::kittest::Queryable;
            let dialog = h.ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", "Bookmark")))).expect("the dialog is open");
            h.query_all_by_label(label).find(|n| dialog.contains(n.rect().center())).expect(label).click();
            for _ in 0..3 {
                h.step();
            }
        }
        let mut app = app_with("Alpha beta gamma");
        for (word, name) in [("Alpha", "_Hidden"), ("beta", "Zulu"), ("gamma", "Yankee")] {
            let _ = app.run("select.text", json!({"text": word}));
            let _ = app.run("insert.bookmark", json!({"name": name}));
        }
        let doc = &app.session.doc;
        assert_eq!(bookmark_names(doc, false, false), ["Yankee", "Zulu"], "by name, hidden ones left out");
        assert_eq!(bookmark_names(doc, true, false), ["Zulu", "Yankee"], "by location");
        assert_eq!(bookmark_names(doc, false, true), ["_Hidden", "Yankee", "Zulu"]);
        let _ = app.run("select.text", json!({"text": "gamma"}));
        app.dialog = Dialog::open("bookmark", &mut app);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        assert!(matches!(&h.state().dialog, Some(Dialog::Bookmark { name, .. }) if name == "Yankee"), "opens on the caret's bookmark");
        press(&mut h, "Zulu");
        press(&mut h, "Go To");
        assert_eq!(h.state().session.selected_text(), "beta");
        press(&mut h, "Delete");
        let names: Vec<String> = h.state().session.doc.bookmarks().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["_Hidden", "Yankee"]);
        assert!(matches!(&h.state().dialog, Some(Dialog::Bookmark { name, deleted: true, .. }) if name.is_empty()), "still open");
        assert_eq!(h.state().session.selected_text(), "beta", "the text stays selected");
        press(&mut h, "Close");
        assert!(h.state().dialog.is_none());
        let _ = h.state_mut().run("edit.undo", json!({}));
        assert_eq!(h.state().session.doc.bookmarks().len(), 3, "one undo step");
    }

    /// #1904: with no bookmarks the list says so instead of a blank box.
    #[test]
    fn the_bookmark_dialog_says_when_there_are_no_bookmarks() {
        let mut app = app_with("Alpha beta gamma");
        app.dialog = Dialog::open("bookmark", &mut app);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        let texts: Vec<String> = h
            .output()
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("The document has no bookmarks yet.")), "placeholder shown");
    }

    /// Find, Replace and Go To are one dialog with three tabs (#712).
    #[test]
    fn find_replace_and_go_to_are_one_dialog() {
        let mut app = app_with("One");
        for (name, tab) in [("find", FindTab::Find), ("replace", FindTab::Replace), ("goto", FindTab::GoTo)] {
            let _ = app.run("ui.dialog", json!({"name": name}));
            assert!(matches!(&app.dialog, Some(Dialog::Find { tab: t, .. }) if *t == tab), "{name}");
            assert_eq!(app.dialog.as_ref().map(|d| d.name()), Some(name));
        }
    }

    fn app_with(text: &str) -> WordApp {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let _ = app.run("document.setText", json!({"text": text}));
        app
    }

    fn props(app: &WordApp) -> wordcraft_doc::props::ParaProps {
        app.session.doc.para_at(&app.session.sel.focus).map(|p| p.props.clone()).unwrap_or_default()
    }

    /// #819: the text a shortcut sends with its key isn't typed into the box it opened, whether it
    /// comes in the same frame or the next; real typing (a key press first) is.
    #[test]
    fn shortcut_text_is_swallowed_once() {
        let key = |key, alt| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { alt, ..Default::default() },
        };
        let text = |t: &str| egui::Event::Text(t.into());
        let run = |events: &[egui::Event]| {
            let mut swallow = true;
            events.iter().filter(|e| !swallow_shortcut_text(&mut swallow, e)).cloned().collect::<Vec<_>>()
        };
        assert_eq!(run(&[key(egui::Key::Q, true), text("q")]), vec![key(egui::Key::Q, true)], "Alt+Q's q, same frame");
        assert_eq!(run(&[text("q"), text("b")]), vec![text("b")], "the q a frame late; then typing");
        assert_eq!(run(&[key(egui::Key::B, false), text("b")]), vec![key(egui::Key::B, false), text("b")], "a click-opened box takes typing");
    }

    /// #819: Search matches every word anywhere in the label, location or id, in any order.
    #[test]
    fn command_search_matches_every_word() {
        let app = app_with("");
        let reg = app.session.registry.clone();
        let table = reg.get("insert.table").unwrap();
        assert!(command_matches(table, "insert table"), "the box's own example");
        assert!(command_matches(table, "  Table   INSERT "), "any order, any case, extra spaces");
        assert!(!command_matches(table, "insert chart"), "every word has to match");
        assert!(!command_matches(table, ""), "an empty box lists nothing");
        assert!(!command_matches(table, "   "));
    }

    /// Ordinary Turkish queries must find dotted-I labels, while canonical English labels and
    /// ASCII command IDs retain their old case-insensitive search in every interface language.
    #[test]
    fn command_search_respects_turkish_i_pairs_and_keeps_canonical_matches() {
        use egui_kittest::kittest::Queryable;
        // Turkish casing follows the language code; the labels are fixtures with both I pairs.
        const LABELS: &str =
            "\tItalic\tİtalik\n\tHome\tGiriş\n\tFont\tYazı Tipi\n\tTable of Contents\tİçindekiler Tablosu\n\tReferences\tBaşvurular\n";
        let reg = app_with("").session.registry.clone();
        let italic = reg.get("format.italic").unwrap();
        let table = reg.get("insert.table").unwrap();
        let toc = reg.all().iter().find(|c| c.label == "Table of Contents" && c.location == "References › Table of Contents").unwrap();
        crate::i18n::set_current(crate::i18n::fixture_lang("tr", LABELS));
        for query in ["italik", "İTALİK", "YAZI TİPİ", "FORMAT.ITALIC", "ITALIC"] {
            assert!(command_matches(italic, query), "tr: {query}");
        }
        for query in ["içindekiler", "TABLE OF CONTENTS"] {
            assert!(command_matches(toc, query), "tr: {query}");
        }
        for query in ["INSERT", "INSERT.TABLE"] {
            assert!(command_matches(table, query), "tr: {query}");
        }
        assert_eq!(crate::i18n::lowercase("Iİıi"), "ıiıi");
        // The same labels under another language's code use Unicode's default casing.
        crate::i18n::set_current(crate::i18n::fixture_lang("x-fixture", LABELS));
        assert!(!command_matches(italic, "italik"), "İ lowercases to i plus a combining dot");
        assert_eq!(crate::i18n::lowercase("Iİıi"), "ii\u{307}ıi");
        crate::i18n::set_current(crate::i18n::Lang::EN);

        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            move |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                if fonts {
                    show(app, &ctx);
                } else {
                    crate::theme::install_fonts_for(&ctx, false);
                    fonts = true;
                }
            },
            app_with("Unchanged document"),
        );
        let before = serde_json::to_value(&h.state().session.doc).unwrap();
        for (query, expected) in
            [("ITALIC", "Italic   —   Home › Font"), ("FORMAT.ITALIC", "Italic   —   Home › Font"), ("INSERT.TABLE", "Table   —   Insert › Tables")]
        {
            h.state_mut().dialog = Some(Dialog::Commands { query: query.into(), swallow: false });
            h.run_steps(4);
            assert!(
                h.query(egui_kittest::kittest::By::new().role(egui::accesskit::Role::Button).label_contains(expected)).is_some(),
                "{query}\n{:?}",
                h.query_all_by_role(egui::accesskit::Role::Button).collect::<Vec<_>>()
            );
        }
        assert_eq!(serde_json::to_value(&h.state().session.doc).unwrap(), before, "search does not change the document");
    }

    /// #3914: the Phonetic Guide dialog opens with the keyboard in its Ruby text box, so typing
    /// fills the box and does not replace the selected text in the document.
    #[test]
    fn phonetic_guide_ruby_box_takes_the_keyboard_on_open() {
        let mut app = app_with("Unchanged document");
        app.dialog = Some(Dialog::Phonetic {
            base: "Unchanged".into(),
            ruby: String::new(),
            align: "center".into(),
            size: 12.0,
            offset: 0.0,
            existing: false,
            focus: true,
        });
        let mut h = app_harness(app);
        h.run_steps(4);
        h.event(egui::Event::Text("ab".into()));
        h.run_steps(2);
        match h.state().dialog.as_ref() {
            Some(Dialog::Phonetic { ruby, .. }) => assert_eq!(ruby, "ab", "the typed text reaches the Ruby text box"),
            _ => panic!("the Phonetic Guide dialog is still open"),
        }
    }

    /// #2986: a box that keeps the keyboard asks for focus only while it lacks it. `request_focus`
    /// always interrupts IME; on X11 that recreates the XIM context, and Search asking every frame
    /// left fcitx taking no keys at all. Once the box has focus, no frame interrupts, and typing lands;
    /// after Tab takes the keyboard away, the box takes it back with one reset, not one per frame.
    #[test]
    fn boxes_that_keep_focus_do_not_interrupt_ime_every_frame() {
        fn typed(d: Option<&Dialog>) -> Option<&str> {
            match d? {
                Dialog::Commands { query, .. } | Dialog::Icons { query, .. } => Some(query.as_str()),
                Dialog::Find { goto_target, .. } => Some(goto_target.as_str()),
                Dialog::Refs(r) => match r.as_ref() {
                    crate::dialogs_refs::RefsDialog::NewLabel(f) => Some(f.new_label.as_str()),
                    _ => None,
                },
                _ => None,
            }
        }
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        for name in ["commands", "icons", "goto", "newLabel"] {
            let mut app = app_with("Unchanged document");
            let dialog = match name {
                "newLabel" => Dialog::Refs(Box::new(crate::dialogs_refs::RefsDialog::NewLabel(crate::dialogs_refs::CaptionForm::read(&app)))),
                _ => Dialog::open(name, &mut app).unwrap(),
            };
            app.dialog = Some(dialog);
            let mut h = app_harness(app);
            // The box takes the keyboard from the page in the first frames.
            h.run_steps(4);
            let interrupted =
                |h: &egui_kittest::Harness<'_, WordApp>| h.output().platform_output.ime.as_ref().map(|i| i.should_interrupt_composition);
            for frame in 0..6 {
                h.step();
                assert_eq!(interrupted(&h), Some(false), "{name}: frame {frame} resets the input method");
            }
            for (k, t) in [(egui::Key::A, "a"), (egui::Key::B, "b")] {
                h.event(key(k));
                h.event(egui::Event::Text(t.into()));
                h.step();
                assert_eq!(interrupted(&h), Some(false), "{name}: typing {t} resets the input method");
            }
            assert_eq!(typed(h.state().dialog.as_ref()), Some("ab"), "{name}: the keys reach the box");

            // Tab moves the keyboard to the dialog's next control and the box takes it back. As
            // egui-winit does, count a reset when IME comes back on or is interrupted while on (each
            // recreates the XIM context): one or two for the round trip, then none, and the next
            // key reaches the box.
            let reset = |h: &egui_kittest::Harness<'_, WordApp>, on: &mut bool| {
                let ime = h.output().platform_output.ime.as_ref();
                let reset = ime.is_some_and(|i| !*on || i.should_interrupt_composition);
                *on = ime.is_some();
                reset
            };
            let mut on = true;
            h.event(key(egui::Key::Tab));
            let mut back = 0;
            for _ in 0..4 {
                h.step();
                back += usize::from(reset(&h, &mut on));
            }
            assert!((1..=2).contains(&back), "{name}: Tab and back reset the input method {back} times");
            for frame in 0..4 {
                h.step();
                assert!(!reset(&h, &mut on), "{name}: frame {frame} after Tab resets the input method");
            }
            h.event(key(egui::Key::C));
            h.event(egui::Event::Text("c".into()));
            h.step();
            assert_eq!(typed(h.state().dialog.as_ref()), Some("abc"), "{name}: the box has the keyboard back after Tab");
        }
    }

    /// Open the Paragraph dialog and press OK without changing anything.
    fn ok_unchanged(app: &mut WordApp) {
        ok_edit(app, |_| {});
    }

    /// Open the Paragraph dialog, change its options (`opts`) with `edit` and press OK.
    fn ok_with(app: &mut WordApp, edit: impl FnOnce(&mut [bool; 4])) {
        ok_edit(app, |d| {
            if let Dialog::Paragraph { opts, .. } = d {
                edit(opts);
            }
        });
    }

    /// Open the Paragraph dialog, change it with `edit` and press OK.
    fn ok_edit(app: &mut WordApp, edit: impl FnOnce(&mut Dialog)) {
        let Some(mut d) = Dialog::open("paragraph", app) else { panic!("no paragraph dialog") };
        edit(&mut d);
        apply_paragraph(app, &d);
    }

    /// The `i`th paragraph's own formatting.
    fn para_props(app: &WordApp, i: usize) -> wordcraft_doc::props::ParaProps {
        app.session.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(i)).map(|p| p.props.clone()).unwrap_or_default()
    }

    /// Three paragraphs, the first centred, indented, 30 pt after, kept with the next and with a
    /// line spacing of 6 (more than the dialog's box goes up to), all of them selected.
    fn mixed_selection() -> WordApp {
        let mut app = app_with("One\nTwo\nThree");
        let _ = app.run("caret.docStart", json!({}));
        let props = json!({"align": "center", "indentLeft": 36, "spaceAfter": 30, "keepNext": true, "lineSpacing": {"rule": "multiple", "value": 6}});
        let _ = app.run("para.set", json!({ "props": props }));
        let _ = app.run("select.all", json!({}));
        app
    }

    #[test]
    fn paragraph_dialog_leaves_what_the_selected_paragraphs_differ_in() {
        // #1346: OK gave every selected paragraph the caret paragraph's formatting, as three
        // undo steps.
        let mut app = mixed_selection();
        let was: Vec<_> = (0..3).map(|i| para_props(&app, i)).collect();
        let Some(Dialog::Paragraph { mixed, .. }) = Dialog::open("paragraph", &mut app) else { panic!() };
        let differ = ["align", "keepNext", "left", "lineSpacing", "spaceAfter"].map(String::from);
        assert_eq!(mixed, BTreeSet::from(differ), "the fields the paragraphs differ in show blank");
        let depth = app.session.undo_depth();
        ok_unchanged(&mut app);
        assert_eq!((0..3).map(|i| para_props(&app, i)).collect::<Vec<_>>(), was, "an unchanged OK changes nothing");
        assert_eq!(app.session.undo_depth(), depth, "nor adds an undo step");
        // One field changed: just that one, in every paragraph, as one undo step.
        ok_edit(&mut app, |d| {
            if let Dialog::Paragraph { before, .. } = d {
                *before = 6.0;
            }
        });
        assert_eq!(app.session.undo_depth(), depth + 1, "one undo step");
        for (i, was) in was.iter().enumerate() {
            assert_eq!(para_props(&app, i), wordcraft_doc::props::ParaProps { space_before: Some(6.0), ..was.clone() }, "paragraph {i}");
        }
        let _ = app.run("edit.undo", json!({}));
        assert_eq!((0..3).map(|i| para_props(&app, i)).collect::<Vec<_>>(), was, "undo takes all of it back");
    }

    /// #1346: a check box the selected paragraphs differ in shows as neither on nor off; a click
    /// and OK turn it on in all of them, and the other fields they differ in stay as they were.
    /// #3028: the records dialog refuses a range with no records itself, before Save Changes asks
    /// about the unsaved document; a good range goes on to that question.
    #[test]
    fn merge_records_dialog_checks_the_range_before_asking_to_save() {
        use egui_kittest::kittest::Queryable;
        let mut app = app_with("Dear ");
        let _ = app.run("mailings.recipients", json!({"csv": "Name\nAda\nAlan\nGrace"}));
        let _ = app.run("text.insert", json!({"text": "unsaved"}));
        assert!(app.session.dirty);
        app.dialog = Dialog::open("mergeRecords", &mut app);
        if let Some(Dialog::MergeRecords { choice, from, to, .. }) = &mut app.dialog {
            *choice = 2;
            *from = "3".into();
            *to = "2".into();
        }
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
        for _ in 0..2 {
            h.step();
        }
        let Some(Dialog::MergeRecords { message, .. }) = &h.state().dialog else { panic!("the dialog stays open") };
        assert!(message.contains("no records from 3 to 2"), "{message}");
        if let Some(Dialog::MergeRecords { to, .. }) = &mut h.state_mut().dialog {
            *to = "3".into();
        }
        for _ in 0..2 {
            h.step();
        }
        h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
        for _ in 0..2 {
            h.step();
        }
        assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("saveChanges"));
        assert!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body).contains("unsaved"), "nothing merged yet");
    }

    #[test]
    fn paragraph_dialog_sets_a_mixed_check_box_for_all() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut app = mixed_selection();
        app.dialog = Dialog::open("paragraph", &mut app);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        // The line spacing box doesn't pull 6 into its range: still blank, still left alone.
        let Some(Dialog::Paragraph { mixed, .. }) = &h.state().dialog else { panic!("no paragraph dialog") };
        assert!(mixed.contains("lineSpacing"), "{mixed:?}");
        if let Some(Dialog::Paragraph { tab, .. }) = &mut h.state_mut().dialog {
            *tab = 1;
        }
        for _ in 0..4 {
            h.step();
        }
        // Screen readers hear it as mixed.
        let toggled = |label: &str| format!("{:?}", h.get_by_label(label).accesskit_node().toggled());
        assert_eq!(toggled("Keep with next"), "Some(Mixed)", "neither on nor off");
        assert_eq!(toggled("Keep lines together"), "Some(False)", "shared");
        h.get_by_label("Keep with next").click();
        for _ in 0..2 {
            h.step();
        }
        h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
        for _ in 0..2 {
            h.step();
        }
        let app = h.state();
        assert!(app.dialog.is_none(), "OK closes the dialog");
        let p: Vec<_> = (0..3).map(|i| para_props(app, i)).collect();
        assert_eq!(p.iter().map(|p| p.keep_next).collect::<Vec<_>>(), [Some(true); 3]);
        assert_eq!(p.iter().map(|p| p.align).collect::<Vec<_>>(), [Some(Align::Center), None, None], "alignment untouched");
        assert_eq!(p.iter().map(|p| p.space_after).collect::<Vec<_>>(), [Some(30.0), None, None], "spacing untouched");
        assert_eq!(p[0].line_spacing, Some(wordcraft_doc::props::LineSpacing::Multiple(6.0)), "line spacing untouched");
    }

    #[test]
    fn paragraph_dialog_sets_only_the_options_that_changed() {
        // #512: mirror indents, same-style spacing, suppress line numbers, don't hyphenate.
        let mut app = app_with("Hello");
        ok_unchanged(&mut app);
        let p = props(&app);
        assert_eq!((p.mirror_indents, p.contextual_spacing, p.suppress_line_numbers, p.suppress_hyphens), (None, None, None, None));
        ok_with(&mut app, |o| *o = [true; 4]);
        let p = props(&app);
        assert_eq!(
            (p.mirror_indents, p.contextual_spacing, p.suppress_line_numbers, p.suppress_hyphens),
            (Some(true), Some(true), Some(true), Some(true))
        );
        let Some(Dialog::Paragraph { opts, .. }) = Dialog::open("paragraph", &mut app) else { panic!() };
        assert_eq!(opts, [true; 4], "the dialog shows what is set");
        ok_with(&mut app, |o| o[3] = false);
        assert_eq!(props(&app).suppress_hyphens, Some(false));
        assert_eq!(props(&app).mirror_indents, Some(true));
    }

    #[test]
    fn paragraph_dialog_shows_direction_and_visual_alignment() {
        let mut app = app_with("سلام دنیا");
        let _ = app.run("para.rtl", json!({}));
        let _ = app.run("para.set", json!({"props": {"indentLeft": 36.0}}));
        let Some(Dialog::Paragraph { rtl, align, left, right, .. }) = Dialog::open("paragraph", &mut app) else { panic!() };
        assert!(rtl);
        assert_eq!(align, "right", "a start-aligned right-to-left paragraph shows as right aligned");
        assert_eq!((left, right), (0.0, 0.5), "the start indent is the right one");
    }

    #[test]
    fn paragraph_dialog_ok_keeps_what_it_shows() {
        for (rtl, align) in [(false, "para.alignCenter"), (false, "para.alignRight"), (true, "para.alignLeft"), (true, "para.alignRight")] {
            let mut app = app_with("متن");
            let _ = app.run(if rtl { "para.rtl" } else { "para.ltr" }, json!({}));
            let _ = app.run(align, json!({}));
            let _ = app.run("para.set", json!({"props": {"indentLeft": 18.0, "indentRight": 9.0}}));
            let before = props(&app);
            ok_unchanged(&mut app);
            let after = props(&app);
            assert_eq!((after.bidi, after.align), (before.bidi, before.align), "{align} in rtl={rtl}");
            assert_eq!((after.indent_left, after.indent_right), (Some(18.0), Some(9.0)), "{align} in rtl={rtl}");
        }
    }

    #[test]
    fn paragraph_dialog_keeps_fixed_line_spacing() {
        use wordcraft_doc::props::LineSpacing;
        for (cmd, rule) in [(json!({"exactly": 24}), LineSpacing::Exactly(24.0)), (json!({"atLeast": 24}), LineSpacing::AtLeast(24.0))] {
            // OK without changes, and OK after changing only Keep with next (#467).
            let mut app = app_with("Example");
            let _ = app.run("para.lineSpacing", cmd.clone());
            ok_unchanged(&mut app);
            assert_eq!(props(&app).line_spacing, Some(rule), "unchanged OK, {cmd}");
            ok_edit(&mut app, |d| {
                if let Dialog::Paragraph { keep_next, .. } = d {
                    *keep_next = true;
                }
            });
            let p = props(&app);
            assert_eq!((p.line_spacing, p.keep_next), (Some(rule), Some(true)), "Keep with next, {cmd}");
        }
        // A changed value still sets a multiple.
        let mut app = app_with("Example");
        let _ = app.run("para.lineSpacing", json!({"exactly": 24}));
        ok_edit(&mut app, |d| {
            if let Dialog::Paragraph { line, .. } = d {
                *line = 1.5;
            }
        });
        assert_eq!(props(&app).line_spacing, Some(LineSpacing::Multiple(1.5)));
    }

    #[test]
    fn font_dialog_sets_the_asian_and_latin_fonts_separately() {
        // #505: the Asian text font box and the Latin text font box each set their own font, and
        // OK leaves the one not changed as it was.
        let mut app = app_with("Hello 你好");
        let _ = app.run("select.all", json!({}));
        let fonts = |app: &WordApp| {
            let c = app.session.doc.para_at(&Pos::body(0, 0)).map(|p| p.props_of_char(0).clone()).unwrap_or_default();
            (c.font, c.font_ea)
        };
        let latin = fonts(&app).0;
        let Some(mut d) = Dialog::open("font", &mut app) else { panic!("no font dialog") };
        let Dialog::Font { asian, font_ea, .. } = &mut d else { panic!() };
        assert!(*asian, "Chinese text in the selection shows the Asian text font box");
        *font_ea = "SimSun".into();
        apply_font(&mut app, &d);
        assert_eq!(fonts(&app), (latin, Some("SimSun".into())));
        let Some(mut d) = Dialog::open("font", &mut app) else { panic!() };
        let Dialog::Font { font, font_ea, .. } = &mut d else { panic!() };
        assert_eq!(font_ea, "SimSun");
        *font = "Georgia".into();
        apply_font(&mut app, &d);
        assert_eq!(fonts(&app), (Some("Georgia".into()), Some("SimSun".into())));
        // One undo step each.
        let _ = app.run("edit.undo", json!({}));
        assert_eq!(fonts(&app).1.as_deref(), Some("SimSun"));
        assert_ne!(fonts(&app).0.as_deref(), Some("Georgia"));
    }

    #[test]
    fn paragraph_dialog_changes_direction() {
        let mut app = app_with("Hello");
        // Right to left, aligned right on the page, 1" from the right edge.
        ok_edit(&mut app, |d| {
            if let Dialog::Paragraph { rtl, align, right, .. } = d {
                *rtl = true;
                *align = "right".into();
                *right += 1.0;
            }
        });
        let p = props(&app);
        assert_eq!(p.bidi, Some(true));
        assert_eq!(p.align, Some(Align::Left), "right on the page is the start edge");
        assert_eq!(p.indent_left, Some(72.0), "the right indent is the start indent");
        // A new direction alone keeps the alignment and indents where the dialog shows them.
        ok_edit(&mut app, |d| {
            if let Dialog::Paragraph { rtl, .. } = d {
                *rtl = false;
            }
        });
        let p = props(&app);
        assert_eq!((p.bidi, p.align, p.indent_left, p.indent_right), (Some(false), Some(Align::Right), Some(0.0), Some(72.0)));
    }

    /// Switching the Paragraph dialog's tabs re-centred it, so the tab strip jumped up and down
    /// by half the height difference (#507): the top edge now stays put and only the bottom moves.
    #[test]
    fn switching_paragraph_tabs_keeps_the_top_edge() {
        use egui_kittest::kittest::Queryable;
        let mut app = app_with("Some text");
        app.dialog = Dialog::open("paragraph", &mut app);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        let mut tops = vec![];
        let mut oks = vec![];
        for page in [0u8, 1, 2, 0] {
            if let Some(Dialog::Paragraph { tab, .. }) = &mut h.state_mut().dialog {
                *tab = page;
            }
            for _ in 0..4 {
                h.step();
            }
            tops.push(h.get_by_label("Indents and Spacing").rect().top());
            oks.push(h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").rect().top());
        }
        assert!((oks[0] - oks[1]).abs() > 10.0, "the tabs differ in height: {oks:?}");
        for t in &tops {
            assert!((t - tops[0]).abs() <= 0.5, "the tab strip stays put: {tops:?}");
        }
    }

    /// #3663: "NaN" in the Font dialog's Size box parses as a float; the preview draws at 12 pt
    /// instead of a NaN size, which tripped epaint's glyph assertion and closed the app.
    #[test]
    fn font_dialog_preview_ignores_a_size_that_is_not_a_number() {
        for (typed, pt) in [("NaN", 12.0), ("nan", 12.0), ("inf", 12.0), ("-infinity", 12.0), ("10.5", 10.5), ("200", 72.0), ("", 12.0)] {
            assert_eq!(preview_size(typed), pt, "{typed:?}");
        }
        let mut app = app_with("Example");
        let _ = app.run("select.all", json!({}));
        app.dialog = Dialog::open("font", &mut app);
        let Some(Dialog::Font { size, .. }) = &mut app.dialog else { panic!("no font dialog") };
        *size = "NaN".into();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        assert!(matches!(&h.state().dialog, Some(Dialog::Font { size, .. }) if size == "NaN"), "the dialog stays open");
    }

    #[test]
    fn font_dialog_ok_keeps_what_the_user_left_alone() {
        // Issue #466: OK sent every field, clearing small caps, spacing and double underline
        // when only the size changed.
        let mut app = app_with("Example");
        let _ = app.run("select.all", json!({}));
        let _ = app.run("format.set", json!({"props": {"smallCaps": true, "caps": true, "hidden": true, "spacing": 2, "underline": "double"}}));
        let Some(mut d) = Dialog::open("font", &mut app) else { panic!("no font dialog") };
        if let Dialog::Font { size, small_caps, spacing, .. } = &mut d {
            assert!(*small_caps, "the dialog shows small caps");
            assert_eq!(*spacing, 2.0);
            *size = "14".into();
        }
        apply_font(&mut app, &d);
        let c = app.session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).map(|p| p.props_of_char(0).clone()).unwrap_or_default();
        assert_eq!(c.size, Some(14.0));
        assert_eq!((c.small_caps, c.caps, c.hidden, c.spacing), (Some(true), Some(true), Some(true), Some(2.0)));
        assert_eq!(c.underline, Some(wordcraft_doc::props::Underline::Double));
        // A field the user does change still applies.
        let Some(mut d) = Dialog::open("font", &mut app) else { panic!("no font dialog") };
        if let Dialog::Font { small_caps, .. } = &mut d {
            *small_caps = false;
        }
        apply_font(&mut app, &d);
        let c = app.session.doc.para_at(&wordcraft_doc::Pos::body(0, 0)).map(|p| p.props_of_char(0).clone()).unwrap_or_default();
        assert_eq!((c.small_caps, c.caps, c.size), (Some(false), Some(true), Some(14.0)));
    }

    /// Kerning for fonts and its Points and above size set `w:kern` on the selection in one undo
    /// step; off is 0. Text that differs shows the box neither on nor off, and OK leaves each
    /// run's own kerning.
    #[test]
    fn font_dialog_sets_kerning_for_fonts() {
        let mut app = app_with("Kerned plain");
        let _ = app.run("select.all", json!({}));
        let _ = app.run("format.set", json!({"props": {"kern": 0}}));
        let kern = |app: &WordApp, off| app.session.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.props_of_char(off).kern);
        let Some(mut d) = Dialog::open("font", &mut app) else { panic!("no font dialog") };
        let Dialog::Font { kern: on, kern_at, .. } = &mut d else { panic!() };
        assert_eq!((*on, *kern_at), (Some(false), 12.0), "off, offering 12 pt");
        *on = Some(true);
        *kern_at = 16.0;
        apply_font(&mut app, &d);
        assert_eq!((kern(&app, 0), kern(&app, 7)), (Some(16.0), Some(16.0)));
        let _ = app.run("edit.undo", json!({}));
        assert_eq!(kern(&app, 0), Some(0.0), "one undo step");
        let _ = app.run("edit.redo", json!({}));
        let Some(mut d) = Dialog::open("font", &mut app) else { panic!() };
        let Dialog::Font { kern: on, kern_at, .. } = &mut d else { panic!() };
        assert_eq!((*on, *kern_at), (Some(true), 16.0));
        *on = Some(false);
        apply_font(&mut app, &d);
        assert_eq!((kern(&app, 0), kern(&app, 7)), (Some(0.0), Some(0.0)));
        let _ = app.run("select.text", json!({"text": "Kerned"}));
        let _ = app.run("format.set", json!({"props": {"kern": 14}}));
        let _ = app.run("select.all", json!({}));
        let Some(mut d) = Dialog::open("font", &mut app) else { panic!() };
        let Dialog::Font { kern: on, size, .. } = &mut d else { panic!() };
        assert_eq!(*on, None, "mixed kerning shows neither on nor off");
        *size = "13".into();
        apply_font(&mut app, &d);
        assert_eq!((kern(&app, 0), kern(&app, 7)), (Some(14.0), Some(0.0)), "each run keeps its kerning");
    }

    /// #1178: Modify Style's font box is the ribbon's font box, so typing part of a name picks the
    /// installed family instead of storing the typed text.
    #[test]
    fn modify_style_font_box_finds_the_family() {
        use egui_kittest::kittest::{By, Queryable};
        let mut app = app_with("Example");
        app.dialog = Dialog::modify_style(&app, "Normal");
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
            move |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                if fonts {
                    show(app, &ctx);
                } else {
                    crate::theme::install_fonts_for(&ctx, false);
                    fonts = true;
                }
            },
            app,
        );
        h.run_steps(3);
        h.get(By::new().role(egui::accesskit::Role::TextInput).value(wordcraft_doc::styles::BODY_FONT)).focus();
        h.step();
        let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        h.event(key(egui::Key::A, egui::Modifiers::COMMAND));
        h.step();
        h.event(egui::Event::Text("aptos disp".into()));
        h.step();
        // Enter both picks the highlighted family and presses OK, as in the Font dialog.
        h.event(key(egui::Key::Enter, egui::Modifiers::NONE));
        h.run_steps(2);
        assert!(h.state().dialog.is_none(), "Enter pressed OK");
        let rc = h.state().session.doc.styles.resolve_char(Some("Normal"), &wordcraft_doc::CharProps::default());
        assert_eq!(rc.font, wordcraft_doc::styles::HEADING_FONT);
    }

    /// A dialog was pinned by its anchor, so dragging its title bar did nothing (#732).
    #[test]
    fn dragging_a_dialog_title_bar_moves_it() {
        let mut app = app_with("Some text");
        app.dialog = Dialog::open("paragraph", &mut app);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        let id = egui::Id::new(("dialog", "Paragraph"));
        let before = h.ctx.memory(|m| m.area_rect(id)).expect("dialog is open");
        let screen = h.ctx.content_rect();
        assert!((before.center().x - screen.center().x).abs() <= 1.0, "opens centred: {before:?} in {screen:?}");
        let grab = egui::pos2(before.center().x, before.top() + 10.0);
        let drop = grab + vec2(-200.0, 120.0);
        let button =
            |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        h.event(egui::Event::PointerMoved(grab));
        h.step();
        h.event(button(grab, true));
        h.step();
        h.event(egui::Event::PointerMoved(drop));
        h.step();
        h.event(button(drop, false));
        for _ in 0..4 {
            h.step();
        }
        let after = h.ctx.memory(|m| m.area_rect(id)).expect("dialog is still open");
        assert!((after.min.x - (before.min.x - 200.0)).abs() <= 1.0, "moved left: {before:?} -> {after:?}");
        assert!((after.min.y - (before.min.y + 120.0)).abs() <= 1.0, "moved down: {before:?} -> {after:?}");
    }

    /// #1885: the close control is the whole box at the title bar's right end, as on a Windows
    /// dialog, not only the ✕. A click 6 px from the window's right edge, mid title bar, closes
    /// the dialog; one just under the title bar leaves it open; a drag that starts on the box
    /// doesn't move the dialog. The box spans the title bar egui lays out.
    #[test]
    fn the_title_bar_close_box_is_the_whole_corner() {
        fn press(h: &mut egui_kittest::Harness<'_, WordApp>, from: egui::Pos2, to: egui::Pos2) {
            let button =
                |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            h.event(egui::Event::PointerMoved(from));
            h.step();
            h.event(button(from, true));
            h.step();
            if to != from {
                h.event(egui::Event::PointerMoved(to));
                h.step();
            }
            h.event(button(to, false));
            for _ in 0..4 {
                h.step();
            }
        }
        let mut app = app_with("Some text");
        app.dialog = Dialog::open("wordCount", &mut app);
        let mut h = app_harness(app);
        for _ in 0..4 {
            h.step();
        }
        let id = egui::Id::new(("dialog", "Word Count"));
        let before = h.ctx.memory(|m| m.area_rect(id)).expect("dialog is open");
        let bx = crate::window_close::close_rect(before, 1.0, crate::window_close::geometry(&h.ctx));
        let title = h.ctx.read_response(id.with("__title_click")).expect("egui's title bar").rect;
        assert!((bx.top() - before.top()).abs() <= 1.5 && (bx.bottom() - title.bottom()).abs() <= 1.5, "box {bx:?} spans the title bar {title:?}");
        assert!(bx.width() >= 1.5 * bx.height() && bx.right() + 1.5 >= before.right(), "a wide box at the right end: {bx:?} in {before:?}");
        let under = egui::pos2(before.right() - 6.0, bx.bottom() + 4.0);
        press(&mut h, under, under);
        assert!(h.state().dialog.is_some(), "a click under the title bar keeps the dialog");
        let at = egui::pos2(before.right() - 6.0, bx.center().y);
        press(&mut h, at, at + vec2(-150.0, 90.0));
        assert!(h.state().dialog.is_some(), "a drag from the box isn't a click");
        let after = h.ctx.memory(|m| m.area_rect(id)).expect("dialog is still open");
        assert!((after.min - before.min).length() <= 1.0, "the box isn't a drag handle: {before:?} -> {after:?}");
        press(&mut h, at, at);
        assert!(h.state().dialog.is_none(), "a click in the box, away from the X, closes the dialog");
    }

    fn app_harness(app: WordApp) -> egui_kittest::Harness<'static, WordApp> {
        egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        )
    }

    /// The Page Setup paper size as opened, in points, and the Paper size entry it shows.
    fn opened_paper(app: &mut WordApp) -> ((f32, f32), Option<&'static str>) {
        let Some(Dialog::PageSetup { paper, .. }) = Dialog::open("pageSetup", app) else { panic!("no page setup dialog") };
        let ppu = crate::options::unit().pt_per_unit();
        ((paper.0 * ppu, paper.1 * ppu), paper_entry(paper))
    }

    /// #1433: Page Setup's Paper size shows the named size the section has, in either
    /// orientation, and Custom size (None) for anything else.
    #[test]
    fn page_setup_reads_the_paper_size() {
        let mut app = app_with("Page");
        let sect = |app: &WordApp| wordcraft_engine::cmd::page::sect(&app.session);
        let ((w, h), entry) = opened_paper(&mut app);
        assert!((w - sect(&app).page_w).abs() < 0.01 && (h - sect(&app).page_h).abs() < 0.01, "{w}×{h}");
        assert_eq!(entry, wordcraft_geom::paper_name(w, h));
        assert!(entry.is_some(), "a new document has a named size");
        let _ = app.run("layout.size", json!({"name": "A4"}));
        let _ = app.run("layout.orientation", json!({"value": "landscape"}));
        let ((w, h), entry) = opened_paper(&mut app);
        assert!(w > h, "landscape shows the wide side as the width: {w}×{h}");
        assert_eq!(entry, Some("A4"));
        let _ = app.run("layout.size", json!({"width": 300.0, "height": 500.0}));
        assert_eq!(opened_paper(&mut app).1, None, "Custom size");
    }

    /// #1433: a custom width and height applied with OK set the section's size (before the
    /// orientation, so a wide custom size ends up landscape); an unchanged size doesn't run
    /// `layout.size`, and switching orientation alone only turns the page.
    #[test]
    fn page_setup_applies_a_custom_paper_size() {
        use egui_kittest::kittest::Queryable;
        let ppu = crate::options::unit().pt_per_unit();
        let size_steps = |app: &WordApp| app.session.undo_labels().iter().filter(|l| l.as_str() == "Size").count();
        let sect = |app: &WordApp| {
            let s = wordcraft_engine::cmd::page::sect(&app.session);
            (s.page_w.round(), s.page_h.round(), s.landscape)
        };
        // OK on an unchanged dialog leaves the size alone.
        let mut h = app_harness(app_with("Page"));
        let before = sect(h.state());
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().dialog.is_none());
        assert_eq!((sect(h.state()), size_steps(h.state())), (before, 0), "no layout.size for an unchanged size");

        for ((w, ht), want) in [((400.0, 600.0), (400.0, 600.0, false)), ((800.0, 400.0), (800.0, 400.0, true))] {
            h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
            h.run_steps(2);
            if let Some(Dialog::PageSetup { paper, landscape, .. }) = h.state_mut().dialog.as_mut() {
                *paper = (w / ppu, ht / ppu);
                *landscape = w > ht;
            }
            h.run_steps(2);
            h.get_by_label("OK").click();
            h.run_steps(3);
            assert_eq!(sect(h.state()), want, "{w}×{ht}");
        }
        assert_eq!(size_steps(h.state()), 2);

        // Portrait again with the same paper: the dialog swaps width and height, which is no new size.
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(2);
        if let Some(Dialog::PageSetup { paper, landscape, .. }) = h.state_mut().dialog.as_mut() {
            *paper = (paper.1, paper.0);
            *landscape = false;
        }
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert_eq!(sect(h.state()), (400.0, 800.0, false));
        assert_eq!(size_steps(h.state()), 2, "turning the page isn't a new size");
    }

    /// #1433: the Paper group's own widgets: choosing A4 in Paper size fills Width and Height,
    /// Landscape swaps them, OK gives an A4 landscape section; typing a width wider than the
    /// height turns the orientation to Landscape and shows Custom size; choosing Custom size
    /// keeps the numbers and stays chosen.
    #[test]
    fn page_setup_paper_widgets_work_together() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::Queryable;
        let ppu = crate::options::unit().pt_per_unit();
        let mut h = app_harness(app_with("Page"));
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(3);
        let state = |h: &egui_kittest::Harness<'_, WordApp>| match &h.state().dialog {
            Some(Dialog::PageSetup { paper, landscape, .. }) => {
                ((paper.0 * ppu * 100.0).round() / 100.0, (paper.1 * ppu * 100.0).round() / 100.0, *landscape)
            }
            _ => panic!("no page setup dialog"),
        };
        let combo = |h: &egui_kittest::Harness<'_, WordApp>| h.get_by_role_and_label(Role::ComboBox, "Paper size:").value();
        assert_eq!(combo(&h).as_deref(), Some("Letter"));
        h.get_by_role_and_label(Role::ComboBox, "Paper size:").click();
        h.run_steps(3);
        h.get_by_label("A4").click();
        h.run_steps(3);
        assert_eq!(state(&h), (595.28, 841.89, false), "A4 fills the fields");
        assert_eq!(combo(&h).as_deref(), Some("A4"));
        h.get_by_label("Landscape").click();
        h.run_steps(3);
        assert_eq!(state(&h), (841.89, 595.28, true), "Landscape swaps them");
        assert_eq!(combo(&h).as_deref(), Some("A4"));
        h.get_by_label("OK").click();
        h.run_steps(3);
        let sect = wordcraft_engine::cmd::page::sect(&h.state().session);
        assert_eq!(((sect.page_w * 100.0).round() / 100.0, (sect.page_h * 100.0).round() / 100.0, sect.landscape), (841.89, 595.28, true));

        // Portrait A4 again; a width wider than the height turns it to Landscape, Custom size.
        h.state_mut().run("layout.orientation", json!({"value": "portrait"})).unwrap();
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(3);
        h.get_by_value("8.27\"").focus();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("15".into()));
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(state(&h), (1080.0, 841.89, true), "wider than tall: Landscape");
        assert_eq!(combo(&h).as_deref(), Some("Custom size"));

        // Back to A4, then Custom size: the numbers stay and so does the choice.
        h.get_by_role_and_label(Role::ComboBox, "Paper size:").click();
        h.run_steps(3);
        h.get_by_label("A4").click();
        h.run_steps(3);
        h.get_by_role_and_label(Role::ComboBox, "Paper size:").click();
        h.run_steps(3);
        h.get_by_label("Custom size").click();
        h.run_steps(3);
        assert_eq!(state(&h), (841.89, 595.28, true));
        assert_eq!(combo(&h).as_deref(), Some("Custom size"));
    }

    /// #1433: Page Setup checks the new size and margins together: a page its margins don't fit
    /// keeps the dialog open with a message and applies nothing; a smaller page with smaller
    /// margins applies (margins first, so no step is refused on the way).
    #[test]
    fn page_setup_refuses_margins_that_dont_fit_and_applies_smaller_ones_first() {
        use egui_kittest::kittest::Queryable;
        let ppu = crate::options::unit().pt_per_unit();
        let mut h = app_harness(app_with("Page"));
        let before = wordcraft_engine::cmd::page::sect(&h.state().session);
        let undo = h.state().session.undo_depth();
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(2);
        if let Some(Dialog::PageSetup { paper, .. }) = h.state_mut().dialog.as_mut() {
            *paper = (100.0 / ppu, 500.0 / ppu);
        }
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(
            matches!(&h.state().dialog, Some(Dialog::PageSetup { message, .. }) if message.contains("margins don't fit")),
            "stays open with a message"
        );
        assert_eq!((wordcraft_engine::cmd::page::sect(&h.state().session), h.state().session.undo_depth()), (before, undo), "nothing applied");

        if let Some(Dialog::PageSetup { top, bottom, left, right, .. }) = h.state_mut().dialog.as_mut() {
            (*top, *bottom, *left, *right) = (18.0 / ppu, 18.0 / ppu, 18.0 / ppu, 18.0 / ppu);
        }
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(h.state().dialog.is_none());
        let s = wordcraft_engine::cmd::page::sect(&h.state().session);
        assert_eq!((s.page_w.round(), s.page_h.round(), s.margin_left.round(), s.margin_top.round()), (100.0, 500.0, 18.0, 18.0));
    }

    /// Open Page Setup in `h`, set its paper and margins (points) and press OK.
    fn page_setup_ok(h: &mut egui_kittest::Harness<'_, WordApp>, (w, ht): (f32, f32), (t, b, l, r): (f32, f32, f32, f32)) {
        use egui_kittest::kittest::Queryable;
        let ppu = crate::options::unit().pt_per_unit();
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(2);
        if let Some(Dialog::PageSetup { paper, landscape, top, bottom, left, right, .. }) = h.state_mut().dialog.as_mut() {
            *paper = (w / ppu, ht / ppu);
            *landscape = w > ht;
            (*top, *bottom, *left, *right) = (t / ppu, b / ppu, l / ppu, r / ppu);
        }
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
    }

    /// #1433: one OK in Page Setup is one undo step, whichever order its commands run in (the
    /// size first; the margins first; margins of 0 in between), and Undo restores the section.
    #[test]
    fn page_setup_ok_is_one_undo_step() {
        let sect = |h: &egui_kittest::Harness<'_, WordApp>| wordcraft_engine::cmd::page::sect(&h.state().session);
        // Size first, then orientation and margins.
        let mut h = app_harness(app_with("Page"));
        let (before, depth) = (sect(&h), h.state().session.undo_depth());
        page_setup_ok(&mut h, (400.0, 600.0), (36.0, 36.0, 36.0, 36.0));
        assert!(h.state().dialog.is_none());
        assert_eq!((sect(&h).page_w, sect(&h).margin_left), (400.0, 36.0));
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one step");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(sect(&h), before, "one undo restores the section");

        // Neither size first nor margins first fits on the way: margins of 0 in between.
        h.state_mut().run("layout.margins", json!({"top": 72.0, "bottom": 72.0, "left": 250.0, "right": 250.0})).unwrap();
        let (before, depth) = (sect(&h), h.state().session.undo_depth());
        page_setup_ok(&mut h, (300.0, 900.0), (400.0, 400.0, 36.0, 36.0));
        assert!(h.state().dialog.is_none());
        let s = sect(&h);
        assert_eq!((s.page_w, s.page_h, s.margin_top, s.margin_left), (300.0, 900.0, 400.0, 36.0));
        assert_eq!(h.state().session.undo_depth(), depth + 1, "one step");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(sect(&h), before, "one undo restores the section");
    }

    /// #1433: a command the dialog runs that fails (here `layout.size`, refused by the second
    /// selected section's margins) shows its error, keeps the dialog open and leaves the
    /// document and the undo stack as they were.
    #[test]
    fn page_setup_shows_a_refused_command_and_applies_nothing() {
        let mut app = app_with("one");
        app.run("layout.break", json!({"kind": "nextPage"})).unwrap();
        app.run("text.insert", json!({"text": "two"})).unwrap();
        app.run("layout.margins", json!({"left": 150.0, "right": 150.0})).unwrap();
        // Both sections selected, the first one's settings in the dialog.
        app.run("select.all", json!({})).unwrap();
        let sel = &mut app.session.sel;
        std::mem::swap(&mut sel.anchor, &mut sel.focus);
        let doc = app.session.doc.sections().iter().map(|(_, x)| (*x).clone()).collect::<Vec<_>>();
        let depth = app.session.undo_depth();
        let mut h = app_harness(app);
        page_setup_ok(&mut h, (300.0, 500.0), (72.0, 72.0, 72.0, 72.0));
        assert!(
            matches!(&h.state().dialog, Some(Dialog::PageSetup { message, .. }) if message.contains("margins don't fit")),
            "stays open with the error"
        );
        let after = h.state().session.doc.sections().iter().map(|(_, x)| (*x).clone()).collect::<Vec<_>>();
        assert_eq!((after, h.state().session.undo_depth()), (doc, depth), "nothing applied");
    }

    /// #1433: Layout › Size › More Paper Sizes… opens Page Setup.
    #[test]
    fn more_paper_sizes_opens_page_setup() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(400.0, 600.0))
            .build_ui_state(|ui, app: &mut WordApp| crate::ribbon::size_menu(ui, app), app_with("Page"));
        h.run_steps(2);
        h.get(egui_kittest::kittest::By::new().label_contains("More Paper Sizes…")).click();
        h.run_steps(2);
        assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("pageSetup"));
    }

    /// #1320: Page Setup shows the section's vertical alignment, and OK applies a change.
    #[test]
    fn page_setup_dialog_sets_vertical_alignment() {
        use egui_kittest::kittest::Queryable;
        use wordcraft_doc::section::PageVAlign;
        let mut app = app_with("Some text");
        let _ = app.run("layout.verticalAlign", json!({"value": "bottom"}));
        app.dialog = Dialog::open("pageSetup", &mut app);
        assert!(matches!(app.dialog, Some(Dialog::PageSetup { valign: PageVAlign::Bottom, .. })), "shows the section's value");
        if let Some(Dialog::PageSetup { valign, .. }) = &mut app.dialog {
            *valign = PageVAlign::Justify;
        }
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        assert!(h.query_by_label("Vertical alignment:").is_some(), "the dialog has the control");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
        for _ in 0..4 {
            h.step();
        }
        assert_eq!(wordcraft_engine::cmd::page::sect(&h.state().session).valign, PageVAlign::Justify);
    }

    /// #1230: Page Setup's Different first page check box gives the title page no page number
    /// (its footer is the empty first-page one), in one undo step.
    #[test]
    fn page_setup_different_first_page_leaves_the_title_page_unnumbered() {
        use egui_kittest::kittest::Queryable;
        let mut app = app_with("Title");
        app.run("insert.pageBreak", json!({})).unwrap();
        app.run("text.insert", json!({"text": "Body"})).unwrap();
        app.run("insert.pageNumber", json!({"position": "bottom", "align": "center"})).unwrap();
        let _ = app.run("insert.closeHeader", json!({}));
        assert!(app.session.layout().pages[0].footer_story.is_some(), "page 1 is numbered");
        app.dialog = Dialog::open("pageSetup", &mut app);
        assert!(matches!(app.dialog, Some(Dialog::PageSetup { first_page: false, odd_even: false, .. })));
        let depth = app.session.undo_depth();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        for _ in 0..4 {
            h.step();
        }
        assert!(h.query_by_label("Different Odd & Even Pages").is_some(), "the dialog has both check boxes");
        h.get_by_label("Different First Page").click();
        for _ in 0..2 {
            h.step();
        }
        h.get_by_role_and_label(egui::accesskit::Role::Button, "OK").click();
        for _ in 0..4 {
            h.step();
        }
        let s = &mut h.state_mut().session;
        assert!(wordcraft_engine::cmd::page::sect(s).title_page);
        assert!(!s.doc.settings.even_odd_headers, "left as it was");
        assert_eq!(s.undo_depth(), depth + 1);
        let pages = &s.layout().pages;
        assert!(pages[0].footer_story.is_none(), "the title page shows no page number");
        assert!(pages[1].footer_story.is_some(), "the next page keeps its number");
    }

    /// #2701: in the SmartArt Text Pane, Tab demotes and Shift+Tab promotes the focused item, as
    /// Word's pane does; the row keeps its focus, so the next Tab acts on the same item. An item
    /// that can't go down a level (the one before it isn't deeper) is left as it is.
    #[test]
    fn text_pane_tab_demotes_and_shift_tab_promotes_the_focused_item() {
        use egui_kittest::kittest::Queryable;
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        app.run("insert.smartArt", json!({"layout": "hierarchy"})).unwrap();
        app.run("smartArt.textPane", json!({"value": true})).unwrap();
        let levels = |app: &WordApp| {
            wordcraft_engine::cmd::smart_art::selected_smart_art(&app.session).map(|g| g.items.iter().map(|i| i.level).collect::<Vec<u8>>())
        };
        assert_eq!(levels(&app), Some([0, 1, 2, 1, 2, 2].to_vec()), "Lead, Design, Sketches, Build, Code, Tests");
        let mut h = app_harness(app);
        h.run_steps(2);
        // Focus a pane row: the text box with this value (the graphic drawn on the page shows the
        // same words as shape names, so match by role).
        let focus_row = |h: &mut egui_kittest::Harness<'static, WordApp>, text: &str| {
            let node = h
                .query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::TextInput).value(text))
                .next()
                .unwrap_or_else(|| panic!("no text box for {text}"));
            node.focus();
            h.run_steps(2);
        };
        let tab = |h: &mut egui_kittest::Harness<'static, WordApp>, shift: bool| {
            if shift {
                h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
            } else {
                h.key_press(egui::Key::Tab);
            }
            h.run_steps(2);
        };
        // Focus "Design" (the second top-level item, at level 1): Tab can't demote it, because
        // the item before it ("Lead") is shallower — Word leaves the level alone.
        focus_row(&mut h, "Design");
        assert_eq!(h.state().smart_art_item, Some(1), "the pane remembers the focused row");
        tab(&mut h, false);
        assert_eq!(levels(h.state()), Some([0, 1, 2, 1, 2, 2].to_vec()), "Tab on an item that can't go down changes nothing");
        assert_eq!(h.state().smart_art_item, Some(1), "the row keeps focus after a refused demote");
        // Focus "Build" (level 1 after the deeper "Sketches"): Tab demotes it and its sub-items,
        // and the next Tab (the row stayed focused) demotes them again; Shift+Tab promotes back.
        focus_row(&mut h, "Build");
        tab(&mut h, false);
        assert_eq!(levels(h.state()), Some([0, 1, 2, 2, 3, 3].to_vec()), "Tab demotes the focused item");
        tab(&mut h, false);
        assert_eq!(levels(h.state()), Some([0, 1, 2, 3, 4, 4].to_vec()), "the same row takes the next Tab");
        tab(&mut h, true);
        assert_eq!(levels(h.state()), Some([0, 1, 2, 2, 3, 3].to_vec()), "Shift+Tab promotes them back");
    }
}
