//! Manage Styles: every style with its preview and description; modify, create, delete and show or hide them.

use craft::behavior::{ColumnKey, ColumnWidth, Selection, SelectionChange, item_source::keyed};
use craft::theme::Mode;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, TooltipSpec, clicked,
};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Picture, Shade, Text};
use craft::ui::form::Field;
use craft::ui::list::{List, ListRow};
use craft::ui::pickers::Select;
use craft::ui::primitives::textures::Revision;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::{DialogRole, FooterButton};
use egui::{ColorImage, Context, Ui, Vec2, vec2};
use serde_json::{Value, json};

use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

/// The dialog's key, which is also the name `dialogs::open` knows it by.
const KEY: &str = "manageStyles";

/// The style list's one column, which holds the style's name.
const NAME: ColumnKey = ColumnKey::new("name");

/// What a preview image was made for: the style, its type, the styles' revision, the room, the
/// interface mode and the pixel density.
#[derive(Clone, PartialEq)]
struct PreviewKey {
    selected: String,
    ty: String,
    styles: Revision,
    room: [u32; 2],
    mode: Mode,
    ppp: u32,
}

/// A label from text the app words.
pub(super) fn label(text: impl std::fmt::Display) -> Label {
    Label::from(text.to_string())
}

/// A button that says why it is disabled; true when it was clicked.
pub(super) fn action(ui: &mut Ui, salt: &str, text: Label, enabled: bool, reason: Option<Label>) -> bool {
    let mut button = Button::new(text.clone()).id_salt(salt);
    if let (false, Some(reason)) = (enabled, reason) {
        button = button.tooltip(TooltipSpec::new(text).disabled_text(reason));
    }
    clicked(&button.enabled(enabled).show(ui))
}

/// What the dialog asks the app to do; the settle hook runs them, since the dialog holds no app.
enum Request {
    /// `styles.setVisibility` with the one flag that changed.
    Visibility {
        style: String,
        gallery: Option<bool>,
        hidden: Option<bool>,
    },
    Delete(String),
    Modify(String),
    /// New Style, based on the style of this name.
    New(String),
}

/// Manage Styles on the dialog stack.
pub(crate) struct ManageStylesDialog {
    id: DialogId,
    alphabetical: bool,
    /// The selected style's id.
    selected: String,
    /// The `styles.manage` list in the dialog's order, refreshed by `prepare`.
    styles: Vec<Value>,
    /// The preview image and the key it was made for.
    preview: Option<(PreviewKey, ColorImage)>,
    /// How many preview images were made: the picture's key, so a new image is a new texture.
    made: u64,
    /// The room and interface mode the body saw in its last pass; `settle` renders the image for them.
    room: Option<(Vec2, Mode)>,
    requests: Vec<Request>,
    /// The selection the list last scrolled to.
    revealed: String,
}

fn text_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn flag(v: &Value, k: &str, default: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(default)
}

/// Opens Manage Styles with `params["style"]` selected (the first style when none); `None` for another name.
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    if name != KEY {
        return None;
    }
    let dialog = ManageStylesDialog {
        id: dialog_id(KEY),
        alphabetical: false,
        selected: params.get("style").and_then(Value::as_str).unwrap_or_default().to_string(),
        styles: Vec::new(),
        preview: None,
        made: 0,
        room: None,
        requests: Vec::new(),
        revealed: String::new(),
    };
    Some(open_with(app, parent, dialog, DialogHooks { prepare: Some(prepare), settle }))
}

impl ManageStylesDialog {
    /// Selects the style `id`: a style a child dialog just created.
    pub(super) fn select(&mut self, id: String) {
        self.selected = id;
    }

    fn current(&self) -> Option<&Value> {
        self.styles.iter().find(|x| text_of(x, "id") == self.selected)
    }

    fn body(&mut self, ui: &mut Ui) {
        let sort = [Entry::new(false, label(tl!("As Recommended"))), Entry::new(true, label(tl!("Alphabetical")))];
        let picked = Field::new(label(tl!("Sort order:")))
            .show(ui, |ui, _| Select::new(label(tl!("Sort order:")), Bind::given(self.alphabetical), &sort).show(ui).value().copied())
            .inner
            .flatten();
        if let Some(alphabetical) = picked {
            self.alphabetical = alphabetical;
        }
        let reveal = (self.revealed != self.selected).then(|| self.selected.clone());
        self.revealed = self.selected.clone();
        let mut selection = Selection::single(self.selected.clone());
        let styles = &self.styles;
        let source = keyed(styles, |st| text_of(st, "id"));
        let mut list = List::<String>::new(label(tl!("Styles")))
            .header(false)
            .columns(vec![ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0))])
            .select(SelectMode::Single)
            // The list scrolls its own rows, as main's 200 pt box did, so the dialog's body (which scrolls when the
            // window is short) does not scroll it too.
            .max_rows(10)
            .selection(Bind::bound(&mut selection));
        if let Some(key) = reveal {
            list = list.reveal(key);
        }
        let out = list.show_items(ui, &source, &mut |_, icx| {
            let Some(st) = styles.get(icx.at) else { return ListRow::spacer() };
            let glyph = match st.get("type").and_then(Value::as_str) {
                Some("character") => "a",
                Some("linked") => "¶a",
                Some("table") => "#",
                _ => "¶",
            };
            let row = ListRow::new(label(text_of(st, "name"))).lead_text(label(glyph), label("¶a"));
            if flag(st, "hidden", false) { row.cell_muted(NAME) } else { row }
        });
        for event in out.events {
            match event {
                CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                    if let Some(key) = keys.last() {
                        self.selected = key.clone();
                    }
                }
                CollectionEvent::Activate(key) => {
                    self.selected = key.clone();
                    if self.current().is_some_and(|st| text_of(st, "type") != "table") {
                        self.requests.push(Request::Modify(key));
                    }
                }
                _ => {}
            }
        }
        let Some(cur) = self.current().cloned() else { return };
        let ty = text_of(&cur, "type");
        let lines = if ty == "table" { 3.0 } else { 2.0 };
        let room = vec2(ui.available_width(), lines * ui.text_style_height(&egui::TextStyle::Body));
        self.room = Some((room, Env::of(ui).view().mode()));
        if let Some((key, image)) = &self.preview {
            let image = image.clone();
            let make = move || image.clone();
            Picture::new(label(tl!("Preview")), self.made, &make).revision(key.styles).show(ui);
        }
        Text::new(label(describe(&cur))).shade(Shade::Muted).show(ui);
        let style = self.selected.clone();
        if ty != "table" {
            let gallery = flag(&cur, "inGallery", false);
            if let Some(on) =
                Checkbox::new(Bind::given(gallery)).label(label(tl!("Show in the Styles gallery"))).id_salt("gallery").show(ui).value().copied()
            {
                self.requests.push(Request::Visibility { style: style.clone(), gallery: Some(on), hidden: None });
            }
        }
        let hidden = flag(&cur, "hidden", false);
        if let Some(on) = Checkbox::new(Bind::given(hidden)).label(label(tl!("Hide from style lists"))).id_salt("hidden").show(ui).value().copied() {
            self.requests.push(Request::Visibility { style: style.clone(), gallery: None, hidden: Some(on) });
        }
        let builtin = flag(&cur, "builtIn", true);
        let table = ty == "table";
        if action(ui, "modify", label(tl!("Modify…")), !table, Some(label(tl!("Table styles are edited from Table Design.")))) {
            self.requests.push(Request::Modify(style.clone()));
        }
        if action(ui, "new", label(tl!("New Style…")), true, None) {
            self.requests.push(Request::New(text_of(&cur, "name")));
        }
        if action(ui, "delete", label(tl!("Delete")), !builtin, Some(label(tl!("Built-in styles can't be deleted.")))) {
            self.requests.push(Request::Delete(style));
        }
    }
}

impl StackDialog for ManageStylesDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let close = [FooterButton::new(label(tl!("Close")), DialogRole::Cancel)];
        let shown = Dialog::new(label(tl!("Manage Styles"))).buttons(close).show(ui, |ui, _| self.body(ui));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({"alphabetical": self.alphabetical, "selected": self.selected}))
    }
}

/// Reads the list in the dialog's order from the session (`styles.manage` would reopen the dialog) and keeps the selection on a style that exists.
fn prepare(app: &mut WordApp, id: DialogId) {
    let Some(alphabetical) = app.dialog_host.get_by_id::<ManageStylesDialog>(id).map(|d| d.alphabetical) else { return };
    let list = wordcraft_engine::cmd::para::manage_list(&app.session, alphabetical);
    let styles = list.as_array().cloned().unwrap_or_default();
    if let Some(d) = app.dialog_host.get_mut_by_id::<ManageStylesDialog>(id) {
        if !styles.iter().any(|x| text_of(x, "id") == d.selected) {
            d.selected = styles.first().map(|x| text_of(x, "id")).unwrap_or_default();
        }
        d.styles = styles;
    }
}

/// Runs what the dialog asked for and refreshes the preview. True once the dialog is gone.
// PLAN: assumed `DialogHooks::settle` returns true when the dialog finished and its hooks can be dropped (dg.A1, dialogs-w13).
fn settle(app: &mut WordApp, ctx: &Context, id: DialogId) -> Settle {
    if app.dialog_host.take_by_id::<ManageStylesDialog>(id).is_some() {
        return Settle::Done;
    }
    let Some(d) = app.dialog_host.get_mut_by_id::<ManageStylesDialog>(id) else { return Settle::Done };
    for request in std::mem::take(&mut d.requests) {
        match request {
            Request::Visibility { style, gallery, hidden } => {
                let mut params = json!({"style": style});
                if let Some(g) = gallery {
                    params["gallery"] = json!(g);
                }
                if let Some(h) = hidden {
                    params["hidden"] = json!(h);
                }
                let _ = app.run("styles.setVisibility", params);
            }
            Request::Delete(style) => {
                let _ = app.run("styles.delete", json!({"style": style}));
                if let Some(d) = app.dialog_host.get_mut_by_id::<ManageStylesDialog>(id) {
                    d.selected.clear();
                }
            }
            Request::Modify(style) => {
                super::style_forms::open_modify_child(app, &style, id);
            }
            Request::New(based_on) => {
                super::style_forms::open_new_child(app, &based_on, id);
            }
        }
    }
    refresh_preview(app, ctx, id);
    Settle::Keep
}

/// Makes the preview of the selected style when the one held was made for something else.
fn refresh_preview(app: &mut WordApp, ctx: &Context, id: DialogId) {
    let Some(d) = app.dialog_host.get_by_id::<ManageStylesDialog>(id) else { return };
    let (Some(cur), Some((room, mode))) = (d.current(), d.room) else { return };
    let (selected, ty) = (d.selected.clone(), text_of(cur, "type"));
    let held = d.preview.as_ref().map(|(key, _)| key.clone());
    let ppp = ctx.pixels_per_point();
    let styles = crate::previews::style_revision(app, mode);
    let key = PreviewKey { selected, ty, styles, room: [room.x.to_bits(), room.y.to_bits()], mode, ppp: ppp.to_bits() };
    if held.as_ref() == Some(&key) {
        return;
    }
    let image = crate::previews::style_preview_image(app, &key.selected, &key.ty, room, ppp, mode);
    if let Some(d) = app.dialog_host.get_mut_by_id::<ManageStylesDialog>(id) {
        d.made += 1;
        d.preview = image.map(|image| (key, image));
    }
}

/// A style's description: its type, the formatting it sets and what it is based on.
fn describe(st: &Value) -> String {
    let f = st.get("format").cloned().unwrap_or_default();
    let fs = |k: &str| f.get(k).and_then(Value::as_str);
    let fnum = |k: &str| f.get(k).and_then(Value::as_f64);
    let pt = |x: f64| format!("{}", (x * 10.0).round() / 10.0);
    let mut parts: Vec<String> = vec![
        match st.get("type").and_then(Value::as_str) {
            Some("character") => tl!("Character style"),
            Some("linked") => tl!("Linked style (paragraph and character)"),
            Some("table") => tl!("Table style"),
            _ => tl!("Paragraph style"),
        }
        .to_string(),
    ];
    if st.get("builtIn").and_then(Value::as_bool).unwrap_or(false) {
        parts.push(tl!("Built-in style").to_string());
    }
    if let Some(x) = fs("font") {
        parts.push(crate::i18n::fmt(tl!("Font: {font}"), &[("font", x)]));
    }
    if let Some(x) = fnum("size") {
        parts.push(crate::i18n::fmt(tl!("{size} pt"), &[("size", &pt(x))]));
    }
    if f.get("bold").and_then(Value::as_bool) == Some(true) {
        parts.push(tl!("Bold").to_string());
    }
    if f.get("italic").and_then(Value::as_bool) == Some(true) {
        parts.push(tl!("Italic").to_string());
    }
    if let Some(x) = fs("color") {
        parts.push(crate::i18n::fmt(tl!("Color: {color}"), &[("color", &format!("#{x}"))]));
    }
    if let Some(x) = fnum("spaceBefore") {
        parts.push(crate::i18n::fmt(tl!("Space before: {pt} pt"), &[("pt", &pt(x))]));
    }
    if let Some(x) = fnum("spaceAfter") {
        parts.push(crate::i18n::fmt(tl!("Space after: {pt} pt"), &[("pt", &pt(x))]));
    }
    if let Some(b) = st.get("basedOn").and_then(Value::as_str) {
        parts.push(crate::i18n::fmt(tl!("Based on: {style}"), &[("style", b)]));
    }
    parts.join(", ")
}
