import io, sys

def patch(path, old, new):
    s = io.open(path, encoding='utf-8', newline='').read()
    eol = "\r\n" if "\r\n" in s else "\n"
    o = old.replace("\n", eol); n = new.replace("\n", eol)
    if s.count(o) != 1:
        sys.exit(f"{path}: anchor count {s.count(o)}")
    io.open(path, 'w', encoding='utf-8', newline='').write(s.replace(o, n, 1))
    print("patched", path)

# Dialog::Password body - add command to destructuring and use it
patch('crates/ui-egui/src/dialogs.rs',
'''        Dialog::Password { name, params, password, message } => {
            ui.label(crate::i18n::fmt(tl!("{name} is protected with a password."), &[("name", name)]));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(tl!("Password:"));
                let r = ui.add(egui::TextEdit::singleline(password.as_mut_string()).password(true).desired_width(220.0));
                if password.is_empty() && !r.has_focus() {
                    r.request_focus();
                }
            });
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).small().color(ui.visuals().error_fg_color));
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok && !password.is_empty() {
                let mut p = params.clone();
                if let Some(o) = p.as_object_mut() {
                    o.insert("password".into(), json!(password.as_str()));
                }
                // `execute`, not `run`: Save Changes was asked when the open began.
                match app.execute("file.open", p) {
                    Ok(_) => {
                        app.ui.backstage = false;
                        return true;
                    }
                    Err(e) if wordcraft_engine::io::wrong_password(&e) => {
                        *message = tl!("That password isn't right. Check it (passwords are case-sensitive) and try again.").to_string();
                        password.as_mut_string().clear();
                    }
                    Err(e) => *message = e,
                }
            }
            cancel
        }''',
'''        Dialog::Password { name, command, params, password, message } => {
            ui.label(crate::i18n::fmt(tl!("{name} is protected with a password."), &[("name", name)]));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(tl!("Password:"));
                let r = ui.add(egui::TextEdit::singleline(password.as_mut_string()).password(true).desired_width(220.0));
                if password.is_empty() && !r.has_focus() {
                    r.request_focus();
                }
            });
            if !message.is_empty() {
                ui.label(egui::RichText::new(message.as_str()).small().color(ui.visuals().error_fg_color));
            }
            let (ok, cancel) = buttons(ui, tl!("OK"));
            if ok && !password.is_empty() {
                let mut p = params.clone();
                if let Some(o) = p.as_object_mut() {
                    o.insert("password".into(), json!(password.as_str()));
                }
                // `execute`, not `run`: Save Changes was asked when the open began.
                match app.execute(&command, p) {
                    Ok(_) => {
                        app.ui.backstage = false;
                        return true;
                    }
                    Err(e) if wordcraft_engine::io::wrong_password(&e) => {
                        *message = tl!("That password isn't right. Check it (passwords are case-sensitive) and try again.").to_string();
                        password.as_mut_string().clear();
                    }
                    Err(e) => *message = e,
                }
            }
            cancel
        }''')

print("dialogs.rs patched")