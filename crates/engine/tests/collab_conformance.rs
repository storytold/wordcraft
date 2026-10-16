//! Protocol 2 conformance: replays craft-collab's WordCraft schema-1 transcripts
//! (`conformance/wordcraft-1-*.jsonl`, format in craft-collab's `docs/conformance.md`) against the
//! engine. The test stands in for the transport `org.craft.conformance`: `run` lines are engine
//! commands, `in` lines go to the session's collab inbox, `out` lines are matched against what
//! the session sent, `model` lines against the document, `notice` lines against the status-bar
//! messages, and `decodesTo` against what another peer would build from every update so far.
//! The chat transcripts (`wordcraft-1-chat*`, craft-collab `docs/chat.md`) add `chat` lines (the
//! document's chat), `typing` lines (who the app shows typing) and `chatDecodesTo`; `run` lines
//! may be refused (`fails`), and ids the app makes up are bound (`"$mine1"`) and reused.
//!
//! The transcripts live in the craft-collab repository: `CRAFT_COLLAB_DIR`, or a checkout next
//! to this one (`../craft-collab-wordcraft`, then `../craft-collab`). Without one the test says
//! so and passes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value as Json, json};
use wordcraft_engine::collab_core::crypto::{self, Key};
use wordcraft_engine::collab_core::json::{to_any, to_json};
use wordcraft_engine::collab_core::{Replica, b64_decode};
use wordcraft_engine::doc::{Block, Blocks, Document, Paragraph};
use wordcraft_engine::{Session, collab};
use wordcraft_plugin::push::{Incoming, Pushed};

const TRANSPORT: &str = "org.craft.conformance";

fn transcripts() -> Option<Vec<PathBuf>> {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let dirs: Vec<PathBuf> = match std::env::var_os("CRAFT_COLLAB_DIR") {
        Some(d) => vec![PathBuf::from(d)],
        None => vec![here.join("craft-collab-wordcraft"), here.join("craft-collab")],
    };
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir.join("conformance")) else { continue };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("wordcraft-1-") && n.ends_with(".jsonl")))
            .collect();
        files.sort();
        if !files.is_empty() {
            return Some(files);
        }
    }
    None
}

/// Pattern matching of `out` lines: `$any`, `$b64`, `$int`, `$absent`, and variables bound the
/// first time they meet a value (`$clientId`, `$mine1`, …).
fn matches(pat: &Json, v: Option<&Json>, vars: &mut BTreeMap<String, Json>) -> bool {
    match pat {
        Json::String(p) if p == "$absent" => v.is_none(),
        Json::String(p) if p == "$any" => v.is_some(),
        Json::String(p) if p == "$b64" => v.and_then(Json::as_str).is_some_and(|s| b64_decode(s).is_ok()),
        Json::String(p) if p == "$int" => v.is_some_and(|v| v.is_i64() || v.is_u64()),
        Json::String(p) if p.starts_with('$') && p.len() > 1 => match (vars.get(p), v) {
            (Some(bound), Some(v)) => bound == v,
            (None, Some(v)) => {
                vars.insert(p.clone(), v.clone());
                true
            }
            _ => false,
        },
        Json::Object(o) => {
            let Some(Json::Object(vo)) = v else { return false };
            o.iter().all(|(k, p)| matches(p, vo.get(k), vars))
        }
        Json::Array(a) => {
            let Some(Json::Array(va)) = v else { return false };
            a.len() == va.len() && a.iter().zip(va).all(|(p, x)| matches(p, Some(x), vars))
        }
        other => v == Some(other),
    }
}

fn extra(msg: &Json) -> bool {
    matches!(msg.get("method").and_then(Json::as_str), Some("collab/awareness" | "collab/sync1" | "host/log"))
}

/// JSON as Yjs carries it (integral numbers as integers).
fn norm(v: Json) -> Json {
    to_json(&to_any(&v))
}

fn para_json(p: &Paragraph) -> Json {
    let mut runs: Vec<Json> = Vec::new();
    let mut start = 0;
    let mut k = 0;
    for r in &p.runs {
        let text = p.text.get(start..start + r.len).unwrap_or_default();
        start += r.len;
        let attrs = norm(serde_json::to_value(&r.props).unwrap_or(Json::Null));
        // Objects are runs of their own.
        let mut piece = String::new();
        let flush = |piece: &mut String, runs: &mut Vec<Json>| {
            if piece.is_empty() {
                return;
            }
            let same = runs.last().is_some_and(|l: &Json| l.get("embed").is_none() && strip_text(l) == attrs);
            if same {
                if let Some(t) = runs.last_mut().and_then(|l| l.get_mut("text")) {
                    *t = json!(format!("{}{}", t.as_str().unwrap_or(""), piece));
                }
            } else {
                let mut o = attrs.as_object().cloned().unwrap_or_default();
                o.insert("text".into(), json!(piece.clone()));
                runs.push(Json::Object(o));
            }
            piece.clear();
        };
        for c in text.chars() {
            if c == '\u{FFFC}' {
                flush(&mut piece, &mut runs);
                let mut o = attrs.as_object().cloned().unwrap_or_default();
                o.insert("text".into(), json!("\u{FFFC}"));
                o.insert("embed".into(), norm(serde_json::to_value(p.objects.get(k)).unwrap_or(Json::Null)));
                k += 1;
                runs.push(Json::Object(o));
            } else {
                piece.push(c);
            }
        }
        flush(&mut piece, &mut runs);
    }
    let mut out = json!({"text": p.text, "runs": runs});
    let props = norm(serde_json::to_value(&p.props).unwrap_or(Json::Null));
    if props.as_object().is_some_and(|o| !o.is_empty()) {
        out["props"] = props;
    }
    out
}

fn strip_text(run: &Json) -> Json {
    let mut o = run.as_object().cloned().unwrap_or_default();
    o.remove("text");
    Json::Object(o)
}

fn blocks_json(bl: &Blocks) -> Vec<Json> {
    bl.iter()
        .map(|b| match &**b {
            Block::Para(p) => para_json(p),
            Block::Table(t) => {
                json!({"table": t.rows.iter().map(|r| r.cells.iter().map(|c| blocks_json(&c.blocks)).collect::<Vec<_>>()).collect::<Vec<_>>()})
            }
        })
        .collect()
}

/// A document as the transcripts describe it.
fn model_of(title: &str, doc: Option<&Document>) -> Json {
    json!({"title": title, "body": doc.map(|d| blocks_json(&d.body)).unwrap_or_default()})
}

/// `run` params with every string naming a bound variable replaced by its value.
fn substitute(v: &Json, vars: &BTreeMap<String, Json>) -> Json {
    match v {
        Json::String(s) if s.starts_with('$') => vars.get(s).cloned().unwrap_or_else(|| v.clone()),
        Json::Array(a) => Json::Array(a.iter().map(|x| substitute(x, vars)).collect()),
        Json::Object(o) => Json::Object(o.iter().map(|(k, x)| (k.clone(), substitute(x, vars))).collect()),
        _ => v.clone(),
    }
}

/// The chat another peer reads from `updates` (already decrypted).
fn decoded_chat(updates: &[Vec<u8>]) -> Json {
    let mut r = Replica::new(77);
    for u in updates {
        let _ = r.apply_update(u);
    }
    r.chat_now().to_json()
}

/// What another peer builds from `updates` (already decrypted).
fn decoded(updates: &[Vec<u8>]) -> Json {
    let mut r = Replica::new(77);
    for u in updates {
        let _ = r.apply_update(u);
    }
    match r.refresh(&Document::new()) {
        Ok(Some(out)) => model_of(&r.title().unwrap_or_default(), Some(&out.doc)),
        _ => model_of(&r.title().unwrap_or_default(), None),
    }
}

struct Replay {
    s: Session,
    now: u64,
    pending: Vec<Json>,
    notices: Vec<String>,
    vars: BTreeMap<String, Json>,
    /// Updates a peer would have: valid `in` ones and the app's own, decrypted.
    updates: Vec<Vec<u8>>,
    key: Option<Key>,
    room: String,
    title: String,
}

impl Replay {
    fn open(&self, method: &str, text: &str) -> Option<Vec<u8>> {
        let data = b64_decode(text).ok()?;
        match &self.key {
            Some(k) => crypto::decrypt(k, &self.room, method, &data).ok(),
            None => Some(data),
        }
    }
    fn collect(&mut self) {
        for m in self.s.collab_take_outgoing() {
            let method = m.get("method").and_then(Json::as_str).unwrap_or("").to_string();
            let p = &m["params"];
            if method == "collab/update" || method == "collab/sync2" {
                if let Some(u) = p.get("update").and_then(Json::as_str).and_then(|t| self.open(&method, t)) {
                    self.updates.push(u);
                }
            } else if method.is_empty()
                && let Some(u) = m.get("result").and_then(|r| r.get("update")).and_then(Json::as_str).and_then(|t| self.open("collab/snapshot", t))
            {
                self.updates.push(u);
            }
            self.pending.push(m);
        }
        self.notices.extend(self.s.collab_take_notices());
        if let Some(t) = self.s.active_room().and_then(|r| r.title()) {
            self.title = t;
        }
    }
    fn model(&self) -> Json {
        if collab::read_only(&self.s).as_deref() == Some("Joining…") {
            return model_of(&self.title, None);
        }
        model_of(&self.title, Some(&self.s.doc))
    }
}

fn replay(path: &PathBuf) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut s = Session::new(Document::new());
    s.author = "Conformance".into();
    let mut r = Replay {
        s,
        now: 1_000,
        pending: vec![],
        notices: vec![],
        vars: BTreeMap::new(),
        updates: vec![],
        key: None,
        room: String::new(),
        title: String::new(),
    };
    r.s.collab_serve_in_memory(TRANSPORT);
    for (n, line) in text.lines().enumerate() {
        let n = n + 1;
        let l: Json = serde_json::from_str(line).map_err(|e| format!("line {n}: {e}"))?;
        match l["t"].as_str().unwrap_or("") {
            "run" => {
                let cmd = l["command"].as_str().unwrap_or("");
                let params = substitute(&l["params"], &r.vars);
                let fails = l.get("fails").and_then(Json::as_bool) == Some(true);
                if cmd == "collab.join" {
                    r.room = params["room"].as_str().unwrap_or("").to_string();
                    r.key = params.get("key").and_then(Json::as_str).and_then(|k| Key::from_base64(k).ok());
                }
                match r.s.run(cmd, &params) {
                    Ok(out) if fails => return Err(format!("line {n}: {cmd} should have been refused, gave {out}")),
                    Ok(_) | Err(_) if fails => {}
                    Ok(_) => {}
                    Err(e) => return Err(format!("line {n}: {cmd}: {e}")),
                }
                r.collect();
            }
            "in" => {
                let msg = l["msg"].clone();
                let valid = l.get("valid").and_then(Json::as_bool) != Some(false);
                let method = msg.get("method").and_then(Json::as_str).unwrap_or("").to_string();
                let p = msg.get("params").cloned().unwrap_or(Json::Null);
                if valid && (method == "collab/update" || method == "collab/sync2") && p.get("room").and_then(Json::as_str) == Some(r.room.as_str()) {
                    let sealed = if p.get("snapshot").and_then(Json::as_bool) == Some(true) { "collab/snapshot" } else { method.as_str() };
                    if let Some(u) = p.get("update").and_then(Json::as_str).and_then(|t| r.open(sealed, t)) {
                        r.updates.push(u);
                    }
                }
                r.now += 10;
                r.s.collab_poll_pushed(r.now, vec![Pushed { plugin: TRANSPORT.into(), incoming: Incoming::Message(msg) }]);
                r.collect();
            }
            "out" => {
                let pat = &l["msg"];
                let optional = l.get("optional").and_then(Json::as_bool) == Some(true);
                let mut i = 0;
                let mut found = None;
                while i < r.pending.len() {
                    let mut vars = r.vars.clone();
                    if matches(pat, r.pending.get(i), &mut vars) {
                        r.vars = vars;
                        found = Some(i);
                        break;
                    }
                    if !extra(&r.pending[i]) {
                        break;
                    }
                    i += 1;
                }
                match found {
                    Some(i) => {
                        r.pending.drain(..=i);
                        if let Some(want) = l.get("decodesTo") {
                            let got = decoded(&r.updates);
                            if &got != want {
                                return Err(format!("line {n}: decodes to\n{got}\nnot\n{want}"));
                            }
                        }
                        if let Some(want) = l.get("chatDecodesTo") {
                            let got = decoded_chat(&r.updates);
                            if !matches(want, Some(&got), &mut r.vars) {
                                return Err(format!("line {n}: the chat decodes to\n{got}\nnot\n{want}"));
                            }
                        }
                    }
                    None if optional => {}
                    None => return Err(format!("line {n}: expected {pat}\n sent: {:?}", r.pending)),
                }
            }
            "quiet" => {
                if let Some(m) = r.pending.iter().find(|m| !extra(m)) {
                    return Err(format!("line {n}: expected quiet, sent {m}"));
                }
                r.pending.clear();
            }
            "model" => {
                let got = r.model();
                if got != l["model"] {
                    return Err(format!("line {n}: model\n{got}\nnot\n{}", l["model"]));
                }
            }
            "chat" => {
                let got = r.s.doc.chat.to_json();
                if !matches(&l["chat"], Some(&got), &mut r.vars) {
                    return Err(format!("line {n}: the chat is\n{got}\nnot\n{}", l["chat"]));
                }
            }
            "typing" => {
                let mut want: Vec<String> =
                    l["names"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                want.sort();
                let got = r.s.chat_typing_names();
                if got != want {
                    return Err(format!("line {n}: typing {got:?}, expected {want:?}"));
                }
            }
            "notice" => {
                let want = l["contains"].as_str().unwrap_or("");
                if !r.notices.iter().any(|t| t.contains(want)) {
                    return Err(format!("line {n}: no notice containing {want:?} (got {:?})", r.notices));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[test]
fn craft_collab_transcripts() {
    let Some(files) = transcripts() else {
        eprintln!("craft-collab's WordCraft transcripts aren't here (set CRAFT_COLLAB_DIR): skipped");
        return;
    };
    let mut failures = Vec::new();
    assert!(files.len() >= 5, "{files:?}");
    if !files.iter().any(|f| f.to_string_lossy().contains("-chat")) {
        eprintln!("craft-collab's chat transcripts (wordcraft-1-chat*) aren't here yet: replaying the others");
    }
    for f in &files {
        eprintln!("replaying {}", f.display());
        if let Err(e) = replay(f) {
            failures.push(format!("{}: {e}", f.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
