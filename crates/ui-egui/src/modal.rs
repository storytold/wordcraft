//! Optional document text editing through an installed Neovim process. Only the focused page
//! hands over its keyboard; dialogs and other fields retain their normal keyboard behaviour.

use crate::WordApp;
use serde_json::{Value, json};
use std::collections::VecDeque;
#[cfg(not(target_arch = "wasm32"))]
use wordcraft_doc::StoryRef;
#[cfg(not(target_arch = "wasm32"))]
use wordcraft_engine::cmd::modal_text::Projection;

#[cfg(not(target_arch = "wasm32"))]
mod neovim;

#[derive(Default)]
pub(crate) struct State {
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<neovim::Worker>,
    pending: bool,
    input_pending: bool,
    blocking: bool,
    applying: bool,
    queue: VecDeque<String>,
    deferred: VecDeque<(String, Value, bool)>,
    #[cfg(not(target_arch = "wasm32"))]
    disable_requested: bool,
    #[cfg(not(target_arch = "wasm32"))]
    before: String,
    #[cfg(not(target_arch = "wasm32"))]
    story: StoryRef,
    #[cfg(not(target_arch = "wasm32"))]
    document: u64,
    #[cfg(not(target_arch = "wasm32"))]
    revision: u64,
    #[cfg(not(target_arch = "wasm32"))]
    last_poll: Option<web_time::Instant>,
    #[cfg(not(target_arch = "wasm32"))]
    selection: Option<wordcraft_engine::Selection>,
    pub caret: Option<wordcraft_doc::Pos>,
    pub mode: String,
    pub command: String,
    pub error: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    history: Vec<(u64, u64)>,
    #[cfg(not(target_arch = "wasm32"))]
    history_at: usize,
}

impl WordApp {
    /// Before the framework classifies keys into word-processor commands. Native hosts call
    /// this once per raw-input batch; the public method also lets a test host take the same path.
    pub fn modal_raw_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.modal_poll(ctx);
        if !self.ui.modal_editing
            || !self.canvas.focused(ctx)
            || self.ui.backstage
            || self.modal_open()
            || self.session.view.read_mode
            || self.keytips_active
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        let mut keys = String::new();
        raw.events.retain(|event| {
            match event {
                egui::Event::Text(text) => {
                    keys.push_str(&text.replace('<', "<LT>"));
                    false
                }
                egui::Event::Key { key, pressed, modifiers, .. } => {
                    // An escape hatch remains a normal app command, even when the external
                    // editor is busy or its configuration has remapped every other key.
                    if modifiers.ctrl && modifiers.alt && modifiers.shift && *key == egui::Key::M {
                        return true;
                    }
                    if modifiers.mac_cmd {
                        return true;
                    }
                    // Keep the host's keyboard route out of the page and into its menus.
                    if !modifiers.ctrl && !modifiers.alt && matches!(key, egui::Key::F6 | egui::Key::F10) {
                        return true;
                    }
                    if *pressed && let Some(text) = key_text(*key, *modifiers) {
                        keys.push_str(&text);
                    }
                    false
                }
                // These accompany native clipboard key events; Ctrl+V belongs to the modal
                // editor (block selection), not to the word processor's Paste command.
                egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) => false,
                _ => true,
            }
        });
        if !keys.is_empty() {
            if keys.len() <= 65536 && self.canvas.modal.queue.len() < 128 {
                self.canvas.modal.queue.push_back(keys);
            } else {
                self.modal_fail("the modal input queue is full; typing was stopped".into());
            }
        }
        self.modal_send(ctx);
    }

    /// Framework-injected input (automation and accessibility) is classified after the native
    /// raw-input hook. The focused page takes that stream too, rather than typing command
    /// letters into the document. Physical events consumed by the raw hook never reach here.
    pub(crate) fn modal_editor_inputs(&mut self, inputs: &[craft::behavior::EditorInput], ctx: &egui::Context) -> bool {
        if !cfg!(unix) || !self.ui.modal_editing {
            return false;
        }
        let mut keys = String::new();
        for input in inputs {
            if let Some(text) = editor_text(input) {
                keys.push_str(&text);
            }
        }
        if !keys.is_empty() {
            if keys.len() <= 65536 && self.canvas.modal.queue.len() < 128 {
                self.canvas.modal.queue.push_back(keys);
            } else {
                self.modal_fail("the modal input queue is full; typing was stopped".into());
            }
        }
        self.modal_send(ctx);
        true
    }

    pub(crate) fn modal_command(&mut self, params: &Value) -> Result<Value, String> {
        if params.get("inspect").and_then(Value::as_bool) == Some(true) {
            return Ok(json!({"enabled":self.ui.modal_editing,"mode":self.canvas.modal.mode,
                "pending":self.canvas.modal.pending,"queued":self.canvas.modal.queue.len(),"error":self.canvas.modal.error}));
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = params;
            return Err("modal editing requires an installed desktop editor".into());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if !cfg!(unix) {
                return Err("modal editing is available on Unix desktop builds".into());
            }
            if let Some(directory) = params.get("configDirectory").and_then(Value::as_str) {
                self.ui.modal_config_directory = directory.to_string();
            }
            if let Some(keys) = params.get("keys").and_then(Value::as_str) {
                if !self.ui.modal_editing {
                    return Err("modal editing is off".into());
                }
                if keys.len() > 65536 || self.canvas.modal.queue.len() >= 128 {
                    return Err("modal input limit exceeded".into());
                }
                self.canvas.modal.queue.push_back(keys.to_string());
            } else {
                let enabled = params.get("value").and_then(Value::as_bool).unwrap_or(!self.ui.modal_editing);
                if !enabled && self.ui.modal_editing && self.canvas.modal.worker.is_some() {
                    self.canvas.modal.disable_requested = true;
                    self.canvas.modal.queue.push_back("<Esc>".into());
                    if let Some(ctx) = self.ctx.clone() {
                        self.modal_send(&ctx);
                    }
                    return Ok(json!({"enabled":true,"pending":true}));
                }
                if self.canvas.modal.pending || !self.canvas.modal.queue.is_empty() {
                    if enabled {
                        return Err("wait for the current modal edit before restarting the editor".into());
                    }
                    self.canvas.modal.disable_requested = true;
                    return Ok(json!({"enabled":true,"pending":true}));
                }
                self.ui.modal_editing = enabled;
                self.canvas.modal = State::default();
            }
            if let Some(ctx) = self.ctx.clone() {
                self.modal_send(&ctx);
            }
            Ok(json!({"enabled": self.ui.modal_editing,"mode": self.canvas.modal.mode,"pending":self.canvas.modal.pending,
                "error":self.canvas.modal.error}))
        }
    }

    pub(crate) fn modal_defer(&mut self, id: &str, params: &Value, user: bool) -> Result<bool, String> {
        if self.canvas.modal.applying
            || !self.ui.modal_editing
            || (!self.canvas.modal.pending && !self.canvas.modal.input_pending && self.canvas.modal.queue.is_empty())
        {
            return Ok(false);
        }
        if self.session.registry.get(id).is_none() && id != "ui.print" {
            return Ok(false);
        }
        if self.canvas.modal.deferred.len() >= 16 {
            return Err("too many commands are waiting for the modal editor".into());
        }
        if self.canvas.modal.blocking && self.canvas.modal.queue.back().is_none_or(|key| key != "<Esc>") {
            self.canvas.modal.queue.push_back("<Esc>".into());
        }
        self.canvas.modal.deferred.push_back((id.to_string(), params.clone(), user));
        Ok(true)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn modal_flush_deferred(&mut self) {
        while !self.canvas.modal.pending && !self.canvas.modal.input_pending && self.canvas.modal.queue.is_empty() {
            let Some((id, params, user)) = self.canvas.modal.deferred.pop_front() else {
                break;
            };
            let result = if user { self.run(&id, params) } else { self.execute(&id, params) };
            if let Err(error) = result {
                self.status_long(error);
            }
        }
    }

    fn modal_fail(&mut self, message: String) {
        self.canvas.modal = State { error: Some(message.clone()), ..Default::default() };
        self.ui.modal_editing = false;
        self.status_long(crate::i18n::fmt(tl!("Modal editing stopped: {error}"), &[("error", &message)]));
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn modal_send(&mut self, ctx: &egui::Context) {
        if !self.ui.modal_editing
            || self.canvas.modal.pending
            || !self.has_document()
            || (self.canvas.modal.disable_requested && !self.canvas.modal.input_pending && self.canvas.modal.queue.is_empty())
        {
            return;
        }
        let story = self.session.sel.focus.story;
        let projection = match Projection::new(&self.session.doc, story) {
            Ok(projection) => projection,
            Err(error) => {
                self.modal_fail(error.to_string());
                return;
            }
        };
        let document = self.session.document_id();
        let state = &mut self.canvas.modal;
        let sync = state.worker.is_none()
            || state.document != document
            || state.story != story
            || projection.text != state.before
            || state.revision != self.session.rev();
        let place = state.selection.as_ref() != Some(&self.session.sel);
        if state.queue.is_empty() && !sync && !place && state.last_poll.is_some_and(|last| last.elapsed() < std::time::Duration::from_millis(150)) {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
            return;
        }
        if state.worker.is_none() {
            match neovim::Worker::start(self.ui.modal_config_directory.clone(), ctx.clone()) {
                Ok(worker) => state.worker = Some(worker),
                Err(error) => {
                    self.modal_fail(error);
                    return;
                }
            }
        }
        let Some(focus) = projection.offset(&self.session.sel.focus) else {
            self.modal_fail("the caret is outside the editable text".into());
            return;
        };
        let mut keys = String::new();
        if !sync {
            while state.queue.front().is_some_and(|next| keys.len() + next.len() <= 65536) {
                if let Some(next) = state.queue.pop_front() {
                    keys.push_str(&next);
                }
            }
        }
        let keep_insert = sync && state.document == document && state.story == story && state.mode.starts_with('i');
        let request =
            json!({"keepInsert":keep_insert,"sync":sync,"place":place,"text":projection.text,"focus":focus,"keys":keys,"saved":!self.session.dirty});
        let result = state.worker.as_ref().map(|worker| worker.requests.try_send(request));
        if !matches!(result, Some(Ok(()))) {
            self.modal_fail("the external editor is unavailable".into());
            return;
        }
        state.before = projection.text;
        state.story = story;
        state.document = document;
        state.revision = self.session.rev();
        state.last_poll = Some(web_time::Instant::now());
        state.selection = Some(self.session.sel.clone());
        state.pending = true;
        if sync {
            state.history.clear();
            state.history_at = 0;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }

    #[cfg(target_arch = "wasm32")]
    fn modal_send(&mut self, _ctx: &egui::Context) {}

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn modal_poll(&mut self, ctx: &egui::Context) {
        let reply = self.canvas.modal.worker.as_ref().and_then(|worker| worker.replies.try_recv().ok());
        if let Some(reply) = reply {
            if let Ok(value) = &reply
                && let Some(query) = value.get("layoutQuery")
            {
                let answer = self.modal_layout_query(query);
                if let Some(worker) = &self.canvas.modal.worker {
                    let _ = worker.geometry.try_send(answer);
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(10));
                return;
            }
            self.canvas.modal.pending = false;
            match reply.and_then(|reply| self.modal_apply(reply)) {
                Ok(()) => {}
                Err(error) => {
                    self.modal_fail(error);
                    return;
                }
            }
            self.modal_flush_deferred();
            if self.canvas.modal.disable_requested && !self.canvas.modal.input_pending && self.canvas.modal.queue.is_empty() {
                self.ui.modal_editing = false;
                self.canvas.modal = State::default();
                return;
            }
            self.modal_send(ctx);
        }
        if !self.canvas.modal.pending && self.ui.modal_editing {
            self.modal_send(ctx);
        }
        if self.canvas.modal.pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn modal_poll(&mut self, _ctx: &egui::Context) {}

    /// A displayed-row mapping pauses inside Neovim while the drawing thread first applies
    /// its pending text, then measures the actual rich-document layout. This also works inside
    /// a macro: each request sees the preceding edit's reflow, without guessing a text width.
    #[cfg(not(target_arch = "wasm32"))]
    fn modal_layout_query(&mut self, query: &Value) -> Result<Value, String> {
        self.modal_apply(query["snapshot"].clone())?;
        let story = self.canvas.modal.story;
        let projection = Projection::new(&self.session.doc, story).map_err(|e| e.to_string())?;
        let at = query["at"].as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("invalid displayed-line position")?;
        let from = projection.pos(at).map_err(|e| e.to_string())?;
        let count = query["count"].as_u64().unwrap_or(1).clamp(1, 4096);
        let layout = self.session.layout();
        let mut page = self.session.page_hint;
        let mut to = from.clone();
        let action = query["action"].as_str().ok_or("missing displayed-line action")?;
        let mut goal = query["goal"].as_f64().filter(|x| x.is_finite()).map(|x| x as f32);
        if matches!(action, "first" | "last") {
            to = if action == "first" { self.session.doc.start_of(story) } else { self.session.doc.end_of(story) };
            for _ in 1..count {
                let Some((next, _)) = layout.vertical(&to, None, 1, page) else { break };
                to = next;
                if let Some(caret) = layout.caret_on(&to, page) {
                    page = caret.page;
                }
            }
            let focus = projection.offset(&to).ok_or("the displayed row is outside the story")?;
            return Ok(json!({"text":projection.text,"focus":focus}));
        }
        if matches!(action, "up" | "down") {
            for _ in 0..count {
                let Some((next, x)) = layout.vertical(&to, goal, if action == "up" { -1 } else { 1 }, page) else { break };
                goal = Some(x);
                if next == to {
                    break;
                }
                to = next;
                if let Some(caret) = layout.caret_on(&to, page) {
                    page = caret.page;
                }
            }
            let focus = projection.offset(&to).ok_or("the displayed row is outside the story")?;
            return Ok(json!({"text":projection.text,"focus":focus,"goal":goal}));
        }
        if action != "row" {
            return Err("unknown displayed-line action".into());
        }
        let (first, _) = layout.line_bounds(&from, page).ok_or("no displayed line at the caret")?;
        for _ in 1..count {
            let Some((next, _)) = layout.vertical(&to, None, 1, page) else { break };
            if next == to {
                break;
            }
            to = next;
            if let Some(caret) = layout.caret_on(&to, page) {
                page = caret.page;
            }
        }
        let (_, finish) = layout.line_bounds(&to, page).ok_or("no displayed line at the selection end")?;
        let (_, whole_end, last) = layout.line_span(&to, page).ok_or("no displayed line span at the caret")?;
        let start = projection.offset(&first).ok_or("invalid displayed line start")?;
        let finish = projection.offset(&finish).ok_or("invalid displayed line end")?;
        let mut erase_end = projection.offset(&whole_end).ok_or("invalid displayed line span")?;
        if last && projection.text.as_bytes().get(erase_end) == Some(&b'\n') {
            erase_end += 1;
        }
        Ok(json!({"text":projection.text,"start":start,"finish":finish,"eraseEnd":erase_end}))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn modal_apply(&mut self, reply: Value) -> Result<(), String> {
        let state = &self.canvas.modal;
        if state.document != self.session.document_id() {
            return Err("the active document changed during modal editing".into());
        }
        let projection = Projection::new(&self.session.doc, state.story).map_err(|e| e.to_string())?;
        if projection.text != state.before || state.revision != self.session.rev() {
            return Err("the document changed while the external editor was working".into());
        }
        if reply["busy"].as_bool() == Some(true) {
            self.canvas.modal.blocking = true;
            self.canvas.modal.input_pending = true;
            self.canvas.modal.mode = reply["mode"].as_str().unwrap_or("").to_string();
            return Ok(());
        }
        self.canvas.modal.blocking = false;
        let state = &self.canvas.modal;
        let text = reply.get("text").and_then(Value::as_str).ok_or("the editor returned no text")?;
        let anchor = reply["anchor"].as_u64().and_then(|v| usize::try_from(v).ok()).ok_or("invalid modal selection")?;
        let focus = reply["focus"].as_u64().and_then(|v| usize::try_from(v).ok()).ok_or("invalid modal selection")?;
        if !text.is_char_boundary(anchor) || !text.is_char_boundary(focus) {
            return Err("invalid modal selection".into());
        }
        let sequence = reply.get("sequence").and_then(Value::as_u64).ok_or("the editor returned no undo sequence")?;
        let old_selection = self.session.sel.clone();
        let text_changed = text != state.before;
        let target = state.history.iter().position(|(seq, _)| *seq == sequence);
        let history_at = state.history_at;
        if state.history.get(history_at).is_some_and(|(_, mark)| *mark != self.session.undo_mark()) {
            return Err("the document undo history changed outside the modal editor".into());
        }
        if target.is_none() && state.history.iter().any(|(seq, _)| *seq > sequence) {
            return Err("the editor requested an intermediate undo state that has no native rich-document snapshot".into());
        }
        let saved = self.session.edit_snapshot();
        if let Some(target) = target.filter(|target| *target != history_at) {
            let steps = target.abs_diff(history_at);
            let command = if target < history_at { "edit.undo" } else { "edit.redo" };
            for _ in 0..steps {
                if let Err(error) = self.session.run(command, &json!({})) {
                    self.session.restore(saved);
                    return Err(error.to_string());
                }
            }
            if Projection::new(&self.session.doc, state.story).map_err(|e| e.to_string())?.text != text {
                self.session.restore(saved);
                return Err("the document undo history changed outside the modal editor".into());
            }
            self.canvas.modal.history_at = target;
        } else if text != state.before {
            if target == Some(history_at) {
                self.session.join_next_undo();
            }
            let params = json!({"story":state.story,"before":state.before,"edits":reply["edits"],"anchor":reply["anchor"],"focus":reply["focus"]});
            self.canvas.modal.applying = true;
            let applied = self.run("text.applyBufferEdits", params);
            self.canvas.modal.applying = false;
            applied?;
            let state = &mut self.canvas.modal;
            if target.is_none() {
                state.history.truncate(state.history_at.saturating_add(1));
                state.history.push((sequence, self.session.undo_mark()));
                state.history_at = state.history.len().saturating_sub(1);
            }
        }
        let state = &mut self.canvas.modal;
        if state.history.is_empty() {
            state.history.push((sequence, self.session.undo_mark()));
        }
        // Selection follows the editor without creating an undo step.
        let projection = Projection::new(&self.session.doc, state.story).map_err(|e| e.to_string())?;

        let selection = wordcraft_engine::Selection {
            anchor: projection.pos(anchor).map_err(|e| e.to_string())?,
            focus: projection.pos(focus).map_err(|e| e.to_string())?,
        };
        if selection != self.session.sel {
            self.session.set_selection(selection);
        }
        state.selection = Some(self.session.sel.clone());
        let caret = reply["caret"].as_u64().and_then(|n| usize::try_from(n).ok()).unwrap_or(focus);
        state.caret = Some(projection.pos(caret).map_err(|e| e.to_string())?);
        state.before = projection.text;
        state.revision = self.session.rev();
        state.input_pending = reply["inputPending"].as_bool().unwrap_or(false);
        state.mode = reply["mode"].as_str().unwrap_or("").to_string();
        state.blocking = state.mode.starts_with("no") || state.mode.starts_with('c');
        state.input_pending |= state.blocking;
        state.command = reply["command"].as_str().unwrap_or("").to_string();
        self.canvas.scroll_to_caret |= text_changed || old_selection != self.session.sel;
        if reply["save"].as_bool() == Some(true) {
            if let Some(path) = reply["savePath"].as_str().filter(|path| !path.is_empty()) {
                self.run("file.saveAs", json!({"path":path}))?;
            } else {
                self.run("file.save", json!({}))?;
            }
        }
        if let Some(problem) = reply["problem"].as_str() {
            return Err(problem.to_string());
        }
        Ok(())
    }
}

fn editor_text(input: &craft::behavior::EditorInput) -> Option<String> {
    use craft::behavior::EditorInput;
    use craft::behavior::intent::{EditCommand, Granularity, NavMove};
    Some(match input {
        EditorInput::Text(text) => text.replace('<', "<LT>"),
        EditorInput::Newline => "<CR>".into(),
        EditorInput::Tab => "<Tab>".into(),
        EditorInput::BackTab => "<S-Tab>".into(),
        EditorInput::Cancel => "<Esc>".into(),
        EditorInput::Delete { forward, by, .. } => {
            format!("<{}{}>", if *by == Granularity::Word { "C-" } else { "" }, if *forward { "Del" } else { "BS" })
        }
        EditorInput::Move(mv, mods) => {
            let key = match mv {
                NavMove::Back => "Left",
                NavMove::Forward => "Right",
                NavMove::Up => "Up",
                NavMove::Down => "Down",
                NavMove::Start => "Home",
                NavMove::End => "End",
                NavMove::PageUp => "PageUp",
                NavMove::PageDown => "PageDown",
                _ => return None,
            };
            format!(
                "<{}{}{}>",
                if mods.extend { "S-" } else { "" },
                if matches!(mods.by, Granularity::Word | Granularity::Document) { "C-" } else { "" },
                key
            )
        }
        EditorInput::Edit(command) => match command {
            EditCommand::Undo => "u".into(),
            EditCommand::Redo => "<C-r>".into(),
            EditCommand::Copy => "<C-c>".into(),
            EditCommand::Cut => "<C-x>".into(),
            EditCommand::Paste => "<C-v>".into(),
            EditCommand::SelectAll => "ggVG".into(),
            EditCommand::Delete => "<Del>".into(),
            _ => return None,
        },
        _ => return None,
    })
}

fn key_text(key: egui::Key, mods: egui::Modifiers) -> Option<String> {
    use egui::Key;
    let special = match key {
        Key::Escape => "Esc",
        Key::Enter => "CR",
        Key::Backspace => "BS",
        Key::Delete => "Del",
        Key::Tab => "Tab",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::F11 | Key::F12 => {
            crate::keys::key_name(key)?
        }
        _ if mods.ctrl || mods.alt => crate::keys::key_name(key)?,
        _ => return None,
    };
    let mut prefix = String::new();
    if mods.ctrl {
        prefix.push_str("C-");
    }
    if mods.alt {
        prefix.push_str("A-");
    }
    if mods.shift {
        prefix.push_str("S-");
    }
    Some(format!("<{prefix}{special}>"))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use wordcraft_doc::{Document, Pos};
    use wordcraft_engine::Session;

    fn app(text: &str) -> WordApp {
        WordApp::new(Session::new(Document::from_text(text)), Default::default())
    }

    fn prime(app: &mut WordApp, sequence: u64) -> Result<(), wordcraft_engine::CmdError> {
        app.canvas.modal.document = app.session.document_id();
        app.canvas.modal.revision = app.session.rev();
        app.canvas.modal.before = Projection::new(&app.session.doc, StoryRef::Body)?.text;
        app.canvas.modal.history = vec![(sequence, app.session.undo_mark())];
        Ok(())
    }

    #[test]
    fn modal_undo_restores_the_native_rich_document() {
        let mut app = app("one MIDDLE three");
        app.session.run("select.range", &json!({"anchor":{"block":0,"off":4},"focus":{"block":0,"off":10}})).unwrap();
        app.session.run("format.bold", &json!({})).unwrap();
        let original = app.session.doc.clone();
        prime(&mut app, 0).unwrap();
        app.modal_apply(json!({"text":"1 MIDDLE 3","edits":[{"start":0,"end":3,"text":"1"},{"start":11,"end":16,"text":"3"}],
            "anchor":0,"focus":0,"mode":"n","command":"","sequence":1}))
            .unwrap();
        assert_eq!(app.session.doc.para_at(&Pos::body(0, 2)).unwrap().props_of_char(2).bold, Some(true));
        app.modal_apply(json!({"text":"one MIDDLE three","edits":[],"anchor":0,"focus":0,"mode":"n","sequence":0})).unwrap();
        assert!(app.session.doc.same_content(&original), "Undo restores rich runs, not just plain text");
        app.modal_apply(json!({"text":"1 MIDDLE 3","edits":[],"anchor":0,"focus":0,"mode":"n","sequence":1})).unwrap();
        assert_eq!(app.session.doc.plain_text(StoryRef::Body), "1 MIDDLE 3");
        assert_eq!(app.session.doc.para_at(&Pos::body(0, 2)).unwrap().props_of_char(2).bold, Some(true));
    }

    #[test]
    fn late_modal_reply_does_not_overwrite_newer_document_edits() {
        let mut app = app("original");
        prime(&mut app, 0).unwrap();
        app.session.run("text.insert", &json!({"text":"new "})).unwrap();
        let newer = app.session.doc.clone();
        assert!(app.modal_apply(json!({"text":"old reply","edits":[],"anchor":0,"focus":0,"sequence":1})).is_err());
        assert!(app.session.doc.same_content(&newer));
    }
    #[test]
    fn focused_page_routes_modal_keys_without_typing_them_into_the_document() {
        let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_eframe(|_| app("abc"));
        h.run_steps(3);
        let ctx = h.ctx.clone();
        h.state_mut().canvas.focus_page(&ctx);
        h.run_steps(3);
        assert!(h.state().canvas.focused(&ctx));
        let (worker, requests, replies) = neovim::Worker::fake();
        let a = h.state_mut();
        prime(a, 0).unwrap();
        a.ui.modal_editing = true;
        a.canvas.modal.worker = Some(worker);
        a.canvas.modal.selection = Some(a.session.sel.clone());
        a.canvas.modal.mode = "n".into();
        a.canvas.modal.last_poll = Some(web_time::Instant::now());
        h.event(egui::Event::Text("h".into()));
        let mut routed = false;
        for _ in 0..8 {
            h.step();
            if let Ok(request) = requests.try_recv() {
                routed |= request["keys"] == "h";
                replies.send(Ok(json!({"text":"abc","edits":[],"anchor":0,"focus":0,"sequence":0,"mode":"n"}))).unwrap();
            }
            if routed {
                break;
            }
        }
        assert!(routed, "the page gives the keystroke to the modal editor");
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), "abc");
        // A text field outside the canvas keeps its normal keyboard. The real focus is moved,
        // not merely a flag painted on the page.
        h.state_mut().run("ui.backstage", json!({"value":true,"page":"options"})).unwrap();
        h.step();
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Text("settings".into()));
        h.state_mut().modal_raw_input(&ctx, &mut raw);
        assert!(raw.events.iter().any(|event| matches!(event,egui::Event::Text(text) if text=="settings")));
    }

    #[test]
    fn displayed_row_queries_use_native_wrap_and_keep_the_paragraph_when_deleting_a_row() {
        let text = "Wide WWW letters, narrow iii letters, and café words. ".repeat(25);
        let mut app = app(&text);
        prime(&mut app, 0).unwrap();
        let snapshot = |text: &str, sequence| json!({"text":text,"edits":[],"anchor":0,"focus":0,"caret":0,"mode":"n","sequence":sequence});
        let row = app.modal_layout_query(&json!({"action":"row","at":0,"count":1,"snapshot":snapshot(&text,0)})).unwrap();
        let start = row["start"].as_u64().unwrap() as usize;
        let end = row["eraseEnd"].as_u64().unwrap() as usize;
        assert_eq!(start, 0);
        assert!(end > 0 && end < text.len(), "a displayed row is smaller than this wrapped paragraph");
        let down = app.modal_layout_query(&json!({"action":"down","at":0,"count":1,"snapshot":snapshot(&text,0)})).unwrap();
        let focus = down["focus"].as_u64().unwrap() as usize;
        assert!(focus >= end && focus < text.len(), "down moves within the same paragraph");
        let remaining = &text[end..];
        let next = app
            .modal_layout_query(&json!({"action":"row","at":0,"count":1,
            "snapshot":{"text":remaining,"edits":[{"start":0,"end":end,"text":""}],"anchor":0,"focus":0,"caret":0,"mode":"n","sequence":1}}))
            .unwrap();
        assert_eq!(app.session.doc.body.len(), 1, "deleting a wrapped row must not split or remove the paragraph");
        assert_eq!(app.session.doc.plain_text(StoryRef::Body), remaining);
        assert_eq!(next["text"], remaining, "a subsequent macro step measures the edited document's reflow");
        app.modal_apply(snapshot(&text, 0)).unwrap();
        assert_eq!(app.session.doc.plain_text(StoryRef::Body), text);
    }

    #[test]
    fn save_and_disable_wait_until_pending_text_is_applied() {
        let mut app = app("before");
        app.ui.modal_editing = true;
        prime(&mut app, 0).unwrap();
        app.canvas.modal.pending = true;
        assert!(app.run("file.save", json!({})).unwrap()["pending"].as_bool().unwrap());
        assert_eq!(app.canvas.modal.deferred.len(), 1);
        app.modal_command(&json!({"value":false})).unwrap();
        assert!(app.ui.modal_editing, "disabling must not discard the in-flight edit");
        assert!(app.canvas.modal.disable_requested);
    }
    #[test]
    fn a_waiting_operator_accepts_the_next_key_and_native_commands_cancel_it_before_saving() {
        let mut app = app("before");
        app.ui.modal_editing = true;
        prime(&mut app, 0).unwrap();
        app.modal_apply(json!({"busy":true,"mode":"no"})).unwrap();
        assert_eq!(app.session.doc.plain_text(StoryRef::Body), "before");
        assert!(app.canvas.modal.input_pending);
        app.modal_command(&json!({"keys":"w"})).unwrap();
        assert_eq!(app.canvas.modal.queue.front().map(String::as_str), Some("w"));
        assert_eq!(app.run("file.save", json!({})).unwrap()["pending"], true);
        assert_eq!(app.canvas.modal.queue.back().map(String::as_str), Some("<Esc>"));
        assert_eq!(app.canvas.modal.deferred.len(), 1);
    }

    #[test]
    fn idle_editor_replies_keep_pending_formatting_and_do_not_scroll_back_to_the_caret() {
        let mut app = app("plain");
        app.session.run("format.bold", &json!({})).unwrap();
        let pending = app.session.pending.clone();
        prime(&mut app, 0).unwrap();
        app.canvas.scroll_to_caret = false;
        app.modal_apply(json!({"text":"plain","edits":[],"anchor":0,"focus":0,"mode":"i","sequence":0})).unwrap();
        assert_eq!(app.session.pending, pending);
        assert!(!app.canvas.scroll_to_caret, "idle polling must not undo the user's scroll position");
    }
}
