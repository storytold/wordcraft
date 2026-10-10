//! The browser shell: web `Services`, drag-and-drop, the unsaved-changes guard and the eframe
//! web runner.

use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Inbox, Services, WordApp};

const DOC_EXTS: &[&str] = &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "rtf", "txt", "md", "html", "htm", "tex", "json"];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const RECIPIENT_EXTS: &[&str] = &["csv", "tsv", "txt"];
const CANVAS_ID: &str = "wordcraft_canvas";
const LOADING_ID: &str = "wordcraft_loading";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        // `?host=parent`: a same-origin page framing WordCraft (such as the Nextcloud app) opens
        // files in it and stores what it saves (`host`). It guards leaving the page itself.
        let bridge = if query_param("host").as_deref() == Some("parent") { host::Bridge::new() } else { None };
        let dirty = Rc::new(Cell::new(false));
        if bridge.is_none()
            && let Err(e) = guard_unload(dirty.clone())
        {
            log::error!("no unsaved-changes guard: {e}");
        }
        let mut options = eframe::WebOptions::default();
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let app_bridge = bridge.clone();
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!("wordcraft-web: wgpu backend {:?}", rs.adapter.get_info().backend);
                    }
                    let inbox: Inbox = Inbox::default();
                    if let Err(e) = listen_for_pasted_pictures(inbox.clone(), cc.egui_ctx.clone()) {
                        log::error!("pasting pictures is unavailable: {e}");
                    }
                    let doc = if query().contains("sample") { wordcraft_engine::sample::sample_document() } else { wordcraft_doc::Document::new() };
                    let mut services = services(inbox.clone(), cc.egui_ctx.clone(), dirty);
                    if let Some(bridge) = &app_bridge {
                        bridge.connect(&mut services, inbox.clone(), cc.egui_ctx.clone());
                    }
                    let mut app = WordApp::new(Session::new(doc), services);
                    app.autosave = false;
                    if let Some(bridge) = &app_bridge {
                        // `?author=`: the host's name for the user signs comments and tracked
                        // changes (the browser keeps no File › Options name between visits).
                        if let Some(author) = query_param("author") {
                            let _ = app.session.run("file.setAuthor", &serde_json::json!({ "name": author }));
                        }
                        bridge.post_ready();
                    }
                    Ok(Box::new(WebShell { app, inbox }))
                }),
            )
            .await;
        if let (Err(e), Some(bridge)) = (&result, &bridge) {
            bridge.post_failed(&format!("{e:?}"));
        }
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

fn query_param(key: &str) -> Option<String> {
    web_sys::UrlSearchParams::new_with_str(&query()).ok()?.get(key)
}

/// Wraps the app to read dropped files asynchronously (browsers can't read them synchronously)
/// and feed them through the inbox.
struct WebShell {
    app: WordApp,
    inbox: Inbox,
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
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

/// Largest picture pasted from the browser's clipboard (as `insert.picture` allows).
const MAX_PASTED_BYTES: f64 = (200u32 << 20) as f64;
/// Most pictures one paste inserts.
const MAX_PASTED_PICTURES: u32 = 20;

/// Pasting a picture (#45): eframe's paste handler only reads text, so a clipboard holding just
/// pictures (a screenshot, a browser's Copy Image) is read here and the pictures arrive through
/// the inbox like dropped ones. A clipboard with text is left to eframe.
fn listen_for_pasted_pictures(inbox: Inbox, ctx: egui::Context) -> Result<(), String> {
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let on_paste = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |e: web_sys::ClipboardEvent| {
        let Some(data) = e.clipboard_data() else { return };
        if !data.get_data("text").unwrap_or_default().is_empty() {
            return;
        }
        let Some(files) = data.files() else { return };
        for i in 0..files.length().min(MAX_PASTED_PICTURES) {
            let Some(file) = files.get(i) else { continue };
            if !file.type_().starts_with("image/") {
                continue;
            }
            if file.size() > MAX_PASTED_BYTES {
                log::warn!("pasted picture is larger than 200 MB; not inserted");
                continue;
            }
            let inbox = inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(buf) => {
                        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                        // The inbox inserts files named like pictures; the engine reads the format from the bytes.
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push(("pasted.png".into(), bytes));
                        ctx.request_repaint();
                    }
                    Err(e) => log::error!("couldn't read the pasted picture: {e:?}"),
                }
            });
        }
    });
    document.add_event_listener_with_callback("paste", on_paste.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    // The listener lives as long as the page.
    on_paste.forget();
    Ok(())
}

/// `dirty` is the flag the `beforeunload` guard reads ([`guard_unload`]).
fn services(inbox: Inbox, ctx: egui::Context, dirty: Rc<Cell<bool>>) -> Services {
    let open_inbox = inbox.clone();
    Services {
        open_async: Some(Box::new(move |purpose: &str| {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            let dialog = if purpose == "picture" {
                rfd::AsyncFileDialog::new().add_filter("Pictures", IMAGE_EXTS)
            } else if purpose == "recipients" {
                rfd::AsyncFileDialog::new().add_filter("Recipient lists", RECIPIENT_EXTS)
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
        download: Some(Box::new(download)),
        print: Some(Box::new(print_pdf)),
        inbox: Some(inbox),
        on_dirty: Some(Box::new(move |d| dirty.set(d))),
        ..Default::default()
    }
}

/// Closing, reloading or leaving the tab with unsaved changes asks first. Browsers show their own
/// confirmation (a page can only ask for it), so there is no Save button: Cancel, then File › Save.
fn guard_unload(dirty: Rc<Cell<bool>>) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let on_unload = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(move |e: web_sys::BeforeUnloadEvent| {
        if dirty.get() {
            e.prevent_default();
            // Older browsers ask only when `returnValue` is set.
            e.set_return_value("unsaved changes");
        }
    });
    window.add_event_listener_with_callback("beforeunload", on_unload.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    // The listener lives as long as the page.
    on_unload.forget();
    Ok(())
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

/// The hidden iframe the last print loaded its PDF into.
const PRINT_FRAME_ID: &str = "wordcraft-print-frame";

/// Opens `bytes` (a PDF) as an object URL in a hidden iframe and calls the
/// iframe's own `print()` once it has finished loading — this is the
/// standard way to drive the browser's native print dialog on a PDF without
/// a download or a popup window the browser might block. The previous
/// print's iframe and object URL are removed first, so only one copy of the
/// document's PDF stays in the page (the last one may still be printing).
fn print_pdf(bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    if let Some(old) = document.get_element_by_id(PRINT_FRAME_ID) {
        if let Ok(frame) = old.clone().dyn_into::<web_sys::HtmlIFrameElement>() {
            let src = frame.src();
            if src.starts_with("blob:") {
                web_sys::Url::revoke_object_url(&src).ok();
            }
        }
        old.remove();
    }

    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/pdf");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;

    let iframe: web_sys::HtmlIFrameElement =
        match document.create_element("iframe").map_err(js).and_then(|e| e.dyn_into().map_err(|_| "not an iframe".to_string())) {
            Ok(f) => f,
            Err(e) => {
                web_sys::Url::revoke_object_url(&url).ok();
                return Err(e);
            }
        };
    iframe.set_id(PRINT_FRAME_ID);
    let attached = iframe
        .style()
        .set_property("display", "none")
        .map_err(js)
        .and_then(|()| document.body().ok_or_else(|| "no body".to_string()))
        .and_then(|body| body.append_child(&iframe).map_err(js));
    if let Err(e) = attached {
        iframe.remove();
        web_sys::Url::revoke_object_url(&url).ok();
        return Err(e);
    }

    // `onload` fires once the PDF has actually rendered inside the iframe;
    // printing before that would show a blank page. Set the handler BEFORE
    // `src` so a fast/cached load can't fire before we're listening.
    let iframe_for_load = iframe.clone();
    let on_load = wasm_bindgen::closure::Closure::once_into_js(move || {
        if let Some(w) = iframe_for_load.content_window() {
            // Ignore errors: some browsers (notably Firefox with a built-in
            // PDF viewer) print fine but report a benign cross-origin-style
            // error back on this call.
            let _ = w.print();
        }
    });
    iframe.set_onload(Some(on_load.unchecked_ref()));
    iframe.set_src(&url);
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
        Some("tex") => "application/x-tex",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// The `postMessage` bridge to a same-origin parent page (`?host=parent`). Messages are plain
/// objects with a `type`:
///
/// - `wordcraft:ready` (to the parent): the app is listening; send a document now.
/// - `wordcraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
///   `Uint8Array`. It opens like a file picked with File › Open.
/// - `wordcraft:save` (to the parent): `{ name, bytes }` (a transferred `ArrayBuffer`) for Save,
///   Save As and Export. `name` is the document's file name for Save, or the path the parent
///   picked for Save As and Export. The parent stores the file and reports failures itself.
/// - `wordcraft:pick-save` (to the parent): `{ id, name }` when Save As or Export asks where to
///   save, with a suggested file name.
/// - `wordcraft:picked` (from the parent): `{ id, path }` answers it; a missing or empty `path`
///   cancels.
/// - `wordcraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
/// - `wordcraft:failed` (to the parent): `{ error }` when the app couldn't start.
///
/// Only the parent window on the page's own origin is heard or answered.
mod host {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::mpsc::{Receiver, Sender, channel};

    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
    use wordcraft_ui_egui::file_dialogs::FileDialogRequest;
    use wordcraft_ui_egui::{Inbox, Services};

    #[derive(Clone)]
    pub struct Bridge {
        window: web_sys::Window,
        parent: web_sys::Window,
        origin: String,
        /// Save locations asked of the parent and not answered yet, by request id.
        pending: Rc<RefCell<HashMap<u32, Sender<Option<String>>>>>,
        next_id: Rc<Cell<u32>>,
    }

    impl Bridge {
        /// `None` when the page isn't framed (there is no parent to talk to).
        pub fn new() -> Option<Self> {
            let window = web_sys::window()?;
            let parent = window.parent().ok().flatten()?;
            if js_sys::Object::is(&parent, &window) {
                return None;
            }
            let origin = window.location().origin().ok()?;
            Some(Self { window, parent, origin, pending: Rc::default(), next_id: Rc::default() })
        }

        /// Route Save, Export, save locations and the unsaved-changes state to the parent, and
        /// hear documents and answers from it.
        pub fn connect(&self, services: &mut Services, inbox: Inbox, ctx: egui::Context) {
            self.listen(inbox, ctx);
            let bridge = self.clone();
            services.download = Some(Box::new(move |name: &str, bytes: &[u8]| bridge.post_save(name, bytes)));
            let bridge = self.clone();
            services.file_dialog = Some(Box::new(move |request: FileDialogRequest| bridge.ask(request)));
            let bridge = self.clone();
            let reported = Cell::new(None);
            services.on_dirty = Some(Box::new(move |dirty: bool| {
                if reported.get() != Some(dirty) {
                    reported.set(Some(dirty));
                    bridge.send(&[("type", "wordcraft:dirty".into()), ("dirty", dirty.into())]);
                }
            }));
        }

        pub fn post_ready(&self) {
            self.send(&[("type", "wordcraft:ready".into()), ("version", env!("CARGO_PKG_VERSION").into())]);
        }

        pub fn post_failed(&self, error: &str) {
            self.send(&[("type", "wordcraft:failed".into()), ("error", error.into())]);
        }

        fn post_save(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
            let buffer = js_sys::Uint8Array::from(bytes).buffer();
            let message = message(&[("type", "wordcraft:save".into()), ("name", name.into()), ("bytes", buffer.clone().into())])
                .ok_or_else(|| "couldn't build the message".to_string())?;
            self.parent
                .post_message_with_transfer(&message, &self.origin, &js_sys::Array::of1(&buffer))
                .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))
        }

        /// Save As and Export ask the parent where to save; the answer arrives as
        /// `wordcraft:picked`. Opening uses the browser's file picker (`Services::open_async`),
        /// so other requests are cancelled at once.
        fn ask(&self, request: FileDialogRequest) -> Receiver<Option<String>> {
            let (tx, rx) = channel();
            if let FileDialogRequest::Save { name } = request {
                let id = self.next_id.get().wrapping_add(1);
                self.next_id.set(id);
                if self.send(&[("type", "wordcraft:pick-save".into()), ("id", id.into()), ("name", name.as_str().into())]) {
                    self.pending.borrow_mut().insert(id, tx);
                }
            }
            rx
        }

        fn listen(&self, inbox: Inbox, ctx: egui::Context) {
            let parent = self.parent.clone();
            let origin = self.origin.clone();
            let pending = self.pending.clone();
            let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                    return;
                }
                let data = event.data();
                match string(&data, "type").as_deref() {
                    Some("wordcraft:open") => {
                        let name = string(&data, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Document.docx".to_string());
                        let Ok(bytes) = js_sys::Reflect::get(&data, &JsValue::from_str("bytes")) else { return };
                        let bytes = if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
                            array.to_vec()
                        } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
                            js_sys::Uint8Array::new(&bytes).to_vec()
                        } else {
                            return;
                        };
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                    }
                    Some("wordcraft:picked") => {
                        let Some(id) = number(&data, "id") else { return };
                        let Some(tx) = pending.borrow_mut().remove(&id) else { return };
                        // The app polls for the answer each frame; a closed app just drops it.
                        let _ = tx.send(string(&data, "path").filter(|p| !p.trim().is_empty()));
                    }
                    _ => return,
                }
                ctx.request_repaint();
            });
            if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
                // The listener lives as long as the page.
                on_message.forget();
            }
        }

        /// Post a message to the parent; false if it couldn't be sent.
        fn send(&self, fields: &[(&str, JsValue)]) -> bool {
            message(fields).is_some_and(|m| self.parent.post_message(&m, &self.origin).is_ok())
        }
    }

    fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
        let m = js_sys::Object::new();
        for (key, value) in fields {
            js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
        }
        Some(m)
    }

    fn string(data: &JsValue, key: &str) -> Option<String> {
        js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_string()
    }

    /// A request id: a whole number that fits in `u32`.
    fn number(data: &JsValue, key: &str) -> Option<u32> {
        let n = js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_f64()?;
        (n.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&n)).then_some(n as u32)
    }
}
