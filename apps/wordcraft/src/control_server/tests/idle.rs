//! Real loopback clients exercising ownership-aware idle and absolute frame limits.

use super::*;
use wordcraft_ui_egui::{WordApp, ask_agent::Status};

/// Build a real document and claim its native selection over TCP before measuring idle time.
fn owned_client(period: Duration) -> (Conn, Receiver<ControlRequest>, AgentHub, WordApp, u64) {
    let (port, rx, hub) = boot_agents(period);
    let mut app = WordApp::new(wordcraft_engine::Session::new(wordcraft_doc::Document::new()), Default::default());
    app.ask_agent.hub = Some(hub.clone());
    app.run("text.insert", json!({"text":"Original selection"})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let mut client = authed(port);
    assert_eq!(client.call(r#"{"method":"agent.listen","params":{"timeoutMs":0,"agent":"Idle owner"}}"#)["result"]["none"], true);
    wordcraft_ui_egui::ask_agent::command(&mut app, &json!({"instruction":"Improve the selection"})).unwrap();
    let claimed = client.call(r#"{"method":"agent.listen","params":{"timeoutMs":0}}"#);
    let id = claimed["result"]["id"].as_u64().unwrap();
    (client, rx, hub, app, id)
}

/// Model the real window's serialized hop: native edits and replies run on this test thread.
fn dispatch(rx: &Receiver<ControlRequest>, app: &mut WordApp) {
    let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let wordcraft_ui_egui::control::Outcome::Done(reply) = wordcraft_ui_egui::control::handle(app, &egui::Context::default(), &request) else {
        panic!("expected an immediate native control response")
    };
    request.reply.send(reply).unwrap();
}

/// Poll only state transitions, with a generous outer limit rather than a timing-sensitive sleep.
fn wait_requeued(hub: &AgentHub, id: u64) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if hub.snapshot(id).unwrap().status == Status::Queued && hub.agents().is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(hub.snapshot(id).unwrap().status, Status::Queued, "a dead or incomplete client still owns the request");
    assert!(hub.agents().is_empty(), "the dead client retained its listener slot");
}

/// Thinking is not disconnection: preserve a live owner across several idle periods, including
/// a non-final progress reply, then accept two real scoped edits and a final answer.
#[test]
fn review_idle_live_owner_can_think_progress_edit_and_finish() {
    let period = Duration::from_millis(250);
    let (mut client, rx, hub, mut app, id) = owned_client(period);
    std::thread::sleep(period * 3);
    assert!(matches!(hub.snapshot(id).unwrap().status, Status::Working { .. }), "idle thinking disconnected a live working owner");
    client.send(&json!({"method":"agent.reply","params":{"id":id,"text":"Thinking","done":false}}).to_string());
    dispatch(&rx, &mut app);
    assert_eq!(client.recv()["result"]["done"], false);
    std::thread::sleep(period * 3);
    assert!(matches!(hub.snapshot(id).unwrap().status, Status::Working { .. }), "done:false released the live owner's work");
    for text in ["First real edit", "Second real edit"] {
        if text == "Second real edit" {
            client.send(
                &json!({"method":"engine.execute","params":{"command":"select.text","params":{"text":"First real edit"},"request":id}}).to_string(),
            );
            dispatch(&rx, &mut app);
            assert_eq!(client.recv()["ok"], true);
        }
        client.send(&json!({"method":"engine.execute","params":{"command":"text.insert","params":{"text":text},"request":id}}).to_string());
        dispatch(&rx, &mut app);
        let response = client.recv();
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), text);
    }
    client.send(&json!({"method":"agent.reply","params":{"id":id,"text":"Finished"}}).to_string());
    dispatch(&rx, &mut app);
    assert_eq!(client.recv()["result"]["done"], true);
    assert_eq!(hub.snapshot(id).unwrap().status, Status::Answered);
    assert!(client.closed(), "an answered request must not extend idle life");
}

/// A complete split UTF-8 frame remains intact after idle renewal; no retry may drop its prefix.
#[test]
fn review_idle_owner_preserves_a_split_utf8_native_command() {
    let period = Duration::from_millis(300);
    let (mut client, rx, hub, mut app, id) = owned_client(period);
    std::thread::sleep(period * 3);
    assert!(matches!(hub.snapshot(id).unwrap().status, Status::Working { .. }), "owner expired before its split reply");
    let line = json!({"method":"engine.execute","params":{"command":"text.insert","params":{"text":"Café 🌍"},"request":id}}).to_string();
    let split = line.find('🌍').unwrap() + 1;
    client.1.write_all(&line.as_bytes()[..split]).unwrap();
    std::thread::sleep(period / 3);
    client.1.write_all(&line.as_bytes()[split..]).unwrap();
    client.1.write_all(b"\n").unwrap();
    dispatch(&rx, &mut app);
    assert_eq!(client.recv()["ok"], true);
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Café 🌍");
}

/// Owning a request cannot turn an incomplete, slowly supplied frame into an infinite lease.
#[test]
fn review_idle_owner_partial_line_has_one_absolute_deadline() {
    let period = Duration::from_millis(250);
    for drip in [false, true] {
        let (mut client, rx, hub, _app, id) = owned_client(period);
        client.1.write_all(b"{\"method\":").unwrap();
        let start = Instant::now();
        if drip {
            while start.elapsed() < period * 3 {
                if client.1.write_all(b" ").is_err() {
                    break;
                }
                std::thread::sleep(period / 5);
            }
        } else {
            std::thread::sleep(period * 2);
        }
        wait_requeued(&hub, id);
        assert!(rx.try_recv().is_err(), "a partial frame reached native dispatch");
        assert!(client.closed(), "incomplete owner frame stayed open");
    }
}

/// Real EOF immediately releases ownership rather than waiting for the next reflection period.
#[test]
fn review_idle_owner_eof_requeues_and_releases_its_listener() {
    let (client, _rx, hub, _app, id) = owned_client(Duration::from_secs(10));
    drop(client);
    wait_requeued(&hub, id);
}

/// The owner exemption is not granted to another authenticated listener on the same server.
#[test]
fn review_idle_nonowner_expires_while_the_owner_stays_live() {
    let period = Duration::from_millis(250);
    let (mut owner, _rx, hub, _app, id) = owned_client(period);
    let port = owner.1.peer_addr().unwrap().port();
    let mut unused = authed(port);
    assert_eq!(unused.call(r#"{"method":"agent.listen","params":{"timeoutMs":0,"agent":"Unused"}}"#)["result"]["none"], true);
    assert!(unused.closed(), "non-owner listener got an indefinite lease");
    assert!(matches!(hub.snapshot(id).unwrap().status, Status::Working { .. }), "the separate owner expired too");
    assert_eq!(hub.agents(), ["Idle owner"]);
    owner.send(r#"{"method":"auth"}"#);
    assert_eq!(owner.recv()["ok"], true);
}

/// Blank lines are not authenticated requests and must not renew the first-request deadline.
#[test]
fn review_idle_blank_lines_do_not_keep_an_unauthenticated_connection_alive() {
    let period = Duration::from_millis(250);
    let (port, _rx, _) = boot_agents(period);
    let mut client = Conn::open(port);
    let mut writer = client.1.try_clone().unwrap();
    let drip = std::thread::spawn(move || {
        for _ in 0..20 {
            if writer.write_all(b"\n").is_err() {
                break;
            }
            std::thread::sleep(period / 5);
        }
    });
    let start = Instant::now();
    let mut response = String::new();
    let closed = client.0.read_line(&mut response);
    // Closing with unread blank bytes may produce TCP reset instead of clean EOF. Both are
    // terminal; a receive timeout or actual response is still a failure of the idle bound.
    assert!(
        match &closed {
            Ok(0) => true,
            Err(error) =>
                matches!(error.kind(), std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::BrokenPipe),
            _ => false,
        },
        "unused connection did not terminate: {closed:?}"
    );
    assert!(start.elapsed() < period * 3, "blank lines renewed an unauthenticated connection");
    drip.join().unwrap();
}

/// Frame validation and the 1 MiB cap apply equally to authenticated owners and normal clients.
#[test]
fn review_idle_malformed_utf8_json_and_oversize_lines_are_refused() {
    for payload in [b"\xff\n".to_vec(), b"{not JSON}\n".to_vec(), vec![b'x'; MAX_LINE + 1]] {
        let (mut client, _rx, hub, _app, id) = owned_client(Duration::from_millis(250));
        client.1.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        client.1.write_all(&payload).unwrap();
        if payload.len() > MAX_LINE {
            client.1.write_all(b"\n").unwrap();
        }
        let response = client.recv();
        assert_eq!(response["ok"], false);
        let expected = if payload.len() > MAX_LINE {
            "line too long"
        } else if payload[0] == 255 {
            "not UTF-8"
        } else {
            "bad JSON"
        };
        assert!(response["error"].as_str().unwrap().contains(expected), "{response}");
        wait_requeued(&hub, id);
        assert!(client.closed());
    }
}

/// Exercise exhaustion in a separate test process, so changing its private counter cannot race
/// with any other TCP fixture. The final allocatable id is unique; MAX itself is refused.
#[test]
fn review_idle_connection_ids_stop_at_exhaustion() {
    const CHILD: &str = "WORDCRAFT_TEST_CONNECTION_EXHAUSTION";
    if std::env::var(CHILD).as_deref() != Ok("1") {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "control_server::tests::idle::review_idle_connection_ids_stop_at_exhaustion", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"), "the exhaustion subprocess ran zero tests");
        return;
    }
    NEXT_CONN.store(u64::MAX - 1, Ordering::SeqCst);
    let (port, rx) = boot();
    let mut first = authed(port);
    first.send(r#"{"method":"document.inspect"}"#);
    assert_eq!(answer(&rx).connection, Some(u64::MAX - 1));
    assert_eq!(first.recv()["ok"], true);
    let mut exhausted = Conn::open(port);
    assert!(exhausted.closed(), "an exhausted server accepted a reused id");
    assert_eq!(NEXT_CONN.load(Ordering::SeqCst), u64::MAX);
}

/// A socket can own more than one request: completing one must not revoke another's idle grace.
#[test]
fn review_idle_any_remaining_owned_work_keeps_the_connection_live() {
    let period = Duration::from_millis(250);
    let (mut client, rx, hub, mut app, first) = owned_client(period);
    let second = hub
        .ask(Request {
            id: 0,
            instruction: "Another window's work".into(),
            history: vec![],
            selection: json!({"anchor":app.session.sel.anchor,"focus":app.session.sel.focus,"text":app.session.selected_text()}),
            document: Value::Null,
            target: Target { document: app.session.document_id().to_string(), revision: app.session.rev() },
            agent: None,
        })
        .unwrap();
    assert_eq!(client.call(r#"{"method":"agent.listen","params":{"timeoutMs":0}}"#)["result"]["id"], second);
    client.send(&json!({"method":"agent.reply","params":{"id":first,"text":"First done"}}).to_string());
    dispatch(&rx, &mut app);
    assert_eq!(client.recv()["result"]["done"], true);
    assert_eq!(hub.snapshot(first).unwrap().status, Status::Answered);
    std::thread::sleep(period * 3);
    assert!(matches!(hub.snapshot(second).unwrap().status, Status::Working { .. }), "finishing one request revoked another's live ownership");
    hub.close(second);
    assert!(client.closed(), "closing the last owned request must restore idle expiry");
}

/// A partial first frame cannot buy extra time for an unauthenticated or idle non-owner peer.
#[test]
fn review_frame_nonowners_retain_the_original_idle_deadline() {
    let period = Duration::from_millis(800);
    for authenticated in [false, true] {
        let (port, _rx, _hub) = boot_agents(period);
        let mut client = if authenticated { authed(port) } else { Conn::open(port) };
        let start = Instant::now();
        std::thread::sleep(period * 3 / 4);
        client.1.write_all(b"{\"method\":\"auth\"").unwrap();
        assert!(client.closed(), "an incomplete non-owner frame stayed open");
        assert!(start.elapsed() < period * 3 / 2, "first byte extended a non-owner's idle deadline, authenticated={authenticated}");
    }
}

/// A working owner gets a full, fixed frame window from its first byte, even if the preceding
/// empty wait was almost over; a valid split native command must not inherit its residual time.
#[test]
fn review_frame_owner_gets_one_full_window_after_its_first_byte() {
    let period = Duration::from_millis(800);
    let (mut client, rx, hub, mut app, id) = owned_client(period);
    let line = json!({"method":"engine.execute","params":{"command":"text.insert","params":{"text":"Within the first-byte window"},"request":id}})
        .to_string();
    let split = line.len() / 2;
    std::thread::sleep(period * 3 / 4);
    client.1.write_all(&line.as_bytes()[..split]).unwrap();
    std::thread::sleep(period / 2);
    assert!(matches!(hub.snapshot(id).unwrap().status, Status::Working { .. }), "valid partial frame lost its owner's full first-byte budget");
    client.1.write_all(&line.as_bytes()[split..]).unwrap();
    client.1.write_all(b"\n").unwrap();
    dispatch(&rx, &mut app);
    assert_eq!(client.recv()["ok"], true);
    assert_eq!(app.session.doc.plain_text(wordcraft_doc::StoryRef::Body), "Within the first-byte window");
}
