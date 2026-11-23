//! Review dialogs on the form host (#412): Compare / Combine Documents (with the More >>
//! comparison settings) and AutoCorrect Options (AutoCorrect, AutoFormat As You Type and Math
//! AutoCorrect). Each ends by running a command (`review.compare`, `review.combine`,
//! `tools.autocorrect`), so scripts and agents get the same result without them. The Restrict
//! Editing pane lives in `restrict_pane`.

use craft::behavior::{ColumnKey, ColumnWidth, SelectionChange, keyed};
use craft::host::PickRequest;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ListRow;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{FormCx, PathRow};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::GroupBox;
use craft::ui::table::Table;
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::text::TextField;
use egui::Ui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_engine::cmd::tools::AutoCorrectPrefs;

use crate::WordApp;
use crate::dialogs::host::{Asks, DialogHooks, Filed, FormHost, Settle, WordForm, open_form, open_form_with};

/// The dialog `ui.dialog` opens by `name`: `compare`, `combine` or `autoCorrect` (its `tab`
/// parameter picks the page). Other names open nothing here.
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "compare" | "combine" => {
            let form = CompareForm::new(app, name == "combine");
            Some(open_form(app, parent, form))
        }
        "autoCorrect" => {
            let mut form = AutoCorrectForm::new(app);
            if let Some(tab) = params.get("tab").and_then(Value::as_u64) {
                form.tab = u8::try_from(tab).unwrap_or(0).min(2);
            }
            Some(open_form_with(app, parent, form, DialogHooks { prepare: None, settle: settle_lists }))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Compare / Combine

/// The comparison settings (More >>), in the order the dialog shows them, with their
/// `review.compare` parameter names.
pub const COMPARE_SETTINGS: [(&str, &str); 10] = [
    ("moves", "Moves"),
    ("comments", "Comments"),
    ("formatting", "Formatting"),
    ("caseChanges", "Case changes"),
    ("whiteSpace", "White space"),
    ("tables", "Tables"),
    ("headersFooters", "Headers and footers"),
    ("footnotes", "Footnotes and endnotes"),
    ("textBoxes", "Text boxes"),
    ("fields", "Fields"),
];

/// The Compare / Combine Documents dialog's values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareForm {
    pub combine: bool,
    /// The original is the open document (else the file `original`).
    pub original_current: bool,
    pub original: String,
    pub revised: String,
    /// Label changes with.
    pub author: String,
    /// More >> shown.
    pub more: bool,
    /// [`COMPARE_SETTINGS`], all on by default as in Word.
    pub settings: [bool; 10],
    pub character_level: bool,
    /// Show changes in: `original`, `revised` or `new`.
    pub show_in: String,
}

impl Default for CompareForm {
    fn default() -> Self {
        CompareForm {
            combine: false,
            original_current: true,
            original: String::new(),
            revised: String::new(),
            author: String::new(),
            more: false,
            settings: [true; 10],
            character_level: false,
            show_in: "new".into(),
        }
    }
}

impl CompareForm {
    pub fn new(app: &WordApp, combine: bool) -> CompareForm {
        CompareForm { combine, author: app.session.author.clone(), ..CompareForm::default() }
    }

    /// The `review.compare` / `review.combine` parameters; `None` until a revised document is
    /// chosen.
    pub fn params(&self) -> Option<Value> {
        let revised = self.revised.trim();
        if revised.is_empty() || (!self.original_current && self.original.trim().is_empty()) {
            return None;
        }
        let mut v = json!({
            "path": revised,
            "author": self.author.trim(),
            "level": if self.character_level { "character" } else { "word" },
            "showIn": self.show_in,
        });
        if !self.original_current {
            v["original"] = json!(self.original.trim());
        }
        for ((key, _), on) in COMPARE_SETTINGS.iter().zip(self.settings) {
            v[*key] = json!(on);
        }
        Some(v)
    }
}

impl WordForm for CompareForm {
    fn key(&self) -> &'static str {
        if self.combine { "combine" } else { "compare" }
    }

    fn title(&self) -> Label {
        Label::from(if self.combine { tl!("Combine Documents").to_string() } else { tl!("Compare Documents").to_string() })
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let can = Env::of(ui).view().host().is_some_and(|h| h.files().can_pick());
        let reason = (!can).then(|| Label::from(tl!("Comparing with another file needs the desktop app.").to_string()));
        let another = !self.original_current;
        let (mut original_pick, mut revised_pick): (Option<String>, Option<String>) = (None, None);
        Stack::new().show(ui, |ui, _| {
            if let Some(reason) = &reason {
                Notice::new(Tone::Neutral, reason.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            Row::new().show(ui, |ui, _| {
                Field::new(Label::from(tl!("Original document").to_string())).show(ui, |ui, _| {
                    RadioGroup::new(Label::from(tl!("Original document").to_string()), Bind::bound(&mut self.original_current)).show(ui, |ui, _| {
                        Stack::new().show(ui, |ui, _| {
                            Radio::new(true).label(Label::from(tl!("This document").to_string())).show(ui);
                            Radio::new(false).label(Label::from(tl!("Another file").to_string())).show(ui);
                        });
                    });
                });
                // The field greys the row while "This document" is chosen.
                Field::new(Label::from(tl!("Another file").to_string())).enabled(another).show(ui, |ui, _| {
                    let mut row = PathRow::new(Label::from(tl!("Another file").to_string()), Bind::bound(&mut self.original))
                        .placeholder(Label::from(tl!("Another file").to_string()))
                        .pick(PickRequest::open("document"));
                    if let Some(r) = &reason {
                        row = row.browse_reason(r.clone());
                    }
                    original_pick = row.show(ui).picked;
                });
                Field::new(Label::from(tl!("Revised document").to_string())).show(ui, |ui, _| {
                    let mut row = PathRow::new(Label::from(tl!("Revised document").to_string()), Bind::bound(&mut self.revised))
                        .pick(PickRequest::open("document"));
                    if let Some(r) = &reason {
                        row = row.browse_reason(r.clone());
                    }
                    revised_pick = row.show(ui).picked;
                });
            });
            Field::new(Label::from(tl!("Label changes with:").to_string())).show(ui, |ui, _| {
                TextField::new(Label::from(tl!("Label changes with:").to_string()), Bind::bound(&mut self.author)).show(ui);
            });
            let more = (if self.more { tl!("<< Less") } else { tl!("More >>") }).to_string();
            if clicked(&Button::new(Label::from(more)).show(ui)) {
                self.more = !self.more;
            }
            if self.more {
                GroupBox::new(Label::from(tl!("Comparison settings").to_string()), None).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        for column in 0..2 {
                            Stack::new().show(ui, |ui, _| {
                                for (at, (_, name)) in COMPARE_SETTINGS.iter().enumerate().filter(|(at, _)| at % 2 == column) {
                                    if let Some(on) = self.settings.get_mut(at) {
                                        Checkbox::new(Bind::bound(on)).label(Label::from(tl!(name).to_string())).show(ui);
                                    }
                                }
                            });
                        }
                    });
                    Text::new(Label::from(
                        tl!("Formatting, case changes, white space and tables are compared; the other settings are kept for later.").to_string(),
                    ))
                    .show(ui);
                });
                Row::new().show(ui, |ui, _| {
                    Field::new(Label::from(tl!("Show changes at:").to_string())).show(ui, |ui, _| {
                        RadioGroup::new(Label::from(tl!("Show changes at:").to_string()), Bind::bound(&mut self.character_level)).show(
                            ui,
                            |ui, _| {
                                Stack::new().show(ui, |ui, _| {
                                    Radio::new(false).label(Label::from(tl!("Word level").to_string())).show(ui);
                                    Radio::new(true).label(Label::from(tl!("Character level").to_string())).show(ui);
                                });
                            },
                        );
                    });
                    Field::new(Label::from(tl!("Show changes in:").to_string())).show(ui, |ui, _| {
                        RadioGroup::new(Label::from(tl!("Show changes in:").to_string()), Bind::bound(&mut self.show_in)).show(ui, |ui, _| {
                            Stack::new().show(ui, |ui, _| {
                                for (v, l) in [("original", "Original document"), ("revised", "Revised document"), ("new", "New document")] {
                                    Radio::new(v.to_string()).label(Label::from(tl!(l).to_string())).show(ui);
                                }
                            });
                        });
                    });
                });
            }
        });
        // A picked file replaces what the row shows, as a set.
        if let Some(path) = original_pick {
            self.original = path;
            self.original_current = false;
        }
        if let Some(path) = revised_pick {
            self.revised = path;
        }
        self.params().is_some()
    }

    fn commands(&self) -> Vec<(String, Value)> {
        let id = if self.combine { "review.combine" } else { "review.compare" };
        self.params().map(|p| (id.to_string(), p)).into_iter().collect()
    }

    fn applied(&mut self, app: &mut WordApp, filed: Filed, outcome: Result<&[Value], &str>) {
        if matches!(filed, Filed::Ok)
            && let Ok(results) = outcome
            && let Some(changes) = results.first().and_then(|r| r.get("changes")).and_then(Value::as_u64)
        {
            app.status(format!("Compared: {changes} changes"));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// AutoCorrect Options

/// A change to one of the AutoCorrect lists, run at once (as in Word) by [`settle_lists`].
#[derive(Clone, Debug, PartialEq)]
pub enum ListEdit {
    /// An entry of the Replace / With list; success clears both boxes.
    Entry(Value),
    /// An exception; success clears its box.
    Exception(Value),
}

/// The AutoCorrect Options dialog's values.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AutoCorrectForm {
    /// 0 AutoCorrect, 1 AutoFormat As You Type, 2 Math AutoCorrect.
    pub tab: u8,
    /// The options as edited (applied on OK); the lists change at once, as in Word.
    pub options: AutoCorrectPrefs,
    /// Replace / With boxes.
    pub from: String,
    pub to: String,
    /// Exceptions shown, and the exception being typed.
    pub show_exceptions: bool,
    pub exception: String,
    /// Math AutoCorrect filter.
    pub math_filter: String,
    pub message: String,
    /// List changes filed by the body, run by the dialog's settle step.
    #[serde(skip)]
    pub pending: Vec<ListEdit>,
}

impl AutoCorrectForm {
    pub fn new(app: &WordApp) -> AutoCorrectForm {
        AutoCorrectForm { options: app.session.prefs.autocorrect.clone(), ..AutoCorrectForm::default() }
    }

    /// The `tools.autocorrect` switches.
    pub fn params(&self) -> Value {
        let o = &self.options;
        json!({
            "replaceText": o.replace_text,
            "twoInitialCaps": o.two_initial_caps,
            "capSentences": o.cap_sentences,
            "capCells": o.cap_cells,
            "capDays": o.cap_days,
            "capsLock": o.caps_lock,
            "smartQuotes": o.smart_quotes,
            "fractions": o.fractions,
            "ordinals": o.ordinals,
            "boldItalic": o.bold_italic,
            "dashes": o.dashes,
            "links": o.links,
            "bullets": o.bullets,
            "numbering": o.numbering,
            "borderLines": o.border_lines,
            "tabIndent": o.tab_indent,
            "headingStyles": o.heading_styles,
            "doubleSpacePeriod": o.double_space_period,
            "autoAddFirstLetter": o.auto_add_first_letter,
            "autoAddInitialCaps": o.auto_add_initial_caps,
        })
    }
}

/// Run a list change at once and take the lists it reports back.
fn list_change(app: &mut WordApp, f: &mut AutoCorrectForm, params: Value) -> bool {
    match app.run("tools.autocorrect", params) {
        Ok(_) => {
            let now = app.session.prefs.autocorrect.clone();
            f.options.entries = now.entries;
            f.options.removed = now.removed;
            f.options.exceptions = now.exceptions;
            f.options.exceptions_removed = now.exceptions_removed;
            f.message.clear();
            true
        }
        Err(e) => {
            f.message = e;
            false
        }
    }
}

/// Runs the list changes the body filed. The body never holds the app, so the changes run here,
/// in the frame that filed them. Always `false`: the hook lives as long as the dialog, which may
/// file again.
fn settle_lists(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<AutoCorrectForm>>(id) else { return Settle::Keep };
    let mut form = host.form().clone();
    if form.pending.is_empty() {
        return Settle::Keep;
    }
    for edit in std::mem::take(&mut form.pending) {
        match edit {
            ListEdit::Entry(params) => {
                if list_change(app, &mut form, params) {
                    form.from.clear();
                    form.to.clear();
                }
            }
            ListEdit::Exception(params) => {
                if list_change(app, &mut form, params) {
                    form.exception.clear();
                }
            }
        }
    }
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<AutoCorrectForm>>(id) {
        *host.form_mut() = form;
    }
    Settle::Keep
}

const REPLACE: ColumnKey = ColumnKey::new("replace");
const WITH: ColumnKey = ColumnKey::new("with");
const TABS: [u8; 3] = [0, 1, 2];

fn tab_name(tab: u8) -> Label {
    match tab {
        0 => Label::from(tl!("AutoCorrect").to_string()),
        1 => Label::from(tl!("AutoFormat As You Type").to_string()),
        _ => Label::from(tl!("Math AutoCorrect").to_string()),
    }
}

/// A table of `(first, second)` rows keyed by the first column; the key of the row the user
/// picked this pass. `second` adds the With column.
fn list_table(ui: &mut Ui, name: &str, rows: &[(String, String)], second: bool, selectable: bool) -> Option<String> {
    let mut columns = vec![ColumnSpec::new(REPLACE, Label::from(name.to_string()), ColumnWidth::Controls(12.0))];
    if second {
        columns.push(ColumnSpec::new(WITH, Label::from(tl!("With:").to_string()), ColumnWidth::Content));
    }
    let source = keyed(rows, |r: &(String, String)| r.0.clone());
    let out = Table::<String>::new(Label::from(name.to_string()), columns)
        .select(if selectable { SelectMode::Single } else { SelectMode::None })
        .striped(true)
        // The dialog's body scrolls, so the table caps its rows and scrolls the rest itself.
        .max_rows(8)
        .empty(Label::from(tl!("(none)").to_string()))
        .show_items(ui, &source, &mut |key, icx| {
            let with = rows.iter().find(|r| r.0 == *key).map(|r| r.1.clone()).unwrap_or_default();
            let row = ListRow::new(key.clone()).cells(icx.cells());
            if second { row.cell(WITH, with) } else { row }
        });
    let mut picked = None;
    for event in out.events {
        if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
            && let Some(key) = keys.into_iter().last()
        {
            picked = Some(key);
        }
    }
    picked
}

impl AutoCorrectForm {
    /// The AutoCorrect page: capitalisation and replacement switches, the Exceptions editor and the
    /// Replace / With list.
    fn autocorrect_tab(&mut self, ui: &mut Ui) {
        Checkbox::new(Bind::bound(&mut self.options.two_initial_caps)).label(Label::from(tl!("Correct TWo INitial CApitals").to_string())).show(ui);
        Checkbox::new(Bind::bound(&mut self.options.cap_sentences))
            .label(Label::from(tl!("Capitalize first letter of sentences").to_string()))
            .show(ui);
        if clicked(&Button::new(Label::from(tl!("Exceptions…").to_string())).show(ui)) {
            self.show_exceptions = !self.show_exceptions;
        }
        if self.show_exceptions {
            self.exceptions(ui);
        }
        Checkbox::new(Bind::bound(&mut self.options.cap_cells))
            .label(Label::from(tl!("Capitalize first letter of table cells").to_string()))
            .show(ui);
        Checkbox::new(Bind::bound(&mut self.options.cap_days)).label(Label::from(tl!("Capitalize names of days").to_string())).show(ui);
        Checkbox::new(Bind::bound(&mut self.options.caps_lock))
            .label(Label::from(tl!("Correct accidental usage of cAPS LOCK key").to_string()))
            .show(ui);
        Checkbox::new(Bind::bound(&mut self.options.replace_text)).label(Label::from(tl!("Replace text as you type").to_string())).show(ui);
        Row::new().show(ui, |ui, _| {
            Field::new(Label::from(tl!("Replace:").to_string())).show(ui, |ui, _| {
                TextField::new(Label::from(tl!("Replace:").to_string()), Bind::bound(&mut self.from)).show(ui);
            });
            Field::new(Label::from(tl!("With:").to_string())).show(ui, |ui, _| {
                TextField::new(Label::from(tl!("With:").to_string()), Bind::bound(&mut self.to)).show(ui);
            });
        });
        // The list, filtered to what's typed in Replace (as Word scrolls to it).
        let list = self.options.list();
        let typed = self.from.trim().to_lowercase();
        let shown: Vec<(String, String)> = list
            .iter()
            .filter(|(a, _, _)| typed.is_empty() || a.to_lowercase().starts_with(&typed))
            .take(500)
            .map(|(a, b, _)| (a.clone(), b.clone()))
            .collect();
        if let Some(key) = list_table(ui, tl!("Replace:"), &shown, true, true)
            && let Some((a, b)) = shown.iter().find(|(a, _)| *a == key)
        {
            self.from = a.clone();
            self.to = b.clone();
        }
        let exists = list.iter().any(|(a, _, _)| *a == self.from);
        let same = list.iter().any(|(a, b, _)| *a == self.from && *b == self.to);
        let can_add = !self.from.trim().is_empty() && !same;
        Row::new().show(ui, |ui, _| {
            let name = if exists { tl!("Replace") } else { tl!("Add") };
            if clicked(&Button::new(Label::from(name.to_string())).enabled(can_add).show(ui)) {
                let edit = ListEdit::Entry(json!({"add": {"from": self.from, "to": self.to}}));
                self.pending.push(edit);
            }
            if clicked(&Button::new(Label::from(tl!("Delete").to_string())).enabled(exists).show(ui)) {
                let edit = ListEdit::Entry(json!({"delete": self.from}));
                self.pending.push(edit);
            }
        });
    }

    /// The exceptions editor under the Exceptions… button.
    fn exceptions(&mut self, ui: &mut Ui) {
        GroupBox::new(Label::from(tl!("Exceptions…").to_string()), None).show(ui, |ui, _| {
            Checkbox::new(Bind::bound(&mut self.options.auto_add_first_letter))
                .label(Label::from(tl!("Automatically add words to list (First Letter)").to_string()))
                .show(ui);
            Checkbox::new(Bind::bound(&mut self.options.auto_add_initial_caps))
                .label(Label::from(tl!("Automatically add words to list (INitial CAps)").to_string()))
                .show(ui);
            let list = self.options.exception_list();
            let rows: Vec<(String, String)> = list.iter().map(|e| (e.clone(), String::new())).collect();
            if let Some(picked) = list_table(ui, tl!("Don't capitalize after:"), &rows, false, true) {
                self.exception = picked;
            }
            TextField::new(Label::from(tl!("Don't capitalize after:").to_string()), Bind::bound(&mut self.exception)).show(ui);
            let known = list.contains(&self.exception.trim().to_lowercase());
            let can_add = !self.exception.trim().is_empty() && !known;
            Row::new().show(ui, |ui, _| {
                if clicked(&Button::new(Label::from(tl!("Add").to_string())).enabled(can_add).show(ui)) {
                    let edit = ListEdit::Exception(json!({"addException": self.exception}));
                    self.pending.push(edit);
                }
                if clicked(&Button::new(Label::from(tl!("Delete").to_string())).enabled(known).show(ui)) {
                    let edit = ListEdit::Exception(json!({"deleteException": self.exception}));
                    self.pending.push(edit);
                }
            });
        });
    }

    /// The AutoFormat As You Type page.
    fn autoformat_tab(&mut self, ui: &mut Ui) {
        let o = &mut self.options;
        GroupBox::new(Label::from(tl!("Replace as you type").to_string()), None).show(ui, |ui, _| {
            Checkbox::new(Bind::bound(&mut o.smart_quotes)).label(Label::from(tl!("Smart quotes instead of straight quotes").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut o.fractions)).label(Label::from(tl!("Fractions (1/2) as fraction characters (½)").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut o.ordinals)).label(Label::from(tl!("Ordinals (1st) as superscript").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut o.bold_italic)).label(Label::from(tl!("*Bold* and _italic_ as real formatting").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut o.dashes)).label(Label::from(tl!("Hyphens (--) as dashes (—)").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut o.links)).label(Label::from(tl!("Internet addresses as links").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut o.double_space_period)).label(Label::from(tl!("Add a period with double-space").to_string())).show(ui);
        });
        GroupBox::new(Label::from(tl!("Apply as you type").to_string()), None).show(ui, |ui, _| {
            Checkbox::new(Bind::bound(&mut o.bullets)).label(Label::from(tl!("Automatic bulleted lists").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut o.numbering)).label(Label::from(tl!("Automatic numbered lists").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut o.border_lines)).label(Label::from(tl!("Border lines").to_string())).show(ui);
            Checkbox::new(Bind::bound(&mut o.heading_styles)).label(Label::from(tl!("Built-in Heading styles").to_string())).show(ui);
        });
        GroupBox::new(Label::from(tl!("Automatically as you type").to_string()), None).show(ui, |ui, _| {
            Checkbox::new(Bind::bound(&mut o.tab_indent))
                .label(Label::from(tl!("Set left- and first-indent with tabs and backspaces").to_string()))
                .show(ui);
        });
    }

    /// The Math AutoCorrect page: each `\name` with its character, filtered by the typed prefix.
    fn math_tab(&mut self, ui: &mut Ui) {
        Text::new(Label::from(tl!("In equations, typing a name after a backslash replaces it with its symbol.").to_string())).show(ui);
        Field::new(Label::from(tl!("Replace:").to_string())).show(ui, |ui, _| {
            TextField::new(Label::from(tl!("Replace:").to_string()), Bind::bound(&mut self.math_filter)).show(ui);
        });
        let filter = self.math_filter.trim().trim_start_matches('\\').to_string();
        let rows: Vec<(String, String)> = wordcraft_doc::math_symbols::SYMBOLS
            .iter()
            .filter(|(n, _)| filter.is_empty() || n.starts_with(filter.as_str()))
            .map(|(name, c)| (format!("\\{name}"), c.to_string()))
            .collect();
        list_table(ui, tl!("Replace:"), &rows, true, false);
    }
}

impl WordForm for AutoCorrectForm {
    fn key(&self) -> &'static str {
        "autoCorrect"
    }

    fn title(&self) -> Label {
        Label::from(tl!("AutoCorrect").to_string())
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        Stack::new().show(ui, |ui, _| {
            let source = keyed(&TABS, |t: &u8| *t);
            let strip = TabStrip::new(Label::from(tl!("AutoCorrect Options").to_string()))
                .kind(TabKind::Text)
                .shown(self.tab)
                .natural_height(true)
                .show_items(ui, &source, &mut |key, _| Tab::new(tab_name(*key)));
            for event in strip.response.events {
                if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                    && let Some(key) = keys.into_iter().last()
                {
                    self.tab = key.min(2);
                }
            }
            match self.tab {
                0 => self.autocorrect_tab(ui),
                1 => self.autoformat_tab(ui),
                _ => self.math_tab(ui),
            }
            if !self.message.is_empty() {
                Notice::new(Tone::Bad, Label::from(self.message.clone())).layout(NoticeLayout::Inline).show(ui);
            }
        });
        true
    }

    fn commands(&self) -> Vec<(String, Value)> {
        vec![("tools.autocorrect".to_string(), self.params())]
    }
}

#[cfg(test)]
mod tests {
    use egui::vec2;
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::*;
    use crate::{Services, WordApp};

    fn app(text: &str) -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::from_text(text)), Services::default())
    }

    #[test]
    fn autocorrect_entries_persist_in_prefs_and_junk_is_ignored() {
        let mut a = app("");
        a.run("tools.autocorrect", json!({"add": {"from": "brb", "to": "be right back"}, "capDays": false})).unwrap();
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        let mut b = app("");
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.session.prefs.autocorrect.replacement("brb").as_deref(), Some("be right back"));
        assert!(!b.session.prefs.autocorrect.cap_days);
        for c in "brb ".chars() {
            b.session.run("text.insert", &json!({"text": c.to_string()})).unwrap();
        }
        assert_eq!(b.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Be right back ");
        // A damaged AutoCorrect section reads as the defaults; the rest of the file still loads.
        let mut v: Value = serde_json::from_str(&saved).unwrap();
        v["editing"]["autocorrect"] = json!({"entries": 7, "capDays": "x"});
        v["author"] = json!("Ada");
        let mut c = app("");
        c.apply_prefs(serde_json::from_value(v).unwrap());
        assert_eq!(c.session.prefs.autocorrect, AutoCorrectPrefs::default());
        assert_eq!(c.session.author, "Ada");
        // Hostile entries are dropped on load.
        let mut v: Value = serde_json::from_str(&saved).unwrap();
        v["editing"]["autocorrect"]["entries"] = json!([["", "x"], ["ok", "fine"], ["ok", "dup"], ["a\nb", "x"]]);
        let mut d = app("");
        d.apply_prefs(serde_json::from_value(v).unwrap());
        assert_eq!(d.session.prefs.autocorrect.entries, vec![("ok".to_string(), "fine".to_string())]);
    }

    /// #2848 through the real dialog: tick "Add a period with double-space" on the AutoFormat As
    /// You Type tab, press OK, and a double Space typed in the page makes a period.
    #[test]
    fn the_double_space_option_is_set_in_the_dialog_and_works_when_typing() {
        use egui_kittest::kittest::Queryable;
        let mut app = app("");
        app.run("tools.autocorrect", json!({})).unwrap();
        let mut h = crate::test_harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
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
        h.get_by_label("AutoFormat As You Type").click();
        for _ in 0..3 {
            h.step();
        }
        h.get_by_label("Add a period with double-space").click();
        for _ in 0..3 {
            h.step();
        }
        h.get_by_label("OK").click();
        for _ in 0..4 {
            h.step();
        }
        assert!(h.state().dialog_host.stack().is_empty(), "OK closes the dialog");
        assert!(h.state().session.prefs.autocorrect.double_space_period, "the option is on");
        let ctx = h.ctx.clone();
        h.state_mut().canvas.focus_page(&ctx);
        for t in ["Hello", " ", " ", "world"] {
            h.event(egui::Event::Text(t.to_string()));
            h.step();
        }
        assert_eq!(h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Hello. world");
    }
}
