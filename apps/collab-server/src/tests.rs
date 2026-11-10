//! Server tests: token checks and a real WebSocket round trip with two clients.

use futures_util::{SinkExt, StreamExt};
use jsonwebtoken::{EncodingKey, Header};
use serde_json::json;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::tungstenite::Message;

use wordcraft_collab::wire::{ClientMsg, HistoryAction, Op, OpId, OpKind, ServerMsg, Target};

use crate::{AppState, Claims, Config, app};

fn token(cfg: &Config, sub: &str, role: &str, app: &str, ttl: i64) -> String {
    token_sid(cfg, sub, role, app, ttl, "session-1")
}

fn token_sid(cfg: &Config, sub: &str, role: &str, app: &str, ttl: i64, sid: &str) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    let claims = Claims {
        sub: sub.into(),
        aud: "collab".into(),
        doc: "doc-1".into(),
        app: app.into(),
        role: role.into(),
        sid: sid.into(),
        iat: now,
        exp: if ttl >= 0 { now + ttl as u64 } else { now.saturating_sub(ttl.unsigned_abs()) },
    };
    let secret = cfg.jwt_secret.as_deref().unwrap_or_default();
    jsonwebtoken::encode(&Header::new(jsonwebtoken::Algorithm::HS256), &claims, &EncodingKey::from_secret(secret.as_bytes())).expect("encode")
}

fn dev_config() -> Config {
    let dir = std::env::temp_dir().join(format!("wordcraft-collab-test-{}", std::process::id()));
    Config { bind: "127.0.0.1:0".into(), jwt_secret: Some("test-secret".into()), jwt_pem: None, data_dir: dir, lease_ttl_secs: 120, cabinet: None }
}

/// A cabinet on loopback: each accepted connection is answered with the next
/// reply (`None` closes without an answer, a lost response) and its request is
/// queued for the test. The URL's `{sessionId}` is substituted by the server.
async fn fake_cabinet(replies: Vec<Option<&'static str>>) -> (String, tokio::sync::mpsc::Receiver<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("address");
    let (sender, receiver) = tokio::sync::mpsc::channel(16);
    tokio::spawn(async move {
        for reply in replies {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let mut request = Vec::new();
            let mut chunk = [0u8; 8192];
            while let Ok(n) = stream.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
                if let Some(split) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..split]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:").map(|value| value.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    if request.len() >= split + 4 + length {
                        break;
                    }
                }
            }
            let _ = sender.send(String::from_utf8_lossy(&request).to_string()).await;
            if let Some(reply) = reply {
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        }
    });
    (format!("http://{address}/api/v1/internal/office/sessions/{{sessionId}}/commit"), receiver)
}

fn cabinet_reply(body: &'static str) -> &'static str {
    // `Content-Length` must match the body, so the reply is built once here.
    Box::leak(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_boxed_str())
}

fn config_with_cabinet(cfg: &Config, url: &str) -> Config {
    Config {
        cabinet: Some(crate::cabinet::CabinetConfig {
            url: url.to_string(),
            token: "service-token".into(),
            timeout: Duration::from_secs(5),
            attempts: 3,
        }),
        ..cfg.clone()
    }
}

async fn spawn_server_with(cfg: Config) -> String {
    let state = AppState::new(cfg);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app(state)).await;
    });
    format!("ws://{addr}/ws")
}

async fn next_committed_cabinet(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) -> (u64, Option<wordcraft_collab::wire::CabinetCommit>) {
    loop {
        match next_msg(ws).await {
            ServerMsg::Committed { version, cabinet, .. } => return (version, cabinet),
            ServerMsg::Applied { .. } | ServerMsg::Nodes { .. } | ServerMsg::Presence { .. } | ServerMsg::Left { .. } => continue,
            other => panic!("expected committed, got {other:?}"),
        }
    }
}

#[test]
fn a_valid_token_is_accepted() {
    let cfg = dev_config();
    let claims = crate::verify_token(&cfg, &token(&cfg, "alice", "editor", "wordcraft", 600)).expect("valid token");
    assert_eq!(claims.sub, "alice");
    assert_eq!(claims.role, "editor");
}

#[test]
fn an_expired_token_is_refused() {
    let cfg = dev_config();
    let err = crate::verify_token(&cfg, &token(&cfg, "alice", "editor", "wordcraft", -3600)).expect_err("expired");
    assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Forbidden);
}

#[test]
fn a_token_for_another_app_is_refused() {
    let cfg = dev_config();
    let err = crate::verify_token(&cfg, &token(&cfg, "alice", "editor", "gridcraft", 600)).expect_err("wrong app");
    assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Forbidden);
}

#[test]
fn an_unknown_role_is_refused() {
    let cfg = dev_config();
    let err = crate::verify_token(&cfg, &token(&cfg, "alice", "admin", "wordcraft", 600)).expect_err("bad role");
    assert_eq!(err.code, wordcraft_collab::wire::RejectCode::Forbidden);
}

async fn spawn_server() -> (String, Config) {
    let cfg = dev_config();
    let state = AppState::new(cfg.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app(state)).await;
    });
    (format!("ws://{addr}/ws"), cfg)
}

async fn connect(
    url: &str,
    cfg: &Config,
    role: &str,
    client: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    connect_from(url, cfg, role, client, None).await
}

async fn connect_ttl(
    url: &str,
    cfg: &Config,
    role: &str,
    client: &str,
    ttl: i64,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("connect");
    let auth = ClientMsg::Auth { token: token(cfg, client, role, "wordcraft", ttl), client_id: client.into(), last_version: None };
    ws.send(Message::Text(serde_json::to_string(&auth).expect("auth json").into())).await.expect("send auth");
    ws
}

async fn connect_from(
    url: &str,
    cfg: &Config,
    role: &str,
    client: &str,
    last_version: Option<u64>,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.expect("connect");
    let auth = ClientMsg::Auth { token: token(cfg, client, role, "wordcraft", 600), client_id: client.into(), last_version };
    ws.send(Message::Text(serde_json::to_string(&auth).expect("auth json").into())).await.expect("send auth");
    ws
}

async fn next_msg(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) -> ServerMsg {
    // A peer that was already in the room receives a join announcement (an
    // empty-state `presence` frame) whenever someone joins. Tests that count
    // operations, answers or real presence frames skip it here; the test that
    // checks the announcement reads raw frames through `next_frame`.
    loop {
        let msg = next_frame(ws).await;
        if is_join_announcement(&msg) {
            continue;
        }
        return msg;
    }
}

async fn next_frame(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) -> ServerMsg {
    loop {
        let frame = timeout(Duration::from_secs(5), ws.next()).await.expect("no timeout").expect("frame").expect("ok frame");
        match frame {
            Message::Text(text) => return serde_json::from_str(&text).expect("server message"),
            // Control frames may arrive at any time.
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("expected text frame, got {other:?}"),
        }
    }
}

/// A join announcement is the server telling a peer that was already in the
/// room who joined and with what role, before that participant's first
/// presence frame. It reuses the presence frame with an empty state.
fn is_join_announcement(msg: &ServerMsg) -> bool {
    matches!(msg, ServerMsg::Presence { state, .. } if state.as_object().is_some_and(|state| state.is_empty()))
}

async fn next_applied(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) -> wordcraft_collab::wire::Applied {
    loop {
        match next_msg(ws).await {
            ServerMsg::Applied { applied } => return applied,
            ServerMsg::Nodes { .. } | ServerMsg::Presence { .. } | ServerMsg::Left { .. } | ServerMsg::Tail { .. } => continue,
            other => panic!("expected applied, got {other:?}"),
        }
    }
}

async fn next_committed(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) -> (u64, String) {
    loop {
        match next_msg(ws).await {
            ServerMsg::Committed { version, sha256, .. } => return (version, sha256),
            ServerMsg::Applied { .. } | ServerMsg::Nodes { .. } | ServerMsg::Presence { .. } | ServerMsg::Left { .. } => continue,
            other => panic!("expected committed, got {other:?}"),
        }
    }
}

async fn node_of(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>) -> u32 {
    loop {
        match next_msg(ws).await {
            ServerMsg::Nodes { nodes } => {
                return nodes.iter().find(|n| n.story == "body").map(|n| n.id).expect("a body node");
            }
            ServerMsg::Welcome { .. } | ServerMsg::Snapshot { .. } | ServerMsg::Tail { .. } => {}
            other => panic!("unexpected before nodes: {other:?}"),
        }
    }
}

fn insert(client: &str, seq: u64, base: u64, node: u32, off: usize, text: &str) -> Op {
    Op {
        op_id: OpId::new(client, seq),
        base_version: base,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", node, off),
        payload: json!({"op": "insert_text", "text": text}),
        history: None,
    }
}

async fn send_op(ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, op: Op) -> bool {
    ws.send(Message::Text(serde_json::to_string(&ClientMsg::Op { op }).expect("json").into())).await.is_ok()
}

#[tokio::test]
async fn two_clients_edit_through_the_server() {
    let (url, cfg) = spawn_server().await;
    let mut alice = connect(&url, &cfg, "editor", "alice").await;
    let mut bob = connect(&url, &cfg, "editor", "bob").await;

    let welcome = next_msg(&mut alice).await;
    assert!(matches!(welcome, ServerMsg::Welcome { .. }));
    let node_alice = node_of(&mut alice).await;
    let _ = next_msg(&mut bob).await;
    let node_bob = node_of(&mut bob).await;
    assert_eq!(node_alice, node_bob, "both clients see the same session node");

    alice
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("alice", 1, 0, node_alice, 0, "Hello") }).expect("json").into()))
        .await
        .expect("send op");
    let applied = next_msg(&mut alice).await;
    assert!(matches!(applied, ServerMsg::Applied { .. }), "{applied:?}");
    let echoed = next_msg(&mut bob).await;
    assert!(matches!(echoed, ServerMsg::Applied { .. }), "{echoed:?}");

    bob.send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("bob", 1, 1, node_bob, 5, " world") }).expect("json").into()))
        .await
        .expect("send op");
    let applied = next_msg(&mut alice).await;
    match applied {
        ServerMsg::Applied { applied } => assert_eq!(applied.version, 2),
        other => panic!("expected applied, got {other:?}"),
    }
}

#[tokio::test]
async fn presence_reaches_the_other_client_and_moves_no_version() {
    let (url, cfg) = spawn_server().await;
    let mut alice = connect(&url, &cfg, "editor", "alice").await;
    let mut bob = connect(&url, &cfg, "editor", "bob").await;
    let _ = next_msg(&mut alice).await;
    let node = node_of(&mut alice).await;
    let _ = next_msg(&mut bob).await;
    let _ = node_of(&mut bob).await;

    alice
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Presence { state: json!({"cursor": {"story": "body", "nodeId": node, "off": 3}}) })
                .expect("json")
                .into(),
        ))
        .await
        .expect("presence");
    match next_msg(&mut bob).await {
        ServerMsg::Presence { client_id, role, state } => {
            assert_eq!(client_id, "alice");
            assert_eq!(role, "editor", "the server stamps the sender's role from the token");
            assert_eq!(state["cursor"]["off"], 3);
        }
        other => panic!("expected presence, got {other:?}"),
    }

    // Presence is ephemeral: the next accepted operation is version 1.
    alice
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("alice", 1, 0, node, 0, "x") }).expect("json").into()))
        .await
        .expect("op");
    match next_msg(&mut alice).await {
        ServerMsg::Applied { applied } => assert_eq!(applied.version, 1),
        other => panic!("expected applied, got {other:?}"),
    }
}

#[tokio::test]
async fn a_join_announcement_gives_earlier_peers_the_newcomer_role() {
    let (url, cfg) = spawn_server().await;
    let mut owner = connect(&url, &cfg, "owner", "alice").await;
    expect_welcome(&mut owner, &[("alice", "owner")]).await;
    let node = node_of(&mut owner).await;

    // The second client joins; alice, already in the room, learns bob's role
    // before bob sends any presence frame.
    let mut editor = connect(&url, &cfg, "editor", "bob").await;
    expect_welcome(&mut editor, &[("alice", "owner"), ("bob", "editor")]).await;
    let _ = node_of(&mut editor).await;
    expect_join(&mut owner, "bob", "editor").await;

    // A third participant joins with another role: both peers that were
    // already connected learn it from the join announcement, again before the
    // newcomer's first presence.
    let mut viewer = connect(&url, &cfg, "viewer", "carol").await;
    expect_welcome(&mut viewer, &[("alice", "owner"), ("bob", "editor"), ("carol", "viewer")]).await;
    let _ = node_of(&mut viewer).await;
    expect_join(&mut owner, "carol", "viewer").await;
    expect_join(&mut editor, "carol", "viewer").await;

    // Presence frames keep carrying the sender's role to every other peer.
    let presence = || ClientMsg::Presence { state: json!({"cursor": {"story": "body", "nodeId": node, "off": 1}}) };
    viewer.send(Message::Text(serde_json::to_string(&presence()).expect("json").into())).await.expect("presence");
    expect_presence(&mut owner, "carol", "viewer").await;
    expect_presence(&mut editor, "carol", "viewer").await;

    editor.send(Message::Text(serde_json::to_string(&presence()).expect("json").into())).await.expect("presence");
    expect_presence(&mut owner, "bob", "editor").await;
    expect_presence(&mut viewer, "bob", "editor").await;

    owner.send(Message::Text(serde_json::to_string(&presence()).expect("json").into())).await.expect("presence");
    expect_presence(&mut editor, "alice", "owner").await;
    expect_presence(&mut viewer, "alice", "owner").await;
}

/// Read the next frame, which must be a welcome listing exactly these peers and
/// their roles.
async fn expect_welcome(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    expected: &[(&str, &str)],
) {
    match next_msg(ws).await {
        ServerMsg::Welcome { participants, .. } => {
            let mut roles: Vec<_> = participants.iter().map(|p| (p.client_id.clone(), p.role.clone())).collect();
            roles.sort();
            let mut want: Vec<_> = expected.iter().map(|(id, role)| ((*id).to_string(), (*role).to_string())).collect();
            want.sort();
            assert_eq!(roles, want, "{participants:?}");
        }
        other => panic!("expected welcome, got {other:?}"),
    }
}

/// Read the next raw frame, which must be the join announcement for this peer.
async fn expect_join(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    client: &str,
    role: &str,
) {
    match next_frame(ws).await {
        ServerMsg::Presence { client_id, role: announced, state } => {
            assert_eq!(client_id, client);
            assert_eq!(announced, role, "the join announcement carries the token role");
            assert_eq!(state, json!({}), "the newcomer has not sent a caret yet");
        }
        other => panic!("expected a join announcement for {client}, got {other:?}"),
    }
}

async fn expect_presence(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    client: &str,
    role: &str,
) {
    loop {
        match next_msg(ws).await {
            ServerMsg::Presence { client_id, role: sender_role, .. } => {
                assert_eq!(client_id, client);
                assert_eq!(sender_role, role, "presence {client_id} must carry the token role");
                return;
            }
            ServerMsg::Nodes { .. } | ServerMsg::Left { .. } => continue,
            other => panic!("expected presence for {client}, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_reconnecting_client_catches_up_from_the_journal() {
    let (url, cfg) = spawn_server().await;
    let mut alice = connect(&url, &cfg, "editor", "alice").await;
    let mut bob = connect(&url, &cfg, "editor", "bob").await;
    let _ = next_msg(&mut alice).await;
    let node = node_of(&mut alice).await;
    let _ = next_msg(&mut bob).await;
    let _ = node_of(&mut bob).await;

    bob.send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("bob", 1, 0, node, 0, "B") }).expect("json").into()))
        .await
        .expect("bob");
    match next_msg(&mut bob).await {
        ServerMsg::Applied { applied } => assert_eq!(applied.version, 1),
        other => panic!("expected applied, got {other:?}"),
    }
    drop(bob);
    // Alice's edit lands while Bob is away; her queue may also carry `left`.
    alice
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("alice", 1, 1, node, 1, "A") }).expect("json").into()))
        .await
        .expect("alice");
    loop {
        match next_msg(&mut alice).await {
            ServerMsg::Applied { applied } if applied.version == 2 => break,
            ServerMsg::Applied { .. } => continue,
            ServerMsg::Left { .. } | ServerMsg::Presence { .. } | ServerMsg::Nodes { .. } => continue,
            other => panic!("expected applied, got {other:?}"),
        }
    }

    let mut bob = connect_from(&url, &cfg, "editor", "bob", Some(1)).await;
    match next_msg(&mut bob).await {
        ServerMsg::Welcome { version, resync, .. } => {
            assert_eq!(version, 2);
            assert!(!resync, "the journal still has the tail");
        }
        other => panic!("expected welcome, got {other:?}"),
    }
    let mut replayed = false;
    loop {
        match next_msg(&mut bob).await {
            ServerMsg::Tail { entries } => {
                assert!(entries.iter().any(|entry| entry.version == 2), "{entries:?}");
                replayed = true;
            }
            ServerMsg::Nodes { .. } => break,
            other => panic!("unexpected catch-up frame {other:?}"),
        }
    }
    assert!(replayed, "the missing version was replayed");
}

#[tokio::test]
async fn a_client_can_ask_for_a_snapshot_without_losing_the_journal() {
    let (url, cfg) = spawn_server().await;
    let mut alice = connect(&url, &cfg, "editor", "alice").await;
    let _ = next_msg(&mut alice).await;
    let node = node_of(&mut alice).await;
    alice
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("alice", 1, 0, node, 0, "Hello") }).expect("json").into()))
        .await
        .expect("insert");
    let _ = next_msg(&mut alice).await;

    alice.send(Message::Text(serde_json::to_string(&ClientMsg::Resync { last_version: 0 }).expect("json").into())).await.expect("resync");
    match next_msg(&mut alice).await {
        ServerMsg::Welcome { version, resync, .. } => {
            assert_eq!(version, 1);
            assert!(resync, "the client asked for the snapshot, not the tail");
        }
        other => panic!("expected welcome, got {other:?}"),
    }
    let mut last = false;
    loop {
        match next_msg(&mut alice).await {
            ServerMsg::Snapshot { last: done, .. } => {
                if done {
                    last = true;
                }
            }
            ServerMsg::Nodes { .. } => break,
            other => panic!("unexpected resync frame {other:?}"),
        }
    }
    assert!(last, "the snapshot ended");
}

#[tokio::test]
async fn a_ninth_participant_is_refused() {
    let (url, cfg) = spawn_server().await;
    let mut clients = Vec::new();
    for index in 0..8 {
        clients.push(connect(&url, &cfg, "editor", &format!("client-{index}")).await);
    }
    // Let registration finish before the ninth connects.
    for client in &mut clients {
        let _ = next_msg(client).await;
    }
    let mut ninth = connect(&url, &cfg, "editor", "client-8").await;
    match next_msg(&mut ninth).await {
        ServerMsg::Rejected { reject } => assert_eq!(reject.code, wordcraft_collab::wire::RejectCode::TooLarge),
        other => panic!("expected a full room, got {other:?}"),
    }
}

#[tokio::test]
async fn a_viewer_cannot_write() {
    let (url, cfg) = spawn_server().await;
    let mut viewer = connect(&url, &cfg, "viewer", "carol").await;
    let _ = next_msg(&mut viewer).await;
    let node = node_of(&mut viewer).await;

    viewer
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("carol", 1, 0, node, 0, "x") }).expect("json").into()))
        .await
        .expect("send op");
    match next_msg(&mut viewer).await {
        ServerMsg::Rejected { reject } => {
            assert_eq!(reject.code, wordcraft_collab::wire::RejectCode::Forbidden);
        }
        other => panic!("expected rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn an_operation_cannot_carry_another_clients_id() {
    let (url, cfg) = spawn_server().await;
    let mut alice = connect(&url, &cfg, "editor", "alice").await;
    let _ = next_msg(&mut alice).await;
    let node = node_of(&mut alice).await;

    // Authenticated as `alice`, the frame claims `bob`'s id: the room must
    // refuse it, not file it in bob's id space.
    assert!(send_op(&mut alice, insert("bob", 1, 0, node, 0, "forged")).await);
    match next_msg(&mut alice).await {
        ServerMsg::Rejected { reject } => {
            assert_eq!(reject.code, wordcraft_collab::wire::RejectCode::Invalid);
            assert!(reject.message.contains("another client"), "{}", reject.message);
        }
        other => panic!("expected rejection, got {other:?}"),
    }
    // Nothing was applied: alice's own operation still lands at version 1.
    assert!(send_op(&mut alice, insert("alice", 1, 0, node, 0, "mine")).await);
    match next_msg(&mut alice).await {
        ServerMsg::Applied { applied } => assert_eq!(applied.version, 1),
        other => panic!("expected applied, got {other:?}"),
    }
}

#[test]
fn a_room_is_restored_from_disk() {
    let cfg = dev_config();
    let doc = "doc-restore";
    let state = AppState::new(cfg.clone());
    let room = state.room_for(doc);
    let node = {
        let mut guard = room.lock().expect("room lock");
        guard.room.join(None, false).nodes.first().expect("a node").id
    };
    let op = insert("alice", 1, 0, node, 0, "Hi");
    room.lock().expect("room lock").room.submit("alice", wordcraft_collab::room::Role::Editor, op).expect("apply");
    state.persist(doc, &room, None).expect("persist");
    drop(room);
    drop(state);

    let restored = AppState::new(cfg);
    let room = restored.room_for(doc);
    let guard = room.lock().expect("room lock");
    assert_eq!(guard.room.version(), 1, "the snapshot restores the version");
    let text = guard.room.model().session().doc.body.first().and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default();
    assert_eq!(text, "Hi");
}

#[test]
fn a_retry_after_a_snapshot_restore_is_not_applied_twice() {
    let cfg = dev_config();
    let doc = "doc-dedup-restore";
    let state = AppState::new(cfg.clone());
    let room = state.room_for(doc);
    let node = {
        let mut guard = room.lock().expect("room lock");
        guard.room.join(None, false).nodes.first().expect("a node").id
    };
    let op = insert("alice", 1, 0, node, 0, "Hi");
    let first = room.lock().expect("room lock").room.submit("alice", wordcraft_collab::room::Role::Editor, op.clone()).expect("apply");
    assert_eq!(first.version, 1);
    state.persist(doc, &room, None).expect("persist");
    drop(room);
    drop(state);

    // A restart: the room comes back at version 1 with the saved `opId` window.
    let restored = AppState::new(cfg);
    let room = restored.room_for(doc);
    let mut guard = room.lock().expect("room lock");
    assert_eq!(guard.room.version(), 1);
    let again = guard.room.submit("alice", wordcraft_collab::room::Role::Editor, op).expect("the retry is deduplicated");
    assert_eq!(first, again, "the client gets the answer it lost");
    assert_eq!(guard.room.version(), 1, "the repeat moves no version");
    let text = guard.room.model().session().doc.body.first().and_then(|b| b.as_para()).map(|p| p.text.clone()).unwrap_or_default();
    assert_eq!(text, "Hi", "the operation is not applied a second time");
}

#[test]
fn a_restored_room_takes_kept_markup_from_the_source_docx() {
    let cfg = dev_config();
    let doc = "doc-kept";
    let state = AppState::new(cfg.clone());
    let room = state.room_for(doc);
    let source = wordcraft_docx::write(&wordcraft_doc::Document::new()).expect("a source docx");
    let from_file = wordcraft_docx::read_with_password(&source, None).expect("read the source");
    let expected = from_file.kept.get().is_some();
    state.persist(doc, &room, Some(source)).expect("persist model + source");
    drop(room);
    drop(state);

    let restored = AppState::new(cfg);
    let room = restored.room_for(doc);
    let guard = room.lock().expect("room lock");
    assert_eq!(guard.room.model().session().doc.kept.get().is_some(), expected, "kept comes back from the source, not the JSON");
}

#[test]
fn a_shutdown_snapshot_keeps_every_open_room() {
    let cfg = dev_config();
    let state = AppState::new(cfg.clone());
    let room = state.room_for("doc-shutdown");
    let node = {
        let mut guard = room.lock().expect("room lock");
        guard.room.join(None, false).nodes.first().expect("a node").id
    };
    let op = insert("alice", 1, 0, node, 0, "Bye");
    room.lock().expect("room lock").room.submit("alice", wordcraft_collab::room::Role::Editor, op).expect("apply");
    state.persist_all().expect("persist on shutdown");
    drop(room);
    drop(state);

    let restored = AppState::new(cfg);
    let room = restored.room_for("doc-shutdown");
    assert_eq!(room.lock().expect("room lock").room.version(), 1);
}

#[test]
fn a_second_session_needs_the_owner_role() {
    let state = AppState::new(dev_config());
    let room = state.room_for("doc-lease");
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    assert!(super::register(&room, "alice", "sid-1", wordcraft_collab::room::Role::Editor, &tx, None).is_ok());

    let denied = super::register(&room, "bob", "sid-2", wordcraft_collab::room::Role::Editor, &tx, None);
    assert_eq!(denied.expect_err("a foreign session needs the owner").code, wordcraft_collab::wire::RejectCode::Lease);

    assert!(super::register(&room, "carol", "sid-3", wordcraft_collab::room::Role::Owner, &tx, None).is_ok());
}

/// The acceptance gate from TASK.md §8: the committed file opens in LibreOffice
/// without a repair prompt. Ignored by default because it needs `soffice`.
#[tokio::test]
#[ignore = "needs LibreOffice (soffice) on PATH"]
async fn the_committed_docx_opens_in_libreoffice() {
    let (url, cfg) = spawn_server().await;
    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let _ = next_msg(&mut owner).await;
    let node = node_of(&mut owner).await;

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Op { op: insert("owner", 1, 0, node, 0, "Hello collaboration") }).expect("json").into(),
        ))
        .await
        .expect("send op");
    let _ = next_msg(&mut owner).await;

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: wordcraft_collab::wire::OpId::new("owner", 2), expected_version: 1, etag: None })
                .expect("json")
                .into(),
        ))
        .await
        .expect("commit");
    let committed = loop {
        match next_msg(&mut owner).await {
            ServerMsg::Committed { version, sha256, warnings, .. } => break (version, sha256, warnings),
            ServerMsg::Applied { .. } => continue,
            other => panic!("expected committed, got {other:?}"),
        }
    };
    assert_eq!(committed.0, 1);
    assert!(!committed.1.is_empty());

    let docx = cfg.data_dir.join("doc-1").join("source.docx");
    let bytes = std::fs::read(&docx).expect("the server saved the committed file");
    assert!(bytes.starts_with(b"PK"), "a .docx is a zip package");

    let work = std::env::temp_dir().join(format!("wordcraft-collab-lo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work dir");
    let status = std::process::Command::new("soffice")
        .args(["--headless", "--convert-to", "txt:Text", "--outdir"])
        .arg(&work)
        .arg(&docx)
        .status()
        .expect("run soffice");
    assert!(status.success(), "LibreOffice converts the committed document");

    let txt = std::fs::read_dir(&work)
        .expect("converted files")
        .filter_map(|entry| entry.ok())
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "txt"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap_or_default())
        .expect("a converted text file");
    assert!(txt.contains("Hello collaboration"), "converted text: {txt:?}");
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test]
async fn a_command_operation_runs_on_the_server() {
    let (url, cfg) = spawn_server().await;
    let mut alice = connect(&url, &cfg, "editor", "alice").await;
    let _ = next_msg(&mut alice).await;
    let node = node_of(&mut alice).await;

    let command = Op {
        op_id: OpId::new("alice", 1),
        base_version: 0,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", node, 0),
        payload: json!({"command": "text.insert", "params": {"text": "Hi"}}),
        history: None,
    };
    alice.send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: command }).expect("json").into())).await.expect("send");
    loop {
        match next_msg(&mut alice).await {
            ServerMsg::Applied { applied } => {
                assert_eq!(applied.version, 1);
                break;
            }
            // A command that grows the document also refreshes the node map.
            ServerMsg::Nodes { .. } => continue,
            other => panic!("expected applied, got {other:?}"),
        }
    }

    // Commands that reach beyond the document never run on a server.
    let forbidden = Op {
        op_id: OpId::new("alice", 2),
        base_version: 1,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", node, 0),
        payload: json!({"command": "file.new"}),
        history: None,
    };
    alice.send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: forbidden }).expect("json").into())).await.expect("send");
    match next_msg(&mut alice).await {
        ServerMsg::Rejected { reject } => assert_eq!(reject.code, wordcraft_collab::wire::RejectCode::Forbidden),
        other => panic!("expected rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn a_comment_allocation_reaches_a_peer_as_a_node_delta_and_it_keeps_editing() {
    let (url, cfg) = spawn_server().await;
    let mut alice = connect(&url, &cfg, "editor", "alice").await;
    let _ = next_msg(&mut alice).await;
    let node = node_of(&mut alice).await;
    assert!(send_op(&mut alice, insert("alice", 1, 0, node, 0, "Hello")).await);
    assert_eq!(next_applied(&mut alice).await.version, 1);

    let mut bob = connect(&url, &cfg, "editor", "bob").await;
    let _ = next_msg(&mut bob).await;
    let body = loop {
        if let ServerMsg::Nodes { nodes } = next_msg(&mut bob).await {
            break nodes.iter().find(|node| node.story == "body").expect("a body node").id;
        }
    };

    let comment = Op {
        op_id: OpId::new("alice", 2),
        base_version: 1,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", body, 0),
        payload: json!({
            "command": "review.newComment",
            "params": {"text": "note"},
            "selection": {"anchor": {"nodeId": body, "off": 0}, "focus": {"nodeId": body, "off": 5}},
        }),
        history: None,
    };
    assert!(send_op(&mut alice, comment).await);
    let applied = next_applied(&mut alice).await;
    assert_eq!(applied.version, 2);
    let delta = applied.nodes.clone().expect("the answer carries the new ids");
    let new_node = delta.changed.iter().find(|node| node.story.starts_with("part:")).expect("a comment paragraph").clone();

    // Bob receives the same answer with the delta and edits the comment
    // paragraph by the id it names: no snapshot is needed to continue.
    let peer = loop {
        if let ServerMsg::Applied { applied } = next_msg(&mut bob).await
            && applied.version == 2
        {
            break applied;
        }
    };
    assert!(peer.nodes.is_some(), "the broadcast carries the id delta too");
    let edit = Op {
        op_id: OpId::new("bob", 1),
        base_version: 2,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new(new_node.story.clone(), new_node.id, 0),
        payload: json!({"op": "insert_text", "text": "!"}),
        history: None,
    };
    assert!(send_op(&mut bob, edit).await);
    assert_eq!(next_applied(&mut bob).await.version, 3, "bob's own answer");
    assert_eq!(next_applied(&mut alice).await.version, 3, "alice applies it too");
}

#[tokio::test]
async fn the_transport_client_edits_through_the_server() {
    use wordcraft_collab_ws::{Event, TransportEvent};

    let (url, cfg) = spawn_server().await;
    let token = token(&cfg, "alice", "editor", "wordcraft", 600);
    let mut handle = wordcraft_collab_ws::spawn(url, token, "alice");

    let mut joined = false;
    let mut nodes = None;
    while !joined || nodes.is_none() {
        match timeout(Duration::from_secs(10), handle.events.recv()).await.expect("an event in time") {
            Some(TransportEvent::Event(Event::Welcome { .. })) => joined = true,
            Some(TransportEvent::Event(Event::Nodes { nodes: list })) => nodes = Some(list),
            Some(_) => {}
            None => panic!("the transport ended before the session joined"),
        }
    }
    let node = nodes.expect("nodes").iter().find(|node| node.story == "body").expect("a body node").id;
    handle.submit(OpKind::Primitive, Target::new("body", node, 0), json!({"op": "insert_text", "text": "Hi"})).expect("submit");

    loop {
        match timeout(Duration::from_secs(10), handle.events.recv()).await.expect("an answer in time") {
            Some(TransportEvent::Event(Event::Applied { applied })) => {
                assert_eq!(applied.version, 1);
                break;
            }
            Some(TransportEvent::Event(Event::Rejected { reject })) => panic!("refused: {reject:?}"),
            Some(_) => {}
            None => panic!("the transport ended before the answer"),
        }
    }
}

fn committed_text(cfg: &Config) -> String {
    let bytes = std::fs::read(cfg.data_dir.join("doc-1").join("source.docx")).expect("a committed file");
    let doc = wordcraft_docx::read_with_password(&bytes, None).expect("read the committed docx");
    doc.body.iter().filter_map(|block| block.as_para()).map(|para| para.text.clone()).collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn undo_and_redo_over_the_socket_change_the_committed_text() {
    let (url, cfg) = spawn_server().await;
    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let _ = next_msg(&mut owner).await;
    let node = node_of(&mut owner).await;

    owner
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("owner", 1, 0, node, 0, "Hello") }).expect("json").into()))
        .await
        .expect("insert");
    match next_msg(&mut owner).await {
        ServerMsg::Applied { applied } => assert_eq!(applied.version, 1),
        other => panic!("expected applied, got {other:?}"),
    }

    let undo = Op {
        op_id: OpId::new("owner", 2),
        base_version: 1,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", node, 0),
        payload: json!({}),
        history: Some(HistoryAction::Undo { op_id: OpId::new("owner", 1) }),
    };
    owner.send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: undo }).expect("json").into())).await.expect("undo");
    match next_msg(&mut owner).await {
        ServerMsg::Applied { applied } => assert_eq!(applied.version, 2),
        other => panic!("expected applied, got {other:?}"),
    }

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: OpId::new("owner", 3), expected_version: 2, etag: None }).expect("json").into(),
        ))
        .await
        .expect("commit");
    match next_msg(&mut owner).await {
        ServerMsg::Committed { version, .. } => assert_eq!(version, 2),
        other => panic!("expected committed, got {other:?}"),
    }
    assert_eq!(committed_text(&cfg), "", "the undo removed the typed text from the file");

    let redo = Op {
        op_id: OpId::new("owner", 4),
        base_version: 2,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", node, 0),
        payload: json!({}),
        history: Some(HistoryAction::Redo { op_id: OpId::new("owner", 1) }),
    };
    owner.send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: redo }).expect("json").into())).await.expect("redo");
    match next_msg(&mut owner).await {
        ServerMsg::Applied { applied } => assert_eq!(applied.version, 3),
        other => panic!("expected applied, got {other:?}"),
    }

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: OpId::new("owner", 5), expected_version: 3, etag: None }).expect("json").into(),
        ))
        .await
        .expect("commit");
    match next_msg(&mut owner).await {
        ServerMsg::Committed { version, .. } => assert_eq!(version, 3),
        other => panic!("expected committed, got {other:?}"),
    }
    assert_eq!(committed_text(&cfg), "Hello", "the redo brought the text back");
    let _ = std::fs::remove_dir_all(&cfg.data_dir);
}

#[tokio::test]
async fn comments_and_tracked_changes_keep_the_jwt_actor() {
    let (url, cfg) = spawn_server().await;
    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let _ = next_msg(&mut owner).await;
    let node = node_of(&mut owner).await;

    let insert = Op {
        op_id: OpId::new("owner", 1),
        base_version: 0,
        epoch: None,
        kind: OpKind::Primitive,
        target: Target::new("body", node, 0),
        payload: json!({"op": "insert_text", "text": "Hello"}),
        history: None,
    };
    assert!(send_op(&mut owner, insert).await);
    assert_eq!(next_applied(&mut owner).await.version, 1);

    let selection = json!({"anchor": {"nodeId": node, "off": 0}, "focus": {"nodeId": node, "off": 5}});
    let command = |seq: u64, base: u64, command: &str, params: serde_json::Value| Op {
        op_id: OpId::new("owner", seq),
        base_version: base,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", node, 0),
        payload: json!({"command": command, "params": params, "selection": selection}),
        history: None,
    };
    assert!(send_op(&mut owner, command(2, 1, "review.trackChanges", json!({"value": true}))).await);
    assert_eq!(next_applied(&mut owner).await.version, 2);
    assert!(send_op(&mut owner, command(3, 2, "review.newComment", json!({"text": "please check"}))).await);
    assert_eq!(next_applied(&mut owner).await.version, 3);

    let typed = Op {
        op_id: OpId::new("owner", 4),
        base_version: 3,
        epoch: None,
        kind: OpKind::Command,
        target: Target::new("body", node, 0),
        payload: json!({"command": "text.insert", "params": {"text": "X"}, "selection": {"anchor": {"nodeId": node, "off": 0}, "focus": {"nodeId": node, "off": 0}}}),
        history: None,
    };
    assert!(send_op(&mut owner, typed).await);
    assert_eq!(next_applied(&mut owner).await.version, 4);

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: OpId::new("owner", 5), expected_version: 4, etag: None }).expect("json").into(),
        ))
        .await
        .expect("commit");
    assert_eq!(next_committed(&mut owner).await.0, 4);

    let bytes = std::fs::read(cfg.data_dir.join("doc-1").join("source.docx")).expect("the committed file");
    let doc = wordcraft_docx::read_with_password(&bytes, None).expect("read the committed docx");
    assert!(doc.comments.values().any(|comment| comment.author == "owner"), "the comment carries the JWT sub: {:?}", doc.comments);
    assert!(
        doc.revisions.iter().any(|revision| revision.author == "owner" && matches!(revision.kind, wordcraft_doc::RevisionKind::Insert)),
        "the tracked insertion carries the JWT sub: {:?}",
        doc.revisions
    );
    let _ = std::fs::remove_dir_all(&cfg.data_dir);
}

#[tokio::test]
async fn an_expired_session_is_refused_on_the_next_operation() {
    let (url, cfg) = spawn_server().await;
    let mut editor = connect_ttl(&url, &cfg, "editor", "alice", 1).await;
    let _ = next_msg(&mut editor).await;
    let node = node_of(&mut editor).await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    editor
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("alice", 1, 0, node, 0, "late") }).expect("json").into()))
        .await
        .expect("op");
    match next_msg(&mut editor).await {
        ServerMsg::Rejected { reject } => assert_eq!(reject.code, wordcraft_collab::wire::RejectCode::TokenExpired),
        other => panic!("expected tokenExpired, got {other:?}"),
    }
}

#[test]
fn a_silent_session_loses_the_lease_after_its_ttl() {
    let mut cfg = dev_config();
    cfg.lease_ttl_secs = 0;
    let state = AppState::new(cfg);
    let room = state.room_for("doc-lease-ttl");
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    assert!(super::register(&room, "alice", "sid-1", wordcraft_collab::room::Role::Editor, &tx, None).is_ok());
    // The only peer of sid-1 disconnected, and the TTL is zero.
    room.lock().expect("room lock").peers.remove("alice");
    assert!(super::register(&room, "bob", "sid-2", wordcraft_collab::room::Role::Editor, &tx, None).is_ok(), "a stale lease is taken over");
    assert_eq!(room.lock().expect("room lock").session, "sid-2");
}

/// A fresh data directory per test: rooms are restored from disk, so two tests
/// sharing `dev_config`'s directory would see each other's snapshots.
fn config_for(name: &str) -> Config {
    let dir = std::env::temp_dir().join(format!("wordcraft-collab-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Config { bind: "127.0.0.1:0".into(), jwt_secret: Some("test-secret".into()), jwt_pem: None, data_dir: dir, lease_ttl_secs: 120, cabinet: None }
}

/// The M87 path with two clients: the room posts the built `.docx` to the
/// cabinet, and both peers see the cabinet's version and etag, not a local
/// digest pretending to be a stored version. The second commit carries the
/// etag the first answer named.
#[tokio::test]
async fn a_commit_is_published_to_the_cabinet_and_every_client_sees_its_version() {
    let first = cabinet_reply(r#"{"version":1794186677,"etag":"1794186677"}"#);
    let second = cabinet_reply(r#"{"version":1794186678,"etag":"1794186678"}"#);
    let (cabinet_url, mut requests) = fake_cabinet(vec![Some(first), Some(second)]).await;
    let cfg = config_with_cabinet(&config_for("cabinet-publish"), &cabinet_url);
    let url = spawn_server_with(cfg.clone()).await;

    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let mut editor = connect(&url, &cfg, "editor", "editor").await;
    let _ = next_msg(&mut owner).await;
    let node = node_of(&mut owner).await;
    let _ = next_msg(&mut editor).await;
    let _ = node_of(&mut editor).await;

    owner
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("owner", 1, 0, node, 0, "Cabinet") }).expect("json").into()))
        .await
        .expect("insert");
    let _ = next_msg(&mut owner).await;
    let _ = next_msg(&mut editor).await;

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: OpId::new("owner", 2), expected_version: 1, etag: None }).expect("json").into(),
        ))
        .await
        .expect("commit");
    let (version, cabinet) = next_committed_cabinet(&mut owner).await;
    assert_eq!(version, 1, "the room version is unchanged by the publication");
    assert_eq!(
        cabinet,
        Some(wordcraft_collab::wire::CabinetCommit { version: 1_794_186_677, etag: "1794186677".into() }),
        "the panel shows the cabinet's copy"
    );
    let (_, peer) = next_committed_cabinet(&mut editor).await;
    assert_eq!(peer, cabinet, "every subscriber sees the cabinet's version");

    let request = requests.recv().await.expect("the cabinet was called");
    assert!(request.starts_with("POST /api/v1/internal/office/sessions/session-1/commit"), "{request}");
    assert!(request.contains("Authorization: Bearer service-token"), "{request}");
    assert!(request.contains(r#""fileId":"doc-1""#), "{request}");
    assert!(request.contains(r#""version":1"#), "{request}");
    assert!(!request.contains("expectedVersion"), "the first commit of a session expects nothing: {request}");

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: OpId::new("owner", 3), expected_version: 1, etag: None }).expect("json").into(),
        ))
        .await
        .expect("commit again");
    let (_, again) = next_committed_cabinet(&mut owner).await;
    assert_eq!(again.map(|commit| commit.etag), Some("1794186678".to_string()));
    let second = requests.recv().await.expect("the cabinet was called again");
    assert!(second.contains(r#""expectedVersion":"1794186677""#), "the etag of the previous answer is the expectation: {second}");
    let _ = std::fs::remove_dir_all(&cfg.data_dir);
}

/// `409`: the file changed outside the session. Both clients resync (a
/// retryable error), and nothing advances the room's expected etag.
#[tokio::test]
async fn a_cabinet_conflict_resyncs_every_client() {
    let (cabinet_url, mut requests) = fake_cabinet(vec![Some("HTTP/1.1 409 Conflict\r\nContent-Length: 2\r\n\r\n{}")]).await;
    let cfg = config_with_cabinet(&config_for("cabinet-conflict"), &cabinet_url);
    let url = spawn_server_with(cfg.clone()).await;

    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let mut editor = connect(&url, &cfg, "editor", "editor").await;
    let _ = next_msg(&mut owner).await;
    let node = node_of(&mut owner).await;
    let _ = next_msg(&mut editor).await;
    let _ = node_of(&mut editor).await;
    owner
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("owner", 1, 0, node, 0, "Edited") }).expect("json").into()))
        .await
        .expect("insert");
    let _ = next_msg(&mut owner).await;
    let _ = next_msg(&mut editor).await;

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: OpId::new("owner", 2), expected_version: 1, etag: None }).expect("json").into(),
        ))
        .await
        .expect("commit");

    for socket in [&mut owner, &mut editor] {
        let notice = loop {
            match next_msg(socket).await {
                ServerMsg::Error { code, message, retryable } => break (code, message, retryable),
                ServerMsg::Applied { .. } | ServerMsg::Nodes { .. } | ServerMsg::Presence { .. } | ServerMsg::Left { .. } => continue,
                other => panic!("expected a resync notice, got {other:?}"),
            }
        };
        assert_eq!(notice.0, "cabinetConflict");
        assert!(notice.2, "the client resyncs: {}", notice.1);
    }
    let _ = requests.recv().await.expect("one request");
    assert!(requests.try_recv().is_err(), "409 is not retried");
    let _ = std::fs::remove_dir_all(&cfg.data_dir);
}

/// TASK.md §6: after a `409` the owner saves the file the room built as a
/// copy. The bytes go to the owner only, the cabinet is not written again,
/// and an editor cannot ask for them.
#[tokio::test]
async fn a_refused_commit_can_be_saved_as_a_copy_without_overwriting_the_cabinet() {
    use sha2::Digest;
    let (cabinet_url, mut requests) = fake_cabinet(vec![Some("HTTP/1.1 409 Conflict\r\nContent-Length: 2\r\n\r\n{}")]).await;
    let cfg = config_with_cabinet(&config_for("cabinet-copy"), &cabinet_url);
    let url = spawn_server_with(cfg.clone()).await;

    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let mut editor = connect(&url, &cfg, "editor", "editor").await;
    let _ = next_msg(&mut owner).await;
    let node = node_of(&mut owner).await;
    let _ = next_msg(&mut editor).await;
    let _ = node_of(&mut editor).await;
    owner
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("owner", 1, 0, node, 0, "Save me") }).expect("json").into()))
        .await
        .expect("insert");
    let _ = next_msg(&mut owner).await;
    let _ = next_msg(&mut editor).await;

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: OpId::new("owner", 2), expected_version: 1, etag: None }).expect("json").into(),
        ))
        .await
        .expect("commit");
    for socket in [&mut owner, &mut editor] {
        loop {
            match next_msg(socket).await {
                ServerMsg::Error { code, .. } => {
                    assert_eq!(code, "cabinetConflict");
                    break;
                }
                ServerMsg::Applied { .. } | ServerMsg::Nodes { .. } | ServerMsg::Presence { .. } | ServerMsg::Left { .. } => continue,
                other => panic!("expected a conflict notice, got {other:?}"),
            }
        }
    }
    let _ = requests.recv().await.expect("the commit reached the cabinet once");

    // An editor may not take the file: it is the owner's commit to save.
    editor
        .send(Message::Text(serde_json::to_string(&ClientMsg::SaveCopy { op_id: OpId::new("editor", 2) }).expect("json").into()))
        .await
        .expect("saveCopy");
    let reject = loop {
        match next_msg(&mut editor).await {
            ServerMsg::Rejected { reject } => break reject,
            ServerMsg::Applied { .. } | ServerMsg::Nodes { .. } | ServerMsg::Presence { .. } | ServerMsg::Left { .. } => continue,
            other => panic!("expected a rejection, got {other:?}"),
        }
    };
    assert_eq!(reject.code, wordcraft_collab::wire::RejectCode::Forbidden);

    // The owner gets the room's file, chunk after chunk, and the cabinet gets
    // no second request: a copy never overwrites it.
    owner
        .send(Message::Text(serde_json::to_string(&ClientMsg::SaveCopy { op_id: OpId::new("owner", 3) }).expect("json").into()))
        .await
        .expect("saveCopy");
    let mut bytes = Vec::new();
    let mut sha: Option<String> = None;
    loop {
        match next_msg(&mut owner).await {
            ServerMsg::SaveCopy { version, sha256, bytes: chunk, last, .. } => {
                assert_eq!(version, 1);
                sha.get_or_insert(sha256);
                bytes.extend_from_slice(&chunk);
                if last {
                    break;
                }
            }
            ServerMsg::Applied { .. } | ServerMsg::Nodes { .. } | ServerMsg::Presence { .. } | ServerMsg::Left { .. } => continue,
            other => panic!("expected a copy chunk, got {other:?}"),
        }
    }
    assert_eq!(&bytes[..4], b"PK\x03\x04", "the copy is a .docx the room built");
    let sha = sha.expect("every chunk names the digest");
    assert_eq!(sha, crate::hex(&sha2::Sha256::digest(&bytes)), "the copy's digest matches its bytes");
    assert!(requests.try_recv().is_err(), "saving a copy never posts to the cabinet");
    let _ = std::fs::remove_dir_all(&cfg.data_dir);
}

/// Without a refused commit there is nothing to copy: the request is refused
/// instead of building a file behind the owner's back.
#[tokio::test]
async fn a_copy_without_a_refused_commit_is_refused() {
    let cfg = config_for("cabinet-no-copy");
    let url = spawn_server_with(cfg.clone()).await;
    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let _ = next_msg(&mut owner).await;
    let _ = node_of(&mut owner).await;
    owner
        .send(Message::Text(serde_json::to_string(&ClientMsg::SaveCopy { op_id: OpId::new("owner", 1) }).expect("json").into()))
        .await
        .expect("saveCopy");
    let reject = loop {
        match next_msg(&mut owner).await {
            ServerMsg::Rejected { reject } => break reject,
            ServerMsg::Applied { .. } | ServerMsg::Nodes { .. } | ServerMsg::Presence { .. } | ServerMsg::Left { .. } => continue,
            other => panic!("expected a rejection, got {other:?}"),
        }
    };
    assert_eq!(reject.code, wordcraft_collab::wire::RejectCode::Conflict);
    let _ = std::fs::remove_dir_all(&cfg.data_dir);
}

/// `401`: a deployment error. The owner is told once, without a retry, and no
/// other peer is disturbed.
#[tokio::test]
async fn a_refused_cabinet_token_is_reported_once_without_a_retry() {
    let (cabinet_url, mut requests) = fake_cabinet(vec![Some("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")]).await;
    let cfg = config_with_cabinet(&config_for("cabinet-token"), &cabinet_url);
    let url = spawn_server_with(cfg.clone()).await;

    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let _ = next_msg(&mut owner).await;
    let node = node_of(&mut owner).await;
    owner
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("owner", 1, 0, node, 0, "x") }).expect("json").into()))
        .await
        .expect("insert");
    let _ = next_msg(&mut owner).await;

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: OpId::new("owner", 2), expected_version: 1, etag: None }).expect("json").into(),
        ))
        .await
        .expect("commit");
    let reject = loop {
        match next_msg(&mut owner).await {
            ServerMsg::Rejected { reject } => break reject,
            ServerMsg::Applied { .. } => continue,
            other => panic!("expected a rejection, got {other:?}"),
        }
    };
    assert_eq!(reject.code, wordcraft_collab::wire::RejectCode::Forbidden);
    assert!(reject.message.contains("token"), "{}", reject.message);
    let request = requests.recv().await.expect("one request");
    let request_line = request.lines().next().unwrap_or_default();
    assert!(!request_line.contains("service-token"), "the token never goes in the URL: {request_line}");
    assert!(requests.try_recv().is_err(), "401 gets exactly one attempt");
    let _ = std::fs::remove_dir_all(&cfg.data_dir);
}

/// A network failure: the first answer is lost, the retry carries the same body
/// (same `expectedVersion`, same digest) and both clients see the stored copy.
#[tokio::test]
async fn a_lost_cabinet_answer_is_retried_with_the_same_body() {
    let ok = cabinet_reply(r#"{"version":1794186677,"etag":"1794186677"}"#);
    let (cabinet_url, mut requests) = fake_cabinet(vec![None, Some(ok)]).await;
    let cfg = config_with_cabinet(&config_for("cabinet-retry"), &cabinet_url);
    let url = spawn_server_with(cfg.clone()).await;

    let mut owner = connect(&url, &cfg, "owner", "owner").await;
    let mut editor = connect(&url, &cfg, "editor", "editor").await;
    let _ = next_msg(&mut owner).await;
    let node = node_of(&mut owner).await;
    let _ = next_msg(&mut editor).await;
    let _ = node_of(&mut editor).await;
    owner
        .send(Message::Text(serde_json::to_string(&ClientMsg::Op { op: insert("owner", 1, 0, node, 0, "Retry") }).expect("json").into()))
        .await
        .expect("insert");
    let _ = next_msg(&mut owner).await;
    let _ = next_msg(&mut editor).await;

    owner
        .send(Message::Text(
            serde_json::to_string(&ClientMsg::Commit { op_id: OpId::new("owner", 2), expected_version: 1, etag: None }).expect("json").into(),
        ))
        .await
        .expect("commit");
    let (_, cabinet) = next_committed_cabinet(&mut owner).await;
    assert_eq!(cabinet.map(|commit| commit.etag), Some("1794186677".to_string()));
    let (_, peer) = next_committed_cabinet(&mut editor).await;
    assert!(peer.is_some(), "the peer sees the stored version");

    let first = requests.recv().await.expect("first attempt");
    let second = requests.recv().await.expect("second attempt");
    let body_of = |request: &str| request.split_once("\r\n\r\n").map(|(_, body)| body.to_string()).unwrap_or_default();
    assert_eq!(body_of(&first), body_of(&second), "a retry is the same body, so it cannot become a second version");
    let _ = std::fs::remove_dir_all(&cfg.data_dir);
}
