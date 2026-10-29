use super::{Serialize, Stroke, Tokens, Ui, Value, WordApp, buttons, json, semibold};

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
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldForm {
    pub category: String,
    pub name: String,
    /// `\@` picture for date fields.
    pub date_format: String,
    /// `\*` value for number fields (empty = none).
    pub number_format: String,
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
        if self.preserve && field_kind(&self.name) != FieldKind::Toc {
            s.push_str(" \\* MERGEFORMAT");
        }
        s
    }

    /// The code to insert: typed (Field Codes) or built from the options.
    pub fn final_code(&self) -> String {
        if self.codes { self.code.trim().to_string() } else { self.instruction() }
    }

    /// The `insert.field` parameters (a merge field shows «name» until merged).
    pub fn params(&self) -> Option<Value> {
        let code = self.final_code();
        if code.is_empty() || code.len() > 4096 {
            return None;
        }
        let name = wordcraft_layout::fields::field_name(&code);
        Some(if name == "MERGEFIELD" { json!({"instr": code, "result": format!("«{}»", merge_field_name(&code))}) } else { json!({"instr": code}) })
    }
}

/// The merge field a `MERGEFIELD` code names (quoted names may hold spaces).
fn merge_field_name(code: &str) -> String {
    let rest = code.trim_start().split_once(char::is_whitespace).map(|x| x.1.trim_start()).unwrap_or("");
    match rest.strip_prefix('"') {
        Some(q) => q.split('"').next().unwrap_or("").to_string(),
        None => rest.split_whitespace().next().unwrap_or("").to_string(),
    }
}

pub(super) fn field_ui(app: &mut WordApp, ui: &mut Ui, f: &mut FieldForm) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.set_width(620.0);
    ui.horizontal_top(|ui| {
        // Left: category and field names.
        ui.vertical(|ui| {
            ui.set_width(200.0);
            ui.label(egui::RichText::new(tl!("Pick a field from the list")).font(semibold(12.5)));
            ui.label(tl!("Categories:"));
            let shown = tl!(f.category.as_str()).to_string();
            craft_ui::ComboBox::from_id_salt("field_category").selected_text(shown).width(190.0).show_ui(ui, |ui| {
                for c in FIELD_CATEGORIES {
                    ui.selectable_value(&mut f.category, (*c).to_string(), tl!(c));
                }
            });
            ui.label(tl!("Field names:"));
            let mut picked = None;
            egui::Frame::new().stroke(Stroke::new(1.0, t.border)).inner_margin(2.0).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("field_names").max_height(230.0).auto_shrink([false, false]).show(ui, |ui| {
                    for (name, cat, _) in FIELDS {
                        if (f.category == "(All)" || f.category == *cat) && ui.selectable_label(f.name == *name, *name).clicked() {
                            picked = Some(*name);
                        }
                    }
                });
            });
            if let Some(n) = picked {
                f.select(n);
            }
        });
        ui.add_space(14.0);
        // Right: the field's options, or its code.
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(tl!("Field properties")).font(semibold(12.5)));
            if let Some((_, _, desc)) = FIELDS.iter().find(|(n, _, _)| *n == f.name) {
                ui.label(egui::RichText::new(format!("{} {}", tl!("Description:"), tl!(desc))).small().color(t.text_dim));
            }
            ui.add_space(4.0);
            if f.codes {
                ui.label(tl!("Field codes:"));
                ui.add(egui::TextEdit::multiline(&mut f.code).desired_rows(3).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                ui.label(egui::RichText::new(tl!("Type the field's name and its switches, for example PAGE \\* roman.")).small().color(t.text_dim));
            } else {
                field_options(ui, f, &t);
                ui.add_space(4.0);
                ui.checkbox(&mut f.preserve, tl!("Preserve formatting during updates"));
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Field code:")).font(semibold(12.0)));
        ui.label(egui::RichText::new(f.final_code()).monospace());
    });
    ui.horizontal(|ui| {
        let label = if f.codes { "Hide Codes" } else { "Field Codes" };
        if ui.button(tl!(label)).clicked() {
            if !f.codes {
                f.code = f.instruction();
            }
            f.codes = !f.codes;
        }
    });
    let params = f.params();
    let (ok, cancel) = buttons(ui, "OK", "Cancel", params.is_some());
    if ok && let Some(p) = params {
        let _ = app.run("insert.field", p);
    }
    ok || cancel
}

fn field_options(ui: &mut Ui, f: &mut FieldForm, t: &Tokens) {
    match field_kind(&f.name) {
        FieldKind::Date => {
            ui.label(tl!("Date formats:"));
            ui.add(egui::TextEdit::singleline(&mut f.date_format).desired_width(f32::INFINITY));
            egui::Frame::new().stroke(Stroke::new(1.0, t.border)).inner_margin(2.0).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("field_dates").max_height(170.0).auto_shrink([false, false]).show(ui, |ui| {
                    for pic in DATE_FORMATS {
                        let sample = wordcraft_engine::cmd::insert::format_date(pic);
                        if ui.selectable_label(f.date_format == *pic, sample).on_hover_text(*pic).clicked() {
                            f.date_format = (*pic).to_string();
                        }
                    }
                });
            });
        }
        FieldKind::Number | FieldKind::Sequence => {
            if field_kind(&f.name) == FieldKind::Sequence {
                ui.label(tl!("Sequence name:"));
                ui.add(egui::TextEdit::singleline(&mut f.target).desired_width(200.0));
            }
            ui.label(tl!("Format:"));
            let shown = NUMBER_FORMATS.iter().find(|(v, _)| *v == f.number_format).map(|(_, l)| *l).unwrap_or("(none)");
            craft_ui::ComboBox::from_id_salt("field_number").selected_text(if shown == "(none)" { tl!(shown) } else { shown }).width(200.0).show_ui(
                ui,
                |ui| {
                    for (v, l) in NUMBER_FORMATS {
                        ui.selectable_value(&mut f.number_format, (*v).to_string(), if *l == "(none)" { tl!(l) } else { l });
                    }
                },
            );
        }
        FieldKind::Bookmark => {
            ui.label(tl!("Bookmark name:"));
            if f.bookmarks.is_empty() {
                ui.add(egui::TextEdit::singleline(&mut f.target).desired_width(200.0));
                ui.label(egui::RichText::new(tl!("The document has no bookmarks yet.")).small().color(t.text_dim));
            } else {
                craft_ui::ComboBox::from_id_salt("field_bookmark").selected_text(f.target.clone()).width(200.0).show_ui(ui, |ui| {
                    for b in &f.bookmarks {
                        ui.selectable_value(&mut f.target, b.clone(), b);
                    }
                });
            }
        }
        FieldKind::Merge => {
            ui.label(tl!("Field name:"));
            ui.add(egui::TextEdit::singleline(&mut f.target).desired_width(200.0));
            if !f.merge_fields.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for m in f.merge_fields.clone() {
                        if ui.selectable_label(f.target == m, &m).clicked() {
                            f.target = m;
                        }
                    }
                });
            }
        }
        FieldKind::Style => {
            ui.label(tl!("Style name:"));
            craft_ui::ComboBox::from_id_salt("field_style").selected_text(f.target.clone()).width(200.0).show_ui(ui, |ui| {
                for n in &f.styles {
                    ui.selectable_value(&mut f.target, n.clone(), n);
                }
            });
            ui.add_space(4.0);
            for (c, label) in STYLEREF_SWITCHES {
                let mut on = f.style_switches.contains(*c);
                if ui.checkbox(&mut on, tl!(label)).changed() {
                    f.style_switches.retain(|x| x != *c);
                    if on {
                        f.style_switches.push(*c);
                    }
                }
            }
        }
        FieldKind::Toc | FieldKind::Plain => {
            ui.label(egui::RichText::new(tl!("This field has no options.")).small().color(t.text_dim));
        }
    }
}
