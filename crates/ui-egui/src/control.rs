//! Programmatic control of the running app (agents, tests, MCP bridge).
//!
//! Methods (JSON lines over the host's transport, see `docs/control-protocol.md`):
//! - `engine.execute {command, params}`: run any command (engine or `ui.*`)
//! - `engine.commands {search?}`: engine and app print commands with enablement and parameters
//! - `document.inspect`: document structure; `ui.inspect`: UI state, canvas geometry, perf
//! - `ui.wheel {x, y, dx?, dy, unit?, phase?}`: a mouse-wheel / trackpad scroll at a window point.
//! - `ui.click {x, y, button?, count?, shift?, cmd?}`, `ui.move {x, y}`, `ui.drag {x, y, toX, toY}`:
//!   real pointer input in screen points (reaches every widget)
//! - `ui.clickText {page, x, y}`: click at a page position (points from the page's top-left)
//! - `ui.key {key, shift?, alt?, cmd?}`, `ui.text {text}`: keyboard input through egui
//! - `ui.screenshot {path?}`: PNG of the window; `ui.render {path?, page?, scale?}`: page PNG
//!   (without `path`, both reply with the PNG as base64 `png`)
//! - `ui.parity`: feature catalog parity

use std::sync::mpsc::Sender;

use serde_json::{Value, json};

use wordcraft_engine::file_access;

use crate::WordApp;

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Value>,
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, std::sync::mpsc::Receiver<Value>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx }, rx)
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

/// Programmatic calls never open the Save prompt, so closing unsaved work must be explicit:
/// save first, or pass `"force": true` (only a real boolean true counts) to discard it (#1268).
fn close_refused(app: &WordApp, id: &str, params: &Value) -> Option<&'static str> {
    let force = params.get("force").and_then(Value::as_bool) == Some(true);
    (id == "file.close" && app.session.dirty && !force)
        .then_some("the document has unsaved changes: save it first, or pass {\"force\": true} to discard them and close")
}

/// The reply to a request that needs a document while none is open (#1129).
const NO_DOCUMENT: &str = "no document is open; create one (file.new) or open one (file.open) first";

pub fn handle(app: &mut WordApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let f = |k: &str| p.get(k).and_then(Value::as_f64).map(|v| v as f32);
    match req.method.as_str() {
        "engine.execute" | "command" => {
            let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
            let params = p.get("params").cloned().filter(|v| !v.is_null()).unwrap_or(json!({}));
            if let Some(e) = close_refused(app, id, &params) {
                return err(e);
            }
            file_access::invoke_automation(|| match app.execute(id, params) {
                Ok(v) => ok(v),
                Err(e) => err(e),
            })
        }
        "engine.commands" => {
            let reg = app.session.registry.clone();
            let mut commands: Vec<Value> = reg.all().iter()
                .map(|c| json!({"id": c.id, "label": c.label, "location": c.location, "shortcut": c.shortcut, "params": c.params, "enabled": (c.enabled)(&app.session).is_none()}))
                .collect();
            commands.extend(crate::printing::commands(app));
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
            app.synthetic.push(egui::Event::MouseWheel { unit, delta, phase, modifiers: mods(p) });
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
                None => match render_png(app, page, scale) {
                    Ok(v) => ok(v),
                    Err(e) => err(e),
                },
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
            if let Some(e) = close_refused(app, other, p) {
                return err(e);
            }
            file_access::invoke_automation(|| match app.execute(other, p.clone()) {
                Ok(v) => ok(v),
                Err(e) => err(e),
            })
        }
    }
}

/// A page rendered by the engine, as base64 PNG in the reply (the same shape as the headless
/// MCP server's `ui.render`).
fn render_png(app: &WordApp, page: u64, scale: f32) -> Result<Value, String> {
    let page = usize::try_from(page.max(1)).unwrap_or(usize::MAX).saturating_sub(1);
    let l = app.session.export_layout();
    let pg = l.pages.get(page).ok_or_else(|| format!("no page {}", page + 1))?;
    let img = wordcraft_render::render_page(&app.session.doc, pg, scale.clamp(0.1, 8.0), &Default::default());
    let png = img.to_png();
    Ok(json!({"png": wordcraft_engine::cmd::insert::base64_encode(&png), "width": img.width, "height": img.height, "pages": l.pages.len()}))
}

/// The window screenshot: written to `path` when one was given, else base64 PNG in the reply
/// (never a fixed name in the shared temporary folder).
pub fn save_screenshot(image: &egui::ColorImage, path: Option<&str>) -> Value {
    let [w, h] = image.size;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let rendered = wordcraft_render::Rendered { width: w as u32, height: h as u32, pixels: bytes };
    let png = rendered.to_png();
    let Some(path) = path else {
        return json!({"ok": true, "result": {"png": wordcraft_engine::cmd::insert::base64_encode(&png), "width": w, "height": h}});
    };
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
