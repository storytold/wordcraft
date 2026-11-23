//! The one-field dialogs: Rename Document, Insert Table, Watermark, Field Code / Edit Field Code and Cell Margins.
//! Each is a hosted parameter dialog with a held OK: OK files the values, `settle` runs the command and answers the
//! refusal (shown under the rows) or acceptance.

use egui::Context;
use serde_json::{Value, json};

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;

use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

/// Opens the dialog `name` when it is one of this file's. An unavailable dialog (Edit Field Code without a field, Cell
/// Margins outside a table) opens nothing. The context only recalls a dialog's remembered values
/// (`HostedParamDialog::new`); before the window's first frame there is none, and a bare context recalls nothing, so
/// the dialog still opens for the user who asked (as production's did).
pub(crate) fn open(app: &mut WordApp, name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let ctx = app.ctx.clone().unwrap_or_default();
    match name {
        "renameDocument" => Some(open_rename(app, &ctx, parent)),
        "insertTable" => Some(open_insert_table(app, &ctx, parent)),
        "watermark" => Some(open_watermark(app, &ctx, parent)),
        "fieldCode" => Some(open_field_code(app, &ctx, parent, None)),
        "editFieldCode" => {
            let field = wordcraft_engine::cmd::fields::selected(&app.session).into_iter().next()?;
            Some(open_field_code(app, &ctx, parent, Some(field.instr.clone())))
        }
        "tableCellMargins" => open_cell_margins(app, &ctx, parent),
        _ => None,
    }
}

/// Takes the filed OK of the hosted dialog `id`, runs `run` on its values and answers the dialog with the result. The
/// hook is spent once the dialog is gone; a pass without a filed OK, and a refused one, keep it.
fn settle_with(app: &mut WordApp, id: DialogId, run: impl FnOnce(&mut WordApp, &ParamResult) -> Result<(), String>) -> Settle {
    let Some(dialog) = app.dialog_host.get_mut_by_id::<HostedParamDialog>(id) else {
        return Settle::Done;
    };
    let Some(result) = dialog.take_ok() else {
        return Settle::Keep;
    };
    let answer = run(app, &result).map_err(Label::from);
    if let Some(dialog) = app.dialog_host.get_mut_by_id::<HostedParamDialog>(id) {
        dialog.answer_ok(answer);
    }
    Settle::Keep
}

fn open_hosted(
    app: &mut WordApp,
    parent: Option<DialogId>,
    dialog: HostedParamDialog,
    settle: fn(&mut WordApp, &Context, DialogId) -> Settle,
) -> DialogId {
    open_with(app, parent, dialog.held_ok(), DialogHooks { prepare: None, settle })
}

fn open_rename(app: &mut WordApp, ctx: &Context, parent: Option<DialogId>) -> DialogId {
    let fields = DialogFields::from_pairs([("name", app.title_stem())]);
    let dialog = HostedParamDialog::new(ctx, "renameDocument", fields, |fields| {
        let params = vec![Param::new("name", tl!("Document name").to_string(), ParamKind::text("")).max_chars(200)];
        ParamDialog::command("renameDocument", tl!("Rename Document").to_string(), params)
            .ok_label(Label::from(tl!("Rename")))
            .values(Bind::bound(fields))
            .alt_resets(false)
    });
    open_hosted(app, parent, dialog, settle_rename)
}

fn settle_rename(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    settle_with(app, id, |app, result| {
        let name = result.fields.text("name").unwrap_or_default().trim().to_string();
        // An unsaved document would otherwise take the name "Document1" from `file.rename`.
        if name == app.title_stem() {
            return Ok(());
        }
        app.run("file.rename", json!({"name": name})).map(|_| ())
    })
}

fn open_insert_table(app: &mut WordApp, ctx: &Context, parent: Option<DialogId>) -> DialogId {
    let fields = DialogFields::from_pairs([("cols", 5_i64), ("rows", 2_i64)]);
    let dialog = HostedParamDialog::new(ctx, "insertTable", fields, |fields| {
        let params = vec![
            Param::new("cols", tl!("Number of columns").to_string(), ParamKind::int(Some(1), Some(63), 5)),
            Param::new("rows", tl!("Number of rows").to_string(), ParamKind::int(Some(1), Some(1000), 2)),
        ];
        ParamDialog::command("insertTable", tl!("Insert Table").to_string(), params).values(Bind::bound(fields)).alt_resets(false)
    });
    open_hosted(app, parent, dialog, settle_insert_table)
}

fn settle_insert_table(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    settle_with(app, id, |app, result| {
        let rows = result.fields.int("rows").unwrap_or(2);
        let cols = result.fields.int("cols").unwrap_or(5);
        app.run("insert.table", json!({"rows": rows, "cols": cols})).map(|_| ())
    })
}

fn open_watermark(app: &mut WordApp, ctx: &Context, parent: Option<DialogId>) -> DialogId {
    let fields = DialogFields::from_pairs([("text", "CONFIDENTIAL"), ("layout", "diagonal")]);
    let dialog = HostedParamDialog::new(ctx, "watermark", fields, |fields| {
        let layout = vec![
            ("diagonal".to_string(), Label::from(tl!("Diagonal").to_string())),
            ("horizontal".to_string(), Label::from(tl!("Horizontal").to_string())),
        ];
        let params = vec![
            // The old dialog's text field was about 280 pt wide, wider than the field length: some 38 characters.
            Param::new("text", tl!("Text").to_string(), ParamKind::text("CONFIDENTIAL")).max_chars(200).field_chars(38),
            // The old dialog shows the two layouts side by side under the text, with no caption.
            Param::new("layout", tl!("Layout").to_string(), ParamKind::radio(layout, Some("diagonal".into()))).options_in_row(),
        ];
        ParamDialog::command("watermark", tl!("Custom Watermark").to_string(), params).values(Bind::bound(fields)).alt_resets(false)
    });
    open_hosted(app, parent, dialog, settle_watermark)
}

fn settle_watermark(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    settle_with(app, id, |app, result| {
        let text = result.fields.text("text").unwrap_or_default().to_string();
        let diagonal = result.fields.text("layout") != Some("horizontal");
        app.run("design.watermark", json!({"text": text, "diagonal": diagonal})).map(|_| ())
    })
}

/// The key of the Field Code dialog: Edit Field Code is its own dialog, so both can be told apart by key.
fn field_code_key(edit: bool) -> &'static str {
    if edit { "editFieldCode" } else { "fieldCode" }
}

fn open_field_code(app: &mut WordApp, ctx: &Context, parent: Option<DialogId>, code: Option<String>) -> DialogId {
    let edit = code.is_some();
    let fields = DialogFields::from_pairs([("code", code.unwrap_or_default())]);
    let key = field_code_key(edit);
    let dialog = HostedParamDialog::new(ctx, key, fields, move |fields| {
        let ready = fields.text("code").is_some_and(|c| !c.trim().is_empty());
        // PLAN: the sheet's `below` does not exist on `ParamDialog`; the examples line is the row's `hint`.
        let params = vec![
            Param::new("code", tl!("Field codes").to_string(), ParamKind::text(""))
                .mono()
                .hint(tl!("PAGE · NUMPAGES · DATE \\@ \"MMMM d, yyyy\" · AUTHOR · TITLE · SEQ Figure").to_string()),
        ];
        let title = if edit { tl!("Edit Field Code") } else { tl!("Field Code") };
        ParamDialog::command(key, title.to_string(), params).ok_enabled(ready).values(Bind::bound(fields)).alt_resets(false)
    });
    open_hosted(app, parent, dialog, if edit { settle_edit_field_code } else { settle_field_code })
}

fn run_field_code(app: &mut WordApp, id: DialogId, edit: bool) -> Settle {
    settle_with(app, id, |app, result| {
        let code = result.fields.text("code").unwrap_or_default().trim().to_string();
        if code.is_empty() {
            return Ok(());
        }
        let run = if edit { app.run("fields.setCode", json!({"code": code})) } else { app.run("insert.field", json!({"instr": code})) };
        run.map(|_| ())
    })
}

fn settle_field_code(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    run_field_code(app, id, false)
}

fn settle_edit_field_code(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    run_field_code(app, id, true)
}

/// The caret's table cell margins in points (top, left, bottom, right), `None` outside a table. Word's default
/// stands in for a table that sets none and whose style sets none.
fn cell_margins_pt(app: &WordApp) -> Option<[f64; 4]> {
    let (tp, _, _) = app.session.sel.focus.path.cell()?;
    let table = app.session.doc.table(app.session.sel.focus.story, &tp)?;
    let style = table.props.style.as_deref().and_then(|id| app.session.doc.styles.table_style(id));
    let margins = table.props.cell_margins.or_else(|| style.and_then(|s| s.parts.cell_margins)).unwrap_or([0.0, 5.4, 0.0, 5.4]);
    let fallback = [0.0, 5.4, 0.0, 5.4];
    let mut out = [0.0; 4];
    for (i, slot) in out.iter_mut().enumerate() {
        let v = margins.get(i).copied().unwrap_or(0.0);
        let v = if v.is_finite() { v } else { fallback[i] };
        *slot = f64::from(v).clamp(0.0, 144.0);
    }
    Some(out)
}

const MARGIN_SIDES: [(&str, &str); 4] = [("top", "Top"), ("left", "Left"), ("bottom", "Bottom"), ("right", "Right")];

fn open_cell_margins(app: &mut WordApp, ctx: &Context, parent: Option<DialogId>) -> Option<DialogId> {
    let margins = cell_margins_pt(app)?;
    let fields = DialogFields::from_pairs(MARGIN_SIDES.iter().zip(margins).map(|((key, _), v)| (*key, v)));
    let kind = crate::options::unit_kind(app.session.units());
    let dialog = HostedParamDialog::new(ctx, "tableCellMargins", fields, move |fields| {
        let params: Vec<Param> = MARGIN_SIDES
            .iter()
            .map(|(key, label)| Param::number(*key, tl!(label).to_string(), 0.0, 1.0, 2).bounds(Some(0.0), Some(144.0)).unit(kind))
            .collect();
        ParamDialog::command("tableCellMargins", tl!("Cell Margins").to_string(), params).values(Bind::bound(fields)).alt_resets(false)
    });
    Some(open_hosted(app, parent, dialog, settle_cell_margins))
}

fn settle_cell_margins(app: &mut WordApp, _: &Context, id: DialogId) -> Settle {
    settle_with(app, id, |app, result| {
        let now = cell_margins_pt(app).ok_or_else(|| tl!("Put the caret in a table first.").to_string())?;
        let mut sent = [0.0; 4];
        for (slot, (key, _)) in sent.iter_mut().zip(MARGIN_SIDES) {
            *slot = result.fields.number(key).filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, 144.0);
        }
        // A confirm with nothing moved changes nothing: the other three sides keep their stored values.
        if sent.iter().zip(now).all(|(a, b)| (a - b).abs() <= 1e-4) {
            return Ok(());
        }
        let params: Value = json!({"top": sent[0], "left": sent[1], "bottom": sent[2], "right": sent[3]});
        app.run("table.cellMargins", params).map(|_| ())
    })
}
