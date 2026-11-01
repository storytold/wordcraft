//! Documents the operating system hands the installed web app ("Open with WordCraft"): the
//! browser passes them to `window.launchQueue`'s consumer as file handles, declared by
//! `file_handlers` in `manifest.webmanifest`. Their bytes go through the inbox, as dropped files
//! do, so each opens beside the documents already open and none replaces unsaved work.
//!
//! A handle is only read: Save still asks where to write, since the browser grants no write
//! access with a launch.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wordcraft_ui_egui::Inbox;
use wordcraft_ui_egui::i18n;

/// Largest launched file read into memory; wasm32 has 4 GB at most, shared with the app.
const MAX_LAUNCHED_BYTES: f64 = (256u32 << 20) as f64;
/// Most files one launch opens; the operating system can pass a whole folder's selection.
const MAX_LAUNCHED_FILES: u32 = 20;

/// Register the consumer once, at startup. Browsers without `launchQueue` (Firefox, Safari, or a
/// page that isn't installed) skip this silently: there is nothing to receive.
pub(super) fn listen(inbox: Inbox, ctx: egui::Context, messages: Rc<RefCell<Vec<String>>>) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let queue = js_sys::Reflect::get(&window, &"launchQueue".into()).map_err(|e| format!("{e:?}"))?;
    if queue.is_undefined() || queue.is_null() {
        return Ok(());
    }
    let set_consumer: js_sys::Function = js_sys::Reflect::get(&queue, &"setConsumer".into())
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        .map_err(|_| "launchQueue.setConsumer is not a function".to_string())?;
    let consumer = wasm_bindgen::closure::Closure::<dyn FnMut(wasm_bindgen::JsValue)>::new(move |params: wasm_bindgen::JsValue| {
        let Ok(files) = js_sys::Reflect::get(&params, &"files".into()) else { return };
        // A launch without files (the app icon, a link) has an empty list.
        let Ok(files) = files.dyn_into::<js_sys::Array>() else { return };
        if files.length() > MAX_LAUNCHED_FILES {
            log::warn!("launch: {} files given, opening the first {MAX_LAUNCHED_FILES}", files.length());
        }
        let handles: Vec<web_sys::FileSystemFileHandle> =
            files.iter().take(MAX_LAUNCHED_FILES as usize).filter_map(|f| f.dyn_into::<web_sys::FileSystemFileHandle>().ok()).collect();
        if handles.is_empty() {
            return;
        }
        let (inbox, ctx, messages) = (inbox.clone(), ctx.clone(), messages.clone());
        // One after another, in the order given: the documents open in that order, and only one
        // file's bytes are copied at a time.
        wasm_bindgen_futures::spawn_local(async move {
            for handle in handles {
                let name = handle.name();
                match read(&handle).await {
                    Ok(bytes) => inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes)),
                    Err(error) => {
                        log::warn!("{name}: {error}");
                        let msg = i18n::fmt(i18n::t("Couldn't open {path}: {error}"), &[("path", &name), ("error", &error)]);
                        messages.borrow_mut().push(msg);
                    }
                }
                ctx.request_repaint();
            }
        });
    });
    set_consumer.call1(&queue, consumer.as_ref()).map_err(|e| format!("{e:?}"))?;
    // The browser calls the consumer for every later launch into this window, for the page's life.
    consumer.forget();
    Ok(())
}

/// The file's bytes, refusing one over [`MAX_LAUNCHED_BYTES`] before reading it.
async fn read(handle: &web_sys::FileSystemFileHandle) -> Result<Vec<u8>, String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let file: web_sys::File = wasm_bindgen_futures::JsFuture::from(handle.get_file()).await.map_err(js)?.dyn_into().map_err(js)?;
    if file.size() > MAX_LAUNCHED_BYTES {
        return Err("the file is larger than 256 MB".into());
    }
    let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await.map_err(js)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
