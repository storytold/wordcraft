//! Review the warnings produced when a worksheet opens.

use std::collections::{HashMap, HashSet};

use craft::ui::buttons::Button;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::sections::{DialogRole, FooterButton};
use serde::Deserialize;
use serde_json::{Value, json};
use wordcraft_doc::Pos;

use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

#[derive(Deserialize)]
struct Entry {
    kind: String,
    id: Option<u64>,
    #[serde(rename = "upstreamId")]
    upstream_id: Option<u64>,
    message: Option<String>,
    saved: Option<f64>,
    displayed: Option<f64>,
    source: Option<String>,
    pos: Pos,
}

struct ImportReview {
    document: u64,
    entries: Vec<Entry>,
    downstream: HashMap<u64, Vec<usize>>,
    expanded: HashSet<u64>,
}

pub(crate) fn open(app: &mut WordApp, review: &Value) {
    if let Ok(entries) = serde_json::from_value::<Vec<Entry>>(review.clone()) {
        let mut downstream = HashMap::<u64, Vec<usize>>::new();
        for (index, entry) in entries.iter().enumerate() {
            if let Some(root) = entry.upstream_id {
                downstream.entry(root).or_default().push(index);
            }
        }
        let dialog = ImportReview { document: app.session.document_id(), entries, downstream, expanded: HashSet::new() };
        open_with(app, None, dialog, DialogHooks { prepare: None, settle });
    }
}

impl StackDialog for ImportReview {
    type Result = (u64, Pos);

    fn id(&self) -> DialogId {
        dialog_id("worksheetImportReview")
    }

    fn key(&self) -> Option<&str> {
        Some("worksheetImportReview")
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let mut target = None;
        let shown = Dialog::new(tl!("Worksheet Import Review").to_string())
            .buttons([FooterButton::new(tl!("Close").to_string(), DialogRole::Accept)])
            .show(ui, |ui, _| {
                if self.entries.is_empty() {
                    Text::new(tl!("No import warnings or calculation errors were found.")).show(ui);
                } else {
                    Text::new(tl!("Choose an entry to go to its location in the document.")).show(ui);
                    for (index, entry) in self.entries.iter().enumerate().filter(|(_, entry)| entry.upstream_id.is_none()) {
                        if entry.show(ui, index) {
                            target = Some(entry.pos.clone());
                        }
                        if let Some(id) = entry.id
                            && let Some(dependents) = self.downstream.get(&id)
                        {
                            let expanded = self.expanded.contains(&id);
                            let label = if expanded { tl!("Hide dependent calculations") } else { tl!("Show dependent calculations") };
                            if Button::new(format!("{label} ({})", dependents.len())).id_salt(("dependents", id)).show(ui).clicked() {
                                if expanded {
                                    self.expanded.remove(&id);
                                } else {
                                    self.expanded.insert(id);
                                }
                            }
                            if self.expanded.contains(&id) {
                                for &child in dependents {
                                    if let Some(entry) = self.entries.get(child)
                                        && entry.show(ui, child)
                                    {
                                        target = Some(entry.pos.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            });
        target.map(|pos| DialogResult::Ok((self.document, pos))).or_else(|| shown.output.closed.map(|_| DialogResult::Cancel))
    }
}

impl Entry {
    fn show(&self, ui: &mut egui::Ui, index: usize) -> bool {
        let label = match self.kind.as_str() {
            "calculationError" => tl!("Calculation error"),
            "calculationUnverified" => tl!("Unverified calculation"),
            "savedSnapshot" => tl!("Saved snapshot"),
            "savedResult" => tl!("Saved result comparison"),
            _ => tl!("Import warning"),
        };
        let detail = match (self.saved, self.displayed) {
            (Some(saved), Some(displayed)) => format!("{} {saved} / {} {displayed}", tl!("Saved"), tl!("Displayed")),
            _ => self.message.clone().unwrap_or_default(),
        };
        let source = self.source.as_deref().map_or(String::new(), |source| format!(" {source}"));
        Button::new(format!("{label}{source}: {detail}")).id_salt(index).show(ui).clicked()
    }
}

fn settle(app: &mut WordApp, ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(result) = app.dialog_host.take_by_id::<ImportReview>(id) else { return Settle::Keep };
    if let DialogResult::Ok((document, pos)) = result
        && app.session.document_id() == document
        && app.run("caret.set", json!({"pos": pos})).is_ok()
    {
        app.canvas.nav_to_caret = true;
        app.canvas.scroll_to_caret = false;
        app.canvas.focus_page(ctx);
    }
    Settle::Done
}
