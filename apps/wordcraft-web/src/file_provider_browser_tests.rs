//! Original synthetic Word document; one page owns both temporary JS exception injections.
//! This fixture drives the real publisher and WordApp logic consumer, without WebRunner/GPU.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::Session;
use wordcraft_ui_egui::{Services, WordApp, file_providers::Host};

use super::{API, EVENT, error, install, publish};

wasm_bindgen_test_configure!(run_in_browser);

// Fault injection stays in Rust. The actual Function::call0 catches a recoverable exception;
// restore the exact own descriptor (or remove our shadow) before propagating that Result.
// Fatal WASM traps, true memory exhaustion and process termination are outside this fixture.
fn with_replacement(target: &JsValue, key: &str, replacement: &JsValue, callback: &Function) -> Result<Result<JsValue, JsValue>, JsValue> {
    let object = Reflect::get(&js_sys::global(), &"Object".into())?;
    let key = JsValue::from_str(key);
    let descriptor = call(&object, "getOwnPropertyDescriptor", &Array::of2(target, &key))?;
    let original = Reflect::get(target, &key)?;
    let outcome = match Reflect::set(target, &key, replacement) {
        Ok(true) => callback.call0(&JsValue::UNDEFINED),
        Ok(false) => Err(error("injected method is not writable")),
        Err(error) => Err(error),
    };
    if descriptor.is_undefined() {
        if !Reflect::delete_property(target.unchecked_ref::<Object>(), &key)? {
            return Err(error("could not remove injected method"));
        }
    } else {
        call(&object, "defineProperty", &Array::of3(target, &key, &descriptor))?;
    }
    let restored = call(&object, "getOwnPropertyDescriptor", &Array::of2(target, &key))?;
    if !Object::is(&Reflect::get(target, &key)?, &original) || restored.is_undefined() != descriptor.is_undefined() {
        return Err(error("injected property was not restored exactly"));
    }
    if !descriptor.is_undefined() {
        for attribute in ["value", "get", "set", "writable", "enumerable", "configurable"] {
            if !Object::is(&Reflect::get(&restored, &attribute.into())?, &Reflect::get(&descriptor, &attribute.into())?) {
                return Err(error("injected property descriptor changed"));
            }
        }
    }
    Ok(outcome)
}

fn fault_receipt(calls: u32, requests: &Array) -> Result<JsValue, JsValue> {
    let receipt = Object::new();
    for (key, value) in
        [("calls", JsValue::from_f64(f64::from(calls))), ("restored", JsValue::from_bool(true)), ("requests", JsValue::from(requests.clone()))]
    {
        if !Reflect::set(&receipt, &key.into(), &value)? {
            return Err(error("could not record restored fault injection"));
        }
    }
    Ok(receipt.into())
}

fn copy_failure(callback: &Function) -> Result<JsValue, JsValue> {
    let calls = Rc::new(Cell::new(0_u32));
    let observed = calls.clone();
    let fault = Closure::<dyn FnMut(JsValue) -> Result<JsValue, JsValue>>::new(move |_| {
        observed.set(observed.get().saturating_add(1));
        Err(js_sys::RangeError::new("injected WordCraft byte copy failure").into())
    });
    let array = Reflect::get(&js_sys::global(), &"Uint8Array".into())?;
    with_replacement(&array, "from", fault.as_ref(), callback)??;
    fault_receipt(calls.get(), &Array::new())
}

fn request_failure(callback: &Function) -> Result<JsValue, JsValue> {
    let window = web_sys::window().ok_or_else(|| error("no test window"))?;
    let original = Reflect::get(&window, &"dispatchEvent".into())?.dyn_into::<Function>()?;
    let receiver: JsValue = window.into();
    let target = receiver.clone();
    let calls = Rc::new(Cell::new(0_u32));
    let observed = calls.clone();
    let requests = Array::new();
    let rejected = requests.clone();
    let fault = Closure::<dyn FnMut(JsValue) -> Result<JsValue, JsValue>>::new(move |event: JsValue| {
        if Reflect::get(&event, &"type".into())?.as_string().as_deref() == Some(EVENT) {
            let detail = Reflect::get(&event, &"detail".into())?;
            let kind = Reflect::get(&detail, &"type".into())?.as_string();
            if matches!(kind.as_deref(), Some("saveDialog" | "openRequest")) {
                observed.set(observed.get().saturating_add(1));
                let request = Object::new();
                for name in ["type", "requestId"] {
                    if !Reflect::set(&request, &name.into(), &Reflect::get(&detail, &name.into())?)? {
                        return Err(error("could not record rejected request"));
                    }
                }
                rejected.push(&request);
                return Err(js_sys::Error::new("injected WordCraft request dispatch failure").into());
            }
        }
        original.call1(&receiver, &event)
    });
    with_replacement(&target, "dispatchEvent", fault.as_ref(), callback)??;
    fault_receipt(calls.get(), &requests)
}

struct Events {
    window: web_sys::Window,
    listener: Closure<dyn FnMut(web_sys::Event)>,
    details: Rc<RefCell<Vec<JsValue>>>,
    installed: Option<JsValue>,
}

impl Events {
    fn new() -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or_else(|| error("no test window"))?;
        if Reflect::has(&window, &API.into())? {
            return Err(error("fixture requires its own blank page API"));
        }
        let details = Rc::new(RefCell::new(Vec::new()));
        let received = details.clone();
        let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            if let Some(event) = event.dyn_ref::<web_sys::CustomEvent>() {
                received.borrow_mut().push(event.detail());
            }
        });
        window.add_event_listener_with_callback(EVENT, listener.as_ref().unchecked_ref())?;
        Ok(Self { window, listener, details, installed: None })
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
        if let Some(owned) = &self.installed
            && let Ok(current) = Reflect::get(&self.window, &API.into())
            && Object::is(&current, owned)
        {
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
    let mut output = ctx.run_pass(egui::RawInput::default(), |ctx| app.logic(ctx));
    // This logic-only fixture has no renderer; explicitly discard its generated texture uploads.
    output.textures_delta.clear();
}

#[wasm_bindgen_test]
fn page_local_provider_delivery_failures_release_owners_and_recover() -> Result<(), JsValue> {
    let mut events = Events::new()?;
    let host = Host::default();
    let ctx = egui::Context::default();
    // Capture our API even if install publishes it and the ready dispatch then fails.
    // Events::new already established the page was blank; Drop removes only this identity.
    let installed = install(host.clone(), ctx.clone());
    let api = Reflect::get(&events.window, &API.into())?;
    events.installed = Some(api.clone());
    installed?;
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
