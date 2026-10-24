import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# apps/wordcraft/src/apple_events.rs - use open_from_shell for macOS
patch('apps/wordcraft/src/apple_events.rs',
'''            Event::Open(paths) => {
                for p in paths.iter().filter_map(|p| p.to_str()) {
                    match app.run("file.open", serde_json::json!({"path": p})) {
                        // Leave Backstage, as opening from the Open dialog does.
                        Ok(_) => app.ui.backstage = false,
                        Err(e) => {
                            log::warn!("{p}: {e}");
                            let e = e.to_string();
                            let msg = wordcraft_ui_egui::tl!("Couldn't open {path}: {error}");
                            app.status(wordcraft_ui_egui::i18n::fmt(msg, &[("path", p), ("error", &e)]));
                        }
                    }
                }
            }''',
'''            Event::Open(paths) => {
                for p in paths.iter().filter_map(|p| p.to_str()) {
                    // macOS Finder/Open With/drop on Dock uses the same semantics as the command line:
                    // a template becomes a new document, any other file opens as itself (#3433).
                    match app.open_from_shell(p, None) {
                        Ok(_) => app.ui.backstage = false,
                        Err(e) => {
                            log::warn!("{p}: {e}");
                            let e = e.to_string();
                            let msg = wordcraft_ui_egui::tl!("Couldn't open {path}: {error}");
                            app.status(wordcraft_ui_egui::i18n::fmt(msg, &[("path", p), ("error", &e)]));
                        }
                    }
                }
            }''')

print("apple_events.rs patched")