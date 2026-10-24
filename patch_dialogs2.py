import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# Dialog::Password body - use command and add command to destructuring
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

print("dialogs.rs patched")