use serde_json::json;
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Block, Document, StoryRef};

use super::*;

/// Each body paragraph's text with its fields written in place as `{instr}`.
fn marked_text(s: &Session) -> Vec<String> {
    s.doc
        .para_paths(StoryRef::Body)
        .iter()
        .filter_map(|path| s.doc.para(StoryRef::Body, path))
        .map(|p| {
            let mut objects = p.objects.iter();
            p.text
                .chars()
                .map(|c| match (c, c == OBJ) {
                    (_, true) => match objects.next() {
                        Some(InlineObject::Field { instr, .. }) => format!("{{{instr}}}"),
                        _ => "{?}".into(),
                    },
                    (c, false) => c.to_string(),
                })
                .collect()
        })
        .collect()
}

/// A session on `text`, one paragraph per line.
fn session(text: &str) -> Session {
    Session::new(Document::from_text(text))
}

/// A concordance document: `rows` (lines of tab-separated cells) turned into a table.
fn concordance_doc(rows: &str) -> Session {
    let mut c = session(rows);
    c.run("select.all", &json!({})).unwrap();
    c.run("table.fromText", &json!({"separator": "tab"})).unwrap();
    c
}

/// A fresh folder for files a test writes.
#[cfg(not(target_arch = "wasm32"))]
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wc-automark-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const POTTERY: &str =
    "Kilns are hot. Kiln walls hold heat, the kiln cools.\nGlaze the pots with glaze, then glaze again.\nThe Press, the press.\nKilnsmith.";

/// AutoMark reads the concordance file's table and marks, in every paragraph, the first exact
/// (same case, whole word) occurrence of each row's text with that row's entry (`Main:Sub` as a
/// subentry, the text itself when the entry is empty). Running it again marks nothing more, one
/// undo takes it all back, and Insert Index lists the entries.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn automark_marks_the_first_exact_match_in_each_paragraph() {
    let dir = scratch("file");
    let path = dir.join("Concordance.docx");
    let mut c = concordance_doc("Kiln\tKilns\nglaze\tGlazes:clear\nPress\t");
    c.run("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    let path = path.to_string_lossy().to_string();

    let mut s = session(POTTERY);
    let before = s.doc.clone();
    let r = s.run("references.autoMark", &json!({"path": path})).unwrap();
    assert_eq!(r, json!({"marked": 3, "terms": 3}));
    assert_eq!(
        marked_text(&s),
        vec![
            r#"Kilns are hot. Kiln{XE "Kilns"} walls hold heat, the kiln cools."#,
            r#"Glaze the pots with glaze{XE "Glazes:clear"}, then glaze again."#,
            r#"The Press{XE "Press"}, the press."#,
            "Kilnsmith.",
        ]
    );
    let hidden = s.doc.body[0].as_para().unwrap().runs.iter().any(|r| r.props.hidden == Some(true));
    assert!(hidden, "the XE field is hidden text, as Mark Entry makes it");

    let again = s.run("references.autoMark", &json!({"path": path})).unwrap();
    assert_eq!(again["marked"], 0, "each paragraph already has its entries");

    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.doc.same_content(&before), "one undo step took the marks back");
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(marked_text(&s)[2], r#"The Press{XE "Press"}, the press."#);

    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.index", &json!({"columns": 1, "headings": false})).unwrap();
    let lines: Vec<String> =
        s.doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some(ENTRY_STYLE)).map(|p| p.plain_text()).collect();
    assert_eq!(lines, vec!["Glazes", "clear, 1", "Kilns, 1", "Press, 1"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A document without a table is read a line at a time, the text and the entry separated by a
/// tab (a line without one marks its own text), whether it is plain text or a document.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn automark_reads_tab_separated_lines_without_a_table() {
    let dir = scratch("lines");
    let txt = dir.join("concordance.txt");
    std::fs::write(&txt, "Kiln\tKilns\r\nglaze\tGlazes:clear\r\nPress\r\n").unwrap();
    let docx = dir.join("concordance.docx");
    session("Kiln\tKilns\nglaze\tGlazes:clear\nPress").run("file.saveAs", &json!({"path": docx.to_string_lossy()})).unwrap();
    for path in [&txt, &docx] {
        let mut s = session(POTTERY);
        let r = s.run("references.autoMark", &json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(r, json!({"marked": 3, "terms": 3}), "{}", path.display());
        assert_eq!(marked_text(&s)[2], r#"The Press{XE "Press"}, the press."#);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A web page works as a concordance file, and only the file itself is read: the picture it
/// names stays where it is, though opening the page to edit it brings the picture in.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_web_page_concordance_is_read_without_its_pictures() {
    let dir = scratch("html");
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 30, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(dir.join("logo.png"), &png).unwrap();
    let page = dir.join("concordance.html");
    let html = r#"<p><img src="logo.png" alt="Logo"></p><table><tr><td>Kiln</td><td>Kilns</td></tr><tr><td>Press</td><td></td></tr></table>"#;
    std::fs::write(&page, html).unwrap();
    assert_eq!(crate::io::open_path(&page).unwrap().media.len(), 1, "opened to edit, the page brings its picture");
    assert!(crate::io::open_path_up_to(&page, MAX_FILE).unwrap().media.is_empty(), "read for its rows, it doesn't");
    assert_eq!(read_concordance(&page).unwrap(), vec![("Kiln".to_string(), "Kilns".to_string()), ("Press".into(), String::new())]);

    let mut s = session(POTTERY);
    let r = s.run("references.autoMark", &json!({"path": page.to_string_lossy()})).unwrap();
    assert_eq!(r, json!({"marked": 2, "terms": 2}));
    assert_eq!(marked_text(&s)[2], r#"The Press{XE "Press"}, the press."#);
    assert!(s.doc.media.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Files AutoMark can't use are errors that change nothing and leave no undo step: a missing
/// file, a folder, noise in a document's clothing, a file over 32 MB (never read), no file at
/// all, and a network path (refused before anything looks at it). Files it can read but that
/// hold nothing usable (empty, binary bytes read as text, text in a broken encoding) mark nothing
/// they don't contain and never panic.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn automark_survives_hostile_concordance_files() {
    let dir = scratch("hostile");
    let noise: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    std::fs::write(dir.join("noise.docx"), &noise).unwrap();
    std::fs::write(dir.join("noise.txt"), &noise).unwrap();
    std::fs::write(dir.join("empty.txt"), b"").unwrap();
    std::fs::write(dir.join("empty.docx"), b"").unwrap();
    // Not UTF-8: a lone continuation byte and a truncated sequence around a tab.
    std::fs::write(dir.join("latin.txt"), b"Kiln\t\x80Kilns\xE2\x82\nglaze\xFF").unwrap();
    // UTF-16 with its byte-order mark.
    let utf16: Vec<u8> = [0xFEFFu16].into_iter().chain("Kiln\tOvens".encode_utf16()).flat_map(u16::to_le_bytes).collect();
    std::fs::write(dir.join("utf16.txt"), utf16).unwrap();
    // Over the limit, as a sparse file: its size is all that is looked at.
    let huge = std::fs::File::create(dir.join("huge.txt")).unwrap();
    huge.set_len(MAX_FILE + 1).unwrap();
    std::fs::create_dir_all(dir.join("folder.docx")).unwrap();
    let at = |name: &str| dir.join(name).to_string_lossy().into_owned();

    let mut s = session(POTTERY);
    let before = s.doc.clone();
    for name in ["missing.docx", "folder.docx", "noise.docx", "empty.docx", "huge.txt"] {
        let e = s.run("references.autoMark", &json!({"path": at(name)})).unwrap_err();
        assert!(matches!(e, CmdError::Failed(_)), "{name}: {e}");
    }
    let e = s.run("references.autoMark", &json!({"path": at("huge.txt")})).unwrap_err();
    assert!(e.to_string().ends_with("file is larger than 32 MB"), "{e}");
    for network in [r"\\evil.example\share\Concordance.docx", "//evil.example/share/Concordance.docx", r"\\?\UNC\evil.example\share\c.docx"] {
        let e = s.run("references.autoMark", &json!({"path": network})).unwrap_err();
        assert_eq!(e.to_string(), CmdError::Params("the concordance file can't be on another computer".into()).to_string(), "{network}");
    }
    for bad in [json!({}), json!({"path": 5}), json!({"path": [], "entries": ["Kiln"]})] {
        assert!(matches!(s.run("references.autoMark", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    assert!(s.doc.same_content(&before));
    assert!(!s.can_undo(), "no undo step");

    for name in ["empty.txt", "noise.txt"] {
        let r = s.run("references.autoMark", &json!({"path": at(name)})).unwrap();
        assert_eq!(r["marked"], 0, "{name}: {r}");
    }
    assert!(s.doc.same_content(&before), "nothing in those files is in the document");
    // The broken bytes are read as text: the row's text still matches, its entry is what was read.
    s.run("references.autoMark", &json!({"path": at("latin.txt")})).unwrap();
    assert!(marked_text(&s)[0].starts_with(r#"Kilns are hot. Kiln{XE ""#), "{:?}", marked_text(&s));
    assert!(marked_text(&s)[1].starts_with("Glaze the pots with glaze,"), "glaze\\xFF is not glaze");
    let mut s = session(POTTERY);
    s.run("references.autoMark", &json!({"path": at("utf16.txt")})).unwrap();
    assert_eq!(marked_text(&s)[0], r#"Kilns are hot. Kiln{XE "Ovens"} walls hold heat, the kiln cools."#);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The marks are ordinary XE fields: saved to .docx and opened again, they are still there,
/// hidden, and Insert Index lists them.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn automarked_entries_survive_save_and_reopen() {
    let dir = scratch("reopen");
    let mut s = session(POTTERY);
    s.run("references.autoMark", &json!({"entries": [["Kiln", "Kilns"], ["glaze", "Glazes:clear"], "Press"]})).unwrap();
    let marked = marked_text(&s);
    let path = dir.join("Pottery.docx").to_string_lossy().into_owned();
    s.run("file.saveAs", &json!({"path": path})).unwrap();
    let mut s = session("");
    s.run("file.open", &json!({"path": path})).unwrap();
    assert_eq!(marked_text(&s), marked);
    let para = s.doc.body[0].as_para().unwrap();
    assert!(para.runs.iter().any(|r| r.props.hidden == Some(true)), "still hidden");
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("references.index", &json!({"columns": 1, "headings": false})).unwrap();
    let lines: Vec<String> =
        s.doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some(ENTRY_STYLE)).map(|p| p.plain_text()).collect();
    assert_eq!(lines, vec!["Glazes", "clear, 1", "Kilns, 1", "Press, 1"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Rows read as Word reads them: a cell's paragraphs joined, a row of one cell marking its own
/// text, quotes left out of the entry (they would end the XE field's text), the tables after the
/// first read too, rows with nothing to find (or more than Word's Find takes) skipped, and the
/// paragraphs around the tables left out.
#[test]
fn concordance_rows_come_from_every_table() {
    let first = concordance_doc("Kiln\tKiln \"firing\"\nPress\tPresses");
    let second = concordance_doc("\tEmpty\nWheel\tWheels");
    let mut doc = Document::from_text("Concordance");
    doc.body.extend(first.doc.body.iter().filter(|b| b.as_table().is_some()).cloned());
    doc.body.push(std::sync::Arc::new(Block::Para(wordcraft_doc::Paragraph::new())));
    doc.body.extend(second.doc.body.iter().filter(|b| b.as_table().is_some()).cloned());
    let Some(Block::Table(t)) = doc.body.get_mut(1).map(std::sync::Arc::make_mut) else { panic!("the first table") };
    // The first cell holds two paragraphs; the second row keeps one cell only.
    t.rows[0].cells[0].blocks.push(std::sync::Arc::new(Block::Para(wordcraft_doc::Paragraph::with_text("walls", CharProps::default()))));
    t.rows[1].cells.truncate(1);
    let rows = concordance(&doc);
    assert_eq!(
        rows,
        vec![
            ("Kiln walls".to_string(), "Kiln \"firing\"".to_string()),
            ("Press".into(), String::new()),
            (String::new(), "Empty".into()),
            ("Wheel".into(), "Wheels".into()),
        ]
    );
    let terms: Vec<(String, String)> = rows.iter().filter_map(|(t, e)| term(t, e)).map(|t| (t.text, t.instr)).collect();
    assert_eq!(
        terms,
        vec![
            ("Kiln walls".to_string(), r#"XE "Kiln firing""#.to_string()),
            ("Press".into(), r#"XE "Press""#.into()),
            ("Wheel".into(), r#"XE "Wheels""#.into()),
        ]
    );
    let long = "x".repeat(MAX_TEXT + 1);
    assert!(term(&long, "Long").is_none(), "longer than Find takes");
    assert!(term(&long[1..], "Long").is_some());
}

/// Rows marking the same entry act one after another, as Word runs them: the first row with a
/// match in a paragraph marks it (at its own first match, even with another row's match
/// earlier), and the later row finds the entry already there. Different entries matching at one
/// place keep the rows' order, and a paragraph that held the entry before is left alone.
#[test]
fn rows_for_one_entry_mark_a_paragraph_once() {
    let mut s = session("Kiln, then kiln.\nKiln firing starts.\nOnly kiln{XE} here.");
    // A field already in the third paragraph, as an earlier Mark Entry leaves it.
    let path = s.doc.para_paths(StoryRef::Body)[2].clone();
    let off = s.doc.para(StoryRef::Body, &path).unwrap().text.find('{').unwrap();
    let p = s.doc.para_mut(StoryRef::Body, &path).unwrap();
    p.delete(off, off + "{XE}".len()).unwrap();
    let field =
        InlineObject::Field { instr: r#"XE "Kilns""#.into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() };
    p.insert_object(off, field, &CharProps::default()).unwrap();
    let r = s
        .run("references.autoMark", &json!({"entries": [["kiln", "Kilns"], ["Kiln", "Kilns"], ["Kiln firing", "Firing"], ["Kiln", "Ovens"]]}))
        .unwrap();
    assert_eq!(r, json!({"marked": 5, "terms": 4}));
    assert_eq!(
        marked_text(&s),
        vec![
            r#"Kiln{XE "Ovens"}, then kiln{XE "Kilns"}."#,
            r#"Kiln{XE "Kilns"}{XE "Ovens"} firing{XE "Firing"} starts."#,
            r#"Only kiln{XE "Kilns"} here."#,
        ]
    );
}

/// A row of several words matches only where its last word ends too: "Kiln firing" is not in
/// "Kiln firings", nor "Kiln" in "Kilns".
#[test]
fn a_phrase_matches_whole_words_only() {
    let mut s = session("Kiln firings start, then Kiln firing ends.\nKilns, then Kiln.");
    let r = s.run("references.autoMark", &json!({"entries": [["Kiln firing", "Firing"], ["Kiln", "Kilns"]]})).unwrap();
    assert_eq!(r, json!({"marked": 3, "terms": 2}));
    assert_eq!(marked_text(&s), vec![r#"Kiln{XE "Kilns"} firings start, then Kiln firing{XE "Firing"} ends."#, r#"Kilns, then Kiln{XE "Kilns"}."#]);
}

/// `entries` takes texts and [text, entry] pairs; anything else is refused before any change.
#[test]
fn entries_are_texts_or_pairs() {
    let mut s = session("Clay and glaze.");
    let r = s.run("references.autoMark", &json!({"entries": ["Clay", ["glaze"], ["glaze", ""], "  ", ["", "Nothing"]]})).unwrap();
    assert_eq!(r, json!({"marked": 2, "terms": 3}));
    assert_eq!(marked_text(&s), vec![r#"Clay{XE "Clay"} and glaze{XE "glaze"}."#]);
    let before = s.doc.clone();
    for bad in [json!({"entries": [5]}), json!({"entries": [["a", "b", "c"]]}), json!({"entries": [["a", 5]]}), json!({"entries": "Clay"})] {
        assert!(matches!(s.run("references.autoMark", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    assert!(s.doc.same_content(&before));
}

/// Each paragraph is read once: a concordance eight times longer, of words the document doesn't
/// hold, costs no more steps than the short one, so a long concordance on a long document stays
/// fast. (Searching the paragraph once per row would cost eight times as much.)
#[test]
fn a_longer_concordance_costs_no_more_reading() {
    let text = vec!["Kilns hold heat while the glaze sets on every pot in the room."; 200].join("\n");
    let steps = |rows: usize| {
        let mut s = session(&text);
        let entries: Vec<Value> = (0..rows).map(|i| json!([format!("Absent{i}"), "Nowhere"])).chain([json!(["glaze", "Glaze"])]).collect();
        let mut marked = Value::Null;
        let n = wordcraft_doc::work::count(|| marked = s.run("references.autoMark", &json!({"entries": entries})).unwrap());
        assert_eq!(marked["marked"], 200);
        n
    };
    let (short, long) = (steps(100), steps(800));
    assert!(long <= short + short / 10, "800 rows took {long} steps, 100 rows {short}");
}
