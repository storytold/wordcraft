//! WebSocket transport for the collaboration client.
//!
//! [`spawn`] starts a task that owns the socket and a [`Client`] state
//! machine: the app sends intents, the task assigns `opId`s, sends them, and
//! forwards the server's answers as [`TransportEvent`]s. On a disconnect the
//! task reconnects with a backoff, sends `auth` with its last version, and
//! resends the operations the server never confirmed (ADR-0003).
//!
//! The task never logs the token and never puts it in the URL (ADR-0008).

#![cfg(not(target_arch = "wasm32"))]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};
use tokio_tungstenite::tungstenite::Message;

use wordcraft_collab::wire::{ClientMsg, OpKind, ServerMsg, Target};
pub use wordcraft_collab_client::Event;
use wordcraft_collab_client::{Client, State};

type Sink = futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>;

/// What the frame loop does after one app intent.
enum Flow {
    Continue,
    Reconnect,
}

/// What the app asks the task to send.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    Op {
        kind: OpKind,
        target: Target,
        payload: serde_json::Value,
    },
    /// Undo or redo this client's newest step; the client owns the stacks.
    Undo,
    Redo,
    /// Ephemeral state; dropped rather than queued when the session is not in sync.
    Presence {
        state: serde_json::Value,
    },
    /// Build the file on the server at the version the client knows.
    Commit,
    /// Ask for a snapshot even when the journal tail is available.
    Resync,
    /// Drop the socket; the task reconnects with the token in [`Handle`].
    Reconnect,
}

/// What the app hears back.
#[derive(Clone, Debug, PartialEq)]
pub enum TransportEvent {
    /// A protocol event from the client state machine.
    Event(Event),
    /// The socket dropped; the task is reconnecting.
    Disconnected { reason: String },
}

/// The app's handle to a session.
pub struct Handle {
    intents: mpsc::UnboundedSender<Intent>,
    token: Arc<Mutex<String>>,
    pub events: mpsc::UnboundedReceiver<TransportEvent>,
}

impl Handle {
    /// Ask for an operation to be sent. The task assigns its id and base
    /// version, so the caller cannot reuse stale ids.
    pub fn submit(&self, kind: OpKind, target: Target, payload: serde_json::Value) -> Result<(), String> {
        self.intents.send(Intent::Op { kind, target, payload }).map_err(|_| "the session task is gone".to_string())
    }

    /// Undo this client's newest step (AdR-0010).
    pub fn undo(&self) -> Result<(), String> {
        self.intents.send(Intent::Undo).map_err(|_| "the session task is gone".to_string())
    }

    /// Redo the newest undone step.
    pub fn redo(&self) -> Result<(), String> {
        self.intents.send(Intent::Redo).map_err(|_| "the session task is gone".to_string())
    }

    /// Publish this user's cursor and selection; never journaled, never queued.
    pub fn presence(&self, state: serde_json::Value) -> Result<(), String> {
        self.intents.send(Intent::Presence { state }).map_err(|_| "the session task is gone".to_string())
    }

    /// Ask the server to write the file.
    pub fn commit(&self) -> Result<(), String> {
        self.intents.send(Intent::Commit).map_err(|_| "the session task is gone".to_string())
    }

    /// Fetch a fresh snapshot (a command allocated ids this replica cannot derive).
    pub fn resync(&self) -> Result<(), String> {
        self.intents.send(Intent::Resync).map_err(|_| "the session task is gone".to_string())
    }

    /// Replace the JWT and reconnect. The cabinet pushes a fresh token before
    /// the old one expires; the message waits for the next socket.
    pub fn renew_token(&self, token: impl Into<String>) -> Result<(), String> {
        *self.token.lock().map_err(|_| "the token lock is poisoned".to_string())? = token.into();
        self.intents.send(Intent::Reconnect).map_err(|_| "the session task is gone".to_string())
    }
}

/// Start a session against `url`. Returns immediately; connection progress
/// arrives as events.
pub fn spawn(url: impl Into<String>, token: impl Into<String>, client_id: impl Into<String>) -> Handle {
    let (intents_tx, intents_rx) = mpsc::unbounded_channel();
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let (url, token, client_id) = (url.into(), token.into(), client_id.into());
    let token = Arc::new(Mutex::new(token));
    tokio::spawn(run(url, token.clone(), client_id, intents_rx, events_tx));
    Handle { intents: intents_tx, token, events: events_rx }
}

async fn run(
    url: String,
    token: Arc<Mutex<String>>,
    client_id: String,
    mut intents: mpsc::UnboundedReceiver<Intent>,
    events: mpsc::UnboundedSender<TransportEvent>,
) {
    let mut client = Client::new(client_id);
    let mut backoff = Duration::from_millis(250);
    loop {
        if events.is_closed() {
            return;
        }
        let token = token.lock().map(|token| token.clone()).unwrap_or_default();
        match tokio_tungstenite::connect_async(&url).await {
            Ok((socket, _)) => {
                backoff = Duration::from_millis(250);
                if !session(&mut client, socket, &token, &mut intents, &events).await {
                    return;
                }
                client.close();
            }
            Err(err) => {
                let _ = events.send(TransportEvent::Disconnected { reason: err.to_string() });
                sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        }
    }
}

/// One connection: auth, then pump operations and answers. Returns `false`
/// when the app dropped the handle, `true` to reconnect.
async fn session(
    client: &mut Client,
    socket: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    token: &str,
    intents: &mut mpsc::UnboundedReceiver<Intent>,
    events: &mpsc::UnboundedSender<TransportEvent>,
) -> bool {
    let (mut sink, mut stream) = socket.split();
    let last = if client.version() > 0 { Some(client.version()) } else { None };
    if send(&mut sink, &client.auth(token, last)).await.is_err() {
        return !events.is_closed();
    }

    // The first heartbeat is one period away: an immediate ping would race the
    // handshake and add a control frame before the session is usable.
    let mut heartbeat = interval_at(Instant::now() + Duration::from_secs(15), Duration::from_secs(15));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut resend_after_welcome = false;
    let mut waiting: VecDeque<(OpKind, Target, serde_json::Value)> = VecDeque::new();

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        let _ = events.send(TransportEvent::Disconnected { reason: "socket closed".into() });
                        return !events.is_closed();
                    }
                    Some(Ok(_)) => continue,
                };
                let msg: ServerMsg = match serde_json::from_str(&text) {
                    Ok(msg) => msg,
                    Err(_) => continue,
                };
                let event = client.on(msg);
                let welcome = matches!(event, Event::Welcome { .. });
                let _ = events.send(TransportEvent::Event(event));
                if welcome && !resend_after_welcome {
                    resend_after_welcome = true;
                    for msg in client.unconfirmed() {
                        if send(&mut sink, &msg).await.is_err() { return !events.is_closed() }
                    }
                }
                if welcome {
                    while let Some((kind, target, payload)) = waiting.pop_front() {
                        let op = client.op(kind, target, payload);
                        if send(&mut sink, &ClientMsg::Op { op }).await.is_err() {
                            let _ = events.send(TransportEvent::Disconnected { reason: "send failed".into() });
                            return true;
                        }
                    }
                }
                if client.state() == State::Closed {
                    return !events.is_closed();
                }
            }
            intent = intents.recv() => {
                let Some(intent) = intent else { return false };
                match handle_intent(&mut *client, &mut sink, intent, &mut waiting).await {
                    Flow::Continue => {}
                    Flow::Reconnect => {
                        let _ = events.send(TransportEvent::Disconnected { reason: "send failed".into() });
                        return true;
                    }
                }
            }
            _ = heartbeat.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    let _ = events.send(TransportEvent::Disconnected { reason: "heartbeat failed".into() });
                    return true;
                }
            }
        }
    }
}

/// One intent from the app: queue it before the handshake, send it after, or
/// ask the frame loop to reconnect. Kept out of `session` so the select loop
/// stays readable.
async fn handle_intent(client: &mut Client, sink: &mut Sink, intent: Intent, waiting: &mut VecDeque<(OpKind, Target, serde_json::Value)>) -> Flow {
    let joined = client.state() == State::Joined;
    match intent {
        Intent::Op { kind, target, payload } => {
            if !joined {
                // Not in sync yet: the operation waits for the welcome.
                waiting.push_back((kind, target, payload));
                return Flow::Continue;
            }
            let op = client.op(kind, target, payload);
            if send(sink, &ClientMsg::Op { op }).await.is_err() {
                return Flow::Reconnect;
            }
        }
        Intent::Undo => {
            if joined
                && let Some(op) = client.undo()
                && send(sink, &ClientMsg::Op { op }).await.is_err()
            {
                return Flow::Reconnect;
            }
        }
        Intent::Redo => {
            if joined
                && let Some(op) = client.redo()
                && send(sink, &ClientMsg::Op { op }).await.is_err()
            {
                return Flow::Reconnect;
            }
        }
        Intent::Presence { state } => {
            if joined && send(sink, &ClientMsg::Presence { state }).await.is_err() {
                return Flow::Reconnect;
            }
        }
        Intent::Commit => {
            if joined {
                let op_id = client.next_op_id();
                let commit = ClientMsg::Commit { op_id, expected_version: client.version(), etag: None };
                if send(sink, &commit).await.is_err() {
                    return Flow::Reconnect;
                }
            }
        }
        Intent::Resync => {
            if joined && send(sink, &ClientMsg::Resync { last_version: client.version() }).await.is_err() {
                return Flow::Reconnect;
            }
        }
        Intent::Reconnect => return Flow::Reconnect,
    }
    Flow::Continue
}

async fn send(sink: &mut Sink, msg: &ClientMsg) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}
