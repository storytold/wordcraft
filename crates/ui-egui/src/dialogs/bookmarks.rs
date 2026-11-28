use std::sync::LazyLock;

use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, FieldLayout, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::{
    ActionEnabled, FormCx, ListManager, ListManagerAction, ListManagerChoice, ListManagerEntry, ListManagerWords,
};
use craft::ui::pickers::{IconPicker, IconPickerGroup, IconPickerGroups, IconPickerItem};
use craft::ui::primitives::{Glyph, IconSpec};
use craft::ui::runtime::env::{Env, Label};
use serde::{Deserialize, Serialize};
use wordcraft_render::icon_lib::{self, Category};

use super::host::{Asks, DialogHooks, Settle, WordForm, open_form, open_with};
use super::{Value, WordApp};
use egui::Ui;
use serde_json::json;

/// The innermost bookmark around the caret, which the Bookmark dialog opens on (as Word does);
/// empty when the caret is in none.
pub(super) fn bookmark_at_caret(s: &wordcraft_engine::Session) -> String {
    let at = &s.sel.focus;
    s.doc
        .bookmarks()
        .into_iter()
        .rev()
        .find(|(name, start)| !name.starts_with('_') && start <= at && *at <= wordcraft_engine::cmd::insert::bookmark_span(&s.doc, name, start).1)
        .map(|(name, _)| name)
        .unwrap_or_default()
}

/// The bookmarks the Bookmark dialog lists: by name or in document order, the hidden ones
/// (`_Toc…`, `_Ref…`) only when asked for.
pub(super) fn bookmark_names(doc: &wordcraft_doc::Document, by_location: bool, hidden: bool) -> Vec<String> {
    let mut names: Vec<String> = doc.bookmarks().into_iter().map(|(n, _)| n).filter(|n| hidden || !n.starts_with('_')).collect();
    if !by_location {
        names.sort_by_key(|n| n.to_lowercase());
    }
    names
}

/// Insert › Bookmark as Word lays it out: the name box over the list, Add / Delete / Go To beside
/// them, Sort by and Hidden bookmarks below. Add closes the dialog; Delete and Go To leave it open,
/// and once a bookmark is deleted Cancel reads Close. The list names come from the session each
/// pass (`prepare`); the dialog keeps only the user's choices.
pub(crate) struct BookmarkDialog {
    /// The name box's text: it opens on the bookmark around the caret, and an outside OK (`confirm`) adds it.
    name: String,
    by_location: bool,
    hidden: bool,
    /// The names listed, refreshed from the session before every pass.
    names: Vec<String>,
    deleted: bool,
    /// A Delete or Go To pressed in the last pass, run by `settle`.
    pending: Option<ListManagerChoice>,
}

impl BookmarkDialog {
    fn actions(words: &ListManagerWords) -> Vec<ListManagerAction> {
        let add = ListManagerAction::new("add", Label::from(tl!("Add")), ActionEnabled::OnValidName);
        let delete = ListManagerAction::delete(words).keeps_open();
        // `ActionEnabled::OnEntry` is non-exhaustive outside the framework, so Go To starts from Edit's enabling.
        let mut go_to = ListManagerAction::edit(words).keeps_open();
        go_to.id = "goTo".into();
        go_to.label = Label::from(tl!("Go To"));
        vec![add, delete, go_to]
    }
}

impl StackDialog for BookmarkDialog {
    /// The name Add closed the dialog with, `None` for Cancel.
    type Result = Option<String>;

    fn id(&self) -> DialogId {
        dialog_id("bookmark")
    }

    fn key(&self) -> Option<&str> {
        Some("bookmark")
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"name": self.name}))
    }

    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        match (field, value.as_str()) {
            ("name", Some(name)) => {
                self.name = name.to_owned();
                Ok(())
            }
            ("name", None) => Err(FieldRefusal::Rejects(&value).message(field)),
            _ => Err(FieldRefusal::Unknown.message(field)),
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Self::Result> {
        let name = self.name.trim();
        if !wordcraft_engine::cmd::insert::bookmark_name_ok(name) {
            return None;
        }
        Some(Some(name.to_string()))
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<ListManagerWords>();
        let entries: Vec<ListManagerEntry> = self.names.iter().map(|n| ListManagerEntry::new(n.clone(), n.clone())).collect();
        let (by_location, hidden, name) = (&mut self.by_location, &mut self.hidden, &mut self.name);
        let mut list = ListManager::new(Label::from(tl!("Bookmark")), Label::from(tl!("Bookmark name:")), entries, Self::actions(&words))
            .empty(Label::from(tl!("The document has no bookmarks yet.")))
            .name_box(Bind::bound(name))
            .name_check(|n| wordcraft_engine::cmd::insert::bookmark_name_ok(n.trim()))
            .under_list(move |ui| {
                RadioGroup::new(Label::from(tl!("Sort by:")), Bind::bound(&mut *by_location)).show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, cx| {
                        for (at, (value, label)) in [(false, "Name"), (true, "Location")].into_iter().enumerate() {
                            let side = SideLabel::new().layout(FieldLayout::After);
                            cx.labelled(ui, at, Label::from(tl!(label)), side, Radio::new(value));
                        }
                    });
                });
                Checkbox::new(Bind::bound(&mut *hidden)).label(Label::from(tl!("Hidden bookmarks"))).show(ui);
            });
        if self.deleted {
            list = list.cancel_label(Label::from(tl!("Close")));
        }
        let out = list.show(ui);
        if out.closed.is_some() {
            return Some(DialogResult::Ok(out.choice.map(|choice| choice.name)));
        }
        if let Some(choice) = out.choice {
            self.pending = Some(choice);
        }
        None
    }
}

/// Runs what the dialog's last pass chose. Add arrives as the dialog's result because the dialog
/// closes with it; Delete and Go To are read from the open dialog.
fn settle_bookmark(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(dialog) = app.dialog_host.get_mut_by_id::<BookmarkDialog>(id) else {
        if let Some(DialogResult::Ok(Some(name))) = app.dialog_host.take_by_id::<BookmarkDialog>(id) {
            let _ = app.run("insert.bookmark", json!({"name": name}));
        }
        return Settle::Done;
    };
    let Some(choice) = dialog.pending.take() else {
        return Settle::Keep;
    };
    match choice.action.as_str() {
        "delete" => {
            if app.run("insert.deleteBookmark", json!({"name": choice.name})).is_ok()
                && let Some(dialog) = app.dialog_host.get_mut_by_id::<BookmarkDialog>(id)
            {
                dialog.deleted = true;
                // The deleted name leaves the box, as in Word.
                dialog.name.clear();
            }
        }
        "goTo" => {
            let _ = app.run("edit.goto", json!({"bookmark": choice.name}));
        }
        _ => {}
    }
    Settle::Keep
}

/// Refreshes the listed names from the session before the pass.
fn prepare_bookmark(app: &mut WordApp, id: DialogId) {
    let Some(dialog) = app.dialog_host.get_by_id::<BookmarkDialog>(id) else {
        return;
    };
    let names = bookmark_names(&app.session.doc, dialog.by_location, dialog.hidden);
    if let Some(dialog) = app.dialog_host.get_mut_by_id::<BookmarkDialog>(id) {
        dialog.names = names;
    }
}

/// Opens Insert › Bookmark on the bookmark around the caret.
fn open_bookmark(app: &mut WordApp, parent: Option<DialogId>) -> DialogId {
    let dialog = BookmarkDialog {
        name: bookmark_at_caret(&app.session),
        by_location: false,
        hidden: false,
        names: bookmark_names(&app.session.doc, false, false),
        deleted: false,
        pending: None,
    };
    open_with(app, parent, dialog, DialogHooks { prepare: Some(prepare_bookmark), settle: settle_bookmark })
}

/// The library icons' glyphs, built once: an item borrows its glyph for `'static`.
static GLYPHS: LazyLock<Vec<Box<dyn Glyph + Send>>> =
    LazyLock::new(|| icon_lib::ICONS.iter().map(|icon| Box::new(crate::icons::library_glyph(icon)) as Box<dyn Glyph + Send>).collect());

/// The picker's groups: one per library category, in the library's order.
static ICON_GROUPS: LazyLock<Vec<IconPickerGroup<String>>> = LazyLock::new(|| {
    Category::ALL
        .into_iter()
        .map(|category| {
            let items = icon_lib::ICONS
                .iter()
                .enumerate()
                .filter(|(_, icon)| icon.category == category)
                .filter_map(|(at, icon)| {
                    let glyph: &'static dyn Glyph = &**GLYPHS.get(at)?;
                    let keywords = icon.keywords.iter().map(|k| (*k).to_string()).collect();
                    Some(IconPickerItem::new(icon.id.to_string(), icon.name).keywords(keywords).icon(IconSpec::glyph(glyph)))
                })
                .collect();
            IconPickerGroup::new(category.name(), items)
        })
        .collect()
});

thread_local! {
    // The groups titled in the interface language they were made for (the library's category names are English).
    static TRANSLATED_GROUPS: std::cell::RefCell<Option<(&'static str, std::rc::Rc<Vec<IconPickerGroup<String>>>)>> =
        const { std::cell::RefCell::new(None) };
}

/// [`ICON_GROUPS`] with each category's name in the interface language, made again when the language changes.
fn translated_groups() -> std::rc::Rc<Vec<IconPickerGroup<String>>> {
    let lang = crate::i18n::current().code();
    TRANSLATED_GROUPS.with_borrow_mut(|cached| match cached {
        Some((made_for, groups)) if *made_for == lang => std::rc::Rc::clone(groups),
        _ => {
            let groups: Vec<IconPickerGroup<String>> =
                ICON_GROUPS.iter().map(|g| IconPickerGroup::new(tl!(g.name.as_str()).to_string(), g.items.clone())).collect();
            let groups = std::rc::Rc::new(groups);
            *cached = Some((lang, std::rc::Rc::clone(&groups)));
            groups
        }
    })
}

/// Insert › Icons: the library as a searchable grid with a filter row; Insert runs `insert.icon`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct IconsForm {
    /// The id of the picked icon.
    picked: Option<String>,
}

impl WordForm for IconsForm {
    fn key(&self) -> &'static str {
        "icons"
    }

    fn title(&self) -> Label {
        Label::from(tl!("Icons"))
    }

    fn ok_label(&self) -> Option<Label> {
        Some(Label::from(tl!("Insert")))
    }

    fn collection_body(&self) -> bool {
        // The picker's grid owns the scroll; a scrolling dialog body under it would be a second owner.
        true
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
        let groups = translated_groups();
        let out = IconPicker::new(Label::from(tl!("Icons")), Bind::bound(&mut self.picked), groups.as_slice())
            .group_display(IconPickerGroups::Filter)
            .empty(Label::from(tl!("No icons match your search.")))
            .show(ui);
        if out.picked.is_some() && out.response.double_clicked() {
            // PLAN: assumed FormCx::accept (dialogs-w5, S-L285)
            cx.accept();
        }
        self.picked.is_some()
    }

    fn commands(&self) -> Vec<(String, Value)> {
        self.picked.iter().map(|id| ("insert.icon".to_string(), json!({"id": id}))).collect()
    }
}

/// Opens Insert › Bookmark (`bookmark`) or Insert › Icons (`icons`).
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        "bookmark" => Some(open_bookmark(app, parent)),
        "icons" => Some(open_form(app, parent, IconsForm::default())),
        _ => None,
    }
}
