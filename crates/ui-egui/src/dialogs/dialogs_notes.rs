//! Footnote and Endnote (References › Footnotes launcher, #385): where notes go, how they are
//! numbered, for the whole document or the caret's section; Convert; Insert with a custom mark.
//! The form ends in commands (`references.noteOptions`, `references.convertNotes`,
//! `references.footnote` / `references.endnote`), so agents get the same result without it.

use craft::ui::buttons::Button;
use craft::ui::choice::{Radio, RadioGroup};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::dialog::DialogId;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::patterns::form_dialogs::FormCx;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton, GroupBox};
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::WordApp;
use crate::dialogs::host::{Asks, WordForm, open_form};

/// Number formats notes offer: OOXML name and a sample (not translated). The Format Page Numbers
/// dialog offers the same list (#1113).
pub(crate) const FORMATS: [(&str, &str); 5] = [
    ("decimal", "1, 2, 3, …"),
    ("lowerLetter", "a, b, c, …"),
    ("upperLetter", "A, B, C, …"),
    ("lowerRoman", "i, ii, iii, …"),
    ("upperRoman", "I, II, III, …"),
];

/// Longest custom mark, in characters.
const MARK_LIMIT: usize = 10;

/// Largest "Start at" value.
const MAX_START: f64 = 32_767.0;

/// The dialog's fields; `[footnotes, endnotes]` where each kind has its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteForm {
    /// Which kind the format fields and Insert are for.
    pub endnote: bool,
    pub pos: [String; 2],
    pub fmt: [String; 2],
    pub start: [u32; 2],
    pub restart: [String; 2],
    /// A custom mark for Insert (empty: numbered).
    pub mark: String,
    /// Apply to the whole document (else the caret's section).
    pub whole: bool,
}

impl Default for NoteForm {
    /// Word's defaults: footnotes at the page bottom in 1, 2, 3; endnotes at the document end in i, ii, iii.
    fn default() -> Self {
        NoteForm {
            endnote: false,
            pos: ["pageBottom".into(), "docEnd".into()],
            fmt: ["decimal".into(), "lowerRoman".into()],
            start: [1, 1],
            restart: ["continuous".into(), "continuous".into()],
            mark: String::new(),
            whole: true,
        }
    }
}

impl NoteForm {
    /// The options in effect at the caret.
    pub fn read(app: &mut WordApp) -> NoteForm {
        let v = app.session.run("references.noteOptions", &json!({})).unwrap_or_default();
        let s = |kind: &str, k: &str, d: &str| v.get(kind).and_then(|o| o.get(k)).and_then(Value::as_str).unwrap_or(d).to_string();
        let n = |kind: &str| v.get(kind).and_then(|o| o.get("numStart")).and_then(Value::as_u64).unwrap_or(1).clamp(1, 32_767) as u32;
        NoteForm {
            endnote: false,
            pos: [s("footnote", "pos", "pageBottom"), s("endnote", "pos", "docEnd")],
            fmt: [s("footnote", "numFmt", "decimal"), s("endnote", "numFmt", "lowerRoman")],
            start: [n("footnote"), n("endnote")],
            restart: [s("footnote", "numRestart", "continuous"), s("endnote", "numRestart", "continuous")],
            mark: String::new(),
            whole: true,
        }
    }

    /// `references.noteOptions` parameters for the chosen kind.
    pub fn params(&self) -> Value {
        let k = usize::from(self.endnote);
        let get = |a: &[String; 2]| a.get(k).cloned().unwrap_or_default();
        json!({
            "kind": if self.endnote { "endnote" } else { "footnote" },
            "pos": get(&self.pos),
            "numFmt": get(&self.fmt),
            "numStart": self.start.get(k).copied().unwrap_or(1),
            "numRestart": get(&self.restart),
            "scope": if self.whole { "document" } else { "section" },
        })
    }
}

/// Opens Footnote and Endnote for the options in effect at the caret.
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    if name != "noteOptions" {
        return None;
    }
    let mut form = NoteForm::read(app);
    form.endnote = params.get("kind").and_then(Value::as_str) == Some("endnote");
    Some(open_form(app, parent, form))
}

impl WordForm for NoteForm {
    fn key(&self) -> &'static str {
        "noteOptions"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Footnote and Endnote").to_string())
    }

    fn ok_label(&self) -> Option<Label> {
        Some(Label::from(if self.endnote { tl!("Insert Endnote").to_string() } else { tl!("Insert Footnote").to_string() }))
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        let endnote = self.endnote;
        let k = usize::from(endnote);
        Stack::new().show(ui, |ui, _| {
            GroupBox::new(Label::from(tl!("Location").to_string()), None).show(ui, |ui, _| {
                RadioGroup::new(Label::from(tl!("Location").to_string()), Bind::bound(&mut self.endnote)).show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, _| {
                        Row::new().show(ui, |ui, _| {
                            Radio::new(false).label(Label::from(tl!("Footnotes").to_string())).show(ui);
                            if let Some(pos) = self.pos.get_mut(0) {
                                let options = [("pageBottom", "Bottom of Page"), ("beneathText", "Below text")];
                                let entries: Vec<Entry<String>> =
                                    options.iter().map(|(v, l)| Entry::new(v.to_string(), tl!(l).to_string())).collect();
                                Select::new(Label::from(tl!("Footnotes").to_string()), Bind::bound(pos), &entries).enabled(!endnote).show(ui);
                            }
                        });
                        Row::new().show(ui, |ui, _| {
                            Radio::new(true).label(Label::from(tl!("Endnotes").to_string())).show(ui);
                            if let Some(pos) = self.pos.get_mut(1) {
                                let options = [("sectEnd", "End of section"), ("docEnd", "End of document")];
                                let entries: Vec<Entry<String>> =
                                    options.iter().map(|(v, l)| Entry::new(v.to_string(), tl!(l).to_string())).collect();
                                Select::new(Label::from(tl!("Endnotes").to_string()), Bind::bound(pos), &entries).enabled(endnote).show(ui);
                            }
                        });
                    });
                });
                let opener = Button::new(Label::from(format!("{}…", tl!("Convert")))).show(ui);
                Menu::new(Label::from(tl!("Convert").to_string())).opened_by(&opener, Trigger::Click).show(ui, |ui, _| {
                    for (to, l) in
                        [("endnote", "Footnotes to endnotes"), ("footnote", "Endnotes to footnotes"), ("swap", "Swap footnotes and endnotes")]
                    {
                        if MenuItem::new(Label::from(tl!(l).to_string())).show(ui).ran {
                            asks.push(Asks::Run("references.convertNotes".to_string(), json!({ "to": to })));
                        }
                    }
                });
            });
            GroupBox::new(Label::from(tl!("Format").to_string()), None).show(ui, |ui, _| {
                Field::new(Label::from(tl!("Number format").to_string())).show(ui, |ui, _| {
                    if let Some(v) = self.fmt.get_mut(k) {
                        let entries: Vec<Entry<String>> = FORMATS.iter().map(|(v, l)| Entry::new(v.to_string(), l.to_string())).collect();
                        Select::new(Label::from(tl!("Number format").to_string()), Bind::bound(v), &entries).show(ui);
                    }
                });
                Field::new(Label::from(tl!("Custom mark:").to_string())).show(ui, |ui, _| {
                    TextField::new(Label::from(tl!("Custom mark:").to_string()), Bind::bound(&mut self.mark)).limit(Some(MARK_LIMIT)).show(ui);
                });
                Field::new(Label::from(tl!("Start at:").to_string())).show(ui, |ui, _| {
                    if let Some(v) = self.start.get_mut(k) {
                        let out = NumberField::new(Label::from(tl!("Start at:").to_string()), Bind::given(Some(f64::from(*v))))
                            .range(1.0..=MAX_START)
                            .show(ui);
                        if let Some(Some(n)) = out.value().copied() {
                            *v = n.round().clamp(1.0, MAX_START) as u32;
                        }
                    }
                });
                Field::new(Label::from(tl!("Numbering").to_string())).show(ui, |ui, _| {
                    if let Some(v) = self.restart.get_mut(k) {
                        let each_page: &[(&str, &str)] = if endnote { &[] } else { &[("eachPage", "Restart Each Page")] };
                        let entries: Vec<Entry<String>> = [("continuous", "Continuous numbering"), ("eachSect", "Restart Each Section")]
                            .into_iter()
                            .chain(each_page.iter().copied())
                            .map(|(v, l)| Entry::new(v.to_string(), tl!(l).to_string()))
                            .collect();
                        Select::new(Label::from(tl!("Numbering").to_string()), Bind::bound(v), &entries).show(ui);
                    }
                });
                Field::new(Label::from(tl!("Apply to:").to_string())).show(ui, |ui, _| {
                    let scopes = [
                        Entry::new(true, Label::from(tl!("Whole document").to_string())),
                        Entry::new(false, Label::from(tl!("This section").to_string())),
                    ];
                    Select::new(Label::from(tl!("Apply to:").to_string()), Bind::bound(&mut self.whole), &scopes).show(ui);
                });
            });
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let insert = if self.endnote { "references.endnote" } else { "references.footnote" };
        vec![("references.noteOptions".to_string(), self.params()), (insert.to_string(), json!({ "mark": self.mark.trim() }))]
    }

    /// Apply keeps the dialog open and sets the options only.
    fn extra_buttons(&self) -> Vec<FooterButton> {
        vec![FooterButton::new(Label::from(tl!("Apply").to_string()), DialogRole::Other).stays_open(true)]
    }

    fn extra(&mut self, _at: usize) -> Vec<(String, Value)> {
        vec![("references.noteOptions".to_string(), self.params())]
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::NoteForm;
    use crate::dialogs::host::FormHost;
    use crate::{Services, WordApp};

    /// #385: the Footnotes group's launcher opens Footnote and Endnote; its settings apply to the
    /// caret's section. Scripts get the options, never the dialog.
    #[test]
    fn footnote_and_endnote_opens_from_the_launcher_and_applies() {
        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        assert_eq!(a.run("references.noteOptions", json!({})).unwrap(), json!({"pending": "noteOptions"}));
        let host = a.dialog_host.get_mut::<FormHost<NoteForm>>().expect("dialog");
        let form = host.form_mut();
        assert_eq!((form.pos[0].as_str(), form.fmt[1].as_str(), form.start[0]), ("pageBottom", "lowerRoman", 1));
        form.pos[0] = "beneathText".into();
        form.restart[0] = "eachPage".into();
        form.whole = false;
        let params = form.params();
        crate::dialogs::host::close_all(&mut a, &egui::Context::default());
        a.run("references.noteOptions", params).unwrap();
        let o = a.session.doc.note_options(&a.session.doc.last_section, false);
        assert_eq!(o.pos, wordcraft_doc::section::NotePos::BeneathText);
        assert!(a.session.doc.settings.footnote_pr.is_empty(), "this section only");
        let got = a.execute("references.noteOptions", json!({})).unwrap();
        assert_eq!(got["footnote"]["numRestart"], "eachPage");
    }
}
