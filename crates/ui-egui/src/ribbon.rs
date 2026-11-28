//! The ribbon: tab strip and every tab's groups. Each control runs a command by id.

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::ribbon::RibbonLook;
use craft::menu::{Ribbon, RibbonOutput, RibbonRow, RibbonTab};
use craft::theme::Variant;
use craft::ui::contract::{Bind, Entry};
use craft::ui::primitives::IconSpec;
use craft::ui::tabs::StripEnd;
use egui::Ui;
use serde_json::json;

use crate::WordApp;

mod arrange;
mod charts;
pub(crate) mod common;
mod context_menu;
mod design;
mod draw;
mod galleries;
mod home;
mod insert;
mod language;
mod layout;
mod mailings;
mod picture;
mod references;
mod review;
mod shape;
mod table;
mod tabs;
mod underline;
mod view;

pub(crate) use arrange::wrap_text_items;
use charts::{chart_design, smart_art_design};
use common::custom_groups;
pub(crate) use common::weight_menu;
pub(crate) use context_menu::command_menu;
use design::design;
use draw::draw;
use home::home;
use insert::insert;
#[cfg(test)]
pub(crate) use language::PROOFING_LANGUAGES;
#[cfg(test)]
pub(crate) use language::all_languages;
#[cfg(test)]
pub(crate) use language::short_languages;
pub(crate) use language::sort_key;
pub(crate) use language::{
    caret_language, caret_language_proofed, caret_missing_dictionary, caret_no_proof, caret_unreadable_dictionary, language_menu,
};
pub(crate) use language::{display_name, language_list, language_name, same_language, tag_for_pack};
use layout::layout;
use mailings::mailings;
use picture::picture_format;
use references::{references, zotero};
use review::{developer, help, review};
use shape::shape_format;
use table::{table_design, table_layout};
use tabs::reading_toolbar;
pub use tabs::{
    TABS, contextual_tabs, has_chart_selected, has_picture_selected, has_shape_selected, has_smart_art_selected, in_table_public, resolve_tab,
};
use view::{outlining, view};

/// What a pass with no ribbon reports.
pub(crate) fn no_ribbon() -> RibbonOutput<String> {
    // PLAN: assumed `RibbonOutput::tab_ids` (S-L361) has a `Default` value; the field replaces `menu`.
    RibbonOutput { shown: None, picked: None, controls: None, tabs: None, tab_ids: Default::default() }
}

/// How the ribbon buttons look, in points: the old ribbon drew its large icons 32
/// points square and the rest 17, its menu arrows as filled triangles in 10 points, and its small
/// rows 22 points apart (20-point buttons a tight gap apart); its tabs rest in the text colour
/// over a 3-point underline; its body stands on its own lighter fill, ruled above and below.
const RIBBON_LOOK: RibbonLook = RibbonLook::new(32, 17).arrow(10).row(20).tabs(3).body();

/// Draws the Read toolbar or the full ribbon, switching to and from the contextual Equation and
/// Outlining tabs and showing the available tab strip. The output carries the record of the ribbon's
/// controls for the keytips.
pub fn show(app: &mut WordApp, ui: &mut Ui) -> RibbonOutput<String> {
    if app.session.view.read_mode {
        reading_toolbar(app, ui);
        return no_ribbon();
    }
    // Editing an equation brings up the Equation tab; leaving it goes back.
    if app.session.math.is_some() {
        if app.equation_prev_tab.is_none() {
            app.equation_prev_tab = Some(app.ui.tab.clone());
            app.ui.tab = "Equation".into();
        }
    } else if let Some(prev) = app.equation_prev_tab.take()
        && app.ui.tab == "Equation"
    {
        app.ui.tab = prev;
    }
    // Outline view brings up the Outlining tab; closing it goes back.
    if app.session.view.mode == wordcraft_layout::ViewMode::Outline {
        if app.outline_prev_tab.is_none() {
            app.outline_prev_tab = Some(app.ui.tab.clone());
            app.ui.tab = "Outlining".into();
        }
    } else if let Some(prev) = app.outline_prev_tab.take()
        && app.ui.tab == "Outlining"
    {
        app.ui.tab = prev;
    }
    // The tabs shown: File, the user's tabs in their order (Customize Ribbon, #379), then the
    // contextual ones (the Outlining tab right after File). A contextual tab (Table, Picture
    // Format) may be stored while the selection moved away, and a tab may have been hidden.
    let user_tabs = app.session.ribbon.tabs().into_owned();
    let mut names: Vec<(String, bool)> = vec![("File".to_string(), false)];
    names.extend(user_tabs.iter().filter(|e| !e.hidden).map(|e| (e.name.clone(), e.custom)));
    for ct in contextual_tabs(&app.session) {
        if ct == "Outlining" {
            names.insert(1.min(names.len()), (ct.to_string(), false));
        } else if !names.iter().any(|(n, _)| n == ct) {
            names.push((ct.to_string(), false));
        }
    }
    {
        let listed: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
        let next = resolve_tab(&app.ui.tab, &listed);
        let next = if listed.contains(&next) { next } else { listed.iter().copied().find(|n| *n != "File").unwrap_or(next) };
        if next != app.ui.tab {
            app.ui.tab = next.to_string();
        }
    }
    let tabs: Vec<RibbonTab<String>> = names
        .iter()
        .map(|(name, custom)| {
            // A custom tab's name is the user's own text: shown as typed.
            let label = if *custom { name.clone() } else { tl!(name.as_str()).to_string() };
            let tab = RibbonTab::new(name.clone(), label);
            let contextual = wordcraft_engine::ribbon::CONTEXTUAL_TABS.contains(&name.as_str()) && !user_tabs.iter().any(|e| e.name == *name);
            if contextual { tab.contextual() } else { tab }
        })
        .collect();

    let track = app.session.doc.settings.track_changes;
    let editing_label = if track { tl!("Reviewing") } else { tl!("Editing") };
    let modes = [
        Entry::new(CommandRef::with_params("review.trackChanges", json!({"value": false})), tl!("Editing — edit the document directly").to_string())
            .checked(!track),
        Entry::new(CommandRef::with_params("review.trackChanges", json!({"value": true})), tl!("Reviewing — edits become suggestions").to_string())
            .checked(track),
    ];
    let ends = [
        // The strip lays these out start to end, so Share, the old strip's outermost button, comes last. Comments and
        // Editing are plain (ghost) buttons, as in v0.18.1; Share is a filled accent button.
        StripEnd::command(tl!("Comments").to_string(), IconSpec::svg(IconId::new("comment")), CommandRef::new("view.commentsPane")),
        StripEnd::menu(editing_label.to_string(), IconSpec::svg(IconId::new("wordcraft.pencil")), &modes),
        StripEnd::command(
            tl!("Share").to_string(),
            IconSpec::svg(IconId::new("share")),
            CommandRef::with_params("ui.backstage", json!({"value": true, "page": "export"})),
        )
        .variant(Variant::PRIMARY),
    ];
    // The context menu's body captures these, never `app`, which the row body borrows.
    let qat: Vec<String> = app.session.ribbon.qat().into_iter().map(str::to_string).collect();
    let qat_shown = app.ui.quick_access_toolbar;
    // The collapse state is written through `ui.collapseRibbon`, the one write path, when the strip toggles it.
    let mut collapsed = app.ui.ribbon_collapsed;
    let out = Ribbon::new(tl!("Ribbon").to_string(), &tabs, Bind::given(app.ui.tab.clone()))
        .trailing(&ends)
        .minimised(Bind::bound(&mut collapsed))
        .context_menu(&mut |ui, request, _| context_menu::rows(ui, request, &qat, qat_shown))
        // The old ribbon drew its large icons 32 points square and every other icon 17, and its
        // dialog launcher as a small corner arrow.
        .look(RIBBON_LOOK)
        .launcher_icon(IconId::new("launcher"))
        .show(ui, |row, key| {
            let entry = user_tabs.iter().find(|e| e.name == *key);
            match entry {
                // Every tab hidden: nothing to show.
                Some(e) if e.hidden => return,
                Some(e) if e.custom => return custom_groups(row, app, &e.groups),
                _ => {}
            }
            tab_body(row, app, key.as_str());
            if let Some(e) = entry {
                custom_groups(row, app, &e.groups);
            }
        });
    if collapsed != app.ui.ribbon_collapsed {
        let _ = app.run("ui.collapseRibbon", json!({}));
    }
    // A click on a tab is reported as `shown`, a pick from the overflow menu as `picked`.
    if let Some(tab) = out.picked.clone().or_else(|| out.shown.clone())
        && tab != app.ui.tab
    {
        let _ = app.run("ui.tab", json!({"tab": tab}));
        // A click on a tab name keeps typing in the document (#1093): the press moves focus off the page when the pass
        // ends, so the page takes the keyboard back after it. A tab reached with the keys keeps it, and File's page has
        // its own focus.
        let by_key = ui.ctx().input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. })));
        if !by_key && !app.ui.backstage {
            app.canvas.focus_page_after_pass();
        }
    }
    out
}

/// The groups of the built-in tab `tab`.
// PLAN: the tab functions (`home`, `insert`, ..., `equation_tab::show`, `custom_groups`) still take `(app, ui)` at the
// base; their owners (ribbon-w9, w15-w19) port them to `(row, app)`, which is what is called here. `RibbonRow` has no
// `Ui` accessor, so an `(app, ui)` call cannot be made from the row. `mod galleries;` is ribbon-w12's file and
// `widgets/font_box.rs` is ribbon-w11's.
fn tab_body(row: &mut RibbonRow<'_>, app: &mut WordApp, tab: &str) {
    match tab {
        "Home" => home(row, app),
        "Insert" => insert(row, app),
        "Draw" => draw(row, app),
        "Design" => design(row, app),
        "Layout" => layout(row, app),
        "References" => references(row, app),
        "Mailings" => mailings(row, app),
        "Review" => review(row, app),
        "View" => view(row, app),
        "Developer" => developer(row, app),
        "Citations" => zotero(row, app),
        "Help" => help(row, app),
        "Table Design" => table_design(row, app),
        "Table Layout" => table_layout(row, app),
        "Equation" if app.session.math.is_some() => crate::equation_tab::show(row, app),
        "Picture Format" => picture_format(row, app),
        "Shape Format" => shape_format(row, app),
        "SmartArt Design" => smart_art_design(row, app),
        "Outlining" => outlining(row, app),
        "Chart Design" => chart_design(row, app),
        _ => home(row, app),
    }
}

#[cfg(test)]
mod tests;
