//! Find, Replace and Go To: one modeless dialog with three tabs. The dialog files requests; this
//! module runs them as commands and reports back in its message line.

use craft::text::find::FindOptions;
use craft::text::ui::find::{FindReplaceDialog, FindRequest, FindTab, GoToKind};
use craft::ui::dialog::{DialogId, dialog_id};
use serde_json::{Value, json};

use super::find::goto_page;
use super::host::{DialogHooks, Keyed, Settle, open_with};
use crate::WordApp;

/// The key the dialog is known by.
const KEY: &str = "find";

/// The Find and Replace dialog as the dialog host holds it, under its dialog name.
pub(crate) type FindHost = Keyed<FindReplaceDialog>;

/// Opens the named tab (`find`, `replace`, `goto`); an open dialog switches tab and query and keeps
/// everything else the user typed.
pub(super) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let tab = match name {
        "find" => FindTab::Find,
        "replace" => FindTab::Replace,
        "goto" => FindTab::GoTo,
        _ => return None,
    };
    let query = query(app);
    let id = dialog_id(KEY);
    // PLAN: assumed `Keyed::inner_mut` (dialogs-w13) and `FindReplaceDialog::show_tab` (dialogs-w4, S-L312).
    if let Some(open) = app.dialog_host.get_mut_by_id::<FindHost>(id) {
        open.inner_mut().show_tab(tab, Some(query));
        return Some(id);
    }
    let dialog = build(app, tab, query);
    let keyed = FindHost::new(KEY, dialog);
    // On macOS Find and Replace opens as a utility panel in a window of its own
    // (Services::find_window), titled as in the document window.
    let keyed = if app.services.find_window { keyed.in_own_window(crate::i18n::t("Find and Replace")) } else { keyed };
    Some(open_with(app, parent, keyed, DialogHooks { prepare: Some(prepare), settle }))
}

/// The selection's text when there is one, else the last Find what.
fn query(app: &WordApp) -> String {
    let s = &app.session;
    if s.sel.is_collapsed() { s.find.query.clone() } else { s.selected_text() }
}

/// The Go To kinds; bookmark names starting with `_` are hidden ones.
fn kinds(app: &WordApp) -> Vec<GoToKind> {
    let bookmarks: Vec<String> = app.session.doc.bookmarks().into_iter().map(|(n, _)| n).filter(|n| !n.starts_with('_')).collect();
    vec![
        GoToKind::typed(tl!("Page"), tl!("Type +2 or -1 to move from the page you are on."), true),
        GoToKind::named(tl!("Bookmark"), bookmarks, tl!("This document has no bookmarks.")),
    ]
}

fn build(app: &WordApp, tab: FindTab, query: String) -> FindReplaceDialog {
    let s = &app.session;
    let options = FindOptions {
        match_case: s.find.match_case,
        whole_word: s.find.whole_word,
        prefix: s.find.match_prefix,
        suffix: s.find.match_suffix,
        ignore_punctuation: s.find.ignore_punct,
        ignore_space: s.find.ignore_space,
        wildcards: s.find.wildcards,
        regex: s.find.regex,
        ..FindOptions::default()
    };
    FindReplaceDialog::new(query, tab).word(true).options(options).highlighted(s.find.highlight).go_to_kinds(kinds(app))
}

/// Keeps the bookmark names current while the dialog is open.
fn prepare(app: &mut WordApp, id: DialogId) {
    let kinds = kinds(app);
    // PLAN: assumed `FindReplaceDialog::set_go_to_kinds` (dialogs-w4, S-L312).
    if let Some(d) = app.dialog_host.get_mut_by_id::<FindHost>(id) {
        d.inner_mut().set_go_to_kinds(kinds);
    }
}

fn settle(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let request = match app.dialog_host.get_mut_by_id::<FindHost>(id) {
        Some(d) => d.inner_mut().take_request(),
        None => {
            let _ = app.dialog_host.take_by_id::<Keyed<FindReplaceDialog>>(id);
            return Settle::Done;
        }
    };
    if let Some(request) = request {
        let message = run_request(app, request);
        if let Some(d) = app.dialog_host.get_mut_by_id::<FindHost>(id) {
            d.inner_mut().set_message(message);
        }
    }
    Settle::Keep
}

fn params(query: &str, with: &str, o: &FindOptions) -> Value {
    json!({
        "text": query,
        "with": with,
        "matchCase": o.match_case,
        "wholeWord": o.whole_word,
        "regex": o.regex,
        "wildcards": o.wildcards,
        "matchPrefix": o.prefix,
        "matchSuffix": o.suffix,
        "ignorePunct": o.ignore_punctuation,
        "ignoreSpace": o.ignore_space,
    })
}

/// Runs a request; the line the dialog shows afterwards (empty clears it).
fn run_request(app: &mut WordApp, request: FindRequest) -> String {
    match request {
        FindRequest::FindNext { query, options, .. } => {
            // One step per request: `edit.find` would select a match itself, and the step after it skip one.
            match app.run("edit.findNext", params(&query, "", &options)) {
                Ok(r) => format!("Match {} of {}", r["index"].as_u64().unwrap_or(0) + 1, r["count"]),
                Err(e) => e,
            }
        }
        FindRequest::Replace { query, with, options, .. } => match app.run("edit.replace", params(&query, &with, &options)) {
            Ok(_) => String::new(),
            Err(e) => e,
        },
        FindRequest::ReplaceAll { query, with, options, .. } => match app.run("edit.replaceAll", params(&query, &with, &options)) {
            Ok(r) => format!("All done. We made {} replacements.", r["replaced"]),
            Err(e) => e,
        },
        FindRequest::Highlight { query, options, on, .. } => {
            let mut p = params(&query, "", &options);
            p["highlight"] = json!(on);
            match app.run("edit.advancedFind", p) {
                Ok(r) if on => crate::i18n::fmt(tl!("{count} items highlighted"), &[("count", &r["count"].to_string())]),
                Ok(_) => String::new(),
                Err(e) => e,
            }
        }
        FindRequest::GoTo { kind: 0, target, .. } => match goto_page(&target) {
            Some(p) => go(app, p),
            None => tl!("Type a page number, or +2 / -1.").to_string(),
        },
        FindRequest::GoTo { kind: 1, target, .. } if !target.is_empty() => go(app, json!({"bookmark": target})),
        FindRequest::GoToStep { kind: 0, forward, .. } => go(app, json!({"pageBy": if forward { 1 } else { -1 }})),
        _ => String::new(),
    }
}

fn go(app: &mut WordApp, p: Value) -> String {
    match app.run("edit.goto", p) {
        Ok(_) => String::new(),
        Err(e) => e,
    }
}
