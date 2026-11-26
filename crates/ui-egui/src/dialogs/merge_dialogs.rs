use craft::behavior::{ColumnWidth, SelectionChange, keyed};
use craft::theme::variant::Tone;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnKey, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::{FormButtons, FormCx, HostedReport, ReportNote};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::table::Table;
use craft::ui::text::{NumberField, TextField};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::host::{Asks, DialogHooks, FormHost, Settle, WordForm, open_form, open_form_with, open_with};
use super::mailings::ADDRESS_PARTS;
use crate::WordApp;

/// Opens the mail-merge dialog `name` (`insertMergeField`, `selectTable`, `findRecipient`, `mergeRecords`,
/// `printRecords`, `ruleIf`, `ruleSkipIf`, `matchFields`, `checkErrors`); `None` for any other name.
/// `selectTable` needs `params["sheets"]`; the rest of `params` is what OK loads.
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "insertMergeField" => Some(open_insert_merge_field(app, parent)),
        "selectTable" => {
            let sheets: Vec<String> = params.get("sheets").and_then(Value::as_array)?.iter().filter_map(Value::as_str).map(str::to_string).collect();
            if sheets.is_empty() {
                return None;
            }
            let mut load = params.clone();
            if let Some(o) = load.as_object_mut() {
                o.remove("sheets");
            }
            Some(open_form(app, parent, SelectTableForm { sheets, selected: 0, headers: true, params: load }))
        }
        "findRecipient" => Some(open_find_recipient(app, parent)),
        "mergeRecords" | "printRecords" => Some(open_merge_records(app, parent, name == "printRecords")),
        "ruleIf" | "ruleSkipIf" => Some(open_merge_rule(app, parent, name == "ruleSkipIf")),
        "matchFields" | "checkErrors" => {
            let id = if name == "matchFields" { "mailings.matchFields" } else { "mailings.checkErrors" };
            let result = app.session.run(id, &json!({})).unwrap_or_default();
            report(app, id, &result)
        }
        _ => None,
    }
}

/// Rows of a list of plain names the user picks one of: the picked name, if any, and whether a row was activated
/// (Enter or a double click).
fn pick_name(ui: &mut egui::Ui, label: &str, names: &[String], current: &str) -> (Option<String>, bool) {
    let idx: Vec<usize> = (0..names.len()).collect();
    let response =
        List::<usize>::new(Label::from(label.to_owned())).select(SelectMode::Single).show_items(ui, &keyed(&idx, |i| *i), &mut |key, _| {
            let name = names.get(*key).cloned().unwrap_or_default();
            let on = name == current;
            ListRow::new(name).selected(on)
        });
    let mut picked = None;
    let mut activated = false;
    for e in response.events {
        match e {
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => {
                if let Some(n) = keys.first().and_then(|k| names.get(*k)) {
                    picked = Some(n.clone());
                }
            }
            CollectionEvent::Selection { change: SelectionChange::Toggle(k), .. } => {
                if let Some(n) = names.get(k) {
                    picked = Some(n.clone());
                }
            }
            CollectionEvent::Activate(k) => {
                if let Some(n) = names.get(k) {
                    picked = Some(n.clone());
                    activated = true;
                }
            }
            _ => {}
        }
    }
    (picked, activated)
}

// ---------------------------------------------------------------------------------------------
// Insert Merge Field

/// Insert Merge Field: one of the recipient list's fields, or a typed name. Modeless.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InsertMergeFieldForm {
    /// The list's fields, copied in before every pass (it stays open while the list changes).
    #[serde(skip)]
    pub fields: Vec<String>,
    pub field: String,
}

impl WordForm for InsertMergeFieldForm {
    fn key(&self) -> &'static str {
        "insertMergeField"
    }
    fn title(&self) -> Label {
        tl!("Insert Merge Field").into()
    }
    fn ok_label(&self) -> Option<Label> {
        Some(tl!("Insert").into())
    }
    fn modeless(&self) -> bool {
        true
    }
    fn body(&mut self, ui: &mut egui::Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        if self.fields.is_empty() {
            Text::new(tl!("Select recipients first to choose from their fields.")).show(ui);
        } else if let (Some(name), _) = pick_name(ui, tl!("Fields:"), &self.fields, &self.field) {
            self.field = name;
        }
        TextField::new(tl!("Field:"), Bind::bound(&mut self.field)).show(ui);
        !self.field.trim().is_empty()
    }
    fn commands(&self) -> Vec<(String, Value)> {
        vec![("mailings.insertField".into(), json!({"field": self.field.trim()}))]
    }
}

fn copy_headers(app: &mut WordApp, id: DialogId) {
    let headers = app.session.merge.headers.clone();
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<InsertMergeFieldForm>>(id) {
        host.form_mut().fields = headers;
    }
}

/// Opens Insert Merge Field on the first field of the list.
pub(crate) fn open_insert_merge_field(app: &mut WordApp, parent: Option<DialogId>) -> DialogId {
    let fields = app.session.merge.headers.clone();
    let form = InsertMergeFieldForm { field: fields.first().cloned().unwrap_or_default(), fields };
    open_form_with(app, parent, form, DialogHooks::prepare_only(copy_headers))
}

// ---------------------------------------------------------------------------------------------
// Select Table

/// Select Table: which sheet of a workbook holds the recipients, and whether its first row names the fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SelectTableForm {
    /// The workbook's sheet names: what the dialog is asked about, so they are fields (a skipped field is
    /// lost in a `set_json_field` round trip, and the control channel could not list the choices).
    sheets: Vec<String>,
    pub selected: usize,
    pub headers: bool,
    /// The file `mailings.recipients` loads; kept through a round trip for the same reason.
    params: Value,
}

impl WordForm for SelectTableForm {
    fn key(&self) -> &'static str {
        "selectTable"
    }
    fn title(&self) -> Label {
        tl!("Select Table").into()
    }
    fn body(&mut self, ui: &mut egui::Ui, _cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool {
        Text::new(tl!("Choose the sheet that holds the recipients.")).show(ui);
        let current = self.sheets.get(self.selected).cloned().unwrap_or_default();
        let (picked, activated) = pick_name(ui, tl!("Sheets"), &self.sheets, &current);
        if let Some(name) = picked
            && let Some(i) = self.sheets.iter().position(|s| *s == name)
        {
            self.selected = i;
        }
        if activated {
            asks.push(Asks::Accept);
        }
        Checkbox::new(Bind::bound(&mut self.headers)).label(tl!("First row of data contains column headers")).show(ui);
        true
    }
    fn commands(&self) -> Vec<(String, Value)> {
        let mut p = self.params.clone();
        if let Some(o) = p.as_object_mut() {
            o.insert("sheet".into(), json!(self.selected + 1));
            o.insert("headers".into(), json!(self.headers));
        }
        vec![("mailings.recipients".into(), p)]
    }
}

/// Opens Select Table over a workbook's `sheets`; OK loads `params` (the file) with the chosen sheet.
pub(crate) fn open_select_table(app: &mut WordApp, sheets: Vec<String>, params: Value) -> DialogId {
    open_form(app, None, SelectTableForm { sheets, selected: 0, headers: true, params })
}

// ---------------------------------------------------------------------------------------------
// Find Recipient

/// Find Recipient: the first record with a field containing the text.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FindRecipientForm {
    pub text: String,
}

impl WordForm for FindRecipientForm {
    fn key(&self) -> &'static str {
        "findRecipient"
    }
    fn title(&self) -> Label {
        tl!("Find Recipient").into()
    }
    fn ok_label(&self) -> Option<Label> {
        Some(tl!("Find").into())
    }
    fn body(&mut self, ui: &mut egui::Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        TextField::new(tl!("Find:"), Bind::bound(&mut self.text)).show(ui);
        !self.text.trim().is_empty()
    }
    fn commands(&self) -> Vec<(String, Value)> {
        vec![("mailings.findRecipient".into(), json!({"text": self.text.trim()}))]
    }
}

/// Opens Find Recipient.
pub(crate) fn open_find_recipient(app: &mut WordApp, parent: Option<DialogId>) -> DialogId {
    open_form(app, parent, FindRecipientForm::default())
}

// ---------------------------------------------------------------------------------------------
// Merge Records

/// Merge to New Document / Print: which records. `choice` is 0 All, 1 Current record, 2 From–To.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MergeRecordsForm {
    #[serde(skip)]
    pub print: bool,
    pub choice: u8,
    pub from: Option<f64>,
    pub to: Option<f64>,
}

impl MergeRecordsForm {
    /// The command's parameters; `None` for a range that isn't two numbers.
    fn params(&self) -> Option<Value> {
        match self.choice {
            0 => Some(json!({"records": "all"})),
            1 => Some(json!({"records": "current"})),
            _ => match (self.from, self.to) {
                (Some(f), Some(t)) if f >= 0.0 && t >= 0.0 => Some(json!({"from": f as u64, "to": t as u64})),
                _ => None,
            },
        }
    }
}

impl WordForm for MergeRecordsForm {
    fn key(&self) -> &'static str {
        if self.print { "printRecords" } else { "mergeRecords" }
    }
    fn title(&self) -> Label {
        tl!(if self.print { "Merge to Printer" } else { "Merge to New Document" }).into()
    }
    fn ok_label(&self) -> Option<Label> {
        self.print.then(|| tl!("Print").into())
    }
    fn body(&mut self, ui: &mut egui::Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let mut choice = self.choice;
        RadioGroup::new(tl!("Records"), Bind::bound(&mut choice)).show(ui, |ui, _| {
            Radio::new(0u8).label(tl!("All")).show(ui);
            Radio::new(1u8).label(tl!("Current record")).show(ui);
            Radio::new(2u8).label(tl!("From:")).show(ui);
        });
        self.choice = choice;
        // Typing a number picks the range.
        if let Some(v) = NumberField::new(tl!("From:"), Bind::given(self.from)).show(ui).value().copied() {
            self.from = v;
            self.choice = 2;
        }
        if let Some(v) = NumberField::new(tl!("To:"), Bind::given(self.to)).show(ui).value().copied() {
            self.to = v;
            self.choice = 2;
        }
        self.params().is_some()
    }
    /// An empty or out-of-range selection is refused before any save prompt.
    fn check(&self, app: &WordApp) -> Result<(), Label> {
        let p = self.params().ok_or_else(|| -> Label { tl!("Enter a number.").into() })?;
        wordcraft_engine::cmd::mailings::merge_range(&app.session, &p).map(|_| ()).map_err(|e| Label::from(e.to_string()))
    }
    fn commands(&self) -> Vec<(String, Value)> {
        let id = if self.print { "mailings.printDocuments" } else { "mailings.finish" };
        self.params().map(|p| vec![(id.to_string(), p)]).unwrap_or_default()
    }
}

/// Opens Merge Records (`print`: Merge to Printer), over all records.
pub(crate) fn open_merge_records(app: &mut WordApp, parent: Option<DialogId>, print: bool) -> DialogId {
    let last = app.session.merge.rows.len() as f64;
    open_form(app, parent, MergeRecordsForm { print, choice: 0, from: Some(1.0), to: Some(last) })
}

// ---------------------------------------------------------------------------------------------
// Merge Rule

/// Rules › If…Then…Else / Skip Record If.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MergeRuleForm {
    /// `IF` or `SKIPIF`.
    #[serde(skip)]
    pub rule: String,
    #[serde(skip)]
    pub fields: Vec<String>,
    pub field: String,
    pub value: String,
    pub then: String,
    #[serde(rename = "else")]
    pub els: String,
}

impl MergeRuleForm {
    /// The `mailings.rules` parameters, once a field is named.
    pub fn params(&self) -> Option<Value> {
        let field = self.field.trim();
        if field.is_empty() {
            return None;
        }
        Some(if self.rule == "SKIPIF" {
            json!({"rule": "SKIPIF", "field": field, "value": self.value})
        } else {
            json!({"rule": "IF", "field": field, "value": self.value, "then": self.then, "else": self.els})
        })
    }
}

impl WordForm for MergeRuleForm {
    fn key(&self) -> &'static str {
        if self.rule == "SKIPIF" { "ruleSkipIf" } else { "ruleIf" }
    }
    fn title(&self) -> Label {
        tl!(if self.rule == "SKIPIF" { "Skip Record If" } else { "If…Then…Else" }).into()
    }
    fn body(&mut self, ui: &mut egui::Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let skip = self.rule == "SKIPIF";
        Text::new(tl!(if skip {
            "Leave a recipient out of the merge when a field has this value."
        } else {
            "Insert one text or another, depending on a field's value."
        }))
        .show(ui);
        if self.fields.is_empty() {
            TextField::new(tl!("Field:"), Bind::bound(&mut self.field)).show(ui);
        } else {
            let entries: Vec<Entry<String>> = self.fields.iter().map(|f| Entry::new(f.clone(), Label::from(f.clone()))).collect();
            if let Some(f) = Select::new(tl!("Field:"), Bind::given(self.field.clone()), &entries).show(ui).value().cloned() {
                self.field = f;
            }
        }
        TextField::new(tl!("Equals:"), Bind::bound(&mut self.value)).show(ui);
        if !skip {
            TextField::new(tl!("Then insert:"), Bind::bound(&mut self.then)).show(ui);
            TextField::new(tl!("Otherwise insert:"), Bind::bound(&mut self.els)).show(ui);
        }
        self.params().is_some()
    }
    fn commands(&self) -> Vec<(String, Value)> {
        self.params().map(|p| vec![("mailings.rules".to_string(), p)]).unwrap_or_default()
    }
}

/// Opens Merge Rule (`skip`: Skip Record If) on the list's first field.
pub(crate) fn open_merge_rule(app: &mut WordApp, parent: Option<DialogId>, skip: bool) -> DialogId {
    let fields = app.session.merge.headers.clone();
    let form = MergeRuleForm {
        rule: if skip { "SKIPIF" } else { "IF" }.into(),
        field: fields.first().cloned().unwrap_or_default(),
        fields,
        ..Default::default()
    };
    open_form(app, parent, form)
}

// ---------------------------------------------------------------------------------------------
// Match Fields and Check for Errors

/// Match Fields: which recipient-list field fills each part of an address block and greeting line. Close only.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchFieldsForm {
    #[serde(skip)]
    fields: Vec<String>,
    /// `mailings.matchFields`' `address`: part key to field name.
    #[serde(skip)]
    address: Value,
}

impl WordForm for MatchFieldsForm {
    fn key(&self) -> &'static str {
        "matchFields"
    }
    fn title(&self) -> Label {
        tl!("Match Fields").into()
    }
    // PLAN: assumed FormButtons (dialogs-w5)
    fn buttons(&self) -> FormButtons {
        FormButtons::Close
    }
    fn body(&mut self, ui: &mut egui::Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        if self.fields.is_empty() {
            Text::new(tl!("Select recipients first to match their fields.")).show(ui);
            return true;
        }
        Text::new(tl!("The recipient-list field that fills each part of an address block and greeting line.")).show(ui);
        const PART: ColumnKey = ColumnKey::new("part");
        const FIELD: ColumnKey = ColumnKey::new("field");
        let columns =
            vec![ColumnSpec::new(PART, tl!("Address part"), ColumnWidth::Content), ColumnSpec::new(FIELD, tl!("Field"), ColumnWidth::fill(1.0))];
        let parts: Vec<usize> = (0..ADDRESS_PARTS.len()).collect();
        let address = &self.address;
        Table::<usize>::new(tl!("Address parts"), columns).striped(true).show_items(ui, &keyed(&parts, |i| *i), &mut |key, icx| {
            let (part, k) = ADDRESS_PARTS.get(*key).copied().unwrap_or(("", ""));
            let row = ListRow::new(tl!(part)).cells(icx.cells()).cell(PART, tl!(part));
            match address.get(k).and_then(Value::as_str) {
                Some(f) => row.cell(FIELD, f.to_string()),
                None => row.cell(FIELD, tl!("(not matched)")).cell_muted(FIELD),
            }
        });
        true
    }
    fn commands(&self) -> Vec<(String, Value)> {
        Vec::new()
    }
}

/// Puts a report's Copy text on the clipboard; spent once the report is gone.
fn copy_report(app: &mut WordApp, ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(report) = app.dialog_host.get_mut_by_id::<HostedReport>(id) else {
        return Settle::Done;
    };
    if let Some(text) = report.take_copied() {
        ctx.copy_text(text);
    }
    Settle::Keep
}

/// The dialog that shows a report command's result: Match Fields, Check for Errors.
pub(crate) fn report(app: &mut WordApp, id: &str, result: &Value) -> Option<DialogId> {
    let strings = |k: &str| -> Vec<String> {
        result.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    match id {
        "mailings.matchFields" => {
            Some(open_form(app, None, MatchFieldsForm { fields: strings("fields"), address: result.get("address").cloned().unwrap_or(Value::Null) }))
        }
        "mailings.checkErrors" => {
            let unknown = strings("unknownFields");
            let records = result.get("records").and_then(Value::as_u64).unwrap_or(0).to_string();
            let summary = if unknown.is_empty() {
                tl!("No errors found: every merge field is in the recipient list.")
            } else {
                tl!("These merge fields aren't in the recipient list:")
            };
            let mut notes: Vec<ReportNote> = unknown.into_iter().map(|f| ReportNote::new(Tone::Bad, f)).collect();
            notes.push(ReportNote::new(Tone::Info, crate::i18n::fmt(tl!("Recipients: {count}"), &[("count", &records)])));
            let report = HostedReport::new(tl!("Check for Errors"), summary, vec![]).notes(notes);
            Some(open_with(app, None, report, DialogHooks { prepare: None, settle: copy_report }))
        }
        _ => None,
    }
}
