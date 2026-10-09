//! The browser shell: web `Services`, drag-and-drop, and the eframe web runner.

use serde_json::json;
use wasm_bindgen::JsCast as _;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Inbox, Services, WordApp};

use crate::host::{self, Host, Msg};

const DOC_EXTS: &[&str] = &["docx", "docm", "dotx", "odt", "rtf", "txt", "md", "html", "htm", "json"];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const CANVAS_ID: &str = "wordcraft_canvas";
const LOADING_ID: &str = "wordcraft_loading";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    if let Some(c) = host::accent_from_url() {
        wordcraft_ui_egui::theme::set_app_color(c);
    }
    let host = Host::from_url();
    wasm_bindgen_futures::spawn_local(async move {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let mut options = eframe::WebOptions::default();
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!("wordcraft-web: wgpu backend {:?}", rs.adapter.get_info().backend);
                    }
                    let inbox: Inbox = Inbox::default();
                    let doc = if query().contains("sample") { wordcraft_engine::sample::sample_document() } else { wordcraft_doc::Document::new() };
                    let mut app = WordApp::new(Session::new(doc), services(inbox.clone(), cc.egui_ctx.clone(), host.clone()));
                    app.autosave = false;
                    if let Some(h) = &host {
                        let ctx = cc.egui_ctx.clone();
                        h.listen(move || ctx.request_repaint());
                        h.post("ready", json!({}), None);
                    }
                    Ok(Box::new(WebShell { app, inbox, host, dirty_sent: false }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>WordCraft failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

/// Wraps the app to read dropped files asynchronously (browsers can't read them synchronously)
/// and feed them through the inbox.
struct WebShell {
    app: WordApp,
    inbox: Inbox,
    host: Option<Host>,
    dirty_sent: bool,
}

impl WebShell {
    /// Requests from the host page (host mode), and unsaved-changes news back to it.
    fn host_messages(&mut self) {
        let Some(host) = self.host.clone() else { return };
        let msgs = std::mem::take(&mut *host.queue.borrow_mut());
        for msg in msgs {
            match msg {
                // Opened right away (not through the inbox), so a `run` sent after `open` sees the new document.
                Msg::Open { id, name, bytes } => {
                    let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
                    let r = self.app.run("file.open", json!({"path": name, "data": data}));
                    if r.is_ok() {
                        self.app.ui.backstage = false;
                    }
                    host.post("result", outcome(id, r), None);
                }
                Msg::Run { id, command, params } => {
                    let r = self.app.run(&command, params);
                    host.post("result", outcome(id, r), None);
                }
                Msg::Save { id, name } => {
                    // The bytes go back through `download` (services), tagged with this id.
                    *host.pending_save.borrow_mut() = Some(id.clone());
                    let params = name.map(|n| json!({"path": n})).unwrap_or_else(|| json!({}));
                    if let Err(error) = self.app.run("file.save", params) {
                        host.pending_save.borrow_mut().take();
                        host.post("result", outcome(id, Err(error)), None);
                    }
                }
            }
        }
        if self.app.session.dirty != self.dirty_sent {
            self.dirty_sent = self.app.session.dirty;
            host.post("dirty", json!({"value": self.dirty_sent}), None);
        }
    }
}

/// A `result` message for the host.
fn outcome(id: serde_json::Value, r: Result<serde_json::Value, String>) -> serde_json::Value {
    match r {
        Ok(result) => json!({"id": id, "ok": true, "result": result}),
        Err(error) => json!({"id": id, "ok": false, "error": error}),
    }
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for f in dropped {
            let inbox = self.inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped".into());
                match f.bytes_async().await {
                    Ok(bytes) => {
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                        ctx.request_repaint();
                    }
                    Err(e) => log::error!("couldn't read dropped file {name}: {e}"),
                }
            });
        }
        self.host_messages();
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

fn services(inbox: Inbox, ctx: egui::Context, host: Option<Host>) -> Services {
    let open_inbox = inbox.clone();
    Services {
        open_async: Some(Box::new(move |purpose: &str| {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            let dialog = if purpose == "picture" {
                rfd::AsyncFileDialog::new().add_filter("Pictures", IMAGE_EXTS)
            } else {
                rfd::AsyncFileDialog::new().add_filter("Documents", DOC_EXTS)
            };
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = dialog.pick_file().await else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
        })),
        // Exports ask for a name; the browser decides where the download goes.
        pick_save: Some(Box::new(|name: &str| Some(name.to_string()))),
        download: Some(Box::new(move |name: &str, bytes: &[u8]| {
            // Host mode: a saved document goes back to the host page instead of downloading.
            if let Some(h) = &host
                && name.to_ascii_lowercase().ends_with(".docx")
            {
                let id = h.pending_save.borrow_mut().take().unwrap_or(serde_json::Value::Null);
                h.post("saved", json!({"id": id, "name": name}), Some(bytes));
                return;
            }
            if let Err(e) = download(name, bytes) {
                log::error!("download of {name} failed: {e}");
            }
        })),
        inbox: Some(inbox),
        ..Default::default()
    }
}

/// Trigger a browser download of `bytes` named after the last component of `path`.
fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "document".into());
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(&name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(&name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    // Revoke after the click has been dispatched; the download keeps its own reference.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("odt") => "application/vnd.oasis.opendocument.text",
        Some("rtf") => "application/rtf",
        Some("html") => "text/html",
        Some("md" | "txt") => "text/plain",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}
