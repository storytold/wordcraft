use super::chrome::buttons;
use super::*;

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
                    ui.add_sized([CELL_W, ROW_H - 4.0], egui::TextEdit::singleline(f).font(egui::TextStyle::Button).hint_text(tl!("Field name")))
                        .accessible_name(format!("{} {}", tl!("Field name"), i + 1));
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
                            let field_name = fields.get(j).map_or("", String::as_str);
                            let cell = ui
                                .add_sized([CELL_W, ROW_H - 4.0], egui::TextEdit::singleline(c).id_salt(("recipient_cell", r, j)))
                                .accessible_name(crate::i18n::fmt(tl!("{field}, record {n}"), &[("field", field_name), ("n", &(r + 1).to_string())]));
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
        craft_ui::ComboBox::from_id_salt(id).width(130.0).selected_text(shown).show_ui(ui, |ui| {
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
                craft_ui::ComboBox::from_id_salt(("rf_join", i)).width(60.0).selected_text(tl!(if r.or { "Or" } else { "And" })).show_ui(ui, |ui| {
                    ui.selectable_value(&mut r.or, false, tl!("And"));
                    ui.selectable_value(&mut r.or, true, tl!("Or"));
                });
            }
            field_combo(ui, ("rf_field", i), &mut r.field, false);
            craft_ui::ComboBox::from_id_salt(("rf_op", i)).width(150.0).selected_text(tl!(r.op.label())).show_ui(ui, |ui| {
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
                craft_ui::ComboBox::from_id_salt(("rs_dir", i))
                    .width(110.0)
                    .selected_text(tl!(if *desc { "Descending" } else { "Ascending" }))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(desc, false, tl!("Ascending"));
                        ui.selectable_value(desc, true, tl!("Descending"));
                    });
            });
            ui.end_row();
        }
    });
}
