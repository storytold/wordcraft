//! A socket-level reconnect: the server drops the connection after receiving
//! an operation, the client reconnects, sends `auth`, and replays the
//! unconfirmed operation with the same `opId` (ADR-0003). The server answers
//! the duplicate with the applied result.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use wordcraft_collab::wire::{Applied, ClientMsg, OpKind, ServerMsg, Target};
use wordcraft_collab_ws::{Event, TransportEvent, spawn};

const WAIT: Duration = Duration::from_secs(10);

async fn next_client_msg(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> ClientMsg {
    let message = timeout(WAIT, socket.next()).await.expect("a client frame").expect("a frame").expect("a text frame");
    let Message::Text(text) = message else { panic!("expected text, got {message:?}") };
    serde_json::from_str(&text).expect("a protocol message")
}

async fn send_server_msg(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>, msg: &ServerMsg) {
    let text = serde_json::to_string(msg).expect("json");
    socket.send(Message::Text(text.into())).await.expect("send");
}

async fn welcome(socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>, last_version: Option<u64>) {
    match next_client_msg(socket).await {
        ClientMsg::Auth { last_version: seen, .. } => assert_eq!(seen, last_version),
        other => panic!("expected auth, got {other:?}"),
    }
    send_server_msg(
        socket,
        &ServerMsg::Welcome { session_id: "s1".into(), role: "editor".into(), version: 0, epoch: 0, participants: Vec::new(), resync: false },
    )
    .await;
}

#[tokio::test]
async fn an_unconfirmed_operation_is_resent_after_a_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");

    let server = tokio::spawn(async move {
        // First connection: welcome, take one operation, drop the socket
        // before answering it.
        let (stream, _) = listener.accept().await.expect("first connection");
        let mut socket = tokio_tungstenite::accept_async(stream).await.expect("handshake");
        welcome(&mut socket, None).await;
        let first = match next_client_msg(&mut socket).await {
            ClientMsg::Op { op } => op,
            other => panic!("expected op, got {other:?}"),
        };
        drop(socket);

        // Second connection: the same operation arrives again.
        let (stream, _) = listener.accept().await.expect("second connection");
        let mut socket = tokio_tungstenite::accept_async(stream).await.expect("handshake");
        welcome(&mut socket, None).await;
        let second = match next_client_msg(&mut socket).await {
            ClientMsg::Op { op } => op,
            other => panic!("expected resent op, got {other:?}"),
        };
        assert_eq!(second.op_id, first.op_id, "the client reuses the operation id (ADR-0003)");
        assert_eq!(second.payload, first.payload);
        send_server_msg(
            &mut socket,
            &ServerMsg::Applied { applied: Applied { op_id: second.op_id.clone(), version: 1, epoch: 0, delta: None, conflict: false } },
        )
        .await;
        // Keep the socket open until the client has read the answer.
        let _ = timeout(Duration::from_secs(1), socket.next()).await;
        (first, second)
    });

    let mut handle = spawn(format!("ws://{addr}"), "test-token", "alice");
    let event = timeout(WAIT, handle.events.recv()).await.expect("first welcome").expect("event");
    assert!(matches!(event, TransportEvent::Event(Event::Welcome { version: 0, .. })), "{event:?}");

    handle.submit(OpKind::Primitive, Target::new("body", 7, 0), json!({"text": "Hi"})).expect("submit");

    // The server dropped the socket without answering, so the operation is
    // still unconfirmed and the transport reports the drop.
    let event = timeout(WAIT, handle.events.recv()).await.expect("disconnect").expect("event");
    assert!(matches!(event, TransportEvent::Disconnected { .. }), "{event:?}");

    let event = timeout(WAIT, handle.events.recv()).await.expect("second welcome").expect("event");
    assert!(matches!(event, TransportEvent::Event(Event::Welcome { .. })), "{event:?}");

    let event = timeout(WAIT, handle.events.recv()).await.expect("applied").expect("event");
    let TransportEvent::Event(Event::Applied { applied }) = event else { panic!("expected applied, got {event:?}") };
    assert_eq!(applied.version, 1);

    let (first, second) = server.await.expect("server task");
    assert_eq!(first.target, Target::new("body", 7, 0));
    assert_eq!(second.kind, OpKind::Primitive);
}
