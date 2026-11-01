//! Documents the operating system hands the installed web app ("Open with WordCraft"): the
//! browser passes them to `window.launchQueue`'s consumer as file handles, declared by
//! `file_handlers` in `manifest.webmanifest`. Their bytes go through the inbox, as dropped files
//! do, so each opens beside the documents already open and none replaces unsaved work.
//!
//! A handle is only read: Save still asks where to write, since the browser grants no write
//! access with a launch.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wordcraft_ui_egui::Inbox;
use wordcraft_ui_egui::i18n;

/// Largest launched file read into memory; wasm32 has 4 GB at most, shared with the app.
const MAX_LAUNCHED_BYTES: f64 = (256u32 << 20) as f64;
/// Most launched files waiting to be read, over every launch; the operating system can pass a
/// whole folder's selection.
const MAX_WAITING_FILES: usize = 20;
/// How often the reader looks whether the app has opened the file it read last.
const DRAIN_POLL_MS: i32 = 50;
/// Most read failures one report names on the status bar.
const MAX_REPORTED_FAILURES: usize = 3;

/// Launched files not read yet, whether a reader is going through them, and how many files the
/// launches since the last report left out.
#[derive(Default)]
struct Launches {
    waiting: VecDeque<web_sys::FileSystemFileHandle>,
    reading: bool,
    omitted: usize,
}

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
    let launches = Rc::new(RefCell::new(Launches::default()));
    let consumer = wasm_bindgen::closure::Closure::<dyn FnMut(wasm_bindgen::JsValue)>::new(move |params: wasm_bindgen::JsValue| {
        let Ok(files) = js_sys::Reflect::get(&params, &"files".into()) else { return };
        // A launch without files (the app icon, a link) has an empty list.
        let Ok(files) = files.dyn_into::<js_sys::Array>() else { return };
        let handles = files.iter().filter_map(|f| f.dyn_into::<web_sys::FileSystemFileHandle>().ok());
        let start = {
            let mut l = launches.borrow_mut();
            for handle in handles {
                if l.waiting.len() < MAX_WAITING_FILES {
                    l.waiting.push_back(handle);
                } else {
                    l.omitted = l.omitted.saturating_add(1);
                }
            }
            let start = !l.reading && (!l.waiting.is_empty() || l.omitted > 0);
            l.reading |= start;
            start
        };
        if start {
            wasm_bindgen_futures::spawn_local(read_all(launches.clone(), inbox.clone(), ctx.clone(), messages.clone()));
        }
    });
    set_consumer.call1(&queue, consumer.as_ref()).map_err(|e| format!("{e:?}"))?;
    // The browser calls the consumer for every later launch into this window, for the page's life.
    consumer.forget();
    Ok(())
}

/// Read the waiting files one after another, in the order given, until none is left. A file is
/// read only once the app has taken everything out of the inbox, so at most one launched file's
/// bytes wait in memory, however many files and launches arrive. The files left out are reported
/// once the last one has opened: opening a document clears the status bar. Files that couldn't be
/// read are reported then too, for the same reason: a failure shown earlier would be replaced as
/// soon as the next file opened.
async fn read_all(launches: Rc<RefCell<Launches>>, inbox: Inbox, ctx: egui::Context, messages: Rc<RefCell<Vec<String>>>) {
    let mut failed: Vec<String> = Vec::new();
    loop {
        let next = launches.borrow_mut().waiting.pop_front();
        let Some(handle) = next else {
            while !inbox_empty(&inbox) {
                sleep(DRAIN_POLL_MS).await;
            }
            let omitted = {
                let mut l = launches.borrow_mut();
                if !l.waiting.is_empty() {
                    continue;
                }
                l.reading = false;
                std::mem::take(&mut l.omitted)
            };
            let omission = (omitted > 0).then(|| {
                i18n::fmt(
                    i18n::t("{count} more files weren't opened: WordCraft opens up to {limit} at a time."),
                    &[("count", &omitted.to_string()), ("limit", &MAX_WAITING_FILES.to_string())],
                )
            });
            let report: Vec<String> = std::mem::take(&mut failed).into_iter().chain(omission).collect();
            if !report.is_empty() {
                messages.borrow_mut().push(report.join(" "));
                ctx.request_repaint();
            }
            return;
        };
        while !inbox_empty(&inbox) {
            sleep(DRAIN_POLL_MS).await;
        }
        let name = handle.name();
        match read(&handle).await {
            Ok(bytes) => inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes)),
            Err(error) => {
                log::warn!("{name}: {error}");
                // The rest are in the log: a status line holds a few files at most.
                if failed.len() < MAX_REPORTED_FAILURES {
                    failed.push(i18n::fmt(i18n::t("Couldn't open {path}: {error}"), &[("path", &name), ("error", &error)]));
                }
            }
        }
        ctx.request_repaint();
    }
}

fn inbox_empty(inbox: &Inbox) -> bool {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
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
