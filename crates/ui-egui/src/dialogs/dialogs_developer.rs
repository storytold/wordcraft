//! Developer › Code: the document's VBA macros, run by a macro add-in (craft-vba). Macros (list,
//! run, edit, create), Visual Basic (the code modules in an editor) and Macro Security. Everything
//! goes through the `developer.*` commands; the add-in owns the language, WordCraft the interface.
//! WordCraft's own recorded command macros stay in View › Macros.
//!
//! The document's own ribbon buttons (Developer › Ribbon Buttons, `developer.*RibbonButton`) run
//! its macros. They show on the tab they name, a built-in one or their own, whether or not a
//! macro add-in is loaded (without one, a click says to install one), and are saved with the
//! document so Word shows them too.

use std::rc::Rc;

use egui::{RichText, Ui};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_doc::{Ribbon, RibbonGroup};

use crate::WordApp;
use crate::widgets::{big, group_titled, small};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "form", rename_all = "camelCase")]
pub enum DevDialog {
    Macros(MacrosForm),
    VisualBasic(Editor),
    Security,
    RibbonButtons(ButtonsForm),
}

/// Developer › Ribbon Buttons: the document's buttons, and the form adding one.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ButtonsForm {
    /// The document's macros, to pick one from.
    pub macros: Vec<String>,
    pub macro_name: String,
    pub label: String,
    pub tab: String,
    pub group: String,
    pub image: String,
    pub small: bool,
    pub error: Option<String>,
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
            DevDialog::RibbonButtons(_) => "ribbonButtons",
        }
    }
    pub fn title(&self) -> &'static str {
        match self {
            DevDialog::Macros(_) => "Macros",
            DevDialog::VisualBasic(_) => "Visual Basic",
            DevDialog::Security => "Macro Security",
            DevDialog::RibbonButtons(_) => "Ribbon Buttons",
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
        "ribbonButtons" => Some(buttons_dialog(app, None)),
        _ => None,
    }
}

/// Developer › Ribbon Buttons, with `macro_name` filled in (the first macro when `None`).
pub fn buttons_dialog(app: &mut WordApp, macro_name: Option<&str>) -> DevDialog {
    let mut f = ButtonsForm { tab: "Macros".into(), group: "Macros".into(), image: "MacroPlay".into(), ..Default::default() };
    if visible()
        && let Ok(r) = app.session.run("developer.macros", &json!({}))
    {
        f.macros = r
            .get("macros")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| m.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .take(1000)
            .collect();
    }
    f.macro_name = macro_name.map(str::to_string).or_else(|| f.macros.first().cloned()).unwrap_or_default();
    DevDialog::RibbonButtons(f)
}

/// The document's ribbon tabs (WordCraft tab name, groups) and whether WordCraft may change them.
fn book(app: &WordApp) -> (Vec<(String, Vec<RibbonGroup>)>, bool) {
    thread_local! {
        // A richer ribbon kept from the file, parsed once: (its bytes' address and length, buttons).
        static KEPT: std::cell::RefCell<Option<((usize, usize), Rc<Ribbon>)>> = const { std::cell::RefCell::new(None) };
    }
    let doc = &app.session.doc;
    let (r, editable): (Rc<Ribbon>, bool) = match wordcraft_docx::custom_ui::kept_part(doc) {
        Some(data) => {
            let key = (data.as_ptr() as usize, data.len());
            let r = KEPT.with_borrow_mut(|k| match k {
                Some((at, r)) if *at == key => r.clone(),
                _ => {
                    let r = Rc::new(wordcraft_docx::custom_ui::parse_buttons(data));
                    *k = Some((key, r.clone()));
                    r
                }
            });
            (r, false)
        }
        None => match &doc.ribbon {
            Some(r) if !r.is_empty() => (Rc::new(r.clone()), true),
            _ => return (Vec::new(), true),
        },
    };
    let mut tabs: Vec<(String, Vec<RibbonGroup>)> = Vec::new();
    for t in &r.tabs {
        let name = wordcraft_engine::cmd::developer_vba::tab_name(t);
        match tabs.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            Some((_, groups)) => groups.extend(t.groups.iter().cloned()),
            None => tabs.push((name, t.groups.clone())),
        }
    }
    (tabs, editable)
}

/// The tabs the document's buttons sit on (the tab row adds those it doesn't have).
pub fn book_tabs(app: &WordApp) -> Vec<String> {
    book(app).0.into_iter().filter(|(_, g)| !g.is_empty()).map(|(n, _)| n).collect()
}

/// Whether the document has buttons on `tab`.
pub fn has_book_groups(app: &WordApp, tab: &str) -> bool {
    book(app).0.iter().any(|(n, g)| n.eq_ignore_ascii_case(tab) && !g.is_empty())
}

/// The picture names offered for a button (Office's `imageMso` names, kept for Word) and the
/// icon WordCraft draws for each.
const PICTURES: &[(&str, &str)] = &[
    ("MacroPlay", "macros"),
    ("HappyFace", "check"),
    ("FileSave", "save"),
    ("FilePrint", "print"),
    ("Copy", "copy"),
    ("Paste", "paste"),
    ("Cut", "cut"),
    ("Undo", "undo"),
    ("Redo", "redo"),
];

fn picture(name: Option<&str>) -> &'static str {
    name.and_then(|n| PICTURES.iter().find(|(k, _)| k.eq_ignore_ascii_case(n))).map(|(_, i)| *i).unwrap_or("macros")
}

/// The button's name for screen readers (and tests), and its tip on hover.
fn described(resp: egui::Response, label: &str, tip: &str) {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    resp.on_hover_text(tip);
}

/// Draws the document's groups on `tab`, after the tab's own; small buttons stack three high.
pub fn book_ribbon_groups(app: &mut WordApp, ui: &mut Ui, tab: &str) {
    let (tabs, _) = book(app);
    let groups: Vec<RibbonGroup> = tabs.into_iter().filter(|(n, _)| n.eq_ignore_ascii_case(tab)).flat_map(|(_, g)| g).collect();
    for (i, g) in groups.iter().enumerate() {
        let title = if g.label.trim().is_empty() { g.id.clone() } else { g.label.clone() };
        group_titled(ui, &format!("document:{i}:{}", g.id), &title, None, app, |ui, app| {
            let tip = |b: &wordcraft_doc::RibbonButton| {
                b.screentip.clone().unwrap_or_else(|| crate::i18n::fmt(tl!("Runs the macro {macro}"), &[("macro", &b.on_action)]))
            };
            let mut k = 0;
            while let Some(b) = g.buttons.get(k) {
                if b.large {
                    let resp = big(ui, app, picture(b.image.as_deref()), &b.label, "developer.pressRibbonButton", json!({"id": b.id}), false);
                    described(resp, &b.label, &tip(b));
                    k += 1;
                    continue;
                }
                let column: Vec<_> = g.buttons.iter().skip(k).take_while(|b| !b.large).take(3).collect();
                k += column.len().max(1);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(1.0, 0.0);
                    for b in column {
                        let resp = small(
                            ui,
                            app,
                            picture(b.image.as_deref()),
                            Some(&b.label),
                            &b.label,
                            crate::widgets::ButtonAction::new("developer.pressRibbonButton", json!({"id": b.id}), false),
                        );
                        described(resp, &b.label, &tip(b));
                    }
                });
            }
        });
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
            let (close, edit, to_ribbon) = macros(app, ui, f);
            if let Some(module) = edit {
                *d = DevDialog::VisualBasic(editor(app, Some(&module)));
                return false;
            }
            if let Some(name) = to_ribbon {
                *d = buttons_dialog(app, Some(&name));
                return false;
            }
            close
        }
        DevDialog::VisualBasic(e) => visual_basic(app, ui, e),
        DevDialog::Security => security(app, ui),
        DevDialog::RibbonButtons(f) => buttons(app, ui, f),
    }
}

/// The built-in tabs a button may go on, besides a tab of the document's own.
const BUTTON_TABS: &[&str] = &["Macros", "Home", "Insert", "Design", "Layout", "References", "Mailings", "Review", "View", "Developer"];

/// Developer › Ribbon Buttons: the document's buttons (Remove) and a form adding one. True when it closes.
fn buttons(app: &mut WordApp, ui: &mut Ui, f: &mut ButtonsForm) -> bool {
    let mut close = false;
    let (tabs, editable) = book(app);
    let mut remove = None;
    ui.set_min_width(480.0);
    ui.weak(tl!("Buttons that run this document's macros. They are saved with the document, and Word shows them too."));
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(460.0);
        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
            if tabs.iter().all(|(_, g)| g.iter().all(|g| g.buttons.is_empty())) {
                ui.weak(tl!("No buttons yet."));
            }
            for (tab, groups) in &tabs {
                for g in groups {
                    for b in &g.buttons {
                        ui.horizontal(|ui| {
                            ui.label(format!("{} › {} › {}", tl!(tab.as_str()), g.label, b.label));
                            ui.weak(crate::i18n::fmt(tl!("runs {macro}"), &[("macro", &b.on_action)]));
                            if editable && ui.small_button(tl!("Remove")).clicked() {
                                remove = Some(b.id.clone());
                            }
                        });
                    }
                }
            }
        });
    });
    if let Some(id) = remove {
        f.error = app.run("developer.removeRibbonButton", json!({"id": id})).err();
    }
    if !editable {
        ui.label(
            RichText::new(tl!(
                "This document's ribbon came from another program with more than buttons: its buttons run, but WordCraft doesn't change it."
            ))
            .weak(),
        );
        return ui.button(tl!("Close")).clicked();
    }
    ui.add_space(4.0);
    ui.strong(tl!("New button"));
    egui::Grid::new("dev_button_form").num_columns(2).spacing(egui::vec2(8.0, 4.0)).show(ui, |ui| {
        ui.label(tl!("Macro:"));
        craft_ui::ComboBox::from_id_salt("dev_button_macro").selected_text(f.macro_name.clone()).width(260.0).show_ui(ui, |ui| {
            for m in &f.macros {
                ui.selectable_value(&mut f.macro_name, m.clone(), m);
            }
        });
        ui.end_row();
        ui.label("");
        ui.add(egui::TextEdit::singleline(&mut f.macro_name).hint_text(tl!("or type a name: Module1.Go")).desired_width(260.0));
        ui.end_row();
        ui.label(tl!("Label:"));
        ui.add(egui::TextEdit::singleline(&mut f.label).hint_text(tl!("the macro's name")).desired_width(260.0));
        ui.end_row();
        ui.label(tl!("Tab:"));
        craft_ui::ComboBox::from_id_salt("dev_button_tab").selected_text(tl!(f.tab.as_str()).to_string()).width(260.0).show_ui(ui, |ui| {
            for t in BUTTON_TABS {
                ui.selectable_value(&mut f.tab, t.to_string(), tl!(t));
            }
        });
        ui.end_row();
        ui.label("");
        ui.add(egui::TextEdit::singleline(&mut f.tab).hint_text(tl!("or a new tab's name")).desired_width(260.0));
        ui.end_row();
        ui.label(tl!("Group:"));
        ui.add(egui::TextEdit::singleline(&mut f.group).desired_width(260.0));
        ui.end_row();
        ui.label(tl!("Picture:"));
        ui.horizontal(|ui| {
            for (name, icon) in PICTURES {
                let on = f.image == *name;
                let (r, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
                let t = crate::theme::Tokens::get(ui.ctx());
                if on || resp.hovered() {
                    ui.painter().rect_filled(r, 4.0, if on { t.accent.gamma_multiply(0.25) } else { t.panel });
                }
                crate::icons::paint(ui.painter(), egui::Rect::from_center_size(r.center(), egui::vec2(18.0, 18.0)), icon, t.icon, t.accent);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, *name));
                if resp.on_hover_text(*name).clicked() {
                    f.image = name.to_string();
                }
            }
        });
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut f.small, tl!("Small button (three to a column)"));
        ui.end_row();
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(!f.macro_name.trim().is_empty(), egui::Button::new(tl!("Add"))).clicked() {
            let params =
                json!({"macro": f.macro_name.trim(), "label": f.label.trim(), "tab": f.tab.trim(), "group": f.group.trim(), "image": f.image, "large": !f.small});
            match app.run("developer.addRibbonButton", params) {
                Ok(_) => {
                    f.error = None;
                    f.label.clear();
                }
                Err(e) => f.error = Some(e),
            }
        }
        if ui.button(tl!("Close")).clicked() {
            close = true;
        }
    });
    error(ui, &f.error);
    close
}

fn error(ui: &mut Ui, e: &Option<String>) {
    if let Some(e) = e {
        ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
    }
}

/// The Macros dialog: (close, the module to open in the editor, the macro to put on the ribbon).
fn macros(app: &mut WordApp, ui: &mut Ui, f: &mut MacrosForm) -> (bool, Option<String>, Option<String>) {
    let enabled = wordcraft_engine::cmd::developer_vba::macros_allowed(&app.session);
    let mut close = false;
    let mut edit = None;
    let mut to_ribbon = None;
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
        if ui
            .add_enabled(chosen.is_some(), egui::Button::new(tl!("Add to Ribbon…")))
            .on_hover_text(tl!("A ribbon button that runs this macro, saved with the document"))
            .clicked()
        {
            to_ribbon = chosen.clone();
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
    (close, edit, to_ribbon)
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
