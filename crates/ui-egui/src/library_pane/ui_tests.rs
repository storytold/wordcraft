//! The Library pane driven as a person would, over folders of pictures drawn here in code.

use std::path::{Path, PathBuf};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use serde_json::json;
use wordcraft_engine::Session;

use crate::{Services, WordApp};

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-library-pane-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `w` × `h` PNG of one colour.
fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(w, h, Rgba([rgb[0], rgb[1], rgb[2], 255])));
    wordcraft_geom::picture::encode_format(&img, ImageFormat::Png).unwrap()
}

fn app() -> WordApp {
    WordApp::new(Session::new(wordcraft_doc::Document::from_text("Field notes")), Services::default())
}

fn harness(app: WordApp) -> Harness<'static, WordApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

/// The pane open on `dir`, every file read.
fn open_on(dir: &Path) -> Harness<'static, WordApp> {
    let mut app = app();
    app.run("library.addFolder", json!({"path": dir.to_string_lossy()})).unwrap();
    app.run("view.libraryPane", json!({"value": true})).unwrap();
    let mut h = harness(app);
    for _ in 0..200 {
        if h.state().library.ready() {
            break;
        }
        h.step();
    }
    assert!(h.state().library.ready());
    h.run_steps(4);
    h
}

/// Move there, press, release: one frame each, as a real click arrives.
fn click(h: &mut Harness<'_, WordApp>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
}

#[test]
fn insert_library_opens_the_pane_on_its_pictures_tab() {
    let mut app = app();
    app.ui.tab = "Insert".into();
    let mut h = harness(app);
    h.get_by_label("Library").click();
    h.run_steps(3);
    assert!(h.state().session.view.library_pane, "the ribbon button opens the pane");
    assert_eq!(h.state().library.tab, super::ITEMS);
    assert!(h.query_by_label("Pictures").is_some(), "the tab strip");
    // No folders yet: the pane says what to do, and Add Folder… (no picker here) takes a path.
    assert!(h.query_by_label_contains("Add a folder of pictures").is_some());
    let dir = folder("add");
    std::fs::write(dir.join("kiln.png"), png(40, 30, [180, 90, 40])).unwrap();
    h.get_by_label("Add Folder…").click();
    h.run_steps(2);
    assert!(h.state().library.adding);
    h.state_mut().library.new_folder = dir.to_string_lossy().into_owned();
    h.run_steps(1);
    h.get_by_label("Add").click();
    for _ in 0..60 {
        h.step();
    }
    assert_eq!(h.state().session.prefs.library_folders.len(), 1, "kept with the preferences");
    assert!(h.query_by_label("Picture: kiln.png").is_some());
    // The same button closes it again (the topmost "Library": the pane's title is below the ribbon).
    let button = h.get_all_by_label("Library").map(|n| n.rect().center()).min_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
    click(&mut h, button);
    assert!(!h.state().session.view.library_pane);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn clicking_a_picture_inserts_it_selected_in_one_undo_step() {
    let dir = folder("click");
    let red = png(64, 32, [200, 30, 30]);
    std::fs::write(dir.join("red.png"), &red).unwrap();
    std::fs::write(dir.join("teal.png"), png(16, 16, [20, 160, 160])).unwrap();
    let mut h = open_on(&dir);
    let depth = h.state().session.undo_depth();
    h.get_by_label("Picture: red.png").click();
    h.run_steps(3);
    let s = &h.state().session;
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    let media: Vec<Vec<u8>> = s.doc.media.values().map(|m| m.to_vec()).collect();
    assert_eq!(media, vec![red], "the file's own picture");
    assert!(!s.sel.is_collapsed(), "the new picture is selected, as Insert › Pictures leaves it");
    assert!(h.state().canvas.want_focus || h.state().canvas.focused, "typing carries on in the document");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn search_shows_only_the_pictures_that_match() {
    let dir = folder("search");
    std::fs::write(dir.join("harbour.png"), png(8, 8, [10, 60, 200])).unwrap();
    std::fs::write(dir.join("orchard.png"), png(8, 8, [30, 160, 40])).unwrap();
    let mut h = open_on(&dir);
    assert!(h.query_by_label("Picture: harbour.png").is_some() && h.query_by_label("Picture: orchard.png").is_some());
    h.state_mut().library.query = "HARB".into();
    for _ in 0..40 {
        h.step();
    }
    assert_eq!(h.state().library.hits(), Some(1));
    assert!(h.query_by_label("Picture: harbour.png").is_some());
    assert!(h.query_by_label("Picture: orchard.png").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// A damaged picture is listed by its name (listing reads nothing); its thumbnail can't be drawn
/// and a click on it inserts nothing and says why, while the others work as usual.
#[test]
fn a_damaged_picture_is_listed_but_not_inserted() {
    let dir = folder("damaged");
    let good = png(8, 8, [90, 90, 90]);
    std::fs::write(dir.join("good.png"), &good).unwrap();
    std::fs::write(dir.join("bad.png"), b"\x89PNG not really").unwrap();
    let mut h = open_on(&dir);
    assert_eq!(h.state().library.files.len(), 2);
    h.get_by_label("Picture: bad.png").click();
    h.run_steps(3);
    assert!(h.state().session.doc.media.is_empty(), "nothing went in");
    assert!(h.state().status_msg.as_ref().is_some_and(|(m, _)| !m.is_empty()), "the status line says why");
    h.get_by_label("Picture: good.png").click();
    h.run_steps(3);
    let media: Vec<Vec<u8>> = h.state().session.doc.media.values().map(|m| m.to_vec()).collect();
    assert_eq!(media, vec![good]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Every word the pane shows, WordCraft's and the shared pane's own, comes from the interface
/// catalog, so it can be shown in the person's language; placeholders survive for the pane.
#[test]
fn the_pane_speaks_the_interface_language() {
    let mut app = app();
    let host = super::WordHost(&mut app);
    crate::i18n::pseudo::set(true);
    let shown: Vec<String> = super::words().map(|w| craft_library_egui::Host::text(&host, w).into_owned()).collect();
    crate::i18n::pseudo::set(false);
    for (english, w) in super::words().zip(&shown) {
        assert!(w.contains('⟦'), "{english} goes through tl!: {w}");
    }
    let reading = shown.iter().find(|w| w.contains("{read}")).map(String::as_str).unwrap_or_default();
    assert!(reading.contains("{total}"), "placeholders are kept for the pane: {reading}");
    assert!(super::words().any(|w| w == "All folders"), "the shared pane's own words are listed");
    assert_eq!(super::LABELS.nouns, wordcraft_engine::library::NOUNS, "the commands' keys stay as they are");
}

/// The shared pane draws WordCraft's pictures as one grid, each named under its thumbnail.
#[test]
fn pictures_share_one_grid() {
    let dir = folder("grid");
    for (name, rgb) in [("a.png", [200, 40, 40]), ("b.png", [40, 200, 40]), ("c.png", [40, 40, 200])] {
        std::fs::write(dir.join(name), png(30, 20, rgb)).unwrap();
    }
    let h = open_on(&dir);
    let rects: Vec<egui::Rect> = ["a.png", "b.png", "c.png"].iter().map(|n| h.get_by_label(&format!("Picture: {n}")).rect()).collect();
    assert_eq!(rects[0].top(), rects[1].top(), "side by side, not under a heading each");
    assert!(h.query_by_label("a").is_none(), "no per-file heading");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `w` × `h` JPEG of noise, which compresses badly and decodes slowly, as a camera photo does.
fn noisy_jpeg(w: u32, h: u32) -> Vec<u8> {
    let mut seed = 0x2545_f491_u32;
    let img = RgbaImage::from_fn(w, h, |x, y| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let n = (seed & 0x3f) as u8;
        Rgba([(x / 24) as u8 ^ n, (y / 16) as u8 ^ n, n.wrapping_mul(3), 255])
    });
    wordcraft_geom::picture::encode_format(&DynamicImage::ImageRgba8(img), ImageFormat::Jpeg).unwrap()
}

/// The size of the texture the pane draws `file`'s picture with, if it has made one: the pane
/// names them `library {file} {index} {width}`, which the background drawing relies on.
fn texture(h: &Harness<'_, WordApp>, file: &str) -> Option<[usize; 2]> {
    let prefix = format!("library {file} 0 ");
    let textures = h.ctx.tex_manager();
    let textures = textures.read();
    textures.allocated().find(|(_, m)| m.name.starts_with(&prefix)).map(|(_, m)| m.size)
}

/// The file the pane lists whose name ends with `name`.
fn listed(h: &Harness<'_, WordApp>, name: &str) -> String {
    h.state().library.files.iter().find(|f| f.file.ends_with(name)).map(|f| f.file.clone()).unwrap()
}

/// Step, a frame every 10 ms, until `done`, at most a minute; the slowest frame.
fn step_until(h: &mut Harness<'_, WordApp>, done: impl Fn(&Harness<'_, WordApp>) -> bool) -> std::time::Duration {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut slowest = std::time::Duration::ZERO;
    while !done(h) {
        assert!(std::time::Instant::now() < until, "not within a minute");
        let t = std::time::Instant::now();
        h.step();
        slowest = slowest.max(t.elapsed());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    slowest
}

#[test]
fn a_large_photo_never_holds_up_a_frame_and_its_thumbnail_arrives() {
    let jpeg = noisy_jpeg(4000, 3000);
    let t = std::time::Instant::now();
    wordcraft_engine::library::draw_thumbnail(&jpeg, 256).unwrap();
    let decode = t.elapsed();
    let dir = folder("large");
    std::fs::write(dir.join("harbour.jpg"), &jpeg).unwrap();
    let mut h = harness(app());
    h.state_mut().run("library.addFolder", json!({"path": dir.to_string_lossy()})).unwrap();
    h.state_mut().run("view.libraryPane", json!({"value": true})).unwrap();
    // The first frames list the folder and ask for the thumbnail: a placeholder of the pane's
    // blank-cell shape comes back while the photo is drawn on the worker.
    let mut slowest = step_until(&mut h, |h| h.state().library.ready() && texture(h, &listed(h, "harbour.jpg")).is_some());
    let file = listed(&h, "harbour.jpg");
    assert_eq!(texture(&h, &file), Some([4, 3]), "the placeholder");
    assert!(h.state().library_thumbs.drawing() || h.state().library_thumbs.waiting().0 == 1, "drawn on the worker");
    // Frames go on while it is drawn; then the pane's texture is the thumbnail.
    slowest = slowest.max(step_until(&mut h, |h| texture(h, &file).is_some_and(|s| s[0] > 4)));
    let [w, ht] = texture(&h, &file).unwrap();
    assert!(w >= 128 && (ht as f32 / w as f32 - 0.75).abs() < 0.02, "{w} × {ht}");
    assert_eq!(h.state().library_thumbs.waiting(), (0, 0));
    assert!(slowest < decode / 2, "a frame took {slowest:?}; drawing the photo takes {decode:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A PNG whose header says `w` × `h`, with no pixels to speak of: listed, never drawn.
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
fn a_picture_that_cannot_be_drawn_shows_the_question_mark() {
    let dir = folder("bomb");
    std::fs::write(dir.join("bomb.png"), png_claiming(100_000, 100_000)).unwrap();
    let mut h = open_on(&dir);
    let file = listed(&h, "bomb.png");
    assert!(h.state().library.files.iter().all(|f| f.error.is_none()), "its header reads");
    step_until(&mut h, |h| texture(h, &file).is_some_and(|s| s != [4, 3]));
    assert_eq!(texture(&h, &file), Some(super::thumbs::FAILED), "the \"?\"");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn another_folder_or_closing_the_pane_drops_the_photos_waiting() {
    let photos = folder("waiting-photos");
    let jpeg = noisy_jpeg(4000, 3000);
    for name in ["a.jpg", "b.jpg", "c.jpg", "d.jpg"] {
        std::fs::write(photos.join(name), &jpeg).unwrap();
    }
    let empty = folder("waiting-empty");
    let mut h = open_on(&photos);
    h.state_mut().run("library.addFolder", json!({"path": empty.to_string_lossy()})).unwrap();
    h.state_mut().library.folder = None;
    step_until(&mut h, |h| h.state().library_thumbs.waiting().0 >= 2);
    // Another folder: what waits is dropped (the photo being drawn finishes), and the pane reads
    // its folders again, so none of its placeholders is left blank.
    let shown = h.state().session.prefs.library_folders.iter().find(|f| f.contains("waiting-empty")).cloned();
    h.state_mut().library.folder = shown;
    h.step();
    assert_eq!(h.state().library_thumbs.waiting(), (0, 0));
    // All folders again: the photos are asked for again.
    h.state_mut().library.folder = None;
    step_until(&mut h, |h| h.state().library_thumbs.waiting().0 >= 1);
    // The pane closed: dropped in the same frame.
    h.state_mut().run("view.libraryPane", json!({"value": false})).unwrap();
    h.step();
    assert_eq!(h.state().library_thumbs.waiting(), (0, 0));
    assert!(!h.state().library.ready(), "the pane reads its folders again when it opens");
    let _ = std::fs::remove_dir_all(photos);
    let _ = std::fs::remove_dir_all(empty);
}
