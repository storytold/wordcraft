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

use craft_ui::capture::CaptureSlot;

use serde_json::{Value, json};

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
pub(crate) fn close_refused(app: &WordApp, id: &str, params: &Value) -> Option<&'static str> {
    let force = params.get("force").and_then(Value::as_bool) == Some(true);
    if force {
        return None;
    }
    if id == "file.close" && app.session.dirty {
        return Some("the document has unsaved changes: save it first, or pass {\"force\": true} to discard them and close");
    }
    // Comment text still being typed is unsaved work too (#1348): replacing the document would drop it.
    (crate::discards_document(id, params) && crate::panes::has_comment_drafts(app))
        .then_some("a comment is still being edited: finish it first, or pass {\"force\": true} to discard it")
}

/// The reply to a request that needs a document while none is open (#1129).
const NO_DOCUMENT: &str = "no document is open; create one (file.new) or open one (file.open) first";

/// Dispatches one JSON-lines control request: commands and inspection, parity, synthetic click,
/// drag, key, wheel and text input, screenshots and renders, resize, focus and quit, and unknown
/// ids as commands. It refuses closing unsaved work and document operations without a document.
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
                None => match render_png(app, page, scale) {
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
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            ok(json!({"width": size.x, "height": size.y}))
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
            match app.execute(other, p.clone()) {
                Ok(v) => ok(v),
                Err(e) => err(e),
            }
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

/// The bounded admission policy for `ui.screenshot` captures. This is the screenshot queue;
/// the engine's 64-entry UI-intent queue is unrelated. The per-image and aggregate limits are
/// working-set limits, not just raw RGBA: encoding a PNG and base64-encoding it for a reply can
/// briefly need several times the raw pixel buffer.
pub(crate) const SCREENSHOT_MAX_REQUESTS: usize = 4;
pub(crate) const SCREENSHOT_MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;
pub(crate) const SCREENSHOT_MAX_AGGREGATE_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const SCREENSHOT_MAX_RETRIES: u32 = 2;
/// An incompressible PNG of `raw` RGBA bytes. Deflate's stored-block worst case adds five bytes
/// per 64 KiB block; zlib/PNG framing and the encoder's small internal buffers get 64 KiB more.
/// Checked, because every byte count here comes from a hostile window size.
fn png_upper_bound(raw: usize) -> Option<usize> {
    let blocks = raw.div_ceil(65_535);
    raw.checked_add(blocks.checked_mul(5)?)?.checked_add(64 * 1024)
}

/// The length of the base64 text for `bytes`, rounded up, as `base64_encode` produces it.
fn base64_upper_bound(bytes: usize) -> Option<usize> {
    bytes.div_ceil(3).checked_mul(4)
}

/// Peak memory of one screenshot, in bytes: the completed `ColorImage`, the one straight-alpha
/// RGBA copy handed to the PNG encoder, the PNG buffer, and (for a reply without a path) the
/// base64 string. All of these can be live at once, so admission reserves their sum rather than
/// only the raw pixels or a PNG-only estimate. Checked throughout: a hostile surface size must
/// result in an error, never a wrapped budget.
fn working_set(raw: usize, inline_reply: bool) -> Option<usize> {
    let png = png_upper_bound(raw)?;
    let mut peak = raw.checked_add(raw)?.checked_add(png)?;
    if inline_reply {
        peak = peak.checked_add(base64_upper_bound(png)?)?;
    }
    Some(peak)
}

/// The surface reported by one `RawInput`, before it is tied to a document. Captured in the host
/// input hook because `InputState::viewport_rect` and the last painted screen rect are stale on a
/// hidden logic tick; RawInput is the host's answer for this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SurfaceGeometry {
    viewport: egui::Vec2,
    /// Native pixels per point times egui's zoom, so a zoom change is its own epoch.
    pixels_per_point: f32,
    max_texture_side: usize,
    minimized: bool,
    occluded: bool,
}

impl SurfaceGeometry {
    /// Derives this frame's capturable surface from raw viewport data, rejecting a missing, non-finite
    /// or zero-sized visible surface and recording minimized or occluded state.
    pub(crate) fn from_raw(raw: &egui::RawInput, ctx: &egui::Context) -> Option<Self> {
        let viewport = raw.viewports.get(&raw.viewport_id)?;
        let minimized = viewport.minimized == Some(true);
        let occluded = viewport.occluded == Some(true);
        // A minimized/occluded surface may report no rectangle at all. Its stale size is harmless
        // because such a surface is never capturable; a visible surface without a rectangle is not
        // fresh enough to use.
        let rect = match viewport.inner_rect.or(raw.screen_rect) {
            Some(rect) => rect,
            None if minimized || occluded => ctx.content_rect(),
            None => return None,
        };
        let zoom = ctx.zoom_factor();
        let pixels_per_point = viewport.native_pixels_per_point.map_or_else(|| ctx.pixels_per_point(), |native| native * zoom);
        let max_texture_side = raw.max_texture_side.filter(|side| *side > 0).unwrap_or_else(|| ctx.input(|i| i.max_texture_side));
        if !rect.width().is_finite() || !rect.height().is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return None;
        }
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 || !zoom.is_finite() || zoom <= 0.0 {
            return None;
        }
        Some(Self { viewport: rect.size(), pixels_per_point, max_texture_side, minimized, occluded })
    }

    /// Pairs the surface geometry with the document id a capture belongs to.
    pub(crate) fn with_document(self, document: u64) -> CaptureGeometry {
        CaptureGeometry { surface: self, document }
    }
}

/// The viewport geometry and document identity a capture is admitted, issued and completed
/// against. `WordApp` keeps a monotonic epoch: any change, including a change back, makes an
/// older image stale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CaptureGeometry {
    surface: SurfaceGeometry,
    document: u64,
}

impl CaptureGeometry {
    /// Returns the document id the capture was admitted for.
    pub(crate) fn document(self) -> u64 {
        self.document
    }

    /// Explains why the surface cannot be captured, when it is minimized or fully occluded, and
    /// returns `None` when it can.
    pub(crate) fn unavailable_reason(self) -> Option<&'static str> {
        if self.surface.minimized {
            Some("the window is minimized")
        } else if self.surface.occluded {
            Some("the window is fully occluded")
        } else {
            None
        }
    }

    /// The screenshot's physical size, rounded to the nearest pixel.
    ///
    /// The renderer's maximum texture side is deliberately not applied here. It is a texture
    /// limit, not the size of the surface the callback reports; capping one axis would distort
    /// the expected aspect and reject the real image. [`Self::oversize_reason`] rejects a window
    /// that cannot have a surface texture at all, before admission.
    pub(crate) fn expected_size(self) -> Option<[usize; 2]> {
        if self.unavailable_reason().is_some() {
            return None;
        }
        let axis = |points: f32| -> Option<usize> {
            let px = (f64::from(points) * f64::from(self.surface.pixels_per_point)).round();
            if !px.is_finite() || px <= 0.0 || px > usize::MAX as f64 {
                return None;
            }
            Some(px as usize)
        };
        Some([axis(self.surface.viewport.x)?, axis(self.surface.viewport.y)?])
    }

    /// Why this surface is too large to capture at all: a window larger than the renderer's
    /// maximum texture side has no surface texture to copy. Reject it explicitly rather than
    /// silently capping the expected size and failing exact comparisons later.
    pub(crate) fn oversize_reason(self) -> Option<String> {
        let [width, height] = self.expected_size()?;
        let max = self.surface.max_texture_side;
        (max > 0 && (width > max || height > max))
            .then(|| format!("screenshot failed: the window is {width}×{height} pixels, over the renderer's {max}-pixel texture limit"))
    }

    /// Checked `width * height * 4` bytes for the expected image.
    pub(crate) fn expected_bytes(self) -> Option<usize> {
        let [width, height] = self.expected_size()?;
        width.checked_mul(height)?.checked_mul(4)
    }

    /// The admission reservation for the complete capture working set (image plus encoding).
    /// `inline_reply` includes the base64 string a reply without a path produces.
    pub(crate) fn reserved_bytes(self, inline_reply: bool) -> Option<usize> {
        working_set(self.expected_bytes()?, inline_reply)
    }
}

/// One admitted capture. The request owns its token, path, deadline, immutable document identity,
/// geometry and reservation; the shared slot owns the image and its weak callback.
pub(crate) struct Shot {
    pub(crate) token: u64,
    pub(crate) path: Option<String>,
    pub(crate) reply: Sender<Value>,
    /// One monotonic deadline for the whole request, including retries.
    pub(crate) deadline: f64,
    pub(crate) ready_at: f64,
    pub(crate) frames: u32,
    pub(crate) issued: bool,
    pub(crate) retries: u32,
    /// The document the request was admitted for. It never changes; a document switch cancels.
    pub(crate) document: u64,
    pub(crate) geometry: CaptureGeometry,
    pub(crate) epoch: u64,
    pub(crate) expected: [usize; 2],
    /// Admission reservation, including the encoding working set.
    pub(crate) bytes: usize,
    pub(crate) slot: Option<CaptureSlot>,
    pub(crate) callback: Option<egui::ScreenshotCallback>,
}

/// All outstanding `ui.screenshot` requests.
#[derive(Default)]
pub(crate) struct Shots {
    pub(crate) token: u64,
    pub(crate) requests: Vec<Shot>,
}

impl Shots {
    /// Whether any screenshot request is outstanding.
    pub(crate) fn in_flight(&self) -> bool {
        !self.requests.is_empty()
    }

    /// Sums, saturating, the admission bytes reserved by outstanding screenshot requests.
    fn reserved(&self) -> usize {
        self.requests.iter().fold(0usize, |sum, shot| sum.saturating_add(shot.bytes))
    }
}

/// Builds a screenshot error reply carrying the request token.
fn error_reply(token: u64, message: impl std::fmt::Display) -> Value {
    json!({"ok": false, "error": message.to_string(), "request": token})
}

/// Sends one screenshot error reply, ignoring a disconnected receiver.
fn reject_shot(reply: Sender<Value>, token: u64, message: impl std::fmt::Display) {
    let _ = reply.send(error_reply(token, message));
}

/// Advances the screenshot token, wrapping and skipping zero so every request has a non-zero
/// token.
fn next_token(app: &mut WordApp) -> u64 {
    let mut token = app.shots.token.wrapping_add(1);
    if token == 0 {
        token = 1;
    }
    app.shots.token = token;
    token
}

/// Admit one screenshot request against the count and byte budgets **before** a capture is
/// issued. A rejected request gets one error reply, with its request token, and never evicts an
/// earlier request.
pub(crate) fn admit_screenshot(app: &mut WordApp, ctx: &egui::Context, path: Option<String>, reply: Sender<Value>) {
    let token = next_token(app);
    let Some(geometry) = app.capture_geometry.map(|g| CaptureGeometry { document: app.session.document_id(), ..g }) else {
        reject_shot(reply, token, "screenshot failed: the window geometry is not known yet");
        return;
    };
    if let Some(reason) = geometry.unavailable_reason() {
        reject_shot(reply, token, format!("screenshot failed: {reason}"));
        return;
    }
    let Some(expected) = geometry.expected_size() else {
        reject_shot(reply, token, "screenshot failed: the window has no capturable size");
        return;
    };
    if let Some(reason) = geometry.oversize_reason() {
        reject_shot(reply, token, reason);
        return;
    }
    let inline_reply = path.is_none();
    let Some(bytes) = geometry.reserved_bytes(inline_reply) else {
        reject_shot(reply, token, "screenshot failed: the window size overflows the screenshot budget");
        return;
    };
    if bytes > SCREENSHOT_MAX_IMAGE_BYTES {
        reject_shot(
            reply,
            token,
            format!(
                "screenshot budget exceeded: {bytes} bytes reserved for one {}×{} image, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit",
                expected[0], expected[1]
            ),
        );
        return;
    }
    if app.shots.requests.len() >= SCREENSHOT_MAX_REQUESTS {
        reject_shot(
            reply,
            token,
            format!("screenshot budget exceeded: {} requests are already outstanding (limit {SCREENSHOT_MAX_REQUESTS})", app.shots.requests.len()),
        );
        return;
    }
    let reserved = app.shots.reserved().saturating_add(bytes);
    if reserved > SCREENSHOT_MAX_AGGREGATE_BYTES {
        reject_shot(
            reply,
            token,
            format!("screenshot budget exceeded: {reserved} bytes reserved, over the {SCREENSHOT_MAX_AGGREGATE_BYTES}-byte aggregate limit"),
        );
        return;
    }
    let (slot, callback) = CaptureSlot::new(ctx, ctx.viewport_id(), SCREENSHOT_MAX_IMAGE_BYTES);
    let now = crate::now_ms();
    app.shots.requests.push(Shot {
        token,
        path,
        reply,
        deadline: now + 8000.0,
        ready_at: now + 120.0,
        frames: 0,
        issued: false,
        retries: 0,
        document: geometry.document(),
        geometry,
        epoch: app.capture_epoch,
        expected,
        bytes,
        slot: Some(slot),
        callback: Some(callback),
    });
    // The capture command is issued from `ui`; while hidden, logic keeps asking for a painted
    // frame until it gets there.
    ctx.request_repaint();
}

/// Issued only from `ui`, after the layout has settled. Counts one frame per host frame on the
/// first pass, revalidates geometry and the byte reservation, and sends one callback command.
/// `current_pass_index() == 0` is meaningful here because this function is already in a UI pass.
pub(crate) fn issue_screenshots(app: &mut WordApp, ctx: &egui::Context) {
    let now = crate::now_ms();
    let first_pass = ctx.current_pass_index() == 0;
    let mut failures: Vec<(usize, String)> = Vec::new();
    for i in 0..app.shots.requests.len() {
        let (issued, ready_at, frames) = {
            let shot = &app.shots.requests[i];
            (shot.issued, shot.ready_at, shot.frames)
        };
        if issued {
            continue;
        }
        let frames = if first_pass { frames.saturating_add(1) } else { frames };
        app.shots.requests[i].frames = frames;
        if now < ready_at || frames < 3 {
            continue;
        }
        let Some(current) = app.capture_geometry.map(|g| CaptureGeometry { document: app.session.document_id(), ..g }) else {
            failures.push((i, "screenshot failed: the window geometry is not known yet".into()));
            continue;
        };
        if let Some(reason) = current.unavailable_reason() {
            failures.push((i, format!("screenshot failed: {reason} while the capture was queued")));
            continue;
        }
        if current.document() != app.shots.requests[i].document {
            failures.push((i, "screenshot failed: the document changed while the capture was queued".into()));
            continue;
        }
        let Some(expected) = current.expected_size() else {
            failures.push((i, "screenshot failed: the window size overflows the screenshot budget".into()));
            continue;
        };
        if let Some(reason) = current.oversize_reason() {
            failures.push((i, reason));
            continue;
        }
        let inline_reply = app.shots.requests[i].path.is_none();
        let Some(bytes) = current.reserved_bytes(inline_reply) else {
            failures.push((i, "screenshot failed: the window size overflows the screenshot budget".into()));
            continue;
        };
        if bytes > SCREENSHOT_MAX_IMAGE_BYTES {
            failures.push((
                i,
                format!(
                    "screenshot budget exceeded: {bytes} bytes reserved for one image, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit"
                ),
            ));
            continue;
        }
        let others = app.shots.requests.iter().enumerate().filter(|(j, _)| *j != i).fold(0usize, |sum, (_, shot)| sum.saturating_add(shot.bytes));
        if others.saturating_add(bytes) > SCREENSHOT_MAX_AGGREGATE_BYTES {
            failures.push((i, format!("screenshot budget exceeded: outstanding requests would reserve over {SCREENSHOT_MAX_AGGREGATE_BYTES} bytes")));
            continue;
        }
        let epoch = app.capture_epoch;
        let shot = &mut app.shots.requests[i];
        shot.geometry = current;
        shot.expected = expected;
        shot.bytes = bytes;
        shot.epoch = epoch;
        if let Some(callback) = shot.callback.take() {
            ctx.send_viewport_cmd_to(shot.viewport(), egui::ViewportCommand::Screenshot(callback));
            shot.issued = true;
        }
    }
    for (i, message) in failures.into_iter().rev() {
        let shot = app.shots.requests.remove(i);
        if let Some(slot) = shot.slot {
            slot.cancel();
        }
        reject_shot(shot.reply, shot.token, message);
    }
    if app.shots.in_flight() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

impl Shot {
    /// The screenshot callback's viewport: the root window.
    fn viewport(&self) -> egui::ViewportId {
        egui::ViewportId::ROOT
    }
}

/// A completed image is either stale because the surface changed (safe to retry once within the
/// deadline) or invalid for this request (report it at once; retrying cannot help).
enum ImageCheck {
    Stale(String),
    Invalid(String),
}

/// The callback's physical size is the host surface texture's extent, rounded independently of
/// the expected `round(logical points × pixels_per_point)`. One physical pixel on each axis is
/// therefore the same surface seen through a different rounding boundary, not a resize.
fn dimensions_are_a_rounded_variant(expected: [usize; 2], actual: [usize; 2]) -> bool {
    // Both axes are derived from the same logical viewport and the same pixels-per-point; each
    // may round at a boundary one pixel away, but a larger change on either axis is a resize.
    expected[0].abs_diff(actual[0]) <= 1 && expected[1].abs_diff(actual[1]) <= 1
}

/// Rejects an empty, overflowing, underfilled or oversized image. A size one pixel away from the
/// expected surface is accepted as a rounding variant; a larger difference is stale.
fn validate_image(image: &egui::ColorImage, expected: [usize; 2]) -> Result<(), ImageCheck> {
    let [width, height] = image.size;
    if width == 0 || height == 0 {
        return Err(ImageCheck::Invalid("the screenshot has no pixels".to_string()));
    }
    // A screenshot is issued for one exact logical viewport. Exact equality would reject a valid
    // fractional-DPI capture whose callback rounds by one pixel; a larger difference, or a
    // different aspect, is a different surface.
    if !dimensions_are_a_rounded_variant(expected, [width, height]) {
        return Err(ImageCheck::Stale(format!(
            "screenshot is {width}×{height} pixels, expected {}×{}: the window changed size while it was captured",
            expected[0], expected[1]
        )));
    }
    let pixels = width.checked_mul(height).ok_or_else(|| ImageCheck::Invalid("screenshot dimensions overflow".to_string()))?;
    if image.pixels.len() != pixels {
        return Err(ImageCheck::Invalid(format!("screenshot has {} pixels for its {width}×{height} dimensions", image.pixels.len())));
    }
    // The largest image allowed by the one-pixel rounding allowance. The reservation was made for
    // the expected size; a sibling one pixel wider or taller is still the same surface.
    let largest = expected[0]
        .checked_add(1)
        .and_then(|w| expected[1].checked_add(1).and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| ImageCheck::Invalid("screenshot dimensions overflow".to_string()))?;
    if pixels > largest {
        return Err(ImageCheck::Invalid(format!("screenshot has {pixels} pixels, over the {largest} allowed by its reservation")));
    }
    Ok(())
}

/// What one pass over an outstanding screenshot request decides.
enum ShotAction {
    Keep,
    Remove(Value),
    Restart,
}

/// Classify one pending request: deadline first, then admission state, then the completed slot.
fn take_screenshot_action(app: &mut WordApp, index: usize, now: f64, current: Option<CaptureGeometry>, epoch: u64) -> ShotAction {
    let completed = {
        let shot = &mut app.shots.requests[index];
        if now >= shot.deadline {
            if let Some(slot) = shot.slot.take() {
                slot.cancel();
            }
            return ShotAction::Remove(error_reply(shot.token, "no frame was presented before the screenshot timeout"));
        }
        if !shot.issued {
            return ShotAction::Keep;
        }
        if let Some(reason) = current.and_then(CaptureGeometry::unavailable_reason) {
            if let Some(slot) = shot.slot.take() {
                slot.cancel();
            }
            return ShotAction::Remove(error_reply(shot.token, format!("screenshot failed: {reason} while the capture was pending")));
        }
        if current.is_some_and(|geometry| geometry.document() != shot.document) {
            if let Some(slot) = shot.slot.take() {
                slot.cancel();
            }
            return ShotAction::Remove(error_reply(shot.token, "screenshot failed: the document changed while the capture was queued"));
        }
        shot.slot.as_ref().and_then(|slot| slot.take())
    };
    match completed {
        None => ShotAction::Keep,
        Some(Err(error)) => ShotAction::Remove(error_reply(app.shots.requests[index].token, error)),
        Some(Ok(image)) => completed_screenshot_action(app, index, image, current, epoch),
    }
}

/// Judge one taken image: its geometry and epoch identity first, then the image's own validity.
fn completed_screenshot_action(
    app: &mut WordApp,
    index: usize,
    image: std::sync::Arc<egui::ColorImage>,
    current: Option<CaptureGeometry>,
    epoch: u64,
) -> ShotAction {
    let shot = &app.shots.requests[index];
    let stale = current != Some(shot.geometry) || epoch != shot.epoch;
    if stale {
        if shot.retries < SCREENSHOT_MAX_RETRIES {
            return ShotAction::Restart;
        }
        return ShotAction::Remove(error_reply(shot.token, "screenshot geometry changed while it was captured; no image was written"));
    }
    match validate_image(&image, shot.expected) {
        Err(ImageCheck::Stale(_)) if shot.retries < SCREENSHOT_MAX_RETRIES => ShotAction::Restart,
        Err(ImageCheck::Stale(message) | ImageCheck::Invalid(message)) => ShotAction::Remove(error_reply(shot.token, message)),
        Ok(()) => ShotAction::Remove(save_screenshot(image.as_ref(), shot.path.as_deref(), shot.token)),
    }
}

/// Drop one request at `index` with its reply, cancelling any slot still held.
fn remove_shot(app: &mut WordApp, index: usize, value: Value) {
    let shot = app.shots.requests.remove(index);
    if let Some(slot) = shot.slot {
        slot.cancel();
    }
    let _ = shot.reply.send(value);
}

/// Drop one request at `index` and send an error reply for it.
fn reject_removed_shot(app: &mut WordApp, index: usize, message: impl std::fmt::Display) {
    let shot = app.shots.requests.remove(index);
    if let Some(slot) = shot.slot {
        slot.cancel();
    }
    reject_shot(shot.reply, shot.token, message);
}

/// Reissue the request at `index` against the current geometry. Returns whether it was reissued:
/// a request removed for a refusal keeps `index` for the request that slides into its place.
fn restart_shot(app: &mut WordApp, ctx: &egui::Context, index: usize, now: f64, current: Option<CaptureGeometry>, epoch: u64) -> bool {
    let Some(current) = current else {
        reject_removed_shot(app, index, "screenshot failed: the window geometry is not known");
        return false;
    };
    let Some(expected) = current.expected_size() else {
        reject_removed_shot(app, index, "screenshot failed: the window size overflows the screenshot budget");
        return false;
    };
    if let Some(reason) = current.oversize_reason() {
        reject_removed_shot(app, index, reason);
        return false;
    }
    let inline_reply = app.shots.requests[index].path.is_none();
    let Some(bytes) = current.reserved_bytes(inline_reply) else {
        reject_removed_shot(app, index, "screenshot failed: the window size overflows the screenshot budget");
        return false;
    };
    let others = app.shots.requests.iter().enumerate().filter(|(j, _)| *j != index).fold(0usize, |sum, (_, shot)| sum.saturating_add(shot.bytes));
    if bytes > SCREENSHOT_MAX_IMAGE_BYTES || others.saturating_add(bytes) > SCREENSHOT_MAX_AGGREGATE_BYTES {
        reject_removed_shot(app, index, "screenshot budget exceeded while retrying after a resize");
        return false;
    }
    let (slot, callback) = CaptureSlot::new(ctx, ctx.viewport_id(), SCREENSHOT_MAX_IMAGE_BYTES);
    let shot = &mut app.shots.requests[index];
    shot.slot = Some(slot);
    shot.callback = Some(callback);
    shot.issued = false;
    shot.ready_at = now + 120.0;
    shot.frames = 0;
    shot.retries = shot.retries.saturating_add(1);
    shot.geometry = current;
    shot.expected = expected;
    shot.bytes = bytes;
    shot.epoch = epoch;
    true
}

/// Called from `logic`: take each completed slot once, check the deadline **before** writing,
/// validate request/document/geometry identity, and only then encode and reply. A late, stale or
/// oversized image is dropped without a file; a document switch cancels instead of rebinding.
pub(crate) fn drain_screenshots(app: &mut WordApp, ctx: &egui::Context) {
    let now = crate::now_ms();
    let current = app.capture_geometry.map(|g| CaptureGeometry { document: app.session.document_id(), ..g });
    let epoch = app.capture_epoch;
    let mut i = 0;
    while i < app.shots.requests.len() {
        match take_screenshot_action(app, i, now, current, epoch) {
            ShotAction::Keep => i += 1,
            ShotAction::Remove(value) => remove_shot(app, i, value),
            ShotAction::Restart => {
                if restart_shot(app, ctx, i, now, current, epoch) {
                    i += 1;
                }
            }
        }
    }
    if app.shots.in_flight() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

/// The screenshot as a straight-alpha PNG, using one RGBA copy rather than the extra full-image
/// copy [`wordcraft_render::Rendered::to_png`] makes for its premultiplied input. The callback's
/// `ColorImage` is premultiplied; PNG stores straight alpha.
fn screenshot_png(image: &egui::ColorImage) -> Option<Vec<u8>> {
    let [width, height] = image.size;
    if width == 0 || height == 0 {
        return None;
    }
    let pixels = width.checked_mul(height)?;
    if image.pixels.len() != pixels {
        return None;
    }
    let mut rgba = Vec::with_capacity(pixels.checked_mul(4)?);
    for color in &image.pixels {
        let [r, g, b, a] = color.to_array();
        if a == 0 || a == 255 {
            rgba.extend_from_slice(&[r, g, b, a]);
        } else {
            let straight = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
            rgba.extend_from_slice(&[straight(r), straight(g), straight(b), a]);
        }
    }
    let img = image::RgbaImage::from_raw(u32::try_from(width).ok()?, u32::try_from(height).ok()?, rgba)?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// The window screenshot: written to `path` when one was given, else base64 PNG in the reply
/// (never a fixed name in the shared temporary folder). Every reply carries the request token so
/// a client can correlate it with the admission, timeout and error replies.
pub fn save_screenshot(image: &egui::ColorImage, path: Option<&str>, token: u64) -> Value {
    let [w, h] = image.size;
    let Some(raw) = w.checked_mul(h).and_then(|pixels| pixels.checked_mul(4)) else {
        return error_reply(token, "screenshot dimensions overflow the byte count");
    };
    let inline_reply = path.is_none();
    let Some(peak) = working_set(raw, inline_reply) else {
        return error_reply(token, "screenshot working set overflows the byte count");
    };
    if peak > SCREENSHOT_MAX_IMAGE_BYTES {
        return error_reply(
            token,
            format!("screenshot needs {peak} bytes of working set, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit"),
        );
    }
    let Some(png) = screenshot_png(image) else {
        return error_reply(token, "screenshot PNG encoding failed");
    };
    if png.len() > SCREENSHOT_MAX_IMAGE_BYTES {
        return error_reply(token, format!("screenshot PNG uses {} bytes, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit", png.len()));
    }
    let Some(path) = path else {
        let encoded = wordcraft_engine::cmd::insert::base64_encode(&png);
        if encoded.len() > SCREENSHOT_MAX_IMAGE_BYTES {
            return error_reply(
                token,
                format!("screenshot base64 reply uses {} bytes, over the {SCREENSHOT_MAX_IMAGE_BYTES}-byte per-image limit", encoded.len()),
            );
        }
        return json!({"ok": true, "request": token, "result": {"png": encoded, "width": w, "height": h}});
    };
    #[cfg(target_arch = "wasm32")]
    let _ = (&png, path);
    #[cfg(not(target_arch = "wasm32"))]
    {
        if !path.to_ascii_lowercase().ends_with(".png") {
            return error_reply(token, format!("{path}: a screenshot path must end in .png"));
        }
        if let Err(e) = wordcraft_engine::io::write_file(path, &png) {
            return error_reply(token, e);
        }
    }
    json!({"ok": true, "request": token, "result": {"path": path, "width": w, "height": h}})
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a visible root-surface fixture with the given viewport, scale and texture limit for
    /// screenshot tests.
    fn surface(viewport: egui::Vec2, pixels_per_point: f32, max_texture_side: usize) -> SurfaceGeometry {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, viewport);
        let mut raw = egui::RawInput { screen_rect: Some(rect), max_texture_side: Some(max_texture_side), ..Default::default() };
        raw.viewports.insert(
            egui::ViewportId::ROOT,
            egui::ViewportInfo {
                inner_rect: Some(rect),
                native_pixels_per_point: Some(pixels_per_point),
                minimized: Some(false),
                occluded: Some(false),
                ..Default::default()
            },
        );
        SurfaceGeometry::from_raw(&raw, &ctx).expect("a visible surface")
    }

    /// Builds a capture geometry fixture for document id 7.
    fn geometry(viewport: egui::Vec2, pixels_per_point: f32, max_texture_side: usize) -> CaptureGeometry {
        CaptureGeometry { surface: surface(viewport, pixels_per_point, max_texture_side), document: 7 }
    }

    /// Creates a black test image of the given size.
    fn image(width: usize, height: usize) -> egui::ColorImage {
        egui::ColorImage::filled([width, height], egui::Color32::BLACK)
    }

    /// The reservation covers every live copy: the image, the RGBA copy the encoder consumes, the
    /// PNG buffer and the base64 reply string. A path reply needs no base64.
    #[test]
    fn screenshot_working_set_counts_image_copy_png_and_base64() {
        let raw = 1_000_000usize;
        let png = png_upper_bound(raw).expect("the PNG bound");
        let base64 = base64_upper_bound(png).expect("the base64 bound");
        assert_eq!(working_set(raw, false), Some(raw + raw + png));
        assert_eq!(working_set(raw, true), Some(raw + raw + png + base64));
        assert!(working_set(raw, true).expect("bounded") > raw * 4, "raw pixels alone are not the peak");
        assert_eq!(base64_upper_bound(0), Some(0));
        assert_eq!(base64_upper_bound(1), Some(4));
        assert_eq!(base64_upper_bound(4), Some(8), "rounded up, as the encoder pads");
        assert_eq!(base64_upper_bound(usize::MAX), None, "overflow is refused, not wrapped");
        assert_eq!(working_set(usize::MAX, true), None);
    }

    /// The 128 MiB per-image budget is a working-set budget, not a raw-pixel budget: a
    /// 3000×3000 inline reply no longer fits even though its raw pixels are only 36 MB, while a
    /// path reply for the same image (no base64) does.
    #[test]
    fn the_per_image_budget_reflects_the_encoding_peak() {
        let fits = geometry(egui::vec2(2000.0, 2000.0), 1.0, 16_384);
        assert!(fits.reserved_bytes(true).expect("bounded") <= SCREENSHOT_MAX_IMAGE_BYTES);

        let too_big_inline = geometry(egui::vec2(3000.0, 3000.0), 1.0, 16_384);
        assert!(too_big_inline.reserved_bytes(true).expect("bounded") > SCREENSHOT_MAX_IMAGE_BYTES);
        assert!(too_big_inline.reserved_bytes(false).expect("bounded") <= SCREENSHOT_MAX_IMAGE_BYTES);
    }

    /// A texture limit must not cap the expected physical size: capping one axis changes the
    /// expected aspect and makes the real callback look stale. Out-of-limit surfaces are rejected
    /// explicitly instead.
    #[test]
    fn expected_size_is_not_capped_by_the_texture_limit() {
        let wide = geometry(egui::vec2(10_000.0, 2_000.0), 1.0, 8_192);
        assert_eq!(wide.expected_size(), Some([10_000, 2_000]));
        let reason = wide.oversize_reason().expect("over the texture limit");
        assert!(reason.contains("texture limit"), "{reason}");

        let fits = geometry(egui::vec2(8_000.0, 2_000.0), 1.0, 8_192);
        assert_eq!(fits.expected_size(), Some([8_000, 2_000]));
        assert_eq!(fits.oversize_reason(), None);
    }

    /// Fractional DPI rounds the host surface and the expected size at different boundaries: a
    /// one-pixel difference on each axis is the same capture, a two-pixel difference is stale.
    #[test]
    fn a_callback_one_pixel_off_the_rounded_size_is_accepted() {
        let g = geometry(egui::vec2(100.4, 100.4), 1.5, 16_384);
        assert_eq!(g.expected_size(), Some([151, 151]));
        assert!(validate_image(&image(152, 150), [151, 151]).is_ok());
        assert!(matches!(validate_image(&image(153, 149), [151, 151]), Err(ImageCheck::Stale(_))));
        assert!(matches!(validate_image(&image(800, 600), [151, 151]), Err(ImageCheck::Stale(_))));
    }

    /// A real resize is stale even when one axis happens to be close: both axes must match within
    /// the one-pixel rounding allowance.
    #[test]
    fn a_two_pixel_change_is_a_resize_not_rounding() {
        assert!(dimensions_are_a_rounded_variant([1000, 1000], [1001, 999]));
        assert!(!dimensions_are_a_rounded_variant([1000, 1000], [1200, 800]));
        assert!(!dimensions_are_a_rounded_variant([1000, 1000], [1002, 999]));
        assert!(!dimensions_are_a_rounded_variant([1000, 1000], [1000, 998]));
    }
}

#[cfg(test)]
mod screenshot_tests {
    use super::save_screenshot;

    struct Cleanup(std::path::PathBuf);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A screenshot path must end in `.png` and is refused otherwise; a PNG is written, while a
    /// symlinked PNG target is not followed or overwritten.
    #[test]
    fn screenshot_path_must_be_png_and_does_not_follow_symlinks() {
        let dir = std::env::temp_dir().join(format!("wc-shot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _cleanup = Cleanup(dir.clone());
        let img = egui::ColorImage::filled([2, 2], egui::Color32::RED);

        let txt = dir.join("shot.txt");
        let r = save_screenshot(&img, txt.to_str(), 101);
        assert_eq!(r["ok"], false);
        assert!(!txt.exists());

        let png = dir.join("shot.png");
        let r = save_screenshot(&img, png.to_str(), 102);
        assert_eq!(r["ok"], true);
        assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG"));

        #[cfg(unix)]
        {
            let victim = dir.join("victim.txt");
            std::fs::write(&victim, "keep").unwrap();
            let link = dir.join("link.png");
            std::os::unix::fs::symlink(&victim, &link).unwrap();
            let _ = save_screenshot(&img, link.to_str(), 103);
            assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        }

        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let roots = wordcraft_engine::file_access::AutomationRoots::new(None, Some(&other)).unwrap();
        let outside = dir.join("outside.png");
        let r = wordcraft_engine::file_access::confine(roots.as_ref(), || save_screenshot(&img, outside.to_str(), 104));
        assert_eq!(r["ok"], false);
        assert!(!outside.exists());
    }
}
