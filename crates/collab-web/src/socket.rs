//! The browser's `WebSocket`, driven from the UI frame.
//!
//! There are no threads on wasm32: the socket's callbacks push events into a
//! queue and [`Transport::poll`] drains it once per frame. A close schedules a
//! reconnect with the same backoff as the desktop transport (250 ms doubling to
//! 10 s); operations the server never confirmed are resent by `Collab` after
//! the next welcome, so a dropped socket loses nothing.
//!
//! The token is never part of `url`: it goes in the first `auth` frame, exactly
//! as on the desktop (ADR-0008).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use web_sys::{CloseEvent, Event, MessageEvent, WebSocket};

/// What the frame loop hears from the socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// The socket is open: send `auth`.
    Opened,
    /// One text frame from the server.
    Text(String),
    /// The socket closed; a reconnect is scheduled.
    Closed,
}

/// The socket and the messages it produced since the last frame.
pub struct Transport {
    inner: Rc<RefCell<Inner>>,
}

impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transport").field("socket", &"<WebSocket>").finish()
    }
}

struct Inner {
    url: String,
    socket: Option<WebSocket>,
    incoming: VecDeque<Incoming>,
    attempt: u32,
    live: bool,
    closers: Closers,
}

#[derive(Default)]
#[allow(dead_code, reason = "the closures are kept alive by being stored; they are never called from Rust")]
struct Closers {
    open: Option<Closure<dyn FnMut(Event)>>,
    message: Option<Closure<dyn FnMut(MessageEvent)>>,
    close: Option<Closure<dyn FnMut(CloseEvent)>>,
}

impl Transport {
    /// Open `url`; the token is not part of it.
    pub fn open(url: &str) -> Result<Transport, String> {
        let inner = Rc::new(RefCell::new(Inner {
            url: url.to_string(),
            socket: None,
            incoming: VecDeque::new(),
            attempt: 0,
            live: true,
            closers: Closers::default(),
        }));
        dial(&inner)?;
        Ok(Transport { inner })
    }

    /// Drain what arrived since the previous frame.
    pub fn poll(&self) -> Vec<Incoming> {
        let mut inner = self.inner.borrow_mut();
        inner.incoming.drain(..).collect()
    }

    /// Send one text frame; an error means the socket is not open (the caller
    /// keeps the operation for the resend after reconnect).
    pub fn send(&self, text: &str) -> Result<(), String> {
        let inner = self.inner.borrow();
        let Some(socket) = &inner.socket else { return Err("the collaboration socket is not open".into()) };
        if socket.ready_state() != WebSocket::OPEN {
            return Err("the collaboration socket is not open".into());
        }
        socket.send_with_str(text).map_err(|err| format!("cannot send a collaboration frame: {err:?}"))
    }

    /// Drop the current socket and dial again (a fresh token, a manual retry).
    pub fn reconnect(&self) {
        self.inner.borrow_mut().attempt = 0;
        let inner = self.inner.clone();
        if dial(&inner).is_err() {
            inner.borrow_mut().incoming.push_back(Incoming::Closed);
            schedule(&inner);
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.live = false;
        if let Some(socket) = inner.socket.take() {
            let _ = socket.close();
        }
    }
}

/// Make one socket and wire its callbacks.
fn dial(inner: &Rc<RefCell<Inner>>) -> Result<(), String> {
    let url = inner.borrow().url.clone();
    // Drop the previous socket's callbacks first: they point at closures this
    // call replaces.
    if let Some(old) = inner.borrow_mut().socket.take() {
        old.set_onopen(None);
        old.set_onmessage(None);
        old.set_onclose(None);
        let _ = old.close();
    }
    let socket = WebSocket::new(&url).map_err(|err| format!("cannot open a collaboration socket: {err:?}"))?;
    socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
    let weak = Rc::downgrade(inner);

    let on_open = Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
        if let Some(inner) = weak.upgrade() {
            let mut inner = inner.borrow_mut();
            inner.attempt = 0;
            inner.incoming.push_back(Incoming::Opened);
        }
    });
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    let weak = Rc::downgrade(inner);
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(inner) = weak.upgrade() else { return };
        if let Some(text) = event.data().as_string() {
            inner.borrow_mut().incoming.push_back(Incoming::Text(text));
        }
    });
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let weak = Rc::downgrade(inner);
    let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |_event: CloseEvent| {
        let Some(inner) = weak.upgrade() else { return };
        {
            let mut inner = inner.borrow_mut();
            if !inner.live {
                return;
            }
            inner.socket = None;
            inner.incoming.push_back(Incoming::Closed);
        }
        schedule(&inner);
    });
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    let mut inner = inner.borrow_mut();
    inner.socket = Some(socket);
    inner.closers = Closers { open: Some(on_open), message: Some(on_message), close: Some(on_close) };
    Ok(())
}

/// Reconnect after a backoff: 250 ms, doubling, at most 10 s.
fn schedule(inner: &Rc<RefCell<Inner>>) {
    let delay = {
        let mut inner = inner.borrow_mut();
        if !inner.live {
            return;
        }
        let delay = 250u32.saturating_mul(1u32 << inner.attempt.min(6));
        inner.attempt = inner.attempt.saturating_add(1);
        delay.min(10_000)
    };
    let weak = Rc::downgrade(inner);
    let timer = Closure::<dyn FnMut()>::once(move || {
        if let Some(inner) = weak.upgrade()
            && inner.borrow().live
            && let Err(_error) = dial(&inner)
        {
            schedule(&inner);
        }
    });
    if let Some(window) = web_sys::window()
        && window.set_timeout_with_callback_and_timeout_and_arguments_0(timer.as_ref().unchecked_ref(), delay as i32).is_ok()
    {
        // A closure that fires once; keeping it alive for the page would leak
        // one small allocation per reconnect, dropping it would free code that
        // is still running.
        timer.forget();
    }
}
