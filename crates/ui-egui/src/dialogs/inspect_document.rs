//! File › Info › Check for Issues › Inspect Document (#5676): the Document Inspector's report —
//! comments, tracked changes, document properties, hidden text, headers and footers — with the
//! inspector's Remove All per category, as Word's. A removal runs one `file.inspect` with
//! `remove`, and the report is read afresh, so the counts follow what the removal took out.

use egui::{Context, Ui};
use serde_json::{Value, json};

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Gap, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::sections::{DialogRole, FooterButton};

use super::host::{DialogHooks, Settle, open_with};
use crate::WordApp;

/// The name the dialog opens under (dg.A2) and the key of its id.
const KEY: &str = "inspectDocument";

/// The Inspect Document dialog: the last report, and the categories whose Remove All was
/// pressed in the last pass, run by `settle`.
pub(crate) struct InspectDialog {
    /// The last `file.inspect` report. `Null` on a failed read: every category then shows "None"
    /// as a count would, so the error is named above instead.
    report: Value,
    /// Why the last read or removal fell short: the read's error, or what a removal left.
    message: Option<String>,
    /// The categories whose Remove All was pressed in the last pass, run by `settle`.
    requests: Vec<String>,
}

impl StackDialog for InspectDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(KEY)
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let report = self.report.clone();
        let message = self.message.clone();
        let requests = &mut self.requests;
        let shown = Dialog::new(tl!("Inspect Document").to_string())
            // The inspector's one button: Close (as Word's, which has no OK).
            .buttons([FooterButton::new(tl!("Close").to_string(), DialogRole::Accept)])
            .show(ui, |ui, _| {
                if let Some(m) = &message {
                    // A failed read is not a clean document: the error is shown in the error tone,
                    // where a note of a removal would otherwise read as part of the report.
                    let tone = if report.is_null() { Tone::Bad } else { Tone::Neutral };
                    Notice::new(tone, m.clone()).layout(NoticeLayout::Inline).show(ui);
                }
                let _ = Text::new(tl!("Inspect the document for content you may want to remove before sharing it:").to_string()).show(ui);
                let found = report.get("found").cloned().unwrap_or(json!({}));
                let count = |k: &str| found.get(k).and_then(Value::as_u64).unwrap_or(0);
                let none = || tl!("None").to_string();
                // One row per category: what the inspector found, and Remove All for what it can
                // take out. A removal is one `file.inspect` with `remove`, run in `settle`.
                let comments = count("comments");
                Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                    let _ = Text::new(tl!("Comments").to_string()).show(ui);
                    let _ = Text::new(if comments > 0 { comments.to_string() } else { none() }).show(ui);
                    if comments > 0 && clicked(&Button::new(tl!("Remove All")).id_salt("remove-comments").show(ui)) {
                        requests.push("comments".into());
                    }
                });
                let revisions = count("revisions");
                Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                    let _ = Text::new(tl!("Revisions").to_string()).show(ui);
                    let _ = Text::new(if revisions > 0 { revisions.to_string() } else { none() }).show(ui);
                    if revisions > 0 && clicked(&Button::new(tl!("Remove All")).id_salt("remove-revisions").show(ui)) {
                        requests.push("revisions".into());
                    }
                });
                let properties = found.get("properties").and_then(Value::as_bool).unwrap_or(false);
                Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                    let _ = Text::new(tl!("Document properties").to_string()).show(ui);
                    if properties {
                        let _ = Text::new(tl!("Removable document properties are present.").to_string()).show(ui);
                        if clicked(&Button::new(tl!("Remove All")).id_salt("remove-properties").show(ui)) {
                            requests.push("properties".into());
                        }
                    } else {
                        let _ = Text::new(none()).show(ui);
                    }
                });
                if found.get("hiddenText").and_then(Value::as_bool).unwrap_or(false) {
                    let pieces = found.get("hiddenCount").and_then(Value::as_u64).unwrap_or(0);
                    Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                        let _ = Text::new(tl!("Hidden text").to_string()).show(ui);
                        let _ = Text::new(crate::i18n::fmt(tl!("{count} piece(s)"), &[("count", &pieces.to_string())])).show(ui);
                        if clicked(&Button::new(tl!("Remove All")).id_salt("remove-hidden").show(ui)) {
                            requests.push("hidden".into());
                        }
                    });
                    if found.get("truncated").and_then(Value::as_bool).unwrap_or(false) {
                        let _ = Notice::new(Tone::Neutral, tl!("The list of pieces is cut; the count is exact.").to_string())
                            .layout(NoticeLayout::Inline)
                            .show(ui);
                    }
                } else {
                    Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                        let _ = Text::new(tl!("Hidden text").to_string()).show(ui);
                        let _ = Text::new(none()).show(ui);
                    });
                }
                // Shown whatever the removable hidden-text branch says: hidden text the inspector
                // can't reach is a finding of its own, and it stays after a removal took the
                // removable out.
                if !found.get("unsupported").and_then(Value::as_array).is_none_or(|a| a.is_empty()) {
                    Notice::new(Tone::Neutral, tl!("Some hidden text is inside objects or table styles the inspector can't remove.").to_string())
                        .layout(NoticeLayout::Inline)
                        .show(ui);
                }
                let headers = count("headers");
                Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                    let _ = Text::new(tl!("Headers and footers").to_string()).show(ui);
                    let _ = Text::new(if headers > 0 { headers.to_string() } else { none() }).show(ui);
                    if headers > 0 && clicked(&Button::new(tl!("Remove All")).id_salt("remove-headers").show(ui)) {
                        requests.push("headers".into());
                    }
                });
            });
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }

    /// The report and the message, so automation reads what the dialog shows.
    fn json_fields(&self) -> Option<Value> {
        Some(json!({"report": self.report, "message": self.message}))
    }
}

/// Runs the removals the pass pressed, then reads the report afresh, so the counts follow what
/// the removal took out. A removal's failure and its leftovers are the message, not an error to
/// the status bar: the dialog is where the inspector's work is followed.
fn settle(app: &mut WordApp, _ctx: &Context, id: DialogId) -> Settle {
    let requests = app.dialog_host.get_mut_by_id::<InspectDialog>(id).map(|d| std::mem::take(&mut d.requests)).unwrap_or_default();
    if !requests.is_empty() {
        let mut message = None;
        for key in &requests {
            match app.run("file.inspect", json!({"remove": [key]})) {
                Ok(out) => {
                    let residual = out.get("residual").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
                    message = Some(if residual > 0 {
                        crate::i18n::fmt(
                            tl!(
                                "Removed, but {count} piece(s) a removal couldn't reach remain in objects kept from the source file or table styles."
                            ),
                            &[("count", &residual.to_string())],
                        )
                    } else {
                        tl!("Removed.").to_string()
                    });
                }
                Err(e) => message = Some(e),
            }
        }
        // The report is read afresh even after a refused removal, so what the dialog shows is the
        // document as it now stands, not the report the press was answered from.
        let report = app.session.run("file.inspect", &json!({})).unwrap_or_default();
        if let Some(d) = app.dialog_host.get_mut_by_id::<InspectDialog>(id) {
            d.report = report;
            d.message = message;
        }
    }
    match app.dialog_host.take_by_id::<InspectDialog>(id) {
        Some(_) => Settle::Done,
        None => Settle::Keep,
    }
}

/// Opens the dialog (`inspectDocument`, dg.A2).
pub(crate) fn open(app: &mut WordApp, _name: &str, _params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    // Word's Document Inspector: the last Inspect's report, Remove All per category. A failed
    // read must not read as a clean document: Null reports every category as "None", so the
    // error is named where the counts would otherwise stand.
    let (report, message) = match app.session.run("file.inspect", &json!({})) {
        Ok(report) => (report, None),
        Err(e) => (Value::Null, Some(e.to_string())),
    };
    // The dialog is opened again while it shows: the open one stays (as `macros::open` does; a
    // second dialog under the same id would cancel the first).
    let open = dialog_id(KEY);
    if let Some(d) = app.dialog_host.get_mut_by_id::<InspectDialog>(open) {
        d.report = report;
        d.message = message;
        return Some(open);
    }
    Some(open_with(app, parent, InspectDialog { report, message, requests: Vec::new() }, DialogHooks { prepare: None, settle }))
}
