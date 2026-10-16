//! draw.io in a child webview: editing a diagram without leaving the document.
//!
//! The editor is draw.io itself — the open-source web app (Apache-2.0,
//! <https://github.com/jgraph/drawio>), which ships with the app in `Resources/drawio` and is
//! served from there by a custom protocol. Nothing is fetched and nothing is sent anywhere: the
//! picture never leaves the document.
//!
//! draw.io runs in its *embed mode* (`?embed=1&proto=json`), which is a conversation with the page
//! around it over `postMessage`: it answers `window.parent` and listens there. So the webview
//! loads [`PAGE`], a page of ours, with draw.io in an `<iframe>` inside it — the shape a diagram
//! editor is meant to be embedded in. That page hands the picture over (`load`, with the `<mxfile>`
//! read out of the PNG by [`wordcraft_diagram::png`], so draw.io has nothing to parse) and
//! forwards every answer to the app; the export comes back as `xmlpng`, a PNG with the edited
//! diagram inside it, which is exactly what the document stores.
//!
//! [`Editor::send`] talks to that page (`window.wcSend`), which passes the message on to draw.io.
//! Desktop only: a browser can't host a second browsing context inside the app's page.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use raw_window_handle::{HasWindowHandle, WindowHandle};
use serde_json::json;
use wry::dpi::{LogicalPosition, LogicalSize, Position, Size};
use wry::http::{HeaderValue, Request, Response, StatusCode, header};
use wry::{Rect, WebView, WebViewBuilder};

/// The app's window, which the editor is opened inside. The host hands it over on each frame;
/// [`Window::new`] takes anything with a raw window handle (an `eframe::Frame`, a winit window).
/// A handle that can't be used is remembered as `None`, so opening says so instead of panicking.
pub struct Window<'a>(Option<WindowHandle<'a>>);

impl<'a> Window<'a> {
    pub fn new(window: &'a impl HasWindowHandle) -> Self {
        Self(window.window_handle().ok())
    }
}

/// The page [`PAGE`] is served at, and the scheme its draw.io app is served under. On Windows the
/// page has to be an `http` URL — WebView2 only intercepts `http(s)`, and wry turns the request
/// back into the scheme for us.
#[cfg(target_os = "windows")]
const BASE: &str = "http://wcdrawio.local";
#[cfg(not(target_os = "windows"))]
const BASE: &str = "wcdrawio://local";

/// The name of the protocol, which is also what makes the two origins above the same origin (the
/// page and the app it frames are served here together).
const SCHEME: &str = "wcdrawio";

/// The page that hosts draw.io, with two values filled in on the way out: the draw.io page to
/// frame, and the diagram to hand it.
const PAGE: &str = include_str!("editor_page.html");

/// Where the draw.io web app lives: `Resources/drawio` in a packaged app.
const BUNDLE: &str = "drawio";

/// draw.io's embed mode: `spin` shows a spinner until the diagram arrives, `libraries` is the
/// shape panel, `noSaveBtn`/`noExitBtn` leave the buttons to the app, `offline` keeps it off the
/// network (the shape sets are all there, so nothing needs fetching).
const PARAMS: &str = "?embed=1&proto=json&spin=1&libraries=1&noSaveBtn=1&saveAndExit=0&noExitBtn=1&offline=1";

/// draw.io's own embed site, framed instead of the bundled app when a build has none.
const EMBED_SITE: &str = "https://embed.diagrams.net/";

/// What the editor has to say.
#[derive(Clone, Debug)]
pub enum Event {
    /// draw.io answered `load`: the diagram is on screen.
    Ready,
    /// The edited diagram, as a PNG with its XML inside it.
    Exported(Vec<u8>),
    /// draw.io's own Save or Exit was pressed: the edit is over.
    Closed,
    /// Something went wrong; the text is for the user.
    Failed(String),
}

/// An open draw.io editor over one picture.
pub struct Editor {
    view: WebView,
    rx: Receiver<String>,
    /// The picture's pixels, so the export comes back at the resolution it went in at.
    pixels: (u32, u32),
    /// The diagram's width in draw.io's own units, from the `load` answer.
    model_width: f64,
    /// Whether the picture has been handed over: draw.io asks for it once.
    sent: bool,
}

impl Editor {
    /// Open draw.io in a child of `window`, with `png` (a draw.io PNG) loaded into it.
    pub fn open(window: &Window<'_>, png: &[u8]) -> Result<Self, String> {
        // wry panics on a handle it can't use, so ask first.
        let Some(handle) = &window.0 else {
            return Err("the diagram editor can't be opened in this window".into());
        };
        // The diagram as draw.io wrote it into the PNG: handing draw.io the XML itself means the
        // picture can't fail to load over some PNG flavour it doesn't read back.
        let xml = wordcraft_diagram::png::drawio_xml(png).ok_or("the picture's diagram couldn't be read")?;
        let pixels = wordcraft_diagram::png::size(png).unwrap_or((0, 0));
        let root = drawio_dir();
        let src = match &root {
            Some(dir) => {
                log::info!("diagram editor: draw.io from {}", dir.display());
                format!("{BASE}/index.html{PARAMS}")
            }
            None => {
                log::warn!("diagram editor: no draw.io app was found, falling back to {EMBED_SITE}");
                format!("{EMBED_SITE}{PARAMS}")
            }
        };
        let served = Arc::new(Served { page: page(&src, &xml), root });
        let (tx, rx) = mpsc::channel();
        let handler = Arc::clone(&served);
        let view = WebViewBuilder::new()
            .with_url(format!("{BASE}/wceditor.html"))
            .with_custom_protocol(SCHEME.to_owned(), move |_id, request| serve(&handler, &request))
            .with_visible(false)
            .with_ipc_handler(move |request: wry::http::Request<String>| {
                // draw.io's thread: hand the message to the UI thread.
                let _ = tx.send(request.into_body());
            })
            .build_as_child(handle)
            .map_err(|e| format!("the diagram editor couldn't start: {e}"))?;
        Ok(Self { view, rx, pixels, model_width: 0.0, sent: false })
    }

    /// What draw.io has said since the last call. Cheap: call it every frame.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(message) = self.rx.try_recv() {
            self.handle(&message, &mut out);
        }
        out
    }

    /// Ask draw.io for the picture: the answer arrives as [`Event::Exported`].
    pub fn save(&mut self) {
        let scale = self.export_scale();
        self.send(&json!({"action": "export", "format": "xmlpng", "scale": scale, "border": 0}));
    }

    /// Put the webview at `x,y` (`y` measured from the bottom of the window, as AppKit counts)
    /// and give it `w × h`, in points.
    pub fn set_bounds(&self, x: f64, y: f64, w: f64, h: f64) {
        // A child webview of zero size is a webview that never draws again.
        let bounds =
            Rect { position: Position::Logical(LogicalPosition::new(x, y)), size: Size::Logical(LogicalSize::new(w.max(16.0), h.max(16.0))) };
        if let Err(e) = self.view.set_bounds(bounds) {
            log::warn!("diagram editor: set_bounds failed: {e}");
        }
    }

    /// Show or hide the editor (it starts hidden).
    pub fn set_visible(&self, visible: bool) {
        if let Err(e) = self.view.set_visible(visible) {
            log::warn!("diagram editor: set_visible failed: {e}");
        }
    }

    /// Move the keyboard to the diagram.
    pub fn focus(&self) {
        let _ = self.view.focus();
    }

    /// Run `javascript` in the page.
    fn send(&self, literal: &serde_json::Value) {
        let script = format!("window.wcSend({literal})");
        if let Err(e) = self.view.evaluate_script(&script) {
            log::warn!("diagram editor: evaluate_script failed: {e}");
        }
    }

    /// Hand the picture over, once draw.io says it is ready for it.
    fn load(&mut self) {
        if self.sent {
            return;
        }
        self.sent = true;
        if let Err(e) = self.view.evaluate_script("window.wcLoad()") {
            log::warn!("diagram editor: the diagram couldn't be handed over: {e}");
        }
    }

    /// The scale that gives the export the pixel size the picture went in with: draw.io draws
    /// `modelBounds` wide at scale 1.
    fn export_scale(&self) -> f64 {
        match (self.pixels.0, self.model_width) {
            (w, model) if w > 0 && model > 1.0 => (f64::from(w) / model).clamp(0.5, 8.0),
            _ => 1.0,
        }
    }

    fn handle(&mut self, message: &str, out: &mut Vec<Event>) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(message) else {
            // The plain protocol's `ready`, which a page that lost `proto=json` still sends.
            if message.trim() == "ready" {
                self.load();
            }
            return;
        };
        let event = value.get("event").and_then(|e| e.as_str()).unwrap_or_default();
        match event {
            // Only sent with `configure=1`, which we don't ask for; answer it all the same.
            "configure" => self.send(&json!({"action": "configure", "config": {}})),
            "init" => self.load(),
            "load" => {
                self.model_width = value.get("modelBounds").and_then(|b| b.get("width")).and_then(|w| w.as_f64()).unwrap_or(0.0);
                out.push(Event::Ready);
            }
            "export" => match value.get("data").and_then(|d| d.as_str()).and_then(png_from_data_url) {
                Some(bytes) => out.push(Event::Exported(bytes)),
                None => out.push(Event::Failed("draw.io didn't return a PNG".into())),
            },
            // draw.io's own Save or Exit (Cmd+S, Escape): keep what was drawn.
            "save" => self.save(),
            "exit" => out.push(Event::Closed),
            // The hosting page couldn't do its part (no diagram to load, no editor to frame).
            "wcError" => {
                let reason = value.get("message").and_then(|m| m.as_str()).unwrap_or("the diagram editor failed");
                out.push(Event::Failed(reason.into()));
            }
            other => log::debug!("diagram editor: unhandled message `{other}`"),
        }
    }
}

/// The draw.io web app to serve: the one packaged with the app, the one in the working tree, or
/// whatever `WORDCRAFT_DRAWIO_DIR` points at. `None` means there is none, and the editor falls
/// back to draw.io's embed site.
fn drawio_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("WORDCRAFT_DRAWIO_DIR") {
        candidates.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(BUNDLE));
        // macOS: WordCraft.app/Contents/MacOS/WordCraft → Contents/Resources/drawio.
        candidates.push(dir.join("../Resources").join(BUNDLE));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor").join(BUNDLE));
    candidates.into_iter().find(|dir| dir.join("index.html").is_file())
}

/// The editor page, with the draw.io page to frame and the diagram to hand it filled in.
fn page(src: &str, xml: &str) -> String {
    PAGE.replace("\"__WC_EDITOR__\"", &literal(src)).replace("\"__WC_DIAGRAM__\"", &literal(xml))
}

/// `text` as a JavaScript string literal. `<` is escaped so the diagram can't end the `<script>`
/// it sits in, whatever it was drawn with.
fn literal(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into()).replace('<', "\\u003c")
}

/// What the protocol serves: the page itself, and the draw.io app under it.
struct Served {
    page: String,
    /// `None` when there is no draw.io app around: the page still loads, framing draw.io's site.
    root: Option<PathBuf>,
}

/// Answer one request: the editor page, or a file of the draw.io app under `root`.
fn serve(served: &Served, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = percent_decode(request.uri().path());
    if path == "/" || path == "/wceditor.html" {
        return reply(200, "text/html; charset=utf-8", Cow::Owned(served.page.clone().into_bytes()));
    }
    let Some(root) = &served.root else {
        log::warn!("diagram editor: nothing serves {path}: no draw.io app is installed");
        return reply(404, "text/plain; charset=utf-8", Cow::Borrowed(b"no draw.io app".as_slice()));
    };
    let Some(file) = file_under(root, &path) else {
        log::warn!("diagram editor: {path} isn't part of the draw.io app");
        return reply(404, "text/plain; charset=utf-8", Cow::Borrowed(b"not found".as_slice()));
    };
    match std::fs::read(&file) {
        Ok(bytes) => reply(200, mime(&file), Cow::Owned(bytes)),
        Err(e) => {
            log::warn!("diagram editor: {} couldn't be read: {e}", file.display());
            reply(404, "text/plain; charset=utf-8", Cow::Borrowed(b"not found".as_slice()))
        }
    }
}

/// The file `path` names inside `root`, if it is one. `..` is refused: the app is a folder of
/// static files, and the page has no business reading anything else.
fn file_under(root: &Path, path: &str) -> Option<PathBuf> {
    let mut file = root.to_path_buf();
    for part in path.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            part => file.push(part),
        }
    }
    if file.is_dir() {
        file.push("index.html");
    }
    file.is_file().then_some(file)
}

/// A response with a status, a type and a body. Everything is same-origin, so no cache headers:
/// the editor is opened over a picture and closed again.
///
/// The status and the content type are this file's own constants, so neither is ever refused; if
/// one somehow were, the answer goes out with what it has instead of taking the editor down with
/// it, which is why nothing here builds a response the fallible way.
fn reply(status: u16, mime: &'static str, body: Cow<'static, [u8]>) -> Response<Cow<'static, [u8]>> {
    let mut response = Response::new(body);
    if let Ok(code) = StatusCode::from_u16(status) {
        *response.status_mut() = code;
    }
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(mime) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(ALLOW_ORIGIN) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    response
}

/// The one header that is on every answer: the page and the app are the same origin, and draw.io
/// asks for its files from inside an `<iframe>`.
const ALLOW_ORIGIN: &str = "*";

/// What a file of the app is, by its extension.
fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or_default() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" | "webmanifest" => "application/json",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" | "cur" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "txt" | "md" | "csv" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        // Stencil sets, templates and the like: draw.io reads them as text or binary and these
        // are all it needs to be told.
        _ => "application/octet-stream",
    }
}

/// `%20` and friends back into the characters they stand for (`draw.io` asks for files with
/// spaces in their names).
fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(byte) = hex(bytes[i + 1]).zip(hex(bytes[i + 2])).map(|(h, l)| h << 4 | l)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value of one hexadecimal digit.
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// `data:image/png;base64,…` to bytes.
fn png_from_data_url(data: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let rest = data.split_once("base64,")?.1;
    base64::engine::general_purpose::STANDARD.decode(rest.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn serves_files_inside_the_app() {
        let root = std::env::temp_dir().join(format!("wordcraft-diagram-serve-{}", std::process::id()));
        std::fs::create_dir_all(root.join("js")).unwrap();
        std::fs::write(root.join("index.html"), b"<html></html>").unwrap();
        std::fs::write(root.join("js/app.min.js"), b"// the app").unwrap();
        assert_eq!(file_under(&root, "/js/app.min.js"), Some(root.join("js/app.min.js")));
        // A folder is its index, and so is the root.
        assert_eq!(file_under(&root, "/index.html"), file_under(&root, "/"));
        assert_eq!(file_under(&root, "/"), Some(root.join("index.html")));
        // Anything the app doesn't have is no file at all.
        assert_eq!(file_under(&root, "/js/missing.js"), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_to_walk_out_of_the_app() {
        assert_eq!(file_under(Path::new("/app"), "/../secrets"), None);
        assert_eq!(file_under(Path::new("/app"), "/js/../../secrets"), None);
    }

    #[test]
    fn tells_files_apart_by_their_extension() {
        assert_eq!(mime(Path::new("a/b/app.min.js")), "text/javascript; charset=utf-8");
        assert_eq!(mime(Path::new("a/b/styles/grapheditor.css")), "text/css; charset=utf-8");
        assert_eq!(mime(Path::new("a/b/favicon.ico")), "image/x-icon");
        assert_eq!(mime(Path::new("a/b/stencils/rack.xml")), "application/xml");
    }

    #[test]
    fn decodes_what_drawio_asks_for() {
        assert_eq!(percent_decode("/stencils/My%20Shapes.xml"), "/stencils/My Shapes.xml");
        assert_eq!(percent_decode("/js/app.min.js"), "/js/app.min.js");
        // A stray `%` is a character, not an escape.
        assert_eq!(percent_decode("/100%"), "/100%");
    }

    #[test]
    fn the_page_carries_the_diagram_and_the_editor() {
        let page = page("wcdrawio://local/index.html?embed=1", "<mxfile><diagram/></mxfile>");
        assert!(page.contains("\"wcdrawio://local/index.html?embed=1\""));
        // The diagram sits in the page as a JavaScript string (its `<` escaped, see below).
        assert!(page.contains(r#""\u003cmxfile>\u003cdiagram/>\u003c/mxfile>""#), "{page}");
        // Nothing is left looking like a placeholder.
        assert!(!page.contains("__WC_EDITOR__") && !page.contains("__WC_DIAGRAM__"));
    }

    #[test]
    fn a_diagram_cannot_end_the_script_it_sits_in() {
        let page = page("x", "</script><script>alert(1)</script>");
        assert!(page.contains("\\u003c/script"));
        assert!(!page.contains("</script><script>alert(1)"));
    }

    #[test]
    fn reads_the_exported_png() {
        let data = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]));
        assert_eq!(png_from_data_url(&data), Some(vec![1, 2, 3]));
        assert_eq!(png_from_data_url("https://example.com/x.png"), None);
    }
}
