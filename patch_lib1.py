import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# crates/ui-egui/src/lib.rs - add open_from_shell after title_stem
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

print("lib.rs: open_from_shell patched")