use std::sync::Arc;

use craft::automation::control::{ControlRequest, Reply};
#[cfg(not(target_arch = "wasm32"))]
use craft::automation::dialogs;
use craft::automation::synthetic::input_method;
use craft::ui::runtime::commands::{ClipItem, ClipboardHandle};
use craft::ui::runtime::windows::{WindowAction, WindowHost};
use serde_json::{Value, json};

use super::screenshots::render_png;
use crate::WordApp;

fn ok(v: Value) -> Reply {
    Reply::from_result(Ok(v))
}

fn err(e: impl std::fmt::Display) -> Reply {
    Reply::from_result(Err(e.to_string()))
}

/// The screen point a point of `page` (page points from its top-left) is shown at; `None` when the
/// page is not on screen. One rule for `ui.click` in document coordinates and `ui.clickText`.
fn page_point_on_screen(canvas: &crate::canvas::CanvasState, page: usize, p: egui::Pos2) -> Option<egui::Pos2> {
    // The placed page's own scale: the page view draws a point at 96/72 screen points per point at 100%, so the
    // zoom alone put a page point short of where it shows.
    let placed = canvas.shown.pages.iter().find(|placed| placed.page == page)?;
    Some(placed.to_screen(craft::behavior::kurbo::Point::new(f64::from(p.x), f64::from(p.y))))
}

impl WordApp {
    /// The next screenshot token: distinct from every earlier one of this app, never zero.
    fn next_capture_token(&mut self) -> u64 {
        let mut token = self.capture_token.wrapping_add(1);
        if token == 0 {
            token = 1;
        }
        self.capture_token = token;
        token
    }
}

/// UI state for `ui.inspect`: preferences, view, windows, open dialogs, canvas geometry, perf.
pub fn inspect(app: &mut WordApp, ctx: &egui::Context) -> Value {
    let r = ctx.input_for(app.active_viewport(), |i| i.content_rect());
    let caret = crate::canvas::caret_screen(app).map(|(p, h)| json!({"x": p.x, "y": p.y, "h": h}));
    let rect = |r: egui::Rect| [r.left(), r.top(), r.width(), r.height()];
    // PLAN: assumed `Label::as_str` and `Tone: Debug` (sh.A3 `status_message`, expiry handled there)
    let status = app.status_message().map(|m| json!({"text": m.text.as_str(), "tone": format!("{:?}", m.tone).to_lowercase()}));
    json!({
        "ui": app.prefs(),
        "view": app.session.view,
        "windows": app.session.windows,
        "proofing": app.spelling_review,
        "dialogs": open_dialogs(app),
        "window": [r.width(), r.height()],
        "canvasRect": app.canvas.canvas_rect().map(rect),
        "pages": app.canvas.page_rects().map(|(_, r)| rect(r)).collect::<Vec<_>>(),
        "scale": app.canvas.zoom(),
        "caret": caret,
        "balloon": app.canvas.selected_balloon(),
        "focused": app.canvas.focused(ctx),
        "title": app.title_stem(),
        "status": status,
        "dirty": app.session.dirty,
        "document": app.session.document_id(),
        "documents": app.session.documents(),
        "perf": {"frameMs": app.frame_ms, "layoutMs": app.session.layout().ms},
    })
}

/// Programmatic calls never open the Save prompt, so closing unsaved work must be explicit:
/// save first, or pass `"force": true` (only a real boolean true counts) to discard it (#1268).
pub(crate) fn close_refused(app: &WordApp, id: &str, params: &Value) -> Option<&'static str> {
    // `file.close` closes only the drawn window when another window shows the same document:
    // the document and its work stay open, so nothing is lost and nothing is refused (the
    // interactive path asks nothing either) (#3274).
    if app.closes_only_its_window(id, params) {
        return None;
    }
    // The document the command closes: `file.close` the shown one, `file.closeDocument` the one
    // its `id` names (the active one by default) (#3274).
    let target = match id {
        "file.close" => app.session.document_id(),
        "file.closeDocument" => match params.get("id") {
            None | Some(Value::Null) => app.session.document_id(),
            // An `id` that isn't a document is the engine's error to give.
            Some(v) => v.as_u64()?,
        },
        _ => return None,
    };
    close_refused_for(app, target, params)
}

/// The refusal for closing the document `target` with unsaved work in it: the same work the
/// Save Changes prompt asks about, so the prompt and the refusal cover the same ground
/// (#1268, #3274).
fn close_refused_for(app: &WordApp, target: u64, params: &Value) -> Option<&'static str> {
    let force = params.get("force").and_then(Value::as_bool) == Some(true);
    if force {
        return None;
    }
    if app.document_unsaved_by_id(target) {
        return Some(UNSAVED);
    }
    // Comment text still being typed is unsaved work too (#1348): closing the document would
    // drop it.
    app.document_has_comment_drafts(target).then_some(COMMENT_UNSAVED)
}

const UNSAVED: &str = "the document has unsaved changes: save it first, or pass {\"force\": true} to discard them and close";
const COMMENT_UNSAVED: &str = "a comment is still being edited: finish it first, or pass {\"force\": true} to discard it";

/// Quitting closes every open document, so unsaved work needs the same explicit `force` as
/// `file.close` rather than going silently (#1268, #3274). The interactive path asks about
/// each document in turn; a programmatic call never prompts, so it refuses.
fn quit_refused(app: &WordApp, params: &Value) -> Option<&'static str> {
    let force = params.get("force").and_then(Value::as_bool) == Some(true);
    if force {
        return None;
    }
    app.has_unsaved_documents().then_some(UNSAVED_QUIT)
}

const UNSAVED_QUIT: &str = "a document has unsaved changes: save it first, or pass {\"force\": true} to quit without saving";

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
fn reply<E: std::fmt::Display, F: std::fmt::Display>(r: Result<Result<Value, E>, F>) -> Reply {
    match r {
        Ok(Ok(v)) => ok(v),
        Ok(Err(e)) => err(e),
        Err(e) => err(e),
    }
}

/// A document method addressed to open document `doc`, which isn't the active one: it runs on
/// that document, and the window keeps showing the active one.
fn handle_in(app: &mut WordApp, doc: u64, method: &str, p: &Value) -> Reply {
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
                if let Some(e) = close_refused_for(app, doc, &params) {
                    return err(e);
                }
                return match app.session.close_document(doc) {
                    Ok(()) => ok(json!({"closed": true, "id": doc, "documents": app.session.documents()})),
                    Err(e) => err(e),
                };
            }
            if id == "file.closeDocument" {
                // The document it names, or the one the request addresses.
                let target = match params.get("id") {
                    None | Some(Value::Null) => doc,
                    // An `id` that isn't a document is the engine's error to give.
                    Some(v) => match v.as_u64() {
                        Some(id) => id,
                        None => return err("`id` must be an open document's id"),
                    },
                };
                if let Some(e) = close_refused_for(app, target, &params) {
                    return err(e);
                }
            }
            let r = app.session.with_document(doc, |s| s.run(id, &params));
            // UI requests the command made carry `document`, so the window can tell they aren't
            // about the document it shows. A command that changes nothing opens the dialog it asks for, as in
            // `after_command`.
            let origin = app.user_origin;
            app.user_origin = origin || app.session.registry.get(id).is_some_and(|spec| !spec.mutates);
            for req in std::mem::take(&mut app.session.ui_requests) {
                app.handle_request(&req);
            }
            app.user_origin = origin;
            reply(r)
        }
    }
}

/// Dispatches one control request: commands and inspection, parity, the library's input and
/// dialog methods, screenshots and renders, resize, focus, clipboard and quit, and unknown ids as
/// commands. It refuses closing unsaved work and document operations without a document.
pub fn handle(app: &mut WordApp, ctx: &egui::Context, req: &ControlRequest) -> Reply {
    let before = app.session.document_id();
    let outcome = handle_request(app, ctx, req);
    // A document a script opened doesn't bring up the missing-fonts box.
    let after = app.session.document_id();
    if after != before {
        crate::font_download::skip(app, after);
    }
    outcome
}

/// The open dialogs for `ui.inspect`. The control library's dialog methods do not build for wasm32, so the web lists none.
#[cfg(not(target_arch = "wasm32"))]
fn open_dialogs(app: &WordApp) -> Value {
    dialogs::inspect(&app.dialog_host)
}

#[cfg(target_arch = "wasm32")]
fn open_dialogs(_app: &WordApp) -> Value {
    json!([])
}

/// The input and dialog methods the control library implements.
fn library_method(app: &mut WordApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Reply> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(r) = dialogs::dialog_method(&mut app.dialog_host, ctx, &req.method, &req.params) {
        return Some(Reply::from_result(r));
    }
    let host = WindowHost::of(ctx);
    let viewport = app.active_viewport();
    let canvas = &app.canvas;
    let map = |page: usize, p: egui::Pos2| page_point_on_screen(canvas, page, p);
    match input_method(&req.method, &req.params, &host, viewport, &map)? {
        Ok(()) => Some(Reply::AfterInput),
        Err(e) => Some(err(e)),
    }
}

/// `ui.clickText {page, x, y, ..}`: a click at a point of a page, as the library's `ui.click`
/// would deliver it at the screen point the page point is shown at.
fn click_text(app: &mut WordApp, ctx: &egui::Context, p: &Value) -> Reply {
    let page = p.get("page").and_then(Value::as_u64).map_or(0, |v| usize::try_from(v).unwrap_or(usize::MAX));
    let num = |k: &str| p.get(k).and_then(Value::as_f64).map(|v| v as f32);
    let (Some(x), Some(y)) = (num("x"), num("y")) else { return err("missing x/y") };
    let Some(pos) = page_point_on_screen(&app.canvas, page, egui::pos2(x, y)) else { return err("page not on screen") };
    // The other params (count, button, modifiers) are the library click's own.
    let mut click = p.as_object().cloned().unwrap_or_default();
    click.remove("page");
    click.insert("x".into(), json!(pos.x));
    click.insert("y".into(), json!(pos.y));
    click.insert("coordinates".into(), json!("screen"));
    match input_method("ui.click", &Value::Object(click), &WindowHost::of(ctx), app.active_viewport(), &|_: usize, _: egui::Pos2| None) {
        Some(Ok(())) => ok(json!({"screen": [pos.x, pos.y]})),
        Some(Err(e)) => err(e),
        None => err("ui.click is not available"),
    }
}

/// `ui.resize {width, height}`: sizes the active window's inner area, within what its surface can be.
fn resize(app: &mut WordApp, ctx: &egui::Context, p: &Value) -> Reply {
    let num = |k: &str| p.get(k).and_then(Value::as_f64).map(|v| v as f32);
    let (Some(w), Some(h)) = (num("width"), num("height")) else { return err("missing width/height") };
    // The window's surface is a GPU texture: no side may pass the largest one (#3734).
    let max_side = ctx.input(|i| i.max_texture_side) as f32 / ctx.pixels_per_point();
    let Some(size) = crate::window_geometry::requested_size(egui::vec2(w, h), max_side) else {
        return err("width and height must be positive numbers no larger than 3.4e38");
    };
    WindowHost::of(ctx).act(app.active_viewport(), WindowAction::SetInnerSize(size));
    ok(json!({"width": size.x, "height": size.y}))
}

/// `ui.clipboard {text?, html?, rtf?, png?}`: puts the given flavours on the clipboard.
fn clipboard(ctx: &egui::Context, p: &Value) -> Reply {
    let text = |k: &str| p.get(k).and_then(Value::as_str);
    let mut items = Vec::new();
    let mut offered = Vec::new();
    if let Some(t) = text("text") {
        items.push(ClipItem::Text(t.into()));
        offered.push("text");
    }
    if let Some(t) = text("html") {
        items.push(ClipItem::Html(t.into()));
        offered.push("html");
    }
    if let Some(t) = text("rtf") {
        items.push(ClipItem::Rtf(t.into()));
        offered.push("rtf");
    }
    if let Some(b) = text("png") {
        // PLAN: assumed `wordcraft_engine::cmd::insert::base64_decode` is exported beside `base64_encode`
        let Some(bytes) = wordcraft_engine::cmd::insert::base64_decode(b) else { return err("`png` is not base64") };
        let Ok(img) = image::load_from_memory(&bytes) else { return err("`png` is not a picture") };
        let rgba = img.to_rgba8();
        let (Ok(w), Ok(h)) = (usize::try_from(rgba.width()), usize::try_from(rgba.height())) else { return err("`png` is too large") };
        items.push(ClipItem::Image(Arc::new(egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw()))));
        offered.push("png");
    }
    if items.is_empty() {
        return err("give at least one of `text`, `html`, `rtf`, `png`");
    }
    // PLAN(platform-w3): the reply should name the flavours the backend could not write; `Written` is read there
    match ClipboardHandle::of(ctx).offer(&items) {
        Ok(_) => ok(json!({"offered": offered})),
        Err(e) => err(format!("{e:?}")),
    }
}

fn handle_request(app: &mut WordApp, ctx: &egui::Context, req: &ControlRequest) -> Reply {
    let p = &req.params;
    if DOC_METHODS.contains(&req.method.as_str()) {
        match addressed(app, p) {
            Ok(Some(doc)) => return handle_in(app, doc, &req.method, p),
            Ok(None) => {}
            Err(e) => return err(e),
        }
    }
    if let Some(r) = library_method(app, ctx, req) {
        return r;
    }
    let s = |k: &str| p.get(k).and_then(Value::as_str);
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
        "ui.clickText" => click_text(app, ctx, p),
        "ui.screenshot" => {
            let token = app.next_capture_token();
            Reply::screenshot(token, s("path").map(str::to_string), app.active_viewport())
        }
        "ui.render" => {
            let page = p.get("page").and_then(Value::as_u64).unwrap_or(1);
            let scale = p.get("scale").and_then(Value::as_f64).map_or(2.0, |v| v as f32);
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
        "ui.resize" => resize(app, ctx, p),
        "ui.focus" => {
            WindowHost::of(ctx).act(app.active_viewport(), WindowAction::Focus);
            app.canvas.focus_page(ctx);
            ok(json!({}))
        }
        // The root's close goes through the close guard, as the OS button's does.
        "ui.closeWindow" => ok(json!({"requested": WindowHost::of(ctx).act(egui::ViewportId::ROOT, WindowAction::Close)})),
        "ui.clipboard" => clipboard(ctx, p),
        "ui.replayAt" => {
            let Some(at) = p.get("at").and_then(Value::as_f64) else { return err("missing `at`") };
            // PLAN(canvas-w14): `crate::ink::replay_at(app: &mut WordApp, at: f32)`
            crate::ink::replay_at(app, at as f32);
            ok(json!({}))
        }
        "ui.accessTree" => Reply::access_tree(),
        "app.quit" => {
            // Quitting closes every open document, so unsaved work needs the same explicit
            // `force` as `file.close` rather than going silently (#3274).
            if let Some(e) = quit_refused(app, p) {
                return err(e);
            }
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
