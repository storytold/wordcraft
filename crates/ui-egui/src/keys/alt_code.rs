use super::{Key, Modifiers, Value, WordApp, json};

/// Alt+numpad character codes are a Windows convention; on macOS Option+digit types its own
/// characters (¡ ™ £…), so they're off there (also in a browser on a Mac).
pub(super) fn alt_codes_supported(ctx: &egui::Context) -> bool {
    !cfg!(target_os = "macos") && ctx.os() != egui::os::OperatingSystem::Mac
}

/// The digit a key types for an Alt code.
///
/// Word counts only numeric-keypad digits. egui 0.36 reports keypad and top-row digits alike
/// (`Key::Num0`…`Num9`, with the same `physical_key`), and eframe passes no winit key location
/// on, so the top-row digits compose codes too (they type nothing else with Alt alone; Ctrl+Alt+1
/// and keytip digits are untouched).
fn alt_code_digit(key: Key) -> Option<char> {
    let n = match key {
        Key::Num0 => 0,
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        _ => return None,
    };
    char::from_digit(n, 10)
}

/// Alt+numpad entry (#961): digits typed with Alt alone held collect; releasing Alt types the
/// character their code names (`text.typeCode`: 0233 → é, 233 → Θ in code page 437). Returns
/// whether the event was part of it (and isn't handled otherwise).
pub(super) fn alt_code_event(app: &mut WordApp, e: &egui::Event) -> bool {
    let alt_only = |m: &Modifiers| m.alt && !m.ctrl && !m.shift && !m.command && !m.mac_cmd;
    match e {
        egui::Event::Key { key, pressed: true, repeat, modifiers, .. } if alt_only(modifiers) => match alt_code_digit(*key) {
            // A held digit's auto-repeat adds nothing (as on Windows).
            Some(_) if *repeat => {
                app.canvas.swallow_text = true;
                true
            }
            Some(d) => {
                // Codes beyond seven digits name nothing; stop collecting rather than grow.
                if app.canvas.alt_code.len() < 8 {
                    app.canvas.alt_code.push(d);
                }
                // The digit's text (Linux sends it even with Alt held) isn't typed.
                app.canvas.swallow_text = true;
                true
            }
            None => {
                app.canvas.alt_code.clear();
                false
            }
        },
        egui::Event::Key { key: Key::AltLeft | Key::AltRight, pressed: false, .. } => {
            finish_alt_code(app);
            false
        }
        egui::Event::ModifiersChanged(m) if !m.alt => {
            finish_alt_code(app);
            false
        }
        egui::Event::Key { pressed: true, .. } => {
            // Another key with Ctrl/Shift held (or without Alt) ends the entry unfinished.
            app.canvas.alt_code.clear();
            false
        }
        _ => false,
    }
}

/// What an Alt code typed, for the platform's own composed character that may follow.
#[derive(Clone, Debug)]
pub(crate) struct AltCodeEcho {
    /// The character typed.
    pub(super) typed: String,
    /// The system composes Alt codes itself (Windows): whatever it sends is the code's character.
    pub(super) system_composes: bool,
    /// The code named a byte of the system's code pages (1–255), so the system's character wins
    /// over ours: it knows the code pages (DBCS ones too) better than our locale guess.
    pub(super) byte_code: bool,
}

/// Alt released: type the collected code's character, if any.
fn finish_alt_code(app: &mut WordApp) {
    let code = std::mem::take(&mut app.canvas.alt_code);
    if code.is_empty() {
        return;
    }
    crate::objects::enter_text_box(app);
    let system_composes = cfg!(target_os = "windows");
    let mut params = json!({"code": code});
    if system_composes {
        // Windows: the code pages of the system locale (Polish 1250/852, Russian 1251/866…).
        let (ansi, oem) = wordcraft_engine::cmd::text::system_code_pages();
        params = json!({"code": code, "ansi": ansi, "oem": oem});
        // The system's composed character comes as text after the digits' swallowed text.
        app.canvas.swallow_text = false;
    }
    if let Ok(r) = app.run("text.typeCode", params) {
        let byte_code = code.trim_start_matches('0').parse::<u32>().is_ok_and(|v| v <= 255);
        app.canvas.alt_code_echo = r.get("char").and_then(Value::as_str).map(|c| AltCodeEcho { typed: c.to_string(), system_composes, byte_code });
    }
}

/// Text arriving right after an Alt code typed `echo` (no key pressed between): whether it was
/// the platform's composed character and is handled. The same character isn't typed twice. On
/// Windows the text is always the composed character: for a code up to 255 it replaces ours (the
/// system knows its code pages), above 255 ours stays (Windows reduces it to a byte; Word types
/// the Unicode character).
pub(super) fn alt_code_echoed(app: &mut WordApp, echo: &AltCodeEcho, text: &str) -> bool {
    if echo.typed == text {
        return true;
    }
    if !echo.system_composes {
        return false;
    }
    if echo.byte_code && text.chars().count() == 1 {
        let s = &app.session;
        let f = s.sel.focus.clone();
        let start = f.off.checked_sub(echo.typed.len());
        let ours =
            start.filter(|_| s.sel.is_collapsed()).filter(|st| s.doc.para_at(&f).and_then(|p| p.text.get(*st..f.off)) == Some(echo.typed.as_str()));
        if let Some(start) = ours {
            app.session.sel = wordcraft_engine::Selection { anchor: wordcraft_doc::Pos { off: start, ..f.clone() }, focus: f };
            let _ = app.run("text.insert", json!({"text": text}));
        }
    }
    true
}
