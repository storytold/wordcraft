//! Review dialogs and the Restrict Editing pane (#412): Compare / Combine Documents (with the
//! More >> comparison settings), AutoCorrect Options (AutoCorrect, AutoFormat As You Type and
//! Math AutoCorrect) and the Restrict Editing side pane. Each ends by running a command
//! (`review.compare`, `review.combine`, `tools.autocorrect`, `review.restrict`), so scripts and
//! agents get the same result without them. Shown through [`crate::dialogs::Dialog::Review`] and
//! [`crate::panes`].

use egui::{Color32, Ui, vec2};
use serde::Serialize;
use serde_json::{Value, json};
use wordcraft_engine::Password;
use wordcraft_engine::cmd::tools::AutoCorrectPrefs;

use crate::WordApp;
use crate::theme::semibold;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "form", rename_all = "camelCase")]
pub enum ReviewDialog {
    Compare(CompareForm),
    AutoCorrect(Box<AutoCorrectForm>),
}

impl ReviewDialog {
    pub fn name(&self) -> &'static str {
        match self {
            ReviewDialog::Compare(f) if f.combine => "combine",
            ReviewDialog::Compare(_) => "compare",
            ReviewDialog::AutoCorrect(_) => "autoCorrect",
        }
    }

    /// The window title (English; the caller translates it).
    pub fn title(&self) -> &'static str {
        match self {
            ReviewDialog::Compare(f) if f.combine => "Combine Documents",
            ReviewDialog::Compare(_) => "Compare Documents",
            ReviewDialog::AutoCorrect(_) => "AutoCorrect",
        }
    }
}

/// The dialog `ui.dialog` opens by `name`: `compare`, `combine` or `autoCorrect`.
/// `restrictEditing` opens the Restrict Editing pane instead (no dialog).
pub fn open(name: &str, app: &mut WordApp) -> Option<ReviewDialog> {
    Some(match name {
        "compare" | "combine" => ReviewDialog::Compare(CompareForm::new(app, name == "combine")),
        "autoCorrect" => ReviewDialog::AutoCorrect(Box::new(AutoCorrectForm::new(app))),
        "restrictEditing" => {
            app.restrict.open = true;
            return None;
        }
        _ => return None,
    })
}

/// Draws the dialog; returns true to close it.
pub fn body(app: &mut WordApp, ui: &mut Ui, d: &mut ReviewDialog) -> bool {
    match d {
        ReviewDialog::Compare(f) => compare_ui(app, ui, f),
        ReviewDialog::AutoCorrect(f) => autocorrect_ui(app, ui, f),
    }
}

/// OK / Cancel; returns (ok, cancel). `ok_enabled` greys OK out.
fn buttons(ui: &mut Ui, ok_enabled: bool) -> (bool, bool) {
    let mut r = (false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(tl!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                r.1 = true;
            }
            let okb =
                ui.add_enabled(ok_enabled, egui::Button::new(egui::RichText::new(tl!("OK")).color(Color32::WHITE)).fill(crate::theme::APP_COLOR));
            if okb.clicked() {
                r.0 = true;
            }
        });
    });
    r
}

fn heading(ui: &mut Ui, s: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!(s)).font(semibold(12.5)));
}

fn error(ui: &mut Ui, message: &str) {
    if !message.is_empty() {
        ui.label(egui::RichText::new(message).small().color(ui.visuals().error_fg_color));
    }
}

// ---------------------------------------------------------------------------------------------
// Compare / Combine

/// The comparison settings (More >>), in the order the dialog shows them, with their
/// `review.compare` parameter names.
pub const COMPARE_SETTINGS: [(&str, &str); 10] = [
    ("moves", "Moves"),
    ("comments", "Comments"),
    ("formatting", "Formatting"),
    ("caseChanges", "Case changes"),
    ("whiteSpace", "White space"),
    ("tables", "Tables"),
    ("headersFooters", "Headers and footers"),
    ("footnotes", "Footnotes and endnotes"),
    ("textBoxes", "Text boxes"),
    ("fields", "Fields"),
];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareForm {
    pub combine: bool,
    /// The original is the open document (else the file `original`).
    pub original_current: bool,
    pub original: String,
    pub revised: String,
    /// Label changes with.
    pub author: String,
    /// More >> shown.
    pub more: bool,
    /// [`COMPARE_SETTINGS`], all on by default as in Word.
    pub settings: [bool; 10],
    pub character_level: bool,
    /// Show changes in: `original`, `revised` or `new`.
    pub show_in: String,
    pub message: String,
}

impl CompareForm {
    pub fn new(app: &WordApp, combine: bool) -> CompareForm {
        CompareForm {
            combine,
            original_current: true,
            original: String::new(),
            revised: String::new(),
            author: app.session.author.clone(),
            more: false,
            settings: [true; 10],
            character_level: false,
            show_in: "new".into(),
            message: String::new(),
        }
    }

    /// The `review.compare` / `review.combine` parameters; `None` until a revised document is
    /// chosen.
    pub fn params(&self) -> Option<Value> {
        let revised = self.revised.trim();
        if revised.is_empty() || (!self.original_current && self.original.trim().is_empty()) {
            return None;
        }
        let mut v = json!({
            "path": revised,
            "author": self.author.trim(),
            "level": if self.character_level { "character" } else { "word" },
            "showIn": self.show_in,
        });
        if !self.original_current {
            v["original"] = json!(self.original.trim());
        }
        for ((key, _), on) in COMPARE_SETTINGS.iter().zip(self.settings) {
            v[*key] = json!(on);
        }
        Some(v)
    }
}

/// Can this front end pick files (desktop), rather than only receive them (web)?
fn can_browse(app: &WordApp) -> bool {
    app.services.file_dialog.is_some() || app.services.pick_open.is_some()
}

/// Put a picked file into the open Compare dialog (`revised`: the revised document's box).
pub(crate) fn set_compare_path(app: &mut WordApp, revised: bool, path: String) {
    if let Some(crate::dialogs::Dialog::Review(d)) = app.dialog.as_mut()
        && let ReviewDialog::Compare(f) = &mut **d
    {
        if revised {
            f.revised = path;
        } else {
            f.original = path;
            f.original_current = false;
        }
    }
}

fn compare_ui(app: &mut WordApp, ui: &mut Ui, f: &mut CompareForm) -> bool {
    let browse = can_browse(app);
    let mut pick: Option<bool> = None;
    if !browse {
        ui.label(egui::RichText::new(tl!("Comparing with another file needs the desktop app.")).small().weak());
    }
    egui::Grid::new("compare_files").num_columns(2).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
        ui.vertical(|ui| {
            heading(ui, "Original document");
            ui.radio_value(&mut f.original_current, true, tl!("This document"));
            ui.horizontal(|ui| {
                ui.radio_value(&mut f.original_current, false, "");
                ui.add_enabled(browse, egui::TextEdit::singleline(&mut f.original).desired_width(170.0).hint_text(tl!("Another file")));
                if ui.add_enabled(browse, egui::Button::new(tl!("Browse…"))).clicked() {
                    pick = Some(false);
                }
            });
        });
        ui.vertical(|ui| {
            heading(ui, "Revised document");
            ui.horizontal(|ui| {
                ui.add_enabled(browse, egui::TextEdit::singleline(&mut f.revised).desired_width(170.0));
                if ui.add_enabled(browse, egui::Button::new(tl!("Browse…"))).clicked() {
                    pick = Some(true);
                }
            });
            ui.horizontal(|ui| {
                ui.label(tl!("Label changes with:"));
                ui.add(egui::TextEdit::singleline(&mut f.author).desired_width(120.0));
            });
        });
        ui.end_row();
    });
    ui.add_space(4.0);
    if ui.button(if f.more { tl!("<< Less") } else { tl!("More >>") }).clicked() {
        f.more = !f.more;
    }
    if f.more {
        heading(ui, "Comparison settings");
        egui::Grid::new("compare_settings").num_columns(2).spacing(vec2(16.0, 2.0)).show(ui, |ui| {
            for (i, (_, label)) in COMPARE_SETTINGS.iter().enumerate() {
                if let Some(on) = f.settings.get_mut(i) {
                    ui.checkbox(on, tl!(label));
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        ui.label(
            egui::RichText::new(tl!("Formatting, case changes, white space and tables are compared; the other settings are kept for later."))
                .small()
                .weak(),
        );
        egui::Grid::new("compare_show").num_columns(2).spacing(vec2(24.0, 2.0)).show(ui, |ui| {
            ui.vertical(|ui| {
                heading(ui, "Show changes at:");
                ui.radio_value(&mut f.character_level, false, tl!("Word level"));
                ui.radio_value(&mut f.character_level, true, tl!("Character level"));
            });
            ui.vertical(|ui| {
                heading(ui, "Show changes in:");
                for (v, l) in [("original", "Original document"), ("revised", "Revised document"), ("new", "New document")] {
                    ui.radio_value(&mut f.show_in, v.to_string(), tl!(l));
                }
            });
            ui.end_row();
        });
    }
    if let Some(revised) = pick {
        // A blocking picker answers now, while this dialog is out of `app.dialog`: fill it here.
        if app.services.file_dialog.is_none()
            && let Some(pick_open) = &app.services.pick_open
        {
            if let Some(path) = pick_open("document") {
                if revised {
                    f.revised = path;
                } else {
                    f.original = path;
                    f.original_current = false;
                }
            }
        } else {
            let _ = app.ask_file(
                crate::file_dialogs::FileDialogRequest::Open { purpose: "document".into() },
                crate::file_dialogs::AfterPick::ComparePath { revised },
            );
        }
    }
    error(ui, &f.message);
    let params = f.params();
    let (ok, cancel) = buttons(ui, params.is_some());
    if ok && let Some(params) = params {
        let id = if f.combine { "review.combine" } else { "review.compare" };
        match app.run(id, params) {
            Ok(v) => {
                let n = v.get("changes").and_then(Value::as_u64).unwrap_or(0).to_string();
                app.status(crate::i18n::fmt(tl!("Compared: {changes} changes"), &[("changes", &n)]));
                return true;
            }
            Err(e) => {
                f.message = e;
                return false;
            }
        }
    }
    cancel
}

// ---------------------------------------------------------------------------------------------
// AutoCorrect Options

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoCorrectForm {
    /// 0 AutoCorrect, 1 AutoFormat As You Type, 2 Math AutoCorrect.
    pub tab: u8,
    /// The options as edited (applied on OK); the lists change at once, as in Word.
    pub options: AutoCorrectPrefs,
    /// Replace / With boxes.
    pub from: String,
    pub to: String,
    /// Exceptions shown, and the exception being typed.
    pub show_exceptions: bool,
    pub exception: String,
    /// Math AutoCorrect filter.
    pub math_filter: String,
    pub message: String,
}

impl AutoCorrectForm {
    pub fn new(app: &WordApp) -> AutoCorrectForm {
        AutoCorrectForm {
            tab: 0,
            options: app.session.prefs.autocorrect.clone(),
            from: String::new(),
            to: String::new(),
            show_exceptions: false,
            exception: String::new(),
            math_filter: String::new(),
            message: String::new(),
        }
    }

    /// The `tools.autocorrect` switches.
    pub fn params(&self) -> Value {
        let o = &self.options;
        json!({
            "replaceText": o.replace_text,
            "capSentences": o.cap_sentences,
            "capCells": o.cap_cells,
            "capDays": o.cap_days,
            "smartQuotes": o.smart_quotes,
            "fractions": o.fractions,
            "ordinals": o.ordinals,
            "dashes": o.dashes,
            "links": o.links,
            "bullets": o.bullets,
            "numbering": o.numbering,
            "borderLines": o.border_lines,
        })
    }
}

/// Run a list change at once and take the lists it reports back.
fn list_change(app: &mut WordApp, f: &mut AutoCorrectForm, params: Value) -> bool {
    match app.run("tools.autocorrect", params) {
        Ok(_) => {
            let now = app.session.prefs.autocorrect.clone();
            f.options.entries = now.entries;
            f.options.removed = now.removed;
            f.options.exceptions = now.exceptions;
            f.options.exceptions_removed = now.exceptions_removed;
            f.message.clear();
            true
        }
        Err(e) => {
            f.message = e;
            false
        }
    }
}

fn autocorrect_ui(app: &mut WordApp, ui: &mut Ui, f: &mut AutoCorrectForm) -> bool {
    ui.horizontal(|ui| {
        for (i, l) in ["AutoCorrect", "AutoFormat As You Type", "Math AutoCorrect"].into_iter().enumerate() {
            if ui.selectable_label(f.tab == i as u8, tl!(l)).clicked() {
                f.tab = i as u8;
            }
        }
    });
    ui.separator();
    match f.tab {
        0 => autocorrect_tab(app, ui, f),
        1 => {
            let o = &mut f.options;
            heading(ui, "Replace as you type");
            ui.checkbox(&mut o.smart_quotes, tl!("Smart quotes instead of straight quotes"));
            ui.checkbox(&mut o.fractions, tl!("Fractions (1/2) as fraction characters (½)"));
            ui.checkbox(&mut o.ordinals, tl!("Ordinals (1st) as superscript"));
            ui.checkbox(&mut o.dashes, tl!("Hyphens (--) as dashes (—)"));
            ui.checkbox(&mut o.links, tl!("Internet addresses as links"));
            heading(ui, "Apply as you type");
            ui.checkbox(&mut o.bullets, tl!("Automatic bulleted lists"));
            ui.checkbox(&mut o.numbering, tl!("Automatic numbered lists"));
            ui.checkbox(&mut o.border_lines, tl!("Border lines"));
        }
        _ => math_tab(ui, f),
    }
    error(ui, &f.message);
    let (ok, cancel) = buttons(ui, true);
    if ok {
        if let Err(e) = app.run("tools.autocorrect", f.params()) {
            f.message = e;
            return false;
        }
        return true;
    }
    cancel
}

fn autocorrect_tab(app: &mut WordApp, ui: &mut Ui, f: &mut AutoCorrectForm) {
    let o = &mut f.options;
    ui.checkbox(&mut o.cap_sentences, tl!("Capitalize first letter of sentences"));
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        if ui.small_button(tl!("Exceptions…")).clicked() {
            f.show_exceptions = !f.show_exceptions;
        }
    });
    if f.show_exceptions {
        exceptions(app, ui, f);
    }
    let o = &mut f.options;
    ui.checkbox(&mut o.cap_cells, tl!("Capitalize first letter of table cells"));
    ui.checkbox(&mut o.cap_days, tl!("Capitalize names of days"));
    ui.checkbox(&mut o.replace_text, tl!("Replace text as you type"));
    egui::Grid::new("ac_boxes").num_columns(2).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
        ui.label(tl!("Replace:"));
        ui.label(tl!("With:"));
        ui.end_row();
        ui.add(egui::TextEdit::singleline(&mut f.from).desired_width(110.0));
        ui.add(egui::TextEdit::singleline(&mut f.to).desired_width(260.0));
        ui.end_row();
    });
    // The list, filtered to what's typed in Replace (as Word scrolls to it).
    let list = f.options.list();
    let typed = f.from.trim().to_lowercase();
    let mut picked: Option<(String, String)> = None;
    egui::ScrollArea::vertical().id_salt("ac_list").max_height(170.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("ac_list_grid").num_columns(2).spacing(vec2(8.0, 1.0)).striped(true).show(ui, |ui| {
            for (a, b, _) in list.iter().filter(|(a, _, _)| typed.is_empty() || a.to_lowercase().starts_with(&typed)).take(500) {
                let selected = f.from == *a;
                if ui.add_sized(vec2(110.0, 16.0), egui::Button::selectable(selected, a.as_str())).clicked() {
                    picked = Some((a.clone(), b.clone()));
                }
                ui.label(b.as_str());
                ui.end_row();
            }
        });
    });
    if let Some((a, b)) = picked {
        f.from = a;
        f.to = b;
    }
    let exists = list.iter().any(|(a, _, _)| *a == f.from);
    let same = list.iter().any(|(a, b, _)| *a == f.from && *b == f.to);
    ui.horizontal(|ui| {
        let label = if exists { tl!("Replace") } else { tl!("Add") };
        if ui.add_enabled(!f.from.trim().is_empty() && !same, egui::Button::new(label)).clicked() {
            let p = json!({"add": {"from": f.from, "to": f.to}});
            if list_change(app, f, p) {
                f.from.clear();
                f.to.clear();
            }
        }
        if ui.add_enabled(exists, egui::Button::new(tl!("Delete"))).clicked() {
            let p = json!({"delete": f.from});
            if list_change(app, f, p) {
                f.from.clear();
                f.to.clear();
            }
        }
    });
}

fn exceptions(app: &mut WordApp, ui: &mut Ui, f: &mut AutoCorrectForm) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(tl!("Don't capitalize after:"));
        let list = f.options.exception_list();
        let mut picked = None;
        egui::ScrollArea::vertical().id_salt("ac_exceptions").max_height(90.0).auto_shrink([false, true]).show(ui, |ui| {
            for e in &list {
                if ui.add(egui::Button::selectable(f.exception == *e, e.as_str())).clicked() {
                    picked = Some(e.clone());
                }
            }
        });
        if let Some(e) = picked {
            f.exception = e;
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut f.exception).desired_width(110.0));
            let known = list.contains(&f.exception.trim().to_lowercase());
            if ui.add_enabled(!f.exception.trim().is_empty() && !known, egui::Button::new(tl!("Add"))).clicked() {
                let p = json!({"addException": f.exception});
                if list_change(app, f, p) {
                    f.exception.clear();
                }
            }
            if ui.add_enabled(known, egui::Button::new(tl!("Delete"))).clicked() {
                let p = json!({"deleteException": f.exception});
                if list_change(app, f, p) {
                    f.exception.clear();
                }
            }
        });
    });
}

fn math_tab(ui: &mut Ui, f: &mut AutoCorrectForm) {
    ui.label(egui::RichText::new(tl!("In equations, typing a name after a backslash replaces it with its symbol.")).small().weak());
    ui.horizontal(|ui| {
        ui.label(tl!("Replace:"));
        ui.add(egui::TextEdit::singleline(&mut f.math_filter).desired_width(140.0));
    });
    let filter = f.math_filter.trim().trim_start_matches('\\').to_string();
    egui::ScrollArea::vertical().id_salt("ac_math").max_height(240.0).auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("ac_math_grid").num_columns(2).spacing(vec2(16.0, 1.0)).striped(true).show(ui, |ui| {
            for (name, c) in wordcraft_doc::math_symbols::SYMBOLS.iter().filter(|(n, _)| filter.is_empty() || n.starts_with(filter.as_str())) {
                ui.label(format!("\\{name}"));
                ui.label(c.to_string());
                ui.end_row();
            }
        });
    });
}

// ---------------------------------------------------------------------------------------------
// Restrict Editing pane

/// The Restrict Editing pane's own state: what is being set up before protection starts, and
/// the passwords being typed.
#[derive(Clone, Debug, Default)]
pub struct RestrictPane {
    pub open: bool,
    /// 1. Formatting restrictions: limit formatting to a selection of styles.
    pub formatting: bool,
    /// Its style list shown, and the styles allowed (by id); `None` until first shown (all).
    pub show_styles: bool,
    pub allowed: Option<Vec<String>>,
    /// 2. Editing restrictions on, and which (index into [`EDIT_MODES`]).
    pub editing: bool,
    pub mode: usize,
    /// 3. Asking for the password to start (or to stop) protection.
    pub asking: bool,
    pub password: Password,
    pub confirm: Password,
    pub message: String,
}

/// The editing restrictions, with their `review.restrict` modes.
pub const EDIT_MODES: [(&str, &str); 4] =
    [("readOnly", "No changes (read only)"), ("trackedChanges", "Tracked changes"), ("comments", "Comments"), ("forms", "Filling in forms")];

/// The styles the formatting restriction lists: paragraph and character styles not hidden.
fn listed_styles(app: &WordApp) -> Vec<(String, String)> {
    use wordcraft_doc::StyleKind;
    let mut v: Vec<(String, String)> = app
        .session
        .doc
        .styles
        .styles
        .iter()
        .filter(|s| matches!(s.kind, StyleKind::Paragraph | StyleKind::Character) && !s.hidden)
        .map(|s| (s.id.clone(), if s.name.is_empty() { s.id.clone() } else { s.name.clone() }))
        .collect();
    v.sort_by_key(|a| a.1.to_lowercase());
    v
}

pub fn restrict_pane(app: &mut WordApp, ui: &mut Ui) {
    let mut st = std::mem::take(&mut app.restrict);
    let enforced = app.session.doc.settings.protection.clone();
    egui::ScrollArea::vertical().id_salt("restrict_pane").show(ui, |ui| match &enforced {
        Some(mode) => enforced_ui(app, ui, &mut st, mode),
        None => setup_ui(app, ui, &mut st),
    });
    app.restrict = st;
}

fn setup_ui(app: &mut WordApp, ui: &mut Ui, st: &mut RestrictPane) {
    heading(ui, "1. Formatting restrictions");
    ui.checkbox(&mut st.formatting, tl!("Limit formatting to a selection of styles"));
    if st.formatting && ui.small_button(tl!("Settings…")).clicked() {
        st.show_styles = !st.show_styles;
    }
    let styles = listed_styles(app);
    if st.allowed.is_none() {
        // Start from the document's own locked styles.
        let doc = &app.session.doc.styles;
        st.allowed = Some(styles.iter().filter(|(id, _)| !doc.get(id).is_some_and(|s| s.locked)).map(|(id, _)| id.clone()).collect());
    }
    if st.formatting && st.show_styles {
        let allowed = st.allowed.get_or_insert_with(Vec::new);
        ui.label(egui::RichText::new(tl!("Checked styles are allowed:")).small());
        ui.horizontal(|ui| {
            if ui.small_button(tl!("All")).clicked() {
                *allowed = styles.iter().map(|(id, _)| id.clone()).collect();
            }
            if ui.small_button(tl!("None")).clicked() {
                allowed.clear();
            }
        });
        egui::ScrollArea::vertical().id_salt("restrict_styles").max_height(160.0).show(ui, |ui| {
            for (id, name) in &styles {
                let mut on = allowed.contains(id);
                if ui.checkbox(&mut on, tl!(name.as_str())).changed() {
                    if on {
                        allowed.push(id.clone());
                    } else {
                        allowed.retain(|x| x != id);
                    }
                }
            }
        });
    }
    heading(ui, "2. Editing restrictions");
    ui.checkbox(&mut st.editing, tl!("Only allow this kind of editing:"));
    ui.add_enabled_ui(st.editing, |ui| {
        let current = EDIT_MODES.get(st.mode).map(|m| m.1).unwrap_or("");
        egui::ComboBox::from_id_salt("restrict_mode").selected_text(tl!(current)).width(220.0).show_ui(ui, |ui| {
            for (i, (_, label)) in EDIT_MODES.iter().enumerate() {
                ui.selectable_value(&mut st.mode, i, tl!(label));
            }
        });
    });
    let mode = EDIT_MODES.get(st.mode).map(|m| m.0).unwrap_or("readOnly");
    if st.editing && matches!(mode, "readOnly" | "comments") {
        heading(ui, "Exceptions (optional)");
        ui.label(egui::RichText::new(tl!("Select text, then tick who can still edit it.")).small());
        let (a, b) = app.session.sel.ordered();
        let inside = app.session.doc.perm_ranges().iter().any(|r| r.everyone() && r.start <= a && b <= r.end && r.start.story == a.story);
        let mut everyone = inside;
        let can = inside || a != b;
        if ui.add_enabled(can, egui::Checkbox::new(&mut everyone, tl!("Everyone"))).changed() {
            let p = if everyone { json!({"exception": "add"}) } else { json!({"exception": "remove"}) };
            if let Err(e) = app.run("review.restrict", p) {
                st.message = e;
            }
        }
    }
    heading(ui, "3. Start enforcement");
    let ready = st.formatting || st.editing;
    if !st.asking {
        if ui.add_enabled(ready, egui::Button::new(tl!("Start Enforcing Protection"))).clicked() {
            st.asking = true;
            st.message.clear();
        }
    } else {
        ui.label(egui::RichText::new(tl!("The password is optional; without it anyone can stop the protection.")).small());
        passwords(ui, st, true);
        ui.horizontal(|ui| {
            if ui.button(tl!("OK")).clicked() {
                if st.password != st.confirm {
                    st.message = tl!("The passwords don't match.").to_string();
                    st.confirm.as_mut_string().clear();
                } else {
                    let mut p =
                        json!({"mode": if st.editing { mode } else { "none" }, "formatting": st.formatting, "password": st.password.as_str()});
                    if st.formatting {
                        let allowed = st.allowed.clone().unwrap_or_default();
                        p["lockedStyles"] =
                            json!(styles.iter().filter(|(id, _)| !allowed.contains(id)).map(|(id, _)| id.clone()).collect::<Vec<_>>());
                    }
                    match app.run("review.restrict", p) {
                        Ok(_) => {
                            st.asking = false;
                            st.message.clear();
                        }
                        Err(e) => st.message = e,
                    }
                    st.password = Password::default();
                    st.confirm = Password::default();
                }
            }
            if ui.button(tl!("Cancel")).clicked() {
                st.asking = false;
                st.password = Password::default();
                st.confirm = Password::default();
            }
        });
    }
    error(ui, &st.message);
}

fn passwords(ui: &mut Ui, st: &mut RestrictPane, confirm: bool) {
    egui::Grid::new("restrict_pw").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
        ui.label(tl!("Password:"));
        ui.add(egui::TextEdit::singleline(st.password.as_mut_string()).password(true).desired_width(140.0));
        ui.end_row();
        if confirm {
            ui.label(tl!("Confirm password:"));
            ui.add(egui::TextEdit::singleline(st.confirm.as_mut_string()).password(true).desired_width(140.0));
            ui.end_row();
        }
    });
}

fn enforced_ui(app: &mut WordApp, ui: &mut Ui, st: &mut RestrictPane, mode: &str) {
    ui.label(tl!("Editing in this document is restricted."));
    ui.add_space(4.0);
    let what = match mode {
        "readOnly" => "The document is read only, except for regions marked editable.",
        "comments" => "Only comments can be added, except in regions marked editable.",
        "trackedChanges" => "Every change is recorded as a tracked change.",
        "forms" => "Only form fields can be filled in.",
        _ => "Only the allowed styles can be applied.",
    };
    ui.label(egui::RichText::new(tl!(what)).small());
    let regions = app.session.doc.perm_ranges().iter().filter(|r| r.everyone() || r.editor.eq_ignore_ascii_case(&app.session.author)).count();
    if regions > 0 {
        ui.add_space(6.0);
        if ui.button(tl!("Find Next Editable Region")).clicked() {
            let from = app.session.sel.ordered().1;
            if let Some((a, b)) = wordcraft_engine::cmd::protect::next_region(&app.session, &from) {
                app.session.sel = wordcraft_engine::Selection { anchor: a, focus: b };
                app.canvas.want_focus = true;
            }
        }
    }
    ui.add_space(10.0);
    let has_password = !app.session.doc.settings.protect_hash.is_empty();
    if !st.asking {
        if ui.button(tl!("Stop Protection")).clicked() {
            if has_password {
                st.asking = true;
                st.message.clear();
            } else {
                stop(app, st);
            }
        }
    } else {
        ui.label(egui::RichText::new(tl!("Type the password to stop the protection.")).small());
        passwords(ui, st, false);
        ui.horizontal(|ui| {
            if ui.button(tl!("OK")).clicked() {
                stop(app, st);
            }
            if ui.button(tl!("Cancel")).clicked() {
                st.asking = false;
                st.password = Password::default();
            }
        });
    }
    error(ui, &st.message);
}

fn stop(app: &mut WordApp, st: &mut RestrictPane) {
    let r = app.run("review.restrict", json!({"stop": true, "password": st.password.as_str()}));
    st.password = Password::default();
    match r {
        Ok(v) => {
            st.asking = false;
            st.message.clear();
            if v.get("verified") == Some(&json!(false)) {
                app.status(tl!("Protection stopped. The document's password uses an older form WordCraft can't check."));
            }
        }
        Err(e) => st.message = e,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::*;
    use crate::{Services, WordApp};

    fn app(text: &str) -> WordApp {
        WordApp::new(Session::new(wordcraft_doc::Document::from_text(text)), Services::default())
    }

    #[test]
    fn autocorrect_entries_persist_in_prefs_and_junk_is_ignored() {
        let mut a = app("");
        a.run("tools.autocorrect", json!({"add": {"from": "brb", "to": "be right back"}, "capDays": false})).unwrap();
        let saved = serde_json::to_string(&a.prefs()).unwrap();
        let mut b = app("");
        b.apply_prefs(serde_json::from_str(&saved).unwrap());
        assert_eq!(b.session.prefs.autocorrect.replacement("brb").as_deref(), Some("be right back"));
        assert!(!b.session.prefs.autocorrect.cap_days);
        for c in "brb ".chars() {
            b.session.run("text.insert", &json!({"text": c.to_string()})).unwrap();
        }
        assert_eq!(b.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Be right back ");
        // A damaged AutoCorrect section reads as the defaults; the rest of the file still loads.
        let mut v: Value = serde_json::from_str(&saved).unwrap();
        v["editing"]["autocorrect"] = json!({"entries": 7, "capDays": "x"});
        v["author"] = json!("Ada");
        let mut c = app("");
        c.apply_prefs(serde_json::from_value(v).unwrap());
        assert_eq!(c.session.prefs.autocorrect, AutoCorrectPrefs::default());
        assert_eq!(c.session.author, "Ada");
        // Hostile entries are dropped on load.
        let mut v: Value = serde_json::from_str(&saved).unwrap();
        v["editing"]["autocorrect"]["entries"] = json!([["", "x"], ["ok", "fine"], ["ok", "dup"], ["a\nb", "x"]]);
        let mut d = app("");
        d.apply_prefs(serde_json::from_value(v).unwrap());
        assert_eq!(d.session.prefs.autocorrect.entries, vec![("ok".to_string(), "fine".to_string())]);
    }

    #[test]
    fn dialogs_open_by_name_and_run_their_commands() {
        let mut a = app("Original text.");
        a.run("review.compare", json!({})).unwrap();
        assert_eq!(a.dialog.as_ref().map(|d| d.name()), Some("compare"));
        a.run("review.combine", json!({})).unwrap();
        assert_eq!(a.dialog.as_ref().map(|d| d.name()), Some("combine"));
        a.run("tools.autocorrect", json!({})).unwrap();
        assert_eq!(a.dialog.as_ref().map(|d| d.name()), Some("autoCorrect"));
        a.dialog = None;
        // Restrict Editing without settings opens the pane and protects nothing.
        a.run("review.restrict", json!({})).unwrap();
        assert!(a.restrict.open && a.dialog.is_none());
        assert!(a.session.doc.settings.protection.is_none());
        // The form's parameters carry every comparison setting.
        let mut f = CompareForm::new(&a, false);
        assert!(f.params().is_none());
        f.revised = "/tmp/revised.docx".into();
        f.settings[3] = false;
        let p = f.params().unwrap();
        assert_eq!(p["caseChanges"], false);
        assert_eq!(p["showIn"], "new");
        assert!(p.get("original").is_none());
    }
}
