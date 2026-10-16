//! Office add-ins' task panes on the desktop: each pane's page is a system web view (`wry`:
//! WKWebView on macOS, WebView2 on Windows) laid over the pane WordCraft draws.
//!
//! Before the add-in's own scripts run, the page gets WordCraft's Office.js (`crates/office-js`,
//! built by `cargo xtask office-js` into an `office-js` folder next to the app). The page's
//! `context.sync()` messages arrive through the web view's IPC channel and are answered from
//! the document by [`wordcraft_ui_egui::office_pane::pump`]. Web views can only be created with
//! the window's handle, which the app has in `ui()`: [`Host::flush`] does the creating, moving and
//! answering once a frame.
//!
//! Sample add-in pages also load Microsoft's `office.js` from its CDN. The bootstrap script turns
//! such scripts off before they run (all platforms); on Windows the web view also can't resolve
//! those hosts ([`BLOCKED_HOSTS`]). wry has no safe hook for a page's sub-resource requests on
//! macOS (WKWebView content rule lists need Objective-C calls, which this crate can't make
//! without `unsafe`), so there the bootstrap is the only guard.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use wordcraft_ui_egui::office_pane::OfficeHost;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::raw_window_handle::HasWindowHandle;
use wry::{Rect, WebView, WebViewBuilder};

/// Hosts that serve Microsoft's `office.js`, which an add-in page must never run in WordCraft:
/// the Windows web view resolves them to nothing.
#[cfg(target_os = "windows")]
const BLOCKED_HOSTS: &[&str] = &["appsforoffice.microsoft.com", "officeapis.public.onecdn.static.microsoft"];

/// WebView2's arguments: wry's defaults (which giving any replaces), and no address for
/// [`BLOCKED_HOSTS`].
#[cfg(target_os = "windows")]
fn browser_args() -> String {
    let rules: Vec<String> = BLOCKED_HOSTS.iter().map(|h| format!("MAP {h} ~NOTFOUND")).collect();
    format!(
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --host-resolver-rules=\"{}\"",
        rules.join(", ")
    )
}

/// The scripts that make a page an Office add-in host: the bootstrap, the generated
/// `wasm-bindgen` glue and the library itself.
struct Library {
    script: String,
}

impl Library {
    /// Reads the built library from `WORDCRAFT_OFFICE_JS`, `office-js` beside the executable, or
    /// (macOS) the app bundle's `Resources/office-js`.
    fn find() -> Option<Library> {
        let mut dirs: Vec<std::path::PathBuf> = std::env::var_os("WORDCRAFT_OFFICE_JS").map(Into::into).into_iter().collect();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            dirs.push(dir.join("office-js"));
            dirs.push(dir.join("../Resources/office-js"));
        }
        let dir = dirs.into_iter().find(|d| d.join("office_js.js").is_file() && d.join("office_js_bg.wasm").is_file())?;
        let glue = std::fs::read_to_string(dir.join("office_js.js")).ok()?;
        let wasm = std::fs::read(dir.join("office_js_bg.wasm")).ok()?;
        let bootstrap = std::fs::read_to_string(dir.join("bootstrap.js")).unwrap_or_default();
        let script =
            format!("{bootstrap}\n{glue}\n;globalThis.__craftOfficeStart && globalThis.__craftOfficeStart(wasm_bindgen, \"{}\");", base64(&wasm));
        Some(Library { script })
    }
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(A.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A page: the web view once created, and what it should show.
struct View {
    url: String,
    webview: Option<WebView>,
    rect: egui::Rect,
    visible: bool,
    /// Scripts waiting for the page (answers, function commands, events).
    scripts: Vec<String>,
    closed: bool,
}

#[derive(Default)]
struct Inner {
    views: HashMap<String, View>,
    inbox: Arc<Mutex<Vec<(String, String)>>>,
    library: Option<Rc<Library>>,
    missing_library_logged: bool,
}

/// The web view host: one handle goes into [`wordcraft_ui_egui::Services::office`], the other
/// stays with the app for [`Host::flush`].
#[derive(Clone, Default)]
pub struct Host(Rc<RefCell<Inner>>);

impl Host {
    pub fn new() -> Host {
        let host = Host::default();
        host.0.borrow_mut().library = Library::find().map(Rc::new);
        host
    }

    /// Creates, moves, shows and hides the web views, and delivers the waiting scripts.
    pub fn flush(&self, window: &impl HasWindowHandle) {
        let mut inner = self.0.borrow_mut();
        let inbox = inner.inbox.clone();
        let library = inner.library.clone();
        if library.is_none() && !inner.missing_library_logged && !inner.views.is_empty() {
            log::warn!("Office add-ins: no office-js library next to the app (build it with `cargo xtask office-js`)");
            inner.missing_library_logged = true;
        }
        inner.views.retain(|_, v| !v.closed);
        for (key, view) in inner.views.iter_mut() {
            let bounds = Rect {
                position: LogicalPosition::new(f64::from(view.rect.min.x), f64::from(view.rect.min.y)).into(),
                size: LogicalSize::new(f64::from(view.rect.width().max(1.0)), f64::from(view.rect.height().max(1.0))).into(),
            };
            if view.webview.is_none() {
                let k = key.clone();
                let inbox = inbox.clone();
                let mut builder = WebViewBuilder::new()
                    .with_url(view.url.clone())
                    .with_bounds(bounds)
                    .with_visible(view.visible)
                    // Links that open a new window go to the browser, never a second web view.
                    .with_new_window_req_handler(|url, _features| {
                        open_with_system(&url);
                        wry::NewWindowResponse::Deny
                    })
                    // Add-in pages are https pages (as the manifest schema requires).
                    .with_navigation_handler(|url| url.starts_with("https://") || url == "about:blank")
                    .with_ipc_handler(move |req: wry::http::Request<String>| {
                        let mut q = match inbox.lock() {
                            Ok(g) => g,
                            Err(p) => p.into_inner(),
                        };
                        if q.len() < 10_000 {
                            q.push((k.clone(), req.into_body()));
                        }
                    });
                if let Some(lib) = &library {
                    builder = builder.with_initialization_script(lib.script.clone());
                }
                #[cfg(target_os = "windows")]
                {
                    use wry::WebViewBuilderExtWindows;
                    builder = builder.with_additional_browser_args(browser_args());
                }
                match builder.build_as_child(window) {
                    Ok(w) => view.webview = Some(w),
                    Err(e) => {
                        log::warn!("Office add-in pane {key}: {e}");
                        view.closed = true;
                        continue;
                    }
                }
            }
            if let Some(w) = &view.webview {
                let _ = w.set_bounds(bounds);
                let _ = w.set_visible(view.visible);
                for script in view.scripts.drain(..) {
                    if let Err(e) = w.evaluate_script(&script) {
                        log::warn!("Office add-in pane {key}: {e}");
                    }
                }
            }
        }
    }
}

/// Opens a web link in the system browser (links an add-in's page opens in a new window).
fn open_with_system(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    if let Err(e) = r {
        log::warn!("opening {url}: {e}");
    }
}

/// `f(<json as a JS string>)`: a JSON string literal is a JavaScript string literal.
fn call(function: &str, arg: &str) -> String {
    let quoted = serde_json::to_string(arg).unwrap_or_else(|_| "\"\"".into());
    format!("globalThis.{function} && globalThis.{function}({quoted});")
}

impl OfficeHost for Host {
    fn show(&mut self, key: &str, url: &str, rect: egui::Rect, _pixels_per_point: f32) {
        let mut inner = self.0.borrow_mut();
        let view = inner.views.entry(key.to_string()).or_insert_with(|| View {
            url: url.to_string(),
            webview: None,
            rect,
            visible: true,
            scripts: Vec::new(),
            closed: false,
        });
        if view.url != url
            && let Some(w) = &view.webview
        {
            let _ = w.load_url(url);
        }
        view.url = url.to_string();
        view.rect = rect;
        view.visible = true;
    }
    fn hide(&mut self, key: &str) {
        if let Some(v) = self.0.borrow_mut().views.get_mut(key) {
            v.visible = false;
        }
    }
    fn close(&mut self, key: &str) {
        if let Some(v) = self.0.borrow_mut().views.get_mut(key) {
            v.closed = true;
            v.visible = false;
        }
    }
    fn load_hidden(&mut self, key: &str, url: &str) {
        let mut inner = self.0.borrow_mut();
        inner.views.entry(key.to_string()).or_insert_with(|| View {
            url: url.to_string(),
            webview: None,
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, 1.0)),
            visible: false,
            scripts: Vec::new(),
            closed: false,
        });
    }
    fn call(&mut self, key: &str, entry: &str, json: &str) {
        if let Some(v) = self.0.borrow_mut().views.get_mut(key) {
            v.scripts.push(call(entry, json));
        }
    }
    fn take_messages(&mut self) -> Vec<(String, String)> {
        let inbox = self.0.borrow().inbox.clone();
        let mut q = match inbox.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        std::mem::take(&mut *q)
    }
    fn reply(&mut self, key: &str, json: &str) {
        if let Some(v) = self.0.borrow_mut().views.get_mut(key) {
            v.scripts.push(call("__craftOfficeReply", json));
        }
    }
}

thread_local! {
    // The app's handle on the host whose other handle is in `Services::office` (web views are
    // made on the UI thread).
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

/// The host for `Services::office`; the app keeps a handle for [`flush`].
pub fn service() -> Box<dyn OfficeHost> {
    let host = Host::new();
    HOST.with(|h| *h.borrow_mut() = Some(host.clone()));
    Box::new(host)
}

/// Once a frame, after the UI: the web views follow the panes.
pub fn flush(window: &impl HasWindowHandle) {
    HOST.with(|h| {
        if let Some(host) = h.borrow().as_ref() {
            host.flush(window);
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(&[0, 255, 128, 7]), "AP+ABw==");
        assert_eq!(super::call("f", "a\"b"), "globalThis.f && globalThis.f(\"a\\\"b\");");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn office_js_hosts_resolve_to_nothing_on_windows() {
        let args = super::browser_args();
        assert!(args.starts_with("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"));
        assert!(args.contains("MAP officeapis.public.onecdn.static.microsoft ~NOTFOUND"), "{args}");
    }
}
