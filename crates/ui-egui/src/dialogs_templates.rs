//! Templates and Add-ins (View › Templates) and the style Organizer (#377). Each button ends in
//! one command (`tools.attachTemplate`, `tools.detachTemplate`, `tools.linkStyles`,
//! `styles.copyFrom`, `styles.delete`, `styles.rename`), so agents get the same result without
//! the dialogs.

use egui::{Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};

use crate::WordApp;
use crate::dialogs::{Dialog, buttons};
use crate::file_dialogs::{AfterPick, FileDialogRequest};
use crate::theme::{Tokens, semibold};

/// The Templates and Add-ins dialog's fields.
#[derive(Clone, Debug, Serialize)]
pub struct TemplatesForm {
    pub path: String,
    /// "Automatically update document styles" (`w:linkStyles`).
    pub link: bool,
    was_path: String,
    was_link: bool,
    pub message: String,
}

impl TemplatesForm {
    pub fn read(app: &mut WordApp) -> TemplatesForm {
        // A file picked for a dialog that has since closed doesn't belong to this one.
        app.dialog_pick = None;
        let s = &app.session.doc.settings;
        let path = s.attached_template.clone().unwrap_or_default();
        TemplatesForm { was_path: path.clone(), path, link: s.link_styles, was_link: s.link_styles, message: String::new() }
    }

    /// OK: attach (updating the styles now when they follow the template), detach, or just switch
    /// the automatic update.
    fn apply(&self, app: &mut WordApp) -> Result<(), String> {
        let path = self.path.trim();
        if path.is_empty() {
            if !self.was_path.trim().is_empty() {
                app.run("tools.detachTemplate", json!({}))?;
            } else if self.link != self.was_link {
                app.run("tools.linkStyles", json!({"value": self.link}))?;
            }
            return Ok(());
        }
        if path != self.was_path || self.link != self.was_link || self.link {
            app.run("tools.attachTemplate", json!({"path": path, "linkStyles": self.link, "update": self.link}))?;
        }
        Ok(())
    }
}

/// Whether this host shows a file picker whose answer comes back to the dialog (the web's picker
/// opens what it picks instead).
fn can_pick(app: &WordApp) -> bool {
    app.services.open_async.is_none() && (app.services.file_dialog.is_some() || app.services.pick_open.is_some())
}

/// Ask for a template or document; the answer arrives in `app.dialog_pick`.
fn pick(app: &mut WordApp) {
    let _ = app.ask_file(FileDialogRequest::Open { purpose: "template".into() }, AfterPick::ForDialog);
}

/// Returns true to close.
pub fn templates(app: &mut WordApp, ui: &mut Ui, f: &mut TemplatesForm) -> bool {
    if let Some(p) = app.dialog_pick.take() {
        f.path = p;
    }
    ui.label(egui::RichText::new(tl!("Document template:")).font(semibold(12.5)));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut f.path).desired_width(300.0));
        if ui.add_enabled(can_pick(app), egui::Button::new(tl!("Attach…"))).clicked() {
            pick(app);
        }
    });
    ui.checkbox(&mut f.link, tl!("Automatically update document styles"));
    ui.add_space(6.0);
    let organizer = ui.button(tl!("Organizer…")).clicked();
    if !f.message.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(f.message.as_str()).small().color(ui.visuals().error_fg_color)).wrap());
    }
    let (ok, cancel) = buttons(ui, tl!("OK"));
    if organizer {
        let back = TemplatesForm { message: String::new(), ..f.clone() };
        app.dialog = Some(Dialog::Organizer { form: Box::new(OrganizerForm::read(app, Some(back))) });
        return true;
    }
    if ok && let Err(e) = f.apply(app) {
        f.message = e;
        return false;
    }
    ok || cancel
}

/// The Organizer's state: this document's styles on the left (read live), another document's or
/// template's on the right.
#[derive(Clone, Debug, Serialize)]
pub struct OrganizerForm {
    /// The other file (empty: none open).
    pub file: String,
    /// Its styles: (name, type).
    pub other: Vec<(String, String)>,
    /// Selected style id in this document.
    pub left: String,
    /// Selected style name in the other file.
    pub right: String,
    /// The new name being typed (Rename…).
    pub rename: Option<String>,
    pub message: String,
    /// Templates and Add-ins, to go back to on Close.
    back: Option<Box<TemplatesForm>>,
}

impl OrganizerForm {
    /// Opens with the attached template (or the template typed in Templates and Add-ins) on the right.
    pub fn read(app: &mut WordApp, back: Option<TemplatesForm>) -> OrganizerForm {
        app.dialog_pick = None;
        let file =
            back.as_ref().map(|b| b.path.trim().to_string()).or_else(|| app.session.doc.settings.attached_template.clone()).unwrap_or_default();
        let mut f = OrganizerForm {
            file: String::new(),
            other: Vec::new(),
            left: String::new(),
            right: String::new(),
            rename: None,
            message: String::new(),
            back: back.map(Box::new),
        };
        if !file.is_empty() {
            f.load(app, &file);
        }
        f
    }

    fn load(&mut self, app: &mut WordApp, path: &str) {
        match app.session.run("styles.organizer", &json!({"path": path})) {
            Ok(v) => {
                let list = v.get("file").and_then(|f| f.get("styles")).and_then(Value::as_array).cloned().unwrap_or_default();
                let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                self.other = list.iter().map(|x| (s(x, "name"), s(x, "type"))).collect();
                self.file = path.to_string();
                self.right.clear();
                self.message.clear();
            }
            Err(e) => self.message = e.to_string(),
        }
    }
}

fn glyph(ty: &str) -> &'static str {
    match ty {
        "character" => "a",
        "table" => "⊞",
        "numbering" => "≡",
        _ => "¶",
    }
}

/// Returns true to close.
pub fn organizer(app: &mut WordApp, ui: &mut Ui, f: &mut OrganizerForm) -> bool {
    if let Some(p) = app.dialog_pick.take() {
        f.load(app, &p);
    }
    let mine = wordcraft_engine::cmd::templates::style_list(&app.session.doc);
    let mine = mine.as_array().cloned().unwrap_or_default();
    let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    if !mine.iter().any(|x| s(x, "id") == f.left) {
        f.left.clear();
    }
    let t = Tokens::get(ui.ctx());
    let list_frame = |ui: &mut Ui, salt: &str, add: &mut dyn FnMut(&mut Ui)| {
        egui::Frame::NONE.stroke(egui::Stroke::new(1.0, t.border)).inner_margin(4).show(ui, |ui| {
            ui.set_width(220.0);
            ui.set_height(240.0);
            egui::ScrollArea::vertical().id_salt(salt).max_height(240.0).auto_shrink([false, false]).show(ui, |ui| add(ui));
        });
    };
    let mut copy = false;
    let mut delete = false;
    let mut rename = false;
    let mut browse = false;
    let left_builtin = mine.iter().find(|x| s(x, "id") == f.left).is_none_or(|x| x.get("builtIn").and_then(Value::as_bool).unwrap_or(true));
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(tl!("Document")).font(semibold(12.5)));
            list_frame(ui, "organizer_mine", &mut |ui| {
                for st in &mine {
                    let id = s(st, "id");
                    if ui.selectable_label(f.left == id, format!("{}  {}", glyph(&s(st, "type")), s(st, "name"))).clicked() {
                        f.left = id;
                        f.rename = None;
                    }
                }
            });
        });
        ui.vertical(|ui| {
            ui.add_space(60.0);
            let w = vec2(96.0, 0.0);
            copy = ui.add_enabled(!f.right.is_empty(), egui::Button::new(format!("◀ {}", tl!("Copy"))).min_size(w)).clicked();
            delete = ui.add_enabled(!left_builtin, egui::Button::new(tl!("Delete")).min_size(w)).clicked();
            rename = ui.add_enabled(!left_builtin, egui::Button::new(tl!("Rename…")).min_size(w)).clicked();
        });
        ui.vertical(|ui| {
            let name = std::path::Path::new(&f.file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "—".into());
            ui.label(egui::RichText::new(name).font(semibold(12.5))).on_hover_text(f.file.as_str());
            list_frame(ui, "organizer_other", &mut |ui| {
                for (n, ty) in &f.other {
                    if ui.selectable_label(f.right == *n, format!("{}  {n}", glyph(ty))).clicked() {
                        f.right = n.clone();
                    }
                }
            });
            browse = ui.add_enabled(can_pick(app), egui::Button::new(tl!("Browse…"))).clicked();
        });
    });
    if copy {
        let r = app.run("styles.copyFrom", json!({"path": f.file, "styles": [f.right]}));
        f.message = r.err().unwrap_or_default();
    }
    if delete {
        let r = app.run("styles.delete", json!({"style": f.left}));
        f.message = r.err().unwrap_or_default();
        f.left.clear();
    }
    if rename {
        f.rename = mine.iter().find(|x| s(x, "id") == f.left).map(|x| s(x, "name"));
    }
    if browse {
        pick(app);
    }
    let mut renamed = false;
    if let Some(new) = f.rename.as_mut() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(tl!("Name:"));
            let r = ui.add(egui::TextEdit::singleline(new).desired_width(220.0));
            renamed = ui.button(tl!("OK")).clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        });
    }
    if renamed && let Some(new) = f.rename.take() {
        let r = app.run("styles.rename", json!({"style": f.left, "name": new}));
        f.message = r.err().unwrap_or_default();
    }
    if !f.message.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(f.message.as_str()).small().color(ui.visuals().error_fg_color)).wrap());
    }
    // Escape while renaming only stops renaming.
    if f.rename.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        f.rename = None;
        return false;
    }
    // Close (Escape too), in a row of its own so the dialog stays as tall as its lists.
    ui.add_space(8.0);
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close |= ui.add(egui::Button::new(egui::RichText::new(tl!("Close")).color(egui::Color32::WHITE)).fill(crate::theme::APP_COLOR)).clicked();
        });
    });
    if close {
        if let Some(back) = f.back.take() {
            app.dialog = Some(Dialog::Templates { form: back });
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use wordcraft_engine::Session;

    /// The ribbon button opens Templates and Add-ins; OK attaches the typed template; the
    /// Organizer copies a style from it into the document and Close goes back.
    #[test]
    fn templates_dialog_attaches_and_organizer_copies() {
        let dir = std::env::temp_dir().join(format!("wordcraft-ui-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("House.dotx");
        let mut t = wordcraft_doc::Document::new();
        t.styles.upsert(wordcraft_doc::styles::Style {
            id: "Memo".into(),
            name: "Memo".into(),
            kind: wordcraft_doc::styles::StyleKind::Paragraph,
            ..Default::default()
        });
        wordcraft_engine::io::save_path(&path, &t).unwrap();
        let p = path.to_string_lossy().to_string();

        let mut a = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        assert_eq!(a.run("tools.templates", json!({})).unwrap(), json!({"pending": "templates"}));
        let Some(Dialog::Templates { form }) = &mut a.dialog else { panic!("dialog") };
        form.path = p.clone();
        let form = form.clone();
        form.apply(&mut a).unwrap();
        assert_eq!(a.session.doc.settings.attached_template.as_deref(), Some(p.as_str()));

        let mut org = OrganizerForm::read(&mut a, None);
        assert!(org.other.iter().any(|(n, _)| n == "Memo"), "{:?}", org.other);
        org.right = "Memo".into();
        let r = a.run("styles.copyFrom", json!({"path": org.file, "styles": [org.right]}));
        assert!(r.is_ok(), "{r:?}");
        assert!(a.session.doc.styles.get("Memo").is_some());
        // Agents and scripts get the state, never the dialog.
        a.dialog = None;
        assert_eq!(a.execute("tools.templates", json!({})).unwrap()["template"], json!(p));
        assert!(a.dialog.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
