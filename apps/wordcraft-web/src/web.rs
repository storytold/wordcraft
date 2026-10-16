//! The browser shell: web `Services`, drag-and-drop, the unsaved-changes guard and the eframe
//! web runner.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wordcraft_engine::Session;
use wordcraft_ui_egui::async_saves::{AsyncSaves, SaveOutcome};
use wordcraft_ui_egui::{Inbox, Services, WordApp};

const DOC_EXTS: &[&str] =
    &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "hwp", "hwpx", "rtf", "txt", "md", "html", "htm", "tex", "json", "xml"];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "wmf", "emf"];
const RECIPIENT_EXTS: &[&str] = wordcraft_engine::cmd::mailings::RECIPIENT_EXTENSIONS;
mod font_store;
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
        let dirty = Rc::new(Cell::new(false));
        let saves = AsyncSaves::default();
        if let Err(e) = guard_unload(dirty.clone(), saves.clone()) {
            log::error!("no unsaved-changes guard: {e}");
        }
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
                    // Fonts the user added on an earlier visit (#582); the app notices them arrive.
                    font_store::load_all(cc.egui_ctx.clone());
                    let inbox: Inbox = Inbox::default();
                    let pasted: Inbox = Inbox::default();
                    if let Err(e) = listen_for_pasted_pictures(pasted.clone(), cc.egui_ctx.clone()) {
                        log::error!("pasting pictures is unavailable: {e}");
                    }
                    // The browser's locale picks the new document's paper, the units and the
                    // decimal separator (#838).
                    let region = wordcraft_ui_egui::i18n::system_region();
                    let sample = query().contains("sample");
                    let doc = if sample { wordcraft_engine::sample::sample_document() } else { wordcraft_engine::blank_document(&region) };
                    let mut session = Session::new(doc);
                    session.system_region = region;
                    let messages = Rc::new(RefCell::new(Vec::new()));
                    let mut app = WordApp::new(session, services(inbox.clone(), cc.egui_ctx.clone(), dirty, saves.clone(), messages.clone()));
                    // Pasted pictures wait apart from dropped files: the app inserts one only when the page is the edit target (#1349).
                    app.services.pasted = Some(pasted);
                    app.autosave = false;
                    // No document to show: start on File › Home, as the desktop app does (#766).
                    if !sample {
                        app.start_screen();
                    }
                    Ok(Box::new(WebShell { app, inbox, messages }))
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
    messages: Rc<RefCell<Vec<String>>>,
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
        let msgs = std::mem::take(&mut *self.messages.borrow_mut());
        for msg in msgs {
            self.app.status(msg);
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
/// the app's `pasted` queue, which inserts them only when the page is the edit target (#1349). A clipboard with text is left to eframe.
fn listen_for_pasted_pictures(inbox: Inbox, ctx: egui::Context) -> Result<(), String> {
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let on_paste = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(move |e: web_sys::ClipboardEvent| {
        // The target is always eframe's hidden text input while the page has focus; the app decides (#1349).
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
                        // The engine reads the format from the bytes; the name is ignored.
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
fn services(inbox: Inbox, ctx: egui::Context, dirty: Rc<Cell<bool>>, saves: AsyncSaves, messages: Rc<RefCell<Vec<String>>>) -> Services {
    let open_inbox = inbox.clone();
    let save_ctx = ctx.clone();
    let font_ctx = ctx.clone();
    let font_messages = messages.clone();
    Services {
        open_async: Some(Box::new(move |purpose: &str| {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            let dialog = if purpose == "picture" {
                rfd::AsyncFileDialog::new().add_filter("Pictures", IMAGE_EXTS)
            } else if purpose == "font" {
                rfd::AsyncFileDialog::new().add_filter("Fonts", wordcraft_fonts::FONT_FILE_EXTS)
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
        async_saves: Some(saves.clone()),
        download: Some(Box::new(move |path: &str, bytes: &[u8]| download(&saves, &save_ctx, path, bytes))),
        print: Some(Box::new(print_pdf)),
        inbox: Some(inbox),
        on_dirty: Some(Box::new(move |d| dirty.set(d))),
        animations_enabled: Some(animations_enabled()),
        store_font: Some(Box::new(font_store::store)),
        forget_font: Some(Box::new(font_store::forget)),
        load_system_fonts: Some(Box::new(move || {
            load_system_fonts(font_ctx.clone(), font_messages.clone());
        })),
        ..Default::default()
    }
}

/// Most local fonts one Load System Installed Fonts installs (a designer's machine has thousands).
const MAX_SYSTEM_FONTS: usize = 500;
/// Most font bytes one Load System Installed Fonts reads into memory (wasm memory is small).
const MAX_SYSTEM_FONT_BYTES: usize = 256 << 20;
/// Most local fonts looked at, installed or not (already available, unreadable, too big).
const MAX_SYSTEM_FONTS_TRIED: u32 = 4000;

/// Query the browser's Local Font Access API (`window.queryLocalFonts()`) and install the faces
/// into the global font database, up to [`MAX_SYSTEM_FONTS`] faces and [`MAX_SYSTEM_FONT_BYTES`].
fn load_system_fonts(ctx: egui::Context, messages: Rc<RefCell<Vec<String>>>) {
    wasm_bindgen_futures::spawn_local(async move {
        let Some(window) = web_sys::window() else {
            return;
        };
        let query_fn = match js_sys::Reflect::get(&window, &wasm_bindgen::JsValue::from_str("queryLocalFonts")) {
            Ok(val) if val.is_function() => js_sys::Function::from(val),
            _ => {
                messages.borrow_mut().push(wordcraft_ui_egui::i18n::t("Local font access is not supported by your browser.").to_string());
                ctx.request_repaint();
                return;
            }
        };
        let fonts_promise = match query_fn.call0(&window) {
            Ok(p) => js_sys::Promise::from(p),
            Err(e) => {
                log::warn!("queryLocalFonts call failed: {e:?}");
                messages.borrow_mut().push(wordcraft_ui_egui::i18n::t("Local font access was denied.").to_string());
                ctx.request_repaint();
                return;
            }
        };
        let fonts_val = match wasm_bindgen_futures::JsFuture::from(fonts_promise).await {
            Ok(val) => val,
            Err(e) => {
                log::warn!("queryLocalFonts promise rejected: {e:?}");
                messages.borrow_mut().push(wordcraft_ui_egui::i18n::t("Local font access was denied.").to_string());
                ctx.request_repaint();
                return;
            }
        };
        let font_list = js_sys::Array::from(&fonts_val);
        let len = font_list.length();
        let mut loaded = 0usize;
        let mut total_bytes = 0usize;
        for i in 0..len.min(MAX_SYSTEM_FONTS_TRIED) {
            if loaded >= MAX_SYSTEM_FONTS || total_bytes >= MAX_SYSTEM_FONT_BYTES {
                log::warn!("local fonts: stopped after {loaded} fonts ({total_bytes} bytes)");
                break;
            }
            let font_data = font_list.get(i);
            let Ok(blob_fn) = js_sys::Reflect::get(&font_data, &wasm_bindgen::JsValue::from_str("blob")) else {
                continue;
            };
            if !blob_fn.is_function() {
                continue;
            }
            let Ok(blob_val) = js_sys::Function::from(blob_fn).call0(&font_data) else {
                continue;
            };
            let Ok(blob_js) = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::from(blob_val)).await else {
                continue;
            };
            let Ok(blob) = blob_js.dyn_into::<web_sys::Blob>() else {
                continue;
            };
            // Skip a face too big on its own or for what is left of the budget, before reading it.
            let size = blob.size().clamp(0.0, usize::MAX as f64) as usize;
            if size > wordcraft_fonts::MAX_FONT_FILE_BYTES || total_bytes.saturating_add(size) > MAX_SYSTEM_FONT_BYTES {
                continue;
            }
            let Ok(array_buffer) = wasm_bindgen_futures::JsFuture::from(blob.array_buffer()).await else {
                continue;
            };
            let bytes = js_sys::Uint8Array::new(&array_buffer).to_vec();
            if bytes.len() > wordcraft_fonts::MAX_FONT_FILE_BYTES {
                continue;
            }
            total_bytes = total_bytes.saturating_add(bytes.len());
            let ps_name = js_sys::Reflect::get(&font_data, &wasm_bindgen::JsValue::from_str("postscriptName"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| format!("local-font-{i}"));
            let name = format!("{ps_name}.ttf");
            match wordcraft_fonts::FontDb::global().install_font(&name, &bytes) {
                Ok(_) => loaded += 1,
                Err(wordcraft_fonts::UserFontError::AlreadyAvailable(_)) => {}
                Err(e) => log::warn!("local font {name} skipped: {e}"),
            }
        }
        if loaded > 0 {
            let msg = wordcraft_ui_egui::i18n::fmt(wordcraft_ui_egui::i18n::t("Loaded {count} system fonts."), &[("count", &loaded.to_string())]);
            messages.borrow_mut().push(msg);
        } else {
            messages.borrow_mut().push(wordcraft_ui_egui::i18n::t("All system fonts are already available.").to_string());
        }
        ctx.request_repaint();
    });
}

/// Whether the system has animation effects on, as the browser passes it on: off while the
/// `prefers-reduced-motion: reduce` media query matches. `None` when the browser can't say. The
/// query is made once and asked every frame; it follows the system's setting by itself.
fn animations_enabled() -> Box<dyn Fn() -> Option<bool>> {
    let reduce = web_sys::window().and_then(|w| w.match_media("(prefers-reduced-motion: reduce)").ok().flatten());
    Box::new(move || reduce.as_ref().map(|query| !query.matches()))
}

/// Closing, reloading or leaving the tab with unsaved changes asks first. Browsers show their own
/// confirmation (a page can only ask for it), so there is no Save button: Cancel, then File › Save.
fn guard_unload(dirty: Rc<Cell<bool>>, saves: AsyncSaves) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let on_unload = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(move |e: web_sys::BeforeUnloadEvent| {
        // A save still in the browser's picker hasn't written anything yet.
        if dirty.get() || saves.pending() > 0 {
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

/// Trigger a browser save dialog or download of `bytes` named after the last component of `path`.
fn download(saves: &AsyncSaves, ctx: &egui::Context, path: &str, bytes: &[u8]) -> Result<(), String> {
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "document".into());
    let window = web_sys::window().ok_or("no window")?;

    // On browsers supporting the File System Access API (Chromium: Chrome, Edge, etc.),
    // `showSaveFilePicker` lets the user choose the save location and filename natively.
    if let Ok(picker_val) = js_sys::Reflect::get(&window, &wasm_bindgen::JsValue::from_str("showSaveFilePicker"))
        && let Ok(picker_fn) = picker_val.dyn_into::<js_sys::Function>()
    {
        match trigger_save_file_picker(saves, ctx, &window, &picker_fn, &name, bytes) {
            Ok(()) => return Ok(()),
            Err(e) => {
                log::warn!("showSaveFilePicker failed: {e}; falling back to anchor download");
            }
        }
    }

    // Fallback for browsers without showSaveFilePicker (e.g. Firefox, Safari):
    trigger_anchor_download(&window, &name, bytes)
}

fn trigger_save_file_picker(
    saves: &AsyncSaves,
    ctx: &egui::Context,
    window: &web_sys::Window,
    picker_fn: &js_sys::Function,
    name: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let ext = std::path::Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("docx").to_ascii_lowercase();
    let dot_ext = format!(".{ext}");
    let mime = mime_for(name);

    let accept_obj = js_sys::Object::new();
    let exts_arr = js_sys::Array::new();
    exts_arr.push(&wasm_bindgen::JsValue::from_str(&dot_ext));
    let _ = js_sys::Reflect::set(&accept_obj, &wasm_bindgen::JsValue::from_str(mime), &exts_arr);

    // Our own neutral description: the format's name, as its extension says it.
    let type_desc = match ext.as_str() {
        "htm" => "HTML".to_string(),
        "txt" => "Text".to_string(),
        e if !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()) => e.to_ascii_uppercase(),
        _ => "Document".to_string(),
    };

    let type_obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&type_obj, &wasm_bindgen::JsValue::from_str("description"), &wasm_bindgen::JsValue::from_str(&type_desc));
    let _ = js_sys::Reflect::set(&type_obj, &wasm_bindgen::JsValue::from_str("accept"), &accept_obj);

    let types_arr = js_sys::Array::new();
    types_arr.push(&type_obj);

    let picker_opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&picker_opts, &wasm_bindgen::JsValue::from_str("suggestedName"), &wasm_bindgen::JsValue::from_str(name));
    let _ = js_sys::Reflect::set(&picker_opts, &wasm_bindgen::JsValue::from_str("types"), &types_arr);

    // Call showSaveFilePicker synchronously while transient user activation is active:
    let prom_val = picker_fn.call1(window, &picker_opts).map_err(js)?;
    let prom: js_sys::Promise = prom_val.dyn_into().map_err(|_| "not a promise".to_string())?;

    let bytes_vec = bytes.to_vec();
    let saves = saves.clone();
    let ctx = ctx.clone();
    let name = name.to_string();
    // The app holds back a Save Changes prompt's New/Open until this ticket ends (#1315).
    let ticket = saves.start();
    wasm_bindgen_futures::spawn_local(async move {
        // The document counts as saved only once the file is written and closed.
        let outcome = match wasm_bindgen_futures::JsFuture::from(prom).await {
            Ok(handle_val) => match write_file_handle(&handle_val, &bytes_vec).await {
                Ok(()) => {
                    log::info!("saved {name} through the save picker");
                    SaveOutcome::Saved
                }
                Err(e) => {
                    log::error!("couldn't write {name}: {e}");
                    SaveOutcome::Failed(e)
                }
            },
            Err(e) => {
                let err_name =
                    js_sys::Reflect::get(&e, &wasm_bindgen::JsValue::from_str("name")).ok().and_then(|v| v.as_string()).unwrap_or_default();
                if err_name == "AbortError" {
                    log::debug!("save picker cancelled");
                    SaveOutcome::Cancelled
                } else {
                    log::error!("showSaveFilePicker error: {e:?}");
                    SaveOutcome::Failed(format!("{e:?}"))
                }
            }
        };
        saves.finish(ticket, outcome);
        ctx.request_repaint();
    });
    Ok(())
}

async fn write_file_handle(handle: &wasm_bindgen::JsValue, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let create_fn: js_sys::Function = js_sys::Reflect::get(handle, &wasm_bindgen::JsValue::from_str("createWritable"))
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "createWritable is not a function".to_string())?;
    let stream_prom: js_sys::Promise = create_fn.call0(handle).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    let stream = wasm_bindgen_futures::JsFuture::from(stream_prom).await.map_err(js)?;

    let write_fn: js_sys::Function = js_sys::Reflect::get(&stream, &wasm_bindgen::JsValue::from_str("write"))
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "write is not a function".to_string())?;
    let uint8 = js_sys::Uint8Array::from(bytes);
    let write_prom: js_sys::Promise = write_fn.call1(&stream, &uint8).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    wasm_bindgen_futures::JsFuture::from(write_prom).await.map_err(js)?;

    let close_fn: js_sys::Function = js_sys::Reflect::get(&stream, &wasm_bindgen::JsValue::from_str("close"))
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "close is not a function".to_string())?;
    let close_prom: js_sys::Promise = close_fn.call0(&stream).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    wasm_bindgen_futures::JsFuture::from(close_prom).await.map_err(js)?;
    Ok(())
}

fn trigger_anchor_download(window: &web_sys::Window, name: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor".to_string())?;
    a.set_href(&url);
    a.set_download(name);
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
        Some("xml") => "application/xml",
        _ => "application/octet-stream",
    }
}
