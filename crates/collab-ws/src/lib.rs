//! WebSocket transport for the collaboration client.
//!
//! [`spawn`] starts a task that owns the socket and a [`Client`] state
//! machine: the app sends intents, the task assigns `opId`s, sends them, and
//! forwards the server's answers as [`TransportEvent`]s. On a disconnect the
//! task reconnects with a backoff, sends `auth` with its last version, and
//! resends the operations the server never confirmed (ADR-0003).
//!
//! The task never logs the token and never puts it in the URL (ADR-0008).

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};
use tokio_tungstenite::tungstenite::Message;

use wordcraft_collab::wire::{ClientMsg, OpKind, ServerMsg, Target};
pub use wordcraft_collab_client::Event;
use wordcraft_collab_client::{Client, State};

/// What the app asks the task to send.
#[derive(Clone, Debug, PartialEq)]
pub struct Intent {
    pub kind: OpKind,
    pub target: Target,
    pub payload: serde_json::Value,
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
    pub events: mpsc::UnboundedReceiver<TransportEvent>,
}

impl Handle {
    /// Ask for an operation to be sent. The task assigns its id and base
    /// version, so the caller cannot reuse stale ids.
    pub fn submit(&self, kind: OpKind, target: Target, payload: serde_json::Value) -> Result<(), String> {
        self.intents.send(Intent { kind, target, payload }).map_err(|_| "the session task is gone".to_string())
    }
}

/// Start a session against `url`. Returns immediately; connection progress
/// arrives as events.
pub fn spawn(url: impl Into<String>, token: impl Into<String>, client_id: impl Into<String>) -> Handle {
    let (intents_tx, intents_rx) = mpsc::unbounded_channel();
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let (url, token, client_id) = (url.into(), token.into(), client_id.into());
    tokio::spawn(run(url, token, client_id, intents_rx, events_tx));
    Handle { intents: intents_tx, events: events_rx }
}

async fn run(
    url: String,
    token: String,
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
    let mut waiting: VecDeque<Intent> = VecDeque::new();

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
                    while let Some(intent) = waiting.pop_front() {
                        let op = client.op(intent.kind, intent.target, intent.payload);
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
                if client.state() != State::Joined {
                    // Not in sync yet: the intent waits for the welcome.
                    waiting.push_back(intent);
                    continue;
                }
                let op = client.op(intent.kind, intent.target, intent.payload);
                if send(&mut sink, &ClientMsg::Op { op }).await.is_err() {
                    let _ = events.send(TransportEvent::Disconnected { reason: "send failed".into() });
                    return true;
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

async fn send(
    sink: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, Message>,
    msg: &ClientMsg,
) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}
