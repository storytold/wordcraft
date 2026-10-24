import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# 3. File drop handling - use open_from_shell
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

# 4. Password dialog retry - use the command that asked
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

# 5. Password dialog creation - pass command
patch('crates/ui-egui/src/lib.rs',
'''            self.dialog = Some(dialogs::Dialog::Password { name, params, password: Default::default(), message: String::new() });''',
'''            self.dialog = Some(dialogs::Dialog::Password { name, command: id.to_string(), params, password: Default::default(), message: String::new() });''')

print("lib.rs patched (2/3)")