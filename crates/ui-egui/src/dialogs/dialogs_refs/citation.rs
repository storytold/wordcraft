use super::shell::Next;
use super::{Serialize, Ui, Value, WordApp, json, vec2};

// ---------------------------------------------------------------------------------------------
// Mark Citation

/// Mark Citation's categories, as Word lists them (8–16 have no name).
const CATEGORIES: [&str; 7] = ["Cases", "Statutes", "Other Authorities", "Rules", "Treatises", "Regulations", "Constitutional Provisions"];

/// The Mark Citation dialog's fields: the long citation (as the table lists it, from the
/// selection), the short one (what Mark All also looks for) and the category.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationForm {
    pub long: String,
    pub short: String,
    pub category: u64,
    pub message: String,
}

impl CitationForm {
    /// The selection as both the long and the short citation, in Cases, as Word's dialog opens.
    pub fn read(app: &WordApp) -> CitationForm {
        let long = app.session.selected_text().split_whitespace().collect::<Vec<_>>().join(" ");
        CitationForm { short: long.clone(), long, category: 1, message: String::new() }
    }

    /// `references.markCitation` parameters (`all` for Mark All).
    pub fn params(&self, all: bool) -> Value {
        json!({"long": self.long.trim(), "short": self.short.trim(), "category": self.category, "all": all})
    }
}

/// Draws the long citation, category and short citation, with Mark and Mark All running
/// `references.markCitation`.
pub(super) fn citation_ui(app: &mut WordApp, ui: &mut Ui, f: &mut CitationForm) -> Next {
    egui::Grid::new("ta").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
        let long_label = ui.label(tl!("Long citation:"));
        ui.add(egui::TextEdit::multiline(&mut f.long).desired_rows(3).desired_width(320.0)).labelled_by(long_label.id);
        ui.end_row();
        let category_label = ui.label(tl!("Category:"));
        let name = |c: u64| CATEGORIES.get(c.saturating_sub(1) as usize).map_or_else(|| c.to_string(), |n| tl!(n).to_string());
        let combo = craft_ui::ComboBox::from_id_salt("ta_cat").selected_text(name(f.category)).width(200.0).show_ui(ui, |ui| {
            for c in 1..=16 {
                ui.selectable_value(&mut f.category, c, name(c));
            }
        });
        combo.response.labelled_by(category_label.id);
        ui.end_row();
        let short_label = ui.label(tl!("Short citation:"));
        ui.add(egui::TextEdit::singleline(&mut f.short).desired_width(320.0)).labelled_by(short_label.id);
        ui.end_row();
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
            let can = !f.long.trim().is_empty();
            if ui.add_enabled(can, egui::Button::new(tl!("Mark All"))).clicked() {
                f.message = match app.run("references.markCitation", f.params(true)) {
                    Ok(v) => {
                        crate::i18n::fmt(tl!("{n} citations marked."), &[("n", &v.get("marked").and_then(Value::as_u64).unwrap_or(0).to_string())])
                    }
                    Err(e) => e,
                };
            }
            if ui.add_enabled(can, crate::widgets::primary_button(ui, tl!("Mark"))).clicked() {
                f.message = match app.run("references.markCitation", f.params(false)) {
                    Ok(_) => tl!("Citation marked.").to_string(),
                    Err(e) => e,
                };
            }
        });
    });
    if close { Next::Close } else { Next::Stay }
}
