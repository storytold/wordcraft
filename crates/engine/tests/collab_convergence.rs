//! Co-editing convergence (protocol 2): two and three sessions run random command sequences
//! concurrently on one shared document (typing, often in the same paragraph, Enter, Backspace,
//! formatting, paragraph alignment, tables, comments, footnotes, page breaks, cut and paste,
//! per-user Undo and Redo), their updates delivered in random order and sometimes twice, and
//! must end with identical documents, equal to what a newcomer joining afterwards builds.
//!
//! The sessions are connected in memory ([`collab::IN_MEMORY`]): the test is the transport.
//!
//! Knobs: `WORDCRAFT_COLLAB_SEEDS` (default 6), `WORDCRAFT_COLLAB_STEPS` (default 70),
//! `WORDCRAFT_COLLAB_SEED` (replay one), `WORDCRAFT_COLLAB_TRACE` (print the steps).

use serde_json::{Value as Json, json};
use wordcraft_engine::collab::{self, IN_MEMORY};
use wordcraft_engine::doc::{Pos, StoryRef};
use wordcraft_engine::{Session, sample};
use wordcraft_plugin::push::{Incoming, Pushed};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// A random position in the body (table cells included); `near` biases it to the first
/// paragraph, so people often type in the same one.
fn pos(rng: &mut Rng, s: &Session) -> Json {
    let paths = s.doc.para_paths(StoryRef::Body);
    let path = if rng.chance(40) { paths.first().cloned() } else { paths.get(rng.below(paths.len())).cloned() }.unwrap_or_default();
    let offs: Vec<usize> =
        s.doc.para(StoryRef::Body, &path).map(|p| p.text.char_indices().map(|(i, _)| i).chain([p.text.len()]).collect()).unwrap_or_else(|| vec![0]);
    let off = *rng.pick(&offs);
    serde_json::to_value(Pos { story: StoryRef::Body, path, off }).unwrap_or(Json::Null)
}

/// A random edit, as a person would make it (each preceded by placing the caret or selecting).
fn generate(rng: &mut Rng, s: &Session, tag: &str) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    let a = pos(rng, s);
    if rng.chance(30) {
        let b = pos(rng, s);
        out.push(("select.range".to_string(), json!({"anchor": a, "focus": b})));
    } else {
        out.push(("caret.set".to_string(), json!({"pos": a})));
    }
    let words = ["the", "é", "😀", "co-edit", "x", tag];
    let (id, p) = match rng.below(24) {
        0..=7 => ("text.insert", json!({"text": format!("{}{}", rng.pick(&words), if rng.chance(50) { " " } else { "" })})),
        8 | 9 => ("text.newParagraph", json!({})),
        10 | 11 => ("text.backspace", json!({})),
        12 => ("text.delete", json!({})),
        13 => ("format.bold", json!({})),
        14 => ("format.italic", json!({})),
        15 => ("format.size", json!({"size": *rng.pick(&[9, 12, 14, 20])})),
        16 => (*rng.pick(&["para.alignCenter", "para.alignRight", "para.alignLeft"]), json!({})),
        17 => ("insert.table", json!({"rows": 1 + rng.below(2), "cols": 1 + rng.below(3)})),
        18 => ("review.newComment", json!({"text": format!("{tag} says")})),
        19 => ("references.footnote", json!({"text": format!("{tag} note")})),
        20 => ("edit.undo", json!({})),
        21 => ("edit.redo", json!({})),
        22 => ("edit.cut", json!({})),
        _ => ("edit.paste", json!({})),
    };
    out.push((id.to_string(), p));
    out
}

struct Peer {
    s: Session,
    now: u64,
}

impl Peer {
    fn deliver(&mut self, msg: Json) {
        if trace() {
            eprintln!("  deliver {}", msg.to_string().chars().take(160).collect::<String>());
        }
        self.now += 7;
        self.s.collab_poll_pushed(self.now, vec![Pushed { plugin: IN_MEMORY.into(), incoming: Incoming::Message(msg) }]);
    }
}

/// App → transport messages become transport → app messages for every other peer.
fn route(peers: &mut [Peer], from: usize, queue: &mut Vec<(usize, Json)>, rng: &mut Rng) {
    for msg in peers[from].s.collab_take_outgoing() {
        let method = msg["method"].as_str().unwrap_or("");
        if method == "collab/joined" || method == "collab/leave" || method.is_empty() {
            continue;
        }
        for to in 0..peers.len() {
            if to != from {
                queue.push((to, msg.clone()));
                if rng.chance(8) {
                    queue.push((to, msg.clone()));
                }
            }
        }
    }
}

fn settle(peers: &mut [Peer], queue: &mut Vec<(usize, Json)>, rng: &mut Rng) {
    for _ in 0..20_000 {
        if queue.is_empty() {
            break;
        }
        let k = rng.below(queue.len());
        let (to, msg) = queue.remove(k);
        peers[to].deliver(msg);
        route(peers, to, queue, rng);
    }
}

fn run(seed: u64, n: usize, steps: usize) -> Result<(), String> {
    let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) ^ 0xC011AB);
    let mut peers: Vec<Peer> = (0..n).map(|_| Peer { s: Session::new(wordcraft_engine::doc::Document::new()), now: 1_000 }).collect();
    for (i, p) in peers.iter_mut().enumerate() {
        p.s.author = format!("Person {i}");
    }
    peers[0].s.set_document(sample::sample_document());
    peers[0].s.collab_join_in_memory(json!({"room": "fuzz", "mode": "share"})).map_err(|e| e.to_string())?;
    for p in peers.iter_mut().skip(1) {
        p.s.collab_join_in_memory(json!({"room": "fuzz", "mode": "join"})).map_err(|e| e.to_string())?;
    }
    let mut queue = Vec::new();
    for i in 0..n {
        route(&mut peers, i, &mut queue, &mut rng);
    }
    settle(&mut peers, &mut queue, &mut rng);
    for (i, p) in peers.iter().enumerate() {
        if collab::read_only(&p.s).is_some() {
            return Err(format!("peer {i} is still joining after the first sync"));
        }
    }
    let mut log = Vec::new();
    for step in 0..steps {
        let i = rng.below(n);
        if rng.chance(55) || queue.is_empty() {
            let tag = format!("p{i}s{step}");
            for (id, p) in generate(&mut rng, &peers[i].s, &tag) {
                if trace() {
                    eprintln!("{step}: peer {i} {id} {p}");
                }
                let r = peers[i].s.run(&id, &p);
                log.push(format!("{step}: peer {i} {id} {p} → {}", if r.is_ok() { "ok".to_string() } else { format!("{:?}", r.err()) }));
            }
            route(&mut peers, i, &mut queue, &mut rng);
        } else {
            let k = rng.below(queue.len());
            let (to, msg) = queue.remove(k);
            if trace() {
                eprintln!("{step}: peer {to} gets {}", msg["method"]);
            }
            peers[to].deliver(msg);
            route(&mut peers, to, &mut queue, &mut rng);
        }
    }
    settle(&mut peers, &mut queue, &mut rng);
    let typed = log.iter().filter(|l| l.contains("text.insert") && l.ends_with("ok")).count();
    if steps >= 40 && typed == 0 {
        return Err(format!("nobody typed anything\n{}", log.join("\n")));
    }
    // A sync round, as when a transport comes back online.
    for i in 0..n {
        peers[i].deliver(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "fuzz", "state": "offline"}}));
        peers[i].deliver(json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "fuzz", "state": "online"}}));
        route(&mut peers, i, &mut queue, &mut rng);
    }
    settle(&mut peers, &mut queue, &mut rng);
    let first = peers[0].s.collab_fingerprint();
    for (i, p) in peers.iter().enumerate().skip(1) {
        let fp = p.s.collab_fingerprint();
        if fp != first {
            return Err(format!("peer {i} differs from peer 0\n{}\n--- steps\n{}", diff(&first, &fp), log.join("\n")));
        }
    }
    // A newcomer gets the same document.
    let mut late = Peer { s: Session::new(wordcraft_engine::doc::Document::new()), now: 1_000 };
    late.s.collab_join_in_memory(json!({"room": "fuzz", "mode": "join"})).map_err(|e| e.to_string())?;
    peers.push(late);
    let last = peers.len() - 1;
    route(&mut peers, last, &mut queue, &mut rng);
    settle(&mut peers, &mut queue, &mut rng);
    let fp = peers[last].s.collab_fingerprint();
    if fp != first {
        return Err(format!("a newcomer differs\n{}", diff(&first, &fp)));
    }
    Ok(())
}

fn trace() -> bool {
    std::env::var_os("WORDCRAFT_COLLAB_TRACE").is_some()
}

/// The first part of two fingerprints that differs.
fn diff(a: &Json, b: &Json) -> String {
    for k in ["body", "parts", "comments", "styles", "numbering", "settings", "media", "files"] {
        if a[k] != b[k] {
            let (x, y) = (a[k].to_string(), b[k].to_string());
            let start = x.bytes().zip(y.bytes()).take_while(|(p, q)| p == q).count().saturating_sub(200);
            let cut = |t: &str| t.get(start.min(t.len())..).unwrap_or("").chars().take(1200).collect::<String>();
            return format!("{k} (from byte {start}):\n- {}\n+ {}", cut(&x), cut(&y));
        }
    }
    String::new()
}

fn seeds() -> Vec<u64> {
    match std::env::var("WORDCRAFT_COLLAB_SEED").ok().and_then(|v| v.trim().parse().ok()) {
        Some(one) => vec![one],
        None => (0..env_num("WORDCRAFT_COLLAB_SEEDS", 6)).collect(),
    }
}

#[test]
fn two_sessions_converge() {
    let steps = env_num("WORDCRAFT_COLLAB_STEPS", 70) as usize;
    for seed in seeds() {
        if let Err(e) = run(seed, 2, steps) {
            panic!("seed {seed} (WORDCRAFT_COLLAB_SEED={seed}): {e}");
        }
    }
}

#[test]
fn three_sessions_converge() {
    let steps = env_num("WORDCRAFT_COLLAB_STEPS", 70) as usize;
    for seed in seeds() {
        if let Err(e) = run(seed + 1_000, 3, steps) {
            panic!("seed {} (WORDCRAFT_COLLAB_SEED={}): {e}", seed + 1_000, seed + 1_000);
        }
    }
}

#[test]
fn another_document_ends_the_session_without_sharing_it() {
    let mut a = Session::new(wordcraft_engine::doc::Document::new());
    a.run("text.insert", &json!({"text": "Room text"})).unwrap();
    a.collab_join_in_memory(json!({"room": "r", "mode": "share"})).unwrap();
    let _ = a.collab_take_outgoing();
    a.run("file.new", &json!({})).unwrap();
    assert!(a.active_room().is_none(), "the session ended with the document");
    let sent = a.collab_take_outgoing();
    let methods: Vec<&str> = sent.iter().filter_map(|m| m["method"].as_str()).collect();
    assert!(!methods.contains(&"collab/update"), "the new document isn't shared: {methods:?}");
    assert!(methods.contains(&"collab/leave"), "the transport is told: {methods:?}");
}

#[test]
fn joining_is_read_only_and_undo_is_per_person() {
    let mut a = Session::new(wordcraft_engine::doc::Document::from_text("one"));
    let mut b = Session::new(wordcraft_engine::doc::Document::new());
    a.collab_join_in_memory(json!({"room": "r", "mode": "share"})).unwrap();
    b.collab_join_in_memory(json!({"room": "r", "mode": "join"})).unwrap();
    assert_eq!(collab::read_only(&b).as_deref(), Some("Joining…"));
    assert!(b.run("text.insert", &json!({"text": "too early"})).is_err());
    let mut peers = vec![Peer { s: a, now: 1_000 }, Peer { s: b, now: 1_000 }];
    let mut rng = Rng(7);
    let mut queue = Vec::new();
    route(&mut peers, 0, &mut queue, &mut rng);
    route(&mut peers, 1, &mut queue, &mut rng);
    settle(&mut peers, &mut queue, &mut rng);
    assert!(collab::read_only(&peers[1].s).is_none());
    let text = |s: &Session| s.doc.para(StoryRef::Body, &wordcraft_engine::doc::Path::top(0)).map(|p| p.text.clone()).unwrap_or_default();
    assert_eq!(text(&peers[1].s), "one");
    peers[0].s.run("caret.set", &json!({"pos": {"story": "body", "path": [0], "off": 3}})).unwrap();
    peers[0].s.run("text.insert", &json!({"text": " two"})).unwrap();
    peers[0].s.run("text.insert", &json!({"text": " three"})).unwrap();
    route(&mut peers, 0, &mut queue, &mut rng);
    settle(&mut peers, &mut queue, &mut rng);
    peers[1].s.run("caret.set", &json!({"pos": {"story": "body", "path": [0], "off": 0}})).unwrap();
    peers[1].s.run("text.insert", &json!({"text": "zero "})).unwrap();
    route(&mut peers, 1, &mut queue, &mut rng);
    settle(&mut peers, &mut queue, &mut rng);
    assert_eq!(text(&peers[0].s), "zero one two three");
    // Ann's undo takes back her typing (one step), not Bob's.
    let r = peers[0].s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(r["perUser"], true);
    route(&mut peers, 0, &mut queue, &mut rng);
    settle(&mut peers, &mut queue, &mut rng);
    assert_eq!(text(&peers[0].s), "zero one");
    assert_eq!(text(&peers[1].s), "zero one");
    assert!(peers[0].s.can_redo());
    peers[0].s.run("edit.redo", &json!({})).unwrap();
    route(&mut peers, 0, &mut queue, &mut rng);
    settle(&mut peers, &mut queue, &mut rng);
    assert_eq!(text(&peers[1].s), "zero one two three");
    // The caret stayed with its text while the others typed.
    assert_eq!(peers[1].s.sel.focus.off, "zero ".len());
}
