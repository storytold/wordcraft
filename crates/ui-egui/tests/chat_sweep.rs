//! Chat sweep. Every command on the agent allow-list (also those marked as not editing: the
//! gate checks them too) runs as a member, with default params and with a representative params
//! set, in several places of a sample document (owner text, a table, a header, an owner's
//! tracked insertion, an owner's comment):
//!
//! - through the gate (`run_as_member`) it is either refused, with the document unchanged, or
//!   it leaves only changes tracked under the member's name plus allowed formatting (announced
//!   once in the chat); accept/reject only resolve tracked changes;
//! - its raw effect (the same command without the guard) is pinned in `UNTRACKED`: an upstream
//!   merge that makes an allowed command change the document without tracking fails this test
//!   until someone reviews the command (take it off the allow-list, or accept the new entry
//!   when the guard refuses it everywhere).
//!
//! The checker below is written independently of `chat_guard` (per-character, JSON-based).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ, Paragraph};
use wordcraft_doc::{Block, Blocks, CharProps, Document};
use wordcraft_engine::Session;
use wordcraft_ui_egui::WordApp;
use wordcraft_ui_egui::chat_gate::{AGENT_COMMANDS, run_as_member};

const ME: &str = "@claude";
const OWNER: &str = "Owner";

/// Allowed mutating commands whose raw effect (without the guard) is an untracked change in at
/// least one place of the sweep. The guard refuses them there. Review every change to this list.
const UNTRACKED: &[&str] = &["para.bullets", "para.numbering", "review.deleteComment"];

fn owner(a: &mut WordApp, id: &str, p: Value) {
    if let Err(e) = a.run(id, p) {
        panic!("sample {id}: {e}");
    }
}

fn sample() -> WordApp {
    let mut a = WordApp::new(Session::new(Document::new()), Default::default());
    a.session.author = OWNER.into();
    owner(&mut a, "document.setText", json!({"text": "Alpha beta gamma.\nDelta epsilon.\nZeta eta."}));
    owner(&mut a, "insert.editHeader", json!({}));
    owner(&mut a, "text.insert", json!({"text": "Owner header café"}));
    owner(&mut a, "insert.closeHeader", json!({}));
    owner(&mut a, "caret.docEnd", json!({}));
    owner(&mut a, "insert.table", json!({"rows": 2, "cols": 2}));
    owner(&mut a, "text.insert", json!({"text": "Cell"}));
    owner(&mut a, "review.trackChanges", json!({"value": true}));
    owner(&mut a, "select.text", json!({"text": "Delta"}));
    owner(&mut a, "caret.right", json!({}));
    owner(&mut a, "text.insert", json!({"text": " new"}));
    owner(&mut a, "review.trackChanges", json!({"value": false}));
    owner(&mut a, "select.text", json!({"text": "Zeta"}));
    owner(&mut a, "review.newComment", json!({"text": "owner's comment"}));
    owner(&mut a, "caret.docStart", json!({}));
    let chat =
        wordcraft_chat::Chat::new(wordcraft_chat::Hub::new(wordcraft_chat::testing::TestEnv::at(1_000)), wordcraft_chat::testing::FakePort::closed());
    let _ = chat.start();
    if let Ok(inv) = chat.invite(ME) {
        let _ = chat.hub().join(&inv.code);
    }
    a.session.chat = Some(std::sync::Arc::new(chat));
    a
}

/// Where the member's selection is before the command.
fn places() -> Vec<(&'static str, Vec<(&'static str, Value)>)> {
    vec![
        ("owner text", vec![("select.text", json!({"text": "beta gamma"}))]),
        ("table cell", vec![("select.text", json!({"text": "Cell"}))]),
        ("owner insertion", vec![("select.text", json!({"text": "epsilon"})), ("select.paragraph", json!({}))]),
        ("comment anchor", vec![("select.text", json!({"text": "eta."})), ("select.paragraph", json!({}))]),
        ("everything", vec![("select.all", json!({}))]),
        ("header", vec![("insert.editHeader", json!({})), ("select.all", json!({}))]),
        (
            "owner paragraph split by a member Enter",
            vec![
                ("select.text", json!({"text": "beta"})),
                ("select.collapse", json!({})),
                ("text.newParagraph", json!({})),
                ("select.text", json!({"text": "beta gamma."})),
            ],
        ),
    ]
}

/// Representative params (besides `{}`), per command.
fn representative(id: &str) -> Value {
    match id {
        "format.bold" | "format.italic" | "format.strikethrough" => json!({"value": true}),
        "format.color" => json!({"color": "FF0000"}),
        "format.font" => json!({"name": "Arial"}),
        "format.highlight" => json!({"color": "yellow"}),
        "format.size" => json!({"size": 14}),
        "format.underline" => json!({"value": true, "style": "double"}),
        "para.align" => json!({"value": "center"}),
        "para.bullets" => json!({"kind": "bullet"}),
        "para.indents" => json!({"left": 36, "firstLine": -18}),
        "para.lineSpacing" => json!({"value": 2}),
        "para.numbering" => json!({"kind": "numbered"}),
        "para.spacing" => json!({"before": 12, "after": 6}),
        "para.style" => json!({"style": "Heading1"}),
        "review.deleteComment" => json!({"id": 0}),
        "review.newComment" => json!({"text": "from the agent"}),
        "review.reply" => json!({"id": 0, "text": "the agent's reply"}),
        "review.resolveComment" => json!({"id": 0}),
        "text.insert" => json!({"text": "new text"}),
        _ => json!({"value": true}),
    }
}

// ---------------------------------------------------------------- independent checker

const CHAR_ALLOWED: [&str; 15] = [
    "bold",
    "italic",
    "underline",
    "underlineColor",
    "strike",
    "doubleStrike",
    "font",
    "size",
    "color",
    "highlight",
    "vertAlign",
    // Their complex-script twins (Persian, Arabic text): Bold, Italic, Font and Size set both.
    "boldCs",
    "italicCs",
    "fontCs",
    "sizeCs",
];
/// Paragraph properties a member may change on the owner's paragraphs. Lists (`numbering`, and
/// the list a style puts the paragraph in) are not among them.
const PARA_ALLOWED: [&str; 9] =
    ["style", "align", "indentLeft", "indentRight", "indentFirst", "spaceBefore", "spaceAfter", "lineSpacing", "contextualSpacing"];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Lens {
    Member,
    Accepted,
    Rejected,
}

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Text,
    Allowed,
    Strict,
}

#[derive(Debug, PartialEq)]
enum Class {
    Clean,
    Format,
    Dirty(&'static str),
}

struct Ctx<'a> {
    d: &'a Document,
    lens: Lens,
    level: Level,
    own: &'a BTreeSet<u32>,
}

fn json_of<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

fn without(v: Value, keys: &[&str]) -> Value {
    match v {
        Value::Object(mut m) => {
            for k in keys {
                m.remove(*k);
            }
            Value::Object(m)
        }
        v => v,
    }
}

impl Ctx<'_> {
    fn who(&self, r: Option<u32>) -> Option<String> {
        r.and_then(|i| self.d.revisions.get(i as usize)).map(|x| format!("{:?}/{}/{}", x.kind, x.author, x.date))
    }
    fn mine(&self, r: Option<u32>) -> bool {
        self.lens == Lens::Member && r.and_then(|i| self.d.revisions.get(i as usize)).is_some_and(|x| x.author == ME)
    }
    fn gone(&self, c: &CharProps) -> bool {
        match self.lens {
            Lens::Member => self.mine(c.ins),
            Lens::Accepted => c.del.is_some(),
            Lens::Rejected => c.ins.is_some(),
        }
    }
    fn joins(&self, mark: &CharProps) -> bool {
        match self.lens {
            Lens::Member => self.mine(mark.ins),
            Lens::Accepted => false,
            Lens::Rejected => mark.ins.is_some(),
        }
    }
    fn revs(&self, c: &CharProps) -> String {
        let keep = self.lens == Lens::Member;
        let ins = if keep && !self.mine(c.ins) { self.who(c.ins) } else { None };
        let del = if keep && !self.mine(c.del) { self.who(c.del) } else { None };
        format!("ins={ins:?} del={del:?}")
    }
    fn look(&self, style: Option<&str>, c: &CharProps) -> String {
        let mut p = c.clone();
        p.ins = None;
        p.del = None;
        match self.level {
            Level::Text => String::new(),
            Level::Allowed => format!("hidden={} {}", self.d.styles.resolve_char(style, &p).hidden, without(json_of(&p), &CHAR_ALLOWED)),
            Level::Strict => format!("hidden={} {}", self.d.styles.resolve_char(style, &p).hidden, json_of(&p)),
        }
    }
    fn para_look(&self, p: &Paragraph) -> String {
        let list = self.d.styles.resolve_para(&p.props).numbering.filter(|n| n.num != 0).map(|n| (n.num, n.level));
        match self.level {
            Level::Text => String::new(),
            Level::Allowed => format!("{} list={list:?}", without(json_of(&p.props), &PARA_ALLOWED)),
            Level::Strict => format!("{} list={list:?}", json_of(&p.props)),
        }
    }
    fn section(&self, p: &Paragraph) -> String {
        if self.level == Level::Text { String::new() } else { json_of(&p.section).to_string() }
    }
}

/// One output line per paragraph as seen through the lens (joined paragraphs are one line).
struct Line {
    head: String,
    section: String,
    segs: Vec<(String, String)>,
    mark: String,
    mark_revs: String,
}

impl Line {
    fn push(&mut self, attrs: String, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.segs.last_mut() {
            Some((a, t)) if *a == attrs => t.push_str(text),
            _ => self.segs.push((attrs, text.to_string())),
        }
    }
    fn render(&self) -> String {
        let body: String = self.segs.iter().map(|(a, t)| format!("<{a}>{t}")).collect();
        format!("P[{} {}] {body} M[{} {}]", self.head, self.section, self.mark, self.mark_revs)
    }
}

fn line_of(c: &Ctx, p: &Paragraph) -> Line {
    let style = p.props.style.as_deref();
    let mut l = Line { head: c.para_look(p), section: c.section(p), segs: Vec::new(), mark: c.look(style, &p.mark), mark_revs: c.revs(&p.mark) };
    let mut objs = p.objects.iter();
    let mut start = 0;
    for r in &p.runs {
        let seg = p.text.get(start..start + r.len).unwrap_or("");
        start += r.len;
        let gone = c.gone(&r.props);
        let attrs = format!("{} {}", c.revs(&r.props), c.look(style, &r.props));
        for ch in seg.chars() {
            if ch == OBJ {
                let o = objs.next();
                if gone {
                    continue;
                }
                let own_anchor = c.lens == Lens::Member
                    && matches!(o, Some(InlineObject::CommentStart { id } | InlineObject::CommentEnd { id }) if c.own.contains(id));
                if !own_anchor {
                    l.push(attrs.clone(), &format!("[{}]", json_of(&o)));
                }
            } else if !gone {
                l.push(attrs.clone(), ch.encode_utf8(&mut [0u8; 4]));
            }
        }
    }
    if start < p.text.len() {
        l.push("uncovered".into(), p.text.get(start..).unwrap_or(""));
    }
    l
}

/// The paragraph look of a joined line: every piece that holds text in the view (the last piece
/// when none does), repeated looks once.
fn heads(pieces: &mut Vec<(String, bool)>) -> String {
    let any = pieces.iter().any(|(_, t)| *t);
    let n = pieces.len();
    let mut keep: Vec<String> = Vec::new();
    for (i, (h, t)) in pieces.drain(..).enumerate() {
        if (t || (!any && i + 1 == n)) && keep.last() != Some(&h) {
            keep.push(h);
        }
    }
    keep.join(" | ")
}

fn flush(cur: &mut Option<Line>, pieces: &mut Vec<(String, bool)>, out: &mut Vec<String>) {
    if let Some(mut done) = cur.take() {
        done.head = heads(pieces);
        out.push(done.render());
    }
    pieces.clear();
}

fn walk(c: &Ctx, bl: &Blocks, out: &mut Vec<String>, depth: usize) {
    if depth > 16 {
        return;
    }
    let mut cur: Option<Line> = None;
    let mut pieces: Vec<(String, bool)> = Vec::new();
    let mut join = false;
    for b in bl {
        match &**b {
            Block::Para(p) => {
                let l = line_of(c, p);
                let piece = (l.head.clone(), !l.segs.is_empty());
                match cur.as_mut() {
                    Some(prev) if join => {
                        for (a, t) in l.segs {
                            prev.push(a, &t);
                        }
                        // A split gives the original mark's revisions and section to the
                        // second paragraph; the paragraph look of EVERY piece that still holds
                        // text is kept (`heads`).
                        prev.mark_revs = l.mark_revs;
                        prev.section = l.section;
                    }
                    _ => {
                        flush(&mut cur, &mut pieces, out);
                        cur = Some(l);
                    }
                }
                pieces.push(piece);
                join = c.joins(&p.mark);
            }
            Block::Table(t) => {
                flush(&mut cur, &mut pieces, out);
                join = false;
                let mut shell = t.clone();
                for r in &mut shell.rows {
                    for cell in &mut r.cells {
                        cell.blocks.clear();
                    }
                }
                out.push(if c.level == Level::Text { "TABLE".into() } else { format!("TABLE {}", json_of(&shell)) });
                for r in &t.rows {
                    out.push("ROW".into());
                    for cell in &r.cells {
                        out.push("CELL".into());
                        walk(c, &cell.blocks, out, depth + 1);
                    }
                }
                out.push("END".into());
            }
        }
    }
    flush(&mut cur, &mut pieces, out);
}

struct Seen {
    stories: BTreeMap<String, Vec<String>>,
    comments: BTreeMap<u32, String>,
}

fn seen(d: &Document, lens: Lens, level: Level, own: &BTreeSet<u32>) -> Seen {
    let c = Ctx { d, lens, level, own };
    let own_parts: BTreeSet<u32> = d.comments.iter().filter(|(k, _)| own.contains(k)).map(|(_, x)| x.part).collect();
    let mut stories = BTreeMap::new();
    let mut body = Vec::new();
    walk(&c, &d.body, &mut body, 0);
    stories.insert("body".to_string(), body);
    for (id, part) in &d.parts {
        if own_parts.contains(id) {
            continue;
        }
        let mut lines = Vec::new();
        walk(&c, &part.blocks, &mut lines, 0);
        stories.insert(format!("part {id} {:?}", part.kind), lines);
    }
    let comments = d
        .comments
        .iter()
        .filter(|(k, _)| !own.contains(k))
        .map(|(k, x)| (*k, format!("{}|{}|{}|{:?}|{}", x.author, x.initials, x.date, x.parent, x.part)))
        .collect();
    Seen { stories, comments }
}

/// Every story of `b` unchanged in `a` (new stories are only seen through a reference).
fn same(b: &Seen, a: &Seen) -> bool {
    b.comments == a.comments && b.stories.iter().all(|(k, v)| a.stories.get(k) == Some(v))
}

/// Everything outside the stories, comments, revisions and numbering.
fn rest(d: &Document) -> Value {
    let mut v = without(json_of(d), &["body", "parts", "comments", "revisions", "numbering"]);
    if let Some(s) = v.get_mut("settings").and_then(Value::as_object_mut) {
        s.remove("trackChanges");
    }
    json!([v, d.media.keys().collect::<Vec<_>>(), d.passthrough.keys().collect::<Vec<_>>()])
}

fn classify(b: &Document, a: &Document, lens: Lens) -> Class {
    let own: BTreeSet<u32> = if lens == Lens::Member {
        b.comments.iter().chain(a.comments.iter()).filter(|(_, c)| c.author == ME).map(|(k, _)| *k).collect()
    } else {
        BTreeSet::new()
    };
    let at = |lvl| (seen(b, lens, lvl, &own), seen(a, lens, lvl, &own));
    let (tb, ta) = at(Level::Text);
    if !same(&tb, &ta) {
        return Class::Dirty("text");
    }
    let (fb, fa) = at(Level::Allowed);
    if !same(&fb, &fa) {
        return Class::Dirty("format");
    }
    if rest(b) != rest(a) {
        return Class::Dirty("document settings");
    }
    let (sb, sa) = at(Level::Strict);
    let strict = same(&sb, &sa) && json_of(&b.numbering) == json_of(&a.numbering);
    match (strict, lens) {
        (true, _) => Class::Clean,
        (false, Lens::Member) => Class::Format,
        (false, _) => Class::Dirty("judge"),
    }
}

fn lens_for(id: &str) -> Lens {
    if id.starts_with("review.accept") {
        Lens::Accepted
    } else if id.starts_with("review.reject") {
        Lens::Rejected
    } else {
        Lens::Member
    }
}

fn announced_formats(a: &WordApp) -> usize {
    a.session.chat.as_ref().map(|c| c.hub().messages()).unwrap_or_default().iter().filter(|m| m.text.contains(" formatted: ")).count()
}

/// The sample with the member's selection placed; `None` when the place cannot be reached.
fn placed(place: &[(&str, Value)]) -> Option<WordApp> {
    let mut a = sample();
    for (c, p) in place {
        run_as_member(&mut a, ME, c, p.clone()).ok()?;
    }
    Some(a)
}

#[test]
fn every_allowed_command_is_refused_tracked_or_allowed_formatting() {
    // `engine.commands` and `view.page` are answered by the control handler, not run.
    let ids: Vec<&str> =
        AGENT_COMMANDS.iter().flat_map(|(_, ids)| ids.iter().copied()).filter(|id| !matches!(*id, "engine.commands" | "view.page")).collect();
    assert_eq!(ids.len(), 71, "{} commands", ids.len());
    let mut untracked: BTreeSet<&str> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();
    let mut runs = 0;
    let (mut refused, mut formatted) = (0, 0);
    for (place_name, place) in places() {
        if placed(&place).is_none() {
            failures.push(format!("place `{place_name}` cannot be reached"));
            continue;
        }
        for id in &ids {
            for params in [json!({}), representative(id)] {
                runs += 1;
                let what = format!("{id} {params} at {place_name}");
                // Raw effect: the command as the member, tracking on, without the guard.
                let raw_dirty = {
                    let Some(mut raw) = placed(&place) else { continue };
                    if let Some(ms) = raw.member_sel.get(ME).cloned() {
                        raw.session.sel = ms.sel;
                    }
                    raw.session.author = ME.into();
                    if lens_for(id) == Lens::Member {
                        raw.session.doc.settings.track_changes = true;
                    }
                    let before = raw.session.doc.clone();
                    let ok = raw.session.run(id, &params).is_ok();
                    ok && matches!(classify(&before, &raw.session.doc, lens_for(id)), Class::Dirty(_))
                };
                if raw_dirty {
                    untracked.insert(id);
                }
                // Through the gate.
                let Some(mut g) = placed(&place) else { continue };
                let before = g.session.doc.clone();
                let said = announced_formats(&g);
                match run_as_member(&mut g, ME, id, params.clone()) {
                    Err(e) => {
                        refused += 1;
                        if g.session.doc != before {
                            failures.push(format!("{what}: refused ({e}) but the document changed"));
                        }
                        if announced_formats(&g) != said {
                            failures.push(format!("{what}: refused but announced"));
                        }
                    }
                    Ok(_) => {
                        let class = classify(&before, &g.session.doc, lens_for(id));
                        let announced = announced_formats(&g) - said;
                        formatted += announced;
                        if raw_dirty {
                            failures.push(format!("{what}: untracked without the guard, accepted through it ({class:?})"));
                        }
                        match class {
                            Class::Dirty(why) => failures.push(format!("{what}: accepted with an untracked change ({why})")),
                            Class::Format if announced != 1 => failures.push(format!("{what}: formatting announced {announced} times")),
                            Class::Clean if announced != 0 => failures.push(format!("{what}: no formatting but announced")),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    assert!(runs > 900, "{runs} runs");
    eprintln!("sweep: {} commands, {runs} runs, {refused} refused, {formatted} announced as formatting", ids.len());
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
    let got: Vec<&str> = untracked.into_iter().collect();
    if got != UNTRACKED {
        panic!(
            "UNTRACKED changed (review each command, then update the list):\n{}",
            got.iter().map(|i| format!("    \"{i}\",")).collect::<Vec<_>>().join("\n")
        );
    }
}

#[test]
fn core_text_commands_stay_tracked() {
    // If an upstream change made these untracked, members could not edit at all.
    for (place, steps) in [
        ("owner text", vec![("text.insert", json!({"text": "X"})), ("text.delete", json!({})), ("text.backspace", json!({}))]),
        ("owner text", vec![("text.newParagraph", json!({})), ("text.insert", json!({"text": "New"}))]),
        ("owner text", vec![("review.newComment", json!({"text": "note"}))]),
    ] {
        let mut a = sample();
        let _ = run_as_member(&mut a, ME, "select.text", json!({"text": "beta gamma"}));
        for (id, p) in steps {
            let r = run_as_member(&mut a, ME, id, p.clone());
            assert!(r.is_ok(), "{place}: {id} {p}: {r:?}");
        }
    }
}
