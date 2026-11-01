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

/// A .docx with paragraphs `body`, font table entries `fonts`, the docDefaults' and a `Body`
/// paragraph style's run properties `defaults` and `style`, and a theme whose minor font is `minor`.
fn docx_with_font_table(body: &str, fonts: &str, defaults: &str, style: &str, minor: &str) -> Vec<u8> {
    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
    const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const PR: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    const CT: &str = "application/vnd.openxmlformats-officedocument";
    let rels = |items: &[(&str, &str)]| {
        let list: String = items.iter().map(|(t, g)| format!(r#"<Relationship Id="rId{t}" Type="{R}/{t}" Target="{g}"/>"#)).collect();
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{PR}">{list}</Relationships>"#)
    };
    let types = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{CT}.wordprocessingml.document.main+xml"/><Override PartName="/word/fontTable.xml" ContentType="{CT}.wordprocessingml.fontTable+xml"/><Override PartName="/word/styles.xml" ContentType="{CT}.wordprocessingml.styles+xml"/><Override PartName="/word/theme/theme1.xml" ContentType="{CT}.theme+xml"/></Types>"#
    );
    let files = [
        ("[Content_Types].xml", types),
        ("_rels/.rels", rels(&[("officeDocument", "word/document.xml")])),
        ("word/_rels/document.xml.rels", rels(&[("fontTable", "fontTable.xml"), ("styles", "styles.xml"), ("theme", "theme/theme1.xml")])),
        ("word/document.xml", format!(r#"<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#)),
        ("word/fontTable.xml", format!(r#"<w:fonts xmlns:w="{W}">{fonts}</w:fonts>"#)),
        (
            "word/styles.xml",
            format!(
                r#"<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr>{defaults}</w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Body"><w:name w:val="Body"/><w:basedOn w:val="Normal"/><w:rPr>{style}</w:rPr></w:style></w:styles>"#
            ),
        ),
        (
            "word/theme/theme1.xml",
            format!(
                r#"<a:theme xmlns:a="{A}" name="T"><a:themeElements><a:fontScheme name="T"><a:majorFont><a:latin typeface="{minor}"/></a:majorFont><a:minorFont><a:latin typeface="{minor}"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#
            ),
        ),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, text) in files {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zip, text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Measured in Word: a font it doesn't have is drawn in the font table's `w:altName` (when it
/// has that font) whether a run, the theme (`w:asciiTheme`), the document defaults or a
/// paragraph style names it; a font it has is drawn in itself, whatever the alternate. Once the
/// font is installed it is drawn in itself, not the alternate.
#[test]
fn a_missing_font_lays_out_in_the_font_tables_alternate_wherever_it_is_named() {
    const TEXT: &str = "illicit little lilies fill tiny jill";
    const FAMILY: &str = "Altfont Sans3";
    let rfonts = |f: &str| format!(r#"<w:rFonts w:ascii="{f}" w:hAnsi="{f}"/>"#);
    let entry = |alt: &str| {
        let alt = if alt.is_empty() { String::new() } else { format!(r#"<w:altName w:val="{alt}"/>"#) };
        format!(r#"<w:font w:name="{FAMILY}">{alt}<w:family w:val="roman"/></w:font>"#)
    };
    let run = |rpr: &str| format!(r#"<w:r><w:rPr>{rpr}</w:rPr><w:t>{TEXT}</w:t></w:r>"#);
    let end_x = |s: &mut Session| s.layout().caret_on(&Pos::body(0, TEXT.len()), 0).unwrap().x;
    let open = |bytes: &[u8]| Session::new(crate::io::open_bytes("alt.docx", bytes).unwrap());
    let direct = format!("<w:p>{}</w:p>", run(&rfonts(FAMILY)));
    let x_in = |family: &str| end_x(&mut open(&docx_with_font_table(&format!("<w:p>{}</w:p>", run(&rfonts(family))), "", "", "", "Source Serif 4")));
    let (mono, sans, cambria) = (x_in("JetBrains Mono"), x_in("Source Sans 3"), x_in("Cambria"));
    let named = [
        ("a run", direct.clone(), String::new(), String::new()),
        (
            "the theme",
            format!("<w:p>{}</w:p>", run(r#"<w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi"/>"#)),
            String::new(),
            String::new(),
        ),
        ("the defaults", format!("<w:p>{}</w:p>", run("")), rfonts(FAMILY), String::new()),
        ("a style", format!(r#"<w:p><w:pPr><w:pStyle w:val="Body"/></w:pPr>{}</w:p>"#, run("")), String::new(), rfonts(FAMILY)),
    ];
    for (place, body, defaults, style) in &named {
        let x = |alt: &str| end_x(&mut open(&docx_with_font_table(body, &entry(alt), defaults, style, FAMILY)));
        assert!((x("JetBrains Mono") - mono).abs() < 0.01, "{place}: drawn in the alternate");
        assert!((x("") - cambria).abs() < 0.01, "{place}: no alternate, a roman font is drawn in Cambria");
        assert!((x("Not Installed Either") - cambria).abs() < 0.01, "{place}: an alternate that is missing too counts as none");
    }
    let mut s = open(&docx_with_font_table(&direct, &entry("JetBrains Mono"), "", "", FAMILY));
    assert!((end_x(&mut s) - mono).abs() < 0.01);
    let data = base64_encode(&renamed("SourceSans3-Regular.ttf", "Source Sans 3", FAMILY));
    s.run("font.add", &json!({"data": data, "name": "AltfontSans3-Regular.ttf"})).unwrap();
    assert!((end_x(&mut s) - sans).abs() < 0.01, "measured with the installed font, not its alternate");
    s.run("font.remove", &json!({"family": FAMILY})).unwrap();
    assert!((end_x(&mut s) - mono).abs() < 0.01, "removed again: the alternate");
}
