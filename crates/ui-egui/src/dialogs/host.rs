//! The app's dialog driver over the framework's `DialogHost`: how a dialog opens, how the app sees what it
//! asked for, and the one form path every Word dialog goes through.
//!
//! A dialog is a framework `StackDialog` on `WordApp::dialog_host`. The app steps that need the whole app (running
//! a form's commands, opening a child, opening a link) are the dialog's *hooks*: `prepare` before the host runs
//! and `settle` after it, once per window pass. `settle` is where a dialog's finished result is taken, so no
//! answer is inferred from side effects.

use craft::color_ui::ui::{ColorPickerDialog, ColorPickerTab};
use craft::ui::contract::Bind;
use craft::ui::dialog::{ChildDialog, DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::patterns::form_dialogs::{AboutTab, FormButtons, FormCx, FormDialog, HostedAbout};
use craft::ui::runtime::env::Label;
use craft::ui::sections::FooterButton;
use egui::{Context, Ui};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use wordcraft_doc::props::Rgb;

use super::dialogs_insert::symbol::{SymbolDialog, SymbolPick, SymbolRequest};
use super::font::{FontForm, FontRequest};
use crate::WordApp;

/// What a settle hook says about itself after a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Settle {
    /// The hook still has work while its dialog is open; it runs again next pass.
    Keep,
    /// The hook is spent and is dropped. A hook says this only once its dialog is gone or it has taken the
    /// finished result (`take_by_id`), never while the dialog is open and may file again.
    Done,
}

/// The app steps of one open dialog.
#[derive(Clone, Copy)]
pub(crate) struct DialogHooks {
    /// Runs before every `DialogHost::run`, where live session data is copied into the dialog.
    pub prepare: Option<fn(&mut WordApp, DialogId)>,
    /// Runs after every `DialogHost::run`, also in the pass the dialog finished, so it can take the result.
    pub settle: fn(&mut WordApp, &Context, DialogId) -> Settle,
}

impl DialogHooks {
    /// Hooks with a `prepare` and nothing to settle.
    pub(crate) fn prepare_only(prepare: fn(&mut WordApp, DialogId)) -> DialogHooks {
        DialogHooks { prepare: Some(prepare), settle: keep }
    }
}

fn keep(_: &mut WordApp, _: &Context, _: DialogId) -> Settle {
    Settle::Keep
}

/// Hooks that are spent the first time they run: for a child whose parent takes its result.
const SPENT: DialogHooks = DialogHooks { prepare: None, settle: spent };

fn spent(_: &mut WordApp, _: &Context, _: DialogId) -> Settle {
    Settle::Done
}

/// Opens `dialog` on the app's host, as a child of `parent` when there is one, with its hooks.
pub(crate) fn open_with<D: StackDialog + 'static>(app: &mut WordApp, parent: Option<DialogId>, dialog: D, hooks: DialogHooks) -> DialogId {
    let id = open_node(app, parent, dialog);
    register(app, id, &[hooks]);
    id
}

fn open_node<D: StackDialog + 'static>(app: &mut WordApp, parent: Option<DialogId>, dialog: D) -> DialogId {
    match parent {
        Some(parent) => app.dialog_host.open_child(dialog, parent),
        None => app.dialog_host.open(dialog),
    }
}

/// Replaces the hooks of `id` with `entries`; the last entry settles first.
fn register(app: &mut WordApp, id: DialogId, entries: &[DialogHooks]) {
    app.dialog_hooks.retain(|(held, _)| *held != id);
    app.dialog_hooks.extend(entries.iter().map(|hooks| (id, *hooks)));
}

/// Runs every `prepare` hook; once per window pass, before `DialogHost::run`.
pub(crate) fn prepare(app: &mut WordApp) {
    let todo: Vec<(DialogId, fn(&mut WordApp, DialogId))> =
        app.dialog_hooks.iter().filter_map(|(id, hooks)| hooks.prepare.map(|run| (*id, run))).collect();
    for (id, run) in todo {
        run(app, id);
    }
}

/// Runs every `settle` hook, children before the dialogs they were opened from, then drops the hooks of dialogs that
/// are no longer open or said they are spent; once per window pass, after `DialogHost::run`.
pub(crate) fn settle(app: &mut WordApp, ctx: &Context) {
    let snapshot = app.dialog_hooks.clone();
    let mut done = vec![false; snapshot.len()];
    for (at, (id, hooks)) in snapshot.iter().enumerate().rev() {
        if let Some(flag) = done.get_mut(at) {
            *flag = (hooks.settle)(app, ctx, *id) == Settle::Done;
        }
    }
    let added = app.dialog_hooks.split_off(snapshot.len().min(app.dialog_hooks.len()));
    let open: Vec<DialogId> = app.dialog_host.open_dialogs().into_iter().map(|dialog| dialog.id).collect();
    app.dialog_hooks =
        snapshot.into_iter().zip(done).filter(|((id, _), finished)| !*finished && open.contains(id)).map(|(entry, _)| entry).chain(added).collect();
}

/// Cancels every open dialog, the top one first, lets their hooks take the cancelled results and forgets the ribbon's
/// More Colors targets, so none outlives its dialog.
pub(crate) fn close_all(app: &mut WordApp, ctx: &Context) {
    let ids: Vec<DialogId> = app.dialog_host.open_dialogs().into_iter().rev().map(|dialog| dialog.id).collect();
    for id in ids {
        app.dialog_host.cancel_by_id(ctx, id);
    }
    settle(app, ctx);
    // PLAN: assumed `WordApp::more_colors` with `clear()` (ribbon-w11, rb.A4's target map)
    app.more_colors.clear();
}

/// Opens the About dialog on `tab`. A second About while one is open opens nothing: the first keeps the input.
pub(crate) fn open_about(app: &mut WordApp, tab: AboutTab) -> Option<DialogId> {
    // PLAN: assumed `credits::about(tab: AboutTab) -> HostedAbout` (chrome-w12)
    let about = crate::credits::about(tab);
    let id = app.dialog_host.open_unless_open(about).ok()?;
    register(app, id, &[DialogHooks { prepare: None, settle: about_settle }]);
    Some(id)
}

fn about_settle(app: &mut WordApp, ctx: &Context, id: DialogId) -> Settle {
    if let Some(url) = app.dialog_host.get_mut_by_id::<HostedAbout>(id).and_then(HostedAbout::take_link) {
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }
    if app.dialog_host.take_by_id::<HostedAbout>(id).is_some() { Settle::Done } else { Settle::Keep }
}

/// What the form's body asks the app for in a pass; `settle` answers each.
pub(crate) enum Asks {
    /// Open the Symbol dialog over this form; its pick comes back as [`ChildAnswer::Symbol`].
    Symbol(SymbolRequest),
    /// Open the Font form over this form; its OK comes back as [`ChildAnswer::Font`].
    Font(FontRequest),
    /// Open More Colors for the form's colour `field`; its OK comes back as [`ChildAnswer::Color`].
    Color {
        /// The form's colour field.
        field: &'static str,
        /// The colour it shows now.
        current: Option<Rgb>,
        /// The tab the picker opens on.
        tab: ColorPickerTab,
    },
    /// Run a command at once.
    Run(String, Value),
    /// Open the dialog of the dg.A2 name over this form; a form that finishes with OK comes back as
    /// [`ChildAnswer::Form`].
    Open(String, Value),
    /// A body action that needs the app now; the form's `applied` gets [`Filed::Act`].
    Act(u32),
    /// The body accepts the dialog (a double click on a list row): files the held OK as OK's button does, so a
    /// disabled OK stays unreachable.
    Accept,
}

/// What a child dialog answered to the form that opened it.
pub(crate) enum ChildAnswer {
    /// The character the Symbol dialog picked.
    Symbol(SymbolPick),
    /// The Font form as it was confirmed.
    Font(Box<FontForm>),
    /// The colour More Colors confirmed for `field`, as `RRGGBB`.
    Color {
        /// The form's colour field.
        field: &'static str,
        /// The colour.
        hex: String,
    },
    /// A form of the named dialog, confirmed, as its serde value.
    Form {
        /// The dialog's name.
        key: String,
        /// The form.
        value: Value,
    },
}

/// Which filing ran the commands a form's `applied` follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filed {
    /// OK.
    Ok,
    /// The extra button at this index of `extra_buttons`.
    Extra(usize),
    /// A body action ([`Asks::Act`]).
    Act(u32),
}

/// The value behind a Word dialog. A dialog is its form, opened through [`open_form`]; OK runs the form's
/// commands, so an agent gets the same result without the dialog.
pub(crate) trait WordForm: Clone + Default + PartialEq + Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The dg.A2 name: the dialog's `key`, and the id is `dialog_id(key)`.
    fn key(&self) -> &'static str;
    /// The dialog's title.
    fn title(&self) -> Label;
    /// The accepting button's label when it is not OK ("Insert", "Mark").
    fn ok_label(&self) -> Option<Label> {
        None
    }
    /// Which of OK and Cancel, or Close alone, the footer shows.
    // PLAN: assumed FormButtons (dialogs-w5)
    fn buttons(&self) -> FormButtons {
        FormButtons::OkCancel
    }
    /// The second button's label when it is not the default.
    fn cancel_label(&self) -> Option<Label> {
        None
    }
    /// Whether the body is one collection (a picker) that owns the scroll, so the dialog's own body scroll is off.
    fn collection_body(&self) -> bool {
        false
    }
    /// Whether the rest of the window stays usable beside the dialog.
    fn modeless(&self) -> bool {
        false
    }
    /// One pass of the body; returns whether the value can be accepted, which is OK's one enabled rule.
    fn body(&mut self, ui: &mut Ui, cx: &mut FormCx<'_, '_>, asks: &mut Vec<Asks>) -> bool;
    /// Checks the filed form against the app before OK's commands run; an `Err` is the refusal the dialog shows and
    /// nothing runs.
    fn check(&self, app: &WordApp) -> Result<(), Label> {
        let _ = app;
        Ok(())
    }
    /// What OK runs; empty means nothing changed.
    fn commands(&self) -> Vec<(String, Value)>;
    /// Buttons after OK and Cancel.
    fn extra_buttons(&self) -> Vec<FooterButton> {
        Vec::new()
    }
    /// The extra button at `at` was picked, in the pass it was picked; may change the form and returns the commands to
    /// run, empty for none.
    fn extra(&mut self, at: usize) -> Vec<(String, Value)> {
        let _ = at;
        Vec::new()
    }
    /// A child dialog answered.
    fn answer(&mut self, answer: ChildAnswer) {
        let _ = answer;
    }
    /// Copies read-only app data (fonts, palette, unit, a preview) into the form's `#[serde(skip)]` fields; runs before
    /// every pass of the dialog.
    fn prepare(&mut self, app: &WordApp) {
        let _ = app;
    }
    /// Writes app state the body edits in place back to the app; runs after every pass and on the confirmed form in
    /// the pass the dialog finishes.
    fn write_back(&self, app: &mut WordApp) {
        let _ = app;
    }
    /// The app step after a filing ran, with what its commands returned or the error. The default posts the error of an
    /// extra button; an OK's error is the refusal the dialog already shows.
    fn applied(&mut self, app: &mut WordApp, filed: Filed, outcome: Result<&[Value], &str>) {
        if let (Filed::Extra(_), Err(error)) = (filed, outcome) {
            app.status(error.to_owned());
        }
    }
}

/// A picked extra button's commands, run by `settle`.
struct Pending {
    at: usize,
    commands: Vec<(String, Value)>,
}

/// How a hosted form finished.
pub(crate) struct FormDone<F> {
    /// The confirmed form.
    pub form: F,
    /// The dialog it was opened over.
    pub parent: Option<DialogId>,
    /// The extra button that closed the dialog: its commands run and the parent is told nothing. `None` is an OK.
    extra: Option<Pending>,
}

/// What a child asked of `settle` that is still to come.
#[derive(Clone, Copy)]
enum Child {
    Symbol(DialogId),
    Color(DialogId, &'static str),
}

/// A [`WordForm`] as a `StackDialog`: a `FormDialog` that holds its OK, so the commands run in `settle` and a refusal
/// shows in the dialog until the form changes.
pub(crate) struct FormHost<F: WordForm> {
    id: DialogId,
    form: F,
    held: HeldAnswer<F>,
    parent: Option<DialogId>,
    accepts: bool,
    pending: Vec<Pending>,
    asks: Vec<Asks>,
    children: Vec<Child>,
}

impl<F: WordForm> FormHost<F> {
    fn new(form: F, parent: Option<DialogId>) -> Self {
        FormHost {
            id: dialog_id(form.key()),
            form,
            held: HeldAnswer::new(),
            parent,
            accepts: false,
            pending: Vec::new(),
            asks: Vec::new(),
            children: Vec::new(),
        }
    }

    /// The form as it stands.
    pub(crate) fn form(&self) -> &F {
        &self.form
    }

    /// The form, to change it from a `prepare` hook or a test.
    pub(crate) fn form_mut(&mut self) -> &mut F {
        &mut self.form
    }
}

impl<F: WordForm> StackDialog for FormHost<F> {
    type Result = FormDone<F>;

    fn id(&self) -> DialogId {
        self.id
    }

    fn modeless(&self) -> bool {
        self.form.modeless()
    }

    fn key(&self) -> Option<&str> {
        Some(self.form.key())
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<FormDone<F>>> {
        let title = self.form.title();
        let ok_label = self.form.ok_label();
        let cancel_label = self.form.cancel_label();
        let buttons_kind = self.form.buttons();
        let buttons = self.form.extra_buttons();
        let collection = self.form.collection_body();
        let waiting = self.held.is_waiting();
        let FormHost { form, held, parent, accepts, pending, asks, .. } = self;
        let mut dialog = FormDialog::new(title, Bind::bound(&mut *form)).held(held).buttons(buttons_kind).collection_body(collection);
        if let Some(label) = ok_label {
            dialog = dialog.ok_label(label);
        }
        if let Some(label) = cancel_label {
            dialog = dialog.cancel_label(label);
        }
        if !buttons.is_empty() {
            dialog = dialog.extra_buttons(buttons);
        }
        let mut can_accept = false;
        let out = dialog.show(ui, |ui, cx, values| {
            can_accept = values.body(ui, cx, asks);
            can_accept && !waiting
        });
        *accepts = can_accept;
        let extra = out.picked.and_then(|picked| picked.checked_sub(2)).map(|at| Pending { at, commands: form.extra(at) });
        match (out.accepted, out.closed, extra) {
            (Some(accepted), _, extra) => {
                pending.extend(extra);
                Some(DialogResult::Ok(FormDone { form: accepted, parent: *parent, extra: None }))
            }
            // An extra button that also closes the dialog finishes it with the button's commands.
            (None, Some(_), Some(extra)) => Some(DialogResult::Ok(FormDone { form: form.clone(), parent: *parent, extra: Some(extra) })),
            (None, Some(_), None) => Some(DialogResult::Cancel),
            (None, None, extra) => {
                pending.extend(extra);
                None
            }
        }
    }

    fn confirm(&mut self, _ctx: &Context) -> Option<FormDone<F>> {
        if let Some(form) = self.held.finish() {
            return Some(FormDone { form, parent: self.parent, extra: None });
        }
        if self.accepts && !self.held.is_waiting() {
            self.held.file(self.form.clone());
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        serde_json::to_value(&self.form).ok()
    }

    fn set_json_field(&mut self, _ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        let mut fields = serde_json::to_value(&self.form).map_err(|e| e.to_string())?;
        let map = fields.as_object_mut().ok_or_else(|| "the dialog has no fields".to_string())?;
        if !map.contains_key(field) {
            return Err(format!("no field `{field}` in this dialog"));
        }
        map.insert(field.to_string(), value);
        self.form = serde_json::from_value(fields).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Opens `form` as a dialog, over `parent` when there is one.
pub(crate) fn open_form<F: WordForm>(app: &mut WordApp, parent: Option<DialogId>, form: F) -> DialogId {
    let id = open_node(app, parent, FormHost::new(form, parent));
    register(app, id, &[form_hooks::<F>()]);
    id
}

/// [`open_form`] with the caller's hooks: `hooks.prepare` runs before every host run, and the form's own settle runs
/// before `hooks.settle`.
pub(crate) fn open_form_with<F: WordForm>(app: &mut WordApp, parent: Option<DialogId>, form: F, hooks: DialogHooks) -> DialogId {
    let id = open_node(app, parent, FormHost::new(form, parent));
    register(app, id, &[hooks, form_hooks::<F>()]);
    id
}

fn form_hooks<F: WordForm>() -> DialogHooks {
    DialogHooks { prepare: Some(prepare_form::<F>), settle: form_settle::<F> }
}

/// Runs the form's `prepare` on a clone of the host's form and puts it back.
fn prepare_form<F: WordForm>(app: &mut WordApp, id: DialogId) {
    let Some(mut form) = current::<F>(app, id) else { return };
    form.prepare(app);
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<F>>(id) {
        *host.form_mut() = form;
    }
}

/// Runs `commands` as one undo step; if one fails the document and the undo stack are put back as they were and the
/// error is returned. Gives what each command returned, in order.
fn run_steps(app: &mut WordApp, commands: Vec<(String, Value)>) -> Result<Vec<Value>, String> {
    let snapshot = app.session.edit_snapshot();
    let mut values = Vec::new();
    for (i, (id, params)) in commands.into_iter().enumerate() {
        if i > 0 {
            app.session.join_next_undo();
        }
        match app.run(&id, params) {
            Ok(value) => values.push(value),
            Err(error) => {
                app.session.restore(snapshot);
                return Err(error);
            }
        }
    }
    Ok(values)
}

/// Runs a dialog's commands as one undo step. If one fails, puts the document and the undo stack back as they were
/// (nothing applied) and returns its error.
pub(crate) fn run_as_one_step(app: &mut WordApp, commands: Vec<(String, Value)>) -> Result<(), String> {
    run_steps(app, commands).map(|_| ())
}

fn outcome(ran: &Result<Vec<Value>, String>) -> Result<&[Value], &str> {
    ran.as_ref().map(Vec::as_slice).map_err(String::as_str)
}

fn current<F: WordForm>(app: &WordApp, id: DialogId) -> Option<F> {
    app.dialog_host.get_by_id::<FormHost<F>>(id).map(|host| host.form.clone())
}

/// Tells the open form that a filing ran, and keeps what `applied` changed in the dialog.
fn apply<F: WordForm>(app: &mut WordApp, id: DialogId, filed: Filed, outcome: Result<&[Value], &str>) {
    let Some(mut form) = current::<F>(app, id) else { return };
    form.applied(app, filed, outcome);
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<F>>(id) {
        *host.form_mut() = form;
    }
}

fn form_settle<F: WordForm>(app: &mut WordApp, ctx: &Context, id: DialogId) -> Settle {
    if let Some(result) = app.dialog_host.take_by_id::<FormHost<F>>(id) {
        if let DialogResult::Ok(mut done) = result {
            if let Some(Pending { at, commands }) = done.extra.take() {
                let ran = run_steps(app, commands);
                done.form.applied(app, Filed::Extra(at), outcome(&ran));
                ctx.request_repaint();
            } else if let Some(parent) = done.parent
                && let Ok(value) = serde_json::to_value(&done.form)
            {
                app.dialog_answers.push((parent, done.form.key().to_owned(), value));
                ctx.request_repaint();
            }
            done.form.write_back(app);
        }
        return Settle::Done;
    }
    let (filed, extras, asks, mut children) = {
        let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<F>>(id) else { return Settle::Keep };
        (host.held.take(), std::mem::take(&mut host.pending), std::mem::take(&mut host.asks), std::mem::take(&mut host.children))
    };
    if let Some(form) = current::<F>(app, id) {
        form.write_back(app);
    }
    let mut answers: Vec<ChildAnswer> = Vec::new();

    if let Some(filed) = filed {
        let refusal = filed.check(app).err();
        let ran = if refusal.is_none() { Some(run_steps(app, filed.commands())) } else { None };
        let answer = match refusal {
            Some(reason) => Err(reason),
            None => match &ran {
                Some(Err(error)) => Err(Label::from(error.clone())),
                _ => Ok(()),
            },
        };
        if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<F>>(id) {
            host.held.answer(answer);
        }
        if let Some(ran) = ran {
            apply::<F>(app, id, Filed::Ok, outcome(&ran));
        }
        ctx.request_repaint();
    }
    for Pending { at, commands } in extras {
        let ran = run_steps(app, commands);
        apply::<F>(app, id, Filed::Extra(at), outcome(&ran));
        ctx.request_repaint();
    }
    for ask in asks {
        match ask {
            Asks::Run(command, params) => {
                if let Err(error) = app.run(&command, params) {
                    app.status(error);
                }
            }
            Asks::Act(n) => apply::<F>(app, id, Filed::Act(n), Ok(&[])),
            Asks::Accept => {
                if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<F>>(id)
                    && host.accepts
                    && !host.held.is_waiting()
                {
                    let form = host.form.clone();
                    host.held.file(form);
                }
            }
            Asks::Open(name, params) => {
                super::route(app, &name, &params, Some(id));
            }
            Asks::Font(request) => {
                let form = request.into_form(app);
                open_form(app, Some(id), form);
            }
            Asks::Symbol(request) => {
                let child = open_with(app, Some(id), SymbolDialog::for_pick(request), SPENT);
                children.push(Child::Symbol(child));
            }
            Asks::Color { field, current, tab } => {
                let picker = crate::colors::more_colors(current, field.to_string(), tab);
                let child = open_with(app, Some(id), picker, SPENT);
                children.push(Child::Color(child, field));
            }
        }
        ctx.request_repaint();
    }

    // PLAN: assumed `WordApp::dialog_answers: Vec<(DialogId, String, Value)>` beside `dialog_hooks` (app-hub, dg.A3)
    let (mailed, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut app.dialog_answers).into_iter().partition(|(parent, ..)| *parent == id);
    app.dialog_answers = rest;
    let font_key = FontForm::default().key();
    for (_, key, value) in mailed {
        if key == font_key
            && let Ok(form) = serde_json::from_value::<FontForm>(value.clone())
        {
            answers.push(ChildAnswer::Font(Box::new(form)));
        } else {
            answers.push(ChildAnswer::Form { key, value });
        }
    }
    let mut waiting = Vec::new();
    for child in children {
        match child {
            Child::Symbol(child_id) => match app.dialog_host.take_by_id::<SymbolDialog>(child_id) {
                Some(DialogResult::Ok(Some(pick))) => answers.push(ChildAnswer::Symbol(pick)),
                Some(_) => {}
                None => waiting.push(child),
            },
            Child::Color(child_id, field) => match app.dialog_host.take_by_id::<ColorPickerDialog>(child_id) {
                Some(DialogResult::Ok(result)) => {
                    let c = result.picked.color;
                    let channel = |v: f32| (v * 255.0).round() as u8;
                    let hex = Rgb(channel(c.r), channel(c.g), channel(c.b)).hex();
                    answers.push(ChildAnswer::Color { field, hex });
                }
                Some(_) => {}
                None => waiting.push(child),
            },
        }
    }
    if let Some(host) = app.dialog_host.get_mut_by_id::<FormHost<F>>(id) {
        host.children.extend(waiting);
        for answer in answers {
            host.form.answer(answer);
        }
    }
    Settle::Keep
}

/// A framework dialog that reports no key, opened under its dg.A2 name so `ui.dialogState` and tests can tell it.
pub(crate) struct Keyed<D: StackDialog> {
    key: &'static str,
    id: DialogId,
    dialog: D,
}

impl<D: StackDialog> Keyed<D> {
    /// Wraps `dialog`, whose id becomes `dialog_id(key)`.
    pub(crate) fn new(key: &'static str, dialog: D) -> Self {
        Keyed { key, id: dialog_id(key), dialog }
    }

    /// The wrapped dialog.
    #[cfg(test)]
    pub(crate) fn inner(&self) -> &D {
        &self.dialog
    }

    /// The wrapped dialog, to change it.
    pub(crate) fn inner_mut(&mut self) -> &mut D {
        &mut self.dialog
    }
}

impl<D: StackDialog> StackDialog for Keyed<D> {
    type Result = D::Result;

    fn id(&self) -> DialogId {
        self.id
    }

    fn modeless(&self) -> bool {
        self.dialog.modeless()
    }

    fn show(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<DialogResult<D::Result>> {
        self.dialog.show(ui, stack)
    }

    fn key(&self) -> Option<&str> {
        Some(self.key)
    }

    fn instance(&self) -> Option<DialogId> {
        self.dialog.instance()
    }

    fn take_child(&mut self) -> Option<ChildDialog> {
        self.dialog.take_child()
    }

    fn confirm(&mut self, ctx: &Context) -> Option<D::Result> {
        self.dialog.confirm(ctx)
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.dialog.held_state()
    }

    fn canvas(&self) -> Option<egui::Id> {
        self.dialog.canvas()
    }

    fn json_fields(&self) -> Option<Value> {
        self.dialog.json_fields()
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        self.dialog.set_json_field(ctx, field, value)
    }

    fn cancelled(&mut self, ctx: &Context) {
        self.dialog.cancelled(ctx);
    }

    fn preview_color(&mut self, key: &str, color: Option<craft::theme::Srgba>) {
        self.dialog.preview_color(key, color);
    }
}

/// Helpers for trials that open and confirm a hosted form.
#[cfg(test)]
pub(crate) mod testing {
    use egui_kittest::Harness;

    use super::{DialogId, WordApp};

    /// A harness that runs the app's logic and UI each frame in a 1440×900 window.
    pub(crate) fn harness(app: WordApp) -> Harness<'static, WordApp> {
        crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app)
    }

    /// Confirms the dialog `id` as OK does and runs the passes it takes to file the held OK and finish: the held OK
    /// files on the first confirm after a shown pass and finishes on a later one. Callers assert on the document or the
    /// open dialogs, never on the confirm's return.
    pub(crate) fn confirm(h: &mut Harness<'static, WordApp>, id: DialogId) {
        let ctx = h.ctx.clone();
        let _ = h.state_mut().dialog_host.confirm_by_id(&ctx, id);
        h.run_steps(3);
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::contract::Show as _;
    use craft::ui::sections::DialogRole;
    use egui_kittest::kittest::Queryable;
    use serde::Deserialize;
    use serde_json::json;
    use wordcraft_engine::Session;

    use super::*;

    /// A form whose one extra button closes the dialog and inserts text.
    #[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
    struct Closing {}

    impl WordForm for Closing {
        fn key(&self) -> &'static str {
            "testClosing"
        }
        fn title(&self) -> Label {
            Label::from("Closing".to_string())
        }
        fn body(&mut self, ui: &mut Ui, _cx: &mut FormCx<'_, '_>, _asks: &mut Vec<Asks>) -> bool {
            // The dialog body runs more than once a frame (measure, then draw): a framework component
            // keeps its identity across the passes, where a bare egui label clashes with itself.
            craft::ui::display::Text::new("Body").show(ui);
            true
        }
        fn commands(&self) -> Vec<(String, Value)> {
            Vec::new()
        }
        fn extra_buttons(&self) -> Vec<FooterButton> {
            vec![FooterButton::new("Remove", DialogRole::Other)]
        }
        fn extra(&mut self, _at: usize) -> Vec<(String, Value)> {
            vec![("text.insert".to_string(), json!({"text": "gone"}))]
        }
    }

    /// An extra footer button runs its commands, and it answers no parent.
    #[test]
    fn a_closing_extra_button_runs_its_commands() {
        let app = WordApp::new(Session::new(wordcraft_doc::Document::new()), crate::Services::default());
        let mut h = testing::harness(app);
        open_form(h.state_mut(), None, Closing::default());
        h.run_steps(4);
        h.get_by_label("Remove").click();
        h.run_steps(4);
        let text = h.state().session.doc.plain_text(wordcraft_doc::StoryRef::Body);
        assert!(text.contains("gone"), "the button's command ran: {text:?}");
        // The framework's form dialog keeps the dialog open on every extra button (`FormDialog::extra_buttons`):
        // only OK or Cancel closes it.
        assert!(!h.state().dialog_host.is_empty(), "an extra button leaves the dialog open");
        assert!(h.state().dialog_answers.is_empty(), "no parent was answered");
    }
}
