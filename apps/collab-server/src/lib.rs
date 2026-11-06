//! Authoritative sync server for WordCraft collaboration.
//!
//! One room per document; clients authenticate with a JWT in the first
//! WebSocket message (ADR-0008), send normalized operations (ADR-0001) and
//! receive the server's authoritative answer. Presence is ephemeral and never
//! journaled (ADR-0006). The file is assembled by the server on commit
//! (ADR-0009).
//!
//! The room logic itself lives in `wordcraft_collab`; this crate is the
//! transport: WebSocket framing, tokens, rooms, presence fan-out, snapshots,
//! metrics and the commit call.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::{Duration, interval, timeout};

use sha2::Digest;
use wordcraft_collab::room::{Role, Room};
use wordcraft_collab::wire::{ClientMsg, Participant, Reject, RejectCode, ServerMsg, Version};
use wordcraft_collab_doc::DocModel;

/// Largest WebSocket frame the server accepts (RFC §9).
pub const MAX_FRAME: usize = 1 << 20;
/// Snapshot chunk size: a large document is sent in several frames.
pub const SNAPSHOT_CHUNK: usize = 512 * 1024;
/// A client that does not authenticate within this window is disconnected.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// Presence is dropped after this without a heartbeat (ADR-0006).
pub const PRESENCE_TTL: Duration = Duration::from_secs(30);

/// Server configuration from the environment.
#[derive(Clone, Debug)]
pub struct Config {
    pub bind: String,
    /// Shared secret for HS256 tokens (local development and tests).
    pub jwt_secret: Option<String>,
    /// PEM public key for ES256 tokens (production, per ADR-0008).
    pub jwt_pem: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let bind = std::env::var("COLLAB_BIND").unwrap_or_else(|_| "127.0.0.1:8087".to_string());
        let jwt_secret = std::env::var("COLLAB_JWT_SECRET").ok();
        let jwt_pem = std::env::var("COLLAB_JWT_PEM").ok();
        if jwt_secret.is_none() && jwt_pem.is_none() {
            return Err("set COLLAB_JWT_SECRET (HS256, local) or COLLAB_JWT_PEM (ES256 public key)".into());
        }
        Ok(Config { bind, jwt_secret, jwt_pem })
    }
}

/// JWT claims the cabinet issues (ADR-0008).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub aud: String,
    pub doc: String,
    pub app: String,
    pub role: String,
    pub sid: String,
    pub iat: u64,
    pub exp: u64,
}

/// One connected client.
struct Peer {
    role: Role,
    out: mpsc::UnboundedSender<ServerMsg>,
}

/// A document room and its connected peers.
struct RoomState {
    room: Room<DocModel>,
    peers: HashMap<String, Peer>,
}

/// Counters for `/metrics`.
#[derive(Default)]
pub struct Metrics {
    connections: AtomicU64,
    ops: AtomicU64,
    rejects: AtomicU64,
    commits: AtomicU64,
    presence: AtomicU64,
}

impl Metrics {
    fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render(&self, rooms: usize) -> String {
        let mut out = String::from("# HELP wordcraft_collab_rooms Open document rooms\n# TYPE wordcraft_collab_rooms gauge\n");
        out.push_str(&format!("wordcraft_collab_rooms {rooms}\n"));
        for (name, value) in [
            ("connections_total", self.connections.load(Ordering::Relaxed)),
            ("ops_applied_total", self.ops.load(Ordering::Relaxed)),
            ("rejects_total", self.rejects.load(Ordering::Relaxed)),
            ("commits_total", self.commits.load(Ordering::Relaxed)),
            ("presence_total", self.presence.load(Ordering::Relaxed)),
        ] {
            out.push_str(&format!("# TYPE wordcraft_collab_{name} counter\nwordcraft_collab_{name} {value}\n"));
        }
        out
    }
}

/// Shared state of the server.
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    rooms: Arc<Mutex<HashMap<String, Arc<Mutex<RoomState>>>>>,
    pub metrics: Arc<Metrics>,
}

impl AppState {
    pub fn new(cfg: Config) -> AppState {
        AppState { cfg: Arc::new(cfg), rooms: Arc::new(Mutex::new(HashMap::new())), metrics: Arc::new(Metrics::default()) }
    }

    fn room_for(&self, doc: &str) -> Arc<Mutex<RoomState>> {
        let mut rooms = self.rooms.lock().unwrap_or_else(|poison| poison.into_inner());
        rooms
            .entry(doc.to_string())
            .or_insert_with(|| {
                Arc::new(Mutex::new(RoomState { room: Room::new(DocModel::new(wordcraft_doc::Document::new())), peers: HashMap::new() }))
            })
            .clone()
    }

    pub fn room_count(&self) -> usize {
        self.rooms.lock().unwrap_or_else(|poison| poison.into_inner()).len()
    }
}

/// The HTTP application: health, metrics and the WebSocket endpoint.
pub fn app(state: AppState) -> Router {
    Router::new().route("/healthz", get(healthz)).route("/metrics", get(metrics)).route("/ws", get(ws_upgrade)).with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn metrics(State(state): State<AppState>) -> String {
    state.metrics.render(state.room_count())
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.max_message_size(MAX_FRAME).on_upgrade(move |socket| handle_socket(socket, state))
}

/// Verify a token and return its claims.
pub fn verify_token(cfg: &Config, token: &str) -> Result<Claims, Reject> {
    let invalid = |message: &str| Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Forbidden, message);
    let mut validation = Validation::new(if cfg.jwt_pem.is_some() { Algorithm::ES256 } else { Algorithm::HS256 });
    validation.set_audience(&["collab", "wordcraft"]);
    validation.set_required_spec_claims(&["exp", "aud", "sub"]);

    let key = match (&cfg.jwt_pem, &cfg.jwt_secret) {
        (Some(pem), _) => DecodingKey::from_ec_pem(pem.as_bytes()).map_err(|_| invalid("bad public key"))?,
        (None, Some(secret)) => DecodingKey::from_secret(secret.as_bytes()),
        (None, None) => return Err(invalid("server has no token key")),
    };
    let data = decode::<Claims>(token, &key, &validation).map_err(|_| invalid("token is invalid or expired"))?;
    let claims = data.claims;
    if claims.app != "wordcraft" {
        return Err(invalid("token is for another application"));
    }
    if Role::parse(&claims.role).is_none() {
        return Err(invalid("token role is unknown"));
    }
    Ok(claims)
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    Metrics::inc(&state.metrics.connections);
    let (mut sink, mut stream) = socket.split();

    let Some((client_id, claims, role, wants_version)) = authenticate(&mut sink, &mut stream, &state).await else {
        return;
    };
    let room = state.room_for(&claims.doc);
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();
    let (plan, peers) = register(&room, &client_id, role, &tx, wants_version);
    if !send_join(&mut sink, &claims, role, &plan, peers).await {
        leave(&state, &room, &client_id);
        return;
    }

    let mut heartbeat = interval(PRESENCE_TTL / 3);
    heartbeat.tick().await; // the first tick is immediate; the first frame stays an answer
    loop {
        tokio::select! {
            incoming = stream.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                if !process_frame(&state, &room, &mut sink, &claims, role, &client_id, &text).await {
                    break;
                }
            }
            outgoing = rx.recv() => {
                let Some(msg) = outgoing else { break };
                if send(&mut sink, &msg).await.is_err() {
                    break;
                }
            }
            _ = heartbeat.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    leave(&state, &room, &client_id);
}

async fn authenticate(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &AppState,
) -> Option<(String, Claims, Role, Option<Version>)> {
    let first = match timeout(AUTH_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => text,
        _ => {
            let _ = sink.close().await;
            return None;
        }
    };
    let (client_id, claims, wants_version) = match parse_auth(&state.cfg, &first) {
        Ok(parts) => parts,
        Err(reject) => {
            let _ = send(sink, &ServerMsg::Rejected { reject }).await;
            let _ = sink.close().await;
            return None;
        }
    };
    let role = Role::parse(&claims.role)?;
    Some((client_id, claims, role, wants_version))
}

fn register(
    room: &Arc<Mutex<RoomState>>,
    client_id: &str,
    role: Role,
    tx: &mpsc::UnboundedSender<ServerMsg>,
    wants_version: Option<Version>,
) -> (wordcraft_collab::room::JoinPlan, Vec<Participant>) {
    let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
    let plan = guard.room.join(wants_version, false);
    guard.peers.insert(client_id.to_string(), Peer { role, out: tx.clone() });
    let peers = guard
        .peers
        .iter()
        .map(|(id, peer)| Participant { client_id: id.clone(), role: peer.role.as_str().to_string(), color: None, presence: None })
        .collect();
    (plan, peers)
}

async fn send_join(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    claims: &Claims,
    role: Role,
    plan: &wordcraft_collab::room::JoinPlan,
    peers: Vec<Participant>,
) -> bool {
    let welcome = ServerMsg::Welcome {
        session_id: claims.sid.clone(),
        role: role.as_str().to_string(),
        version: plan.version,
        epoch: plan.epoch,
        participants: peers,
        resync: plan.snapshot.is_some(),
    };
    if send(sink, &welcome).await.is_err() {
        return false;
    }
    if let Some(snapshot) = plan.snapshot.clone() {
        for msg in snapshot_frames(plan.version, snapshot) {
            if send(sink, &msg).await.is_err() {
                return false;
            }
        }
    } else if let Some(tail) = plan.replay.clone()
        && send(sink, &ServerMsg::Tail { entries: tail }).await.is_err()
    {
        return false;
    }
    send(sink, &ServerMsg::Nodes { nodes: plan.nodes.clone() }).await.is_ok()
}

async fn process_frame(
    state: &AppState,
    room: &Arc<Mutex<RoomState>>,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    claims: &Claims,
    role: Role,
    client_id: &str,
    text: &str,
) -> bool {
    let msg = match parse_client(text) {
        Ok(msg) => msg,
        Err(reject) => return send(sink, &ServerMsg::Rejected { reject }).await.is_ok(),
    };
    match msg {
        ClientMsg::Op { op } => handle_op(state, room, sink, claims, role, client_id, op).await,
        ClientMsg::Presence { state: presence } => {
            Metrics::inc(&state.metrics.presence);
            broadcast(room, client_id, ServerMsg::Presence { client_id: client_id.to_string(), state: presence }, sink).await;
            true
        }
        ClientMsg::Commit { op_id, expected_version, .. } => handle_commit(state, room, sink, role, op_id, expected_version).await,
        ClientMsg::Ping { ts } => send(sink, &ServerMsg::Pong { ts }).await.is_ok(),
        ClientMsg::Auth { .. } => {
            let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "already authenticated");
            send(sink, &ServerMsg::Rejected { reject }).await.is_ok()
        }
    }
}

async fn handle_op(
    state: &AppState,
    room: &Arc<Mutex<RoomState>>,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    claims: &Claims,
    role: Role,
    client_id: &str,
    op: wordcraft_collab::wire::Op,
) -> bool {
    let structural = op.kind.is_structural();
    let result = {
        let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.room.submit(&claims.sub, role, op)
    };
    match result {
        Ok(applied) => {
            Metrics::inc(&state.metrics.ops);
            broadcast(room, client_id, ServerMsg::Applied { applied }, sink).await;
            if structural {
                let nodes = room.lock().unwrap_or_else(|poison| poison.into_inner()).room.nodes();
                broadcast(room, "", ServerMsg::Nodes { nodes }, sink).await;
            }
            true
        }
        Err(reject) => {
            Metrics::inc(&state.metrics.rejects);
            send(sink, &ServerMsg::Rejected { reject }).await.is_ok()
        }
    }
}

async fn handle_commit(
    state: &AppState,
    room: &Arc<Mutex<RoomState>>,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    role: Role,
    op_id: wordcraft_collab::wire::OpId,
    expected_version: Version,
) -> bool {
    if !role.can_commit() {
        let reject = Reject::new(op_id, RejectCode::Forbidden, "only the owner commits");
        return send(sink, &ServerMsg::Rejected { reject }).await.is_ok();
    }
    let version = room.lock().unwrap_or_else(|poison| poison.into_inner()).room.version();
    if expected_version != version {
        let reject = Reject::new(op_id, RejectCode::Conflict, format!("head is version {version}"));
        return send(sink, &ServerMsg::Rejected { reject }).await.is_ok();
    }
    match build_docx(room) {
        Ok((bytes, warnings)) => {
            Metrics::inc(&state.metrics.commits);
            let sha256 = hex(&sha2::Sha256::digest(&bytes));
            send(sink, &ServerMsg::Committed { version, sha256, warnings }).await.is_ok()
        }
        Err(message) => {
            let reject = Reject::new(op_id, RejectCode::Invalid, message);
            send(sink, &ServerMsg::Rejected { reject }).await.is_ok()
        }
    }
}

fn parse_auth(cfg: &Config, text: &str) -> Result<(String, Claims, Option<Version>), Reject> {
    let msg: ClientMsg = serde_json::from_str(text)
        .map_err(|_| Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "the first message must be auth"))?;
    let ClientMsg::Auth { token, client_id, last_version } = msg else {
        return Err(Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Forbidden, "authenticate first"));
    };
    let claims = verify_token(cfg, &token)?;
    if client_id.is_empty() {
        return Err(Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "clientId is required"));
    }
    Ok((client_id, claims, last_version))
}

fn parse_client(text: &str) -> Result<ClientMsg, Reject> {
    serde_json::from_str(text).map_err(|_| Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "malformed message"))
}

async fn send(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, msg: &ServerMsg) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

/// Send to every peer of the room (`broadcast` includes the sender, so the
/// author sees the authoritative version too).
async fn broadcast(room: &Arc<Mutex<RoomState>>, sender: &str, msg: ServerMsg, sink: &mut futures_util::stream::SplitSink<WebSocket, Message>) {
    let targets: Vec<mpsc::UnboundedSender<ServerMsg>> = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.iter().filter(|(id, _)| id.as_str() != sender).map(|(_, peer)| peer.out.clone()).collect()
    };
    for tx in targets {
        let _ = tx.send(msg.clone());
    }
    let _ = send(sink, &msg).await;
}

fn leave(state: &AppState, room: &Arc<Mutex<RoomState>>, client_id: &str) {
    let gone = {
        let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.remove(client_id)
    };
    if gone.is_some() {
        let targets: Vec<mpsc::UnboundedSender<ServerMsg>> = {
            let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.peers.values().map(|peer| peer.out.clone()).collect()
        };
        for tx in targets {
            let _ = tx.send(ServerMsg::Left { client_id: client_id.to_string() });
        }
    }
    let empty = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.is_empty()
    };
    if empty {
        let mut rooms = state.rooms.lock().unwrap_or_else(|poison| poison.into_inner());
        rooms.retain(|_, existing| !Arc::ptr_eq(existing, room));
    }
}

fn snapshot_frames(version: Version, bytes: Vec<u8>) -> Vec<ServerMsg> {
    if bytes.is_empty() {
        return vec![ServerMsg::Snapshot { version, bytes, last: true }];
    }
    let total = bytes.len();
    let mut frames = Vec::new();
    let mut start = 0;
    while start < total {
        let end = (start + SNAPSHOT_CHUNK).min(total);
        frames.push(ServerMsg::Snapshot { version, bytes: bytes[start..end].to_vec(), last: end == total });
        start = end;
    }
    frames
}

fn build_docx(room: &Arc<Mutex<RoomState>>) -> Result<(Vec<u8>, Vec<String>), String> {
    let doc = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.room.model().document().clone()
    };
    let mut warnings = Vec::new();
    if doc.kept.get().is_some() {
        warnings.push("unmodelled paragraph markup is kept only for paragraphs unchanged since open".to_string());
    }
    let bytes = wordcraft_docx::write(&doc).map_err(|err| err.to_string())?;
    Ok((bytes, warnings))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests;
