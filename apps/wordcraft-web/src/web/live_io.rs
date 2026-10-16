//! craft-live mode in the browser: carries out [`wordcraft_web::live::Live`]'s actions with web APIs (a
//! WebSocket with binary frames, `fetch`, `postMessage` to the shell) and turns what happens
//! into its events. Everything about the protocol is in `live.rs`; this only moves bytes.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value as Json;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use wordcraft_ui_egui::WordApp;

use wordcraft_web::live::{Action, Event};

/// Events waiting for the next frame, and who to wake when one arrives.
#[derive(Clone, Default)]
pub struct Events {
    queue: Rc<RefCell<Vec<Event>>>,
    ctx: Rc<RefCell<Option<egui::Context>>>,
}

impl Events {
    pub fn push(&self, e: Event) {
        self.queue.borrow_mut().push(e);
        if let Some(ctx) = self.ctx.borrow().as_ref() {
            ctx.request_repaint();
        }
    }
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }
    pub fn set_context(&self, ctx: egui::Context) {
        *self.ctx.borrow_mut() = Some(ctx);
    }
}

/// The room's WebSocket and its handlers (kept alive as long as the socket is ours).
struct Socket {
    ws: web_sys::WebSocket,
    _open: Closure<dyn FnMut()>,
    _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

impl Socket {
    /// Closes it without telling anyone (no `Closed` event: we know).
    fn detach(self, code: u16, reason: &str) {
        self.ws.set_onopen(None);
        self.ws.set_onmessage(None);
        self.ws.set_onclose(None);
        if self.ws.close_with_code_and_reason(code, reason).is_err() {
            let _ = self.ws.close();
        }
    }
}

pub struct LiveIo {
    pub events: Events,
    socket: Option<Socket>,
}

impl LiveIo {
    pub fn new(events: Events) -> LiveIo {
        LiveIo { events, socket: None }
    }

    /// Carries out `actions`, in order.
    pub fn run(&mut self, actions: Vec<Action>, app: &mut WordApp) {
        for a in actions {
            match a {
                Action::Connect(url) => self.connect(&url),
                Action::Send(bytes) => {
                    if let Some(s) = &self.socket
                        && s.ws.ready_state() == web_sys::WebSocket::OPEN
                        && let Err(e) = s.ws.send_with_u8_array(&bytes)
                    {
                        log::warn!("craft-live: send: {e:?}");
                    }
                }
                Action::Disconnect(code, reason) => {
                    if let Some(s) = self.socket.take() {
                        s.detach(code, &reason);
                    }
                }
                Action::Shell(msg) => post_to_shell(&msg),
                Action::Fetch { url, token } => {
                    let events = self.events.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let r = match http("GET", &url, &token, &[], None).await {
                            Ok((200, body)) => Ok(body),
                            Ok((status, body)) => Err(error_text(status, &body)),
                            Err(e) => Err(e),
                        };
                        events.push(Event::File(r));
                    });
                }
                Action::Upload { save, url, token, name, bytes } => {
                    let events = self.events.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let r = http("PUT", &url, &token, &[("X-Craft-File-Name", &name)], Some(bytes)).await;
                        let result = r.map(|(status, body)| (status, String::from_utf8_lossy(&body).into_owned()));
                        events.push(Event::Uploaded { save, result });
                    });
                }
                Action::Prepare { url, token } => {
                    let events = self.events.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        let r = match http("POST", &url, &token, &[], None).await {
                            Ok((200, body)) => serde_json::from_slice::<Json>(&body).map_err(|e| e.to_string()),
                            Ok((status, body)) => Err(error_text(status, &body)),
                            Err(e) => Err(e),
                        };
                        events.push(Event::Prepared(r));
                    });
                }
                Action::Status(text) => app.status(text),
                Action::Language(code) => {
                    let _ = app.run("ui.language", serde_json::json!({"value": code}));
                }
            }
        }
    }

    fn connect(&mut self, url: &str) {
        if let Some(old) = self.socket.take() {
            old.detach(1000, "replaced");
        }
        let ws = match web_sys::WebSocket::new(url) {
            Ok(ws) => ws,
            Err(e) => {
                log::error!("craft-live: can't open the WebSocket: {e:?}");
                self.events.push(Event::Closed { code: 1006, reason: format!("{e:?}") });
                return;
            }
        };
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let ev = self.events.clone();
        let open = Closure::<dyn FnMut()>::new(move || ev.push(Event::Open));
        let ev = self.events.clone();
        let message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            if let Ok(buf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                ev.push(Event::Frame(js_sys::Uint8Array::new(&buf).to_vec()));
            }
        });
        let ev = self.events.clone();
        let close = Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |e: web_sys::CloseEvent| {
            ev.push(Event::Closed { code: e.code(), reason: e.reason() });
        });
        ws.set_onopen(Some(open.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(message.as_ref().unchecked_ref()));
        ws.set_onclose(Some(close.as_ref().unchecked_ref()));
        self.socket = Some(Socket { ws, _open: open, _message: message, _close: close });
    }
}

/// The server's `{"error": …}`, or the status.
fn error_text(status: u16, body: &[u8]) -> String {
    serde_json::from_slice::<Json>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(Json::as_str).map(str::to_string))
        .map(|e| format!("{e} (HTTP {status})"))
        .unwrap_or_else(|| format!("HTTP {status}"))
}

/// One HTTP request with the session's bearer token: the status and the body.
async fn http(method: &str, url: &str, token: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>) -> Result<(u16, Vec<u8>), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    let h = web_sys::Headers::new().map_err(js)?;
    h.set("Authorization", &format!("Bearer {token}")).map_err(js)?;
    for (k, v) in headers {
        h.set(k, v).map_err(js)?;
    }
    init.set_headers(&h);
    if let Some(b) = body {
        let arr = js_sys::Uint8Array::from(b.as_slice());
        init.set_body(&arr);
    }
    let req = web_sys::Request::new_with_str_and_init(url, &init).map_err(js)?;
    let resp = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&req)).await.map_err(js)?;
    let resp: web_sys::Response = resp.dyn_into().map_err(|_| "not a response".to_string())?;
    let status = resp.status();
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().map_err(js)?).await.map_err(js)?;
    Ok((status, js_sys::Uint8Array::new(&buf).to_vec()))
}

/// Posts `msg` (an object with a `type`) to the shell, same origin only. Nothing when the page
/// isn't framed.
pub fn post_to_shell(msg: &Json) {
    let Some(window) = web_sys::window() else { return };
    let Ok(Some(parent)) = window.parent() else { return };
    if js_sys::Object::is(&parent, &window) {
        return;
    }
    let Ok(origin) = window.location().origin() else { return };
    match js_sys::JSON::parse(&msg.to_string()) {
        Ok(v) => {
            if let Err(e) = parent.post_message(&v, &origin) {
                log::warn!("craft-live: postMessage: {e:?}");
            }
        }
        Err(e) => log::warn!("craft-live: {e:?}"),
    }
}

/// Listens for the shell's `craft-live:*` messages: only from the parent window, only from our
/// own origin.
pub fn listen_to_shell(events: Events) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let origin = window.location().origin().map_err(|e| format!("{e:?}"))?;
    let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
        if e.origin() != origin {
            return;
        }
        let Some(window) = web_sys::window() else { return };
        let Ok(Some(parent)) = window.parent() else { return };
        let from_parent = e.source().is_some_and(|s| js_sys::Object::is(&s, &parent));
        if !from_parent {
            return;
        }
        let Some(text) = js_sys::JSON::stringify(&e.data()).ok().and_then(|t| t.as_string()) else { return };
        if text.len() > 1 << 20 {
            return;
        }
        let Ok(v) = serde_json::from_str::<Json>(&text) else { return };
        if v.get("type").and_then(Json::as_str).is_some_and(|t| t.starts_with("craft-live:")) {
            events.push(Event::Shell(v));
        }
    });
    window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    // The listener lives as long as the page.
    on_message.forget();
    Ok(())
}
