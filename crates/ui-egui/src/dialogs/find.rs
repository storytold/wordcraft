use serde_json::{Value, json};

use crate::WordApp;

/// The commands no search lists: the language-pack commands act on a pack the user picks in
/// Language settings, so a search hit would run them with nothing to act on.
pub(crate) const NOT_IN_SEARCH: [&str; 2] = ["proof.installPack", "proof.removePack"];

/// What Search lists before anything is typed: common commands (#819).
pub(crate) const SEARCH_SUGGESTIONS: [&str; 3] = ["review.spelling", "review.wordCount", "file.print"];

/// Search's "Find … in the document": the Navigation pane opens on its results for `text`.
pub(crate) fn find_in_navigation_pane(app: &mut WordApp, ctx: &egui::Context, text: &str) {
    let qid = app.window_id("nav_query");
    ctx.data_mut(|d| d.insert_temp(qid, text.to_string()));
    app.ui.nav_tab = "results".into();
    let _ = app.run("view.navigationPane", json!({"value": true}));
    let _ = app.run("edit.find", json!({"text": text}));
}

/// Go To's page box as `edit.goto` params: empty steps one page on, `+n` / `-n` move from the
/// current page, a number is that page. `None` for anything else.
pub(super) fn goto_page(text: &str) -> Option<Value> {
    let t = text.trim().replace('\u{2212}', "-");
    if t.is_empty() {
        return Some(json!({"pageBy": 1}));
    }
    if let Some(rest) = t.strip_prefix('+') {
        return rest.parse::<i64>().ok().filter(|n| *n >= 0).map(|n| json!({"pageBy": n}));
    }
    if t.starts_with('-') {
        return t.parse::<i64>().ok().map(|n| json!({"pageBy": n}));
    }
    t.parse::<u64>().ok().filter(|n| *n >= 1).map(|n| json!({"page": n}))
}
