//! The background drawer's bounds, order, cancelling, confinement and hostile pictures, over
//! files written here in code.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

use super::*;
use crate::file_access::{AutomationRoots, confine};
use crate::library::{Item, Nouns};

const WAIT: Duration = Duration::from_secs(60);

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-library-drawer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

fn stamp(path: &Path) -> Stamp {
    let m = std::fs::metadata(path).unwrap();
    Stamp { len: m.len(), modified: m.modified().ok() }
}

fn key(path: &Path, width: u32) -> Key {
    Key { path: path.to_path_buf(), item: 0, width }
}

/// A file named `name` in `dir` holding `bytes`, and its key at `width`.
fn file(dir: &Path, name: &str, bytes: &[u8], width: u32) -> (Key, Stamp) {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    (key(&p, width), stamp(&p))
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, Rgba([30, 120, 200, 255])));
    wordcraft_geom::picture::encode_format(&img, ImageFormat::Png).unwrap()
}

/// A kind whose thumbnails each wait until the test lets one go, saying when one starts: so a
/// test knows what is being drawn and what is waiting without timing anything.
struct Gated {
    started: Sender<String>,
    go: Mutex<Receiver<()>>,
}

impl Kind for Gated {
    type Doc = String;
    fn extensions(&self) -> &[&str] {
        &["txt"]
    }
    fn nouns(&self) -> Nouns {
        Nouns { one: "page", many: "pages" }
    }
    fn open(&self, name: &str, bytes: &[u8]) -> Result<String, String> {
        if bytes == b"panic" {
            panic!("a hostile decoder");
        }
        Ok(name.to_string())
    }
    fn title(&self, _doc: &String) -> Option<String> {
        None
    }
    fn count(&self, _doc: &String) -> usize {
        1
    }
    fn item(&self, _doc: &String, _index: usize) -> Item {
        Item::default()
    }
    fn thumbnail(&self, doc: &String, _index: usize, width: u32) -> Result<Thumbnail, String> {
        let _ = self.started.send(doc.clone());
        let _ = self.go.lock().unwrap().recv();
        Ok(Thumbnail { png: Arc::new(vec![0; width as usize]), width, height: 1 })
    }
}

/// A gated drawer: what starts, the gate, and a signal for each finished thumbnail.
struct Rig {
    drawer: Drawer<Gated>,
    started: Receiver<String>,
    go: Sender<()>,
    woke: Receiver<()>,
}

fn rig(limits: Limits) -> Rig {
    let (started_tx, started) = channel();
    let (go, go_rx) = channel();
    let (woke_tx, woke) = channel();
    let kind = Gated { started: started_tx, go: Mutex::new(go_rx) };
    let drawer = Drawer::with_limits(kind, limits, move || {
        let _ = woke_tx.send(());
    })
    .unwrap();
    Rig { drawer, started, go, woke }
}

impl Rig {
    /// Let the picture being drawn finish, and wait until it has.
    fn finish_one(&self) {
        self.go.send(()).unwrap();
        self.woke.recv_timeout(WAIT).unwrap();
    }
    fn next_started(&self) -> String {
        self.started.recv_timeout(WAIT).unwrap()
    }
}

#[test]
fn the_newest_picture_is_drawn_first_and_at_most_the_limit_wait() {
    let dir = folder("order");
    let r = rig(Limits { queue: 4, ..Limits::default() });
    let (a, sa) = file(&dir, "a.txt", b"a", 64);
    r.drawer.request(a, sa).unwrap();
    assert_eq!(r.next_started(), "a.txt");
    assert!(r.drawer.busy());
    for name in ["b.txt", "c.txt", "d.txt", "e.txt", "f.txt"] {
        let (k, s) = file(&dir, name, name.as_bytes(), 64);
        r.drawer.request(k, s).unwrap();
    }
    assert_eq!(r.drawer.queued(), 4, "the oldest one waiting (b) was dropped");
    assert!(!r.drawer.pending(&key(&dir.join("b.txt"), 64)));
    // Asked for again: it goes to the front, once.
    let (d, sd) = (key(&dir.join("d.txt"), 64), stamp(&dir.join("d.txt")));
    r.drawer.request(d.clone(), sd).unwrap();
    assert_eq!(r.drawer.queued(), 4);
    let mut order = vec![];
    for _ in 0..5 {
        r.finish_one();
        if let Ok(name) = r.started.recv_timeout(Duration::from_millis(200)) {
            order.push(name);
        }
    }
    assert_eq!(order, ["d.txt", "f.txt", "e.txt", "c.txt"]);
    assert_eq!(r.drawer.queued(), 0);
    // Finished keys nobody took are bounded as the queue is: the oldest (a) went.
    let done = r.drawer.take_done();
    let names: Vec<_> = done.iter().filter_map(|(k, _)| k.path.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
    assert!(done.iter().all(|(_, t)| t.as_ref().is_ok_and(|t| t.width == 64)));
    assert_eq!(names, ["d.txt", "f.txt", "e.txt", "c.txt"]);
    assert!(r.drawer.take_done().is_empty(), "taken once");
    assert!(matches!(r.drawer.get(&d, sd), Some(Ok(t)) if t.width == 64));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cancelling_drops_what_waits_and_the_picture_being_drawn_finishes() {
    let dir = folder("cancel");
    let r = rig(Limits::default());
    let (a, sa) = file(&dir, "a.txt", b"a", 64);
    r.drawer.request(a.clone(), sa).unwrap();
    assert_eq!(r.next_started(), "a.txt");
    for name in ["b.txt", "c.txt"] {
        let (k, s) = file(&dir, name, b"x", 64);
        r.drawer.request(k, s).unwrap();
    }
    assert_eq!(r.drawer.cancel(), 2);
    assert_eq!(r.drawer.queued(), 0);
    r.finish_one();
    assert!(r.started.recv_timeout(Duration::from_millis(300)).is_err(), "nothing that was dropped is drawn");
    let done: Vec<Key> = r.drawer.take_done().into_iter().map(|(k, _)| k).collect();
    assert_eq!(done, vec![a.clone()]);
    assert!(r.drawer.get(&a, sa).is_some(), "the one that finished is kept");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn kept_thumbnails_stay_within_their_count_and_bytes() {
    let dir = folder("kept");
    let r = rig(Limits { kept: 3, kept_bytes: 250, ..Limits::default() });
    let mut keys = vec![];
    for i in 0..6 {
        let (k, s) = file(&dir, &format!("{i}.txt"), b"x", 100);
        r.drawer.request(k.clone(), s).unwrap();
        r.finish_one();
        keys.push((k, s));
        let (n, bytes) = r.drawer.kept();
        assert!(n <= 3 && bytes <= 250, "{n} kept, {bytes} bytes");
    }
    // Each is 100 bytes, so two fit; the newest stay. Finished ones not taken are bounded alike.
    assert_eq!(r.drawer.kept(), (2, 200));
    assert_eq!(r.drawer.take_done().len(), 2);
    let (last, s) = keys.last().unwrap().clone();
    assert!(r.drawer.get(&last, s).is_some());
    let (first, s) = keys.first().unwrap().clone();
    assert!(r.drawer.get(&first, s).is_none());
    // A thumbnail of a file that has changed since is not answered.
    assert!(r.drawer.get(&last, Stamp { len: s.len + 1, ..s }).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Wait for one thumbnail with the real picture kind.
fn drawn(d: &Drawer, woke: &Receiver<()>, k: &Key, s: Stamp) -> Result<Thumbnail, String> {
    let until = Instant::now() + WAIT;
    loop {
        if let Some(t) = d.get(k, s) {
            return t;
        }
        assert!(Instant::now() < until, "{k:?} was never drawn");
        let _ = woke.recv_timeout(Duration::from_millis(500));
    }
}

fn pictures(limits: Limits) -> (Drawer, Receiver<()>) {
    let (tx, woke) = channel();
    let d = Drawer::with_limits(Pictures, limits, move || {
        let _ = tx.send(());
    })
    .unwrap();
    (d, woke)
}

#[test]
fn a_file_past_the_size_limit_is_not_read() {
    let dir = folder("huge");
    let (d, woke) = pictures(Limits { file_bytes: 1 << 20, ..Limits::default() });
    // A sparse file: as large as it says, without writing it.
    let p = dir.join("huge.png");
    std::fs::File::create(&p).unwrap().set_len((1 << 20) + 1).unwrap();
    let k = key(&p, 64);
    d.request(k.clone(), stamp(&p)).unwrap();
    let e = drawn(&d, &woke, &k, stamp(&p)).unwrap_err();
    assert!(e.contains("larger than"), "{e}");
    // Also when the stamp it was asked with is out of date: the read itself refuses it.
    let small = Stamp { len: 10, ..stamp(&p) };
    d.request(k.clone(), small).unwrap();
    let e = drawn(&d, &woke, &k, small).unwrap_err();
    assert!(e.contains("limit"), "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A PNG whose header says `w` × `h`, with almost no pixel data: a decompression bomb.
fn png_claiming(w: u32, h: u32) -> Vec<u8> {
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for b in bytes {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 == 1 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc(&body).to_be_bytes());
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn hostile_pictures_are_errors_quickly_and_never_stop_the_worker() {
    let dir = folder("hostile");
    let (d, woke) = pictures(Limits::default());
    let noise: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    let jpeg =
        wordcraft_geom::picture::encode_format(&DynamicImage::ImageRgba8(RgbaImage::from_pixel(64, 64, Rgba([9, 9, 9, 255]))), ImageFormat::Jpeg)
            .unwrap();
    let truncated = jpeg.get(..jpeg.len() / 2).unwrap().to_vec();
    // (A decoder may draw what a truncated JPEG holds, so only that one may succeed.)
    let cases: Vec<(&str, Vec<u8>, bool)> = vec![
        ("noise.png", noise, true),
        ("empty.png", vec![], true),
        ("truncated.jpg", truncated, false),
        ("bomb.png", png_claiming(100_000, 100_000), true),
        ("wide.png", png_claiming(u32::MAX, 1), true),
    ];
    let started = Instant::now();
    for (name, bytes, fails) in &cases {
        let (k, s) = file(&dir, name, bytes, 512);
        d.request(k.clone(), s).unwrap();
        let r = drawn(&d, &woke, &k, s);
        assert!(!fails || r.is_err(), "{name}: {r:?}");
    }
    // The bomb is refused from its header, not decoded.
    assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
    // A picture item that isn't there.
    let (k, s) = file(&dir, "fine.png", &png(30, 20), 64);
    let two = Key { item: 1, ..k.clone() };
    d.request(two.clone(), s).unwrap();
    assert!(drawn(&d, &woke, &two, s).unwrap_err().contains("isn't one of them"));
    // And the worker still draws the next one.
    d.request(k.clone(), s).unwrap();
    let t = drawn(&d, &woke, &k, s).unwrap();
    assert_eq!((t.width, t.height), (30, 20), "never enlarged");
    assert!(d.alive());

    // A decoder that panics is that picture's error.
    let r = rig(Limits::default());
    let (bad, sb) = file(&dir, "bad.txt", b"panic", 64);
    r.drawer.request(bad.clone(), sb).unwrap();
    r.woke.recv_timeout(WAIT).unwrap();
    assert!(r.drawer.get(&bad, sb).unwrap().unwrap_err().contains("could not be drawn"));
    assert!(r.drawer.alive());
    let (ok, so) = file(&dir, "ok.txt", b"ok", 64);
    r.drawer.request(ok.clone(), so).unwrap();
    assert_eq!(r.next_started(), "ok.txt");
    r.finish_one();
    assert!(r.drawer.get(&ok, so).unwrap().is_ok());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reads_stay_inside_the_read_roots_in_force_where_they_were_asked_for() {
    let base = folder("roots");
    let (inside, outside) = (base.join("inside"), base.join("outside"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let (kin, sin) = file(&inside, "in.png", &png(8, 8), 64);
    let (kout, sout) = file(&outside, "out.png", &png(8, 8), 64);
    let (d, woke) = pictures(Limits::default());
    let roots = AutomationRoots::new(Some(&inside), None).unwrap().unwrap();
    confine(Some(&roots), || {
        d.request(kin.clone(), sin).unwrap();
        d.request(kout.clone(), sout).unwrap();
    });
    assert!(drawn(&d, &woke, &kin, sin).is_ok());
    let e = drawn(&d, &woke, &kout, sout).unwrap_err();
    assert!(e.contains("automation"), "{e}");
    // Asked for with no roots in force, the same file is read.
    let wide = Key { width: 65, ..kout };
    d.request(wide.clone(), sout).unwrap();
    assert!(drawn(&d, &woke, &wide, sout).is_ok());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn dropping_the_drawer_does_not_wait_for_the_picture_being_drawn() {
    let dir = folder("drop");
    let r = rig(Limits::default());
    let (a, sa) = file(&dir, "a.txt", b"a", 64);
    r.drawer.request(a, sa).unwrap();
    assert_eq!(r.next_started(), "a.txt");
    let Rig { drawer, go, woke, .. } = r;
    let t = Instant::now();
    drop(drawer);
    assert!(t.elapsed() < Duration::from_secs(1));
    // Let it go: the worker finishes that picture and stops.
    go.send(()).unwrap();
    woke.recv_timeout(WAIT).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_pane_is_answered_at_once_and_as_the_command_answers_once_drawn() {
    let dir = folder("ask");
    std::fs::write(dir.join("kiln.png"), png(60, 40)).unwrap();
    let mut ss = Session::new(wordcraft_doc::Document::new());
    ss.run("library.addFolder", &json!({"path": dir.to_string_lossy()})).unwrap();
    let (d, woke) = pictures(Limits::default());
    let p = json!({"file": dir.join("kiln.png").to_string_lossy(), "picture": 0, "width": 96});
    let Answer::Drawing(k) = d.ask(&ss, &p).unwrap() else { panic!("not drawn yet") };
    assert_eq!(k.width, 96);
    let s = stamp(&dir.join("kiln.png"));
    drawn(&d, &woke, &k, s).unwrap();
    let Answer::Drawn(v) = d.ask(&ss, &p).unwrap() else { panic!("drawn by now") };
    assert_eq!(v, ss.run("library.thumbnail", &p).unwrap());
    // Errors are the command's: a file outside the Library, a bad width, a missing index.
    let outside = folder("ask-outside").join("x.png");
    std::fs::write(&outside, png(4, 4)).unwrap();
    assert!(d.ask(&ss, &json!({"file": outside.to_string_lossy(), "picture": 0})).unwrap_err().contains("not in a Library folder"));
    assert!(d.ask(&ss, &json!({"file": dir.join("kiln.png").to_string_lossy(), "picture": 0, "width": 9000})).is_err());
    assert!(d.ask(&ss, &json!({"file": dir.join("kiln.png").to_string_lossy()})).is_err());
    // A file changed since it was drawn is drawn again.
    std::fs::write(dir.join("kiln.png"), png(200, 150)).unwrap();
    assert!(matches!(d.ask(&ss, &p).unwrap(), Answer::Drawing(_)));
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(outside.parent().unwrap());
}
