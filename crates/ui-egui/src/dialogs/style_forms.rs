//! New Style and Modify Style. Each ends in one command (`styles.create`, `styles.modify`), held until the
//! command answers so a refusal shows in the dialog.

use std::sync::Arc;

use craft::font_manager::Catalog;
use craft::font_manager::picker::FontPickerSettings;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::{DialogFields, FormDialog, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::text::{NumberField, TextField};
use egui::{Context, Ui};
use serde_json::{Value, json};

use super::host::{DialogHooks, Settle, open_with};
use super::styles::{ManageStylesDialog, label};
use crate::WordApp;

const NEW_KEY: &str = "newStyle";
const MODIFY_KEY: &str = "modifyStyle";

/// The values of Modify Style.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ModifyStyleForm {
    id: String,
    name: String,
    font: String,
    size: f64,
    bold: bool,
    italic: bool,
    /// `RRGGBB`, or "auto".
    color: String,
    before: f64,
    after: f64,
}

impl ModifyStyleForm {
    /// What the style resolves to now. `None` for an unknown style.
    fn read(app: &WordApp, id: &str) -> Option<ModifyStyleForm> {
        let st = app.session.doc.styles.get(id)?;
        let rc = app.session.doc.styles.resolve_char(
            if st.kind == wordcraft_doc::StyleKind::Paragraph { Some(id) } else { None },
            &wordcraft_doc::CharProps {
                style: if st.kind == wordcraft_doc::StyleKind::Character { Some(id.into()) } else { None },
                ..Default::default()
            },
        );
        let rp = app.session.doc.styles.resolve_para(&wordcraft_doc::ParaProps { style: Some(id.into()), ..Default::default() });
        Some(ModifyStyleForm {
            id: id.into(),
            name: st.name.clone(),
            font: rc.font,
            size: f64::from(rc.size),
            bold: rc.bold,
            italic: rc.italic,
            color: match rc.color {
                wordcraft_doc::TextColor::Rgb(c) => c.hex(),
                wordcraft_doc::TextColor::Auto => "auto".into(),
            },
            before: f64::from(rp.space_before),
            after: f64::from(rp.space_after),
        })
    }

    /// The `styles.modify` parameters; `font` is the family as the document stores it. An unparsable colour sends none.
    fn params(&self, font: &str) -> Value {
        let mut chr = json!({"font": font, "size": self.size, "bold": self.bold, "italic": self.italic});
        if let Some(c) = wordcraft_doc::Rgb::parse(&self.color) {
            chr["color"] = json!({"Rgb": [c.0, c.1, c.2]});
        }
        json!({"style": self.id, "name": self.name, "chr": chr, "para": {"spaceBefore": self.before, "spaceAfter": self.after}})
    }
}

/// What the Modify Style body asks of the app.
enum Request {
    /// The font box's Add Font… entry.
    AddFont,
}

/// Modify Style on the dialog stack, over a font box that needs the app's catalog and recent fonts.
pub(crate) struct ModifyStyleDialog {
    id: DialogId,
    form: ModifyStyleForm,
    held: HeldAnswer<ModifyStyleForm>,
    catalog: Arc<Catalog>,
    settings: FontPickerSettings,
    requests: Vec<Request>,
}

/// Opens New Style (`newStyle`) or Modify Style (`modifyStyle`, for `params["style"]`) over `parent`; `None` for another
/// name and for an unknown style (TOC1–TOC9 may be absent).
pub(crate) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    match name {
        NEW_KEY => open_new_with(app, "Normal", parent),
        MODIFY_KEY => {
            let id = params.get("style")?.as_str()?;
            let dialog = modify_dialog(app, id)?;
            Some(open_with(app, parent, dialog, modify_hooks()))
        }
        _ => None,
    }
}

/// Opens Modify Style for the style `id`, or nothing for an unknown style.
pub(crate) fn modify_style(app: &mut WordApp, id: &str) -> Option<DialogId> {
    let dialog = modify_dialog(app, id)?;
    Some(open_with(app, None, dialog, modify_hooks()))
}

/// Opens Modify Style over `parent`, which stays open under it; nothing for an unknown style (TOC1–TOC9 may be absent).
pub(crate) fn open_modify_child(app: &mut WordApp, id: &str, parent: DialogId) -> Option<DialogId> {
    let dialog = modify_dialog(app, id)?;
    Some(open_with(app, Some(parent), dialog, modify_hooks()))
}

fn modify_dialog(app: &mut WordApp, id: &str) -> Option<ModifyStyleDialog> {
    let form = ModifyStyleForm::read(app, id)?;
    // PLAN: assumed `WordApp::font_catalog(&self) -> Arc<Catalog>` and `WordApp::font_picker_settings(&self) -> FontPickerSettings` (ribbon-w11, rb.A5).
    Some(ModifyStyleDialog {
        id: dialog_id(MODIFY_KEY),
        form,
        held: HeldAnswer::default(),
        catalog: app.font_catalog(),
        settings: app.font_picker_settings(),
        requests: Vec::new(),
    })
}

fn modify_hooks() -> DialogHooks {
    DialogHooks { prepare: Some(modify_prepare), settle: modify_settle }
}

fn body(ui: &mut Ui, form: &mut ModifyStyleForm, catalog: &Arc<Catalog>, settings: &mut FontPickerSettings, requests: &mut Vec<Request>) {
    Field::new(label(tl!("Name:"))).show(ui, |ui, _| TextField::new(label(tl!("Name:")), Bind::bound(&mut form.name)).show(ui));
    let actions = [label(tl!("Add Font…"))];
    Field::new(label(tl!("Font:"))).show(ui, |ui, _| {
        let out =
            crate::widgets::font_picker(label(tl!("Font:")), Bind::bound(&mut form.font), catalog, Bind::bound(settings), &actions, None).show(ui);
        if out.action == Some(0) {
            requests.push(Request::AddFont);
        }
    });
    // The form keeps a number; a cleared box leaves the value as it was.
    let (mut size, mut before, mut after) = (Some(form.size), Some(form.before), Some(form.after));
    Field::new(label(tl!("Size:")))
        .show(ui, |ui, _| NumberField::new(label(tl!("Size:")), Bind::bound(&mut size)).range(1.0..=1638.0).step(0.5).show(ui));
    Field::new(label(tl!("Color:"))).show(ui, |ui, _| TextField::new(label(tl!("Color:")), Bind::bound(&mut form.color)).show(ui));
    Field::new(label(tl!("Space before/after:"))).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            NumberField::new(label(tl!("Space before")), Bind::bound(&mut before)).range(0.0..=1584.0).id_salt("before").show(ui);
            NumberField::new(label(tl!("Space after")), Bind::bound(&mut after)).range(0.0..=1584.0).id_salt("after").show(ui);
        });
    });
    form.size = size.unwrap_or(form.size);
    form.before = before.unwrap_or(form.before);
    form.after = after.unwrap_or(form.after);
    Row::new().show(ui, |ui, _| {
        Checkbox::new(Bind::bound(&mut form.bold)).label(label(tl!("Bold"))).id_salt("bold").show(ui);
        Checkbox::new(Bind::bound(&mut form.italic)).label(label(tl!("Italic"))).id_salt("italic").show(ui);
    });
}

impl StackDialog for ModifyStyleDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(MODIFY_KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let ModifyStyleDialog { form, held, catalog, settings, requests, .. } = self;
        let shown = FormDialog::new(label(tl!("Modify Style")), Bind::bound(form)).held(held).show(ui, |ui, _, form| {
            body(ui, form, catalog, settings, requests);
            true
        });
        shown.closed.map(|_| if shown.accepted.is_some() { DialogResult::Ok(()) } else { DialogResult::Cancel })
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn confirm(&mut self, _: &Context) -> Option<()> {
        if self.held.finish().is_some() {
            return Some(());
        }
        if !self.held.is_waiting() {
            self.held.file(self.form.clone());
        }
        None
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        let wrong = || format!("wrong value for {field}");
        let f = &mut self.form;
        match field {
            "name" => f.name = value.as_str().ok_or_else(wrong)?.to_string(),
            "font" => f.font = value.as_str().ok_or_else(wrong)?.to_string(),
            "color" => f.color = value.as_str().ok_or_else(wrong)?.to_string(),
            "size" => f.size = value.as_f64().ok_or_else(wrong)?,
            "before" => f.before = value.as_f64().ok_or_else(wrong)?,
            "after" => f.after = value.as_f64().ok_or_else(wrong)?,
            "bold" => f.bold = value.as_bool().ok_or_else(wrong)?,
            "italic" => f.italic = value.as_bool().ok_or_else(wrong)?,
            _ => return Err(format!("unknown field {field}")),
        }
        Ok(())
    }

    fn json_fields(&self) -> Option<Value> {
        let f = &self.form;
        Some(
            json!({"name": f.name, "font": f.font, "size": f.size, "color": f.color, "bold": f.bold, "italic": f.italic, "before": f.before, "after": f.after}),
        )
    }
}

/// Brings the recent fonts in for the font box.
fn modify_prepare(app: &mut WordApp, id: DialogId) {
    let settings = app.font_picker_settings();
    if let Some(d) = app.dialog_host.get_mut_by_id::<ModifyStyleDialog>(id) {
        d.settings = settings;
    }
}

/// Runs the held OK's command and answers it; hands the recent fonts back. True once the dialog is gone.
fn modify_settle(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    if app.dialog_host.take_by_id::<ModifyStyleDialog>(id).is_some() {
        return Settle::Done;
    }
    let Some(d) = app.dialog_host.get_mut_by_id::<ModifyStyleDialog>(id) else { return Settle::Done };
    let requests = std::mem::take(&mut d.requests);
    let settings = d.settings.clone();
    let filed = d.held.take();
    for Request::AddFont in requests {
        let _ = app.run("font.add", json!({}));
    }
    app.set_font_picker_settings(settings);
    if let Some(form) = filed {
        let font = app.previews.font_stored(&form.font);
        let answer = app.run("styles.modify", form.params(&font)).map(|_| ()).map_err(label);
        if answer.is_ok()
            && let Some(parent) = app.dialog_host.stack().parent(id)
            && let Some(manage) = app.dialog_host.get_mut_by_id::<ManageStylesDialog>(parent)
        {
            manage.select(form.id.clone());
        }
        if let Some(d) = app.dialog_host.get_mut_by_id::<ModifyStyleDialog>(id) {
            d.held.answer(answer);
        }
    }
    Settle::Keep
}

/// Opens New Style based on the style named `based_on`, over `parent` (Manage Styles).
pub(crate) fn open_new_child(app: &mut WordApp, based_on: &str, parent: DialogId) -> Option<DialogId> {
    open_new_with(app, based_on, Some(parent))
}

/// `None` before the first frame: the dialog reads its remembered values through the egui context.
fn open_new_with(app: &mut WordApp, based_on: &str, parent: Option<DialogId>) -> Option<DialogId> {
    let ctx = app.ctx.clone()?;
    let fields = DialogFields::from_pairs([("name", "Style1"), ("basedOn", based_on)]);
    let (name_default, based_default) = ("Style1".to_string(), based_on.to_string());
    let dialog = HostedParamDialog::command(&ctx, NEW_KEY, fields, move |_| {
        let params = vec![
            Param::new("name", label(tl!("Name:")), ParamKind::text(name_default.clone())),
            Param::new("basedOn", label(tl!("Style based on:")), ParamKind::text(based_default.clone())),
        ];
        ParamDialog::new(NEW_KEY, label(tl!("Create New Style")), params)
            .footer_note(label(tl!("The new style takes the formatting of the current paragraph.")))
    })
    .held_ok();
    Some(open_with(app, parent, dialog, DialogHooks { prepare: None, settle: new_settle }))
}

/// Runs `styles.create` for the held OK, answers it, and selects the created style in Manage Styles when that is the parent.
fn new_settle(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    if app.dialog_host.take_by_id::<HostedParamDialog>(id).is_some() {
        return Settle::Done;
    }
    let Some(d) = app.dialog_host.get_mut_by_id::<HostedParamDialog>(id) else { return Settle::Done };
    let Some(ok) = d.take_ok() else { return Settle::Keep };
    let text = |k: &str| ok.fields.text(k).unwrap_or_default().to_string();
    let result = app.run("styles.create", json!({"name": text("name"), "basedOn": text("basedOn")}));
    let answer = match result {
        Ok(v) => {
            let created = v.get("id").and_then(Value::as_str).map(str::to_string);
            if let (Some(created), Some(parent)) = (created, app.dialog_host.stack().parent(id))
                && let Some(manage) = app.dialog_host.get_mut_by_id::<ManageStylesDialog>(parent)
            {
                manage.select(created);
            }
            Ok(())
        }
        Err(e) => Err(label(e)),
    };
    if let Some(d) = app.dialog_host.get_mut_by_id::<HostedParamDialog>(id) {
        d.answer_ok(answer);
    }
    Settle::Keep
}
