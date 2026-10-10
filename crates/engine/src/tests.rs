use serde_json::json;
use wordcraft_doc::{Pos, StoryRef};

use crate::{Session, cmd};

fn s() -> Session {
    Session::new(wordcraft_doc::Document::new())
}

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn text(s: &Session) -> String {
    s.doc.plain_text(StoryRef::Body)
}

#[test]
fn every_command_has_unique_id() {
    let reg = cmd::registry();
    let mut ids: Vec<&str> = reg.all().iter().map(|c| c.id).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate command ids");
    assert!(n > 150, "{n} commands");
}

#[test]
fn typing_enter_undo() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "text.insert", json!({"text": " world"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "Second"}));
    assert_eq!(text(&s), "Hello world\nSecond");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "Hello world\n");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "Hello world");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "Hello world");
}

#[test]
fn joined_commands_are_one_undo_step() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Hello"}));
    run(&mut s, "para.indents", json!({"left": 9.0}));
    run(&mut s, "para.indents", json!({"left": 18.0}));
    for x in [27.0, 36.0] {
        s.join_next_undo();
        run(&mut s, "para.indents", json!({"left": x}));
    }
    let indent = |s: &Session| s.doc.para_at(&s.sel.focus).and_then(|p| p.props.indent_left);
    assert_eq!(indent(&s), Some(36.0));
    // One undo reverts the whole second drag, not just its last frame.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(indent(&s), Some(9.0));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(indent(&s), None);
    assert_eq!(text(&s), "Hello");
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(indent(&s), Some(36.0));
}

#[test]
fn backspace_and_delete() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "abc"}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(text(&s), "ab");
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "cd"}));
    run(&mut s, "caret.home", json!({}));
    run(&mut s, "text.backspace", json!({}));
    assert_eq!(text(&s), "abcd");
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "text.delete", json!({}));
    assert_eq!(text(&s), "bcd");
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "text.deleteWordBack", json!({}));
    assert_eq!(text(&s), "");
}

#[test]
fn bold_toggles_selection_and_caret_word() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "make this bold"}));
    run(&mut s, "select.text", json!({"text": "this"}));
    run(&mut s, "format.bold", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(5).bold, Some(true));
    assert_eq!(p.props_of_char(0).bold, None);
    run(&mut s, "format.bold", json!({}));
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().props_of_char(5).bold, Some(false));
    // Caret inside a word formats the word.
    run(&mut s, "caret.set", json!({"pos": {"story": "body", "path": [0], "off": 11}}));
    run(&mut s, "format.italic", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(10).italic, Some(true));
    assert_eq!(p.props_of_char(13).italic, Some(true));
    assert_eq!(p.props_of_char(9).italic, None);
}

#[test]
fn pending_format_applies_to_typing() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "a "}));
    run(&mut s, "format.bold", json!({}));
    run(&mut s, "text.insert", json!({"text": "b"}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.props_of_char(2).bold, Some(true));
    assert_eq!(p.props_of_char(0).bold, None);
}

#[test]
fn styles_and_lists() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Title"}));
    run(&mut s, "para.heading1", json!({}));
    run(&mut s, "text.newParagraph", json!({}));
    // Heading's next style is Normal.
    assert_eq!(s.doc.para_at(&s.sel.focus).unwrap().props.style.as_deref(), Some("Normal"));
    run(&mut s, "para.bullets", json!({}));
    run(&mut s, "text.insert", json!({"text": "one"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.insert", json!({"text": "two"}));
    let n1 = s.doc.para_at(&Pos::body(1, 0)).unwrap().props.numbering.unwrap();
    let n2 = s.doc.para_at(&Pos::body(2, 0)).unwrap().props.numbering.unwrap();
    assert_eq!(n1.num, n2.num);
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "text.newParagraph", json!({})); // empty item ends the list
    assert_eq!(s.doc.para_at(&s.sel.focus).unwrap().props.numbering.map(|n| n.num), Some(0));
    // "1. " autoformat.
    run(&mut s, "text.insert", json!({"text": "1."}));
    run(&mut s, "text.insert", json!({"text": " "}));
    assert!(s.doc.para_at(&s.sel.focus).unwrap().props.numbering.is_some_and(|n| n.num != 0));
}

#[test]
fn applying_a_heading_clears_list_numbering() {
    // Applying a heading (or Title/Normal) to a numbered paragraph removes its direct list
    // numbering. A dead `&& id == "Normal"` term used to let only Normal clear it, so headings
    // kept the numbering.
    for style_cmd in ["para.heading1", "para.heading2", "para.normal"] {
        let mut s = s();
        run(&mut s, "text.insert", json!({"text": "item"}));
        run(&mut s, "para.bullets", json!({}));
        assert!(
            s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.is_some_and(|n| n.num != 0),
            "{style_cmd}: expected list numbering before applying the style"
        );
        run(&mut s, style_cmd, json!({}));
        assert!(
            !s.doc.para_at(&Pos::body(0, 0)).unwrap().props.numbering.is_some_and(|n| n.num != 0),
            "{style_cmd}: should clear direct list numbering"
        );
    }
}

#[test]
fn find_replace() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "cat dog cat\nCat bird"}));
    let r = run(&mut s, "edit.find", json!({"text": "cat"}));
    assert_eq!(r["count"], 3);
    let r = run(&mut s, "edit.replaceAll", json!({"text": "cat", "with": "fox", "matchCase": true}));
    assert_eq!(r["replaced"], 2);
    assert_eq!(text(&s), "fox dog fox\nCat bird");
    let r = run(&mut s, "edit.find", json!({"text": "\\b\\w{3}\\b", "regex": true, "matchCase": false}));
    assert_eq!(r["count"], 4);
}

#[test]
fn clipboard_round_trip() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "alpha beta\ngamma"}));
    run(&mut s, "select.range", json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 1, "off": 2}}));
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard_text, "beta\nga");
    run(&mut s, "caret.docEnd", json!({}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(text(&s), "alpha beta\ngammabeta\nga");
    run(&mut s, "select.all", json!({}));
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(text(&s), "");
    run(&mut s, "edit.paste", json!({"text": "plain\ntext"}));
    assert_eq!(text(&s), "plain\ntext");
}

#[test]
fn tables_commands() {
    let mut s = s();
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 3}));
    assert!(s.sel.focus.path.cell().is_some());
    run(&mut s, "text.insert", json!({"text": "A1"}));
    run(&mut s, "text.tab", json!({}));
    run(&mut s, "text.insert", json!({"text": "B1"}));
    run(&mut s, "table.insertRowBelow", json!({}));
    run(&mut s, "table.insertColumnRight", json!({}));
    let t = s.doc.body.iter().find_map(|b| b.as_table()).unwrap();
    assert_eq!(t.rows.len(), 3);
    assert_eq!(t.cols(), 4);
    run(&mut s, "table.deleteTable", json!({}));
    assert!(s.doc.body.iter().all(|b| b.as_table().is_none()));
    assert!(s.run("table.merge", &json!({})).is_err());
}

#[test]
fn page_setup_and_breaks() {
    let mut s = s();
    run(&mut s, "layout.orientation", json!({"value": "landscape"}));
    assert!(s.doc.last_section.landscape);
    run(&mut s, "layout.size", json!({"name": "A4"}));
    assert!((s.doc.last_section.page_w - 841.89).abs() < 0.1);
    run(&mut s, "layout.margins", json!({"preset": "narrow"}));
    assert_eq!(s.doc.last_section.margin_left, 36.0);
    run(&mut s, "text.insert", json!({"text": "one"}));
    run(&mut s, "layout.break", json!({"kind": "nextPage"}));
    run(&mut s, "text.insert", json!({"text": "two"}));
    assert_eq!(s.doc.sections().len(), 2);
    assert_eq!(s.layout().pages.len(), 2);
    assert!(s.run("layout.margins", &json!({"left": 1000.0})).is_err());
}

#[test]
fn track_changes_and_accept() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "original"}));
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "text.insert", json!({"text": " added"}));
    run(&mut s, "select.text", json!({"text": "orig"}));
    s.author = "Someone Else".into();
    run(&mut s, "text.delete", json!({}));
    // Deleted text stays (marked) until accepted.
    assert_eq!(text(&s), "original added");
    let ch = run(&mut s, "review.changes", json!({}));
    assert_eq!(ch.as_array().unwrap().len(), 2);
    run(&mut s, "review.acceptAll", json!({}));
    assert_eq!(text(&s), "inal added");
}

#[test]
fn comments() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "Some text here"}));
    run(&mut s, "select.text", json!({"text": "text"}));
    let r = run(&mut s, "review.newComment", json!({"text": "Nice"}));
    let id = r["id"].as_u64().unwrap();
    let l = run(&mut s, "review.comments", json!({}));
    assert_eq!(l[0]["text"], "Nice");
    assert_eq!(text(&s), "Some text here");
    run(&mut s, "review.deleteComment", json!({"id": id}));
    assert!(s.doc.comments.is_empty());
    assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().objects.len(), 0);
}

#[test]
fn toc_and_fields() {
    let mut s = Session::new(crate::sample::report());
    run(&mut s, "caret.docStart", json!({}));
    run(&mut s, "references.toc", json!({}));
    let t = text(&s);
    assert!(t.contains("Contents"), "{t}");
    assert!(t.contains("Summary\t1"), "{t}");
    run(&mut s, "references.updateToc", json!({}));
    assert_eq!(s.doc.plain_text(StoryRef::Body).matches("Summary\t").count(), 1, "one TOC entry after update");
}

#[test]
fn toc_page_numbers_follow_headings() {
    let mut s = s();
    run(&mut s, "references.toc", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    for name in ["Alpha", "Bravo", "Charlie"] {
        run(&mut s, "text.pageBreak", json!({}));
        run(&mut s, "text.insert", json!({"text": name}));
        run(&mut s, "para.heading1", json!({}));
        run(&mut s, "text.newParagraph", json!({}));
    }
    run(&mut s, "references.updateToc", json!({}));
    let t = text(&s);
    for (name, page) in [("Alpha", 2), ("Bravo", 3), ("Charlie", 4)] {
        assert!(t.contains(&format!("{name}\t{page}")), "{name} should be on page {page}: {t}");
    }
}

#[test]
fn failed_command_leaves_document_unchanged() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "keep"}));
    let before = text(&s);
    assert!(s.run("format.size", &json!({"size": -3})).is_err());
    assert!(s.run("no.such", &json!({})).is_err());
    assert!(s.run("text.insert", &json!({})).is_err());
    assert_eq!(text(&s), before);
}

#[test]
fn hostile_params_never_panic() {
    let reg = cmd::registry();
    let junk = [
        json!(null),
        json!({}),
        json!({"text": 5, "size": "x", "path": [], "pos": {"block": 9999, "off": 99999}}),
        json!({"value": -1e308, "rows": 1e9}),
    ];
    for spec in reg.all() {
        if spec.id.starts_with("file.") || spec.id == "insert.picture" || spec.id == "insert.textFromFile" {
            continue;
        }
        for j in &junk {
            let mut s = Session::new(crate::sample::sample_document());
            let _ = s.run(spec.id, j);
            s.clamp_selection();
            let _ = s.layout();
        }
    }
}

#[test]
fn inspect_reports_structure() {
    let mut s = Session::new(crate::sample::sample_document());
    let r = run(&mut s, "document.inspect", json!({}));
    assert!(r["pages"].as_u64().unwrap() >= 1);
    assert!(r["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "table"));
    let f = run(&mut s, "format.state", json!({}));
    assert_eq!(f["styleName"], "Title");
}

#[test]
fn caret_navigation() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "first line\nsecond line"}));
    run(&mut s, "caret.down", json!({}));
    assert_eq!(s.sel.focus.path.last(), 1);
    run(&mut s, "caret.end", json!({}));
    assert_eq!(s.sel.focus.off, 11);
    run(&mut s, "caret.up", json!({"extend": true}));
    assert_eq!(s.sel.focus.path.last(), 0);
    assert!(!s.sel.is_collapsed());
    run(&mut s, "caret.wordLeft", json!({}));
    run(&mut s, "caret.docEnd", json!({}));
    assert_eq!(s.sel.focus, Pos::body(1, 11));
    run(&mut s, "caret.left", json!({}));
    assert_eq!(s.sel.focus.off, 10);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_pictures_load_relative_to_the_file() {
    // Issue #98: `<img src="logo.png">` beside an HTML file is embedded when it is opened.
    let dir = std::env::temp_dir().join(format!("wordcraft-html-img-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("img")).unwrap();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(dir.join("img/my logo.png"), &png).unwrap();
    std::fs::write(dir.join("secret.txt"), b"not a picture").unwrap();
    let html = r#"<p><img src="img/my%20logo.png" alt="Logo"></p><p><img src="secret.txt" alt="T"></p><p><img src="http://example.com/x.png" alt="Web"></p>"#;
    std::fs::write(dir.join("page.html"), html).unwrap();
    let doc = crate::io::open_path(&dir.join("page.html"));
    let _ = std::fs::remove_dir_all(&dir);
    let doc = doc.unwrap();
    assert_eq!(doc.media.len(), 1);
    let text = doc.plain_text(StoryRef::Body);
    assert!(!text.contains("Logo") && text.contains('T') && text.contains("Web"), "{text:?}");
}

/// A fresh, empty scratch folder for one test.
#[cfg(not(target_arch = "wasm32"))]
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wordcraft-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(not(target_arch = "wasm32"))]
fn tiny_png() -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_pictures_outside_the_folder_are_not_loaded() {
    // #98 review: `..` must not reach files outside the HTML file's folder.
    let root = scratch_dir("html-img-escape");
    std::fs::create_dir_all(root.join("site/img")).unwrap();
    std::fs::write(root.join("outside.png"), tiny_png()).unwrap();
    std::fs::write(root.join("site/inside.png"), tiny_png()).unwrap();
    let html = r#"<p><img src="../outside.png" alt="Up"></p><p><img src="img/../../outside.png" alt="Sneak"></p><p><img src="img/../inside.png" alt="In"></p>"#;
    std::fs::write(root.join("site/page.html"), html).unwrap();
    let doc = crate::io::open_path(&root.join("site/page.html"));
    let _ = std::fs::remove_dir_all(&root);
    let doc = doc.unwrap();
    assert_eq!(doc.media.len(), 1, "only the picture inside the folder loads");
    let text = doc.plain_text(StoryRef::Body);
    assert!(text.contains("Up") && text.contains("Sneak") && !text.contains("In"), "{text:?}");
}

#[cfg(unix)]
#[test]
fn html_pictures_through_a_symlink_out_of_the_folder_are_not_loaded() {
    // #98 review: a symlink inside the folder must not lead outside it.
    let root = scratch_dir("html-img-symlink");
    std::fs::create_dir_all(root.join("site")).unwrap();
    std::fs::write(root.join("outside.png"), tiny_png()).unwrap();
    std::os::unix::fs::symlink(root.join("outside.png"), root.join("site/link.png")).unwrap();
    std::fs::write(root.join("site/page.html"), r#"<p><img src="link.png" alt="Link"></p>"#).unwrap();
    let doc = crate::io::open_path(&root.join("site/page.html"));
    let _ = std::fs::remove_dir_all(&root);
    let doc = doc.unwrap();
    assert!(doc.media.is_empty());
    assert!(doc.plain_text(StoryRef::Body).contains("Link"));
}

#[cfg(unix)]
#[test]
fn html_picture_that_is_a_named_pipe_is_skipped_without_blocking() {
    // #98 review: opening a FIFO blocks until a writer appears; it must never be opened.
    let dir = scratch_dir("html-img-fifo");
    let fifo = dir.join("pipe.png");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("mkfifo unavailable; skipping");
        return;
    }
    std::fs::write(dir.join("page.html"), r#"<p><img src="pipe.png" alt="Pipe"></p>"#).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let page = dir.join("page.html");
    std::thread::spawn(move || {
        let _ = tx.send(crate::io::open_path(&page));
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    if got.is_err() {
        // Unblock the stuck reader so the thread ends, then fail.
        let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
    }
    let _ = std::fs::remove_dir_all(&dir);
    let doc = got.expect("opening the HTML file blocked on a named pipe").unwrap();
    assert!(doc.media.is_empty());
    assert!(doc.plain_text(StoryRef::Body).contains("Pipe"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_picture_size_limit_holds_whatever_length_the_file_reports() {
    use crate::io::LocalImages;
    let dir = scratch_dir("html-img-limit");
    std::fs::write(dir.join("ten.bin"), [7u8; 10]).unwrap();
    std::fs::write(dir.join("eleven.bin"), [7u8; 11]).unwrap();
    let images = LocalImages::with_limits(&dir, 10, 1000);
    let ten = images.load("ten.bin").map(|d| d.len());
    let eleven = images.load("eleven.bin");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(ten, Some(10));
    assert!(eleven.is_none());
    #[cfg(unix)]
    {
        // A device reports a length of 0 and never ends: it is not a regular file, and the read is
        // bounded anyway.
        let dev = LocalImages::with_limits(std::path::Path::new("/dev"), 1 << 20, 1 << 20);
        assert!(dev.load("zero").is_none());
        assert!(dev.load("null").is_none());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn html_pictures_share_one_read_and_a_per_document_budget() {
    // #98 review: a picture referenced many times is read once, and every reference counts
    // against the document's budget so a large picture can't be multiplied without bound.
    use crate::io::LocalImages;
    let dir = scratch_dir("html-img-budget");
    let png = tiny_png();
    let n = png.len() as u64;
    std::fs::write(dir.join("a.png"), &png).unwrap();
    let images = LocalImages::with_limits(&dir, 1 << 20, 3 * n);
    let first = images.load("a.png").unwrap();
    std::fs::write(dir.join("a.png"), b"changed on disk").unwrap();
    let second = images.load("./a.png").unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second), "the second reference reuses the first read");
    assert_eq!(*second, png);
    assert!(images.load("a.png").is_some());
    assert!(images.load("a.png").is_none(), "budget spent");
    // Files over the per-picture limit still cost what was read.
    std::fs::write(dir.join("big.bin"), vec![1u8; 100]).unwrap();
    std::fs::write(dir.join("small.bin"), [1u8; 5]).unwrap();
    let images = LocalImages::with_limits(&dir, 10, 25);
    assert!(images.load("big.bin").is_none());
    assert!(images.load("big.bin").is_none());
    let small = images.load("small.bin");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(small.is_none(), "two oversized reads used up the budget");
}
