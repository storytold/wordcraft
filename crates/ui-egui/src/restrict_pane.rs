//! The Restrict Editing pane: formatting restrictions, editing restrictions and starting (or
//! stopping) enforcement. Every action runs `review.restrict`.

use craft::behavior::{ColumnKey, ColumnWidth, EditEvent, keyed};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, SpaceAction,
    ToggleColumn,
};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow, ToggleCell, ToggleState};
use craft::ui::notices::Notice;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterBar, FooterButton, Section};
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::json;
use wordcraft_engine::Password;

use crate::WordApp;

/// The style list's check column.
const CHECK: ColumnKey = ColumnKey::new("allowed");
/// The style list's name column.
const NAME: ColumnKey = ColumnKey::new("name");

/// The Restrict Editing pane's own state: what is being set up before protection starts, and
/// the passwords being typed.
#[derive(Clone, Debug, Default)]
pub(crate) struct RestrictPane {
    /// The pane is shown.
    pub open: bool,
    /// 1. Formatting restrictions: limit formatting to a selection of styles.
    pub formatting: bool,
    /// Its style list shown.
    pub show_styles: bool,
    /// The styles allowed (by id); `None` until first shown (all).
    pub allowed: Option<Vec<String>>,
    /// 2. Editing restrictions on.
    pub editing: bool,
    /// Which editing restriction (index into [`EDIT_MODES`]).
    pub mode: usize,
    /// 3. Asking for the password to start (or to stop) protection.
    pub asking: bool,
    /// The password being typed.
    pub password: Password,
    /// Its confirmation.
    pub confirm: Password,
    /// The last error.
    pub message: String,
}

/// The editing restrictions, with their `review.restrict` modes.
pub(crate) const EDIT_MODES: [(&str, &str); 4] =
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

/// The pane body. The pane area scrolls it, so it brings no scroll area of its own.
pub(crate) fn show(app: &mut WordApp, ui: &mut Ui) {
    let mut st = std::mem::take(&mut app.restrict);
    let enforced = app.session.doc.settings.protection.clone();
    match &enforced {
        Some(mode) => enforced_ui(app, ui, &mut st, mode),
        None => setup_ui(app, ui, &mut st),
    }
    app.restrict = st;
}

fn setup_ui(app: &mut WordApp, ui: &mut Ui, st: &mut RestrictPane) {
    let styles = listed_styles(app);
    if st.allowed.is_none() {
        // Start from the document's own locked styles.
        let doc = &app.session.doc.styles;
        st.allowed = Some(styles.iter().filter(|(id, _)| !doc.get(id).is_some_and(|s| s.locked)).map(|(id, _)| id.clone()).collect());
    }
    Section::fixed(tl!("1. Formatting restrictions")).show(ui, |ui, _| {
        Checkbox::new(Bind::bound(&mut st.formatting)).label(tl!("Limit formatting to a selection of styles")).show(ui);
        if st.formatting && Button::new(tl!("Settings…")).show(ui).clicked() {
            st.show_styles = !st.show_styles;
        }
        if st.formatting && st.show_styles {
            style_list(ui, st, &styles);
        }
    });
    Section::fixed(tl!("2. Editing restrictions")).show(ui, |ui, _| {
        Checkbox::new(Bind::bound(&mut st.editing)).label(tl!("Only allow this kind of editing:")).show(ui);
        let entries: Vec<Entry<String>> = EDIT_MODES.iter().map(|(id, label)| Entry::new((*id).to_string(), tl!(label))).collect();
        let current = EDIT_MODES.get(st.mode).map_or("readOnly", |m| m.0).to_string();
        let picked =
            Select::new(tl!("Only allow this kind of editing:"), Bind::given(current), &entries).enabled(st.editing).show(ui).value().cloned();
        if let Some(id) = picked
            && let Some(at) = EDIT_MODES.iter().position(|m| m.0 == id)
        {
            st.mode = at;
        }
        let mode = EDIT_MODES.get(st.mode).map_or("readOnly", |m| m.0);
        if st.editing && matches!(mode, "readOnly" | "comments") {
            Text::new(tl!("Select text, then tick who can still edit it.")).show(ui);
            let (a, b) = app.session.sel.ordered();
            let inside = app.session.doc.perm_ranges().iter().any(|r| r.everyone() && r.start <= a && b <= r.end && r.start.story == a.story);
            let can = inside || a != b;
            if let Some(everyone) = Checkbox::new(Bind::given(inside)).label(tl!("Everyone")).enabled(can).show(ui).value().copied() {
                let p = if everyone { json!({"exception": "add"}) } else { json!({"exception": "remove"}) };
                if let Err(e) = app.run("review.restrict", p) {
                    st.message = e;
                }
            }
        }
    });
    Section::fixed(tl!("3. Start enforcement")).show(ui, |ui, _| {
        let mode = EDIT_MODES.get(st.mode).map_or("readOnly", |m| m.0);
        if !st.asking {
            if Button::new(tl!("Start Enforcing Protection")).enabled(st.formatting || st.editing).show(ui).clicked() {
                st.asking = true;
                st.message.clear();
            }
        } else {
            Text::new(tl!("The password is optional; without it anyone can stop the protection.")).show(ui);
            passwords(ui, st, true);
            match confirm_buttons(ui) {
                Some(true) => start(app, st, mode, &styles),
                Some(false) => {
                    st.asking = false;
                    st.password = Password::default();
                    st.confirm = Password::default();
                }
                None => {}
            }
        }
        message(ui, &st.message);
    });
}

/// The style list: a check column, toggles reported as edits, with All and None.
fn style_list(ui: &mut Ui, st: &mut RestrictPane, styles: &[(String, String)]) {
    Text::new(tl!("Checked styles are allowed:")).show(ui);
    Row::new().show(ui, |ui, _| {
        if Button::new(tl!("All")).show(ui).clicked() {
            st.allowed = Some(styles.iter().map(|(id, _)| id.clone()).collect());
        }
        if Button::new(tl!("None")).show(ui).clicked() {
            st.allowed = Some(Vec::new());
        }
    });
    let positions: Vec<usize> = (0..styles.len()).collect();
    let source = keyed(&positions, |at: &usize| *at);
    let columns = vec![ColumnSpec::toggle(CHECK, Label::Empty, ToggleColumn::default()), ColumnSpec::new(NAME, Label::Empty, ColumnWidth::Content)];
    let allowed = st.allowed.get_or_insert_with(Vec::new);
    let out = List::<usize>::new(tl!("Checked styles are allowed:"))
        .select(SelectMode::Single)
        .space(SpaceAction::ToggleCell)
        .header(false)
        .sizing(Sizing::Content)
        .columns(columns)
        .show_items(ui, &source, &mut |at, icx| {
            let Some((id, name)) = styles.get(*at) else { return ListRow::new(Label::Empty) };
            ListRow::new(name.clone()).cells(icx.cells()).toggle(ToggleCell::check(CHECK, ToggleState::of(allowed.contains(id)), name.clone()))
        });
    for event in &out.events {
        if let CollectionEvent::Toggle(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) = event
            && let Some((id, _)) = styles.get(after.key)
        {
            allowed.retain(|x| x != id);
            if after.on {
                allowed.push(id.clone());
            }
        }
    }
}

/// OK and Cancel in the platform's order: `Some(true)` for OK, `Some(false)` for Cancel.
fn confirm_buttons(ui: &mut Ui) -> Option<bool> {
    let out =
        FooterBar::buttons([FooterButton::new(tl!("OK"), DialogRole::Accept), FooterButton::new(tl!("Cancel"), DialogRole::Cancel)]).show_footer(ui);
    out.picked.map(|i| i == 0)
}

fn start(app: &mut WordApp, st: &mut RestrictPane, mode: &str, styles: &[(String, String)]) {
    if st.password != st.confirm {
        st.message = tl!("The passwords don't match.").to_string();
        st.confirm.as_mut_string().clear();
        return;
    }
    let mut p = json!({"mode": if st.editing { mode } else { "none" }, "formatting": st.formatting, "password": st.password.as_str()});
    if st.formatting {
        let allowed = st.allowed.clone().unwrap_or_default();
        p["lockedStyles"] = json!(styles.iter().filter(|(id, _)| !allowed.contains(id)).map(|(id, _)| id.clone()).collect::<Vec<_>>());
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

/// The password field, and its confirmation when starting.
fn passwords(ui: &mut Ui, st: &mut RestrictPane, confirm: bool) {
    TextField::new(tl!("Password:"), Bind::bound(st.password.as_mut_string())).masked().show(ui);
    if confirm {
        TextField::new(tl!("Confirm password:"), Bind::bound(st.confirm.as_mut_string())).masked().show(ui);
    }
}

fn message(ui: &mut Ui, message: &str) {
    if !message.is_empty() {
        Notice::new(Tone::Bad, message.to_owned()).show(ui);
    }
}

fn enforced_ui(app: &mut WordApp, ui: &mut Ui, st: &mut RestrictPane, mode: &str) {
    Text::new(tl!("Editing in this document is restricted.")).show(ui);
    let what = match mode {
        "readOnly" => "The document is read only, except for regions marked editable.",
        "comments" => "Only comments can be added, except in regions marked editable.",
        "trackedChanges" => "Every change is recorded as a tracked change.",
        "forms" => "Only form fields can be filled in.",
        _ => "Only the allowed styles can be applied.",
    };
    Text::new(tl!(what)).show(ui);
    let regions = app.session.doc.perm_ranges().iter().filter(|r| r.everyone() || r.editor.eq_ignore_ascii_case(&app.session.author)).count();
    let fields = app.session.doc.fillable_controls() as u64;
    if (regions > 0 || fields > 0) && Button::new(tl!("Find Next Editable Region")).show(ui).clicked() {
        let from = app.session.sel.ordered().1;
        if let Some((a, b)) = wordcraft_engine::cmd::protect::next_region(&app.session, &from) {
            app.session.set_selection(wordcraft_engine::Selection { anchor: a, focus: b });
            app.canvas.focus_page(ui.ctx());
        }
    }
    let has_password = !app.session.doc.settings.protect_hash.is_empty();
    if !st.asking {
        if Button::new(tl!("Stop Protection")).show(ui).clicked() {
            if has_password {
                st.asking = true;
                st.message.clear();
            } else {
                stop(app, st);
            }
        }
    } else {
        Text::new(tl!("Type the password to stop the protection.")).show(ui);
        passwords(ui, st, false);
        match confirm_buttons(ui) {
            Some(true) => stop(app, st),
            Some(false) => {
                st.asking = false;
                st.password = Password::default();
            }
            None => {}
        }
    }
    message(ui, &st.message);
}

fn stop(app: &mut WordApp, st: &mut RestrictPane) {
    let r = app.run("review.restrict", json!({"stop": true, "password": st.password.as_str()}));
    st.password = Password::default();
    match r {
        Ok(_) => {
            st.asking = false;
            st.message.clear();
        }
        Err(e) if e.contains(wordcraft_engine::cmd::protect::UNSUPPORTED_PASSWORD) => {
            st.message = tl!("This password type isn't supported.").to_string();
        }
        Err(e) => st.message = e,
    }
}
