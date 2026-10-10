//! Programmatic control of the running app (agents, tests, MCP bridge).
//!
//! Methods (JSON lines over the host's transport, see `docs/control-protocol.md`):
//! - `engine.execute {command, params}`: run any command (engine or `ui.*`)
//! - `engine.commands`: every command with its enablement
//! - `document.inspect`: document structure; `ui.inspect`: UI state, canvas geometry, perf
//! - `ui.click {x, y, button?, count?, shift?, cmd?}`, `ui.move {x, y}`, `ui.drag {x, y, toX, toY}`:
//!   real pointer input in screen points (reaches every widget)
//! - `ui.clickText {page, x, y}`: click at a page position (points from the page's top-left)
//! - `ui.key {key, shift?, alt?, cmd?}`, `ui.text {text}`: keyboard input through egui
//! - `ui.screenshot {path?}`: PNG of the window; `ui.render {path, page?, scale?}`: page PNG
//! - `ui.parity`: feature catalog parity

use std::sync::mpsc::Sender;

use serde_json::{Value, json};

use crate::WordApp;

pub use wordcraft_chat::Principal;

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Value>,
    /// Who sent it: the window's key (`Host`) or a chat member's key. Members go through the
    /// chat gate (`handle_member`).
    pub principal: Principal,
    /// Wall-clock ms ([`crate::now_ms`]) after which the sender stopped waiting: the UI answers
    /// `expired` and does not run it (a window that did not draw must not apply old steps late).
    pub deadline_ms: Option<f64>,
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, std::sync::mpsc::Receiver<Value>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx, principal: Principal::Host, deadline_ms: None }, rx)
    }
    pub fn with_principal(mut self, p: Principal) -> Self {
        self.principal = p;
        self
    }
    pub fn with_deadline_ms(mut self, at: f64) -> Self {
        self.deadline_ms = Some(at);
        self
    }
    /// The sender gave up on it already.
    pub fn expired(&self) -> bool {
        self.deadline_ms.is_some_and(|d| crate::now_ms() > d)
    }
}

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
    let r = ctx.content_rect();
    let caret = crate::canvas::caret_screen(app).map(|(p, h)| json!({"x": p.x, "y": p.y, "h": h}));
    json!({
        "ui": app.prefs(),
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

/// A chat member's request: membership checked again on the UI thread, then the allow-list,
/// then the command runs as the member (see `chat_gate`).
pub(crate) fn handle_member(app: &mut WordApp, handle: &str, req: &ControlRequest) -> Outcome {
    if !app.session.chat.as_ref().is_some_and(|c| c.hub().is_member(handle)) {
        return err(format!("{handle} is not a member of this chat (removed)"));
    }
    let Some((id, params)) = crate::chat_gate::member_command(&req.method, &req.params) else { return err("missing `command`") };
    if !crate::chat_gate::agent_allowed(&id) {
        return err(format!("{id}: {}", crate::chat_gate::NOT_ALLOWED));
    }
    match id.as_str() {
        "engine.commands" => ok(crate::chat_gate::agent_commands(app)),
        "view.page" => {
            let page = params.get("page").and_then(Value::as_u64).unwrap_or(1);
            let scale = params.get("scale").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            match crate::chat_gate::view_page(app, page, scale) {
                Ok(v) => ok(v),
                Err(e) => err(e),
            }
        }
        _ => match crate::chat_gate::run_as_member(app, handle, &id, params) {
            Ok(v) => ok(v),
            Err(e) => err(e),
        },
    }
}

pub fn handle(app: &mut WordApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    if let Principal::Member(handle) = &req.principal {
        return handle_member(app, handle, req);
    }
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let f = |k: &str| p.get(k).and_then(Value::as_f64).map(|v| v as f32);
    match req.method.as_str() {
        "engine.execute" | "command" => {
            let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
            let params = p.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
            match app.execute(id, params) {
                Ok(v) => ok(v),
                Err(e) => err(e),
            }
        }
        "engine.commands" => {
            let reg = app.session.registry.clone();
            ok(Value::Array(
                reg.all()
                    .iter()
                    .map(|c| json!({"id": c.id, "label": c.label, "location": c.location, "shortcut": c.shortcut, "params": c.params, "enabled": (c.enabled)(&app.session).is_none()}))
                    .collect(),
            ))
        }
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
        "ui.text" => {
            let Some(t) = s("text") else { return err("missing text") };
            app.synthetic.push(egui::Event::Text(t.to_string()));
            ok(json!({"queued": true}))
        }
        "ui.screenshot" => Outcome::Screenshot { path: s("path").map(str::to_string) },
        "ui.render" => {
            let path = s("path").unwrap_or("/tmp/wordcraft-page.png");
            let page = p.get("page").and_then(Value::as_u64).unwrap_or(1);
            match app.session.run("file.exportPng", &json!({"path": path, "page": page, "scale": f("scale").unwrap_or(2.0)})) {
                Ok(v) => ok(v),
                Err(e) => err(e),
            }
        }
        "ui.resize" => {
            let (Some(w), Some(h)) = (f("width"), f("height")) else { return err("missing width/height") };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
            ok(json!({}))
        }
        "ui.focus" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            app.canvas.want_focus = true;
            ok(json!({}))
        }
        "app.quit" => {
            app.quit_requested = true;
            ok(json!({}))
        }
        other => {
            // Anything else: a command id.
            match app.execute(other, p.clone()) {
                Ok(v) => ok(v),
                Err(e) => err(e),
            }
        }
    }
}

pub fn save_screenshot(image: &egui::ColorImage, path: Option<&str>) -> Value {
    let path = path.unwrap_or("/tmp/wordcraft-screenshot.png");
    let [w, h] = image.size;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let rendered = wordcraft_render::Rendered { width: w as u32, height: h as u32, pixels: bytes };
    let png = rendered.to_png();
    #[cfg(target_arch = "wasm32")]
    let _ = (&png, path);
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Err(e) = std::fs::write(path, &png) {
            return json!({"ok": false, "error": format!("{path}: {e}")});
        }
    }
    json!({"ok": true, "result": {"path": path, "width": w, "height": h}})
}
