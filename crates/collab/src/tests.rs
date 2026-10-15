//! Replica tests: sharing and joining, typing as character inserts, formatting, Enter, tables,
//! stories, comments, styles, lists, positions, per-user undo, held updates, and a fuzz where
//! two and three replicas edit at once and must converge.

use std::sync::Arc;

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Block, CharProps, Comment, Document, Paragraph, PartKind, Path, Pos, StoryRef, Table, para_block};

use crate::{Replica, canonical};

fn doc_of(lines: &[&str]) -> Document {
    let mut d = Document::new();
    d.body = lines.iter().map(|l| para_block(Paragraph::with_text(l, CharProps::default()))).collect();
    d
}

/// A sharer and a joiner that has the sharer's document.
fn pair(d: &Document) -> (Replica, Document, Replica, Document) {
    let mut a = Replica::new(1);
    let seed = a.seed(d, "T").unwrap();
    let mut b = Replica::new(2);
    b.apply_update(&seed).unwrap();
    let joined = b.refresh(&Document::new()).unwrap().unwrap().doc;
    (a, d.clone(), b, joined)
}

fn text(d: &Document, i: usize) -> String {
    d.para(StoryRef::Body, &Path::top(i)).map(|p| p.text.clone()).unwrap_or_default()
}

/// Delivers `update` to `to`, whose document is `cur`; the document after.
fn deliver(update: Option<Vec<u8>>, to: &mut Replica, cur: &Document) -> Document {
    if let Some(u) = update {
        to.apply_update(&u).unwrap();
    }
    match to.refresh(cur).unwrap() {
        Some(r) => r.doc,
        None => cur.clone(),
    }
}

#[test]
fn joining_reads_the_shared_document() {
    let mut d = doc_of(&["Hello", "World"]);
    d.body.insert(1, Arc::new(Block::Table(Table::new(2, 2, 200.0))));
    let bold = CharProps { bold: Some(true), ..Default::default() };
    d.insert_text(&Pos::body(0, 5), " there", &bold).unwrap();
    let h = d.add_part(PartKind::Header, vec![para_block(Paragraph::with_text("Head", CharProps::default()))]);
    d.last_section.headers.default = Some(h);
    d.insert_object(&Pos::body(2, 0), InlineObject::BookmarkStart { name: "b1".into() }, &CharProps::default()).unwrap();
    let (_, a, _, b) = pair(&d);
    assert_eq!(canonical(&a), canonical(&b));
    assert_eq!(b.parts.get(&h).map(|p| p.kind), Some(PartKind::Header));
}

#[test]
fn typing_is_a_small_update_and_concurrent_typing_merges() {
    let long = "x".repeat(5_000);
    let d = doc_of(&["The fox jumps.", &long]);
    let (mut ra, mut a, mut rb, mut b) = pair(&d);
    a.insert_text(&Pos::body(0, 4), "quick ", &CharProps::default()).unwrap();
    let ua = ra.commit(&a, false).unwrap().unwrap();
    assert!(ua.len() < 200, "typing sent {} bytes", ua.len());
    b.insert_text(&Pos::body(0, 13), " high", &CharProps::default()).unwrap();
    let ub = rb.commit(&b, false).unwrap();
    // Typing in a long paragraph stays small.
    b.insert_text(&Pos::body(1, 2500), "y", &CharProps::default()).unwrap();
    let ub2 = rb.commit(&b, false).unwrap().unwrap();
    assert!(ub2.len() < 200, "typing in a long paragraph sent {} bytes", ub2.len());
    a = deliver(ub, &mut ra, &a);
    a = deliver(Some(ub2), &mut ra, &a);
    b = deliver(Some(ua), &mut rb, &b);
    assert_eq!(text(&a, 0), "The quick fox jumps high.");
    assert_eq!(canonical(&a), canonical(&b));
}

#[test]
fn formatting_and_enter_merge() {
    let d = doc_of(&["Hello world", "Second"]);
    let (mut ra, mut a, mut rb, mut b) = pair(&d);
    a.format_range(&Pos::body(0, 0), &Pos::body(0, 5), &|c| c.bold = Some(true)).unwrap();
    let ua = ra.commit(&a, false).unwrap();
    b.format_range(&Pos::body(0, 3), &Pos::body(0, 11), &|c| c.italic = Some(true)).unwrap();
    b.split_paragraph(&Pos::body(1, 3)).unwrap();
    let ub = rb.commit(&b, false).unwrap();
    a = deliver(ub, &mut ra, &a);
    b = deliver(ua, &mut rb, &b);
    assert_eq!(canonical(&a), canonical(&b));
    let p = a.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(p.props_of_char(0).bold, Some(true));
    assert_eq!(p.props_of_char(4).italic, Some(true));
    assert_eq!(p.props_of_char(4).bold, Some(true));
    assert_eq!(text(&a, 1), "Sec");
    assert_eq!(text(&a, 2), "ond");
}

#[test]
fn stories_comments_styles_and_lists_travel() {
    let d = doc_of(&["Body text"]);
    let (mut ra, mut a, mut rb, mut b) = pair(&d);
    a.set_id_space(ra.id_space());
    b.set_id_space(rb.id_space());
    // Both add a comment, a footnote and a list at once: their ids don't collide.
    for (d, who) in [(&mut a, "Ann"), (&mut b, "Bob")] {
        let part = d.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text(&format!("{who} says"), CharProps::default()))]);
        let id = d.next_comment_id().unwrap();
        d.comments.insert(id, Comment { author: who.into(), part, ..Default::default() });
        d.add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text(who, CharProps::default()))]);
        d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    }
    a.styles.default_chr.size = Some(13.0);
    let ua = ra.commit(&a, false).unwrap();
    let ub = rb.commit(&b, false).unwrap();
    a = deliver(ub, &mut ra, &a);
    b = deliver(ua, &mut rb, &b);
    assert_eq!(canonical(&a), canonical(&b));
    assert_eq!(a.comments.len(), 2);
    assert_eq!(a.parts.values().filter(|p| p.kind == PartKind::Footnote).count(), 2);
    assert_eq!(a.numbering.nums.len(), 2);
    assert_eq!(b.styles.default_chr.size, Some(13.0));
}

#[test]
fn positions_stay_with_their_text() {
    let d = doc_of(&["abc def", "ghi"]);
    let (mut ra, mut a, mut rb, b) = pair(&d);
    let s = rb.sticky(&Pos::body(0, 4)).unwrap();
    let end = rb.sticky(&Pos::body(1, 3)).unwrap();
    a.insert_text(&Pos::body(0, 0), "xx", &CharProps::default()).unwrap();
    a.split_paragraph(&Pos::body(0, 1)).unwrap();
    let ua = ra.commit(&a, false).unwrap().unwrap();
    rb.apply_update(&ua).unwrap();
    let b2 = rb.refresh(&b).unwrap().unwrap().doc;
    assert_eq!(text(&b2, 1), "xabc def");
    assert_eq!(rb.resolve(&s), Some(Pos::body(1, 5)));
    assert_eq!(rb.resolve(&end), Some(Pos::body(2, 3)));
    assert!(rb.resolve(b"\x01\x02").is_none());
}

#[test]
fn undo_is_per_person() {
    let d = doc_of(&["one"]);
    let (mut ra, mut a, mut rb, mut b) = pair(&d);
    a.insert_text(&Pos::body(0, 3), " two", &CharProps::default()).unwrap();
    let ua = ra.commit(&a, false).unwrap();
    // Typing continues the same step.
    a.insert_text(&Pos::body(0, 7), "!", &CharProps::default()).unwrap();
    let ua2 = ra.commit(&a, true).unwrap();
    b.insert_text(&Pos::body(0, 0), "zero ", &CharProps::default()).unwrap();
    let ub = rb.commit(&b, false).unwrap();
    a = deliver(ub, &mut ra, &a);
    b = deliver(ua, &mut rb, &b);
    b = deliver(ua2, &mut rb, &b);
    assert_eq!(text(&a, 0), "zero one two!");
    let undo = ra.undo().unwrap();
    assert!(undo.is_some());
    a = ra.refresh(&a).unwrap().unwrap().doc;
    assert_eq!(text(&a, 0), "zero one");
    b = deliver(undo, &mut rb, &b);
    assert_eq!(text(&b, 0), "zero one");
    let redo = ra.redo().unwrap();
    a = ra.refresh(&a).unwrap().unwrap().doc;
    b = deliver(redo, &mut rb, &b);
    assert_eq!(text(&a, 0), "zero one two!");
    assert_eq!(canonical(&a), canonical(&b));
}

#[test]
fn updates_out_of_order_wait_for_the_gap() {
    let d = doc_of(&["a"]);
    let (mut ra, mut a, mut rb, b) = pair(&d);
    a.insert_text(&Pos::body(0, 1), "b", &CharProps::default()).unwrap();
    let u1 = ra.commit(&a, false).unwrap().unwrap();
    a.insert_text(&Pos::body(0, 2), "c", &CharProps::default()).unwrap();
    let u2 = ra.commit(&a, false).unwrap().unwrap();
    rb.apply_update(&u2).unwrap();
    assert!(rb.has_gaps());
    rb.apply_update(&u1).unwrap();
    assert!(!rb.has_gaps());
    let b2 = rb.refresh(&b).unwrap().unwrap().doc;
    assert_eq!(text(&b2, 0), "abc");
}

#[test]
fn another_apps_document_is_refused_and_garbage_dropped() {
    let mut r = Replica::new(5);
    let g = yrs::Doc::with_client_id(9);
    let u = {
        use yrs::{Map, Transact, WriteTxn};
        let mut t = g.transact_mut();
        let m = t.get_or_insert_map("meta");
        m.insert(&mut t, "app", "gridcraft");
        m.insert(&mut t, "schema", 1);
        t.encode_update_v1()
    };
    assert!(matches!(r.apply_update(&u), Err(crate::CollabError::Mismatch { .. })));
    assert!(r.apply_update(&[0xff; 16]).is_err());
    assert!(r.refresh(&Document::new()).unwrap().is_none());
}

// ---------------------------------------------------------------- fuzz

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
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// A random position in a random paragraph of the body (cells included).
fn pos(rng: &mut Rng, d: &Document) -> Pos {
    let paths = d.para_paths(StoryRef::Body);
    let path = paths.get(rng.below(paths.len())).cloned().unwrap_or_else(|| Path::top(0));
    let p = d.para(StoryRef::Body, &path);
    let offs: Vec<usize> = p.map(|p| p.text.char_indices().map(|(i, _)| i).chain([p.text.len()]).collect()).unwrap_or_else(|| vec![0]);
    Pos { story: StoryRef::Body, path, off: offs[rng.below(offs.len())] }
}

/// A random edit, as the engine's commands make them.
fn edit(rng: &mut Rng, d: &mut Document, tag: &str) {
    let words = ["a", "bé", "c😀", "dd ", " ", "\t", "word"];
    let a = pos(rng, d);
    match rng.below(16) {
        0..=4 => {
            let mut props = CharProps::default();
            if rng.chance(30) {
                props.bold = Some(true);
            }
            let w = format!("{}{}", words[rng.below(words.len())], if rng.chance(20) { tag } else { "" });
            let _ = d.insert_text(&a, &w, &props);
        }
        5 | 6 => {
            let b = pos(rng, d);
            let (x, y) = if a <= b { (a, b) } else { (b, a) };
            let _ = d.delete_range(&x, &y);
        }
        7 => {
            let _ = d.split_paragraph(&a);
        }
        8 | 9 => {
            let b = pos(rng, d);
            let (x, y) = if a <= b { (a, b) } else { (b, a) };
            let which = rng.below(4);
            let _ = d.format_range(&x, &y, &|c| match which {
                0 => c.bold = Some(true),
                1 => c.italic = c.italic.map(|i| !i).or(Some(true)),
                2 => c.size = Some(14.5),
                _ => c.bold = None,
            });
        }
        10 => {
            let align = if rng.chance(50) { wordcraft_doc::Align::Center } else { wordcraft_doc::Align::Right };
            let _ = d.format_paragraphs(&a, &a, &|p| p.align = Some(align));
        }
        11 => {
            let at = rng.below(d.body.len() + 1).min(d.body.len());
            d.body.insert(at, Arc::new(Block::Table(Table::new(1 + rng.below(2), 1 + rng.below(2), 300.0))));
        }
        12 => {
            let _ = d.insert_object(&a, InlineObject::BookmarkStart { name: format!("{tag}{}", rng.below(9)) }, &CharProps::default());
        }
        13 => {
            if d.body.len() > 1 {
                let i = rng.below(d.body.len());
                d.body.remove(i);
            }
        }
        14 => {
            let kind = if rng.chance(50) { PartKind::Footnote } else { PartKind::Header };
            d.add_part(kind, vec![para_block(Paragraph::with_text(tag, CharProps::default()))]);
        }
        _ => {
            let part = d.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text(tag, CharProps::default()))]);
            let id = d.next_comment_id().unwrap();
            d.comments.insert(id, Comment { author: tag.into(), part, ..Default::default() });
        }
    }
    d.ensure_nonempty();
}

struct Peer {
    r: Replica,
    d: Document,
}

fn take(p: &mut Peer, u: &[u8]) -> Result<(), String> {
    p.r.apply_update(u).map_err(|e| e.to_string())?;
    if let Some(r) = p.r.refresh(&p.d).map_err(|e| e.to_string())? {
        p.d = r.doc;
    }
    check(p)
}

/// With `WORDCRAFT_COLLAB_CHECK`: a peer's document is what its shared state reads as.
fn check(p: &Peer) -> Result<(), String> {
    if std::env::var_os("WORDCRAFT_COLLAB_CHECK").is_none() || p.r.has_gaps() {
        return Ok(());
    }
    let mut late = Replica::new(998);
    let _ = late.apply_update(&p.r.full_state());
    let fresh = late.refresh(&Document::new()).ok().flatten().map(|x| canonical(&x.doc)).unwrap_or_default();
    let mine = canonical(&p.d);
    if fresh != mine {
        return Err(format!("a peer's document isn't its shared state: {}", first_difference(&fresh, &mine)));
    }
    Ok(())
}

fn run(seed: u64, n: usize, steps: usize) -> Result<(), String> {
    let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) ^ 0xC0_11AB);
    let mut start = doc_of(&["The first paragraph.", "A second one, longer than the first.", ""]);
    start.body.insert(2, Arc::new(Block::Table(Table::new(2, 2, 300.0))));
    let mut peers: Vec<Peer> = Vec::new();
    let mut first = Replica::new(100);
    let seed_update = first.seed(&start, "fuzz").map_err(|e| e.to_string())?;
    let mut d0 = start.clone();
    d0.set_id_space(first.id_space());
    peers.push(Peer { r: first, d: d0 });
    for i in 1..n {
        let mut r = Replica::new(100 + i as u64);
        r.apply_update(&seed_update).map_err(|e| e.to_string())?;
        let d = r.refresh(&Document::new()).map_err(|e| e.to_string())?.ok_or("no first state")?.doc;
        peers.push(Peer { r, d });
    }
    let mut queue: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut log = Vec::new();
    for step in 0..steps {
        let i = rng.below(n);
        if rng.chance(55) || queue.is_empty() {
            let p = &mut peers[i];
            let undo = rng.chance(8);
            let redo = !undo && rng.chance(4);
            let update = if undo || redo {
                log.push(format!("{step}: {i} {}", if undo { "undo" } else { "redo" }));
                let u = if undo { p.r.undo() } else { p.r.redo() }.map_err(|e| e.to_string())?;
                if let Some(r) = p.r.refresh(&p.d).map_err(|e| e.to_string())? {
                    p.d = r.doc;
                }
                check(p).map_err(|e| format!("step {step}: {e}\n{}", log.join("\n")))?;
                u
            } else {
                let before = canonical(&p.d);
                edit(&mut rng, &mut p.d, &format!("p{i}s{step}"));
                log.push(format!("{step}: {i} edit"));
                let join = rng.chance(30);
                let u = p.r.commit(&p.d, join).map_err(|e| format!("commit: {e}"))?;
                if u.is_none() && before != canonical(&p.d) {
                    return Err(format!("step {step}: an edit made no update"));
                }
                check(p).map_err(|e| format!("step {step}: {e}\n{}", log.join("\n")))?;
                u
            };
            if let Some(u) = update {
                for to in 0..n {
                    if to != i {
                        queue.push((to, u.clone()));
                        if rng.chance(8) {
                            queue.push((to, u.clone()));
                        }
                    }
                }
            }
        } else {
            let k = rng.below(queue.len());
            let (to, u) = queue.remove(k);
            log.push(format!("{step}: {to} receives"));
            take(&mut peers[to], &u).map_err(|e| format!("step {step}: {e}\n{}", log.join("\n")))?;
        }
    }
    while !queue.is_empty() {
        let k = rng.below(queue.len());
        let (to, u) = queue.remove(k);
        take(&mut peers[to], &u)?;
    }
    // A sync round fills what was held.
    for i in 0..n {
        for j in 0..n {
            if i != j {
                let sv = peers[j].r.state_vector();
                let diff = peers[i].r.diff_for(&sv).map_err(|e| e.to_string())?;
                take(&mut peers[j], &diff)?;
            }
        }
    }
    let want = canonical(&peers[0].d);
    for (i, p) in peers.iter().enumerate().skip(1) {
        let got = canonical(&p.d);
        if got != want {
            let fresh = |r: &Replica| {
                let mut late = Replica::new(998);
                let _ = late.apply_update(&r.full_state());
                late.refresh(&Document::new()).ok().flatten().map(|x| canonical(&x.doc)).unwrap_or_default()
            };
            let (f0, fi) = (fresh(&peers[0].r), fresh(&p.r));
            return Err(format!(
                "peer {i} differs from peer 0: {}\n(peer 0 is its shared state: {}, peer {i}: {}; the shared states agree: {})\n--- steps\n{}",
                first_difference(&want, &got),
                f0 == want,
                fi == got,
                f0 == fi,
                log.join("\n")
            ));
        }
    }
    // A newcomer gets the same document.
    let mut late = Replica::new(999);
    late.apply_update(&peers[0].r.full_state()).map_err(|e| e.to_string())?;
    let d = late.refresh(&Document::new()).map_err(|e| e.to_string())?.ok_or("the newcomer got nothing")?.doc;
    if canonical(&d) != want {
        return Err(format!("a newcomer differs ({}): {}", short(&want), first_difference(&want, &canonical(&d))));
    }
    Ok(())
}

/// The first top-level part of two canonical documents that differs.
fn short(j: &serde_json::Value) -> String {
    j.to_string().chars().take(200).collect()
}

fn first_difference(a: &serde_json::Value, b: &serde_json::Value) -> String {
    for k in ["body", "parts", "comments", "styles", "numbering", "settings", "media", "files"] {
        if a[k] != b[k] {
            return format!(
                "{k}:\n{}\nvs\n{}",
                a[k].to_string().chars().take(4000).collect::<String>(),
                b[k].to_string().chars().take(4000).collect::<String>()
            );
        }
    }
    String::new()
}

fn seeds() -> Vec<u64> {
    match std::env::var("WORDCRAFT_COLLAB_SEED").ok().and_then(|v| v.trim().parse().ok()) {
        Some(one) => vec![one],
        None => (0..env_num("WORDCRAFT_COLLAB_SEEDS", 12)).collect(),
    }
}

#[test]
fn two_replicas_converge() {
    let steps = env_num("WORDCRAFT_COLLAB_STEPS", 80) as usize;
    for seed in seeds() {
        if let Err(e) = run(seed, 2, steps) {
            panic!("seed {seed} (WORDCRAFT_COLLAB_SEED={seed}): {e}");
        }
    }
}

#[test]
fn three_replicas_converge() {
    let steps = env_num("WORDCRAFT_COLLAB_STEPS", 80) as usize;
    for seed in seeds() {
        let s = seed + 1_000;
        if let Err(e) = run(s, 3, steps) {
            panic!("seed {s} (WORDCRAFT_COLLAB_SEED={s}): {e}");
        }
    }
}
