//! Fonts added at run time (#582): `font.add`, `font.list`, `font.remove`.

use serde_json::json;
use wordcraft_doc::Pos;

use crate::Session;
use crate::cmd::insert::base64_encode;

/// The bundled face `file` of family `original` renamed `family` (as long as `original`), so it
/// is a font the database doesn't have.
fn renamed(file: &str, original: &str, family: &str) -> Vec<u8> {
    assert_eq!(family.len(), original.len());
    let mut data = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16(original), utf16(family)), (original.as_bytes().to_vec(), family.as_bytes().to_vec())] {
        let mut i = 0;
        while let Some(at) = data[i..].windows(from.len()).position(|w| w == from) {
            data[i + at..i + at + to.len()].copy_from_slice(&to);
            i += at + to.len();
        }
    }
    data
}

/// The caret's x after `text` set in `font`.
fn text_end_x(s: &mut Session, font: &str, text: &str) -> f32 {
    s.run("document.setText", &json!({"text": text})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    s.run("format.font", &json!({"name": font})).unwrap();
    s.layout().caret_on(&Pos::body(0, text.len()), 0).unwrap().x
}

#[test]
fn an_added_font_is_listed_and_lays_out_text_already_set_in_it() {
    // A monospaced face, so it measures unlike whatever proportional font the system falls back
    // to (Source Serif 4 itself where no system fonts are installed, as on the FreeBSD runner).
    // The name has no Serif/Sans/Mono in it, which would pick a fallback of that kind.
    const FAMILY: &str = "Cmdfont Face 4";
    // Narrow letters: set monospaced they take far more room than in any proportional font.
    const TEXT: &str = "illicit little lilies fill tiny jill";
    let mut s = Session::new(wordcraft_doc::Document::new());
    let mono = text_end_x(&mut s, "JetBrains Mono", TEXT);
    // Not installed yet: the text falls back to another font.
    let missing = text_end_x(&mut s, FAMILY, TEXT);
    assert!((missing - mono).abs() > 1.0, "the fallback measures differently: {missing} vs {mono}");

    let data = base64_encode(&renamed("JetBrainsMono-Regular.ttf", "JetBrains Mono", FAMILY));
    let r = s.run("font.add", &json!({"data": data, "name": "CmdfontFace4-Regular.ttf"})).unwrap();
    assert_eq!(r["families"], json!([FAMILY]));
    assert_eq!(r["faces"][0]["style"], "Regular");
    // The text already set in it re-lays out with it, without touching the document.
    let x = s.layout().caret_on(&Pos::body(0, TEXT.len()), 0).unwrap().x;
    assert!((x - mono).abs() < 0.01, "measured with the added font: {x} vs {mono} (missing {missing})");

    let list = s.run("font.list", &json!({"all": true})).unwrap();
    assert!(list["fonts"].as_array().unwrap().iter().any(|f| f["families"] == json!([FAMILY])));
    assert!(list["families"].as_array().unwrap().iter().any(|f| f == FAMILY));
    // It exports to PDF like any other font.
    assert!(crate::io::save_bytes("out.pdf", &s.doc).unwrap().starts_with(b"%PDF"));

    // Adding it again changes nothing.
    let again = s.run("font.add", &json!({"data": data, "name": "again.ttf"}));
    assert!(again.unwrap_err().to_string().contains("already available"));

    let removed = s.run("font.remove", &json!({"family": FAMILY})).unwrap();
    assert_eq!(removed["removed"].as_array().unwrap().len(), 1);
    let x = s.layout().caret_on(&Pos::body(0, TEXT.len()), 0).unwrap().x;
    assert!((x - missing).abs() < 0.01, "falls back again once removed");
    assert!(s.run("font.remove", &json!({"family": FAMILY})).is_err());
}

#[test]
fn adding_a_font_never_changes_text_that_does_not_use_it() {
    const FAMILY: &str = "Metric Serif 4";
    let mut s = Session::new(crate::sample::sample_document());
    let before = s.layout();
    let positions = |s: &mut Session| {
        let l = s.layout();
        (0..s.doc.body.len().min(12)).filter_map(|b| l.caret_on(&Pos::body(b, 0), 0).map(|c| (c.page, c.x, c.top))).collect::<Vec<_>>()
    };
    let a = positions(&mut s);
    s.run("font.add", &json!({"data": base64_encode(&renamed("SourceSerif4-Bold.ttf", "Source Serif 4", FAMILY)), "name": "m.ttf"})).unwrap();
    assert_eq!(positions(&mut s), a);
    assert_eq!(s.layout().pages.len(), before.pages.len());
}

#[test]
fn bad_font_input_is_an_error() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    for p in [
        json!({"data": base64_encode(b"definitely not a font")}),
        json!({"data": "%%% not base64"}),
        json!({"data": base64_encode(b"wOF2\0\x01\0\0\xff\xff\xff\xff")}),
        json!({"path": "/nonexistent/font.ttf"}),
        json!({"path": env!("CARGO_MANIFEST_DIR")}),
        json!({"path": concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")}),
        json!({"url": "https://fonts.googleapis.com/css2?family=Roboto"}),
    ] {
        assert!(s.run("font.add", &p).is_err(), "{p}");
    }
    assert!(s.run("font.remove", &json!({})).is_err());
    assert!(s.run("font.remove", &json!({"family": "Source Sans 3"})).is_err(), "the bundled fonts stay");
    // No file: the UI is asked for a picker.
    let r = s.run("font.add", &json!({})).unwrap();
    assert_eq!(r["pending"], true);
    assert!(s.ui_requests.iter().any(|r| r["open"] == "addFont"));

    let r = s.run("font.loadSystem", &json!({})).unwrap();
    assert_eq!(r["pending"], true);
    assert!(s.ui_requests.iter().any(|r| r["open"] == "loadSystemFonts"));
}

/// A .docx whose text is set in `family`, embedding `font` through the font table (#1171): as an
/// obfuscated `.odttf` when `key` is a real GUID, as a plain `.ttf` when it is all zeros.
fn docx_embedding(family: &str, font: Option<(&[u8], &str)>, text: &str) -> Vec<u8> {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const PR: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    let rels = |items: &[(&str, &str)]| {
        let list: String = items.iter().map(|(t, g)| format!(r#"<Relationship Id="rId{t}" Type="{R}/{t}" Target="{g}"/>"#)).collect();
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{PR}">{list}</Relationships>"#)
    };
    let mut files: Vec<(String, Vec<u8>)> = vec![
        (
            "[Content_Types].xml".into(),
            br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="ttf" ContentType="application/x-font-ttf"/><Default Extension="odttf" ContentType="application/vnd.openxmlformats-officedocument.obfuscatedFont"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/fontTable.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"/></Types>"#.to_vec(),
        ),
        ("_rels/.rels".into(), rels(&[("officeDocument", "word/document.xml")]).into_bytes()),
        ("word/_rels/document.xml.rels".into(), rels(&[("fontTable", "fontTable.xml")]).into_bytes()),
        (
            "word/document.xml".into(),
            format!(r#"<w:document xmlns:w="{W}"><w:body><w:p><w:r><w:rPr><w:rFonts w:ascii="{family}" w:hAnsi="{family}"/><w:sz w:val="20"/></w:rPr><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#).into_bytes(),
        ),
    ];
    let mut table = format!(r#"<w:fonts xmlns:w="{W}" xmlns:r="{R}"><w:font w:name="{family}">"#);
    if let Some((data, key)) = font {
        let digits = key.replace(['{', '}', '-'], "");
        let hex: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let zero = hex.iter().all(|b| *b == 0);
        let mut data = data.to_vec();
        // Obfuscation (ECMA-376 Part 1, 17.8.1): the first 32 bytes XOR the key's bytes, reversed.
        for i in 0..32 {
            data[i] ^= hex[15 - i % 16];
        }
        let ext = if zero { "ttf" } else { "odttf" };
        table.push_str(&format!(r#"<w:embedRegular r:id="rIdfont" w:fontKey="{key}" w:subsetted="0"/>"#));
        files.push(("word/_rels/fontTable.xml.rels".into(), rels(&[("font", &format!("fonts/font1.{ext}"))]).into_bytes()));
        files.push((format!("word/fonts/font1.{ext}"), data));
    }
    table.push_str("</w:font></w:fonts>");
    files.push(("word/fontTable.xml".into(), table.into_bytes()));
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zip, &bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn fonts_embedded_in_a_docx_lay_out_its_text() {
    const TEXT: &str = "illicit little lilies fill tiny jill";
    let end_x =
        |bytes: &[u8]| Session::new(crate::io::open_bytes("e.docx", bytes).unwrap()).layout().caret_on(&Pos::body(0, TEXT.len()), 0).unwrap().x;
    let mono = end_x(&docx_embedding("JetBrains Mono", None, TEXT));
    // Both forms Word and other producers write: an obfuscated .odttf with its key, a plain .ttf
    // with the all-zero key. Each is a family no other test or system has.
    for (family, key) in [("JetBrains Odtt", "{9F1C2A4E-7B3D-4C58-A1E6-0D2B8F5C3E77}"), ("JetBrains Zero", "{00000000-0000-0000-0000-000000000000}")]
    {
        let font = renamed("JetBrainsMono-Regular.ttf", "JetBrains Mono", family);
        let missing = end_x(&docx_embedding(family, None, TEXT));
        assert!((missing - mono).abs() > 1.0, "{family}: not embedded, the text falls back: {missing} vs {mono}");
        let x = end_x(&docx_embedding(family, Some((&font, key)), TEXT));
        assert!((x - mono).abs() < 0.01, "{family}: measured with the embedded font: {x} vs {mono} (missing {missing})");
    }
}

/// A font Word embeds as a subset ("embed only the characters used") goes by a made-up family
/// inside (`___WRD_EMBED_SUB_46`), and the text asks for it by the font table's name
/// (`OBRBWW+Century Schoolbook`): the text is measured with it all the same (#2240). It fell back
/// to a substitute, as if nothing were embedded.
#[test]
fn a_subset_font_word_embeds_lays_out_text_set_in_its_table_name() {
    const TEXT: &str = "illicit little lilies fill tiny jill";
    const FAMILY: &str = "QWERTY+JetBrains Sub";
    let end_x =
        |bytes: &[u8]| Session::new(crate::io::open_bytes("e.docx", bytes).unwrap()).layout().caret_on(&Pos::body(0, TEXT.len()), 0).unwrap().x;
    let mono = end_x(&docx_embedding("JetBrains Mono", None, TEXT));
    let missing = end_x(&docx_embedding(FAMILY, None, TEXT));
    assert!((missing - mono).abs() > 1.0, "not embedded, the text falls back: {missing} vs {mono}");
    let subset = renamed("JetBrainsMono-Regular.ttf", "JetBrains Mono", "___WRD_EMBED_7");
    let x = end_x(&docx_embedding(FAMILY, Some((&subset, "{6A0E3B57-1C2D-4E8F-9A7B-3D5C1E0F2B64}")), TEXT));
    assert!((x - mono).abs() < 0.01, "measured with the embedded subset: {x} vs {mono} (missing {missing})");
}

/// `font.missing`'s fonts, as "font>shownAs".
fn missing_list(s: &mut Session) -> Vec<String> {
    let m = s.run("font.missing", &json!({})).unwrap();
    m["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let shown: Vec<&str> = f["shownAs"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
            format!("{}>{}", f["font"].as_str().unwrap(), shown.join(","))
        })
        .collect()
}

/// The missing fonts of `doc` as laid out.
fn missing_in(doc: wordcraft_doc::Document) -> Vec<crate::cmd::font::MissingFont> {
    let mut s = Session::new(doc);
    let layout = s.layout();
    crate::cmd::font::missing_fonts(&s.doc, &layout)
}

/// A paragraph of `text` set in `font`.
fn para_in(text: &str, font: &str) -> wordcraft_doc::Paragraph {
    wordcraft_doc::Paragraph::with_text(text, wordcraft_doc::CharProps { font: Some(font.into()), ..Default::default() })
}

/// A document of these paragraphs.
fn doc_of(paras: Vec<wordcraft_doc::Paragraph>) -> wordcraft_doc::Document {
    let mut doc = wordcraft_doc::Document::new();
    doc.body = paras.into_iter().map(wordcraft_doc::para_block).collect();
    doc
}

#[test]
fn fonts_that_are_not_installed_are_reported_with_the_font_shown_instead() {
    // A family no system has, with no Serif/Mono in its name: it's drawn in a sans substitute.
    const MISSING: &str = "Wordcraft Nowhere Face";
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("document.setText", &json!({"text": "Bundled\nMissing"})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    s.run("format.font", &json!({"name": "JetBrains Mono"})).unwrap();
    assert!(s.run("format.state", &json!({})).unwrap()["fontShownAs"].is_null(), "a bundled font is installed");
    assert_eq!(missing_list(&mut s), Vec::<String>::new());

    s.run("select.text", &json!({"text": "Missing"})).unwrap();
    s.run("format.font", &json!({"name": MISSING})).unwrap();
    let st = s.run("format.state", &json!({})).unwrap();
    assert_eq!(st["font"], MISSING);
    let shown = st["fontShownAs"].as_str().unwrap_or_default().to_string();
    assert!(!shown.is_empty() && shown != MISSING, "the font box learns the substitute: {st}");
    assert_eq!(missing_list(&mut s), [format!("{MISSING}>{shown}")], "only the missing font, once");

    // A mixed selection has no single font, so nothing to flag in the font box.
    s.run("select.all", &json!({})).unwrap();
    assert!(s.run("format.state", &json!({})).unwrap()["fontShownAs"].is_null());
}

#[test]
fn a_font_only_a_style_names_is_not_reported_missing() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("document.setText", &json!({"text": "Body"})).unwrap();
    s.run("styles.create", &json!({"name": "Unused Look"})).unwrap();
    s.run("styles.modify", &json!({"style": "Unused Look", "chr": {"font": "Wordcraft Style Only Face"}})).unwrap();
    // Creating a style applies it, as in Word: put the text back in Normal.
    s.run("para.style", &json!({"style": "Normal"})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    s.run("format.font", &json!({"name": "JetBrains Mono"})).unwrap();
    assert_eq!(missing_list(&mut s), Vec::<String>::new(), "no text uses the style");
    // Text in the style, with no font of its own, is.
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("document.setText", &json!({"text": "Body"})).unwrap();
    s.run("styles.create", &json!({"name": "Used Look"})).unwrap();
    s.run("styles.modify", &json!({"style": "Used Look", "chr": {"font": "Wordcraft Style Only Face"}})).unwrap();
    s.run("para.style", &json!({"style": "Used Look"})).unwrap();
    let list = missing_list(&mut s);
    assert!(list.iter().any(|f| f.starts_with("Wordcraft Style Only Face>")), "{list:?}");
}

#[test]
fn missing_fonts_are_what_layout_drew_in_shown_text() {
    let mut doc = doc_of(vec![
        para_in("one", "Wordcraft Nowhere Face"),
        para_in("two", "wordcraft nowhere face"),
        para_in("three", "JetBrains Mono"),
        // Spaces, a picture's placeholder and zero-width characters show no font.
        para_in(" \u{FFFC}\t\u{200B}\u{FEFF}\u{FE0F}\u{3164}\u{034F}", "Wordcraft Blank Face"),
    ]);
    // Hidden text isn't laid out.
    doc.body.push(wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text(
        "secret",
        wordcraft_doc::CharProps { font: Some("Wordcraft Hidden Face".into()), hidden: Some(true), ..Default::default() },
    )));
    // A text box story nothing shows any more (deleted, not yet pruned).
    doc.parts.insert(
        77,
        wordcraft_doc::Part {
            kind: wordcraft_doc::PartKind::TextBox,
            blocks: vec![wordcraft_doc::para_block(para_in("gone", "Wordcraft Orphan Face"))],
            body: Default::default(),
        },
    );
    let names: Vec<String> = missing_in(doc).into_iter().map(|f| f.font).collect();
    assert_eq!(names.len(), 1, "one font whatever its case, and nothing else: {names:?}");
    assert!(names[0].eq_ignore_ascii_case("Wordcraft Nowhere Face"));
}

#[test]
fn metric_twins_are_listed_but_flagged_compatible() {
    // A twin by name, installed or not here: "Liberation Sans" stands in for Arial wherever it's
    // the substitute; a made-up family's substitute is never its twin.
    let m = missing_in(doc_of(vec![para_in("Text 😀", "Arial"), para_in("Text", "Wordcraft Nowhere Face")]));
    let nowhere = m.iter().find(|f| f.font == "Wordcraft Nowhere Face").expect("listed");
    assert!(!nowhere.metric_compatible, "{nowhere:?}");
    match crate::cmd::font::shown_as(&wordcraft_doc::Document::new(), "Arial") {
        // Arial is installed: not listed.
        None => assert!(!m.iter().any(|f| f.font == "Arial"), "{m:?}"),
        Some(sub) => {
            let arial = m.iter().find(|f| f.font == "Arial").expect("listed");
            // An emoji drawn in a fallback face doesn't change the verdict on the substitute.
            assert_eq!(arial.metric_compatible, wordcraft_fonts::word::metric_compatible("Arial", &sub), "{arial:?}");
        }
    }
}

#[test]
fn a_missing_roman_font_drawn_in_cambria_is_reported() {
    // Word draws a missing font the font table classes as roman in Cambria (#1613): installed or
    // not, Cambria isn't the font the document asks for.
    const ROMAN: &str = "Wordcraft Roman Gone";
    let mut doc = doc_of(vec![para_in("Text", ROMAN)]);
    doc.settings.roman_fonts = vec![ROMAN.into()];
    let m = missing_in(doc.clone());
    assert_eq!(m.iter().map(|f| f.font.as_str()).collect::<Vec<_>>(), [ROMAN], "{m:?}");
    assert!(!m[0].metric_compatible, "{m:?}");
    // The font box names the face layout draws, not the generic substitute.
    let cambria = wordcraft_fonts::word::resolve("Cambria", false, false).face.get().family.clone();
    assert_eq!(m[0].shown_as, std::slice::from_ref(&cambria));
    let mut s = Session::new(doc);
    s.run("select.all", &json!({})).unwrap();
    assert_eq!(s.run("format.state", &json!({})).unwrap()["fontShownAs"], cambria.as_str());
}

#[test]
fn a_missing_east_asian_font_is_reported_for_cjk_text_only() {
    let props =
        wordcraft_doc::CharProps { font: Some("JetBrains Mono".into()), font_ea: Some("Wordcraft Nowhere Mincho".into()), ..Default::default() };
    let m = missing_in(doc_of(vec![wordcraft_doc::Paragraph::with_text("abc 日本語", props.clone())]));
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].font, "Wordcraft Nowhere Mincho");
    assert!(!m[0].shown_as.is_empty() && !m[0].metric_compatible, "{m:?}");
    // Without CJK text the East Asian font draws nothing.
    assert!(missing_in(doc_of(vec![wordcraft_doc::Paragraph::with_text("abc", props)])).is_empty());
}

#[test]
fn symbol_fonts_are_shown_as_unicode_without_a_missing_font() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("document.setText", &json!({"text": "\u{F0FC}"})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    s.run("format.font", &json!({"name": "Wingdings"})).unwrap();
    // Not installed, so it's drawn in another face, but on purpose: nothing to report.
    if wordcraft_fonts::word::resolve("Wingdings", false, false).substituted {
        assert!(s.run("format.state", &json!({})).unwrap()["fontShownAs"].is_string());
    }
    let layout = s.layout();
    assert!(crate::cmd::font::missing_fonts(&s.doc, &layout).is_empty());
}

#[test]
fn missing_fonts_from_command_search_say_so_in_the_status_line() {
    let mut s = Session::new(doc_of(vec![para_in("Text", "Wordcraft Nowhere Face")]));
    s.run("font.missing", &json!({})).unwrap();
    assert!(s.status.contains("Wordcraft Nowhere Face"), "{}", s.status);
    // Thousands of missing fonts: a few named, the rest counted.
    let mut s = Session::new(doc_of((0..40).map(|i| para_in("x", &format!("Wordcraft Many Face {i}"))).collect()));
    s.run("font.missing", &json!({})).unwrap();
    assert!(s.status.ends_with(" and 35 more") && s.status.len() < 300, "{}", s.status);
}

#[test]
fn a_huge_font_name_is_kept_short() {
    let name = format!("Wordcraft {}", "é".repeat(100_000));
    let m = missing_in(doc_of(vec![para_in("Text", &name)]));
    assert_eq!(m.len(), 1);
    assert!(m[0].font.chars().count() <= 256, "{}", m[0].font.len());
}

#[test]
fn symbol_fonts_drawn_as_unicode_are_not_reported() {
    // Word's bullets are Symbol characters; WordCraft draws them as their Unicode equivalents.
    let m = missing_in(doc_of(vec![para_in("\u{F0B7}", "Symbol"), para_in("\u{F0FC}", "Wingdings")]));
    assert!(m.is_empty(), "{m:?}");
}

#[test]
fn cjk_text_is_reported_under_its_east_asian_font_not_the_latin_one() {
    let props =
        wordcraft_doc::CharProps { font: Some("Wordcraft Latin Gone".into()), font_ea: Some("Wordcraft Asian Gone".into()), ..Default::default() };
    let names = |text: &str| -> Vec<String> {
        missing_in(doc_of(vec![wordcraft_doc::Paragraph::with_text(text, props.clone())])).into_iter().map(|f| f.font).collect()
    };
    assert_eq!(names("日本語"), ["Wordcraft Asian Gone"], "no character asks for the Latin font");
    assert_eq!(names("abc 日本語"), ["Wordcraft Asian Gone", "Wordcraft Latin Gone"]);
    // Without an East Asian font the CJK text asks for the Latin one.
    let latin_only = wordcraft_doc::CharProps { font: Some("Wordcraft Latin Gone".into()), ..Default::default() };
    let m = missing_in(doc_of(vec![wordcraft_doc::Paragraph::with_text("日本語", latin_only)]));
    assert_eq!(m.iter().map(|f| f.font.as_str()).collect::<Vec<_>>(), ["Wordcraft Latin Gone"]);
}

#[test]
fn a_symbol_font_named_for_complex_script_text_is_not_reported() {
    let props = wordcraft_doc::CharProps { font: Some("JetBrains Mono".into()), font_cs: Some("Symbol".into()), ..Default::default() };
    assert!(missing_in(doc_of(vec![wordcraft_doc::Paragraph::with_text("سلام", props)])).is_empty());
}

#[test]
fn a_hostile_number_of_missing_fonts_stays_bounded() {
    let paras = (0..3000).map(|i| para_in("x", &format!("Wordcraft Hostile Face {i}"))).collect();
    let m = missing_in(doc_of(paras));
    assert!(!m.is_empty() && m.len() <= wordcraft_layout::MAX_SUBSTITUTED, "{}", m.len());
}

#[test]
fn a_font_key_matches_the_listed_name_however_long_or_padded() {
    // Leading spaces and more than 256 characters: layout keeps 256, the list trims them.
    let name = format!("   Wordcraft {}", "x".repeat(400));
    let m = missing_in(doc_of(vec![para_in("Text", &name)]));
    assert_eq!(m.len(), 1);
    assert_eq!(crate::cmd::font::font_key(&name), crate::cmd::font::font_key(&m[0].font));
    // The status line shortens such names.
    let mut s = Session::new(doc_of(vec![para_in("Text", &name)]));
    s.run("font.missing", &json!({})).unwrap();
    assert!(s.status.chars().count() < 80, "{}", s.status);
}

#[test]
fn only_characters_that_draw_count() {
    use wordcraft_layout::para::draws_something;
    for c in ['a', '日', '€', '\u{1F600}'] {
        assert!(draws_something(c), "{c:?}");
    }
    for c in [' ', '\t', '\u{FFFC}', '\u{00AD}', '\u{200B}', '\u{FE0F}', '\u{3164}', '\u{FFF0}', '\u{1BCA0}', '\u{E0041}'] {
        assert!(!draws_something(c), "{c:?}");
    }
}

#[test]
fn a_blank_font_name_is_not_a_missing_font() {
    // A file's `w:ascii=" "`: the text is drawn in a substitute, but there's no font to name.
    assert!(missing_in(doc_of(vec![para_in("Text", " "), para_in("More", "\t")])).is_empty());
}

#[test]
fn a_padded_name_is_judged_by_the_face_its_text_is_drawn_in() {
    // Layout looks "Arial " up spaces and all, so its text is in a substitute even where Arial is
    // installed; one of Arial's twins (Helvetica on a Mac) measures like it.
    let m = missing_in(doc_of(vec![para_in("Text", "Arial ")]));
    let Some(arial) = m.iter().find(|f| f.font == "Arial") else { return };
    let drawn = arial.shown_as.first().expect("drawn in a face");
    assert_eq!(arial.metric_compatible, wordcraft_fonts::word::metric_compatible("Arial", drawn), "{arial:?}");
}

#[test]
fn an_east_asian_font_is_shown_as_the_face_its_text_is_drawn_in() {
    // MS Mincho is roman in many font tables, but Cambria is the run's font's substitute: an
    // East Asian font's text goes to the run's font or a fallback face, as the list says.
    const MINCHO: &str = "Wordcraft Mincho Gone";
    let props = wordcraft_doc::CharProps { font: Some("JetBrains Mono".into()), font_ea: Some(MINCHO.into()), ..Default::default() };
    let mut doc = doc_of(vec![wordcraft_doc::Paragraph::with_text("日", props)]);
    doc.settings.roman_fonts = vec![MINCHO.into()];
    let shown = crate::cmd::font::east_asian_shown_as(&doc, MINCHO, "JetBrains Mono").expect("not installed");
    let m = missing_in(doc.clone());
    assert_eq!(m.iter().map(|f| f.font.as_str()).collect::<Vec<_>>(), [MINCHO], "{m:?}");
    assert_eq!(m[0].shown_as, [shown], "{m:?}");
    assert!(crate::cmd::font::east_asian_shown_as(&doc, "JetBrains Mono", "JetBrains Mono").is_none(), "installed");
}
