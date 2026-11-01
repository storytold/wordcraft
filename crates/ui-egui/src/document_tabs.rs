//! Optional desktop tab strip built on WordCraft's existing multi-document engine.
//! The engine, not this widget, owns undo, recovery, selections and save prompts.
use std::path::Path;

use egui::{RichText, Sense};
use serde_json::json;

use crate::{WordApp, documents, theme::Tokens};

impl WordApp {
    /// Reopen only saved files; unsaved documents remain protected by engine recovery.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn restore_document_tabs(&mut self) -> bool {
        if !self.ui.document_tabs || !self.ui.restore_document_tabs { return false; }
        let paths: Vec<String> = self.ui.open_document_paths.iter().take(24)
            .filter(|p| p.len() <= 4096 && Path::new(p.as_str()).is_file())
            .cloned().collect();
        if paths.is_empty() { return false; }
        let preferred = self.ui.preferred_document_path.clone();
        let mut opened = 0;
        for path in paths {
            match self.run("file.open", json!({"path": path})) {
                Ok(result) if result.get("pending").is_none() => opened += 1,
                Ok(_) => { self.dialog = None; },
                Err(e) => log::warn!("Cannot restore document tab: {e}"),
            }
        }
        if opened == 0 { return false; }
        if let Some(preferred) = preferred {
            if let Some(doc) = self.session.documents().into_iter().find(|d| d.path.as_deref() == Some(Path::new(&preferred))) {
                let _ = self.show_document(doc.id);
            }
        }
        self.ui.backstage = false;
        true
    }

    #[cfg(target_arch = "wasm32")]
    pub fn restore_document_tabs(&mut self) -> bool { false }
}

/// The tab order is UI-only. Closing or opening a document reconciles it with the engine.
fn ordered(app: &mut WordApp) -> Vec<wordcraft_engine::DocInfo> {
    let docs = app.session.documents();
    app.document_tab_order.retain(|id| docs.iter().any(|d| d.id == *id));
    for d in &docs {
        if !app.document_tab_order.contains(&d.id) { app.document_tab_order.push(d.id); }
    }
    app.document_tab_order.iter().filter_map(|id| docs.iter().find(|d| d.id == *id).cloned()).collect()
}

pub(crate) fn cycle(app: &mut WordApp, direction: i32) {
    let docs = ordered(app);
    if docs.len() < 2 { return; }
    let current = docs.iter().position(|d| d.active).unwrap_or(0);
    let next = if direction > 0 { (current + 1) % docs.len() } else { (current + docs.len() - 1) % docs.len() };
    if let Err(e) = app.show_document(docs[next].id) { app.status(e); }
}

pub(crate) fn bar(app: &mut WordApp, ui: &mut egui::Ui) {
    if !app.ui.document_tabs || app.ui.backstage || app.session.view.focus_mode { return; }
    let docs = ordered(app);
    let t = Tokens::get(ui.ctx());
    enum Action { Switch(u64), Close(u64), Move(u64, u64), New }
    let mut action = None;
    egui::TopBottomPanel::top("document_tabs").exact_height(35.0)
        .frame(egui::Frame::NONE.fill(t.title_bar).inner_margin(egui::Margin::symmetric(8, 3)))
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                egui::ScrollArea::horizontal().id_salt("document_tabs_scroll").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for doc in &docs {
                            let title = documents::display_title(doc);
                            let short = if title.chars().count() > 22 {
                                format!("{}…", title.chars().take(21).collect::<String>())
                            } else { title.clone() };
                            let label = if doc.dirty { format!("● {short}") } else { short };
                            let response = ui.add(egui::Button::selectable(doc.active, RichText::new(label))
                                .min_size(egui::vec2(112.0, 26.0)).sense(Sense::click_and_drag())).on_hover_text(title);
                            if response.drag_started() { app.dragging_document_tab = Some(doc.id); }
                            if response.hovered() && ui.input(|i| i.pointer.any_released()) {
                                if let Some(from) = app.dragging_document_tab.filter(|from| *from != doc.id) {
                                    action = Some(Action::Move(from, doc.id));
                                }
                            } else if response.clicked() { action = Some(Action::Switch(doc.id)); }
                            if ui.small_button("×").on_hover_text(tl!("Close document")).clicked() {
                                action = Some(Action::Close(doc.id));
                            }
                        }
                    });
                });
                if ui.button("+").on_hover_text(tl!("New document")).clicked() { action = Some(Action::New); }
            });
        });
    if ui.input(|i| i.pointer.any_released()) { app.dragging_document_tab = None; }
    match action {
        Some(Action::Switch(id)) => { if let Err(e) = app.show_document(id) { app.status(e); } }
        Some(Action::Close(id)) => {
            if let Err(e) = app.show_document(id).and_then(|_| app.run("file.close", json!({})).map(|_| ())) { app.status(e); }
        }
        Some(Action::Move(from, to)) => {
            let order = &mut app.document_tab_order;
            if let (Some(a), Some(b)) = (order.iter().position(|x| *x == from), order.iter().position(|x| *x == to)) {
                let id = order.remove(a);
                order.insert(b, id);
            }
        }
        Some(Action::New) => { if let Err(e) = app.run("file.new", json!({})) { app.status(e); } }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use wordcraft_doc::{Document, StoryRef};
    use wordcraft_engine::Session;

    #[test]
    fn optional_tabs_preserve_each_documents_undo_and_text() {
        let mut app = WordApp::new(Session::new(Document::new()), Services::default());
        assert!(!app.ui.document_tabs);
        app.run("ui.documentTabs", json!({"value": true})).unwrap();
        app.run("text.insert", json!({"text": "First"})).unwrap();
        let first = app.session.document_id();
        app.run("file.new", json!({})).unwrap();
        app.run("text.insert", json!({"text": "Second"})).unwrap();
        assert_eq!(app.session.documents().len(), 2);
        app.show_document(first).unwrap();
        assert_eq!(app.session.doc.plain_text(StoryRef::Body), "First");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc.plain_text(StoryRef::Body), "");
        assert!(app.run("ui.documentTabs", json!({"value": false})).is_err());
    }
}
