//! Developer › Code: the document's VBA macros, run by a macro add-in (craft-vba). Macros (list,
//! run, edit, create), Visual Basic (the code modules in an editor) and Macro Security. Everything
//! goes through the `developer.*` commands; the add-in owns the language, WordCraft the interface.
//! WordCraft's own recorded command macros stay in View › Macros.

use egui::{RichText, Ui};
use serde::Serialize;
use serde_json::{Value, json};

use crate::WordApp;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "form", rename_all = "camelCase")]
pub enum DevDialog {
    Macros(MacrosForm),
    VisualBasic(Editor),
    Security,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MacrosForm {
    pub list: Vec<Value>,
    pub selected: usize,
    pub new_name: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Editor {
    /// (name, kind, source)
    pub modules: Vec<(String, String, String)>,
    pub current: usize,
    pub text: String,
    pub dirty: bool,
    pub error: Option<String>,
    pub new_module: String,
}

impl DevDialog {
    pub fn name(&self) -> &'static str {
        match self {
            DevDialog::Macros(_) => "vbaMacros",
            DevDialog::VisualBasic(_) => "visualBasic",
            DevDialog::Security => "macroSecurity",
        }
    }
    pub fn title(&self) -> &'static str {
        match self {
            DevDialog::Macros(_) => "Macros",
            DevDialog::VisualBasic(_) => "Visual Basic",
            DevDialog::Security => "Macro Security",
        }
    }
}

/// Whether the Code group shows: an add-in that runs macros is loaded.
pub fn visible() -> bool {
    !wordcraft_plugin::macro_plugins().is_empty()
}

fn language() -> String {
    wordcraft_plugin::macro_plugins().into_iter().next().map(|(_, l)| l).unwrap_or_else(|| "VBA".into())
}

pub fn open(name: &str, app: &mut WordApp) -> Option<DevDialog> {
    match name {
        "vbaMacros" => {
            let mut f = MacrosForm::default();
            match app.session.run("developer.macros", &json!({})) {
                Ok(r) => f.list = r.get("macros").and_then(Value::as_array).cloned().unwrap_or_default(),
                Err(e) => f.error = Some(e.to_string()),
            }
            Some(DevDialog::Macros(f))
        }
        "visualBasic" => Some(DevDialog::VisualBasic(editor(app, None))),
        "macroSecurity" => Some(DevDialog::Security),
        _ => None,
    }
}

fn editor(app: &mut WordApp, module: Option<&str>) -> Editor {
    let mut e = Editor::default();
    match app.session.run("developer.modules", &json!({})) {
        Ok(r) => {
            e.modules = r
                .get("modules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|m| {
                    let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    (s("name"), s("kind"), s("source"))
                })
                .collect();
        }
        Err(err) => e.error = Some(err.to_string()),
    }
    if let Some(m) = module {
        e.current = e.modules.iter().position(|(n, _, _)| n == m).unwrap_or(0);
    }
    e.text = e.modules.get(e.current).map(|(_, _, s)| s.clone()).unwrap_or_default();
    e
}

/// Draws the dialog; true when it closes.
pub fn body(app: &mut WordApp, ui: &mut Ui, d: &mut DevDialog) -> bool {
    match d {
        DevDialog::Macros(f) => {
            let (close, edit) = macros(app, ui, f);
            if let Some(module) = edit {
                *d = DevDialog::VisualBasic(editor(app, Some(&module)));
                return false;
            }
            close
        }
        DevDialog::VisualBasic(e) => visual_basic(app, ui, e),
        DevDialog::Security => security(app, ui),
    }
}

fn error(ui: &mut Ui, e: &Option<String>) {
    if let Some(e) = e {
        ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
    }
}

/// The Macros dialog: (close, the module to open in the editor).
fn macros(app: &mut WordApp, ui: &mut Ui, f: &mut MacrosForm) -> (bool, Option<String>) {
    let enabled = wordcraft_engine::cmd::developer_vba::macros_allowed(&app.session);
    let mut close = false;
    let mut edit = None;
    if !enabled {
        ui.label(RichText::new(tl!("Macros are disabled for this document. Enable Content in the message bar, or change Macro Security.")).weak());
    }
    ui.horizontal(|ui| {
        ui.label(tl!("Macro name:"));
        ui.add(egui::TextEdit::singleline(&mut f.new_name).desired_width(260.0));
    });
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_size(egui::vec2(420.0, 160.0));
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            if f.list.is_empty() {
                ui.weak(tl!("This document has no macros yet: type a name and Create."));
            }
            for (i, m) in f.list.iter().enumerate() {
                let name = m.get("name").and_then(Value::as_str).unwrap_or("");
                if ui.selectable_label(i == f.selected, name).clicked() {
                    f.selected = i;
                    f.new_name = name.to_string();
                }
            }
        });
    });
    let chosen = f.list.get(f.selected).and_then(|m| m.get("name")).and_then(Value::as_str).map(str::to_string);
    ui.horizontal(|ui| {
        if ui.add_enabled(enabled && chosen.is_some(), egui::Button::new(tl!("Run"))).clicked()
            && let Some(name) = &chosen
        {
            match app.run("developer.runMacro", json!({"name": name})) {
                Ok(_) => close = true,
                Err(e) => f.error = Some(e),
            }
        }
        if ui.add_enabled(chosen.is_some(), egui::Button::new(tl!("Edit"))).clicked()
            && let Some(m) = f.list.get(f.selected)
        {
            edit = m.get("module").and_then(Value::as_str).map(str::to_string);
        }
        let name = f.new_name.trim().to_string();
        let valid =
            !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.chars().next().is_some_and(|c| c.is_ascii_digit());
        let exists = f.list.iter().any(|m| m.get("procedure").and_then(Value::as_str) == Some(name.as_str()));
        if ui.add_enabled(valid && !exists, egui::Button::new(tl!("Create"))).clicked() {
            edit = Some(create_macro(app, &name, &mut f.error));
        }
        if ui.button(tl!("Close")).clicked() {
            close = true;
        }
    });
    error(ui, &f.error);
    (close, edit)
}

/// Adds `Sub name()` to Module1 (made when missing); the module's name.
fn create_macro(app: &mut WordApp, name: &str, err: &mut Option<String>) -> String {
    let module = "Module1".to_string();
    let existing = app
        .session
        .run("developer.modules", &json!({}))
        .ok()
        .and_then(|r| r.get("modules").and_then(Value::as_array).cloned())
        .and_then(|m| m.into_iter().find(|x| x.get("name").and_then(Value::as_str) == Some("Module1")))
        .and_then(|m| m.get("source").and_then(Value::as_str).map(str::to_string))
        .map(|t| format!("{}\r\n\r\n", t.trim_end()))
        .unwrap_or_default();
    let source = format!("{existing}Sub {name}()\r\n\r\nEnd Sub\r\n");
    if let Err(e) = app.run("developer.saveModule", json!({"name": module, "source": source})) {
        *err = Some(e);
    }
    module
}

fn visual_basic(app: &mut WordApp, ui: &mut Ui, e: &mut Editor) -> bool {
    let mut close = false;
    ui.set_min_width(760.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(e.dirty, egui::Button::new(tl!("Save")))
            .on_hover_text(tl!("Saves the module into the document (save the document as .docm to keep it)"))
            .clicked()
        {
            let name = e.modules.get(e.current).map(|(n, _, _)| n.clone()).unwrap_or_else(|| "Module1".into());
            match app.run("developer.saveModule", json!({"name": name, "source": e.text})) {
                Ok(_) => {
                    if let Some(m) = e.modules.get_mut(e.current) {
                        m.2 = e.text.clone();
                    }
                    e.dirty = false;
                    e.error = None;
                }
                Err(err) => e.error = Some(err),
            }
        }
        ui.separator();
        ui.add(egui::TextEdit::singleline(&mut e.new_module).hint_text("Module2").desired_width(110.0));
        let valid = !e.new_module.trim().is_empty() && !e.modules.iter().any(|(n, _, _)| n.eq_ignore_ascii_case(e.new_module.trim()));
        if ui.add_enabled(valid, egui::Button::new(tl!("Insert Module"))).clicked() {
            let name = e.new_module.trim().to_string();
            let source = format!("Attribute VB_Name = \"{name}\"\r\n");
            match app.run("developer.saveModule", json!({"name": name, "source": source})) {
                Ok(_) => {
                    e.modules.push((name, "standard".into(), source.clone()));
                    e.current = e.modules.len() - 1;
                    e.text = source;
                    e.dirty = false;
                    e.new_module.clear();
                }
                Err(err) => e.error = Some(err),
            }
        }
        let standard = e.modules.get(e.current).is_some_and(|(_, k, _)| k == "standard");
        if ui.add_enabled(standard, egui::Button::new(tl!("Remove Module"))).clicked()
            && let Some((name, _, _)) = e.modules.get(e.current).cloned()
        {
            match app.run("developer.deleteModule", json!({"name": name})) {
                Ok(_) => {
                    e.modules.remove(e.current);
                    e.current = 0;
                    e.text = e.modules.first().map(|(_, _, s)| s.clone()).unwrap_or_default();
                    e.dirty = false;
                }
                Err(err) => e.error = Some(err),
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Close")).clicked() {
                close = true;
            }
            ui.weak(language());
        });
    });
    error(ui, &e.error);
    ui.separator();
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(160.0);
            ui.weak(tl!("Modules"));
            if e.modules.is_empty() {
                ui.weak(tl!("No code yet: Insert Module."));
            }
            let mut pick = None;
            for (i, (name, kind, _)) in e.modules.iter().enumerate() {
                let label = if kind == "standard" { name.clone() } else { format!("{name} ({kind})") };
                if ui.selectable_label(i == e.current, label).clicked() {
                    pick = Some(i);
                }
            }
            if let Some(i) = pick
                && i != e.current
            {
                if let Some(m) = e.modules.get_mut(e.current)
                    && e.dirty
                {
                    // Unsaved text stays with its module until saved or closed.
                    m.2 = e.text.clone();
                }
                e.current = i;
                e.text = e.modules.get(i).map(|(_, _, s)| s.clone()).unwrap_or_default();
            }
        });
        ui.separator();
        egui::ScrollArea::both().id_salt("vba_code").max_height(420.0).show(ui, |ui| {
            let resp = ui.add(
                egui::TextEdit::multiline(&mut e.text)
                    .code_editor()
                    .desired_width(560.0)
                    .desired_rows(22)
                    .lock_focus(true)
                    .interactive(!e.modules.is_empty()),
            );
            if resp.changed() {
                e.dirty = true;
            }
        });
    });
    close
}

fn security(app: &mut WordApp, ui: &mut Ui) -> bool {
    let level = app.session.prefs.macro_security.clone();
    for (value, text) in [
        ("disable", tl!("Disable all macros")),
        ("prompt", tl!("Disable macros with notification (enable them per document)")),
        ("enable", tl!("Enable all macros (not recommended: a macro is a program)")),
    ] {
        if ui.radio(level == value, text).clicked()
            && let Err(e) = app.run("developer.macroSecurity", json!({"level": value}))
        {
            app.status(e);
        }
    }
    ui.add_space(6.0);
    ui.button(tl!("Close")).clicked()
}

/// The bar under the ribbon for a document whose macros are disabled (Macro Security: prompt).
pub fn macro_bar(app: &mut WordApp, ui: &mut Ui) {
    if !visible() || app.session.prefs.macro_security != "prompt" || app.session.macros_enabled {
        return;
    }
    let id = app.session.document_id();
    if app.macro_bar_done == Some(id) || !wordcraft_docx::has_macros(&app.session.doc) {
        return;
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    egui::Panel::top("macro_bar").exact_size(32.0).frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(14, 4))).show(
        ui,
        |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(RichText::new(tl!("SECURITY WARNING")).strong());
                ui.label(tl!("Macros have been disabled."));
                if ui.button(tl!("Enable Content")).clicked() {
                    if let Err(e) = app.run("developer.enableMacros", json!({})) {
                        app.status(e);
                    }
                    app.macro_bar_done = Some(id);
                }
                if crate::widgets::icon_button(ui, "close", tl!("Close")).clicked() {
                    app.macro_bar_done = Some(id);
                }
            });
        },
    );
}
