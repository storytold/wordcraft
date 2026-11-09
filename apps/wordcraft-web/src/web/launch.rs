//! Documents the operating system hands the installed web app ("Open with WordCraft"): the
//! browser passes them to `window.launchQueue`'s consumer as file handles, declared by
//! `file_handlers` in `manifest.webmanifest`. Each is read into [`crate::launched`], which opens
//! it as a dropped file opens: beside the documents already open, never replacing unsaved work.
//!
//! A handle is only read: Save still asks where to write, since the browser grants no write
//! access with a launch.

use std::cell::RefCell;
use std::rc::Rc;

use crate::launched::{self, Launches, Step, admit, next_step};
use wasm_bindgen::JsCast as _;

/// How often the reader looks whether the app has opened the file it read last.
const DRAIN_POLL_MS: i32 = 50;

/// Register the consumer once, at startup. Browsers without `launchQueue` (Firefox, Safari, or a
/// page that isn't installed) skip this silently: there is nothing to receive.
pub(super) fn listen(launched: launched::Shared, ctx: egui::Context) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let queue = js_sys::Reflect::get(&window, &"launchQueue".into()).map_err(|e| format!("{e:?}"))?;
    if queue.is_undefined() || queue.is_null() {
        return Ok(());
    }
    let set_consumer: js_sys::Function = js_sys::Reflect::get(&queue, &"setConsumer".into())
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        .map_err(|_| "launchQueue.setConsumer is not a function".to_string())?;
    let launches = Rc::new(RefCell::new(Launches::default()));
    let consumer = wasm_bindgen::closure::Closure::<dyn FnMut(wasm_bindgen::JsValue)>::new(move |params: wasm_bindgen::JsValue| {
        let Ok(files) = js_sys::Reflect::get(&params, &"files".into()) else { return };
        // A launch without files (the app icon, a link) has an empty list.
        let Ok(files) = files.dyn_into::<js_sys::Array>() else { return };
        let handles = files.iter().filter_map(|f| f.dyn_into::<web_sys::FileSystemFileHandle>().ok());
        if admit(&mut launches.borrow_mut(), handles) {
            wasm_bindgen_futures::spawn_local(read_all(launches.clone(), launched.clone(), ctx.clone()));
        }
    });
    set_consumer.call1(&queue, consumer.as_ref()).map_err(|e| format!("{e:?}"))?;
    // The browser calls the consumer for every later launch into this window, for the page's life.
    consumer.forget();
    Ok(())
}

/// Read the waiting files one after another, in the order given, until none is left. A file is
/// read only once the app has opened the previous one, so at most one launched file's bytes wait
/// in memory, however many files and launches arrive. The app reports the files that failed and
/// those left out once the last has opened ([`launched::open_next`]).
async fn read_all(launches: Rc<RefCell<Launches<web_sys::FileSystemFileHandle>>>, launched: launched::Shared, ctx: egui::Context) {
    launched.borrow_mut().start();
    loop {
        let next = next_step(&mut launches.borrow_mut(), launched.borrow().is_drained());
        match next {
            Step::Wait => sleep(DRAIN_POLL_MS).await,
            Step::Finish(omitted) => {
                launched.borrow_mut().finish(omitted, launched::MAX_WAITING_FILES);
                ctx.request_repaint();
                return;
            }
            Step::Read(handle) => {
                let name = handle.name();
                match read(&handle).await {
                    Ok(bytes) => launched.borrow_mut().push(name, bytes),
                    Err(error) => {
                        log::warn!("{name}: {error}");
                        launched.borrow_mut().failed(name, error);
                    }
                }
                ctx.request_repaint();
            }
        }
    }
}

/// Wait `ms` milliseconds on the browser's timer; at once without a window.
async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, _reject: js_sys::Function| {
        let scheduled = web_sys::window().is_some_and(|w| w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms).is_ok());
        if !scheduled {
            let _ = resolve.call0(&wasm_bindgen::JsValue::NULL);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// The file's bytes, refusing one over the limit before reading it.
async fn read(handle: &web_sys::FileSystemFileHandle) -> Result<Vec<u8>, String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let file: web_sys::File = wasm_bindgen_futures::JsFuture::from(handle.get_file()).await.map_err(js)?.dyn_into().map_err(js)?;
    if !launched::size_within_limit(file.size()) {
        return Err("the file is larger than 256 MB".into());
    }
    let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await.map_err(js)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
