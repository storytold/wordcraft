//! craft-live mode without a browser: the framing, the fragment, and the whole client against a
//! stand-in for craft-live's room (a plain Yjs document that answers sync, relays updates, holds
//! a joiner until the seed arrives, asks for saves, as craft-live `docs/design.md` §4–§6 say).

use serde_json::{Value as Json, json};
use wordcraft_engine::Session;
use wordcraft_engine::doc::Document;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, StateVector, Transact, Update};

use super::*;

// ------------------------------------------------------------------------------------ framing

#[test]
fn frames_round_trip_in_lib0() {
    for f in [
        Frame::Sync1(vec![1, 2]),
        Frame::Sync2(vec![]),
        Frame::Update(vec![9; 300]),
        Frame::Awareness(vec![0]),
        Frame::Denied("no".into()),
        Frame::QueryAwareness,
        Frame::Control(json!({"type": "hello", "clientId": 7})),
    ] {
        assert_eq!(Frame::decode(&f.encode()), Some(f));
    }
    // y-websocket's own bytes.
    assert_eq!(Frame::Sync1(vec![7]).encode(), vec![0, 0, 1, 7]);
    assert_eq!(Frame::Update(vec![9; 200]).encode()[..4], [0, 2, 0xc8, 0x01]);
    assert_eq!(Frame::Control(json!({})).encode(), vec![100, 2, b'{', b'}']);
    // Hostile input: nothing panics, nothing malformed passes.
    assert_eq!(Frame::decode(&[]), None);
    assert_eq!(Frame::decode(&[0, 2, 5, 1]), None, "a length past the end");
    assert_eq!(Frame::decode(&[0, 3, 0]), None, "an unknown sync step");
    assert_eq!(Frame::decode(&[9]), None);
    assert_eq!(Frame::decode(&[0xff; 12]), None, "a varuint over 64 bits");
    assert_eq!(Frame::decode(&[100, 3, b'[', b'1', b']']), None, "a control message that isn't an object");
    assert_eq!(Frame::decode(&[100, 2, 0xff, 0xfe]), None, "a control message that isn't JSON");
    let mut seed = 0x1234_5678u64;
    for len in 0..400 {
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (seed >> 56) as u8
            })
            .collect();
        let _ = Frame::decode(&bytes);
    }
}

#[test]
fn varuints_and_percent_encoding() {
    for n in [0u64, 1, 127, 128, 300, 1 << 53, u64::MAX] {
        let mut b = Vec::new();
        write_var(&mut b, n);
        assert_eq!(Reader { data: &b, pos: 0 }.var(), Some(n));
    }
    assert_eq!(pct("Réunion 2026.docx"), "R%C3%A9union%202026.docx");
    assert_eq!(unpct("R%C3%A9union%202026.docx").as_deref(), Some("Réunion 2026.docx"));
    assert_eq!(unpct("a+b").as_deref(), Some("a b"));
    assert_eq!(unpct("%zz"), None);
    assert_eq!(unpct("%4"), None);
    assert_eq!(unpct("%FF"), None, "not UTF-8");
}

// ------------------------------------------------------------------------------ configuration

fn fragment(seed: bool, role: &str) -> String {
    let user = json!({"id": "ada", "name": "Ada Lovelace", "color": "#2E9E5B"}).to_string();
    format!(
        "#craft-live=1&ws={}&room={}&token=tok123&user={}&role={role}&seed={}&file={}&name={}&format=odt&api={}&lang=pt_BR",
        pct("wss://live.example.com/ws/nc%3Acloud%3A288"),
        pct("nc:cloud:288"),
        pct(&user),
        if seed { 1 } else { 0 },
        pct("https://live.example.com/api/v1/rooms/nc%3Acloud%3A288/file"),
        pct("Notes 1.odt"),
        pct("https://live.example.com/api/v1"),
    )
}

#[test]
fn the_fragment_configures_the_session() {
    let c = Config::from_fragment(&fragment(true, "edit")).unwrap().unwrap();
    assert_eq!(c.ws, "wss://live.example.com/ws/nc%3Acloud%3A288");
    assert_eq!(c.room, "nc:cloud:288");
    assert_eq!(c.token, "tok123");
    assert_eq!(c.user, json!({"id": "ada", "name": "Ada Lovelace", "color": "#2E9E5B"}));
    assert!(!c.view);
    assert!(c.seed);
    assert_eq!(c.name, "Notes 1.odt");
    assert_eq!(c.format, "odt");
    assert_eq!(c.api, "https://live.example.com/api/v1");
    assert_eq!(c.socket_url(), "wss://live.example.com/ws/nc%3Acloud%3A288?token=tok123");
    // A language the build has a catalog for (craft-i18n's, from CRAFT_I18N_DIR).
    assert_eq!(c.interface_language().as_deref(), wordcraft_ui_egui::i18n::normalize_pref("pt-br"));

    let v = Config::from_fragment(&fragment(false, "view")).unwrap().unwrap();
    assert!(v.view && !v.seed);
    // Anything but `edit` is read-only.
    assert!(Config::from_fragment(&fragment(false, "admin")).unwrap().unwrap().view);

    // Not a craft-live page.
    assert!(Config::from_fragment("").is_none());
    assert!(Config::from_fragment("#page=3").is_none());
    assert!(Config::from_fragment("#craft-live=2&ws=wss://x").is_none());
    // A craft-live page that can't be used.
    assert!(Config::from_fragment("#craft-live=1").unwrap().is_err());
    assert!(Config::from_fragment("#craft-live=1&ws=https://x&room=r&token=t").unwrap().is_err(), "not a WebSocket URL");
    assert!(Config::from_fragment("#craft-live=1&ws=wss://x&room=r&token=t&seed=1").unwrap().is_err(), "a seeder without the file's URL");
}

#[test]
fn the_shell_config_and_odd_users_work_too() {
    let c = Config::from_json(&json!({
        "ws": "ws://127.0.0.1:9/ws/r", "room": "r", "token": "t", "seed": false, "role": "edit",
        "user": {"id": "", "name": "", "color": "blue"}, "name": "A.docx", "format": ".DOCX", "api": "nope", "lang": "",
    }))
    .unwrap();
    assert!(c.user["id"].as_str().unwrap().starts_with("guest-"));
    // Random hex, never made from the secret token (every peer sees the id).
    let suffix = c.user["id"].as_str().unwrap().strip_prefix("guest-").unwrap();
    assert!(suffix.len() == 8 && suffix.chars().all(|ch| ch.is_ascii_hexdigit()), "{suffix}");
    assert_eq!(c.user["name"], c.user["id"]);
    assert!(wordcraft_engine::collab::valid_color(c.user["color"].as_str().unwrap()));
    assert_eq!(c.format, "docx");
    assert_eq!(c.api, "", "not an http URL");
    assert_eq!(c.interface_language(), None);
    let long = "x".repeat(500);
    let c = Config::from_json(&json!({"ws": "ws://h/ws/r", "room": "r", "token": "t", "user": {"id": long, "name": "a\u{0}b"}})).unwrap();
    assert_eq!(c.user["id"].as_str().unwrap().chars().count(), 128);
    assert_eq!(c.user["name"], "ab");
    for lang in ["de", "de_AT", "fr_FR"] {
        let c = Config::from_json(&json!({"ws": "ws://h/ws/r", "room": "r", "token": "t", "lang": lang})).unwrap();
        // Only languages the build has a catalog for (craft-i18n's, from CRAFT_I18N_DIR).
        let built = wordcraft_ui_egui::i18n::normalize_pref(&lang[..2]).is_some();
        assert_eq!(c.interface_language().is_some(), built, "{lang}");
    }
}

// ----------------------------------------------------------------------------- a fake room

/// One browser tab in the fake room.
struct Tab {
    live: Live,
    s: Session,
    /// Server side: connected, synced, its sync step 1 held until the seed arrives.
    connected: bool,
    synced: bool,
    held: Option<Vec<u8>>,
    client_id: Option<u64>,
    can_upload: bool,
    events: Vec<Event>,
    shell: Vec<Json>,
    status: Vec<String>,
    /// Don't answer `Fetch` (the file is slow).
    hold_fetch: bool,
    fetches: u32,
}

impl Tab {
    fn shell_types(&self) -> Vec<String> {
        self.shell.iter().filter_map(|m| m["type"].as_str().map(str::to_string)).collect()
    }
    fn text(&self) -> String {
        wordcraft_collab::canonical(&self.s.doc).to_string()
    }
    /// The body's paragraphs' text, one line each.
    fn body(&self) -> String {
        body_of(&self.s.doc)
    }
}

fn body_of(doc: &Document) -> String {
    let c = wordcraft_collab::canonical(doc);
    c["body"].as_array().map(|b| b.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

struct Room {
    doc: yrs::Doc,
    seeded: bool,
    dirty: bool,
    /// The token holding the seed lease.
    lease: Option<String>,
    /// The lease holder is connected (only then does its leaving drop the lease).
    lease_connected: bool,
    /// Each tab's last awareness update, for newcomers.
    awareness: Vec<(usize, Vec<u8>)>,
    file_name: String,
    file: Vec<u8>,
    tabs: Vec<Tab>,
    uploads: Vec<(String, Vec<u8>)>,
    next_save: u64,
    now: u64,
    /// Updates from viewers, dropped.
    dropped: u32,
}

fn docx_with(text: &str, name: &str) -> Vec<u8> {
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": text})).unwrap();
    wordcraft_engine::io::save_bytes(name, &s.doc).unwrap()
}

fn token_of(url: &str) -> String {
    url.split("token=").nth(1).map(|t| unpct(t).unwrap_or_default()).unwrap_or_default()
}

impl Room {
    fn new(file_name: &str, file: Vec<u8>) -> Room {
        Room {
            doc: yrs::Doc::new(),
            seeded: false,
            dirty: false,
            lease: None,
            lease_connected: false,
            awareness: Vec::new(),
            file_name: file_name.into(),
            file,
            tabs: Vec::new(),
            uploads: Vec::new(),
            next_save: 1,
            now: 10_000,
            dropped: 0,
        }
    }

    /// Opens the editor as Nextcloud would: a session, the seed lease if it's free, the tab.
    fn open(&mut self, user: &str, edit: bool) -> usize {
        let token = format!("token-{user}-{}", self.tabs.len());
        let seed = !self.seeded && edit && self.lease.is_none();
        if seed {
            self.lease = Some(token.clone());
        }
        let cfg = Config::from_json(&json!({
            "ws": "ws://live.test/ws/nc%3Acloud%3A1", "room": "nc:cloud:1", "token": token,
            "user": {"id": user, "name": format!("{user} name"), "color": "#3B7DD8"},
            "role": if edit { "edit" } else { "view" }, "seed": seed,
            "file": "http://live.test/api/v1/rooms/nc%3Acloud%3A1/file", "name": self.file_name,
            "format": self.file_name.rsplit('.').next().unwrap_or(""), "api": "http://live.test/api/v1", "lang": "",
        }))
        .unwrap();
        let mut tab = Tab {
            live: Live::new(cfg),
            s: Session::new(Document::new()),
            connected: false,
            synced: false,
            held: None,
            client_id: None,
            can_upload: false,
            events: Vec::new(),
            shell: Vec::new(),
            status: Vec::new(),
            hold_fetch: false,
            fetches: 0,
        };
        let now = self.now;
        tab.live.start(&mut tab.s, now);
        self.tabs.push(tab);
        self.tabs.len() - 1
    }

    fn to(&mut self, i: usize, f: Frame) {
        if let Some(t) = self.tabs.get_mut(i)
            && t.connected
        {
            t.events.push(Event::Frame(f.encode()));
        }
    }

    fn control_all(&mut self, v: Json) {
        for i in 0..self.tabs.len() {
            self.to(i, Frame::Control(v.clone()));
        }
    }

    fn peers(&mut self) {
        let list: Vec<(usize, Json)> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.connected)
            .map(|(i, t)| {
                let mut p = json!({"user": t.live.cfg.user, "role": if t.live.cfg.view { "view" } else { "edit" }, "kind": "web"});
                if let Some(c) = t.client_id {
                    p["clientId"] = json!(c);
                }
                (i, p)
            })
            .collect();
        for (i, _) in &list {
            let others: Vec<Json> = list.iter().filter(|(j, _)| j != i).map(|(_, p)| p.clone()).collect();
            self.to(*i, Frame::Control(json!({"type": "peers", "peers": others})));
        }
    }

    fn state_vector(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    fn diff(&self, sv: &[u8]) -> Vec<u8> {
        let sv = StateVector::decode_v1(sv).unwrap_or_default();
        self.doc.transact().encode_diff_v1(&sv)
    }

    fn connect(&mut self, i: usize, url: &str) {
        let token = token_of(url);
        let seeder = self.lease.as_deref() == Some(token.as_str());
        if seeder {
            self.lease_connected = true;
        }
        let synced = self.seeded || seeder;
        let Some(t) = self.tabs.get_mut(i) else { return };
        assert_eq!(token, t.live.cfg.token);
        t.connected = true;
        t.synced = synced;
        t.held = None;
        t.events.push(Event::Open);
        let welcome = json!({
            "type": "welcome", "room": "nc:cloud:1", "user": t.live.cfg.user, "role": if t.live.cfg.view { "view" } else { "edit" },
            "app": "wordcraft", "fileName": self.file_name, "seeded": self.seeded, "seed": seeder && !self.seeded, "modified": self.dirty,
            "server": "fake",
        });
        self.to(i, Frame::Control(welcome));
        if synced {
            let sv = self.state_vector();
            self.to(i, Frame::Sync1(sv));
        }
        for (_, a) in self.awareness.clone() {
            self.to(i, Frame::Awareness(a));
        }
        self.peers();
    }

    fn drop_tab(&mut self, i: usize) {
        let Some(t) = self.tabs.get_mut(i) else { return };
        if !t.connected {
            return;
        }
        t.connected = false;
        let token = t.live.cfg.token.clone();
        self.awareness.retain(|(j, _)| *j != i);
        if !self.seeded && self.lease_connected && self.lease.as_deref() == Some(token.as_str()) {
            self.lease = None;
            self.lease_connected = false;
            for j in 0..self.tabs.len() {
                self.to(j, Frame::Control(json!({"type": "reseed", "message": "The person loading the file left before it was ready."})));
            }
        }
        self.peers();
    }

    fn frame(&mut self, i: usize, bytes: &[u8]) {
        let Some(f) = Frame::decode(bytes) else { panic!("tab {i} sent a malformed frame") };
        let (synced, view) = match self.tabs.get(i) {
            Some(t) => (t.synced, t.live.cfg.view),
            None => return,
        };
        match f {
            Frame::Sync1(sv) => {
                if synced {
                    let d = self.diff(&sv);
                    self.to(i, Frame::Sync2(d));
                } else if let Some(t) = self.tabs.get_mut(i) {
                    t.held = Some(sv);
                }
            }
            Frame::Sync2(u) | Frame::Update(u) => {
                if u.len() <= 2 && u.iter().all(|b| *b == 0) {
                    return;
                }
                if view {
                    self.dropped += 1;
                    return;
                }
                if !synced {
                    return;
                }
                let update = Update::decode_v1(&u).expect("a valid Yjs update");
                self.doc.transact_mut().apply_update(update).unwrap();
                if !self.seeded {
                    if self.state_vector() != vec![0] {
                        self.seeded = true;
                        self.lease = None;
                        for j in 0..self.tabs.len() {
                            let waiting = self.tabs.get(j).is_some_and(|t| t.connected && !t.synced);
                            if waiting {
                                let held = self.tabs.get_mut(j).and_then(|t| {
                                    t.synced = true;
                                    t.held.take()
                                });
                                let d = self.diff(&held.unwrap_or_default());
                                self.to(j, Frame::Sync2(d));
                                let sv = self.state_vector();
                                self.to(j, Frame::Sync1(sv));
                            }
                        }
                        self.control_all(json!({"type": "seeded", "modified": false}));
                    }
                    return;
                }
                if !self.dirty {
                    self.dirty = true;
                    self.control_all(json!({"type": "modified", "modified": true}));
                }
                for j in 0..self.tabs.len() {
                    if j != i && self.tabs.get(j).is_some_and(|t| t.synced) {
                        self.to(j, Frame::Update(u.clone()));
                    }
                }
            }
            Frame::Awareness(a) => {
                self.awareness.retain(|(j, _)| *j != i);
                self.awareness.push((i, a.clone()));
                for j in 0..self.tabs.len() {
                    if j != i {
                        self.to(j, Frame::Awareness(a.clone()));
                    }
                }
            }
            Frame::Control(v) => match v["type"].as_str().unwrap_or("") {
                "hello" => {
                    if let Some(t) = self.tabs.get_mut(i) {
                        t.client_id = v["clientId"].as_u64();
                        t.can_upload = v["can"].as_array().is_some_and(|a| a.iter().any(|c| c == "upload"));
                    }
                    self.peers();
                }
                "saveNow" if !view => {
                    let id = self.next_save;
                    self.next_save += 1;
                    let format = self.file_name.rsplit('.').next().unwrap_or("").to_string();
                    let url = format!("http://live.test/api/v1/rooms/nc%3Acloud%3A1/file?save={id}");
                    self.to(i, Frame::Control(json!({"type": "save", "id": id, "mode": "upload", "name": self.file_name, "format": format, "url": url, "autosave": false})));
                }
                _ => {}
            },
            Frame::Denied(_) | Frame::QueryAwareness => {}
        }
    }

    /// One round: every tab's actions carried out, then its events handled.
    fn step(&mut self) {
        self.now += 50;
        let now = self.now;
        for i in 0..self.tabs.len() {
            let actions = match self.tabs.get_mut(i) {
                Some(t) => {
                    t.live.tick(&mut t.s, now);
                    t.live.take_actions()
                }
                None => continue,
            };
            for a in actions {
                match a {
                    Action::Connect(url) => self.connect(i, &url),
                    Action::Send(b) => {
                        if self.tabs.get(i).is_some_and(|t| t.connected) {
                            self.frame(i, &b);
                        }
                    }
                    Action::Disconnect(..) => self.drop_tab(i),
                    Action::Shell(m) => self.tabs[i].shell.push(m),
                    Action::Fetch { token, .. } => {
                        assert_eq!(token, self.tabs[i].live.cfg.token);
                        self.tabs[i].fetches += 1;
                        if !self.tabs[i].hold_fetch {
                            let file = self.file.clone();
                            self.tabs[i].events.push(Event::File(Ok(file)));
                        }
                    }
                    Action::Upload { save, name, bytes, token, .. } => {
                        assert_eq!(token, self.tabs[i].live.cfg.token);
                        self.uploads.push((unpct(&name).unwrap(), bytes));
                        self.dirty = false;
                        self.tabs[i].events.push(Event::Uploaded { save, result: Ok((200, json!({"saved": true, "version": "2"}).to_string())) });
                        self.control_all(json!({"type": "saved", "at": 1, "modified": false}));
                    }
                    Action::Prepare { token, .. } => {
                        let seed = !self.seeded && self.lease.is_none();
                        if seed {
                            self.lease = Some(token);
                            self.lease_connected = false;
                        }
                        self.tabs[i].events.push(Event::Prepared(Ok(json!({"seed": seed, "seeded": self.seeded}))));
                    }
                    Action::Status(s) => self.tabs[i].status.push(s),
                    Action::Language(_) => {}
                }
            }
        }
        for t in &mut self.tabs {
            let events = std::mem::take(&mut t.events);
            t.live.handle_all(&mut t.s, events, now);
        }
    }

    fn settle(&mut self) {
        for _ in 0..40 {
            self.step();
        }
    }

    fn doc_text(&self) -> String {
        let mut s = Session::new(Document::new());
        s.collab_serve_in_memory("check");
        s.run("collab.join", &json!({"plugin": "check", "room": "check", "mode": "join"})).unwrap();
        let u = self.doc.transact().encode_state_as_update_v1(&StateVector::default());
        s.collab_poll_pushed(
            1,
            vec![Pushed {
                plugin: "check".into(),
                incoming: Incoming::Message(
                    json!({"method": "collab/sync2", "params": {"room": "check", "update": wordcraft_collab::b64_encode(&u)}}),
                ),
            }],
        );
        wordcraft_collab::canonical(&s.doc).to_string()
    }
}

fn type_in(t: &mut Tab, text: &str) -> Result<(), String> {
    t.s.run("text.insert", &json!({"text": text})).map(|_| ()).map_err(|e| e.to_string())
}

// ----------------------------------------------------------------------------- the client

#[test]
fn the_first_editor_seeds_and_the_next_joins_then_edits_meet() {
    let mut room = Room::new("Notes.docx", docx_with("Hello from Nextcloud", "Notes.docx"));
    let a = room.open("ada", true);
    room.settle();
    assert!(room.seeded, "the seeder's document reached the room");
    assert_eq!(room.tabs[a].live.phase, Phase::Live);
    assert!(room.tabs[a].text().contains("Hello from Nextcloud"));
    assert!(room.doc_text().contains("Hello from Nextcloud"));
    assert!(!room.tabs[a].live.seeding, "`seeded` confirmed it");
    assert_eq!(room.tabs[a].s.file_name.as_deref(), Some("Notes.docx"));
    assert!(room.tabs[a].shell_types().contains(&"craft-live:loaded".to_string()));

    let b = room.open("bo", true);
    assert!(!room.tabs[b].live.cfg.seed);
    room.settle();
    assert_eq!(room.tabs[b].live.phase, Phase::Live);
    assert!(room.tabs[b].text().contains("Hello from Nextcloud"));
    assert_eq!(room.tabs[b].s.file_name.as_deref(), Some("Notes.docx"), "the title comes from the room");
    assert!(room.tabs[b].shell_types().contains(&"craft-live:loaded".to_string()));

    type_in(&mut room.tabs[a], " and Ada").unwrap();
    room.settle();
    type_in(&mut room.tabs[b], " and Bo").unwrap();
    room.settle();
    let (ta, tb) = (room.tabs[a].text(), room.tabs[b].text());
    assert_eq!(ta, tb, "both see the same document");
    assert!(ta.contains("and Ada") && ta.contains("and Bo"), "{ta}");
    assert_eq!(room.doc_text(), ta, "and so does the room");
    assert_eq!(room.tabs[a].body().matches("Hello from Nextcloud").count(), 1, "seeded once");

    // The shell hears the room was modified, and who is there (ourselves too).
    let peers = room.tabs[a].shell.iter().rev().find(|m| m["type"] == "craft-live:peers").unwrap().clone();
    let list = peers["peers"].as_array().unwrap();
    assert!(list.iter().any(|p| p["user"]["id"] == "bo"));
    assert!(list.iter().any(|p| p["self"] == true && p["user"]["id"] == "ada" && p["clientId"].is_u64()));
    assert!(room.tabs[a].shell.iter().any(|m| m["type"] == "craft-live:modified" && m["modified"] == true));
    // The people circles come from awareness: each sees the other.
    assert_eq!(room.tabs[a].s.active_room().unwrap().peers().len(), 1);
    assert_eq!(room.tabs[b].s.active_room().unwrap().peers()[0].user.id, "ada");
    assert_eq!(room.tabs[a].s.active_room().unwrap().state, wordcraft_engine::collab::RoomState::Online);
}

#[test]
fn a_joiner_waits_read_only_until_the_seed_arrives() {
    let mut room = Room::new("Plan.docx", docx_with("The plan", "Plan.docx"));
    let a = room.open("ada", true);
    room.tabs[a].hold_fetch = true;
    room.settle();
    assert_eq!(room.tabs[a].live.phase, Phase::Fetching);
    assert!(type_in(&mut room.tabs[a], "x").is_err(), "nothing to edit before the file is loaded");

    let b = room.open("bo", true);
    room.settle();
    assert_eq!(room.tabs[b].live.phase, Phase::Waiting);
    assert!(room.tabs[b].s.active_room().unwrap().joining);
    assert!(type_in(&mut room.tabs[b], "too early").is_err(), "Joining… is read-only");
    assert!(!room.tabs[b].live.loaded());

    // The file arrives: A seeds, B gets the document.
    let file = room.file.clone();
    room.tabs[a].hold_fetch = false;
    room.tabs[a].events.push(Event::File(Ok(file)));
    room.settle();
    assert_eq!(room.tabs[b].live.phase, Phase::Live);
    assert!(room.tabs[b].live.loaded());
    assert!(room.tabs[b].text().contains("The plan"));
    type_in(&mut room.tabs[b], "Now: ").unwrap();
    room.settle();
    assert_eq!(room.tabs[a].text(), room.tabs[b].text());
}

#[test]
fn save_uploads_the_file_in_its_own_format() {
    let odt = {
        let mut s = Session::new(Document::new());
        s.run("text.insert", &json!({"text": "Minutes"})).unwrap();
        wordcraft_engine::io::save_bytes("Minutes.odt", &s.doc).unwrap()
    };
    let mut room = Room::new("Minutes.odt", odt);
    let a = room.open("ada", true);
    room.settle();
    type_in(&mut room.tabs[a], " of the meeting").unwrap();
    room.settle();
    assert!(room.tabs[a].shell.iter().any(|m| m["type"] == "craft-live:modified" && m["modified"] == true));

    // File › Save: the room is asked to save, the engine makes the file, it's uploaded.
    room.tabs[a].events.push(Event::Save);
    room.settle();
    let (name, bytes) = room.uploads.last().cloned().expect("an upload");
    assert_eq!(name, "Minutes.odt");
    let doc = wordcraft_engine::io::open_bytes("Minutes.odt", &bytes).unwrap();
    let body = body_of(&doc);
    assert!(body.contains("Minutes") && body.contains("of the meeting"), "{body}");
    let t = &room.tabs[a];
    assert!(t.shell.iter().any(|m| m["type"] == "craft-live:saved" && m["success"] == true));
    assert_eq!(t.shell.iter().rev().find(|m| m["type"] == "craft-live:modified").unwrap()["modified"], false);
    assert!(t.status.iter().any(|s| s == "Saving to Nextcloud…"));
    assert!(t.status.iter().any(|s| s == "Saved to Nextcloud"));
    assert!(!t.s.dirty, "the file in Nextcloud has every edit");
}

#[test]
fn a_save_request_in_a_format_wordcraft_cant_write_brings_a_docx() {
    let mut room = Room::new("Old.doc", Vec::new());
    let a = room.open("ada", true);
    room.settle();
    assert!(room.seeded, "a 0-byte file is a new, empty document");
    room.tabs[a].events.push(Event::Save);
    room.settle();
    let (name, bytes) = room.uploads.last().cloned().expect("an upload");
    assert_eq!(name, "Old.docx", "craft-live then saves it as a copy beside the file");
    assert!(bytes.starts_with(b"PK"));
}

#[test]
fn a_seeder_that_leaves_hands_the_seed_over() {
    let mut room = Room::new("Plan.docx", docx_with("Handed over", "Plan.docx"));
    let a = room.open("ada", true);
    room.tabs[a].hold_fetch = true;
    room.settle();
    let b = room.open("bo", true);
    room.settle();
    assert_eq!(room.tabs[b].live.phase, Phase::Waiting);
    // Ada's tab connects (as a seeder whose file never comes) and goes away.
    let url = room.tabs[a].live.cfg.socket_url();
    room.connect(a, &url);
    room.drop_tab(a);
    let t = &mut room.tabs[a];
    t.live.close(&mut t.s);
    room.settle();
    // Bo got `reseed`, asked for the lease, loaded the file and seeded the room.
    assert!(room.tabs[b].fetches >= 1);
    assert!(room.seeded);
    assert_eq!(room.tabs[b].live.phase, Phase::Live);
    assert!(room.tabs[b].text().contains("Handed over"));
    assert!(room.doc_text().contains("Handed over"));
}

#[test]
fn a_seed_the_room_doesnt_want_becomes_a_join() {
    let mut room = Room::new("Plan.docx", docx_with("Only once", "Plan.docx"));
    let a = room.open("ada", true);
    room.settle();
    assert!(room.seeded);
    // Bo's page was opened while the lease was free (a race): his tab seeds, but the room
    // already has its document when he connects.
    room.lease = None;
    room.seeded = false;
    let b = room.open("bo", true);
    assert!(room.tabs[b].live.cfg.seed);
    room.seeded = true;
    room.lease = None;
    room.settle();
    assert_eq!(room.tabs[b].live.phase, Phase::Live);
    let tb = room.tabs[b].body();
    assert_eq!(tb.matches("Only once").count(), 1, "not merged twice: {tb}");
    type_in(&mut room.tabs[b], "Bo: ").unwrap();
    room.settle();
    assert_eq!(room.tabs[a].text(), room.tabs[b].text());
}

#[test]
fn a_dropped_connection_comes_back_with_backoff_and_merges() {
    let mut room = Room::new("Plan.docx", docx_with("Base", "Plan.docx"));
    let a = room.open("ada", true);
    room.settle();
    let b = room.open("bo", true);
    room.settle();
    // The network drops Bo.
    room.drop_tab(b);
    room.tabs[b].events.push(Event::Closed { code: 1006, reason: String::new() });
    room.step();
    assert_eq!(room.tabs[b].s.active_room().unwrap().state, wordcraft_engine::collab::RoomState::Offline);
    // Editing goes on offline.
    type_in(&mut room.tabs[b], "Offline edit. ").unwrap();
    type_in(&mut room.tabs[a], "Online edit. ").unwrap();
    // No reconnection before a second has passed; then it reconnects and both sides merge.
    let before = room.now;
    for _ in 0..40 {
        room.step();
        if room.tabs[b].connected {
            break;
        }
    }
    assert!(room.tabs[b].connected);
    assert!(room.now - before >= 1_000, "waited {} ms", room.now - before);
    room.settle();
    let (ta, tb) = (room.tabs[a].text(), room.tabs[b].text());
    assert_eq!(ta, tb);
    assert!(ta.contains("Offline edit") && ta.contains("Online edit"));
    assert_eq!(room.tabs[b].s.active_room().unwrap().state, wordcraft_engine::collab::RoomState::Online);

    // Two failures in a row wait longer.
    room.drop_tab(b);
    room.tabs[b].events.push(Event::Closed { code: 1006, reason: String::new() });
    room.step();
    room.tabs[b].events.push(Event::Closed { code: 1006, reason: String::new() });
    room.step();
    let t0 = room.now;
    while !room.tabs[b].connected && room.now - t0 < 10_000 {
        room.step();
    }
    assert!(room.now - t0 >= 1_900, "the second attempt waits two seconds");
}

#[test]
fn expiry_and_refusal_stop_the_session() {
    let mut room = Room::new("Plan.docx", docx_with("Base", "Plan.docx"));
    let a = room.open("ada", true);
    room.settle();
    room.to(a, Frame::Control(json!({"type": "expired"})));
    room.step();
    room.tabs[a].events.push(Event::Closed { code: 4401, reason: "expired".into() });
    room.settle();
    let t = &room.tabs[a];
    assert_eq!(t.live.phase, Phase::Expired);
    assert!(!t.connected, "no reconnection");
    assert!(t.status.iter().any(|s| s.contains("expired")));
    assert!(t.s.active_room().unwrap().message.as_deref().is_some_and(|m| m.contains("open it again")));

    // A refused connection before the document showed: the shell shows the error.
    let mut room = Room::new("Plan.docx", docx_with("Base", "Plan.docx"));
    let b = room.open("bo", false);
    room.step();
    room.tabs[b].connected = true;
    room.to(b, Frame::Denied("this session is for another document".into()));
    room.tabs[b].events.push(Event::Closed { code: 4403, reason: String::new() });
    room.settle();
    let t = &room.tabs[b];
    assert_eq!(t.live.phase, Phase::Failed);
    let err = t.shell.iter().find(|m| m["type"] == "craft-live:error").expect("an error for the shell");
    assert!(err["message"].as_str().unwrap().contains("another document"));
}

#[test]
fn a_viewer_reads_but_never_edits_or_saves() {
    let mut room = Room::new("Plan.docx", docx_with("Read me", "Plan.docx"));
    room.open("ada", true);
    room.settle();
    let v = room.open("viv", false);
    room.settle();
    assert_eq!(room.tabs[v].live.phase, Phase::Live);
    assert!(room.tabs[v].text().contains("Read me"));
    assert!(type_in(&mut room.tabs[v], "no").is_err());
    room.tabs[v].events.push(Event::Save);
    room.settle();
    assert!(room.tabs[v].status.iter().any(|s| s == "This document is view only"));
    assert!(room.uploads.is_empty());
    assert_eq!(room.dropped, 0, "nothing was even sent");
}

#[test]
fn closing_says_bye_and_tells_the_shell() {
    let mut room = Room::new("Plan.docx", docx_with("Base", "Plan.docx"));
    let a = room.open("ada", true);
    room.settle();
    room.tabs[a].events.push(Event::Close);
    room.settle();
    let t = &room.tabs[a];
    assert_eq!(t.live.phase, Phase::Closed);
    assert!(!t.connected);
    assert!(t.s.active_room().is_none());
    assert!(t.shell_types().contains(&"craft-live:close".to_string()));
    // The shell's own close (Nextcloud closed the editor) is the same, once.
    let n = t.shell.len();
    room.tabs[a].events.push(Event::Shell(json!({"type": "craft-live:close"})));
    room.settle();
    assert_eq!(room.tabs[a].shell.len(), n);
}

#[test]
fn shell_requests_are_answered() {
    let mut room = Room::new("Plan.docx", docx_with("Base", "Plan.docx"));
    let a = room.open("ada", true);
    room.settle();
    room.tabs[a].events.push(Event::Shell(json!({"type": "craft-live:getPeers"})));
    room.tabs[a].events.push(Event::Shell(json!({"type": "craft-live:host", "message": "Hide_Menu_Item", "values": {}})));
    room.tabs[a].events.push(Event::Shell(json!({"type": "craft-live:save", "notify": true})));
    room.settle();
    let t = &room.tabs[a];
    assert!(t.shell.iter().rev().any(|m| m["type"] == "craft-live:peers"));
    assert!(t.status.iter().any(|s| s == "Saving to Nextcloud…"));
}

#[test]
fn hostile_control_messages_change_nothing() {
    let mut room = Room::new("Plan.docx", docx_with("Base", "Plan.docx"));
    let a = room.open("ada", true);
    room.settle();
    let before = room.tabs[a].text();
    for v in [
        json!({"type": 5}),
        json!({"type": "save"}),
        json!({"type": "save", "id": "x", "mode": "upload"}),
        json!({"type": "save", "id": 9, "mode": "webdav", "url": "http://x"}),
        json!({"type": "save", "id": 9, "mode": "upload", "url": "file:///etc/passwd"}),
        json!({"type": "peers", "peers": "nobody"}),
        json!({"type": "notice", "level": [], "text": {}}),
        json!({"type": "welcome", "seed": "yes"}),
        json!({"type": "modified", "modified": null}),
        json!({"type": "reseed", "message": 3}),
        json!({"type": "unknown"}),
    ] {
        room.to(a, Frame::Control(v));
    }
    room.to(a, Frame::Sync2(vec![1, 2, 3, 4, 5]));
    room.to(a, Frame::Update(vec![0xff; 40]));
    room.to(a, Frame::Awareness(vec![0xff; 40]));
    room.tabs[a].events.push(Event::Frame(vec![0xff; 30]));
    room.tabs[a].events.push(Event::Uploaded { save: 77, result: Ok((500, "<html>".into())) });
    room.tabs[a].events.push(Event::Prepared(Err("offline".into())));
    room.tabs[a].events.push(Event::File(Ok(vec![1, 2, 3])));
    room.settle();
    assert_eq!(room.tabs[a].text(), before);
    assert_eq!(room.tabs[a].live.phase, Phase::Live);
    assert!(room.uploads.is_empty());
}

#[test]
fn a_frame_the_socket_refused_waits_and_keeps_the_page_guarded() {
    let cfg = Config::from_json(&json!({"ws": "ws://127.0.0.1:9/ws/r", "room": "r", "token": "t", "seed": false, "role": "edit"})).unwrap();
    let mut live = Live::new(cfg);
    let mut s = Session::new(Document::new());
    assert!(!live.has_unsent());
    // A connection's control message isn't sent again on the next one.
    live.handle(&mut s, Event::Unsent(Frame::Control(json!({"type": "bye"})).encode()));
    live.handle(&mut s, Event::Unsent(vec![9]));
    assert!(!live.has_unsent());
    // An edit waits for the next welcome, and the page asks before closing meanwhile.
    live.handle(&mut s, Event::Unsent(Frame::Update(vec![1, 2, 3]).encode()));
    assert!(live.has_unsent());
}
