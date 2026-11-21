//! The Library pane driven as a person would, over folders of pictures drawn here in code.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft::host::{PickKind, Picked, ScriptedFiles};
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

/// The app in a harness with the craft runtime installed, stepped as the app's driver runs a frame
/// ([`crate::test_harness::frame`]), so a click on a command-bound control runs its command.
fn harness(mut app: WordApp) -> Harness<'static, WordApp> {
    app.ui.language = "en".into();
    let mut h = crate::test_harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    steps(&mut h, 3);
    h
}

fn steps(h: &mut Harness<'_, WordApp>, n: usize) {
    crate::test_harness::frames(h, n);
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
        steps(&mut h, 1);
    }
    assert!(h.state().library.ready());
    steps(&mut h, 4);
    h
}

/// Move there, press, release: one frame each, as a real click arrives.
fn click(h: &mut Harness<'_, WordApp>, pos: egui::Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    steps(h, 1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        steps(h, 1);
    }
    steps(h, 2);
}

#[test]
fn insert_library_opens_the_pane_on_its_pictures_tab() {
    let mut app = app();
    app.run("ui.tab", json!({"tab": "Insert"})).unwrap();
    let mut h = harness(app);
    h.get_by_label("Library").click();
    steps(&mut h, 3);
    assert!(h.state().session.view.library_pane, "the ribbon button opens the pane");
    assert_eq!(h.state().library.tab, super::ITEMS);
    assert_eq!(h.query_all_by_label("Pictures").count(), 2, "the tab strip's Pictures, beside the ribbon's Insert › Pictures");
    // No folders yet: the pane says what to do, and Add Folder… (no picker here) takes a path.
    assert!(h.query_by_label_contains("Add a folder of pictures").is_some());
    let dir = folder("add");
    std::fs::write(dir.join("kiln.png"), png(40, 30, [180, 90, 40])).unwrap();
    h.get_by_label("Add Folder…").click();
    steps(&mut h, 2);
    assert!(h.state().library.adding);
    h.state_mut().library.new_folder = dir.to_string_lossy().into_owned();
    steps(&mut h, 1);
    h.get_by_label("Add").click();
    steps(&mut h, 60);
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
    steps(&mut h, 3);
    let s = &h.state().session;
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    let media: Vec<Vec<u8>> = s.doc.media.values().map(|m| m.to_vec()).collect();
    assert_eq!(media, vec![red], "the file's own picture");
    assert!(!s.sel.is_collapsed(), "the new picture is selected, as Insert › Pictures leaves it");
    let ctx = h.ctx.clone();
    assert!(h.state().canvas.focused(&ctx), "typing carries on in the document");
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
    steps(&mut h, 40);
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
    steps(&mut h, 3);
    assert!(h.state().session.doc.media.is_empty(), "nothing went in");
    assert!(h.state().status_message().is_some_and(|m| !m.text.as_str().is_empty()), "the status line says why");
    h.get_by_label("Picture: good.png").click();
    steps(&mut h, 3);
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
    crate::i18n::apply_language(crate::i18n::Lang::PSEUDO);
    let shown: Vec<String> = super::words().map(|w| craft_library_egui::Host::text(&host, w).into_owned()).collect();
    crate::i18n::apply_language(crate::i18n::Lang::EN);
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

/// The app over the file service `files`.
fn app_over(files: &Arc<ScriptedFiles>) -> WordApp {
    let host =
        craft::host::Host::builder(craft::host::AppInfo { id: "wordcraft-test", name: "WordCraft", version: "0" }).files(files.clone()).build();
    app().with_host(Arc::new(host))
}

/// Add Folder… where the host can pick a folder (#2868): no path field; the folder is asked for
/// and the window goes on drawing while the dialog is open. Its answer adds the folder and shows
/// its pictures; a cancel changes nothing.
#[test]
fn add_folder_asks_the_folder_dialog_and_shows_what_it_picks() {
    let files = Arc::new(ScriptedFiles::held());
    let mut app = app_over(&files);
    app.run("view.libraryPane", json!({"value": true})).unwrap();
    let mut h = harness(app);
    let dir = folder("picker");
    std::fs::write(dir.join("kiln.png"), png(40, 30, [180, 90, 40])).unwrap();

    h.get_by_label("Add Folder…").click();
    steps(&mut h, 3);
    let asked = files.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!((asked[0].kind, asked[0].purpose.as_str()), (PickKind::Folder, "library"));
    assert!(!h.state().library.adding && h.query_by_label("Add").is_none(), "no path field");
    files.answer(Picked::Cancelled);
    steps(&mut h, 3);
    assert!(h.state().session.prefs.library_folders.is_empty(), "a cancel adds nothing");

    h.get_by_label("Add Folder…").click();
    steps(&mut h, 3);
    files.answer(Picked::Paths(vec![dir.clone()]));
    for _ in 0..200 {
        steps(&mut h, 1);
        if h.state().library.ready() && h.query_by_label("Picture: kiln.png").is_some() {
            break;
        }
    }
    let real = std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned();
    assert_eq!(h.state().session.prefs.library_folders, vec![real.clone()]);
    assert_eq!(h.state().library.folder.as_deref(), Some(real.as_str()), "the pane shows the new folder");
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let drawn: Vec<String> = h
        .output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect();
    assert!(drawn.contains(&name), "the folder drop-down names it: {drawn:?}");
    assert!(h.query_by_label("Picture: kiln.png").is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// A file service that answers at once (as the pane draws) still shows the folder it picked: the
/// answer reaches the pane's own state, not the stand-in it has while the pane draws.
#[test]
fn a_folder_picked_at_once_is_shown() {
    let dir = folder("at-once");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![dir.clone()])]));
    let mut app = app_over(&files);
    app.run("view.libraryPane", json!({"value": true})).unwrap();
    let mut h = harness(app);
    h.get_by_label("Add Folder…").click();
    steps(&mut h, 3);
    let real = std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned();
    assert_eq!(h.state().session.prefs.library_folders, vec![real.clone()]);
    assert_eq!(h.state().library.folder.as_deref(), Some(real.as_str()));
    let _ = std::fs::remove_dir_all(dir);
}
