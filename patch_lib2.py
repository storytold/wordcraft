import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# 1. Add open_from_shell after title_stem
patch('crates/ui-egui/src/lib.rs',
'''    /// Document title for the title bar (and the suggested file name). An untitled document is
    /// named in the interface language, as `Document1` is in English.
    pub fn title_stem(&self) -> String {
        match &self.session.path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!(\"Document1\").into()),
            None => match &self.session.file_name {
                Some(name) => name.clone(),
                None if self.session.doc.core.title.is_empty() => tl!(\"Document1\").into(),
                None => self.session.doc.core.title.clone(),
            },
        }
    }

    /// Terminal feedback also reaches the normal window when printing starts through control.''',
'''    /// Document title for the title bar (and the suggested file name). An untitled document is
    /// named in the interface language, as `Document1` is in English.
    pub fn title_stem(&self) -> String {
        match &self.session.path {
            Some(p) => p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!(\"Document1\").into()),
            None => match &self.session.file_name {
                Some(name) => name.clone(),
                None if self.session.doc.core.title.is_empty() => tl!(\"Document1\").into(),
                None => self.session.doc.core.title.clone(),
            },
        }
    }

    /// Open a file the platform handed us: the command line, the file manager, a drop on the
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

    /// Terminal feedback also reaches the normal window when printing starts through control.''')

# 2. .dotm save name
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

print("lib.rs patched (1/3)")