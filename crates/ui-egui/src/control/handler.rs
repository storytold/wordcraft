use super::screenshots::render_png;
use super::{ControlRequest, Value, WordApp, json};

pub enum Outcome {
    Done(Value),
    Screenshot { path: Option<String> },
}

fn ok(v: Value) -> Outcome {
    Outcome::Done(json!({"ok": true, "result": v}))
}

fn err(e: impl std::fmt::Display) -> Outcome {
    Outcome::Done(json!({"ok": false, "error": e.to_string()}))
}

pub fn inspect(app: &mut WordApp, ctx: &egui::Context) -> Value {
    let r = ctx.input_for(app.active_viewport(), |i| i.content_rect());
    let caret = crate::canvas::caret_screen(app).map(|(p, h)| json!({"x": p.x, "y": p.y, "h": h}));
    // Preferences without secrets: the LanguageTool API key only shows as set (#575), also when
    // the credential store keeps it out of `ui.json` (#764).
    let mut prefs = app.prefs();
    if !app.ui.language_tool.api_key.is_empty() {
        prefs.language_tool.api_key = "<set>".into();
    }
    json!({
        "ui": prefs,
        "view": app.session.view,
        "dialog": app.dialog.as_ref().map(|d| serde_json::to_value(d).unwrap_or_default()),
        "window": [r.width(), r.height()],
        "canvasRect": app.canvas.canvas_rect.map(|c| [c.left(), c.top(), c.width(), c.height()]),
        "pages": app.canvas.page_rects.iter().map(|r| [r.left(), r.top(), r.width(), r.height()]).collect::<Vec<_>>(),
        "scale": app.canvas.scale,
        "caret": caret,
        "balloons": app.canvas.balloon_rects.iter().map(|(id, r)| json!({"id": id, "rect": [r.left(), r.top(), r.width(), r.height()]})).collect::<Vec<_>>(),
        "balloon": app.canvas.balloon,
        "focused": app.canvas.focused,
        "title": app.title_stem(),
        "dirty": app.session.dirty,
        "document": app.session.document_id(),
        "documents": app.session.documents(),
        "perf": {"frameMs": app.frame_ms, "renderMs": app.canvas.render_ms, "layoutMs": app.session.layout().ms},
    })
}

fn key_from(name: &str) -> Option<egui::Key> {
    egui::Key::from_name(name).or(match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(egui::Key::Enter),
        "esc" | "escape" => Some(egui::Key::Escape),
        "delete" => Some(egui::Key::Delete),
        "backspace" => Some(egui::Key::Backspace),
        "left" => Some(egui::Key::ArrowLeft),
        "right" => Some(egui::Key::ArrowRight),
        "up" => Some(egui::Key::ArrowUp),
        "down" => Some(egui::Key::ArrowDown),
        "space" => Some(egui::Key::Space),
        "tab" => Some(egui::Key::Tab),
        "alt" | "altleft" | "altgr" => Some(egui::Key::AltLeft),
        "altright" => Some(egui::Key::AltRight),
        "home" => Some(egui::Key::Home),
        "end" => Some(egui::Key::End),
        _ => None,
    })
}

fn mods(p: &Value) -> egui::Modifiers {
    let b = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
    let cmd = b("cmd") || b("command") || b("ctrl");
    egui::Modifiers {
        alt: b("alt"),
        ctrl: cmd && !cfg!(target_os = "macos"),
        shift: b("shift"),
        mac_cmd: cmd && cfg!(target_os = "macos"),
        command: cmd,
    }
}

fn click_events(app: &mut WordApp, pos: egui::Pos2, button: egui::PointerButton, count: u64, m: egui::Modifiers) {
    app.synthetic.push(egui::Event::PointerMoved(pos));
    for _ in 0..count.clamp(1, 3) {
        app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers: m });
        app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: false, modifiers: m });
    }
}

/// Programmatic calls never open the Save prompt, so closing unsaved work must be explicit:
/// save first, or pass `"force": true` (only a real boolean true counts) to discard it (#1268).
pub(crate) fn close_refused(app: &WordApp, id: &str, params: &Value) -> Option<&'static str> {
    let force = params.get("force").and_then(Value::as_bool) == Some(true);
    if force {
        return None;
    }
    if id == "file.close" && app.session.dirty {
        return Some(UNSAVED);
    }
    // Comment text still being typed is unsaved work too (#1348): replacing the document would drop it.
    (crate::discards_document(id, params) && crate::panes::has_comment_drafts(app))
        .then_some("a comment is still being edited: finish it first, or pass {\"force\": true} to discard it")
}

const UNSAVED: &str = "the document has unsaved changes: save it first, or pass {\"force\": true} to discard them and close";

/// The reply to a request that needs a document while none is open (#1129).
const NO_DOCUMENT: &str = "no document is open; create one (file.new) or open one (file.open) first";

/// The methods that take `doc`.
const DOC_METHODS: &[&str] = &["engine.execute", "command", "document.inspect", "ui.render"];

/// The open document other than the active one that a request's `doc` names; `None` for the
/// active document (no `doc`, null, or the active document's id). An id that names no open
/// document is an error.
fn addressed(app: &WordApp, p: &Value) -> Result<Option<u64>, String> {
    let id = match p.get("doc") {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v.as_u64().ok_or("`doc` must be the id of an open document (view.switchWindows lists them)")?,
    };
    if !app.session.is_open(id) {
        return Err(format!("no open document has id {id}"));
    }
    Ok((id != app.session.document_id()).then_some(id))
}

/// The reply to work run on another document: its result, or why it couldn't run or failed.
fn reply<E: std::fmt::Display, F: std::fmt::Display>(r: Result<Result<Value, E>, F>) -> Outcome {
    match r {
        Ok(Ok(v)) => ok(v),
        Ok(Err(e)) => err(e),
        Err(e) => err(e),
    }
}

/// A document method addressed to open document `doc`, which isn't the active one: it runs on
/// that document, and the window keeps showing the active one.
fn handle_in(app: &mut WordApp, doc: u64, method: &str, p: &Value) -> Outcome {
    if app.no_document == Some(doc) {
        return err(NO_DOCUMENT);
    }
    match method {
        "document.inspect" => reply(app.session.with_document(doc, |s| s.run("document.inspect", p))),
        "ui.render" => {
            let page = p.get("page").and_then(Value::as_u64).unwrap_or(1);
            let scale = p.get("scale").and_then(Value::as_f64).map_or(2.0, |v| v as f32);
            let path = p.get("path").and_then(Value::as_str);
            let r = app.session.with_document(doc, |s| match path {
                Some(path) => s.run("file.exportPng", &json!({"path": path, "page": page, "scale": scale})).map_err(|e| e.to_string()),
                None => render_png(s, page, scale),
            });
            reply(r)
        }
        _ => {
            let Some(id) = p.get("command").or(p.get("id")).and_then(Value::as_str) else { return err("missing `command`") };
            let params = p.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
            if id.starts_with("ui.") {
                return err(format!("`{id}` works on the window: make the document active first (view.switchWindows {{\"id\": {doc}}})"));
            }
            // Close, with the same guard as the window's: unsaved changes need `force`.
            if id == "file.close" {
                let force = params.get("force").and_then(Value::as_bool) == Some(true);
                if !force && app.session.documents().iter().any(|d| d.id == doc && d.dirty) {
                    return err(UNSAVED);
                }
                return match app.session.close_document(doc) {
                    Ok(()) => ok(json!({"closed": true, "id": doc, "documents": app.session.documents()})),
                    Err(e) => err(e),
                };
            }
            let r = app.session.with_document(doc, |s| s.run(id, &params));
            // UI requests the command made carry `document`, so the window can tell they aren't
            // about the document it shows.
            for req in std::mem::take(&mut app.session.ui_requests) {
                app.handle_request(&req);
            }
            reply(r)
        }
    }
}

/// Dispatches one JSON-lines control request: commands and inspection, parity, synthetic click,
/// drag, key, wheel and text input, screenshots and renders, resize, focus and quit, and unknown
/// ids as commands. It refuses closing unsaved work and document operations without a document.
pub fn handle(app: &mut WordApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    if DOC_METHODS.contains(&req.method.as_str()) {
        match addressed(app, p) {
            Ok(Some(doc)) => return handle_in(app, doc, &req.method, p),
            Ok(None) => {}
            Err(e) => return err(e),
        }
    }
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let f = |k: &str| p.get(k).and_then(Value::as_f64).map(|v| v as f32);
    match req.method.as_str() {
        "engine.execute" | "command" => {
            let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
            let params = p.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
            if let Some(e) = close_refused(app, id, &params) {
                return err(e);
            }
            match app.execute(id, params) {
                Ok(v) => ok(v),
                Err(e) => err(e),
            }
        }
        "engine.commands" => {
            let reg = app.session.registry.clone();
            let mut commands: Vec<Value> = reg.all().iter()
                .map(|c| json!({"id": c.id, "label": c.label, "location": c.location, "shortcut": c.shortcut, "params": c.params, "enabled": app.session.why_disabled(c.id).is_none()}))
                .collect();
            commands.extend(crate::printing::commands(app));
            commands.extend(crate::languagetool::commands());
            let search = s("search").unwrap_or("").to_ascii_lowercase();
            if !search.is_empty() {
                commands.retain(|command| {
                    ["id", "label", "location", "params"]
                        .iter()
                        .any(|field| command.get(field).and_then(Value::as_str).is_some_and(|text| text.to_ascii_lowercase().contains(&search)))
                });
            }
            ok(Value::Array(commands))
        }
        // The Start screen's empty window has no document to inspect or render (#1129).
        "document.inspect" | "ui.render" if !app.has_document() => err(NO_DOCUMENT),
        "document.inspect" => match app.session.run("document.inspect", p) {
            Ok(v) => ok(v),
            Err(e) => err(e),
        },
        "ui.inspect" => ok(inspect(app, ctx)),
        "ui.parity" => ok(wordcraft_engine::catalog::parity(&app.session.registry)),
        "ui.click" => {
            let (Some(x), Some(y)) = (f("x"), f("y")) else { return err("missing x/y") };
            let button = match s("button") {
                Some("right") => egui::PointerButton::Secondary,
                _ => egui::PointerButton::Primary,
            };
            let count = p.get("count").and_then(Value::as_u64).unwrap_or(1);
            click_events(app, egui::pos2(x, y), button, count, mods(p));
            ok(json!({"queued": true}))
        }
        "ui.clickText" => {
            let page = p.get("page").and_then(Value::as_u64).unwrap_or(0) as usize;
            let (Some(x), Some(y)) = (f("x"), f("y")) else { return err("missing x/y") };
            let Some(pos) = crate::canvas::page_to_screen(app, page, x, y) else { return err("page not on screen") };
            let count = p.get("count").and_then(Value::as_u64).unwrap_or(1);
            click_events(app, pos, egui::PointerButton::Primary, count, mods(p));
            ok(json!({"screen": [pos.x, pos.y]}))
        }
        "ui.move" => {
            let (Some(x), Some(y)) = (f("x"), f("y")) else { return err("missing x/y") };
            app.synthetic.push(egui::Event::PointerMoved(egui::pos2(x, y)));
            ok(json!({"queued": true}))
        }
        "ui.press" | "ui.release" => {
            let (Some(x), Some(y)) = (f("x"), f("y")) else { return err("missing x/y") };
            let pos = egui::pos2(x, y);
            app.synthetic.push(egui::Event::PointerMoved(pos));
            app.synthetic.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: req.method == "ui.press",
                modifiers: mods(p),
            });
            ok(json!({"queued": true}))
        }
        "ui.drag" => {
            let (Some(x), Some(y), Some(tx), Some(ty)) = (f("x"), f("y"), f("toX"), f("toY")) else { return err("missing x/y/toX/toY") };
            let m = mods(p);
            let steps = p.get("steps").and_then(Value::as_u64).unwrap_or(8).clamp(1, 100);
            app.synthetic.push(egui::Event::PointerMoved(egui::pos2(x, y)));
            app.synthetic.push(egui::Event::PointerButton {
                pos: egui::pos2(x, y),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: m,
            });
            for i in 1..=steps {
                let k = i as f32 / steps as f32;
                app.synthetic.push(egui::Event::PointerMoved(egui::pos2(x + (tx - x) * k, y + (ty - y) * k)));
            }
            app.synthetic.push(egui::Event::PointerButton {
                pos: egui::pos2(tx, ty),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: m,
            });
            ok(json!({"queued": true}))
        }
        "ui.key" => {
            let Some(k) = s("key").and_then(key_from) else { return err("unknown key") };
            let m = mods(p);
            app.synthetic.push(egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m });
            app.synthetic.push(egui::Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers: m });
            ok(json!({"queued": true}))
        }
        "ui.wheel" => {
            // Mouse-wheel or trackpad scroll at (x, y): `dy` points (positive scrolls up), or
            // lines with `"unit": "line"`; `phase` "start"/"move"/"end" for a trackpad gesture.
            let (Some(x), Some(y)) = (f("x"), f("y")) else { return err("missing x/y") };
            let unit = match s("unit") {
                Some("line") => egui::MouseWheelUnit::Line,
                Some("page") => egui::MouseWheelUnit::Page,
                _ => egui::MouseWheelUnit::Point,
            };
            let phase = match s("phase") {
                Some("start") => egui::TouchPhase::Start,
                Some("end") => egui::TouchPhase::End,
                _ => egui::TouchPhase::Move,
            };
            let delta = egui::vec2(f("dx").unwrap_or(0.0), f("dy").unwrap_or(0.0));
            app.synthetic.push(egui::Event::PointerMoved(egui::pos2(x, y)));
            app.synthetic.push(egui::Event::MouseWheel { unit, delta, phase, modifiers: mods(p), source: egui::MouseWheelSource::Unknown });
            ok(json!({"queued": true}))
        }
        "ui.text" => {
            let Some(t) = s("text") else { return err("missing text") };
            app.synthetic.push(egui::Event::Text(t.to_string()));
            ok(json!({"queued": true}))
        }
        "ui.screenshot" => Outcome::Screenshot { path: s("path").map(str::to_string) },
        "ui.render" => {
            let page = p.get("page").and_then(Value::as_u64).unwrap_or(1);
            let scale = f("scale").unwrap_or(2.0);
            match s("path") {
                Some(path) => match app.session.run("file.exportPng", &json!({"path": path, "page": page, "scale": scale})) {
                    Ok(v) => ok(v),
                    Err(e) => err(e),
                },
                // No path: the PNG travels in the reply. Never a fixed name in the shared
                // temporary folder, where another user could have planted a link to a file.
                None => match render_png(&app.session, page, scale) {
                    Ok(v) => ok(v),
                    Err(e) => err(e),
                },
            }
        }
        "ui.resize" => {
            let (Some(w), Some(h)) = (f("width"), f("height")) else { return err("missing width/height") };
            // The window's surface is a GPU texture: no side may pass the largest one (#3734).
            let max_side = ctx.input(|i| i.max_texture_side) as f32 / ctx.pixels_per_point();
            let Some(size) = crate::window_geometry::requested_size(egui::vec2(w, h), max_side) else {
                return err("width and height must be positive numbers no larger than 3.4e38");
            };
            ctx.send_viewport_cmd_to(app.active_viewport(), egui::ViewportCommand::InnerSize(size));
            ok(json!({"width": size.x, "height": size.y}))
        }
        "ui.focus" => {
            ctx.send_viewport_cmd_to(app.active_viewport(), egui::ViewportCommand::Focus);
            app.canvas.want_focus = true;
            ok(json!({}))
        }
        "app.quit" => {
            app.quit_requested = true;
            ok(json!({}))
        }
        other => {
            // Anything else: a command id.
            if let Some(e) = close_refused(app, other, p) {
                return err(e);
            }
            match app.execute(other, p.clone()) {
                Ok(v) => ok(v),
                Err(e) => err(e),
            }
        }
    }
}
