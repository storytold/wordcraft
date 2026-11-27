use std::cell::RefCell;

use craft::behavior::{ColumnWidth, EditEvent, IconId, keyed};
use craft::table::ui::{
    Condition, ConditionBuilder, ConditionFields, ConditionGroup, ConditionItem, ConditionKind, ConditionLayout, ConditionValue, Join, Match,
};
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnEvent, ColumnKey, ColumnSpec, EditResponse, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _,
};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, StackDialog};
use craft::ui::display::Text;
use craft::ui::list::{ListRow, RowEvent, TrailingFace, TrailingPart};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::sections::Section;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_engine::cmd::mailings::{FilterOp, FilterRule, SortKey};

use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

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
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
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
}

/// One filter condition row: `field op value`, joined to the row above with And or Or.
#[derive(Clone, Debug, PartialEq, Serialize)]
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

/// Whether the operator `op` compares against a value; blank and not blank do not.
fn takes_value(op: &str) -> bool {
    !matches!(op, "blank" | "notBlank")
}

impl FilterRow {
    /// The builder's rule for this row.
    fn condition(&self) -> Condition {
        let value = if takes_value(self.op.id()) { ConditionValue::Text(self.value.clone()) } else { ConditionValue::None };
        let mut c = Condition::new(self.field.clone(), self.op.id(), value);
        c.join = if self.or { Join::Or } else { Join::And };
        c
    }

    /// The row a builder rule stands for; `None` for an operator the engine doesn't know.
    fn of_condition(c: &Condition) -> Option<FilterRow> {
        let op = FilterOp::ALL.into_iter().find(|o| o.id() == c.operator)?;
        let value = match &c.value {
            ConditionValue::Text(t) => t.clone(),
            _ => String::new(),
        };
        Some(FilterRow { field: c.field.clone(), op, value, or: c.join == Join::Or })
    }
}

/// Whether `f` names one of `fields` (case-insensitively, trimmed).
fn known_field(fields: &[String], f: &str) -> bool {
    !f.trim().is_empty() && fields.iter().any(|h| h.trim().eq_ignore_ascii_case(f.trim()))
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
    /// drops, as the engine drops it when the list is edited). The first rule has nothing to
    /// join, so its `or` never counts.
    fn engine(&self, fields: &[String]) -> (Vec<FilterRule>, Vec<SortKey>) {
        let rules = self
            .rules
            .iter()
            .filter(|r| known_field(fields, &r.field))
            .enumerate()
            .map(|(n, r)| FilterRule { field: r.field.clone(), op: r.op, value: r.value.clone(), or: r.or && n > 0 })
            .collect();
        let sort = self.sort.iter().filter(|(f, _)| known_field(fields, f)).map(|(f, d)| SortKey { field: f.clone(), descending: *d }).collect();
        (rules, sort)
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

    /// The rules as the builder's one joined list.
    fn conditions(&self) -> ConditionGroup {
        // PLAN: assumed `ConditionItem::Rule(Condition)` and `Match::All`.
        ConditionGroup::new(Match::All, self.rules.iter().map(|r| ConditionItem::Rule(r.condition())).collect())
    }

    /// Takes the builder's rules back; a rule on a field the list no longer has drops.
    fn set_conditions(&mut self, group: &ConditionGroup, fields: &[String]) {
        self.rules = group
            .items
            .iter()
            .filter_map(|item| match item {
                ConditionItem::Rule(c) if known_field(fields, &c.field) => FilterRow::of_condition(c),
                _ => None,
            })
            .collect();
    }
}

/// Each record's place in the merge (None: filtered out), when `filter` filters or sorts.
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

/// What the Recipient List dialog edits: the table and its filter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecipientDraft {
    pub fields: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub filter: RecipientFilter,
}

/// The Recipient List dialog: a typed table of recipients, Find in list, and Filter and Sort. OK
/// files the draft; the dialog's settle hook loads it and answers.
pub struct RecipientListDialog {
    /// The table, its filter and sort.
    pub draft: RecipientDraft,
    /// Whether a grid cell's edit session is running (a `Begin` until `Commit` or `Cancel`).
    editing: bool,
    /// "Use an Existing List…" was pressed; settle files the pick and closes.
    pick: bool,
    held: HeldAnswer<RecipientDraft>,
}

impl RecipientListDialog {
    fn new(draft: RecipientDraft) -> Self {
        RecipientListDialog { draft, editing: false, pick: false, held: HeldAnswer::new() }
    }
}

/// Opens the Recipient List (`recipientList`: the session's list when it has one, else a new
/// list; `newRecipientList`: always a new one); `None` for any other name.
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    if !matches!(name, "recipientList" | "newRecipientList") {
        return None;
    }
    let m = &app.session.merge;
    let draft = if name == "recipientList" && !m.headers.is_empty() {
        // Every record (not only the filtered ones), with the filter to edit.
        RecipientDraft { fields: m.headers.clone(), rows: m.source.clone(), filter: RecipientFilter::from_session(m) }
    } else {
        let fields: Vec<String> = NEW_LIST_FIELDS.iter().map(ToString::to_string).collect();
        let rows = vec![vec![String::new(); fields.len()]];
        RecipientDraft { fields, rows, filter: RecipientFilter::default() }
    };
    Some(open_with(app, parent, RecipientListDialog::new(draft), DialogHooks { prepare: None, settle }))
}

/// Files the held draft: loads the table, then applies the filter; a refusal is the dialog's.
/// "Use an Existing List…" hands over to the picker and closes.
/// Returns `true` only once the dialog is gone and its result is taken: the hook files again after a refusal.
fn settle(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some((filed, pick)) = app.dialog_host.get_mut_by_id::<RecipientListDialog>(id).map(|d| (d.held.take(), std::mem::take(&mut d.pick))) else {
        let _ = app.dialog_host.take_by_id::<RecipientListDialog>(id);
        return Settle::Done;
    };
    if pick {
        let _ = app.run("ui.openRecipientList", json!({}));
        app.dialog_host.close_by_id(id);
        return Settle::Keep;
    }
    let Some(draft) = filed else { return Settle::Keep };
    let result = recipient_table(&draft.fields, &draft.rows)
        .map_err(|e| tl!(e).to_string())
        .and_then(|params| app.load_recipients(params))
        // A new list starts unfiltered; the dialog's filter and sort apply to it.
        .and_then(
            |_| if draft.filter.is_empty() { Ok(Value::Null) } else { app.run("mailings.filterRecipients", draft.filter.params(&draft.fields)) },
        );
    if let Some(d) = app.dialog_host.get_mut_by_id::<RecipientListDialog>(id) {
        d.held.answer(result.map(|_| ()).map_err(Into::into));
    }
    Settle::Keep
}

/// What a grid or header field reported this pass.
enum Act {
    Cell(usize, usize, Option<String>, Option<bool>),
    Field(usize, Option<String>),
    RemoveField(usize),
    RemoveRow(usize),
}

/// The committed text of an edit and where its session stands: `Some(true)` after a `Begin`,
/// `Some(false)` after a `Commit` or `Cancel` (Tab off a cell commits).
fn edit_of(out: &EditResponse<String>) -> (Option<String>, Option<bool>) {
    let mut edge = None;
    for e in out.events.iter() {
        match e {
            EditEvent::Begin { .. } => edge = Some(true),
            EditEvent::Commit { .. } | EditEvent::Cancel { .. } => edge = Some(false),
            _ => {}
        }
    }
    (out.value().cloned(), edge)
}

/// The fields the filter builder offers: the table's, every one text.
struct RecipientFields {
    names: Vec<String>,
}

impl ConditionFields for RecipientFields {
    fn fields(&self) -> Vec<(String, String, Option<String>)> {
        self.names.iter().map(|n| (n.clone(), n.clone(), None)).collect()
    }

    fn kind(&self, _field: &str) -> ConditionKind {
        ConditionKind::Text
    }

    fn operators(&self, _field: &str) -> Vec<(String, String)> {
        FilterOp::ALL.into_iter().map(|op| (op.id().to_string(), tl!(op.label()).to_string())).collect()
    }

    fn default_value(&self, _field: &str, operator: &str) -> ConditionValue {
        if takes_value(operator) { ConditionValue::Text(String::new()) } else { ConditionValue::None }
    }
}

/// A recipient grid cell's edit and the rows it must hold, in one place.
fn recipient_body(ui: &mut egui::Ui, d: &mut RecipientDraft, editing: &mut bool, pick: &mut bool) -> bool {
    let RecipientDraft { fields, rows, filter } = d;
    Text::new(tl!("Type the recipients, or load them from a file.")).show(ui);
    if Button::new(tl!("Use an Existing List…")).show(ui).clicked() {
        *pick = true;
    }
    // Each record's place in the merge (None: filtered out), when there is a filter or sort.
    let order = merge_order(fields, rows, filter);
    // Nothing to leave out without a filter: the box shows unticked.
    if order.is_none() {
        filter.only_merged = false;
    }
    Checkbox::new(Bind::bound(&mut filter.only_merged)).label(tl!("Show only records that will be merged")).enabled(order.is_some()).show(ui);
    TextField::search(tl!("Find in list"), Bind::bound(&mut filter.find)).show(ui);
    // The rows the grid shows, by real index: edits and removes go to the right record.
    let fresh = visible_recipients(rows, order.as_deref(), &filter.find, filter.only_merged);
    // A press that ends a cell's edit (one on the row's Remove part) completes its click on release, a pass later: the
    // rows stay held until then, so the row the press landed on is still there for it.
    let gesture = filter.held.is_some() && ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
    let visible = hold_rows(fresh, &mut filter.held, *editing || gesture, rows.len());
    if !filter.find.trim().is_empty() {
        let count = visible.len().to_string();
        Text::new(crate::i18n::fmt(tl!("Matches: {count}"), &[("count", &count)])).show(ui);
    }

    let acts = RefCell::new(Vec::<Act>::new());
    let acts_ref = &acts;
    let can_remove = fields.len() > 1;
    let columns = fields
        .iter()
        .enumerate()
        .map(|(j, f)| ColumnSpec::new(ColumnKey::index(j as u32), f.clone(), ColumnWidth::fill(1.0)).removable(tl!("Remove field"), can_remove))
        .collect();
    // The dialog body scrolls, and a collection owns its own scroll: the grid shows this many records and scrolls
    // the rest itself.
    let mut table = Table::<usize>::new(tl!("Recipients"), columns).striped(true).max_rows(10);
    for (j, name) in fields.iter().enumerate() {
        table = table.header_field(ColumnKey::index(j as u32), TextField::new(tl!("Field name"), Bind::given(name.clone())), move |out| {
            acts_ref.borrow_mut().push(Act::Field(j, edit_of(&out).0));
        });
    }
    let widest = rows.len().to_string();
    let (fields_ro, rows_ro) = (&*fields, &*rows);
    let response = table.show_items(ui, &keyed(&visible, |r| *r), &mut |key, icx| {
        let r = *key;
        // The record's number in the merge; a filtered-out record reads muted.
        let (num, kept) = match order.as_ref().map(|o| o.get(r).copied().flatten()) {
            None => ((r + 1).to_string(), true),
            Some(Some(n)) => (n.to_string(), true),
            Some(None) => ("–".to_string(), false),
        };
        let mut row = ListRow::new(num.clone()).cells(icx.cells()).lead_text(num, widest.clone());
        for j in 0..fields_ro.len() {
            let text = rows_ro.get(r).and_then(|x| x.get(j)).cloned().unwrap_or_default();
            let field_name = fields_ro.get(j).map_or("", String::as_str);
            let label = crate::i18n::fmt(tl!("{field}, record {n}"), &[("field", field_name), ("n", &(r + 1).to_string())]);
            row = row.cell_field(ColumnKey::index(j as u32), TextField::new(label, Bind::given(text)), move |out| {
                let (value, edge) = edit_of(&out);
                acts_ref.borrow_mut().push(Act::Cell(r, j, value, edge));
            });
            if !kept {
                row = row.cell_muted(ColumnKey::index(j as u32));
            }
        }
        row.trailing_part(TrailingPart::new(0), TrailingFace::Icon(IconSpec::svg(IconId::new("wordcraft.close"))), tl!("Remove recipient"))
    });
    for event in response.events {
        match event {
            CollectionEvent::Column(ColumnEvent::Removed { key }) => {
                if let Some(j) = key.position() {
                    acts.borrow_mut().push(Act::RemoveField(j as usize));
                }
            }
            CollectionEvent::Item(r, RowEvent::Trailing { .. }) => acts.borrow_mut().push(Act::RemoveRow(r)),
            _ => {}
        }
    }
    for act in acts.into_inner() {
        match act {
            Act::Cell(r, j, value, edge) => {
                if let Some(v) = value
                    && let Some(cell) = rows.get_mut(r).and_then(|x| x.get_mut(j))
                {
                    *cell = v;
                }
                match edge {
                    // The cell's output arrives after `hold_rows` ran, so the rows of this pass are held here.
                    Some(true) => {
                        *editing = true;
                        if filter.held.is_none() {
                            filter.held = Some(visible.clone());
                        }
                    }
                    Some(false) => *editing = false,
                    None => {}
                }
            }
            Act::Field(j, value) => {
                if let Some(v) = value
                    && let Some(f) = fields.get_mut(j)
                {
                    *f = v;
                }
            }
            Act::RemoveField(i) if fields.len() > 1 && i < fields.len() => {
                fields.remove(i);
                for r in rows.iter_mut() {
                    if i < r.len() {
                        r.remove(i);
                    }
                }
                // The field that held the edit is gone.
                *editing = false;
                filter.held = None;
            }
            Act::RemoveRow(r) if r < rows.len() => {
                rows.remove(r);
                *editing = false;
                filter.held = None;
            }
            Act::RemoveField(_) | Act::RemoveRow(_) => {}
        }
    }
    if Button::new(tl!("New Entry")).show(ui).clicked() {
        // The view would hide the new, empty entry.
        filter.show_all();
        *editing = false;
        rows.push(vec![String::new(); fields.len()]);
    }
    if Button::new(tl!("Add Field")).show(ui).clicked() {
        fields.push(String::new());
        for r in rows.iter_mut() {
            r.push(String::new());
        }
        *editing = false;
        filter.held = None;
    }
    ToggleButton::new(tl!("Filter and Sort"), Bind::bound(&mut filter.open)).show(ui);
    if filter.open {
        Section::fixed(tl!("Filter and Sort")).id_salt("filter").show(ui, |ui, _| filter_section(ui, fields, filter));
    }
    if let Some(order) = &order {
        let count = order.iter().flatten().count().to_string();
        let total = rows.len().to_string();
        Text::new(crate::i18n::fmt(tl!("{count} of {total} recipients will be merged."), &[("count", &count), ("total", &total)])).show(ui);
    }
    true
}

/// Filter and Sort: the joined condition rules and three sort keys.
fn filter_section(ui: &mut egui::Ui, fields: &[String], filter: &mut RecipientFilter) {
    let names: Vec<String> = fields.iter().map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect();
    let offered = RecipientFields { names: names.clone() };
    let mut group = filter.conditions();
    let built = ConditionBuilder::new(tl!("Filter"), Bind::bound(&mut group), &offered).layout(ConditionLayout::Joined).max_rules(64).show(ui);
    if built.changed {
        filter.set_conditions(&group, fields);
    }
    if Button::new(tl!("Clear All")).show(ui).clicked() {
        filter.rules.clear();
        filter.sort = Default::default();
    }
    let mut keys = vec![Entry::new(String::new(), tl!("(none)"))];
    keys.extend(names.iter().map(|n| Entry::new(n.clone(), n.clone())));
    let directions = [Entry::new(false, tl!("Ascending")), Entry::new(true, tl!("Descending"))];
    for (i, (field, desc)) in filter.sort.iter_mut().enumerate() {
        let label = tl!(if i == 0 { "Sort by" } else { "Then by" });
        if let Some(f) = Select::new(label, Bind::given(field.clone()), &keys).id_salt(("sort", i)).show(ui).value().cloned() {
            *field = f;
        }
        let has_field = !field.is_empty();
        if let Some(d) = Select::new(tl!("Order"), Bind::given(*desc), &directions).id_salt(("dir", i)).enabled(has_field).show(ui).value().copied() {
            *desc = d;
        }
    }
}

impl StackDialog for RecipientListDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        craft::ui::dialog::dialog_id("recipientList")
    }

    fn key(&self) -> Option<&str> {
        Some("recipientList")
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let RecipientListDialog { draft, editing, pick, held } = self;
        let out = FormDialog::new(tl!("Recipient List"), Bind::bound(draft)).held(held).show(ui, |ui, _cx, d| recipient_body(ui, d, editing, pick));
        if out.accepted.is_some() { Some(DialogResult::Ok(())) } else { out.closed.map(|_| DialogResult::Cancel) }
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<()> {
        if self.held.finish().is_some() {
            return Some(());
        }
        if !self.held.is_waiting() {
            self.held.file(self.draft.clone());
        }
        None
    }

    fn json_fields(&self) -> Option<Value> {
        let message = self.held.refusal().map(|(m, _)| m.as_str().to_string()).unwrap_or_default();
        Some(json!({"fields": self.draft.fields, "rows": self.draft.rows, "filter": self.draft.filter, "message": message}))
    }

    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let bad = |what: &str| format!("`{field}` needs {what}");
        match field {
            "fields" => self.draft.fields = serde_json::from_value(value).map_err(|_| bad("a list of names"))?,
            "rows" => self.draft.rows = serde_json::from_value(value).map_err(|_| bad("a list of rows"))?,
            "find" => self.draft.filter.find = value.as_str().ok_or_else(|| bad("text"))?.to_string(),
            "onlyMerged" => self.draft.filter.only_merged = value.as_bool().ok_or_else(|| bad("true or false"))?,
            _ => return Err(format!("no field `{field}`")),
        }
        self.editing = false;
        self.draft.filter.held = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(field: &str, op: FilterOp, value: &str, or: bool) -> FilterRow {
        FilterRow { field: field.into(), op, value: value.into(), or }
    }

    /// A round trip through the builder's rules keeps field, operator, value and each later row's
    /// join; the first row's `or` is never sent; a rule on a removed field drops.
    #[test]
    fn filter_rows_and_conditions_convert_both_ways() {
        let fields = vec!["Name".to_string(), "City".to_string()];
        let (a, b) = (FilterOp::ALL[0], FilterOp::ALL[1]);
        let mut f = RecipientFilter { rules: vec![row("Name", a, "x", false), row("City", b, "y", true)], ..Default::default() };
        let before = f.rules.clone();
        let group = f.conditions();
        f.rules.clear();
        f.set_conditions(&group, &fields);
        assert_eq!(f.rules, before);

        f.rules[0].or = true;
        let p = f.params(&fields);
        assert_eq!(p["rules"][0]["join"], "and", "{p}");
        assert_eq!(p["rules"][1]["join"], "or", "{p}");

        let group = f.conditions();
        f.set_conditions(&group, &["Name".to_string()]);
        assert_eq!(f.rules.len(), 1);
        assert_eq!(f.rules[0].field, "Name");
    }
}
