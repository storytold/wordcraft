use super::{
    DefaultApp, MAX_RECENT, SaveChoice, SaveInFlight, Value, WordApp, applied_font, async_saves, chrome, colors, crop, dialogs, dialogs_table_style,
    file_dialogs, i18n, json, keeps_everything, keytips, now_ms, options, panes, paste_picture, printing, push_recent_font, system_clipboard,
    table_pen, theme, widgets, zotero,
};

impl WordApp {
    /// `run` for a command a mouse drag on a table sends: the view stays where it is.
    pub(crate) fn run_keep_view(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.canvas.keep_view = true;
        let r = self.run(id, params);
        self.canvas.keep_view = false;
        r
    }

    /// Run a command as the input method's own transaction while preserving every normal
    /// [`Self::run`] side effect (save prompts, clipboard, body-epoch tracking, status, …).
    ///
    /// The origin is restored on every return, including errors and nested commands.
    pub(crate) fn run_ime(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let previous = std::mem::replace(&mut self.ime_origin, true);
        let result = self.run(id, params);
        self.ime_origin = previous;
        result
    }

    /// Run a command the user asked for (ribbon, shortcut, Backstage, file drop). New, Open,
    /// Close, Envelopes, Labels and Finish & Merge on a document with unsaved changes first ask
    /// Save / Don't Save / Cancel, and the command runs once that is answered (`ui.saveChanges`).
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // Comment text still being typed is written first, so a save or export has it and a
        // close, New or Open counts it as a change (#1348). A draft the document refuses is not
        // part of it: saving and exporting go on without it (the refusal stays on the status
        // line), and replacing the document asks Save / Don't Save / Cancel about it.
        let refused = if flushes_comment_drafts(id, &params) { panes::flush_comment_drafts(self).err() } else { None };
        if id == "file.recover" && params.as_object().is_none_or(|p| p.is_empty()) {
            self.ui.backstage = true;
            self.ui.backstage_page = "recover".into();
        }
        // A button whose command needs input the user hasn't given yet opens its dialog (#240).
        if let Some(name) = input_dialog(id, &params)
            && widgets::enabled(self, id)
        {
            self.dialog = dialogs::Dialog::open(name, self);
            return Ok(json!({"pending": name}));
        }
        if (self.session.dirty || self.save_unconfirmed() || refused.is_some())
            && discards_document(id, &params)
            && !self.closes_only_its_window(id, &params)
        {
            let name = self.prompt_name();
            self.dialog = Some(dialogs::Dialog::SaveChanges {
                name,
                then: id.to_string(),
                params,
                document: self.session.document_id(),
                message: String::new(),
            });
            return Ok(json!({"pending": "saveChanges"}));
        }
        // The ribbon and context-menu Paste commands send no text: read the system clipboard's, as
        // Ctrl+V's paste event carries it (#584), and its HTML and RTF, which egui's paste event
        // doesn't carry (#687).
        let mut params = params;
        if let Some(r) = system_clipboard::fill(self, id, &mut params) {
            return r;
        }
        // Paste without system clipboard text: a picture there is newer than our own copy (copying
        // here replaces it with text), so it wins (#45).
        if id == "edit.paste"
            && params.get("text").is_none()
            && let Some(r) = paste_picture::paste(self)
        {
            return r;
        }
        let applied = applied_font(id, &params);
        let r = self.execute_user(id, params);
        // A save error already on the status line is the reason to see; the refusal shows otherwise.
        if let (Some(e), true) = (refused, r.is_ok()) {
            self.status(e);
        }
        if let (Ok(_), Some(name)) = (&r, applied) {
            push_recent_font(&mut self.ui.recent_fonts, &name);
        }
        if matches!(id, "insert.picture" | "insert.icon") {
            self.select_inserted_picture(&r);
        }
        // review.spelling only moves the selection to the next issue; without this, the ribbon's
        // Spelling & Grammar and Editor buttons looked broken — just a selection with no way to
        // see or apply a suggestion (#3340). Reuse the context menu's suggestions, as Shift+F10
        // already does for the word at the caret.
        if id == "review.spelling"
            && let Ok(v) = &r
            && v.get("done").and_then(serde_json::Value::as_bool) != Some(true)
        {
            self.canvas.context_menu_from_keys = true;
        }
        let r = system_clipboard::explain(self, id, r);
        // Check Accessibility shows its findings in the Accessibility pane, kept up to date there.
        if id == "file.accessibility" && r.is_ok() {
            self.session.view.accessibility_pane = true;
        }
        // Match Fields and Check for Errors show what they found.
        if let Ok(v) = &r
            && let Some(d) = dialogs::Dialog::report(id, v)
        {
            self.dialog = Some(d);
        }
        r
    }

    /// Run a command for a script or an agent (control channel, MCP bridge): never asks first.
    /// UI-level commands (`ui.*`) are handled here, the rest by the engine.
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.clear_command_targets(id);
        if let Some(error) = self.document_required_error(id) {
            return Err(error);
        }
        if let Some(r) = self.ui_command(id, &params) {
            return r;
        }
        if id == "file.close" {
            return self.close_document(&params);
        }
        if id == "file.autosave" {
            return self.set_autosave(&params);
        }
        if let Some(r) = table_pen::toggle(self, id, &params) {
            return r;
        }
        let ctx = self.ctx.clone();
        if let Some(r) = zotero::command(self, id, &params, ctx.as_ref()) {
            if let Err(e) = &r {
                self.status(e.clone());
            }
            return r;
        }
        // Web: saving and exporting become downloads.
        if self.services.download.is_some() && matches!(id, "file.save" | "file.saveAs" | "file.exportPdf" | "file.exportPng") {
            let name = params.get("path").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| self.web_save_name(id));
            let name = if id == "file.exportPdf" && !name.ends_with(".pdf") { format!("{name}.pdf") } else { name };
            // A save with a password encrypts this and later saves (`file.save`'s `password`).
            if matches!(id, "file.save" | "file.saveAs")
                && let Some(pw) = params.get("password").filter(|v| !v.is_null())
            {
                if !wordcraft_engine::io::can_encrypt(&name) || pw.as_str().is_none_or(str::is_empty) {
                    return Err("only Word documents (.docx, .docm, .dotx, .dotm) can be saved with a password, and it can't be empty".into());
                }
                self.session.run("file.encrypt", &json!({"password": pw})).map_err(|e| e.to_string())?;
            }
            // A save is a save, download or not: advance the revision and modified stamp (#262).
            if matches!(id, "file.save" | "file.saveAs") {
                self.session.stamp_save();
            }
            // Word documents are encrypted with the document's password, if it has one.
            let password =
                if matches!(id, "file.save" | "file.saveAs") { self.session.password.as_ref().map(wordcraft_engine::Password::as_str) } else { None };
            let opts = wordcraft_engine::cmd::file::export_options(&self.session, &params);
            let bytes = wordcraft_engine::io::save_bytes_with(&name, &self.session.doc, password, &opts)?;
            // A ticket left over from an earlier call isn't this save's.
            let saves = self.services.async_saves.clone();
            if let Some(saves) = &saves {
                let _ = saves.take_started();
            }
            // An export is a copy, and so is a save in a format that doesn't keep everything (a
            // page image, plain text): the document itself still has unsaved changes.
            let marked_saved = matches!(id, "file.save" | "file.saveAs") && keeps_everything(&name);
            // Save As tells the host page the document is now this file, which it only is when the
            // save marks it saved. A Save under a name the page never sent (an untitled document,
            // an imported .doc saved as .docx) is a new file too, never an overwrite.
            let opened_name = self.session.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string());
            let new_file = id == "file.saveAs" || opened_name.as_deref() != Some(name.as_str());
            let delivered = match (&self.services.host_save, &self.services.download) {
                (Some(host), _) => host(&name, &bytes, marked_saved && new_file),
                (None, Some(download)) => download(&name, &bytes),
                (None, None) => Ok(()),
            };
            if let Err(e) = delivered {
                let msg = i18n::fmt(tl!("Couldn't download {name}: {error}"), &[("name", &name), ("error", &e)]);
                log::error!("download of {name} failed: {e}");
                self.status(msg.clone());
                return Err(msg);
            }
            if marked_saved {
                self.session.dirty = false;
                // The host page keeps the file under this name now, as a desktop save would.
                if self.services.host_save.is_some() {
                    self.session.path = Some(std::path::PathBuf::from(&name));
                }
            }
            // The browser's picker writes the file later (#1315): until it says how that went,
            // the document's save isn't confirmed.
            if let Some(ticket) = saves.as_ref().and_then(async_saves::AsyncSaves::take_started) {
                let document = self.session.document_id();
                self.saves_in_flight.push(SaveInFlight { ticket, name: name.clone(), document, marked_saved });
                return Ok(json!({"downloaded": name, "bytes": bytes.len(), "pendingSave": ticket}));
            }
            return Ok(json!({"downloaded": name, "bytes": bytes.len()}));
        }
        let document = self.session.document_id();
        let body_epoch = self.session.ime_body_epoch();
        let math_session = self.session.math.as_ref().map(|m| m.session);
        let composing_before = !self.canvas.ime_preedit.is_empty();
        let old_path = self.session.path.clone();
        let r = if self.ime_origin {
            self.session.run_ime(id, &params).map_err(|e| e.to_string())
        } else {
            self.session.run(id, &params).map_err(|e| e.to_string())
        };
        if self.session.ime_body_epoch() != body_epoch {
            self.canvas.ime_interrupted_preedit |= composing_before;
            // The body (or the whole document) was replaced under the same id — setText, a
            // version restore, compare, Undo/Redo. A composition belongs to the old content:
            // drop its pre-edit and ask the platform to interrupt it, even when the new bounded
            // window happens to be identical.
            self.canvas.ime_preedit.clear();
            self.canvas.ime_preedit_range = None;
            self.canvas.ime_window_cache = None;
            self.canvas.ime_interrupt_now = true;
        }
        if self.session.math.as_ref().map(|m| m.session) != math_session {
            self.canvas.ime_interrupted_preedit |= composing_before;
            // Entering, leaving or replacing the edited equation changes the IME target's
            // replacement epoch under the same document id. The old composition belongs to a
            // tree this session no longer edits.
            self.canvas.ime_preedit.clear();
            self.canvas.ime_preedit_range = None;
            self.canvas.ime_interrupt_now = true;
        }
        // Copy and Cut put what they took on the system clipboard, whichever control ran them
        // (the ribbon, a context menu, the native menu, or the Cmd+C/X event). Only the key event
        // used to write it, so a Cut button deleted the words without copying them.
        if matches!(id, "edit.copy" | "edit.cut")
            && let Ok(v) = &r
            && let Some(ctx) = &ctx
        {
            // Its HTML too when it has some (an equation as MathML, #816).
            crate::system_clipboard::put_result(self, ctx, v);
        }
        // Another document is shown (not when Open only showed its picker): the window's state
        // for the one it showed is put aside.
        if self.session.document_id() != document {
            // A requested New/Open wins over the asynchronous startup recovery scan.
            // Older copies remain available through File > Recover.
            self.recovery_seen = true;
            self.sync_document();
        }
        // An explicit save to the document's own file lets AutoSave keep writing there.
        if let Ok(v) = &r
            && matches!(id, "file.save" | "file.saveAs")
            && v.get("saved").and_then(Value::as_bool) == Some(true)
            && v.get("path").and_then(Value::as_str).map(std::path::Path::new) == self.session.path.as_deref()
        {
            self.autosave_path = self.session.path.clone();
        }
        // A renamed file is the same file: AutoSave keeps covering it, and Recent drops the old name.
        if r.is_ok()
            && id == "file.rename"
            && let Some(old) = old_path.filter(|old| self.session.path.as_ref() != Some(old))
        {
            if self.autosave_path.as_ref() == Some(&old) {
                self.autosave_path = self.session.path.clone();
            }
            let old = old.to_string_lossy();
            self.ui.recent.retain(|r| *r != old);
        }
        if let Ok(v) = &r {
            self.keep_web_font(id, &params, v);
        }
        self.after_command(id);
        if let Err(e) = &r {
            self.status(e.clone());
        }
        r
    }

    /// Web: store a font added from bytes, or forget the removed ones ([`Services::store_font`]).
    fn keep_web_font(&self, id: &str, params: &Value, result: &Value) {
        match id {
            "font.add" => {
                if let (Some(store), Some(file), Some(data)) =
                    (&self.services.store_font, result.get("file").and_then(Value::as_str), params.get("data").and_then(Value::as_str))
                    && let Some(bytes) = wordcraft_engine::cmd::insert::base64_decode(data)
                {
                    store(file, &bytes);
                }
            }
            "font.remove" => {
                if let Some(forget) = &self.services.forget_font {
                    for f in result.get("removed").and_then(Value::as_array).into_iter().flatten() {
                        if let Some(file) = f.get("file").and_then(Value::as_str) {
                            forget(file);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// User commands can ask for a supported Save As target or an opening password (#55).
    /// Scripts and agents use [`WordApp::execute`], which never opens these dialogs.
    pub(crate) fn execute_user(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // Legacy Word files are import-only (#2445). Only a user's implicit Save asks for
        // a supported target; scripts and explicitly supplied paths keep their normal errors.
        if id == "file.save"
            && params.get("path").and_then(Value::as_str).is_none()
            && self.has_document()
            && self.legacy_word_path()
            && self.services.download.is_none()
        {
            return match self.save_as(file_dialogs::AfterSave::Nothing) {
                file_dialogs::Asked::Done(r) => r,
                file_dialogs::Asked::Pending => Ok(json!({"pending": "saveAs"})),
                file_dialogs::Asked::Busy => Ok(json!({"saved": false})),
            };
        }
        let retry = (id == "file.open").then(|| params.clone());
        let r = self.execute(id, params);
        if let (Err(e), Some(params)) = (&r, retry)
            && wordcraft_engine::io::needs_password(e)
        {
            let path = params.get("path").and_then(Value::as_str).unwrap_or("");
            let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
            self.dialog = Some(dialogs::Dialog::Password { name, params, password: Default::default(), message: String::new() });
            return Ok(json!({"pending": "password"}));
        }
        r
    }

    fn after_command(&mut self, id: &str) {
        options::sync(self);
        self.canvas.caret_visible_since = now_ms();
        if id == "view.outline" {
            self.ui.nav_tab = "headings".into();
        }
        if matches!(id, "view.readColumnWidth" | "view.readTextSize" | "view.readLineSpacing") {
            self.canvas.read_force_reflow = true;
        }
        if id == "view.readMode" {
            self.canvas.read_first_visible = None;
        }
        // Drawing leaves the caret where it was: following it would pull the view off the ink.
        // A mouse drag on a table (`run_keep_view`) does the same (#1738). A caret in an empty
        // story (a footer just made) also spans the whole story: it isn't a selection of it, and
        // the view follows it. Saving, exporting, printing, the document's properties and File ›
        // Options move neither the caret nor the text, so the view stays where the user is
        // reading (#3864); AutoSave runs its save on the session and never gets here.
        let file_keeps_view = matches!(
            id,
            "file.save"
                | "file.saveAs"
                | "file.saveTemplate"
                | "file.exportPdf"
                | "file.exportPng"
                | "file.printPdf"
                | "file.print"
                | "file.printMarkup"
                | "file.properties"
                | "file.info"
                | "file.rename"
                | "file.privacy"
                | "file.privacyOptions"
                | "file.setAuthor"
                | "file.compatibility"
                | "file.encrypt"
                | "file.share"
                | "file.options"
        );
        if (self.session.selects_whole_story() && !self.session.sel.is_collapsed()) || id == "select.all" || self.canvas.keep_view {
            self.canvas.scroll_to_caret = false;
        } else if !file_keeps_view && !["ui.", "view.", "document.", "format.state", "draw."].iter().any(|p| id.starts_with(p)) {
            self.canvas.scroll_to_caret = true;
        }
        for req in std::mem::take(&mut self.session.ui_requests) {
            self.handle_request(&req);
        }
        if let Some(p) = self.session.path.as_ref().map(|p| p.to_string_lossy().to_string())
            && self.ui.recent.first() != Some(&p)
        {
            self.ui.recent.retain(|r| *r != p);
            self.ui.recent.insert(0, p);
            self.ui.recent.truncate(MAX_RECENT);
        }
        if !self.session.status.is_empty() {
            // Engine messages are English; the catalog translates the ones it knows.
            let s = std::mem::take(&mut self.session.status);
            let s = tl!(&s).to_string();
            self.status(s);
        }
        if matches!(id, "table.deleteTable")
            || (self.canvas.table_tool == Some(crate::table_pen::TableTool::Erase) && self.session.sel.focus.path.cell().is_none())
        {
            crate::table_pen::stop(self);
        }
    }

    /// Requests commands make of the UI (open a dialog…).
    pub(crate) fn handle_request(&mut self, req: &Value) {
        // Made for a document that isn't shown (an agent's command run there): its window handles
        // it when it draws.
        if let Some(document) = req.get("document").and_then(Value::as_u64)
            && document != self.session.document_id()
        {
            let app_wide = req.get("interfaceTheme").is_some() || req.get("toggle").and_then(Value::as_str) == Some("interfaceTheme");
            if app_wide || req.get("close").is_some() || !self.queue_for_window(document, req) {
                self.handle_request_for(document, req);
            }
            return;
        }
        // Ctrl+F: type straight into the Navigation pane's search box (#712).
        if req.get("focus").and_then(Value::as_str) == Some("navSearch") {
            self.nav_search_focus = true;
            self.ui.nav_tab = "results".into();
        }
        // #1861: the registered Interface Theme command drives the same setting as Options.
        if let Some(name) = req.get("interfaceTheme").and_then(Value::as_str)
            && let Some(a) = theme::Appearance::from_code(name)
        {
            self.ui.theme = a;
        }
        if req.get("toggle").and_then(Value::as_str) == Some("interfaceTheme") {
            self.ui.theme = if self.ui_is_dark() { theme::Appearance::Light } else { theme::Appearance::Dark };
        }
        if let Some(what) = req.get("open").and_then(Value::as_str) {
            match what {
                "openFile" => self.open_dialog(),
                "saveAs" => {
                    self.save_as_dialog();
                }
                "exportPdf" => self.export_dialog("pdf"),
                "exportPng" => self.export_dialog("png"),
                "insertPicture" => self.pick_picture(),
                "addFont" => self.pick_font(),
                "loadSystemFonts" => self.load_system_fonts(),
                "print" => {
                    self.ui.backstage = true;
                    self.ui.backstage_page = "print".into();
                }
                "options" => {
                    self.ui.backstage = true;
                    self.ui.backstage_page = "options".into();
                    if let Some(pane) = req.get("pane").and_then(Value::as_str) {
                        self.ui.options_pane = pane.into();
                    }
                }
                "pasteSpecial" => self.dialog = Some(dialogs::Dialog::paste_special(self, req)),
                "newWindow" => self.new_window(),
                other => self.dialog = dialogs::Dialog::open(other, self),
            }
        }
        if req.get("close").is_some() {
            self.quit_requested = true;
        }
    }

    /// A request made for document `document`, which isn't shown and which no other window
    /// handles: the interface theme is the app's and changes; Close closes that document if it
    /// has nothing unsaved; the rest (dialogs, panes, focus, scrolling) are about a window's page
    /// and are dropped, as no window shows the document.
    fn handle_request_for(&mut self, document: u64, req: &Value) {
        let theme_only = req.get("interfaceTheme").is_some() || req.get("toggle").and_then(Value::as_str) == Some("interfaceTheme");
        if theme_only {
            let mut req = req.clone();
            if let Some(o) = req.as_object_mut() {
                o.remove("document");
            }
            self.handle_request(&req);
            return;
        }
        if req.get("close").is_some() {
            let dirty = self.session.documents().iter().any(|d| d.id == document && d.dirty);
            if dirty {
                log::warn!("not closing document {document}: it has unsaved changes");
            } else if let Err(e) = self.session.close_document(document) {
                log::warn!("closing document {document}: {e}");
            }
            self.sync_document();
            return;
        }
        log::warn!("dropped a request for document {document}, which no window shows: {req}");
    }

    /// Commands that live in the UI layer.
    pub(crate) fn ui_command(&mut self, id: &str, p: &Value) -> Option<Result<Value, String>> {
        let s = |k: &str| p.get(k).and_then(Value::as_str);
        Some(Ok(match id {
            "ui.checkForUpdates" | "help.checkForUpdates" => {
                crate::updates_ui::check(self);
                Value::Null
            }
            "ui.tab" => {
                let tab = s("tab").unwrap_or("Home").to_string();
                if tab == "File" {
                    self.ui.backstage = true;
                } else {
                    self.ui.tab = tab;
                    self.ui.backstage = false;
                }
                json!({"tab": self.ui.tab})
            }
            "ui.backstage" => {
                self.ui.backstage = p.get("value").and_then(Value::as_bool).unwrap_or(!self.ui.backstage);
                if let Some(pg) = s("page") {
                    self.ui.backstage_page = pg.into();
                }
                if let Some(pane) = s("pane") {
                    self.ui.options_pane = pane.into();
                }
                json!({"backstage": self.ui.backstage})
            }
            "ui.dialog" if s("name") == Some("colors") => {
                // The font colour's More Colors… (#606); `tab`: standard or custom.
                let target = colors::Target::cmd("format.color").remember(colors::Remember::FontColor);
                let current =
                    self.session.run("format.state", &json!({})).ok().and_then(|st| st.get("color").and_then(Value::as_str).and_then(colors::parse));
                let mut d = colors::ColorsDialog::new(current, target);
                if s("tab") == Some("custom") {
                    d.tab = colors::ColorsTab::Custom;
                }
                self.colors = Some(d);
                json!({"dialog": "colors"})
            }
            "ui.dialog" => {
                self.dialog = dialogs::Dialog::open(s("name").unwrap_or(""), self);
                // New / Modify Table Style can open on a part (`"part": "firstRow"`…).
                if let Some(dialogs::Dialog::TableStyle { region, .. }) = &mut self.dialog
                    && let Some(part) = s("part").and_then(wordcraft_engine::cmd::table_style::part_named)
                {
                    *region = dialogs_table_style::part_index(part);
                }
                json!({"dialog": self.dialog.as_ref().map(|d| d.name())})
            }
            "ui.closeDialog" => {
                self.dialog = None;
                self.colors = None;
                json!({})
            }
            "ui.changePicture" => {
                let Some(media) = self.selected_picture_media() else {
                    return Some(Err("select a picture first".into()));
                };
                self.change_picture_target = Some(media);
                self.pick_picture();
                json!({"pending": self.change_picture_target.is_some()})
            }
            // The page's context menu at the caret, as Shift+F10 or the Menu key opens it (#821).
            "ui.contextMenu" => {
                self.canvas.context_menu_from_keys = true;
                self.canvas.want_focus = true;
                json!({"pending": "contextMenu"})
            }
            "ui.paneDock" => return Some(panes::dock_command(self, p)),
            "ui.collapseRibbon" => {
                self.ui.ribbon_collapsed = !self.ui.ribbon_collapsed;
                json!({"collapsed": self.ui.ribbon_collapsed})
            }
            "ui.focusRibbon" => {
                // Focus's "⋯" (#1678): `value` true drops the ribbon down over the page, false
                // puts it away; no value toggles it. Outside Focus there is no ribbon to drop:
                // it stays put away.
                let focus = chrome::Screen::of(&self.session.view) == chrome::Screen::Focus;
                match p.get("value") {
                    None => self.focus_ribbon = focus && !self.focus_ribbon,
                    Some(Value::Bool(on)) => self.focus_ribbon = focus && *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"focusRibbon": self.focus_ribbon})
            }
            "ui.quickAccessToolbar" => {
                // Hide Quick Access Toolbar (its menu) and Show Quick Access Toolbar (the ribbon's
                // right-click menu, Customize Ribbon), #1945: `value` true shows the toolbar in
                // the title bar, false hides it; no value toggles it.
                match p.get("value") {
                    None => self.ui.quick_access_toolbar = !self.ui.quick_access_toolbar,
                    Some(Value::Bool(on)) => self.ui.quick_access_toolbar = *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"quickAccessToolbar": self.ui.quick_access_toolbar})
            }
            "ui.keytips" => {
                if let Some(v) = p.get("value") {
                    let Some(enabled) = v.as_bool() else { return Some(Err("`value` must be a boolean".into())) };
                    self.ui.keytips_enabled = enabled;
                    if !enabled {
                        self.ui.keytips = keytips::Phase::Off;
                        self.ui.keytip_prefix.clear();
                    }
                }
                json!({"enabled": self.ui.keytips_enabled})
            }
            "ui.dark" => {
                // A manual Light/Dark switch (toggles what is shown when no value is given).
                let dark = p.get("value").and_then(Value::as_bool).unwrap_or(!self.ui_is_dark());
                self.ui.theme = if dark { theme::Appearance::Dark } else { theme::Appearance::Light };
                json!({"dark": dark, "theme": self.ui.theme.code()})
            }
            "ui.theme" => {
                // `light`, `dark` or `system` (follow the OS appearance); no value reads it.
                if let Some(v) = s("value") {
                    match theme::Appearance::from_code(v) {
                        Some(a) => self.ui.theme = a,
                        None => return Some(Err(format!("unknown theme {v:?}; use light, dark or system"))),
                    }
                }
                json!({"theme": self.ui.theme.code(), "dark": self.ui_is_dark()})
            }
            "ui.interfaceSize" => {
                // `auto` (the display scale the system reports) or an absolute size in percent,
                // 50–300 (clamped); no value reads it. `percent` in the result is the size the
                // interface is drawn at. Document zoom is `view.zoom` and stays as it is.
                if let Some(v) = p.get("percent").or_else(|| p.get("value")) {
                    let size = match v.as_str() {
                        Some(a) if a.trim().eq_ignore_ascii_case("auto") => Some(theme::InterfaceSize::AUTO),
                        Some(t) => t.trim().trim_end_matches('%').trim().parse::<f64>().ok().and_then(theme::InterfaceSize::from_percent),
                        None => v.as_f64().and_then(theme::InterfaceSize::from_percent),
                    };
                    match size {
                        Some(size) => {
                            self.ui.interface_size = size;
                            self.legacy_interface_size = None;
                        }
                        None => {
                            return Some(Err(format!(
                                "`percent` must be \"auto\" or a number from {} to {}",
                                theme::InterfaceSize::MIN,
                                theme::InterfaceSize::MAX
                            )));
                        }
                    }
                }
                let size = self.ui.interface_size;
                json!({"auto": size.is_auto(), "percent": size.drawn_percent(self.native_scale)})
            }
            "ui.animations" => {
                // `system` (follow the system's animation setting), `on` or `off`; no value reads
                // it. `enabled` is what that comes to now, `system` what the system reports.
                if let Some(v) = s("value") {
                    match theme::Animations::from_code(v) {
                        Some(a) => self.ui.animations = a,
                        None => return Some(Err(format!("unknown animations setting {v:?}; use system, on or off"))),
                    }
                }
                self.read_system_animations();
                json!({"animations": self.ui.animations.code(), "enabled": self.animations_on(), "system": self.system_animations})
            }
            "ui.language" => {
                // `auto` (follow the system) or a language code; anything else is an error.
                if let Some(v) = s("value") {
                    match i18n::normalize_pref(v) {
                        Some(code) => self.ui.language = code.to_string(),
                        None => {
                            let codes: Vec<&str> = i18n::Lang::all().map(i18n::Lang::code).collect();
                            return Some(Err(format!("unknown language `{v}`; use auto or one of {}", codes.join(", "))));
                        }
                    }
                }
                let lang = i18n::Lang::from_pref(&self.ui.language);
                json!({"language": self.ui.language, "effective": lang.code(), "available": i18n::Lang::all().map(|l| json!({"code": l.code(), "name": l.name()})).collect::<Vec<_>>()})
            }
            "ui.recoveryDiscard" => {
                if let Some(answer) = s("answer") {
                    let Some(dialogs::Dialog::DiscardRecovery { id, .. }) = self.dialog.clone() else {
                        return Some(Err("no recovery discard is pending".into()));
                    };
                    match answer {
                        "cancel" => {
                            self.dialog = None;
                            json!({"discarded": false})
                        }
                        "discard" => {
                            let result = self.execute("file.recover", json!({"discard": id}));
                            if result.is_ok() {
                                self.dialog = None;
                            }
                            return Some(result);
                        }
                        _ => return Some(Err("`answer` must be discard or cancel".into())),
                    }
                } else {
                    let Some(id) = s("id") else {
                        return Some(Err("`id` is required".into()));
                    };
                    let Some(info) = self.session.recovery_documents().into_iter().find(|d| d.id == id) else {
                        return Some(Err("no such recovery copy".into()));
                    };
                    self.dialog = Some(dialogs::Dialog::DiscardRecovery { id: info.id, name: info.name });
                    json!({"pending": "discardRecovery"})
                }
            }
            "ui.saveChanges" => {
                let Some(choice) = s("answer").and_then(SaveChoice::parse) else {
                    return Some(Err("`answer` must be save, dontSave or cancel".into()));
                };
                return Some(self.answer_save_changes(choice));
            }
            "ui.openFileDialog" => {
                self.open_dialog();
                json!({})
            }
            // File › Save As › Save as Word Template (#2852): the Save dialog for a .dotx, in the
            // user templates folder. Opens system UI, like `ui.openFileDialog`.
            "ui.saveAsTemplate" => {
                let dir = self.save_as_template_dialog();
                json!({"dir": dir})
            }
            // The built-in file browser (#576), for when the system's file dialog doesn't show up
            // (desktop only): `purpose` is document (default), picture, recipients or save. A system
            // dialog still waiting for an answer is given up as cancelled.
            #[cfg(not(target_arch = "wasm32"))]
            "ui.fileBrowser" => {
                self.browse_builtin(s("purpose").unwrap_or("document"));
                json!({"open": self.file_browser.open.is_some()})
            }
            // Mailings › Select Recipients › Use an Existing List…: pick a CSV/TSV/text file and
            // load it as the mail-merge recipients. Opens system UI, like `ui.openFileDialog`.
            "ui.openRecipientList" => {
                self.pick_recipient_list();
                json!({"pending": self.recipient_list_pending})
            }
            // Native printing submits the selected queue on a worker; the browser owns its dialog.
            "ui.printSettings" => return Some(printing::settings(&mut self.printing, p, self.services.printers.as_ref())),
            "ui.print" | "ui.printSubmit" => return Some(self.print_to_system(p)),
            // File › Options › General › Make WordCraft the default… changes the user's system
            // default apps (Linux) or opens system settings (Windows), so only a click on that
            // button does it ([`WordApp::make_default_app`]); scripts, the control channel and
            // MCP get an error.
            "ui.makeDefaultApp" => {
                return Some(Err(
                    "making WordCraft the default app needs a click in File › Options › General; scripts and agents can't change system defaults"
                        .into(),
                ));
            }
            "ui.discord" => {
                self.canvas.open_url = Some("https://discord.gg/artcraft".into());
                json!({})
            }
            "ui.caretAnimation" => {
                // File › Options › Advanced › Animation: `value` true or false turns the caret's
                // glide and fading blink on or off; no value reads it.
                match p.get("value") {
                    None => {}
                    Some(Value::Bool(on)) => self.ui.caret_animation = *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"caretAnimation": self.ui.caret_animation})
            }
            "ui.alignmentGuides" => {
                // File › Options › Advanced › Editing: `value` true or false turns snapping to
                // alignment guides while moving pictures and shapes on or off; no value reads it.
                match p.get("value") {
                    None => {}
                    Some(Value::Bool(on)) => self.ui.alignment_guides = *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"alignmentGuides": self.ui.alignment_guides})
            }
            "ui.startScreen" => {
                // File › Options › General › Start-up (#766): `value` true or false turns showing
                // File › Home at start-up on or off; no value reads it.
                match p.get("value") {
                    None => {}
                    Some(Value::Bool(on)) => self.ui.start_screen = *on,
                    Some(_) => return Some(Err("`value` must be true or false".into())),
                }
                json!({"startScreen": self.ui.start_screen})
            }
            // Shape Format › Text › Create Link: the next click on an empty text box links the
            // selected box to it (`shape.link` with that box as the target); Esc cancels.
            // Picture Format › Size › Crop: crop mode on the selected picture, or off (`crop`).
            "ui.crop" => match crop::toggle(self) {
                Ok(v) => v,
                Err(e) => return Some(Err(e)),
            },
            "ui.linkTextBox" => {
                let from = wordcraft_engine::cmd::objects::text_box_target(&self.session).filter(|_| widgets::enabled(self, "shape.link"));
                let Some((pos, _)) = from else { return Some(Err("select a text box that isn't linked yet first".into())) };
                self.canvas.link_from = Some(pos);
                self.status(tl!("Click the empty text box the text should continue in (Esc cancels)"));
                json!({"pending": "link"})
            }
            _ => return None,
        }))
    }

    /// Submit the selected print job; the implementation lives beside its localized controls.
    fn print_to_system(&mut self, p: &Value) -> Result<Value, String> {
        self.print_document = Some(self.session.document_id());
        printing::submit(self, p)
    }

    /// File › Options › General › Make WordCraft the default…: hand over to the host's
    /// [`Services::make_default_app`] and say in the status bar what happened. An error when the
    /// host has no hook (web, macOS). Only the button calls this: it is deliberately not a
    /// command, so nothing programmatic (control channel, MCP, macros) can change system defaults.
    pub(crate) fn make_default_app(&mut self) -> Result<Value, String> {
        let Some(hook) = self.services.make_default_app.as_ref() else {
            return Err("WordCraft can't change the default apps here; use your system's settings".into());
        };
        match hook() {
            Ok(DefaultApp::SettingsOpened) => {
                self.status(tl!("In Default apps, choose WordCraft for the document types you want it to open."));
                Ok(json!({"settingsOpened": true}))
            }
            Ok(DefaultApp::MadeDefault) => {
                self.status(tl!("WordCraft is now the default app for Word documents."));
                Ok(json!({"default": true}))
            }
            Err(e) => {
                log::warn!("make default app: {e}");
                let msg = i18n::fmt(tl!("Couldn't make WordCraft the default: {error}"), &[("error", &e)]);
                self.status(msg.clone());
                Err(msg)
            }
        }
    }

    /// Pick a recipient list (desktop: through the file dialog hook; web: it arrives through the
    /// inbox).
    fn pick_recipient_list(&mut self) {
        if let Some(f) = &self.services.open_async {
            f("recipients");
            self.recipient_list_pending = true;
            return;
        }
        let _ = self.ask_file(file_dialogs::FileDialogRequest::Open { purpose: "recipients".into() }, file_dialogs::AfterPick::Recipients);
    }

    /// Load mail-merge recipients and say how many there are. A workbook with several sheets
    /// shows Select Table to choose one first (#334).
    pub(crate) fn load_recipients(&mut self, params: Value) -> Result<Value, String> {
        let r = self.run("mailings.recipients", params.clone());
        if let Ok(v) = &r
            && v.get("chooseSheet").and_then(Value::as_bool) == Some(true)
        {
            let sheets = v
                .get("sheets")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            self.dialog = Some(dialogs::Dialog::SelectTable { sheets, selected: 0, headers: true, message: String::new(), params });
        } else if let Ok(v) = &r {
            let records = v.get("records").and_then(Value::as_u64).unwrap_or(0).to_string();
            self.status(i18n::fmt(tl!("Recipients: {count}"), &[("count", &records)]));
        }
        r
    }

    fn clear_command_targets(&mut self, id: &str) {
        // A pending Change Picture only survives until the next action (#147).
        if !matches!(id, "ui.changePicture" | "picture.change" | "insert.picture") {
            self.change_picture_target = None;
        }
        if !matches!(id, "ui.openRecipientList" | "mailings.recipients") {
            self.recipient_list_pending = false;
        }
    }

    fn document_required_error(&mut self, id: &str) -> Option<String> {
        // No document open (#1129): only the commands that open or create one, Help, and settings.
        if self.has_document() || works_without_document(id) {
            return None;
        }
        let error = tl!("No document is open. Create a document or open one first.").to_string();
        self.status(error.clone());
        Some(error)
    }
}

/// The dialog a user-run command opens when it lacks the input it needs (scripts and agents get
/// the command's own error or default instead): Select Recipients and Edit Recipient List without
/// data, Insert Merge Field without a field, Find Recipient without text, the If and Skip
/// Record If rules (or Rules with no rule at all) without a field, Finish & Merge and Print
/// Documents without records, Table Properties without settings, Symbol and Field without a
/// character or code, and Enclose Characters without a shape.
fn input_dialog(id: &str, params: &Value) -> Option<&'static str> {
    input_dialog_mailings(id, params)
        .or_else(|| input_dialog_insert(id, params))
        .or_else(|| input_dialog_table(id, params))
        .or_else(|| input_dialog_references(id, params))
        .or_else(|| input_dialog_other(id, params))
}

fn has_value(params: &Value, key: &str) -> bool {
    params.get(key).is_some_and(|value| !value.is_null())
}

fn no_settings(params: &Value) -> bool {
    params.as_object().is_none_or(|fields| fields.is_empty())
}

fn input_dialog_mailings(id: &str, params: &Value) -> Option<&'static str> {
    match id {
        "mailings.rules" if !has_value(params, "field") => match params.get("rule").and_then(Value::as_str).map(str::to_ascii_uppercase).as_deref() {
            None | Some("IF") => Some("ruleIf"),
            Some("SKIPIF") => Some("ruleSkipIf"),
            _ => None,
        },
        "mailings.recipients" if !["csv", "path", "data", "rows"].into_iter().any(|key| has_value(params, key)) => Some("recipientList"),
        "mailings.editRecipients" if !has_value(params, "rows") => Some("recipientList"),
        "mailings.insertField" if !has_value(params, "field") => Some("insertMergeField"),
        "mailings.findRecipient" if !has_value(params, "text") => Some("findRecipient"),
        "mailings.finish" if !["path", "records", "from", "to"].into_iter().any(|key| has_value(params, key)) => Some("mergeRecords"),
        "mailings.printDocuments" if !["records", "from", "to"].into_iter().any(|key| has_value(params, key)) => Some("printRecords"),
        "mailings.envelopes" if no_settings(params) => Some("envelopes"),
        "mailings.labels" if no_settings(params) => Some("labels"),
        _ => None,
    }
}

fn input_dialog_insert(id: &str, params: &Value) -> Option<&'static str> {
    match id {
        "insert.symbol" if !has_value(params, "char") => Some("symbol"),
        "insert.field" if !has_value(params, "instr") => Some("field"),
        "insert.smartArt" if !has_value(params, "layout") && !has_value(params, "items") => Some("insertSmartArt"),
        "insert.chart" if !has_value(params, "type") && !has_value(params, "data") => Some("insertChart"),
        "chart.type" if !has_value(params, "type") => Some("changeChartType"),
        "chart.editData" if !["data", "categories", "series"].into_iter().any(|key| has_value(params, key)) => Some("chartData"),
        "insert.bookmark" | "insert.deleteBookmark" if !has_value(params, "name") => Some("bookmark"),
        "format.enclose" if params.get("shape").is_none() => Some("enclose"),
        "insert.crossReference" if !has_value(params, "target") => Some("crossReference"),
        _ => None,
    }
}

fn input_dialog_table(id: &str, params: &Value) -> Option<&'static str> {
    match id {
        "table.properties" if no_settings(params) => Some("tableProperties"),
        "table.cellMargins" if no_settings(params) => Some("tableCellMargins"),
        "table.formula" if !has_value(params, "formula") => Some("tableFormula"),
        "table.sort" if no_settings(params) => Some("tableSort"),
        "table.split" if no_settings(params) => Some("splitCells"),
        "table.fromText" if no_settings(params) => Some("textToTable"),
        "table.toText" if no_settings(params) => Some("tableToText"),
        "para.tabs" if no_settings(params) => Some("tabs"),
        "para.borders" if no_settings(params) => Some("borders"),
        "design.pageBorders" if no_settings(params) => Some("pageBorders"),
        _ => None,
    }
}

fn input_dialog_references(id: &str, params: &Value) -> Option<&'static str> {
    match id {
        "references.noteOptions" if no_settings(params) => Some("noteOptions"),
        "references.caption" if no_settings(params) => Some("caption"),
        "references.index" if no_settings(params) => Some("index"),
        "references.tableOfFigures" if no_settings(params) => Some("tableOfFigures"),
        "references.markEntry" if !has_value(params, "entry") => Some("markEntry"),
        "references.sources" if no_settings(params) => Some("sourceManager"),
        "references.citation" if !has_value(params, "tag") && !has_value(params, "source") => Some("createSource"),
        _ => None,
    }
}

fn input_dialog_other(id: &str, params: &Value) -> Option<&'static str> {
    match id {
        "file.rename" if !has_value(params, "name") => Some("renameDocument"),
        "file.encrypt" if params.get("password").is_none() => Some("encryptPassword"),
        "file.protect" | "review.restrict" if no_settings(params) => Some("restrictEditing"),
        "fields.setCode" if !has_value(params, "code") => Some("editFieldCode"),
        "picture.size" if no_settings(params) => Some("pictureSize"),
        "list.define" if params.get("levels").is_none() => Some("defineList"),
        "review.trackingOptions" if no_settings(params) => Some("trackChangesOptions"),
        "review.compare" if !has_value(params, "path") && !has_value(params, "text") => Some("compare"),
        "review.combine" if !has_value(params, "path") && !has_value(params, "text") => Some("combine"),
        "review.wordCount" if no_settings(params) => Some("wordCount"),
        "review.language" if no_settings(params) => Some("language"),
        "tools.autocorrect" if no_settings(params) => Some("autoCorrect"),
        "tools.templates" => Some("templates"),
        "styles.organizer" if !has_value(params, "path") => Some("organizer"),
        "developer.properties" if no_settings(params) => Some("controlProperties"),
        _ => None,
    }
}

/// Commands that work while no document is open (#1129): opening or creating one, closing the
/// window, Help, adding fonts (a font file dropped on the empty window), and the settings (File ›
/// Options, the view and interface preferences it sets, Customize Ribbon and Keyboard).
/// Everything else needs a document, `ui.*` included: only the interface commands listed here
/// (tabs, Backstage, dialogs, appearance, opening files) work without one.
pub(crate) fn works_without_document(id: &str) -> bool {
    matches!(
        id,
        "file.new"
            | "file.newFromTemplate"
            | "file.listTemplates"
            | "file.open"
            | "file.close"
            | "file.options"
            | "file.setAuthor"
            | "font.add"
            | "font.list"
            | "font.remove"
            | "readAloud.speed"
            | "readAloud.skipCitations"
            | "tools.customizeKeyboard"
            | "tools.customizeRibbon"
            | "view.darkMode"
            | "view.marks"
            | "view.ruler"
            | "view.numeral"
            | "ui.tab"
            | "ui.backstage"
            | "ui.dialog"
            | "ui.closeDialog"
            | "ui.collapseRibbon"
            | "ui.quickAccessToolbar"
            | "ui.keytips"
            | "ui.dark"
            | "ui.theme"
            | "ui.interfaceSize"
            | "ui.animations"
            | "ui.language"
            | "ui.saveChanges"
            | "ui.openFileDialog"
            | "ui.fileBrowser"
            | "ui.makeDefaultApp"
            | "ui.discord"
            | "ui.caretAnimation"
            | "ui.startScreen"
            | "ui.checkForUpdates"
    ) || id.starts_with("help.")
}

/// Commands that save, export, print, send or replace the document: comment text still being
/// typed is written before them (#1348).
fn flushes_comment_drafts(id: &str, params: &Value) -> bool {
    matches!(
        id,
        "file.save"
            | "file.saveAs"
            | "file.saveTemplate"
            | "file.open"
            | "file.exportPdf"
            | "file.exportPng"
            | "file.print"
            | "file.printPdf"
            | "file.share"
            | "ui.print"
            | "ui.printSubmit"
    ) || discards_document(id, params)
}

/// User commands that close the shown document: Close. New, Open, restoring a recovered copy,
/// Envelopes, Labels, Finish & Merge and Print Documents open their document beside it (Open and restoring
/// replace only an untouched blank document, which has nothing to lose).
pub(crate) fn discards_document(id: &str, _params: &Value) -> bool {
    id == "file.close"
}
