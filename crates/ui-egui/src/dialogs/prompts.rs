//! The small questions: a protected file's password, the password a document is saved with,
//! discarding a recovery copy, and overwriting a damaged DOCX.

use craft::ui::dialog::{DialogId, DialogResult, DialogStack, PromptChoice, StackDialog, StandardPrompt, dialog_id};
use craft::ui::patterns::form_dialogs::{PasswordDialog, PasswordPurpose, Secret};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use super::host::{DialogHooks, Keyed, Settle, open_with};
use crate::WordApp;

/// Opens `password`, `encryptPassword` or `discardRecovery` by name; `None` for any other.
/// `password` reads `name`, `then` and `params`, `discardRecovery` reads `id` and `name`.
pub(super) fn open(app: &mut WordApp, name: &str, params: &Value, parent: Option<DialogId>) -> Option<DialogId> {
    let text = |key: &str| params.get(key).and_then(Value::as_str).unwrap_or("");
    match name {
        "password" => {
            let ask = Ask::Open {
                name: text("name").to_owned(),
                then: text("then").to_owned(),
                params: params.get("params").cloned().unwrap_or(Value::Null),
            };
            Some(open_prompt(app, parent, ask, None))
        }
        "encryptPassword" => Some(open_prompt(app, parent, Ask::Encrypt, None)),
        "discardRecovery" if !text("id").is_empty() => Some(open_discard_recovery(app, text("id"), text("name"))),
        _ => None,
    }
}

/// What a password prompt is for.
#[derive(Clone)]
enum Ask {
    /// The password of file `name`; `then` runs again with `params` and the password.
    Open { name: String, then: String, params: Value },
    /// The password the document is saved with, or none to remove it.
    Encrypt,
}

/// What a finished password prompt hands back: the typed password and what it was for.
pub(super) struct PasswordReply {
    secret: Secret,
    ask: Ask,
}

/// A password prompt on the dialog host. Its fields never include the typed secret.
pub(super) struct PasswordPrompt {
    ask: Ask,
    error: Option<String>,
}

impl PasswordPrompt {
    fn key_name(&self) -> &'static str {
        match self.ask {
            Ask::Open { .. } => "password",
            Ask::Encrypt => "encryptPassword",
        }
    }
}

impl StackDialog for PasswordPrompt {
    type Result = PasswordReply;

    fn id(&self) -> DialogId {
        dialog_id(self.key_name())
    }

    fn key(&self) -> Option<&str> {
        Some(self.key_name())
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<PasswordReply>> {
        let purpose = match &self.ask {
            Ask::Open { name, .. } => PasswordPurpose::open(name.clone()),
            Ask::Encrypt => PasswordPurpose::SetOrClear,
        };
        let mut dialog = PasswordDialog::new(purpose);
        if let Some(error) = &self.error {
            dialog = dialog.error(Label::from(error.clone()));
        }
        let out = dialog.show(ui);
        match (out.answer, out.closed) {
            (Some(answer), _) => Some(DialogResult::Ok(PasswordReply { secret: answer.secret, ask: self.ask.clone() })),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn json_fields(&self) -> Option<Value> {
        Some(match &self.ask {
            Ask::Open { name, then, params } => json!({"name": name, "then": then, "params": params, "error": self.error}),
            Ask::Encrypt => json!({"error": self.error}),
        })
    }
}

fn open_prompt(app: &mut WordApp, parent: Option<DialogId>, ask: Ask, error: Option<String>) -> DialogId {
    open_with(app, parent, PasswordPrompt { ask, error }, DialogHooks { prepare: None, settle: settle_password })
}

/// Asks for the password of `name`; once given, `then` runs again with `params` and the password.
pub(crate) fn open_password(app: &mut WordApp, name: &str, then: &str, params: Value) -> DialogId {
    open_prompt(app, None, Ask::Open { name: name.to_owned(), then: then.to_owned(), params }, None)
}

fn settle_password(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(result) = app.dialog_host.take_by_id::<PasswordPrompt>(id) else {
        return Settle::Keep;
    };
    let DialogResult::Ok(PasswordReply { secret, ask }) = result else {
        return Settle::Done;
    };
    match ask {
        Ask::Open { name, then, params } => {
            let mut p = params.clone();
            if let Some(o) = p.as_object_mut() {
                o.insert("password".into(), json!(secret.expose()));
            }
            // `execute`, not `run`: Save Changes was asked when the open began. The command that
            // asked runs again, so a protected template still starts a new document (#3433).
            match app.execute(&then, p) {
                Ok(_) => app.ui.backstage = false,
                Err(e) => {
                    let error = if wordcraft_engine::io::wrong_password(&e) {
                        tl!("That password isn't right. Check it (passwords are case-sensitive) and try again.").to_string()
                    } else {
                        e
                    };
                    open_prompt(app, None, Ask::Open { name, then, params }, Some(error));
                }
            }
        }
        Ask::Encrypt => {
            let cleared = secret.expose().is_empty();
            let password = if cleared { Value::Null } else { json!(secret.expose()) };
            match app.run("file.encrypt", json!({"password": password})) {
                Ok(_) => app.status(if cleared {
                    tl!("The password is removed. Save the document to keep the change.")
                } else {
                    tl!("The document will be saved with a password. Save it to protect it.")
                }),
                Err(e) => {
                    open_prompt(app, None, Ask::Encrypt, Some(e));
                }
            }
        }
    }
    Settle::Done
}

/// Asks whether to discard recovery copy `id` of the document called `name`.
pub(crate) fn open_discard_recovery(app: &mut WordApp, id: &str, name: &str) -> DialogId {
    let prompt = StandardPrompt::custom(crate::i18n::fmt(tl!("Discard the recovery copy of {name}?"), &[("name", name)]))
        .title(tl!("Discard recovery copy"))
        .paragraph(tl!("This cannot be undone. Your original file stays unchanged."))
        .button(PromptChoice::Proceed, tl!("Discard"))
        .button(PromptChoice::Cancel, tl!("Cancel"))
        .instance(id.to_owned())
        .tag(id.to_owned());
    open_with(app, None, Keyed::new("discardRecovery", prompt), DialogHooks { prepare: None, settle: settle_discard })
}

/// The open discard-recovery prompt, for `ui.recoveryDiscard {answer}` to answer through the host.
pub(crate) fn pending_discard_recovery(app: &WordApp) -> Option<DialogId> {
    // PLAN: assumed `OpenDialog { id: DialogId, key: Option<String> }` (`open_dialogs()`).
    app.dialog_host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some("discardRecovery")).map(|d| d.id)
}

/// The only code that runs `file.recover {discard}`; the recovery id is the prompt's tag.
fn settle_discard(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(result) = app.dialog_host.take_by_id::<Keyed<StandardPrompt>>(id) else {
        return Settle::Keep;
    };
    if let DialogResult::Ok(mut answer) = result
        && matches!(answer.choice, PromptChoice::Proceed)
        && let Some(recovery) = answer.take_tag::<String>()
        && let Err(e) = app.execute("file.recover", json!({"discard": recovery}))
    {
        app.status(e);
    }
    Settle::Done
}

/// A save over a damaged DOCX that waits for the user: the document it is for, the path, and the
/// `file.save` parameters to repeat with `confirmDamaged` (the password is not kept).
struct DamagedSave {
    document: u64,
    path: String,
    params: Value,
}

/// Asks before a save overwrites the damaged DOCX at `path`. `shown` are the first omitted parts and
/// `more` how many others there are; Save Anyway repeats `params` with `confirmDamaged`.
pub(crate) fn open_damaged_save(app: &mut WordApp, path: String, shown: &[String], more: usize, document: u64, params: Value) {
    let mut text = wordcraft_engine::cmd::file::skipped_parts_warning(shown);
    if more > 0 {
        text.push(' ');
        text.push_str(&crate::i18n::tn(
            u64::try_from(more).unwrap_or(u64::MAX),
            "Also {n} additional part was omitted.",
            "Also {n} additional parts were omitted.",
        ));
    }
    let prompt = StandardPrompt::custom(tl!("Some parts of this document could not be read and will be missing from the saved file."))
        .title(tl!("Save damaged document?"))
        .paragraph(text)
        .button(PromptChoice::Proceed, tl!("Save Anyway"))
        .button(PromptChoice::Cancel, tl!("Cancel"))
        .tag(DamagedSave { document, path, params });
    let _ = open_with(app, None, Keyed::new("confirmDamagedSave", prompt), DialogHooks { prepare: None, settle: settle_damaged_save });
}

/// Opens the confirmation a `confirmDamagedSave` request asks for (a user's save over a damaged DOCX).
pub(crate) fn open_damaged_save_request(app: &mut WordApp, req: &Value) {
    let Some(path) = req.get("path").and_then(Value::as_str) else {
        return;
    };
    let shown: Vec<String> = req
        .get("skippedParts")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |parts| parts.iter().filter_map(Value::as_str).map(str::to_owned).collect());
    let more = req.get("skippedPartsMore").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let document = req.get("document").and_then(Value::as_u64).unwrap_or_else(|| app.session.document_id());
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
    open_damaged_save(app, path.to_owned(), &shown, more, document, params);
}

/// Save Anyway saves over the damaged file, and a save that ends a close operation lets it go on.
/// Cancel brings the Save Changes prompt back, when a close operation waits for it.
fn settle_damaged_save(app: &mut WordApp, _ctx: &egui::Context, id: DialogId) -> Settle {
    let Some(result) = app.dialog_host.take_by_id::<Keyed<StandardPrompt>>(id) else {
        return Settle::Keep;
    };
    let DialogResult::Ok(mut answer) = result else {
        return Settle::Done;
    };
    let Some(save) = answer.take_tag::<DamagedSave>() else {
        return Settle::Done;
    };
    if !matches!(answer.choice, PromptChoice::Proceed) {
        if app.closing.is_some() {
            let name = app.prompt_name();
            let message = crate::i18n::fmt(tl!("Saving {name} was cancelled, so it is still open and nothing was lost."), &[("name", &name)]);
            app.prompt_again(message);
        }
        return Settle::Done;
    }
    if app.session.document_id() != save.document {
        app.cancel_closing();
        app.status(tl!("The document changed while this confirmation was open. Save canceled.").to_string());
        return Settle::Done;
    }
    let mut params = save.params;
    if !params.is_object() {
        params = json!({});
    }
    params["path"] = json!(save.path);
    params["confirmDamaged"] = json!(true);
    match app.run("file.save", params) {
        Ok(result) if result.get("saved").and_then(Value::as_bool) == Some(true) => {
            if app.closing.is_some()
                && let Err(e) = app.continue_after_save(Some(Ok(result)), true)
            {
                app.status(e);
            }
        }
        Ok(_) => {
            app.prompt_again(tl!("The damaged document was not saved.").to_string());
        }
        Err(error) => {
            app.save_failed(&error);
        }
    }
    Settle::Done
}
