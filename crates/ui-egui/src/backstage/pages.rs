//! The File pages with a form or a list of their own: Info, Save As, Export (and Print) and Recover.

use craft::i18n::Direction;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _, Sizing, clicked};
use craft::ui::display::Text;
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::json;

use super::{error_line, heading, label, subheading};
use crate::WordApp;

/// Recovery is a separate copy. Leaving this page never discards it.
pub(super) fn recovery(ui: &mut Ui, app: &mut WordApp) {
    heading(ui, "Recover documents");
    let _ = Text::new(label("Restore opens an unsaved copy. Your original file stays unchanged.")).show(ui);
    if app.session.password.is_some() {
        let _ = Text::new(label("Recovery is disabled while this document has a password.")).show(ui);
    }
    if let Some(error) = app.session.recovery_error() {
        error_line(ui, error);
    }
    if !app.session.recovery_ready() {
        let _ = Text::new(label("Local recovery is unavailable or still loading.")).show(ui);
    }
    let documents = app.session.recovery_documents();
    if documents.is_empty() && app.session.recovery_ready() {
        let _ = Text::new(label("No recovery copies.")).show(ui);
    }
    let busy = app.session.recovery_busy();
    for (at, doc) in documents.iter().enumerate() {
        let _ = Row::new().gap(Gap::Section).show(ui, |ui, _| {
            let _ = Text::new(doc.name.clone()).show(ui);
            let _ = Text::new(doc.saved_at.clone()).show(ui);
            if clicked(&Button::new(label("Restore")).id_salt(("restore", at)).enabled(!busy).show(ui)) {
                let _ = app.run("file.recover", json!({"restore": doc.id}));
            }
            if clicked(&Button::new(label("Discard…")).id_salt(("discard", at)).enabled(!busy).show(ui)) {
                let _ = app.run("ui.recoveryDiscard", json!({"id": doc.id}));
            }
        });
    }
    let _ = Row::new().gap(Gap::Tight).show(ui, |ui, _| {
        if clicked(&Button::new(label("Continue without restoring")).id_salt("recover-continue").show(ui)) {
            let _ = app.run("ui.backstage", json!({"value": false}));
            app.canvas.focus_page_after_pass();
        }
        if clicked(&Button::new(label("Refresh")).id_salt("recover-refresh").enabled(!busy && app.session.recovery_ready()).show(ui)) {
            let _ = app.run("file.recover", json!({"refresh": true}));
        }
    });
}

/// Save As: Browse… asks where to save. It has no recent-documents list: clicking one of those
/// opens it, which is not what Save As is for.
pub(super) fn save_as(ui: &mut Ui, app: &mut WordApp) {
    heading(ui, "Save As");
    if let Some(error) = app.backstage_state.save_as_error.clone() {
        error_line(ui, &error);
    }
    if clicked(&Button::new(label("Browse…")).id_salt("save-as-browse").show(ui)) {
        app.save_as_dialog();
    }
    // Linux and the BSDs (#3535): WordCraft's own browser when the system dialog doesn't appear.
    #[cfg(all(unix, not(target_os = "macos"), not(target_arch = "wasm32")))]
    if clicked(&Button::new(label("Browse in WordCraft…")).id_salt("save-as-browse-own").show(ui)) {
        let _ = app.run("ui.fileBrowser", json!({"purpose": "save"}));
    }
    // A Word template in the user templates folder, which File › New lists under Personal (#2852).
    if clicked(
        &Button::new(label("Save as Word Template…"))
            .id_salt("save-as-template")
            .tooltip(tl!("Saves a .dotx template in your templates folder. It shows up in File › New under Personal.").to_owned())
            .show(ui),
    ) {
        let _ = app.run("ui.saveAsTemplate", json!({}));
    }
}

/// Info: the document's properties beside its statistics.
pub(super) fn info(ui: &mut Ui, app: &mut WordApp) {
    heading(ui, "Info");
    let info = app.session.run("file.info", &json!({})).unwrap_or_default();
    // The row starts at the reading start, so Properties are at the right in a mirrored interface.
    // Two equal columns across the page, as production's `ui.columns(2)`.
    let _ = Row::new().fill_width().gap(Gap::Section).show(ui, |ui, _| {
        // Protect Document stands a section apart from the properties above it (production's 18 pt).
        let _ = Stack::new().fill_width().gap(Gap::Section).show(ui, |ui, _| {
            let _ = Stack::new().fill_width().gap(Gap::Tight).show(ui, |ui, _| properties(ui, app, &info));
            let _ = Stack::new().fill_width().gap(Gap::Tight).show(ui, |ui, _| protect(ui, app, &info));
        });
        let _ = Stack::new().fill_width().gap(Gap::Tight).show(ui, |ui, _| statistics(ui, &info));
    });
}

/// The editable properties and the password controls.
fn properties(ui: &mut Ui, app: &mut WordApp, _info: &serde_json::Value) {
    subheading(ui, "Properties");
    // The fields edit a copy of the document's properties that is read afresh every pass, so each
    // change goes into the document as it is typed (#260): waiting for the field to lose focus
    // dropped the keystrokes, and leaving File with Escape never loses it at all. The keystrokes
    // of one visit to a field are a single undo step.
    let mut props = app.session.doc.core.clone();
    let mut edited = false;
    // A typing visit to a field is one gesture: its `Begin` starts an undo step and the updates
    // after it join that step. A value that arrives whole (`Set`) is a step of its own.
    let mut join = false;
    // PLAN: the `info_editing` field of `WordApp`/`WindowUi` (lib.rs, window_ui.rs, app.rs; app-hub) is unused now and goes.
    // Each field shows its name beside it, the names in one column so the fields line up (production's two-column table).
    let _ = FieldGroup::new(label("Properties")).show(ui, |ui, _| {
        for (name, value) in [
            ("Title", &mut props.title),
            ("Subject", &mut props.subject),
            ("Author", &mut props.creator),
            ("Keywords", &mut props.keywords),
            ("Category", &mut props.category),
        ] {
            let _ = Field::new(label(name)).show(ui, |ui, _| {
                let out = TextField::new(label(name), Bind::bound(&mut *value)).id_salt(("info-prop", name)).show(ui);
                if out.value().is_some() {
                    edited = true;
                    join = !out.events.began() && out.events.ended().is_none();
                }
            });
        }
    });
    if edited {
        if join {
            app.session.join_next_undo();
        }
        let _ = app.run(
            "file.properties",
            json!({"title": props.title, "subject": props.subject, "author": props.creator, "keywords": props.keywords, "category": props.category}),
        );
    }
    // Advanced Properties › Custom (#2982): view and edit the document's custom properties.
    if clicked(
        &Button::new(label("Advanced Properties…"))
            .id_salt("info-advanced")
            // Across the column, as production's button in its justified column.
            .sizing(Sizing::FillWidth)
            .tooltip(tl!("View and edit the document's custom properties.").to_owned())
            .show(ui),
    ) {
        let _ = app.run("ui.dialog", json!({"name": "customProperties"}));
    }
}

/// Protect Document › Encrypt with Password (#55): whether a password guards the document, and the buttons that set
/// or remove it.
fn protect(ui: &mut Ui, app: &mut WordApp, info: &serde_json::Value) {
    subheading(ui, "Protect Document");
    let encrypted = info.get("encrypted").and_then(|v| v.as_bool()).unwrap_or(false);
    let _ = Text::new(if encrypted {
        label("A password is required to open this document.")
    } else {
        label("Anyone can open this document. Encrypt it with a password to keep it private.")
    })
    .show(ui);
    let _ = Row::new().gap(Gap::Tight).show(ui, |ui, _| {
        let name = if encrypted { label("Change Password…") } else { label("Encrypt with Password…") };
        if clicked(&Button::new(name).id_salt("info-encrypt").show(ui)) {
            let _ = app.run("file.encrypt", json!({}));
        }
        if encrypted && clicked(&Button::new(label("Remove Password")).id_salt("info-unencrypt").show(ui)) {
            let _ = app.run("file.encrypt", json!({"password": null}));
        }
    });
}

/// The document's counts and its location.
fn statistics(ui: &mut Ui, info: &serde_json::Value) {
    subheading(ui, "Statistics");
    for (name, key) in [("Pages", "pages"), ("Words", "words"), ("Paragraphs", "paragraphs"), ("Sections", "sections"), ("Comments", "comments")] {
        let _ = Text::new(format!("{}: {}", tl!(name), info.get(key).map(|v| v.to_string()).unwrap_or_default())).show(ui);
    }
    let path = info.get("path").and_then(|v| v.as_str()).unwrap_or(tl!("Not saved yet"));
    // A path reads left to right whatever its folders are called, so it is set apart from the words
    // around it; a left-to-right interface gets it unchanged.
    let direction = if crate::i18n::current().rtl() { Direction::Rtl } else { Direction::Ltr };
    let shown = craft::i18n::path(path, direction);
    let _ = Text::new(tl!("Location: {path}").replace("{path}", &shown)).show(ui);
}

/// Export, and the Print page that shares its function: formats to save a copy in, or the print
/// settings with their preview.
pub(super) fn export(ui: &mut Ui, app: &mut WordApp) {
    if app.ui.backstage_page == "print" {
        heading(ui, "Print");
        // Print Markup (a print setting, as in Word): the comments beside each page.
        let mut markup = app.session.prefs.print_markup;
        let out = Checkbox::new(Bind::bound(&mut markup))
            .label(label("Print Markup"))
            .tooltip(
                tl!("Comments beside each page and tracked changes in PDFs, prints and images; a comment list at the end of web pages, Markdown and text.")
                    .to_owned(),
            )
            .show(ui);
        if out.value().is_some() {
            let _ = app.run("file.printMarkup", json!({"value": markup}));
        }
        crate::print_page::show(app, ui);
        return;
    }
    heading(ui, "Export");
    let _ = Text::new(label("Save a copy in another format. Printing goes through a PDF you can print from any viewer.")).show(ui);
    crate::print_page::export_formats(app, ui);
}
