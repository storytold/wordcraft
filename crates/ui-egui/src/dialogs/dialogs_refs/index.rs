use craft::behavior::Mixed;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::notices::{Notice, NoticeLayout};
// PLAN: assumed FormButtons (dialogs-w5)
use craft::ui::patterns::form_dialogs::{FormButtons, FormCx};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton, Section};
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_engine::cmd::references::FieldCode;

use super::sample::{LEADERS, leader_char, sample_card};
use crate::WordApp;
use crate::dialogs::host::{Asks, Filed, WordForm, open_form};

/// The tab leaders as drop-down entries (value, translated label).
pub(super) fn leaders() -> Vec<Entry<String>> {
    LEADERS.iter().map(|(v, l)| Entry::new(v.to_string(), Label::from(tl!(l).to_string()))).collect()
}

// ---------------------------------------------------------------------------------------------
// Index and Mark Index Entry

/// The Index dialog's fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IndexForm {
    /// `indented` or `runIn`.
    pub kind: String,
    pub columns: u32,
    pub right_align: bool,
    pub leader: String,
    pub headings: bool,
}

/// The first field in the body whose name is `name`.
pub(super) fn body_field(app: &WordApp, name: &str) -> Option<FieldCode> {
    app.session.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).find_map(|o| match o {
        wordcraft_doc::InlineObject::Field { instr, .. } => {
            Some(FieldCode::parse(instr)).filter(|f| f.name == name && (name != "TOC" || !f.is_table_of_figures()))
        }
        _ => None,
    })
}

impl IndexForm {
    /// The index in the document, or Word's defaults for a new one (two columns).
    pub fn read(app: &WordApp) -> IndexForm {
        match body_field(app, "INDEX") {
            Some(f) => IndexForm {
                kind: if f.has('r') { "runIn" } else { "indented" }.into(),
                columns: f.arg('c').and_then(|c| c.trim().parse().ok()).unwrap_or(1).clamp(1, 4),
                right_align: f.arg('e').is_some_and(|e| e.contains('\t')),
                leader: "dot".into(),
                headings: f.has('h'),
            },
            None => IndexForm { kind: "indented".into(), columns: 2, right_align: false, leader: "dot".into(), headings: true },
        }
    }

    /// `references.index` parameters.
    pub fn params(&self) -> Value {
        json!({"type": self.kind, "columns": self.columns, "rightAlign": self.right_align, "tabLeader": self.leader, "headings": self.headings})
    }

    /// The sample card's lines: (indent level, text, page or empty).
    fn sample_lines(&self) -> Vec<(usize, String, String)> {
        let mut v = Vec::new();
        if self.headings {
            v.push((0, "A".to_string(), String::new()));
        }
        if self.kind == "runIn" {
            v.push((0, format!("{}: {}, 4; {}", tl!("Atmosphere"), tl!("Ionosphere"), tl!("Mesosphere")), "3".to_string()));
        } else {
            v.push((0, tl!("Atmosphere").to_string(), "1".to_string()));
            v.push((1, tl!("Ionosphere").to_string(), "3".to_string()));
            v.push((1, tl!("Mesosphere").to_string(), "4".to_string()));
        }
        v
    }
}

impl WordForm for IndexForm {
    fn key(&self) -> &'static str {
        "index"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Index").to_string())
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("references.index".to_string(), self.params())]
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        sample_card(tl!("Preview"), self.sample_lines(), self.right_align, leader_char(&self.leader)).show(ui);
        let kind = RadioGroup::new(Label::from(tl!("Type:").to_string()), Bind::given(self.kind.clone())).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Radio::new("indented".to_string()).label(Label::from(tl!("Indented").to_string())).show(ui);
                Radio::new("runIn".to_string()).label(Label::from(tl!("Run-in").to_string())).show(ui);
            });
        });
        if let Some(kind) = kind.output.value().cloned() {
            self.kind = kind;
        }
        let columns =
            NumberField::new(Label::from(tl!("Columns:").to_string()), Bind::given(Some(f64::from(self.columns)))).range(1.0..=4.0).show(ui);
        if let Some(Some(n)) = columns.edit.value().cloned() {
            self.columns = n.round().clamp(1.0, 4.0) as u32;
        }
        Checkbox::new(Bind::bound(&mut self.right_align)).label(Label::from(tl!("Page numbers at the right margin").to_string())).show(ui);
        let title = Label::from(tl!("Tab leader:").to_string());
        let entries = leaders();
        let leader = Field::new(title.clone())
            .show(ui, |ui, _| {
                Select::new(title.clone(), Bind::given(Mixed::Same(self.leader.clone())), &entries)
                    .enabled(self.right_align)
                    .show(ui)
                    .value()
                    .cloned()
            })
            .inner
            .flatten();
        if let Some(leader) = leader {
            self.leader = leader;
        }
        Checkbox::new(Bind::bound(&mut self.headings)).label(Label::from(tl!("Letter headings").to_string())).show(ui);
        if Button::new(Label::from(tl!("Mark Entry…").to_string())).show(ui).clicked() {
            asks.push(Asks::Open("markEntry".to_string(), Value::Null));
        }
        true
    }
}

/// The Mark Index Entry dialog's fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MarkForm {
    pub entry: String,
    pub subentry: String,
    /// `page` (current page), `see` (cross-reference) or `range` (page range bookmark).
    pub option: String,
    pub see: String,
    pub bookmark: String,
    pub bookmarks: Vec<String>,
    pub bold: bool,
    pub italic: bool,
    pub message: String,
    /// `message` reports a refusal, not a success.
    pub failed: bool,
}

impl MarkForm {
    pub fn read(app: &WordApp) -> MarkForm {
        let bookmarks: Vec<String> = app.session.doc.bookmarks().into_iter().map(|(n, _)| n).filter(|n| !n.starts_with('_')).collect();
        MarkForm {
            entry: app.session.selected_text().trim().to_string(),
            subentry: String::new(),
            option: "page".into(),
            see: format!("{} ", tl!("See")),
            bookmark: bookmarks.first().cloned().unwrap_or_default(),
            bookmarks,
            bold: false,
            italic: false,
            message: String::new(),
            failed: false,
        }
    }

    /// `references.markEntry` parameters (`all` for Mark All).
    pub fn params(&self, all: bool) -> Value {
        let mut v = json!({"entry": self.entry.trim(), "subentry": self.subentry.trim(), "bold": self.bold, "italic": self.italic, "all": all});
        match self.option.as_str() {
            "see" => v["crossReference"] = json!(self.see.trim()),
            "range" => v["bookmark"] = json!(self.bookmark),
            _ => {}
        }
        if all {
            v["text"] = json!(self.entry.trim());
        }
        v
    }
}

/// The extra footer button that marks every occurrence of the entry.
const MARK_ALL: usize = 0;

impl WordForm for MarkForm {
    fn key(&self) -> &'static str {
        "markEntry"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Mark Index Entry").to_string())
    }

    /// Mark and Mark All are the dialog's actions; there is no OK, only Close.
    fn buttons(&self) -> FormButtons {
        FormButtons::Close
    }

    /// The document stays usable beside the dialog, so the next entry can be selected and marked.
    fn modeless(&self) -> bool {
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        Vec::new()
    }

    /// Both buttons stay open so the next entry can be marked.
    fn extra_buttons(&self) -> Vec<FooterButton> {
        let can = !self.entry.trim().is_empty();
        vec![
            FooterButton::new(Label::from(tl!("Mark All").to_string()), DialogRole::Other).enabled(can).stays_open(true),
            FooterButton::new(Label::from(tl!("Mark").to_string()), DialogRole::Accept).enabled(can).stays_open(true),
        ]
    }

    fn extra(&mut self, at: usize) -> Vec<(String, Value)> {
        vec![("references.markEntry".to_string(), self.params(at == MARK_ALL))]
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Section::fixed(Label::from(tl!("Index").to_string())).id_salt("index").show(ui, |ui, _| {
            let title = Label::from(tl!("Main entry:").to_string());
            let typed = Field::new(title.clone())
                .show(ui, |ui, _| TextField::new(title.clone(), Bind::given(self.entry.clone())).show(ui).value().cloned())
                .inner
                .flatten();
            if let Some(v) = typed {
                self.entry = v;
            }
            let title = Label::from(tl!("Subentry:").to_string());
            let typed = Field::new(title.clone())
                .show(ui, |ui, _| TextField::new(title.clone(), Bind::given(self.subentry.clone())).show(ui).value().cloned())
                .inner
                .flatten();
            if let Some(v) = typed {
                self.subentry = v;
            }
        });
        Section::fixed(Label::from(tl!("Options").to_string())).id_salt("options").show(ui, |ui, _| {
            let options =
                [("see", "Cross-reference:", true), ("page", "This page", true), ("range", "Pages of bookmark:", !self.bookmarks.is_empty())];
            let group = RadioGroup::new(Label::from(tl!("Options").to_string()), Bind::given(self.option.clone())).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    for (id, label, enabled) in options {
                        Radio::new(id.to_string()).label(Label::from(tl!(label).to_string())).enabled(enabled).show(ui);
                    }
                });
            });
            if let Some(option) = group.output.value().cloned() {
                self.option = option;
            }
            let title = Label::from(tl!("Cross-reference text:").to_string());
            let typed = Field::new(title.clone())
                .show(ui, |ui, _| {
                    TextField::new(title.clone(), Bind::given(self.see.clone())).enabled(self.option == "see").show(ui).value().cloned()
                })
                .inner
                .flatten();
            if let Some(v) = typed {
                self.see = v;
            }
            let entries: Vec<Entry<String>> = self.bookmarks.iter().map(|b| Entry::new(b.clone(), Label::from(b.clone()))).collect();
            let title = Label::from(tl!("Bookmark:").to_string());
            let picked = Field::new(title.clone())
                .show(ui, |ui, _| {
                    Select::new(title.clone(), Bind::given(Mixed::Same(self.bookmark.clone())), &entries)
                        .enabled(self.option == "range")
                        .show(ui)
                        .value()
                        .cloned()
                })
                .inner
                .flatten();
            if let Some(v) = picked {
                self.bookmark = v;
            }
        });
        Section::fixed(Label::from(tl!("Page number style").to_string())).id_salt("style").show(ui, |ui, _| {
            Checkbox::new(Bind::bound(&mut self.bold)).label(Label::from(tl!("Bold").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut self.italic)).label(Label::from(tl!("Italic").to_string())).show(ui);
        });
        if !self.message.is_empty() {
            let tone = if self.failed { Tone::Bad } else { Tone::Neutral };
            Notice::new(tone, Label::from(self.message.clone())).layout(NoticeLayout::Inline).show(ui);
        }
        true
    }

    /// The status line shows the engine's count of marked entries or its refusal; the dialog stays open.
    fn applied(&mut self, _app: &mut WordApp, filed: Filed, outcome: Result<&[Value], &str>) {
        let Filed::Extra(at) = filed else { return };
        match outcome {
            Ok(results) => {
                self.message = if at == MARK_ALL {
                    let marked = results.first().and_then(|v| v.get("marked")).and_then(Value::as_u64).unwrap_or(0);
                    crate::i18n::fmt(tl!("{n} entries marked."), &[("n", &marked.to_string())])
                } else {
                    tl!("Entry marked.").to_string()
                };
                self.failed = false;
            }
            Err(e) => {
                self.message = e.to_string();
                self.failed = true;
            }
        }
    }
}

/// Opens the Index dialog or the Mark Index Entry dialog (`name`), over `parent` when it is a child.
pub(super) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "index" => {
            let form = IndexForm::read(app);
            Some(open_form(app, parent, form))
        }
        "markEntry" => {
            let form = MarkForm::read(app);
            Some(open_form(app, parent, form))
        }
        _ => None,
    }
}
