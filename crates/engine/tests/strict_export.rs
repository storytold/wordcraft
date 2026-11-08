//! Strict export (#6005): `strict: true` on the PDF and PNG exports refuses an output that would
//! silently lack something, before any byte is written; `file.strictReport` says what.
use serde_json::{Value, json};
use wordcraft_doc::Document;
use wordcraft_engine::{CmdError, Session};
use wordcraft_fonts::FontDb;

/// A code point in unassigned plane 4 that no face of the host covers (as the layout tests find it).
fn uncovered() -> char {
    let db = FontDb::global();
    let primary = wordcraft_fonts::word::resolve(&Document::default().settings.minor_font, false, false).face;
    (0x40000u32..0x4FFF0)
        .filter_map(char::from_u32)
        .find(|c| !primary.covers(*c) && db.fallback_styled(*c, primary.id(), false, false).is_none() && db.fallback_for(*c, primary.id()).is_none())
        .expect("the host has faces covering all of plane 4")
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("wc-strict-export-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn refused(r: Result<Value, CmdError>) -> String {
    match r {
        Err(CmdError::Failed(m)) => {
            assert!(m.starts_with("strict: "), "{m}");
            m
        }
        other => panic!("expected a strict refusal, got {other:?}"),
    }
}

#[test]
fn strict_refuses_an_uncovered_character_and_writes_nothing() {
    let c = uncovered();
    let d = dir("refuse");
    let mut s = Session::new(Document::from_text(&format!("a{c}b")));
    let pdf = d.join("out.pdf");
    let png = d.join("out.png");
    let msg = refused(s.run("file.exportPdf", &json!({"path": pdf, "strict": true})));
    assert!(msg.contains(&format!("U+{:04X} (×1)", c as u32)), "{msg}");
    refused(s.run("file.save", &json!({"path": pdf, "strict": true})));
    refused(s.run("file.saveAs", &json!({"path": pdf, "strict": true})));
    refused(s.run("file.exportPng", &json!({"path": png, "strict": true})));
    refused(s.run("file.save", &json!({"path": png, "strict": true})));
    assert!(!pdf.exists() && !png.exists());
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_refusal_leaves_an_existing_output_untouched() {
    let c = uncovered();
    let d = dir("keep");
    let pdf = d.join("out.pdf");
    std::fs::write(&pdf, b"previous").unwrap();
    let mut s = Session::new(Document::from_text(&format!("a{c}")));
    refused(s.run("file.exportPdf", &json!({"path": pdf, "strict": true})));
    assert_eq!(std::fs::read(&pdf).unwrap(), b"previous");
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_refused_save_leaves_the_stamps_path_and_dirty_flag_alone() {
    let c = uncovered();
    let d = dir("stamp");
    let mut s = Session::new(Document::from_text(&format!("a{c}")));
    s.dirty = true;
    let before =
        (s.doc.core.modified.clone(), s.doc.core.created.clone(), s.doc.core.revision, s.doc.core.last_modified_by.clone(), s.doc.settings.zoom);
    refused(s.run("file.save", &json!({"path": d.join("out.pdf"), "strict": true})));
    let after =
        (s.doc.core.modified.clone(), s.doc.core.created.clone(), s.doc.core.revision, s.doc.core.last_modified_by.clone(), s.doc.settings.zoom);
    assert_eq!(before, after);
    assert!(s.dirty);
    assert_eq!(s.path, None);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn strict_passes_and_writes_when_nothing_is_missing() {
    let d = dir("pass");
    let mut s = Session::new(Document::from_text("Hello, world."));
    for (cmd, file, magic) in [
        ("file.exportPdf", "a.pdf", &b"%PDF-"[..]),
        ("file.save", "b.pdf", &b"%PDF-"[..]),
        ("file.exportPng", "a.png", &b"\x89PNG"[..]),
        ("file.save", "b.png", &b"\x89PNG"[..]),
    ] {
        let path = d.join(file);
        s.run(cmd, &json!({"path": path, "strict": true})).unwrap();
        assert!(std::fs::read(&path).unwrap().starts_with(magic), "{file}");
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn strict_off_writes_the_same_bytes_as_before_even_with_a_missing_character() {
    let c = uncovered();
    let d = dir("same");
    let mut s = Session::new(Document::from_text(&format!("Hello {c}")));
    let mut bytes = Vec::new();
    for (i, params) in [json!({}), json!({"strict": false})].into_iter().enumerate() {
        let mut p = params;
        p["path"] = json!(d.join(format!("{i}.pdf")));
        s.run("file.exportPdf", &p).unwrap();
        bytes.push(std::fs::read(d.join(format!("{i}.pdf"))).unwrap());
    }
    assert!(!bytes[0].is_empty());
    // The PDF carries a creation time to the second: compare past it.
    assert_eq!(bytes[0].len(), bytes[1].len());
    let png: Vec<Vec<u8>> = [json!({}), json!({"strict": false})]
        .into_iter()
        .enumerate()
        .map(|(i, mut p)| {
            p["path"] = json!(d.join(format!("{i}.png")));
            s.run("file.exportPng", &p).unwrap();
            std::fs::read(d.join(format!("{i}.png"))).unwrap()
        })
        .collect();
    assert_eq!(png[0], png[1]);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn strict_passes_a_document_the_same_pdf_as_without_it() {
    let d = dir("pdfsame");
    let mut s = Session::new(Document::from_text("Plain text"));
    s.run("file.exportPdf", &json!({"path": d.join("a.pdf")})).unwrap();
    s.run("file.exportPdf", &json!({"path": d.join("b.pdf"), "strict": true})).unwrap();
    let (a, b) = (std::fs::read(d.join("a.pdf")).unwrap(), std::fs::read(d.join("b.pdf")).unwrap());
    assert_eq!(a.len(), b.len());
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn strict_is_for_pdf_and_png_outputs_only() {
    let d = dir("other");
    let mut s = Session::new(Document::from_text("Hello"));
    for file in ["a.docx", "a.txt", "a.odt", "a.html"] {
        let path = d.join(file);
        let r = s.run("file.save", &json!({"path": path, "strict": true}));
        assert!(matches!(r, Err(CmdError::Params(_))), "{file}: {r:?}");
        assert!(!path.exists(), "{file}");
    }
    let r = s.run("file.exportPng", &json!({"path": d.join("a.jpg"), "strict": true}));
    assert!(matches!(r, Err(CmdError::Params(_))), "{r:?}");
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn strict_report_has_the_schema_1_shape() {
    let c = uncovered();
    let mut s = Session::new(Document::from_text(&format!("a{c}b{c}")));
    let r = s.run("file.strictReport", &json!({})).unwrap();
    assert_eq!(
        r,
        json!({
            "schema": 1,
            "strict": "failed",
            "missing_glyphs": [{"cp": format!("U+{:04X}", c as u32), "count": 2}],
            "missing_pictures": [],
            "truncated": false,
        })
    );
    let png = s.run("file.strictReport", &json!({"page": 1})).unwrap();
    assert_eq!(png, r);
    assert!(matches!(s.run("file.strictReport", &json!({"page": 99})), Err(CmdError::Params(_))));

    let mut ok = Session::new(Document::from_text("fine"));
    let r = ok.run("file.strictReport", &json!({})).unwrap();
    assert_eq!(r, json!({"schema": 1, "strict": "passed", "missing_glyphs": [], "missing_pictures": [], "truncated": false}));
}

#[test]
fn strict_report_changes_nothing() {
    let c = uncovered();
    let mut s = Session::new(Document::from_text(&format!("a{c}")));
    let before = serde_json::to_string(&s.doc).unwrap();
    s.run("file.strictReport", &json!({})).unwrap();
    assert_eq!(serde_json::to_string(&s.doc).unwrap(), before);
    assert!(!s.dirty);
}
