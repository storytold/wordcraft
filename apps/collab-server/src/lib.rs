//! Authoritative sync server for WordCraft collaboration.
//!
//! One room per document; clients authenticate with a JWT in the first
//! WebSocket message (ADR-0008), send normalized operations (ADR-0001) and
//! receive the server's authoritative answer. Presence is ephemeral and never
//! journaled (ADR-0006). The file is assembled by the server on commit
//! (ADR-0009) and, when `COLLAB_CABINET_URL` names one, posted to the cabinet
//! as its next file version (`cabinet.rs`, M87).
//!
//! The room logic itself lives in `wordcraft_collab`; this crate is the
//! transport: WebSocket framing, tokens, rooms, presence fan-out, snapshots,
//! metrics and the commit call.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

use cabinet::{Cabinet, CabinetConfig, Publication, PublishError, Published};
use sha2::Digest;
use wordcraft_collab::room::{Model, Role, Room};
use wordcraft_collab::wire::{CabinetCommit, ClientMsg, Participant, Reject, RejectCode, ServerMsg, Version};
use wordcraft_collab_engine::SessionModel;

pub mod cabinet;
pub mod store;

use store::{Snapshot, Store};

/// Largest WebSocket frame the server accepts (RFC §9).
pub const MAX_FRAME: usize = 1 << 20;
/// A document the server will hold and commit (matches the reader's budget).
pub const MAX_DOCUMENT: usize = 64 << 20;
/// Snapshot chunk size: a large document is sent in several frames.
pub const SNAPSHOT_CHUNK: usize = 512 * 1024;
/// A client that does not authenticate within this window is disconnected.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// Presence is dropped after this without a heartbeat (ADR-0006).
pub const PRESENCE_TTL: Duration = Duration::from_secs(30);
/// Frames a peer's connection may hold before the peer counts as not reading
/// (ADR WC-0002).
pub const PEER_QUEUE: usize = 512;

/// Server configuration from the environment.
#[derive(Clone, Debug)]
pub struct Config {
    pub bind: String,
    /// Shared secret for HS256 tokens (local development and tests).
    pub jwt_secret: Option<String>,
    /// PEM public key for ES256 tokens (production, per ADR-0008).
    pub jwt_pem: Option<String>,
    /// Where snapshots are written (ADR-0005, ADR-0011).
    pub data_dir: PathBuf,
    /// How long a cabinet session may be silent before another session may take
    /// the lease (ADR-0008). `0` for tests.
    pub lease_ttl_secs: u64,
    /// The cabinet commit callback (M87). `None` keeps the commit server-side
    /// only: the file is built and the host owns the storage.
    pub cabinet: Option<CabinetConfig>,
    /// How often a connected peer is pinged, and the longest any single write
    /// to it may take before the peer counts as gone (ADR WC-0002). Also the
    /// interval at which a peer whose outbox overflowed is closed.
    pub heartbeat: Duration,
    /// Frames one peer's outbox holds before the peer counts as not reading
    /// (ADR WC-0002). 512 in production; tests keep it small so the overflow is
    /// reachable without a stalled megabyte-sized socket buffer.
    pub peer_queue: usize,
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let bind = std::env::var("COLLAB_BIND").unwrap_or_else(|_| "127.0.0.1:8087".to_string());
        let jwt_secret = std::env::var("COLLAB_JWT_SECRET").ok();
        let jwt_pem = std::env::var("COLLAB_JWT_PEM").ok();
        if jwt_secret.is_none() && jwt_pem.is_none() {
            return Err("set COLLAB_JWT_SECRET (HS256, local) or COLLAB_JWT_PEM (ES256 public key)".into());
        }
        let data_dir = std::env::var("COLLAB_DATA_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("collab-data"));
        let lease_ttl_secs = std::env::var("COLLAB_LEASE_TTL_SECS").ok().and_then(|value| value.parse().ok()).unwrap_or(120);
        let cabinet = cabinet_from_env()?;
        Ok(Config { bind, jwt_secret, jwt_pem, data_dir, lease_ttl_secs, cabinet, heartbeat: PRESENCE_TTL / 3, peer_queue: PEER_QUEUE })
    }
}

/// The commit callback from `COLLAB_CABINET_URL` / `COLLAB_CABINET_TOKEN`.
/// The token is a secret: it is read here, kept in `CabinetConfig` (whose
/// `Debug` redacts it) and never written to a log, a metric or a URL.
fn cabinet_from_env() -> Result<Option<CabinetConfig>, String> {
    let url = std::env::var("COLLAB_CABINET_URL").ok().map(|url| url.trim().to_string()).filter(|url| !url.is_empty());
    let Some(url) = url else {
        return Ok(None);
    };
    if !url.starts_with("http://") {
        return Err("COLLAB_CABINET_URL must start with http:// (TLS is terminated by the ingress; https:// is not supported)".into());
    }
    if !url.contains("{sessionId}") {
        return Err("COLLAB_CABINET_URL must contain {sessionId}: the endpoint is /api/v1/internal/office/sessions/{sessionId}/commit".into());
    }
    let token = std::env::var("COLLAB_CABINET_TOKEN").ok().map(|token| token.trim().to_string()).filter(|token| !token.is_empty());
    let Some(token) = token else {
        return Err("COLLAB_CABINET_TOKEN is required with COLLAB_CABINET_URL".into());
    };
    let timeout_ms = std::env::var("COLLAB_CABINET_TIMEOUT_MS").ok().and_then(|value| value.parse().ok()).unwrap_or(cabinet::DEFAULT_TIMEOUT_MS);
    let attempts = std::env::var("COLLAB_CABINET_ATTEMPTS").ok().and_then(|value| value.parse().ok()).unwrap_or(cabinet::DEFAULT_ATTEMPTS).max(1);
    Ok(Some(CabinetConfig { url, token, timeout: Duration::from_millis(timeout_ms.max(1)), attempts }))
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
#[derive(Clone)]
struct Peer {
    role: Role,
    sid: String,
    out: mpsc::Sender<ServerMsg>,
    /// Set when a document frame could not be queued for this peer: its outbox
    /// is full, so it has already missed an accepted operation. Its connection
    /// ends instead of staying open with a document the room has moved past
    /// (ADR WC-0002).
    stale: Arc<Stale>,
}

impl Peer {
    fn new(role: Role, sid: &str, out: mpsc::Sender<ServerMsg>) -> Peer {
        Peer { role, sid: sid.to_string(), out, stale: Arc::new(Stale::default()) }
    }
}

/// A peer that missed a document frame: the flag is what the connection loop
/// reads, the notification is what wakes it out of a write that is stuck
/// because the peer stopped reading. Without the notification the loop is
/// inside that write and cannot look at the flag (ADR WC-0002).
#[derive(Default)]
struct Stale {
    flag: AtomicBool,
    wake: tokio::sync::Notify,
}

impl Stale {
    /// Mark the peer stale; `true` the first time only.
    fn mark(&self) -> bool {
        let first = !self.flag.swap(true, Ordering::Relaxed);
        self.wake.notify_one();
        first
    }
    fn is_set(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Resolves when the peer is marked stale. The permit `notify_one` leaves
    /// behind is taken here, so a mark that lands between two writes is not
    /// lost.
    async fn wait(&self) {
        self.wake.notified().await;
    }
}

impl std::fmt::Debug for Stale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Notify` is not `Debug`; the flag is what matters in a failure message.
        f.debug_struct("Stale").field("set", &self.is_set()).finish()
    }
}

/// A document room and its connected peers.
struct RoomState {
    room: Room<SessionModel>,
    peers: HashMap<String, Peer>,
    /// Cabinet session that holds the lease; a different session may join only
    /// as `owner` or after the lease went silent (ADR-0008).
    session: String,
    lease_seen: std::time::Instant,
    /// Lease TTL for this room.
    lease_ttl_secs: u64,
    /// Fixed-window operation counter, so one client cannot flood a room.
    window: std::time::Instant,
    ops_in_window: u32,
    /// The cabinet's copy of the last accepted commit: the etag the next
    /// commit is checked against (M87). Stored with the snapshot, so a restart
    /// keeps the conflict check.
    published: Option<Published>,
    /// The file the room built for a commit the cabinet refused with `409`, so
    /// the owner can save a copy without overwriting the cabinet (TASK.md §6).
    /// Kept in memory only: it is not part of the document and a restart loses
    /// nothing the cabinet did not already refuse.
    refused: Option<Refused>,
}

/// A built `.docx` the cabinet refused with `409`, kept for the owner's
/// "save a copy" action (TASK.md §6).
#[derive(Clone)]
struct Refused {
    version: Version,
    sha256: String,
    warnings: Vec<String>,
    /// Shared so a request for the copy does not copy the whole file.
    bytes: Arc<Vec<u8>>,
}

/// Counters for `/metrics`.
#[derive(Default)]
pub struct Metrics {
    connections: AtomicU64,
    ops: AtomicU64,
    rejects: AtomicU64,
    commits: AtomicU64,
    presence: AtomicU64,
    /// Commits the cabinet accepted (M87).
    published: AtomicU64,
    /// Commits the cabinet refused with `409` (the file changed elsewhere).
    publish_conflicts: AtomicU64,
    /// Commits not stored by the cabinet: a wrong token, a size refusal, an
    /// exhausted retry or any other refusal.
    publish_errors: AtomicU64,
    /// Refused copies the owner took to save (TASK.md §6).
    copies: AtomicU64,
    /// Peer outboxes that overflowed: each one missed a document frame and was
    /// closed (ADR WC-0002).
    stale: AtomicU64,
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
            ("cabinet_published_total", self.published.load(Ordering::Relaxed)),
            ("cabinet_conflicts_total", self.publish_conflicts.load(Ordering::Relaxed)),
            ("cabinet_errors_total", self.publish_errors.load(Ordering::Relaxed)),
            ("save_copies_total", self.copies.load(Ordering::Relaxed)),
            ("stale_peers_total", self.stale.load(Ordering::Relaxed)),
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
    store: Store,
    /// The cabinet commit callback, when configured.
    cabinet: Option<Cabinet>,
}

impl AppState {
    pub fn new(cfg: Config) -> AppState {
        let store = Store::new(&cfg.data_dir);
        let cabinet = cfg.cabinet.as_ref().map(Cabinet::new);
        AppState { cfg: Arc::new(cfg), rooms: Arc::new(Mutex::new(HashMap::new())), metrics: Arc::new(Metrics::default()), store, cabinet }
    }

    /// Write the room's snapshot (ADR-0005): model JSON plus the source `.docx`
    /// when one is known, plus the `opId` window so retries survive a restart,
    /// plus the cabinet etag when the room published a commit (M87).
    fn persist(&self, doc: &str, room: &Arc<Mutex<RoomState>>, source: Option<Vec<u8>>) -> Result<(), String> {
        let (version, epoch, model, dedup, cabinet) = {
            let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
            (guard.room.version(), guard.room.epoch(), guard.room.model().snapshot(), guard.room.dedup(), guard.published.clone())
        };
        self.store.save(doc, &Snapshot { version, epoch, model, source, dedup, cabinet })
    }

    fn room_for(&self, doc: &str) -> Arc<Mutex<RoomState>> {
        let mut rooms = self.rooms.lock().unwrap_or_else(|poison| poison.into_inner());
        rooms
            .entry(doc.to_string())
            .or_insert_with(|| {
                let restored = self.store.load(doc).and_then(|snapshot| {
                    if snapshot.model.len() > MAX_DOCUMENT {
                        log::warn!("ignoring a snapshot of {} bytes for {doc}", snapshot.model.len());
                        return None;
                    }
                    let mut document = serde_json::from_slice::<wordcraft_doc::Document>(&snapshot.model).ok()?;
                    // `Document::kept` is not serialized (ADR-0011): take it from
                    // the source `.docx` so unchanged paragraphs still write their
                    // original markup. Fingerprints only match paragraphs that
                    // were not edited since the file was read.
                    if let Some(source) = &snapshot.source
                        && let Ok(from_file) = wordcraft_docx::read_with_password(source, None)
                    {
                        document.kept = from_file.kept;
                    }
                    Some(RoomState {
                        room: Room::restore(SessionModel::from_document(document), snapshot.version, snapshot.epoch, snapshot.dedup),
                        peers: HashMap::new(),
                        session: String::new(),
                        lease_seen: std::time::Instant::now(),
                        lease_ttl_secs: self.cfg.lease_ttl_secs,
                        window: std::time::Instant::now(),
                        ops_in_window: 0,
                        published: snapshot.cabinet,
                        refused: None,
                    })
                });
                Arc::new(Mutex::new(restored.unwrap_or_else(|| RoomState {
                    room: Room::new(SessionModel::from_document(wordcraft_doc::Document::new())),
                    peers: HashMap::new(),
                    session: String::new(),
                    lease_seen: std::time::Instant::now(),
                    lease_ttl_secs: self.cfg.lease_ttl_secs,
                    window: std::time::Instant::now(),
                    ops_in_window: 0,
                    published: None,
                    refused: None,
                })))
            })
            .clone()
    }

    pub fn room_count(&self) -> usize {
        self.rooms.lock().unwrap_or_else(|poison| poison.into_inner()).len()
    }

    /// Write every open room before the process exits. A container gets
    /// SIGTERM; without this the last operations since the periodic snapshot
    /// would be lost.
    pub fn persist_all(&self) -> Result<(), String> {
        let rooms: Vec<(String, Arc<Mutex<RoomState>>)> = {
            let guard = self.rooms.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.iter().map(|(doc, room)| (doc.clone(), room.clone())).collect()
        };
        for (doc, room) in rooms {
            self.persist(&doc, &room, None)?;
        }
        Ok(())
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
    // Every write to this peer is bounded by one heartbeat period: a peer that
    // stopped reading is closed instead of holding the connection open with a
    // document the room no longer has (ADR WC-0002).
    let limit = state.cfg.heartbeat;
    let (tx, mut rx) = mpsc::channel::<ServerMsg>(state.cfg.peer_queue.max(1));
    let (plan, peers, stale) = match register(&room, &client_id, &claims.sid, role, &tx, wants_version) {
        Ok(parts) => parts,
        Err(reject) => {
            let _ = send(&mut sink, &ServerMsg::Rejected { reject }, limit).await;
            let _ = sink.close().await;
            return;
        }
    };
    if !send_join(&mut sink, &claims, role, &plan, peers, limit).await {
        leave(&state, &claims.doc, &room, &client_id);
        return;
    }
    announce_join(&room, &client_id, role);

    let conn = Conn { state: &state, room: &room, claims: &claims, role, client_id: &client_id, limit };

    let mut heartbeat = interval(state.cfg.heartbeat);
    heartbeat.tick().await; // the first tick is immediate; the first frame stays an answer
    loop {
        tokio::select! {
            incoming = stream.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => continue,
                };
                if !process_frame(&conn, &mut sink, &text).await {
                    break;
                }
            }
            outgoing = rx.recv() => {
                let Some(msg) = outgoing else { break };
                // The outbox overflowed while this peer was not reading: the
                // accepted operation it missed is gone from its queue, so the
                // connection ends here and the client reconnects and resyncs.
                // `stale.wait()` cancels a write that is stuck on that peer, so
                // the close does not wait for the write bound.
                if stale.is_set() {
                    close_stale(&mut sink, limit).await;
                    break;
                }
                tokio::select! {
                    _ = stale.wait() => {
                        close_stale(&mut sink, limit).await;
                        break;
                    }
                    sent = send(&mut sink, &msg, limit) => {
                        if sent.is_err() {
                            break;
                        }
                    }
                }
            }
            _ = heartbeat.tick() => {
                // The queue drop happens while this loop is blocked on a write,
                // so the flag can only be read here or on the next frame; the
                // heartbeat is what guarantees a stale peer is noticed even
                // when nothing else arrives.
                if stale.is_set() {
                    close_stale(&mut sink, limit).await;
                    break;
                }
                let ping = timeout(limit, sink.send(Message::Ping(Vec::new().into()))).await;
                if !matches!(ping, Ok(Ok(()))) {
                    break;
                }
            }
        }
    }
    leave(&state, &claims.doc, &room, &client_id);
}

/// End a connection whose outbox overflowed. The peer has already missed an
/// accepted operation, so the version it holds can no longer catch up from its
/// own queue; closing makes the client reconnect with its `lastVersion`, and
/// the join answers with the journal tail or a snapshot (ADR WC-0002).
async fn close_stale(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, limit: Duration) {
    log::warn!("closing a peer whose outbox overflowed: it missed an accepted operation");
    if timeout(limit, sink.send(Message::Close(None))).await.is_err() {
        log::warn!("the stale peer did not take the close frame; dropping the socket");
    }
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
            let _ = send(sink, &ServerMsg::Rejected { reject }, state.cfg.heartbeat).await;
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
    sid: &str,
    role: Role,
    tx: &mpsc::Sender<ServerMsg>,
    wants_version: Option<Version>,
) -> Result<(wordcraft_collab::room::JoinPlan, Vec<Participant>, Arc<Stale>), Reject> {
    let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
    const MAX_PARTICIPANTS: usize = 8;
    if guard.peers.len() >= MAX_PARTICIPANTS && !guard.peers.contains_key(client_id) {
        let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::TooLarge, "the room is full");
        return Err(reject);
    }
    if guard.session.is_empty() {
        guard.session = sid.to_string();
        guard.lease_seen = std::time::Instant::now();
    } else if guard.session != sid {
        let lease_alive = guard.peers.values().any(|peer| peer.sid == guard.session);
        let stale = guard.lease_seen.elapsed().as_secs() >= guard.lease_ttl_secs;
        if role != Role::Owner && (lease_alive || !stale) {
            let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Lease, "another session holds this document");
            return Err(reject);
        }
        guard.session = sid.to_string();
        guard.lease_seen = std::time::Instant::now();
    }
    guard.lease_seen = std::time::Instant::now();
    let plan = guard.room.join(wants_version, false);
    let peer = Peer::new(role, sid, tx.clone());
    // The connection keeps the flag, so a queue drop on any path (present)
    // or future one closes *this* connection (ADR WC-0002).
    let stale = peer.stale.clone();
    guard.peers.insert(client_id.to_string(), peer);
    let peers = guard
        .peers
        .iter()
        .map(|(id, peer)| Participant { client_id: id.clone(), role: peer.role.as_str().to_string(), color: None, presence: None })
        .collect();
    Ok((plan, peers, stale))
}

async fn send_join(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    claims: &Claims,
    role: Role,
    plan: &wordcraft_collab::room::JoinPlan,
    peers: Vec<Participant>,
    limit: Duration,
) -> bool {
    let welcome = ServerMsg::Welcome {
        session_id: claims.sid.clone(),
        role: role.as_str().to_string(),
        version: plan.version,
        epoch: plan.epoch,
        participants: peers,
        resync: plan.snapshot.is_some(),
    };
    if send(sink, &welcome, limit).await.is_err() {
        return false;
    }
    if let Some(snapshot) = plan.snapshot.clone() {
        for msg in snapshot_frames(plan.version, snapshot) {
            if send(sink, &msg, limit).await.is_err() {
                return false;
            }
        }
    } else if let Some(tail) = plan.replay.clone()
        && send(sink, &ServerMsg::Tail { entries: tail }, limit).await.is_err()
    {
        return false;
    }
    send(sink, &ServerMsg::Nodes { nodes: plan.nodes.clone() }, limit).await.is_ok()
}

/// Tell the peers already in the room who just joined and with what role, so
/// their participant list shows the role before the newcomer's first presence
/// frame (TASK.md §3). The frame is an ordinary `presence` with an empty state:
/// the clients already fold it through `apply_presence`, and an older peer that
/// does not know the `role` field still parses it. The role comes from the JWT
/// claim (ADR-0008), never from the wire, and the newcomer is not echoed its own
/// join: it learns the existing peers from `welcome`.
fn announce_join(room: &Arc<Mutex<RoomState>>, client_id: &str, role: Role) {
    let targets: Vec<Peer> = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.iter().filter(|(id, _)| id.as_str() != client_id).map(|(_, peer)| peer.clone()).collect()
    };
    let msg = ServerMsg::Presence { client_id: client_id.to_string(), role: role.as_str().to_string(), state: serde_json::json!({}) };
    for peer in &targets {
        // Presence is droppable; a full queue must not stall the room.
        queue_ephemeral(peer, &msg);
    }
}

/// One authenticated connection: what a frame handler needs to answer it.
#[derive(Clone, Copy)]
struct Conn<'a> {
    state: &'a AppState,
    room: &'a Arc<Mutex<RoomState>>,
    claims: &'a Claims,
    role: Role,
    client_id: &'a str,
    /// The write bound: one heartbeat period (ADR WC-0002).
    limit: Duration,
}

async fn process_frame(conn: &Conn<'_>, sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, text: &str) -> bool {
    let Conn { state, room, claims, role, client_id, limit } = *conn;
    let msg = match parse_client(text) {
        Ok(msg) => msg,
        Err(reject) => return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok(),
    };
    {
        let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        if guard.session == claims.sid {
            guard.lease_seen = std::time::Instant::now();
        }
    }
    if now_secs() >= claims.exp {
        let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::TokenExpired, "the session token expired");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    match msg {
        ClientMsg::Op { op } => handle_op(conn, sink, op).await,
        ClientMsg::Presence { state: presence } => {
            Metrics::inc(&state.metrics.presence);
            // Ephemeral state goes to the other participants; the author already
            // knows its own caret and the cabinet's UI keeps it locally.
            let targets: Vec<Peer> = {
                let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
                guard.peers.iter().filter(|(id, _)| id.as_str() != client_id).map(|(_, peer)| peer.clone()).collect()
            };
            let msg = ServerMsg::Presence { client_id: client_id.to_string(), role: role.as_str().to_string(), state: presence };
            for peer in &targets {
                // Presence is droppable; a full queue must not stall the room.
                queue_ephemeral(peer, &msg);
            }
            true
        }
        ClientMsg::Resync { last_version } => {
            let (plan, peers) = {
                let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
                let peers = guard
                    .peers
                    .iter()
                    .map(|(id, peer)| Participant { client_id: id.clone(), role: peer.role.as_str().to_string(), color: None, presence: None })
                    .collect();
                (guard.room.join(Some(last_version), true), peers)
            };
            send_join(sink, claims, role, &plan, peers, limit).await
        }
        ClientMsg::Commit { op_id, expected_version, .. } => {
            let call = CommitCall { client_id, op_id, expected_version };
            handle_commit(conn, sink, call).await
        }
        ClientMsg::SaveCopy { op_id } => handle_save_copy(conn, sink, op_id).await,
        ClientMsg::Ping { ts } => send(sink, &ServerMsg::Pong { ts }, limit).await.is_ok(),
        ClientMsg::Auth { .. } => {
            let reject = Reject::new(wordcraft_collab::wire::OpId::new("server", 0), RejectCode::Invalid, "already authenticated");
            send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok()
        }
    }
}

async fn handle_op(conn: &Conn<'_>, sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, op: wordcraft_collab::wire::Op) -> bool {
    let Conn { state, room, claims, role, client_id, limit } = *conn;
    // An operation id names its author. A peer that could put another client's
    // id on the wire would make the victim skip the answer as its own echo and
    // then, on its own retry, get a cached answer for an edit it never made.
    if op.op_id.client != client_id {
        Metrics::inc(&state.metrics.rejects);
        let reject = Reject::new(op.op_id.clone(), RejectCode::Invalid, "the operation id belongs to another client");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    let structural = op.kind.is_structural();
    let result = {
        let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        if guard.window.elapsed() >= std::time::Duration::from_secs(1) {
            guard.window = std::time::Instant::now();
            guard.ops_in_window = 0;
        }
        if guard.ops_in_window >= 200 {
            Err(Reject::new(op.op_id.clone(), RejectCode::TooLarge, "too many operations; retry in a moment"))
        } else {
            guard.ops_in_window += 1;
            guard.room.submit(&claims.sub, role, op)
        }
    };
    match result {
        Ok(applied) => {
            Metrics::inc(&state.metrics.ops);
            // A command that allocates ids carries them on the answer; peers
            // apply the delta instead of replacing the whole map.
            let delta_sent = applied.nodes.is_some();
            broadcast(state, room, client_id, ServerMsg::Applied { applied }, sink).await;
            let nodes = {
                let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
                let moved = guard.room.take_nodes_changed();
                if !delta_sent && (structural || moved) { Some(guard.room.nodes()) } else { None }
            };
            if let Some(nodes) = nodes {
                broadcast(state, room, "", ServerMsg::Nodes { nodes }, sink).await;
            }
            let due = room.lock().unwrap_or_else(|poison| poison.into_inner()).room.should_snapshot();
            if due && let Err(message) = state.persist(&claims.doc, room, None) {
                log::warn!("snapshot failed for {}: {message}", claims.doc);
            }
            true
        }
        Err(reject) => {
            Metrics::inc(&state.metrics.rejects);
            send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok()
        }
    }
}

/// One commit frame from the owner: the caller's client id (so the answer
/// reaches each peer exactly once), the operation id and the room version the
/// commit was taken at.
struct CommitCall<'a> {
    client_id: &'a str,
    op_id: wordcraft_collab::wire::OpId,
    expected_version: Version,
}

async fn handle_commit(conn: &Conn<'_>, sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, call: CommitCall<'_>) -> bool {
    let Conn { state, room, claims, role, limit, .. } = *conn;
    let CommitCall { client_id, op_id, expected_version } = call;
    if !role.can_commit() {
        let reject = Reject::new(op_id, RejectCode::Forbidden, "only the owner commits");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    let version = room.lock().unwrap_or_else(|poison| poison.into_inner()).room.version();
    if expected_version != version {
        let reject = Reject::new(op_id, RejectCode::Conflict, format!("head is version {version}"));
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    let (bytes, warnings) = match build_docx(room) {
        Ok(built) => built,
        Err(message) => {
            let reject = Reject::new(op_id, RejectCode::Invalid, message);
            return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
        }
    };
    if bytes.len() > MAX_DOCUMENT {
        let reject = Reject::new(op_id, RejectCode::TooLarge, format!("the built file is {} bytes", bytes.len()));
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    Metrics::inc(&state.metrics.commits);
    let sha256 = hex(&sha2::Sha256::digest(&bytes));

    // The cabinet owns the stored file: post the built bytes and let it name
    // the version. Without a callback the host owns the commit and only the
    // built file's digest travels back (M87).
    let outcome = match &state.cabinet {
        Some(cabinet) => {
            let expected = room.lock().unwrap_or_else(|poison| poison.into_inner()).published.as_ref().map(|published| published.etag.clone());
            let publication =
                Publication { session_id: &claims.sid, file_id: &claims.doc, bytes: &bytes, sha256: &sha256, version, expected: expected.as_deref() };
            Some(cabinet.publish(&publication).await)
        }
        None => None,
    };

    let cabinet = match outcome {
        Some(Ok(published)) => {
            Metrics::inc(&state.metrics.published);
            let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.published = Some(published.clone());
            // The refused copy belonged to an older attempt; the cabinet now
            // holds this one.
            guard.refused = None;
            Some(CabinetCommit { version: published.version, etag: published.etag })
        }
        Some(Err(PublishError::Conflict(message))) => {
            Metrics::inc(&state.metrics.publish_conflicts);
            // The file changed outside the session: the room must not
            // overwrite it, and every replica resyncs so the refusal and the
            // stale base are visible everywhere. The built file stays in the
            // room for the owner's "save a copy" action (TASK.md §6).
            log::warn!("cabinet refused the commit of {} as a conflict: {message}", claims.doc);
            let refused = Refused { version, sha256: sha256.clone(), warnings: warnings.clone(), bytes: Arc::new(bytes.clone()) };
            room.lock().unwrap_or_else(|poison| poison.into_inner()).refused = Some(refused);
            let notice =
                ServerMsg::Error { code: "cabinetConflict".into(), message: format!("{message}; save a copy or reopen the file"), retryable: true };
            let _ = state.persist(&claims.doc, room, Some(bytes));
            broadcast(state, room, client_id, notice, sink).await;
            return true;
        }
        Some(Err(error)) => {
            Metrics::inc(&state.metrics.publish_errors);
            // The token and the body stay out of the log; the status does not.
            log::warn!("cabinet did not store the commit of {}: {error}", claims.doc);
            let code = match &error {
                PublishError::Unauthorized(_) => RejectCode::Forbidden,
                PublishError::TooLarge(_) => RejectCode::TooLarge,
                PublishError::Unavailable(_) | PublishError::Refused(_) | PublishError::Conflict(_) => RejectCode::Invalid,
            };
            let _ = state.persist(&claims.doc, room, Some(bytes));
            let reject = Reject::new(op_id, code, error.to_string());
            return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
        }
        None => None,
    };

    if let Err(message) = state.persist(&claims.doc, room, Some(bytes)) {
        log::warn!("snapshot after commit failed: {message}");
    }
    // The version and etag are the cabinet's; every peer shows them, so the
    // participants panel never claims a save the cabinet did not take.
    broadcast(state, room, client_id, ServerMsg::Committed { version, sha256, warnings, cabinet }, sink).await;
    true
}

/// The owner asked for the file of a refused commit (`409`): send the bytes the
/// room built, chunked like a snapshot, to this peer only. Nothing is written
/// to the cabinet (TASK.md §6).
async fn handle_save_copy(
    conn: &Conn<'_>,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    op_id: wordcraft_collab::wire::OpId,
) -> bool {
    let Conn { state, room, role, limit, .. } = *conn;
    if !role.can_commit() {
        let reject = Reject::new(op_id, RejectCode::Forbidden, "only the owner saves a copy of a refused commit");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    }
    let refused = room.lock().unwrap_or_else(|poison| poison.into_inner()).refused.clone();
    let Some(refused) = refused else {
        let reject = Reject::new(op_id, RejectCode::Conflict, "there is no refused commit to save; commit again to see the conflict");
        return send(sink, &ServerMsg::Rejected { reject }, limit).await.is_ok();
    };
    Metrics::inc(&state.metrics.copies);
    let mut offset = 0;
    loop {
        let end = (offset + SNAPSHOT_CHUNK).min(refused.bytes.len());
        let last = end >= refused.bytes.len();
        let chunk = ServerMsg::SaveCopy {
            version: refused.version,
            sha256: refused.sha256.clone(),
            warnings: refused.warnings.clone(),
            bytes: refused.bytes[offset..end].to_vec(),
            last,
        };
        if send(sink, &chunk, limit).await.is_err() {
            return false;
        }
        if last {
            return true;
        }
        offset = end;
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0)
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

/// Write one frame to a peer, bounded by the heartbeat period. A write that
/// does not finish in time means the peer stopped reading: the caller ends the
/// connection and the client reconnects and resyncs (ADR WC-0002).
async fn send(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, msg: &ServerMsg, limit: Duration) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    match timeout(limit, sink.send(Message::Text(text.into()))).await {
        Ok(result) => result.map_err(|_| ()),
        Err(_) => Err(()),
    }
}

/// Queue one document frame for a peer. `false` means the peer missed it: its
/// outbox is full or its connection is gone. A full outbox marks the peer
/// stale — it has already missed an accepted operation — and its connection is
/// closed instead of staying open with a document the room moved past
/// (ADR WC-0002). Counted once per peer.
fn queue(state: &AppState, peer: &Peer, msg: &ServerMsg) -> bool {
    match peer.out.try_send(msg.clone()) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => {
            if peer.stale.mark() {
                Metrics::inc(&state.metrics.stale);
                log::warn!("a peer's outbox is full: it missed a document frame and will be closed");
            }
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

/// Queue an ephemeral frame (presence, a leave notice). Droppable by design: a
/// full outbox must not stall the room, and a lost presence frame is repaired
/// by the next one (ADR-0006). It never marks a peer stale, because it carries
/// no document state.
fn queue_ephemeral(peer: &Peer, msg: &ServerMsg) {
    let _ = peer.out.try_send(msg.clone());
}

/// Send to every peer of the room (`broadcast` includes the sender, so the
/// author sees the authoritative version too).
async fn broadcast(
    state: &AppState,
    room: &Arc<Mutex<RoomState>>,
    sender: &str,
    msg: ServerMsg,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) {
    let targets: Vec<Peer> = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.iter().filter(|(id, _)| id.as_str() != sender).map(|(_, peer)| peer.clone()).collect()
    };
    for peer in &targets {
        // Bounded: a client that stops reading loses the frame instead of
        // growing the server, and is closed rather than left stale: the frame
        // it missed is an accepted operation it can no longer derive.
        let _ = queue(state, peer, &msg);
    }
    let _ = send(sink, &msg, state.cfg.heartbeat).await;
}

fn leave(state: &AppState, doc: &str, room: &Arc<Mutex<RoomState>>, client_id: &str) {
    let gone = {
        let mut guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.remove(client_id)
    };
    if gone.is_some() {
        let targets: Vec<Peer> = {
            let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
            guard.peers.values().cloned().collect()
        };
        for peer in &targets {
            queue_ephemeral(peer, &ServerMsg::Left { client_id: client_id.to_string() });
        }
    }
    let empty = {
        let guard = room.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.peers.is_empty()
    };
    if empty {
        // The last peer left: keep the room's state on disk before dropping it.
        if let Err(message) = state.persist(doc, room, None) {
            log::warn!("snapshot on leave failed for {doc}: {message}");
        }
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
        guard.room.model().session().doc.clone()
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
