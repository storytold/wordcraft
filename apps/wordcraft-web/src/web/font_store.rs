//! Fonts the user added in the browser (#582), kept in IndexedDB (`localStorage` holds about
//! 5 MB, less than one CJK font) so they are there again after a reload. Every failure is logged
//! and skipped: a font that can't be stored still works for the session.

use crate::write_gate::{Event, Verdict, WriteGate, WriteQueue};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{IdbDatabase, IdbRequest, IdbTransactionMode};

const DB_NAME: &str = "wordcraft";
const STORE: &str = "fonts";

fn js(e: JsValue) -> String {
    format!("{e:?}")
}

/// Wait for an IndexedDB request; its result.
async fn done(req: &IdbRequest) -> Result<JsValue, String> {
    let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, reject: js_sys::Function| {
        let ok_req = req.clone();
        let ok = Closure::once_into_js(move || {
            let _ = resolve.call1(&JsValue::NULL, &ok_req.result().unwrap_or(JsValue::UNDEFINED));
        });
        let err = Closure::once_into_js(move || {
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("IndexedDB request failed"));
        });
        req.set_onsuccess(Some(ok.unchecked_ref()));
        req.set_onerror(Some(err.unchecked_ref()));
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.map_err(js)
}

/// The database, with its font store made on first use.
async fn open() -> Result<IdbDatabase, String> {
    let factory = web_sys::window().ok_or("no window")?.indexed_db().map_err(js)?.ok_or("IndexedDB is unavailable")?;
    let req = factory.open_with_u32(DB_NAME, 1).map_err(js)?;
    let upgrade_req = req.clone();
    let upgrade = Closure::<dyn FnMut()>::new(move || {
        if let Ok(db) = upgrade_req.result().and_then(|r| r.dyn_into::<IdbDatabase>())
            && !db.object_store_names().contains(STORE)
            && let Err(e) = db.create_object_store(STORE)
        {
            log::error!("couldn't make the font store: {e:?}");
        }
    });
    req.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let db = done(&req).await;
    req.set_onupgradeneeded(None);
    drop(upgrade);
    db?.dyn_into::<IdbDatabase>().map_err(js)
}

/// A change to the stored fonts.
enum Write {
    Put(String, Vec<u8>),
    Delete(String),
}

thread_local! {
    /// Writes run one at a time, in call order, so a quick add then remove of one font applies in
    /// that order (#1362).
    static QUEUE: RefCell<WriteQueue<Write>> = RefCell::new(WriteQueue::default());
}

/// Run `write` in its own transaction; Ok only once the transaction has completed, so a rollback
/// after the request succeeded (storage full, another tab upgrading the database) is an error (#1362).
async fn write_async(write: &Write) -> Result<(), String> {
    let db = open().await?;
    let tx = db.transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite).map_err(js)?;
    let store = tx.object_store(STORE).map_err(js)?;
    let req = match write {
        Write::Put(name, bytes) => store.put_with_key(&js_sys::Uint8Array::from(bytes.as_slice()), &JsValue::from_str(name)),
        Write::Delete(name) => store.delete(&JsValue::from_str(name)),
    }
    .map_err(js)?;
    let gate = Rc::new(RefCell::new(WriteGate::default()));
    let promise = js_sys::Promise::new(&mut |resolve: js_sys::Function, reject: js_sys::Function| {
        let listen = |event: Event| {
            let (gate, resolve, reject) = (gate.clone(), resolve.clone(), reject.clone());
            Closure::<dyn FnMut()>::new(move || match gate.borrow_mut().on(event) {
                Verdict::Pending => {}
                Verdict::Stored => {
                    let _ = resolve.call0(&JsValue::NULL);
                }
                Verdict::Failed(why) => {
                    let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&why));
                }
            })
        };
        let (req_ok, req_err) = (listen(Event::RequestSuccess), listen(Event::RequestError));
        let (complete, abort, error) = (listen(Event::TxComplete), listen(Event::TxAbort), listen(Event::TxError));
        req.set_onsuccess(Some(req_ok.as_ref().unchecked_ref()));
        req.set_onerror(Some(req_err.as_ref().unchecked_ref()));
        tx.set_oncomplete(Some(complete.as_ref().unchecked_ref()));
        tx.set_onabort(Some(abort.as_ref().unchecked_ref()));
        tx.set_onerror(Some(error.as_ref().unchecked_ref()));
        // The handlers live as long as the transaction does.
        for c in [req_ok, req_err, complete, abort, error] {
            c.forget();
        }
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.map(|_| ()).map_err(js)
}

/// Queue `write`; start the worker if none is running.
fn enqueue(write: Write) {
    let first = QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        q.push(write);
        q.start_next()
    });
    let Some(mut write) = first else { return };
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            let (what, name) = match &write {
                Write::Put(name, _) => ("keep the font", name.clone()),
                Write::Delete(name) => ("forget the stored font", name.clone()),
            };
            if let Err(e) = write_async(&write).await {
                log::error!("couldn't {what} {name}: {e}");
            }
            let next = QUEUE.with(|q| {
                let mut q = q.borrow_mut();
                q.finish();
                q.start_next()
            });
            match next {
                Some(w) => write = w,
                None => break,
            }
        }
    });
}

/// Keep `bytes` (a font file as the user gave it) under `name` (its stored file name).
pub fn store(name: &str, bytes: &[u8]) {
    enqueue(Write::Put(name.to_string(), bytes.to_vec()));
}

/// Forget the font stored under `name`.
pub fn forget(name: &str) {
    enqueue(Write::Delete(name.to_string()));
}

/// The value stored under `key`, read in a transaction of its own.
async fn get_one(db: &IdbDatabase, key: &JsValue) -> Result<JsValue, String> {
    let store = db.transaction_with_str(STORE).map_err(js)?.object_store(STORE).map_err(js)?;
    let req = store.get(key).map_err(js)?;
    done(&req).await
}

/// Add every stored font to the font database (each checked like a new one; one that fails is
/// logged and skipped), then repaint: the app lays out again when fonts change.
pub fn load_all(ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let r: Result<(), String> = async {
            let db = open().await?;
            // Each read gets its own transaction: one that has been waited on may already be
            // finished by the time the next request is made.
            let keys = {
                let store = db.transaction_with_str(STORE).map_err(js)?.object_store(STORE).map_err(js)?;
                js_sys::Array::from(&done(&store.get_all_keys().map_err(js)?).await?)
            };
            // One font at a time: `get_all` would hold every stored font in memory at once (#1333).
            for i in 0..keys.length() {
                let key = keys.get(i);
                let Some(name) = key.as_string() else { continue };
                let value = match get_one(&db, &key).await {
                    Ok(v) => v,
                    Err(e) => {
                        log::warn!("stored font {name} skipped: {e}");
                        continue;
                    }
                };
                let Ok(bytes) = value.dyn_into::<js_sys::Uint8Array>() else { continue };
                if bytes.length() as usize > wordcraft_fonts::MAX_FONT_FILE_BYTES {
                    log::warn!("stored font {name} is too large; skipped");
                    continue;
                }
                if let Err(e) = wordcraft_fonts::FontDb::global().install_font(&name, &bytes.to_vec()) {
                    log::warn!("stored font {name} skipped: {e}");
                }
            }
            Ok(())
        }
        .await;
        if let Err(e) = r {
            log::warn!("no stored fonts: {e}");
        }
        ctx.request_repaint();
    });
}
