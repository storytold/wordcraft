import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# 1. apps/wordcraft/src/main.rs - non-Unicode template path fix
patch('apps/wordcraft/src/main.rs',
'''fn open_non_unicode(app: &mut WordApp, path: &std::path::Path) {
    let name = path.to_string_lossy();
    let opened = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| app.open_from_shell(&name, Some(wordcraft_engine::cmd::insert::base64_encode(&bytes))));
    match opened {
        // Not when it waits for a password: that open names the document itself.
        Ok(r) if r.get("pending").is_none() => app.session.path = Some(path.into()),
        Ok(_) => {}
        Err(e) => log::warn!("{name}: {e}"),
    }
}''',
'''fn open_non_unicode(app: &mut WordApp, path: &std::path::Path) {
    let name = path.to_string_lossy();
    // A template opens as a new document based on it, which has no path of its own (#3433): the
    // template is never what a later save writes to.
    let is_template = wordcraft_engine::io::is_template_name(&name);
    let opened = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| app.open_from_shell(&name, Some(wordcraft_engine::cmd::insert::base64_encode(&bytes))));
    match opened {
        // Not when it waits for a password: that open names the document itself.
        Ok(r) if r.get("pending").is_none() && !is_template => app.session.path = Some(path.into()),
        Ok(_) => {}
        Err(e) => log::warn!("{name}: {e}"),
    }
}''')

# 2. crates/ui-egui/src/lib.rs - open_from_shell + .dotm save name
patch('crates/ui-egui/src/lib.rs',
'''    /// Document title for the title bar (and the suggested file name). An untitled document is
    /// named in the interface language, as `Document1` is in English.
    fn title_stem(&self) -> String {
        match &self.session.path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!(\"Document1\").into()),
            None => match &self.session.file_name {
                Some(name) => name.clone(),
                None if self.session.doc.core.title.is_empty() => tl!(\"Document1\").into(),
                None => self.session.doc.core.title.clone(),
            },
        }
    }''',
'''    /// Open a file the platform handed us: the command line, the file manager, a drop on the
    /// window. A Word template (.dotx/.dotm) starts a NEW document based on it, as in Word
    /// (#3433): the document has no path of its own, the template is attached, and Save asks
    /// where to write instead of overwriting the template on disk. Any other file opens as
    /// itself.
    ///
    /// File > Open is not this: it opens a template as the template, which is Word's way of
    /// editing one.
    pub fn open_from_shell(&mut self, path: &str, data: Option<String>) -> Result<Value, String> {
        let id = if wordcraft_engine::io::is_template_name(path) { "file.newFromTemplate" } else { "file.open" };
        let mut params = json!({"path": path});
        if let Some(data) = data {
            params["data"] = json!(data);
        }
        self.run(id, params)
    }

    /// Document title for the title bar (and the suggested file name). An untitled document is
    /// named in the interface language, as `Document1` is in English.
    fn title_stem(&self) -> String {
        match &self.session.path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!(\"Document1\").into()),
            None => match &self.session.file_name {
                Some(name) => name.clone(),
                None if self.session.doc.core.title.is_empty() => tl!(\"Document1\").into(),
                None => self.session.doc.core.title.clone(),
            },
        }
    }''')

# 3. .dotm save name
patch('crates/ui-egui/src/lib.rs',
'''    /// The name Save suggests: the open file's own Word format (so a .docm keeps its macros),
    /// otherwise .docx.
    fn default_save_name(&self) -> String {
        let ext = self.session.path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_string());
        match ext {
            Some(e) if wordcraft_engine::io::is_word_package(&e) => format!("{}.{e}", self.title_stem()),
            _ => format!("{}.docx", self.title_stem()),
        }
    }''',
'''    /// The name Save suggests: the open file's own Word format (so a .docm keeps its macros),
    /// otherwise .docx.
    fn default_save_name(&self) -> String {
        let ext = self.session.path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_string());
        match ext {
            Some(e) if wordcraft_engine::io::is_word_package(&e) => format!("{}.{e}", self.title_stem()),
            // A new document made from a macro-enabled template keeps its macros, so it is saved
            // as a .docm: a .docx would leave the VBA project out (#3433).
            None if self.from_a_macro_template() => format!("{}.docm", self.title_stem()),
            _ => format!("{}.docx", self.title_stem()),
        }
    }

    /// Was this document made from a macro-enabled template? It carries the template's VBA
    /// project, which a .docx cannot hold.
    fn from_a_macro_template(&self) -> bool {
        self.session
            .doc
            .settings
            .attached_template
            .as_deref()
            .is_some_and(|t| std::path::Path::new(t).extension().is_some_and(|e| e.eq_ignore_ascii_case("dotm")))
    }''')

# 4. File drop handling - use open_from_shell
patch('crates/ui-egui/src/lib.rs',
'''                } else if wordcraft_fonts::is_font_file_name(&lp) {
                    self.run("font.add", json!({"path": path}))
                } else {
                    self.run("file.open", json!({"path": path}))
                };''',
'''                } else if wordcraft_fonts::is_font_file_name(&lp) {
                    self.run("font.add", json!({"path": path}))
                } else {
                    self.open_from_shell(&path, None)
                };''')

# 5. Password dialog retry - use the command that asked
patch('crates/ui-egui/src/lib.rs',
'''        let retry = (id == "file.open").then(|| params.clone());
        let r = self.execute(id, params);
        if let (Err(e), Some(params)) = (&r, retry)
            && wordcraft_engine::io::needs_password(e)
        {
            let path = params.get("path").and_then(Value::as_str).unwrap_or("");
            let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
            self.dialog = Some(dialogs::Dialog::Password { name, params, password: Default::default(), message: String::new() });
            return Ok(json!({"pending": "password"}));
        }
        r''',
'''        // A protected file asks for its password and is opened with it: a template opened from
        // the shell is one of them (#3433).
        let retry = matches!(id, "file.open" | "file.newFromTemplate").then(|| params.clone());
        let r = self.execute(id, params);
        if let (Err(e), Some(params)) = (&r, retry)
            && wordcraft_engine::io::needs_password(e)
        {
            let path = params.get("path").and_then(Value::as_str).unwrap_or("");
            let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
            self.dialog = Some(dialogs::Dialog::Password { name, command: id.to_string(), params, password: Default::default(), message: String::new() });
            return Ok(json!({"pending": "password"}));
        }
        r''')

# 6. Dialog::Password - add command field
patch('crates/ui-egui/src/dialogs.rs',
'''    /// Opening a password-protected document (#55): the password, then `file.open` with
    /// `params` again. The password is never serialised or printed.
    Password {
        name: String,
        #[serde(skip)]
        params: Value,
        #[serde(skip)]
        password: Password,
        message: String,
    },''',
'''    /// Opening a password-protected document (#55): the password, then the command that asked
    /// for it (`file.open`, or `file.newFromTemplate` for a template) with `params` again. The
    /// password is never serialised or printed.
    Password {
        name: String,
        /// The command to run with the password.
        #[serde(skip)]
        command: String,
        #[serde(skip)]
        params: Value,
        #[serde(skip)]
        password: Password,
        message: String,
    },''')

# 7. Dialog::Password body - use command
patch('crates/ui-egui/src/dialogs.rs',
'''        Dialog::Password { name, params, password, message } => {
            ui.label(crate::i18n::fmt(tl!("{name} is protected with a password."), &[("name", name)]));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(crate::i18n::fmt(tl!("Password:")));
                let resp = ui.add(crate::password_field(password).desired_width(240.0));
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let mut o = params.clone();
                    if !password.is_empty() {
                        o.insert("password".into(), json!(password.as_str()));
                    }
                    // `execute`, not `run`: Save Changes was asked when the open began.
                    match app.execute("file.open", o) {
                        Ok(_) => {
                            app.ui.backstage = false;
                            return true;
                        }
                        Err(e) => {
                            *message = e.to_string();
                            *password = Default::default();
                        }
                    }
                }
            });
            if !message.is_empty() {
                ui.colored_label(crate::theme::error_color(), message);
            }
        }''',
'''        Dialog::Password { name, command, params, password, message } => {
            ui.label(crate::i18n::fmt(tl!("{name} is protected with a password."), &[("name", name)]));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(crate::i18n::fmt(tl!("Password:")));
                let resp = ui.add(crate::password_field(password).desired_width(240.0));
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let mut o = params.clone();
                    if !password.is_empty() {
                        o.insert("password".into(), json!(password.as_str()));
                    }
                    // `execute`, not `run`: Save Changes was asked when the open began.
                    match app.execute(&command, o) {
                        Ok(_) => {
                            app.ui.backstage = false;
                            return true;
                        }
                        Err(e) => {
                            *message = e.to_string();
                            *password = Default::default();
                        }
                    }
                }
            });
            if !message.is_empty() {
                ui.colored_label(crate::theme::error_color(), message);
            }
        }''')

# 8. Password dialog creation - pass command
patch('crates/ui-egui/src/lib.rs',
'''            self.dialog = Some(dialogs::Dialog::Password { name, params, password: Default::default(), message: String::new() });''',
'''            self.dialog = Some(dialogs::Dialog::Password { name, command: id.to_string(), params, password: Default::default(), message: String::new() });''')

# 9. crates/engine/src/io.rs - is_template_name
patch('crates/engine/src/io.rs',
'''pub fn is_word_package(ext: &str) -> bool {
    wordcraft_docx::Flavor::from_ext(ext).is_some()
}''',
'''pub fn is_word_package(ext: &str) -> bool {
    wordcraft_docx::Flavor::from_ext(ext).is_some()
}

/// Is `name` a Word template? A template opened from the shell starts a new document based on
/// it, as in Word (#3433); one opened from File > Open is the template itself.
///
/// The OOXML templates (.dotx/.dotm) are Word's own kinds; `.dot` is the Word 97-2003 one,
/// which Word also opens as a new document. `.doc` is not a template: it is a document.
pub fn is_template_name(name: &str) -> bool {
    match ext_of(name).as_str() {
        "dot" => true,
        ext => matches!(wordcraft_docx::Flavor::from_ext(ext), Some(wordcraft_docx::Flavor::Template | wordcraft_docx::Flavor::MacroTemplate)),
    }
}''')

# 10. crates/engine/src/cmd/tools.rs - file.newFromTemplate
patch('crates/engine/src/cmd/tools.rs',
'''        CommandSpec::new("file.newFromTemplate", "New from Template", "File › New", |s, v| {
            let path = p::req_str(v, "path")?;
            let password = p::str(v, "password");
            // The template's own styles and page setup are the document's, so it is opened like
            // a document and only the path is dropped.
            let doc = match p::str(v, "data") {
                Some(data) => {
                    let bytes = super::insert::base64_decode(data).ok_or_else(|| CmdError::Params("bad base64".into()))?;
                    crate::io::open_bytes_in(path, &bytes, password, &s.region()).map_err(CmdError::Failed)?.0
                }
                None => crate::io::open_path_with(std::path::Path::new(path), password).map_err(CmdError::Failed)?.0,
            };
            s.set_document(doc);
            // The template stays attached, as Word's does (#3433): the document is built from it,
            // and "Update Styles from Template" still finds it.
            s.doc.settings.attached_template = Some(path.to_string());
            s.path = None;
            s.dirty = true;
            sel_result(s)
        })
        .params(r#"{"path": string, "data"?: base64 (the template's bytes; `path` then only names it), "password"?: string (for a password-protected Word template)}"#)
        .pure(),''',
'''        CommandSpec::new("file.newFromTemplate", "New from Template", "File › New", |s, v| {
            let path = p::req_str(v, "path")?;
            let password = p::str(v, "password");
            // The template's own styles and page setup are the document's, so it is opened like
            // a document and only the path is dropped.
            let doc = match p::str(v, "data") {
                Some(data) => {
                    let bytes = super::insert::base64_decode(data).ok_or_else(|| CmdError::Params("bad base64".into()))?;
                    crate::io::open_bytes_in(path, &bytes, password, &s.region()).map_err(CmdError::Failed)?.0
                }
                None => crate::io::open_path_with(std::path::Path::new(path), password).map_err(CmdError::Failed)?.0,
            };
            s.set_document(doc);
            // The template stays attached, as Word's does (#3433): the document is built from it,
            // and "Update Styles from Template" still finds it.
            s.doc.settings.attached_template = Some(path.to_string());
            s.path = None;
            s.dirty = true;
            sel_result(s)
        })
        .params(r#"{"path": string, "data"?: base64 (the template's bytes; `path` then only names it), "password"?: string (for a password-protected Word template)}"#)
        .pure(),''')

print("all patches applied")