//! Strict export (#6005): `strict: true` on the PDF and PNG exports refuses an output that would
//! silently lack something, before any byte is written; `file.strictReport` says what.
use serde_json::{Value, json};
use wordcraft_doc::Document;
use wordcraft_engine::{CmdError, Session};
use wordcraft_fonts::FontDb;

/// Code points in unassigned plane 4 that no face of the host covers (as the layout tests find them).
fn uncovered_many(n: usize) -> Vec<char> {
    let db = FontDb::global();
    let primary = wordcraft_fonts::word::resolve(&Document::default().settings.minor_font, false, false).face;
    let out: Vec<char> = (0x40000u32..0x4FFF0)
        .filter_map(char::from_u32)
        .filter(|c| {
            !primary.covers(*c) && db.fallback_styled(*c, primary.id(), false, false).is_none() && db.fallback_for(*c, primary.id()).is_none()
        })
        .take(n)
        .collect();
    assert_eq!(out.len(), n, "the host has faces covering all of plane 4");
    out
}

fn uncovered() -> char {
    uncovered_many(1)[0]
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
            "total_missing_glyphs": 1,
            "total_missing_pictures": 0,
            "glyphs_truncated": false,
            "pictures_truncated": false,
        })
    );
    let png = s.run("file.strictReport", &json!({"page": 1})).unwrap();
    assert_eq!(png, r);
    assert!(matches!(s.run("file.strictReport", &json!({"page": 99})), Err(CmdError::Params(_))));

    let mut ok = Session::new(Document::from_text("fine"));
    let r = ok.run("file.strictReport", &json!({})).unwrap();
    assert_eq!(
        r,
        json!({"schema": 1, "strict": "passed", "missing_glyphs": [], "missing_pictures": [], "truncated": false, "total_missing_glyphs": 0, "total_missing_pictures": 0, "glyphs_truncated": false, "pictures_truncated": false})
    );
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

fn garbage_background(d: &mut Document) {
    d.media.insert("bg.bin".into(), std::sync::Arc::new(b"this is not a picture".to_vec()));
    d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: "bg.bin".into(), fill_type: "stretch".into(), title: String::new() });
}

#[test]
fn an_undecodable_page_background_picture_is_refused_for_pdf_and_png() {
    let d = dir("background");
    let mut doc = Document::from_text("Hello");
    garbage_background(&mut doc);
    let mut s = Session::new(doc);
    let r = s.run("file.strictReport", &json!({})).unwrap();
    assert_eq!(r["strict"], "failed");
    assert_eq!(r["missing_pictures"][0], json!({"part": "bg.bin", "source_part": "background", "reason": "unsupported_format"}));
    let msg = refused(s.run("file.exportPdf", &json!({"path": d.join("a.pdf"), "strict": true})));
    assert!(msg.contains("bg.bin (background)"), "{msg}");
    refused(s.run("file.exportPng", &json!({"path": d.join("a.png"), "strict": true})));
    refused(s.run("file.save", &json!({"path": d.join("b.png"), "strict": true})));
    assert!(!d.join("a.pdf").exists() && !d.join("a.png").exists() && !d.join("b.png").exists());
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_decodable_page_background_picture_passes() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 200, 255, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut doc = Document::from_text("Hello");
    doc.media.insert("bg.png".into(), std::sync::Arc::new(png));
    doc.settings.page_picture = Some(wordcraft_doc::PagePicture { media: "bg.png".into(), fill_type: "stretch".into(), title: String::new() });
    let mut s = Session::new(doc);
    assert_eq!(s.run("file.strictReport", &json!({})).unwrap()["strict"], "passed");
}

#[test]
fn a_picture_wider_than_the_pdf_draws_is_refused_for_pdf_only() {
    // 40000 x 10: past the PDF's decode limit (30000), within what the page raster draws.
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(40_000, 10, image::Rgba([9, 9, 9, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut doc = Document::from_text("Hello");
    doc.media.insert("wide.png".into(), std::sync::Arc::new(png));
    doc.settings.page_picture = Some(wordcraft_doc::PagePicture { media: "wide.png".into(), fill_type: "stretch".into(), title: String::new() });
    let mut s = Session::new(doc);
    assert_eq!(s.run("file.strictReport", &json!({})).unwrap()["strict"], "failed", "PDF");
    assert_eq!(s.run("file.strictReport", &json!({"page": 1})).unwrap()["strict"], "passed", "PNG");
}

fn watermark(text: &str) -> Document {
    let mut doc = Document::from_text("Hello");
    doc.settings.watermark = Some(wordcraft_doc::Watermark { text: text.into(), ..Default::default() });
    doc
}

#[test]
fn watermark_text_no_font_can_draw_is_refused_and_latin_passes() {
    let cs = uncovered_many(2);
    let d = dir("watermark");
    let mut s = Session::new(watermark(&cs.iter().collect::<String>()));
    let r = s.run("file.strictReport", &json!({})).unwrap();
    assert_eq!(r["strict"], "failed");
    let cps: Vec<String> = cs.iter().map(|c| format!("U+{:04X}", *c as u32)).collect();
    let got: Vec<String> = r["missing_glyphs"].as_array().unwrap().iter().map(|g| g["cp"].as_str().unwrap().to_string()).collect();
    assert_eq!(got, cps);
    refused(s.run("file.exportPdf", &json!({"path": d.join("w.pdf"), "strict": true})));
    refused(s.run("file.exportPng", &json!({"path": d.join("w.png"), "strict": true})));
    assert!(!d.join("w.pdf").exists() && !d.join("w.png").exists());

    let mut ok = Session::new(watermark("CONFIDENTIAL"));
    assert_eq!(ok.run("file.strictReport", &json!({})).unwrap()["strict"], "passed");
    ok.run("file.exportPdf", &json!({"path": d.join("ok.pdf"), "strict": true})).unwrap();
    ok.run("file.exportPng", &json!({"path": d.join("ok.png"), "strict": true})).unwrap();
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn wordart_text_effects_keep_their_glyphs_in_the_check() {
    let c = uncovered();
    let props = wordcraft_doc::CharProps { shadow: Some(true), outline: Some(true), ..Default::default() };
    let mut doc = Document::from_text("");
    doc.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(&format!("a{c}b"), props))].into_iter().collect();
    let mut s = Session::new(doc);
    let r = s.run("file.strictReport", &json!({})).unwrap();
    assert_eq!(r["missing_glyphs"][0]["cp"], format!("U+{:04X}", c as u32), "{r}");
}

#[test]
fn each_list_says_for_itself_that_it_was_cut() {
    let many = uncovered_many(300);
    let mut doc = Document::from_text(&many.iter().collect::<String>());
    garbage_background(&mut doc);
    let mut s = Session::new(doc);
    let r = s.run("file.strictReport", &json!({})).unwrap();
    assert_eq!(r["missing_glyphs"].as_array().unwrap().len(), 256);
    assert_eq!((r["truncated"].clone(), r["glyphs_truncated"].clone(), r["pictures_truncated"].clone()), (json!(true), json!(true), json!(false)));
    let lines = wordcraft_engine::strict::lines_from_json(&r);
    assert!(lines[0].ends_with(", …"), "{}", lines[0]);
    assert!(lines[0].starts_with("strict: 300 characters cannot be drawn (first 256 listed): "), "{}", lines[0]);
    assert!(!lines[1].contains('…'), "{}", lines[1]);
    assert_eq!((r["total_missing_glyphs"].clone(), r["total_missing_pictures"].clone()), (json!(300), json!(1)));
}

#[test]
fn a_truncated_report_carries_totals_past_the_cap() {
    let many = uncovered_many(300);
    let mut doc = Document::from_text(&many.iter().collect::<String>());
    for n in 0..300 {
        doc.missing_pictures.push(wordcraft_doc::MissingPicture {
            part: format!("word/media/gone{n}.png"),
            story: "word/document.xml".into(),
            reason: wordcraft_doc::MissingPictureReason::MissingPart,
        });
        // The same record again is one picture.
        doc.missing_pictures.push(wordcraft_doc::MissingPicture {
            part: format!("word/media/gone{n}.png"),
            story: "word/document.xml".into(),
            reason: wordcraft_doc::MissingPictureReason::MissingPart,
        });
    }
    let mut s = Session::new(doc);
    let r = s.run("file.strictReport", &json!({})).unwrap();
    assert_eq!(r["missing_glyphs"].as_array().unwrap().len(), 256);
    assert_eq!(r["missing_pictures"].as_array().unwrap().len(), 256);
    assert_eq!(r["truncated"], true);
    assert_eq!(r["total_missing_glyphs"], 300);
    assert_eq!(r["total_missing_pictures"], 300);
    assert_eq!(r["missing_pictures"][0]["reason"], "missing_part");
    assert_eq!(r["missing_pictures"][0]["source_part"], "word/document.xml");
}

#[test]
fn strict_off_gives_the_same_png_as_before_even_with_a_missing_character() {
    let c = uncovered();
    let d = dir("pngsame");
    let mut s = Session::new(Document::from_text(&format!("Hello {c}")));
    let png: Vec<Vec<u8>> = [json!({}), json!({"strict": false})]
        .into_iter()
        .enumerate()
        .map(|(i, mut p)| {
            p["path"] = json!(d.join(format!("{i}.png")));
            s.run("file.exportPng", &p).unwrap();
            std::fs::read(d.join(format!("{i}.png"))).unwrap()
        })
        .collect();
    assert!(png[0].starts_with(b"\x89PNG"));
    assert_eq!(png[0], png[1]);
    std::fs::remove_dir_all(d).unwrap();
}
