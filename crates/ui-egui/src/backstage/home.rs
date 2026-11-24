//! File › Home and File › Open: the greeting with templates and recent documents, and the Open
//! page's Browse buttons over the same recent list.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, clicked};
use craft::ui::shell::{RecentFile, RecentList, RecentSort, RecentView, StartAction, StartLayout, StartScreen, Template};
use craft::ui::style::DataColor;
use egui::{Color32, Ui};
use serde_json::json;

use super::{HOME_TEMPLATES, TEMPLATES, TemplateThumbs, error_line, heading, label, open_recent, thumbs};
use crate::WordApp;

/// The recent documents as the framework lists them: the app's paths, each named by its file name.
fn recent_files(app: &WordApp) -> Vec<RecentFile> {
    app.ui
        .recent
        .iter()
        .map(|path| {
            let name = std::path::Path::new(path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
            RecentFile::new(path.as_str(), name)
        })
        .collect()
}

/// A template card for each of `list` (display label, `file.new` id), named in the interface language.
pub(super) fn template_cards(list: &[(&str, &str)]) -> Vec<Template> {
    list.iter().map(|(name, id)| Template::new(*id, crate::i18n::tc("template", name).to_string(), "")).collect()
}

/// Runs what the user did on a start screen or recent list. A failed open stays on the page as its
/// error (`BackstageState::open_error`); a later success clears it.
fn run_actions(app: &mut WordApp, actions: Vec<StartAction>) {
    for action in actions {
        match action {
            StartAction::New | StartAction::MoreTemplates => app.ui.backstage_page = "new".into(),
            StartAction::Open => {
                let _ = app.run("ui.openFileDialog", json!({}));
            }
            StartAction::OpenRecent(path) => {
                let failed = open_recent(app, &path.to_string_lossy()).err();
                if failed.is_none() {
                    app.canvas.focus_page_after_pass();
                }
                app.backstage_state.open_error = failed;
            }
            StartAction::RemoveRecent(path) => {
                let _ = app.run("ui.recentRemove", json!({"path": path.to_string_lossy()}));
            }
            StartAction::ClearRecent => {
                let _ = app.run("ui.recentClear", json!({}));
            }
            StartAction::UseTemplate(id) => {
                let _ = app.run("file.new", json!({"template": id}));
                app.ui.backstage = false;
                app.canvas.focus_page_after_pass();
            }
            // Pins, folders and reveal are not part of File.
            _ => {}
        }
    }
}

/// Home: a greeting, Blank plus a short row of templates with a link to New, then Recent.
pub(super) fn home(ui: &mut Ui, app: &mut WordApp) {
    let ctx = ui.ctx().clone();
    let hour = wordcraft_engine::cmd::local_now().3.0;
    let greeting = if hour < 12 {
        "Good morning"
    } else if hour < 18 {
        "Good afternoon"
    } else {
        "Good evening"
    };
    if let Some(error) = app.backstage_state.open_error.clone() {
        error_line(ui, &error);
    }
    let recent = recent_files(app);
    let cards = template_cards(&TEMPLATES[..HOME_TEMPLATES.min(TEMPLATES.len())]);
    let store = thumbs::store(&ctx);
    let region = app.session.region();
    let (wide, high) = thumbs::page_aspect(&region);
    let source = TemplateThumbs { region };
    let (mut view, mut sort) = (RecentView::List, RecentSort::Opened);
    let out = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &recent)
        .layout(StartLayout::Greeting)
        .title(label(greeting))
        .templates(&cards)
        .more_templates(label("More templates →"))
        .max_recent(app.ui.recent_shown())
        .picture_aspect(wide, high)
        .card_zoom(1.0)
        .page_paper(DataColor::new(Color32::WHITE))
        .thumbnails(&store, &source)
        .show(ui);
    run_actions(app, out.actions);
}

/// Open: Browse for a file, then the recent documents.
pub(super) fn open(ui: &mut Ui, app: &mut WordApp) {
    heading(ui, "Open");
    if clicked(&Button::new(label("Browse…")).id_salt("open-browse").show(ui)) {
        let _ = app.run("ui.openFileDialog", json!({}));
    }
    // Linux and the BSDs (#576): the system dialog needs a file chooser portal or zenity; when it
    // doesn't show up, WordCraft's own browser is always one click away.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    if clicked(
        &Button::new(label("Browse in WordCraft…"))
            .id_salt("open-browse-own")
            .tooltip(tl!("Open a file with WordCraft's own browser when the system file dialog doesn't appear.").to_owned())
            .show(ui),
    ) {
        let _ = app.run("ui.fileBrowser", json!({}));
    }
    if let Some(error) = app.backstage_state.open_error.clone() {
        error_line(ui, &error);
    }
    let recent = recent_files(app);
    let out = RecentList::new(&recent).max_recent(app.ui.recent_shown()).empty(label("Documents you open will show up here.")).show(ui);
    run_actions(app, out.actions);
}
