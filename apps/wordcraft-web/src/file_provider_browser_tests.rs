//! Original synthetic Word document; one page owns both temporary JS exception injections.
//! This fixture drives the real publisher and WordApp logic consumer, without WebRunner/GPU.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp, file_providers::Host};

use super::{API, EVENT, error, install, publish};

wasm_bindgen_test_configure!(run_in_browser);

// Fault injection is confined to this fixture. JS finally restores the original descriptors
// even if the production callback throws; this is a recoverable injected error, not real OOM.
#[wasm_bindgen(inline_js = r#"
export function wordcraftProviderCopyFailureForTest(callback) {
    const descriptor = Object.getOwnPropertyDescriptor(Uint8Array, 'from');
    const original = Uint8Array.from;
    let calls = 0;
    try {
        Object.defineProperty(Uint8Array, 'from', {
            configurable: true, writable: true,
            value() { calls += 1; throw new RangeError('injected WordCraft byte copy failure'); }
        });
        callback();
    } finally {
        if (descriptor) Object.defineProperty(Uint8Array, 'from', descriptor);
        else Reflect.deleteProperty(Uint8Array, 'from');
    }
    return { calls, restored: Uint8Array.from === original };
}
export function wordcraftProviderRequestFailureForTest(callback) {
    const descriptor = Object.getOwnPropertyDescriptor(window, 'dispatchEvent');
    const original = window.dispatchEvent;
    let calls = 0;
    const requests = [];
    try {
        Object.defineProperty(window, 'dispatchEvent', {
            configurable: true, writable: true,
            value(event) {
                if (event.type === 'wordcraft-file-provider' &&
                    (event.detail.type === 'saveDialog' || event.detail.type === 'openRequest')) {
                    calls += 1;
                    requests.push({ type: event.detail.type, requestId: event.detail.requestId });
                    throw new Error('injected WordCraft request dispatch failure');
                }
                return original.call(this, event);
            }
        });
        callback();
    } finally {
        if (descriptor) Object.defineProperty(window, 'dispatchEvent', descriptor);
        else Reflect.deleteProperty(window, 'dispatchEvent');
    }
    return { calls, requests, restored: window.dispatchEvent === original };
}
"#)]
extern "C" {
    #[wasm_bindgen(catch, js_name = wordcraftProviderCopyFailureForTest)]
    fn copy_failure(callback: &Function) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = wordcraftProviderRequestFailureForTest)]
    fn request_failure(callback: &Function) -> Result<JsValue, JsValue>;
}

struct Events {
    window: web_sys::Window,
    listener: Closure<dyn FnMut(web_sys::Event)>,
    details: Rc<RefCell<Vec<JsValue>>>,
    installed: bool,
}

impl Events {
    fn new() -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| error("no test window"))?;
        let details = Rc::new(RefCell::new(Vec::new()));
        let received = details.clone();
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(event) = event.dyn_ref::<web_sys::CustomEvent>() {
                received.borrow_mut().push(event.detail());
            }
        });
        window.add_event_listener_with_callback(EVENT, listener.as_ref().unchecked_ref())?;
        Ok(Self { window, listener, details, installed: false })
    }

    fn matching(&self, kind: &str) -> Result<Vec<JsValue>, JsValue> {
        let mut found = Vec::new();
        for detail in self.details.borrow().iter() {
            if Reflect::get(detail, &"type".into())?.as_string().as_deref() == Some(kind) {
                found.push(detail.clone());
            }
        }
        Ok(found)
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        let _ = self.window.remove_event_listener_with_callback(EVENT, self.listener.as_ref().unchecked_ref());
        if self.installed {
            let _ = Reflect::delete_property(self.window.unchecked_ref::<Object>(), &API.into());
        }
    }
}

fn call(api: &JsValue, method: &str, args: &Array) -> Result<JsValue, JsValue> {
    Reflect::get(api, &method.into())?.dyn_into::<Function>()?.apply(api, args)
}

fn field(value: &JsValue, name: &str) -> Result<String, JsValue> {
    Reflect::get(value, &name.into())?.as_string().ok_or_else(|| error(format!("missing {name}")))
}

fn frame(app: &mut WordApp, ctx: &egui::Context) {
    let _ = ctx.run(egui::RawInput::default(), |ctx| app.logic(ctx));
}

#[wasm_bindgen_test]
fn page_local_provider_delivery_failures_release_owners_and_recover() -> Result<(), JsValue> {
    let mut events = Events::new()?;
    let host = Host::default();
    let ctx = egui::Context::default();
    install(host.clone(), ctx.clone())?;
    events.installed = true;
    let api = Reflect::get(&events.window, &API.into())?;
    let connection = call(&api, "registerProvider", &Array::of3(&"test".into(), &"Test storage".into(), &"account".into()))?;
    assert_eq!(host.snapshot()["connectionId"].as_str(), connection.as_string().as_deref());
    let services = Services { file_providers: Some(host.clone()), ..Default::default() };
    let mut app = WordApp::new(Session::new(Document::new()), services);
    frame(&mut app, &ctx);
    app.run("text.insert", json!({"text":"Original synthetic WordCraft document."})).map_err(error)?;
    assert_eq!(app.run("file.save", json!({})).map_err(error)?["pending"], "fileProvider");
    publish(&host, &ctx);
    let dialogs = events.matching("saveDialog")?;
    assert_eq!(dialogs.len(), 1);
    let failed_request = field(&dialogs[0], "requestId")?;
    call(&api, "resolveSaveDialog", &Array::of3(&failed_request.clone().into(), &"provider".into(), &"Recovery.docx".into()))?;
    frame(&mut app, &ctx);
    let captured_revision = app.session.rev();
    let document = app.session.document_id();
    app.run("text.insert", json!({"text":" Newer text must survive failed delivery."})).map_err(error)?;
    let newer_revision = app.session.rev();
    assert_ne!(captured_revision, newer_revision);
    let pending_host = host.clone();
    let pending_ctx = ctx.clone();
    let publisher = Closure::<dyn FnMut()>::new(move || publish(&pending_host, &pending_ctx));
    let fault = copy_failure(publisher.as_ref().unchecked_ref())?;
    assert_eq!(Reflect::get(&fault, &"calls".into())?.as_f64(), Some(1.0), "the actual catch-enabled byte import must be reached");
    assert_eq!(Reflect::get(&fault, &"restored".into())?.as_bool(), Some(true));
    assert!(events.matching("save")?.is_empty(), "a failed copy must not dispatch a partial save payload");
    assert!(host.pending_for(document), "failure consumption belongs to the next actual WordApp logic frame");
    assert!(call(&api, "saveResult", &Array::of3(&failed_request.into(), &"saved".into(), &"late-resource".into())).is_err());
    frame(&mut app, &ctx);
    assert!(!host.pending_for(document), "the production WordApp consumer must release the failed capture");
    assert!(app.session.dirty);
    assert_eq!(app.session.rev(), newer_revision);
    assert!(app.session.doc.plain_text(StoryRef::Body).contains("Newer text must survive"));
    assert_eq!(host.snapshot()["documents"][0]["provider"], false);
    assert_eq!(host.snapshot()["documents"][0]["dirty"], true);
    assert_eq!(host.snapshot()["documents"][0]["pending"], false);

    // A subsequent real UI save crosses the public window event and decodes as a real DOCX.
    app.run("file.save", json!({})).map_err(error)?;
    publish(&host, &ctx);
    let dialogs = events.matching("saveDialog")?;
    assert_eq!(dialogs.len(), 2);
    let retry = field(&dialogs[1], "requestId")?;
    call(&api, "resolveSaveDialog", &Array::of3(&retry.clone().into(), &"provider".into(), &"Recovery.docx".into()))?;
    frame(&mut app, &ctx);
    let retry_revision = app.session.rev();
    publish(&host, &ctx);
    let saves = events.matching("save")?;
    assert_eq!(saves.len(), 1);
    assert_eq!(field(&saves[0], "requestId")?, retry);
    assert_eq!(field(&saves[0], "name")?, "Recovery.docx");
    assert_eq!(Reflect::get(&saves[0], &"revision".into())?.as_f64(), Some(retry_revision as f64));
    let bytes = Reflect::get(&saves[0], &"bytes".into())?.dyn_into::<Uint8Array>()?.to_vec();
    assert_eq!(Reflect::get(&saves[0], &"bytesLength".into())?.as_f64(), Some(bytes.len() as f64));
    let restored = wordcraft_engine::io::open_bytes("Recovery.docx", &bytes).map_err(error)?;
    assert!(restored.plain_text(StoryRef::Body).contains("Newer text must survive"));
    assert!(app.session.dirty);
    assert!(host.pending_for(document));
    call(&api, "saveResult", &Array::of3(&retry.into(), &"saved".into(), &"retry-resource".into()))?;
    frame(&mut app, &ctx);
    assert!(!app.session.dirty);
    assert_eq!(host.snapshot()["documents"][0]["provider"], true);

    // Metadata-only dispatch failures use the same actual publisher; terminal failures still
    // reach the real window listener, and neither unresolved request can pin the account.
    app.run("file.save", json!({})).map_err(error)?;
    let read = call(&api, "requestOpen", &Array::of1(&"retry-resource".into()))?;
    let fault = request_failure(publisher.as_ref().unchecked_ref())?;
    assert_eq!(Reflect::get(&fault, &"calls".into())?.as_f64(), Some(2.0));
    assert_eq!(Reflect::get(&fault, &"restored".into())?.as_bool(), Some(true));
    let failed_reads = events.matching("openFailed")?;
    assert_eq!(failed_reads.len(), 1);
    assert_eq!(field(&failed_reads[0], "requestId")?, read.as_string().ok_or_else(|| error("missing read ID"))?);
    assert_eq!(field(&failed_reads[0], "resourceId")?, "retry-resource");
    let failed_dialogs = events.matching("saveFailed")?;
    assert_eq!(failed_dialogs.len(), 1);
    let dialog = field(&failed_dialogs[0], "requestId")?;
    let rejected = Reflect::get(&fault, &"requests".into())?.dyn_into::<Array>()?;
    assert_eq!(rejected.length(), 2);
    assert_eq!(field(&rejected.get(0), "type")?, "saveDialog");
    assert_eq!(field(&rejected.get(0), "requestId")?, dialog);
    assert_eq!(field(&rejected.get(1), "type")?, "openRequest");
    assert_eq!(Reflect::get(&rejected.get(1), &"requestId".into())?, read);
    assert!(call(&api, "resolveSaveDialog", &Array::of3(&dialog.into(), &"provider".into(), &"Recovery.docx".into())).is_err());
    assert!(call(&api, "cancelOpen", &Array::of2(&read, &"late".into())).is_err());
    assert!(!host.busy());
    assert_eq!(app.session.document_id(), document);
    assert_eq!(app.session.rev(), retry_revision);

    app.run("file.save", json!({})).map_err(error)?;
    publish(&host, &ctx);
    let dialogs = events.matching("saveDialog")?;
    assert_eq!(dialogs.len(), 3);
    let retry_dialog = field(&dialogs[2], "requestId")?;
    call(&api, "resolveSaveDialog", &Array::of3(&retry_dialog.into(), &"cancel".into(), &"".into()))?;
    let retry_read = call(&api, "requestOpen", &Array::of1(&"retry-resource".into()))?;
    publish(&host, &ctx);
    let reads = events.matching("openRequest")?;
    assert_eq!(reads.len(), 1);
    assert_eq!(Reflect::get(&reads[0], &"requestId".into())?, retry_read);
    call(&api, "cancelOpen", &Array::of2(&retry_read, &"cancelled".into()))?;
    publish(&host, &ctx);
    assert!(!host.busy());
    assert_eq!(call(&api, "unregisterProvider", &Array::new())?.as_bool(), Some(true));
    Ok(())
}
