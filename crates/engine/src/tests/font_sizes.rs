//! Relative font sizes keep each selected run's size (#4903).

use serde_json::json;
use wordcraft_doc::{Document, Pos};

use crate::Session;

fn mixed() -> Session {
    let mut doc = Document::from_text("Large heading\nSmall detail");
    for (block, size) in [(0, 20.0), (1, 10.0)] {
        let end = doc.para_at(&Pos::body(block, 0)).unwrap().len();
        doc.format_range(&Pos::body(block, 0), &Pos::body(block, end), &|c| {
            c.size = Some(size);
            c.size_cs = Some(size);
        })
        .unwrap();
    }
    let mut s = Session::new(doc);
    s.run("select.all", &json!({})).unwrap();
    s
}

fn sizes(s: &Session) -> Vec<f32> {
    (0..2)
        .map(|block| {
            let p = s.doc.para_at(&Pos::body(block, 0)).unwrap();
            s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(0)).size
        })
        .collect()
}

#[test]
fn relative_font_sizes_preserve_mixed_runs_and_one_undo() {
    for (command, expected) in [
        ("format.growFont", [22.0, 10.5]),
        ("format.shrinkFont", [18.0, 9.0]),
        ("format.growFont1", [21.0, 11.0]),
        ("format.shrinkFont1", [19.0, 9.0]),
    ] {
        let mut s = mixed();
        let before = s.doc.clone();
        let selection = s.sel.clone();
        let depth = s.undo_depth();
        s.run(command, &json!({})).unwrap();
        assert_eq!(sizes(&s), expected, "{command}");
        assert_eq!(s.sel, selection);
        assert_eq!(s.undo_depth(), depth + 1);
        let after = s.doc.clone();
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc, before);
        s.run("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc, after);
    }
}

#[test]
fn relative_font_sizes_resolve_styles_and_preserve_other_properties() {
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::styles::{Style, StyleKind};

    let mut s = mixed();
    s.doc.styles.default_chr.size_cs = Some(30.0);
    s.doc.styles.upsert(Style {
        id: "Large".into(),
        name: "Large".into(),
        based_on: Some("Normal".into()),
        chr: CharProps { size: Some(20.0), bold: Some(true), ..Default::default() },
        ..Default::default()
    });
    s.doc.styles.upsert(Style {
        id: "Small".into(),
        name: "Small".into(),
        kind: StyleKind::Character,
        chr: CharProps { size: Some(10.0), italic: Some(true), ..Default::default() },
        ..Default::default()
    });
    for block in 0..2 {
        let p = s.doc.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(block)).unwrap();
        p.props.style = Some(if block == 0 { "Large" } else { "Normal" }.into());
        for c in std::iter::once(&mut p.mark).chain(p.runs.iter_mut().map(|r| &mut r.props)) {
            c.size = None;
            c.size_cs = None;
            if block == 1 {
                c.style = Some("Small".into());
            }
        }
    }
    s.run("format.growFont", &json!({})).unwrap();
    assert_eq!(sizes(&s), [22.0, 10.5]);
    for (block, size) in [(0, 22.0), (1, 10.5)] {
        let p = s.doc.para_at(&Pos::body(block, 0)).unwrap();
        assert_eq!(p.mark.size, Some(size));
        for (_, c) in p.run_ranges() {
            assert_eq!((c.size, c.size_cs), (Some(size), Some(size)), "both scripts still get the explicit override");
            let resolved = s.doc.styles.resolve_char(p.props.style.as_deref(), c);
            assert!(if block == 0 { resolved.bold } else { resolved.italic });
        }
    }
}

#[test]
fn relative_font_sizes_change_only_the_selected_unicode_pieces() {
    let mut s = Session::new(Document::from_text("éA中Bz"));
    s.doc.format_range(&Pos::body(0, 2), &Pos::body(0, 3), &|c| c.size = Some(20.0)).unwrap();
    s.doc.format_range(&Pos::body(0, 3), &Pos::body(0, 6), &|c| c.size = Some(10.0)).unwrap();
    let before = s.doc.clone();
    s.run("select.range", &json!({"anchor": Pos::body(0, 2), "focus": Pos::body(0, 6)})).unwrap();
    s.run("format.growFont1", &json!({})).unwrap();
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    let original = before.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.text, "éA中Bz");
    assert_eq!(p.props_of_char(2).size, Some(21.0));
    assert_eq!(p.props_of_char(3).size, Some(11.0));
    assert_eq!(p.props_of_char(0), original.props_of_char(0));
    assert_eq!(p.props_of_char(6), original.props_of_char(6));
    assert_eq!(p.mark, original.mark, "a partial selection leaves the paragraph mark alone");
}

#[test]
fn relative_font_sizes_keep_the_caret_and_word_targets() {
    let mut s = Session::new(Document::new());
    s.run("format.size", &json!({"size": 10})).unwrap();
    for size in [11.0, 12.0] {
        s.run("format.growFont1", &json!({})).unwrap();
        assert_eq!(s.pending.as_ref().unwrap().size, Some(size));
        assert_eq!(s.doc.para_at(&Pos::body(0, 0)).unwrap().mark.size, Some(size));
    }
    s.run("text.insert", &json!({"text": "abcd ef"})).unwrap();
    s.doc.format_range(&Pos::body(0, 0), &Pos::body(0, 2), &|c| c.size = Some(20.0)).unwrap();
    s.doc.format_range(&Pos::body(0, 2), &Pos::body(0, 4), &|c| c.size = Some(10.0)).unwrap();
    s.run("caret.set", &json!({"pos": Pos::body(0, 2)})).unwrap();
    let selection = s.sel.clone();
    s.run("format.growFont1", &json!({})).unwrap();
    assert_eq!(s.sel, selection);
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!((p.props_of_char(0).size, p.props_of_char(2).size, p.props_of_char(5).size), (Some(21.0), Some(11.0), Some(12.0)));
}

#[test]
fn relative_font_sizes_keep_column_cell_and_text_box_targets() {
    use wordcraft_doc::{Path, StoryRef};

    let mut s = super::column_doc();
    super::block(&mut s);
    for (block, size) in [(0, 20.0), (1, 10.0), (2, 14.0)] {
        s.doc.format_range(&Pos::body(block, 0), &Pos::body(block, 6), &|c| c.size = Some(size)).unwrap();
    }
    s.run("format.growFont1", &json!({})).unwrap();
    assert!(s.column_segments().is_some());
    for (block, size) in [(0, 20.0), (1, 10.0), (2, 14.0)] {
        let p = s.doc.para_at(&Pos::body(block, 0)).unwrap();
        assert_eq!((p.props_of_char(1).size, p.props_of_char(2).size, p.props_of_char(4).size), (Some(size), Some(size + 1.0), Some(size)));
        assert_eq!(p.mark.size, Some(size));
    }

    let mut s = Session::new(Document::new());
    s.run("insert.table", &json!({"rows": 1, "cols": 2})).unwrap();
    let paths = [Path(vec![0, 0, 0, 0]), Path(vec![0, 0, 1, 0])];
    for (path, size) in paths.iter().zip([20.0, 10.0]) {
        s.doc.para_mut(StoryRef::Body, path).unwrap().mark.size = Some(size);
    }
    s.run("table.selectTable", &json!({})).unwrap();
    let _ = s.layout();
    s.run("format.growFont1", &json!({})).unwrap();
    for (path, size) in paths.iter().zip([21.0, 11.0]) {
        assert_eq!(s.doc.para(StoryRef::Body, path).unwrap().mark.size, Some(size), "empty cells retain distinct sizes");
    }

    let mut s = Session::new(Document::from_text("Body"));
    let id = s.run("insert.textBox", &json!({"text": "AB"})).unwrap()["story"].as_u64().unwrap() as u32;
    let story = StoryRef::Part(id);
    let at = |off| Pos { story, path: Path::top(0), off };
    for (a, b, size) in [(0, 1, 20.0), (1, 2, 10.0)] {
        s.doc.format_range(&at(a), &at(b), &|c| c.size = Some(size)).unwrap();
    }
    s.run("select.range", &json!({"anchor": Pos::body(0, 0), "focus": Pos::body(0, "\u{fffc}".len())})).unwrap();
    let before = s.doc.para_at(&Pos::body(0, 0)).unwrap().clone();
    s.run("format.growFont1", &json!({})).unwrap();
    assert_eq!(*s.doc.para_at(&Pos::body(0, 0)).unwrap(), before);
    let p = s.doc.para(story, &Path::top(0)).unwrap();
    assert_eq!((p.props_of_char(0).size, p.props_of_char(1).size), (Some(21.0), Some(11.0)));
}

#[test]
fn relative_font_sizes_keep_bounds_protection_and_saved_sizes() {
    for (command, values, expected) in [
        ("format.growFont", [72.0, 1638.0], [80.0, 1638.0]),
        ("format.shrinkFont", [1.0, 8.0], [1.0, 1.0]),
        ("format.growFont1", [1.0, 1638.0], [2.0, 1638.0]),
        ("format.shrinkFont1", [1.0, 1638.0], [1.0, 1637.0]),
    ] {
        let mut s = mixed();
        for (block, size) in values.into_iter().enumerate() {
            let end = s.doc.para_at(&Pos::body(block, 0)).unwrap().len();
            s.doc.format_range(&Pos::body(block, 0), &Pos::body(block, end), &|c| c.size = Some(size)).unwrap();
        }
        s.run(command, &json!({})).unwrap();
        assert_eq!(sizes(&s), expected, "{command}");
        let bytes = crate::io::save_bytes("sizes.docx", &s.doc).unwrap();
        let reopened = Session::new(crate::io::open_bytes("sizes.docx", &bytes).unwrap());
        assert_eq!(sizes(&reopened), expected, "saved {command}");
        s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
        let before = s.doc.clone();
        let depth = s.undo_depth();
        assert!(s.run(command, &json!({})).is_err());
        assert_eq!(s.doc, before);
        assert_eq!(s.undo_depth(), depth);
    }
}

#[test]
fn relative_font_sizes_keep_paragraph_mark_link_filtering() {
    let mut s = mixed();
    for (block, style) in [(0, "Hyperlink"), (1, "CustomLink")] {
        let p = s.doc.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(block)).unwrap();
        p.mark.link = Some("https://example.test/mark".into());
        p.mark.style = Some(style.into());
        p.runs[0].props.link = Some("https://example.test/text".into());
        p.runs[0].props.style = Some("Hyperlink".into());
    }
    s.run("format.growFont", &json!({})).unwrap();
    assert_eq!(sizes(&s), [22.0, 10.5]);
    for block in 0..2 {
        let p = s.doc.para_at(&Pos::body(block, 0)).unwrap();
        assert_eq!(p.mark.link, None);
        assert_eq!(p.mark.style.as_deref(), if block == 0 { None } else { Some("CustomLink") });
        assert!(p.run_ranges().all(|(_, c)| c.link.as_deref() == Some("https://example.test/text") && c.style.as_deref() == Some("Hyperlink")));
    }
}
