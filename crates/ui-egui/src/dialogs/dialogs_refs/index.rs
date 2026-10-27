use super::common::{LEADERS, choice, heading, leader_char, sample};
use super::shell::{Next, go};
use super::*;

// ---------------------------------------------------------------------------------------------
// Index and Mark Index Entry

/// The Index dialog's fields.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
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
}

/// Draws the index preview and type, columns, right-alignment, tab-leader and heading controls,
/// jumps to Mark Entry, and runs `references.index` on OK.
pub(super) fn index_ui(app: &mut WordApp, ui: &mut Ui, f: &mut IndexForm) -> Next {
    let page = |p: &str| p.to_string();
    let lines: Vec<(usize, String, String)> = {
        let mut v = Vec::new();
        if f.headings {
            v.push((0, "A".to_string(), String::new()));
        }
        if f.kind == "runIn" {
            v.push((0, format!("{}: {}, 4; {}", tl!("Atmosphere"), tl!("Ionosphere"), tl!("Mesosphere")), page("3")));
        } else {
            v.push((0, tl!("Atmosphere").to_string(), page("1")));
            v.push((1, tl!("Ionosphere").to_string(), page("3")));
            v.push((1, tl!("Mesosphere").to_string(), page("4")));
        }
        v
    };
    sample(ui, &lines, f.right_align, leader_char(&f.leader));
    ui.add_space(6.0);
    egui::Grid::new("idx").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        ui.label(tl!("Type:"));
        ui.horizontal(|ui| {
            ui.radio_value(&mut f.kind, "indented".to_string(), tl!("Indented"));
            ui.radio_value(&mut f.kind, "runIn".to_string(), tl!("Run-in"));
        });
        ui.end_row();
        let columns_label = ui.label(tl!("Columns:"));
        ui.add(egui::DragValue::new(&mut f.columns).range(1..=4)).labelled_by(columns_label.id);
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut f.right_align, tl!("Page numbers at the right margin"));
        ui.end_row();
        let leader_label = ui.label(tl!("Tab leader:"));
        ui.add_enabled_ui(f.right_align, |ui| {
            let opts: Vec<(String, &str)> = LEADERS.iter().map(|(v, l)| (v.to_string(), *l)).collect();
            choice(ui, "idx_leader", &mut f.leader, &opts, &leader_label);
        });
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut f.headings, tl!("Letter headings"));
        ui.end_row();
    });
    ui.add_space(4.0);
    if ui.button(tl!("Mark Entry…")).clicked() {
        return go(RefsDialog::MarkEntry(MarkForm::read(app)));
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if ok && let Err(e) = app.run("references.index", f.params()) {
        app.status(e);
    }
    if ok || cancel { Next::Close } else { Next::Stay }
}

/// The Mark Index Entry dialog's fields.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
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

/// Draws the index-entry fields and page, cross-reference or bookmark-range options, with Mark
/// and Mark All running `references.markEntry`.
pub(super) fn mark_ui(app: &mut WordApp, ui: &mut Ui, f: &mut MarkForm) -> Next {
    heading(ui, "Index");
    egui::Grid::new("xe").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let entry_label = ui.label(tl!("Main entry:"));
        ui.add(egui::TextEdit::singleline(&mut f.entry).desired_width(220.0)).labelled_by(entry_label.id);
        ui.end_row();
        let subentry_label = ui.label(tl!("Subentry:"));
        ui.add(egui::TextEdit::singleline(&mut f.subentry).desired_width(220.0)).labelled_by(subentry_label.id);
        ui.end_row();
    });
    heading(ui, "Options");
    ui.horizontal(|ui| {
        let see_label = ui.radio_value(&mut f.option, "see".to_string(), tl!("Cross-reference:"));
        ui.add_enabled(f.option == "see", egui::TextEdit::singleline(&mut f.see).desired_width(160.0)).labelled_by(see_label.id);
    });
    ui.radio_value(&mut f.option, "page".to_string(), tl!("This page"));
    ui.horizontal(|ui| {
        let bookmark_label =
            ui.add_enabled_ui(!f.bookmarks.is_empty(), |ui| ui.radio_value(&mut f.option, "range".to_string(), tl!("Pages of bookmark:"))).inner;
        ui.add_enabled_ui(f.option == "range", |ui| {
            let opts: Vec<(String, &str)> = f.bookmarks.iter().map(|b| (b.clone(), b.as_str())).collect();
            let combo = craft_ui::ComboBox::from_id_salt("xe_bm").selected_text(f.bookmark.clone()).width(140.0).show_ui(ui, |ui| {
                for (v, l) in &opts {
                    ui.selectable_value(&mut f.bookmark, v.clone(), *l);
                }
            });
            combo.response.labelled_by(bookmark_label.id);
        });
    });
    heading(ui, "Page number style");
    ui.horizontal(|ui| {
        ui.checkbox(&mut f.bold, tl!("Bold"));
        ui.checkbox(&mut f.italic, tl!("Italic"));
    });
    if !f.message.is_empty() {
        ui.label(egui::RichText::new(&f.message).small());
    }
    let mut close = false;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Close")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            let can = !f.entry.trim().is_empty();
            if ui.add_enabled(can, egui::Button::new(tl!("Mark All"))).clicked() {
                f.message = match app.run("references.markEntry", f.params(true)) {
                    Ok(v) => {
                        crate::i18n::fmt(tl!("{n} entries marked."), &[("n", &v.get("marked").and_then(Value::as_u64).unwrap_or(0).to_string())])
                    }
                    Err(e) => e,
                };
            }
            if ui.add_enabled(can, crate::widgets::primary_button(ui, tl!("Mark"))).clicked() {
                f.message = match app.run("references.markEntry", f.params(false)) {
                    Ok(_) => tl!("Entry marked.").to_string(),
                    Err(e) => e,
                };
            }
        });
    });
    if close { Next::Close } else { Next::Stay }
}
