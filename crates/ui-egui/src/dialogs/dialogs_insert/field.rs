use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::text::TextField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{list_index, select_index, text_field, word};
use crate::WordApp;
use crate::dialogs::host::{Asks, WordForm};

// ---------------------------------------------------------------------------------------------
// Field

/// What a field's options are about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Date,
    Number,
    Bookmark,
    Sequence,
    Merge,
    Toc,
    /// STYLEREF: a style and its search and number switches.
    Style,
    Plain,
}

/// The fields WordCraft computes, by category: (name, category, description).
pub const FIELDS: &[(&str, &str, &str)] = &[
    ("CREATEDATE", "Date and Time", "When this document was first created"),
    ("DATE", "Date and Time", "The date it is today"),
    ("PRINTDATE", "Date and Time", "When this document was most recently printed"),
    ("SAVEDATE", "Date and Time", "When this document was most recently saved"),
    ("TIME", "Date and Time", "The time right now"),
    ("AUTHOR", "Document Information", "Who the document properties name as author"),
    ("COMMENTS", "Document Information", "Comments from the document properties"),
    ("FILENAME", "Document Information", "Name of the file this document is saved as"),
    ("KEYWORDS", "Document Information", "Keywords from the document properties"),
    ("LASTSAVEDBY", "Document Information", "Who last saved the document"),
    ("NUMCHARS", "Document Information", "How many characters the document holds"),
    ("NUMPAGES", "Document Information", "How many pages the whole document has"),
    ("NUMWORDS", "Document Information", "How many words the document holds"),
    ("REVNUM", "Document Information", "How many times the document has been saved"),
    ("SUBJECT", "Document Information", "Subject from the document properties"),
    ("TITLE", "Document Information", "Title from the document properties"),
    ("PAGE", "Numbering", "Number of the page the field sits on"),
    ("SECTION", "Numbering", "Number of the section the field sits in"),
    ("SECTIONPAGES", "Numbering", "How many pages this section has"),
    ("SEQ", "Numbering", "A numbered sequence (figures, tables…)"),
    ("PAGEREF", "Links and References", "Page on which a bookmarked passage falls"),
    ("REF", "Links and References", "Copy of the text a bookmark marks"),
    ("STYLEREF", "Links and References", "Text of the nearest paragraph with a given style (running headers)"),
    ("TOC", "Index and Tables", "A table of contents"),
    ("MERGEFIELD", "Mail Merge", "A mail merge field"),
];

/// STYLEREF's options (ECMA-376 §17.16.5.59): switch letter and label.
pub const STYLEREF_SWITCHES: &[(char, &str)] = &[
    ('l', "Take the last match on the page"),
    ('n', "Show its list number"),
    ('r', "Show its number relative to this field"),
    ('w', "Show its number with all higher levels"),
    ('t', "Number only, without the label's words"),
    ('p', "Add whether it is above or below"),
];

/// The categories, in list order ("(All)" first).
pub const FIELD_CATEGORIES: &[&str] =
    &["(All)", "Date and Time", "Document Information", "Links and References", "Index and Tables", "Mail Merge", "Numbering"];

/// Date and time pictures for the `\@` switch.
pub const DATE_FORMATS: &[&str] = &[
    "M/d/yyyy",
    "dddd, MMMM d, yyyy",
    "MMMM d, yyyy",
    "M/d/yy",
    "yyyy-MM-dd",
    "d-MMM-yy",
    "M.d.yyyy",
    "d MMMM yyyy",
    "MMMM yy",
    "MMM-yy",
    "M/d/yyyy h:mm am/pm",
    "M/d/yyyy h:mm:ss am/pm",
    "h:mm am/pm",
    "h:mm:ss am/pm",
    "HH:mm",
    "HH:mm:ss",
];

/// Number formats for the `\*` switch: (switch value, example).
pub const NUMBER_FORMATS: &[(&str, &str)] = &[
    ("", "(none)"),
    ("Arabic", "1, 2, 3, …"),
    ("alphabetic", "a, b, c, …"),
    ("ALPHABETIC", "A, B, C, …"),
    ("roman", "i, ii, iii, …"),
    ("ROMAN", "I, II, III, …"),
    ("Ordinal", "1st, 2nd, 3rd, …"),
    ("CardText", "One, Two, Three, …"),
    ("OrdText", "First, Second, Third, …"),
];

/// Capitalization formats for the `\*` switch of a text field: (switch value, label).
pub const TEXT_FORMATS: &[(&str, &str)] =
    &[("", "(none)"), ("Upper", "Uppercase"), ("Lower", "Lowercase"), ("FirstCap", "First capital"), ("Caps", "Title case")];

/// Whether the dialog offers the capitalization Format list for `name` (the fields whose result
/// is text).
pub fn takes_text_format(name: &str) -> bool {
    matches!(name, "AUTHOR" | "COMMENTS" | "FILENAME" | "KEYWORDS" | "LASTSAVEDBY" | "SUBJECT" | "TITLE" | "REF" | "STYLEREF" | "MERGEFIELD")
}

/// The capitalization switches in a field code, as their [`TEXT_FORMATS`] values, in the order
/// they appear (layout applies them in that order). Quoted text in the code is not a switch.
pub fn text_formats(code: &str) -> Vec<&'static str> {
    let mut chars = code.chars().peekable();
    let mut quoted = false;
    let mut all = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quoted => {
                chars.next();
            }
            '"' => quoted = !quoted,
            '\\' if chars.peek() == Some(&'*') => {
                chars.next();
                while chars.next_if(|c| c.is_whitespace()).is_some() {}
                let mut word = String::new();
                while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '\\' && *c != '"') {
                    word.push(c);
                }
                if let Some((v, _)) = TEXT_FORMATS.iter().find(|(v, _)| !v.is_empty() && v.eq_ignore_ascii_case(&word)) {
                    all.push(*v);
                }
            }
            _ => {}
        }
    }
    all
}

pub fn field_kind(name: &str) -> FieldKind {
    match name {
        "DATE" | "TIME" | "CREATEDATE" | "SAVEDATE" | "PRINTDATE" => FieldKind::Date,
        "PAGE" | "NUMPAGES" | "SECTION" | "SECTIONPAGES" | "NUMWORDS" | "NUMCHARS" | "REVNUM" => FieldKind::Number,
        "REF" | "PAGEREF" => FieldKind::Bookmark,
        "SEQ" => FieldKind::Sequence,
        "MERGEFIELD" => FieldKind::Merge,
        "TOC" => FieldKind::Toc,
        "STYLEREF" => FieldKind::Style,
        _ => FieldKind::Plain,
    }
}

/// The Field dialog: a field by category, its options, and the field code it makes (or the code
/// typed directly with Field Codes on).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldForm {
    pub category: String,
    pub name: String,
    /// `\@` picture for date fields.
    pub date_format: String,
    /// `\*` value for number fields (empty = none).
    pub number_format: String,
    /// `\*` capitalization value for text fields (empty = none).
    pub text_format: String,
    /// Every capitalization switch of the code typed in Field Codes, in order. The code keeps them
    /// all while the Format list shows the last of them.
    #[serde(skip)]
    typed_text_formats: Vec<&'static str>,
    /// Bookmark (REF, PAGEREF), sequence name (SEQ), merge field (MERGEFIELD) or style name
    /// (STYLEREF).
    pub target: String,
    /// STYLEREF switches, as their letters (`l` search from the bottom of the page, `n`
    /// paragraph number, `r` relative context, `w` full context, `t` suppress non-numbers,
    /// `p` above/below).
    pub style_switches: String,
    /// Keep the formatting applied to the result when it updates (`\* MERGEFORMAT`).
    pub preserve: bool,
    /// Field Codes: type the code directly.
    pub codes: bool,
    pub code: String,
    #[serde(skip)]
    bookmarks: Vec<String>,
    #[serde(skip)]
    merge_fields: Vec<String>,
    /// Paragraph and character style names, for STYLEREF.
    #[serde(skip)]
    styles: Vec<String>,
}

impl FieldForm {
    pub fn new(app: &WordApp) -> FieldForm {
        let mut bookmarks: Vec<String> = app.session.doc.bookmarks().into_iter().map(|(n, _)| n).filter(|n| !n.starts_with('_')).collect();
        bookmarks.sort();
        bookmarks.dedup();
        let mut f = FieldForm {
            category: FIELD_CATEGORIES.first().copied().unwrap_or("(All)").to_string(),
            name: "DATE".into(),
            date_format: DATE_FORMATS.first().copied().unwrap_or("M/d/yyyy").to_string(),
            number_format: String::new(),
            text_format: String::new(),
            typed_text_formats: Vec::new(),
            target: String::new(),
            preserve: true,
            codes: false,
            code: String::new(),
            bookmarks,
            merge_fields: app.session.merge.headers.clone(),
            style_switches: String::new(),
            styles: {
                let mut v: Vec<String> = app
                    .session
                    .doc
                    .styles
                    .styles
                    .iter()
                    .filter(|s| matches!(s.kind, wordcraft_doc::StyleKind::Paragraph | wordcraft_doc::StyleKind::Character) && !s.hidden)
                    .map(|s| s.name.clone())
                    .collect();
                v.sort_by_key(|n| n.to_lowercase());
                v.dedup();
                v
            },
        };
        f.select("DATE");
        f
    }

    /// Choose a field: its options start from sensible defaults.
    pub fn select(&mut self, name: &str) {
        self.name = name.to_string();
        self.number_format.clear();
        self.text_format.clear();
        self.typed_text_formats.clear();
        self.target = match field_kind(name) {
            FieldKind::Bookmark => self.bookmarks.first().cloned().unwrap_or_default(),
            FieldKind::Sequence => "Figure".into(),
            FieldKind::Merge => self.merge_fields.first().cloned().unwrap_or_default(),
            FieldKind::Style => self
                .styles
                .iter()
                .find(|n| n.eq_ignore_ascii_case("Heading 1"))
                .or(self.styles.first())
                .cloned()
                .unwrap_or_else(|| "Heading 1".into()),
            _ => String::new(),
        };
        self.style_switches.clear();
        if field_kind(name) == FieldKind::Date {
            self.date_format = if name == "TIME" { "h:mm am/pm" } else { "M/d/yyyy" }.into();
        }
        self.code = self.instruction();
    }

    /// The field code the options make.
    pub fn instruction(&self) -> String {
        let mut s = self.name.clone();
        let target = self.target.trim();
        match field_kind(&self.name) {
            FieldKind::Date if !self.date_format.trim().is_empty() => s.push_str(&format!(" \\@ \"{}\"", self.date_format.trim().replace('"', ""))),
            FieldKind::Bookmark if !target.is_empty() => s.push_str(&format!(" {} \\h", target.split_whitespace().collect::<Vec<_>>().join("_"))),
            FieldKind::Sequence => s.push_str(&format!(
                " {}",
                if target.is_empty() { "Figure".to_string() } else { target.split_whitespace().collect::<Vec<_>>().join("_") }
            )),
            FieldKind::Merge if !target.is_empty() => {
                if target.contains(char::is_whitespace) {
                    s.push_str(&format!(" \"{}\"", target.replace('"', "")));
                } else {
                    s.push_str(&format!(" {target}"));
                }
            }
            FieldKind::Toc => s.push_str(" \\o \"1-3\" \\h \\z \\u"),
            FieldKind::Style if !target.is_empty() => {
                s.push_str(&format!(" \"{}\"", target.replace(['"', '\\'], "")));
                for (c, _) in STYLEREF_SWITCHES {
                    if self.style_switches.contains(*c) {
                        s.push_str(&format!(" \\{c}"));
                    }
                }
            }
            _ => {}
        }
        if matches!(field_kind(&self.name), FieldKind::Number | FieldKind::Sequence) && !self.number_format.is_empty() {
            s.push_str(&format!(" \\* {}", self.number_format));
        }
        if takes_text_format(&self.name) && !self.text_format.is_empty() {
            if self.typed_text_formats.last().is_some_and(|v| *v == self.text_format) {
                for v in &self.typed_text_formats {
                    s.push_str(&format!(" \\* {v}"));
                }
            } else {
                s.push_str(&format!(" \\* {}", self.text_format));
            }
        }
        if self.preserve && field_kind(&self.name) != FieldKind::Toc {
            s.push_str(" \\* MERGEFORMAT");
        }
        s
    }

    /// The code to insert: typed (Field Codes) or built from the options.
    pub fn final_code(&self) -> String {
        if self.codes { self.code.trim().to_string() } else { self.instruction() }
    }

    /// The Format list: a choice replaces the capitalization switches kept from a typed code.
    pub fn choose_text_format(&mut self, value: &str) {
        self.text_format = value.to_string();
        self.typed_text_formats.clear();
    }

    /// Field Codes / Hide Codes: showing the code starts it from the options; hiding it sets the
    /// Format list to the code's last capitalization switch when the code is for the chosen field,
    /// and keeps all of the code's capitalization switches until another Format is chosen.
    pub fn toggle_codes(&mut self) {
        if !self.codes {
            self.code = self.instruction();
        } else if takes_text_format(&self.name)
            && (wordcraft_layout::fields::field_name(&self.code) == self.name || self.name == "MERGEFIELD" && is_merge_field(&self.code))
        {
            self.typed_text_formats = text_formats(&self.code);
            self.text_format = self.typed_text_formats.last().copied().unwrap_or_default().to_string();
        }
        self.codes = !self.codes;
    }

    /// The `insert.field` parameters (a merge field shows «name» until merged).
    pub fn params(&self) -> Option<Value> {
        let code = self.final_code();
        if code.is_empty() || code.len() > 4096 {
            return None;
        }
        let name = wordcraft_layout::fields::field_name(&code);
        Some(if name == "MERGEFIELD" || is_merge_field(&code) {
            json!({"instr": code, "result": format!("«{}»", merge_field_name(&code))})
        } else {
            json!({"instr": code})
        })
    }
}

/// The byte where a field code's type ends: at a space or a quote, as mail merge reads it.
fn field_type_end(code: &str) -> usize {
    code.find(|c: char| c.is_whitespace() || c == '"').unwrap_or(code.len())
}

/// Whether a typed code is a `MERGEFIELD`, including one with no space before its quoted name.
fn is_merge_field(code: &str) -> bool {
    let code = code.trim_start();
    code[..field_type_end(code)].eq_ignore_ascii_case("MERGEFIELD")
}

/// The merge field a `MERGEFIELD` code names (quoted names may hold spaces).
fn merge_field_name(code: &str) -> String {
    let code = code.trim_start();
    let rest = code[field_type_end(code)..].trim_start();
    match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or("").to_string(),
        None => rest.split_whitespace().next().unwrap_or("").to_string(),
    }
}

impl FieldForm {
    /// The options of the chosen field, by its kind.
    fn options(&mut self, ui: &mut Ui) {
        match field_kind(&self.name) {
            FieldKind::Date => {
                text_field(ui, "field_date", &word("Date formats:"), &mut self.date_format);
                let samples: Vec<String> = DATE_FORMATS.iter().map(|p| wordcraft_engine::cmd::insert::format_date(p)).collect();
                let current = DATE_FORMATS.iter().position(|p| *p == self.date_format);
                if let Some(at) = list_index(ui, &word("Date formats:"), &samples, current).selected
                    && let Some(p) = DATE_FORMATS.get(at)
                {
                    self.date_format = (*p).to_string();
                }
            }
            FieldKind::Number | FieldKind::Sequence => {
                if field_kind(&self.name) == FieldKind::Sequence {
                    text_field(ui, "field_sequence", &word("Sequence name:"), &mut self.target);
                }
                let rows: Vec<Label> = NUMBER_FORMATS.iter().map(|(_, l)| word(l)).collect();
                let current = NUMBER_FORMATS.iter().position(|(v, _)| *v == self.number_format);
                if let Some(at) = select_index(ui, &word("Format:"), &rows, current)
                    && let Some((v, _)) = NUMBER_FORMATS.get(at)
                {
                    self.number_format = (*v).to_string();
                }
            }
            FieldKind::Bookmark => {
                if self.bookmarks.is_empty() {
                    text_field(ui, "field_bookmark", &word("Bookmark name:"), &mut self.target);
                    Text::new(word("The document has no bookmarks yet.")).show(ui);
                } else {
                    let rows: Vec<Label> = self.bookmarks.iter().map(|b| Label::from(b.clone())).collect();
                    let current = self.bookmarks.iter().position(|b| *b == self.target);
                    if let Some(at) = select_index(ui, &word("Bookmark name:"), &rows, current)
                        && let Some(b) = self.bookmarks.get(at)
                    {
                        self.target.clone_from(b);
                    }
                }
            }
            FieldKind::Merge => {
                text_field(ui, "field_merge", &word("Field name:"), &mut self.target);
                if !self.merge_fields.is_empty() {
                    let rows: Vec<Label> = self.merge_fields.iter().map(|m| Label::from(m.clone())).collect();
                    let current = self.merge_fields.iter().position(|m| *m == self.target);
                    if let Some(at) = select_index(ui, &word("Merge fields:"), &rows, current)
                        && let Some(m) = self.merge_fields.get(at)
                    {
                        self.target.clone_from(m);
                    }
                }
            }
            FieldKind::Style => {
                let rows: Vec<Label> = self.styles.iter().map(|n| Label::from(n.clone())).collect();
                let current = self.styles.iter().position(|n| *n == self.target);
                if let Some(at) = select_index(ui, &word("Style name:"), &rows, current)
                    && let Some(n) = self.styles.get(at)
                {
                    self.target.clone_from(n);
                }
                for (c, label) in STYLEREF_SWITCHES {
                    let mut on = self.style_switches.contains(*c);
                    Checkbox::new(Bind::bound(&mut on)).label(word(label)).show(ui);
                    if on != self.style_switches.contains(*c) {
                        self.style_switches.retain(|x| x != *c);
                        if on {
                            self.style_switches.push(*c);
                        }
                    }
                }
            }
            FieldKind::Toc | FieldKind::Plain => {
                if !takes_text_format(&self.name) {
                    Text::new(word("This field has no options.")).show(ui);
                }
            }
        }
        if takes_text_format(&self.name) {
            let rows: Vec<Label> = TEXT_FORMATS.iter().map(|(_, l)| word(l)).collect();
            let current = TEXT_FORMATS.iter().position(|(v, _)| *v == self.text_format);
            if let Some(at) = select_index(ui, &word("Format:"), &rows, current)
                && let Some((v, _)) = TEXT_FORMATS.get(at)
            {
                self.choose_text_format(v);
            }
        }
    }
}

impl WordForm for FieldForm {
    fn key(&self) -> &'static str {
        "field"
    }

    fn title(&self) -> Label {
        word("Field")
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Row::new().show(ui, |ui, _| {
            // Left: category and field names.
            Stack::new().show(ui, |ui, _| {
                Text::new(word("Pick a field from the list")).show(ui);
                let categories: Vec<Label> = FIELD_CATEGORIES.iter().map(|c| word(c)).collect();
                let current = FIELD_CATEGORIES.iter().position(|c| *c == self.category);
                if let Some(at) = select_index(ui, &word("Categories:"), &categories, current)
                    && let Some(c) = FIELD_CATEGORIES.get(at)
                {
                    self.category = (*c).to_string();
                }
                let names: Vec<&'static str> =
                    FIELDS.iter().filter(|(_, cat, _)| self.category == "(All)" || self.category == *cat).map(|(n, _, _)| *n).collect();
                let rows: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
                let current = names.iter().position(|n| *n == self.name);
                if let Some(at) = list_index(ui, &word("Field names:"), &rows, current).selected
                    && let Some(n) = names.get(at)
                {
                    self.select(n);
                }
            });
            // Right: the field's options, or its code.
            Stack::new().show(ui, |ui, _| {
                Text::new(word("Field properties")).show(ui);
                if let Some((_, _, desc)) = FIELDS.iter().find(|(n, _, _)| *n == self.name) {
                    Text::new(Label::from(format!("{} {}", tl!("Description:"), tl!(desc)))).show(ui);
                }
                if self.codes {
                    let label = word("Field codes:");
                    Field::new(label.clone()).id_salt("field_codes").show(ui, |ui, _| {
                        TextField::new(label.clone(), Bind::bound(&mut self.code)).lines(3).mono().show(ui);
                    });
                    Text::new(word("Type the field's name and its switches, for example PAGE \\* roman.")).show(ui);
                } else {
                    self.options(ui);
                    Checkbox::new(Bind::bound(&mut self.preserve)).label(word("Preserve formatting during updates")).show(ui);
                }
            });
        });
        Text::new(Label::from(format!("{} {}", tl!("Field code:"), self.final_code()))).show(ui);
        self.params().is_some()
    }

    fn commands(&self) -> Vec<(String, Value)> {
        self.params().map(|p| vec![("insert.field".to_string(), p)]).unwrap_or_default()
    }

    fn extra_buttons(&self) -> Vec<FooterButton> {
        let label = if self.codes { "Hide Codes" } else { "Field Codes" };
        vec![FooterButton::new(word(label), DialogRole::Other).stays_open(true)]
    }

    fn extra(&mut self, at: usize) -> Vec<(String, Value)> {
        if at == 0 {
            self.toggle_codes();
        }
        Vec::new()
    }
}
