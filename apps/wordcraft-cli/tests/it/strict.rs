//! `convert --strict` (#6005): exit 3 and a report when a PDF or PNG would silently lack a
//! character or a picture, nothing written; exit 0 otherwise.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use wordcraft_doc::Document;
use wordcraft_fonts::FontDb;

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wc-cli-strict-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn convert(input: &Path, output: &Path, strict: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"));
    cmd.arg("convert").arg(input).arg(output);
    if strict {
        cmd.arg("--strict");
    }
    cmd.output().unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// The report's JSON line: the last line of stderr.
fn report_json(o: &Output) -> serde_json::Value {
    let text = stderr(o);
    let last = text.lines().last().unwrap_or_default();
    serde_json::from_str(last).unwrap_or_else(|e| panic!("last stderr line is not JSON ({e}): {text}"))
}

fn json_input(d: &Path, text: &str) -> PathBuf {
    let path = d.join("in.json");
    std::fs::write(&path, serde_json::to_vec(&Document::from_text(text)).unwrap()).unwrap();
    path
}

/// Built without craft-fonts, the host's own fonts decide what is drawable: skip, as the layout
/// tests do.
fn without_fonts() -> bool {
    let missing = option_env!("CRAFT_FONTS_DIR").is_none_or(str::is_empty);
    if missing {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a checkout)");
    }
    missing
}

/// A code point in unassigned plane 4 that no face of the host covers.
fn uncovered() -> char {
    let db = FontDb::global();
    let primary = wordcraft_fonts::word::resolve(&Document::default().settings.minor_font, false, false).face;
    (0x40000u32..0x4FFF0)
        .filter_map(char::from_u32)
        .find(|c| !primary.covers(*c) && db.fallback_styled(*c, primary.id(), false, false).is_none() && db.fallback_for(*c, primary.id()).is_none())
        .expect("the host has faces covering all of plane 4")
}

fn png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([20, 90, 200, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn pic(rid: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline><wp:extent cx="508000" cy="508000"/><wp:docPr id="1" name="P"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="p"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rid}"/></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="508000" cy="508000"/></a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

/// A DOCX whose header holds a picture of `header_media` (`word/media/image2.bin`).
fn docx_with_header_picture(header_media: &[u8]) -> Vec<u8> {
    let doc = format!(
        r#"<w:document {NS}><w:body><w:p><w:r><w:t>text</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr></w:body></w:document>"#
    );
    let drels = format!(r#"<Relationships xmlns="{RELS_NS}"><Relationship Id="rIdH" Type="{REL}/header" Target="header2.xml"/></Relationships>"#);
    let hrels = format!(r#"<Relationships xmlns="{RELS_NS}"><Relationship Id="rIdI" Type="{REL}/image" Target="media/image2.bin"/></Relationships>"#);
    let hdr = format!(r#"<w:hdr {NS}><w:p>{}</w:p></w:hdr>"#, pic("rIdI"));
    package(
        &doc,
        &drels,
        &[("word/header2.xml", hdr.as_bytes()), ("word/_rels/header2.xml.rels", hrels.as_bytes()), ("word/media/image2.bin", header_media)],
    )
}

/// A DOCX whose body picture points outside the file, at `target`.
fn docx_with_external_picture(target: &str) -> Vec<u8> {
    let doc = format!(r#"<w:document {NS}><w:body><w:p>{}<w:r><w:t>text</w:t></w:r></w:p></w:body></w:document>"#, pic("rIdX"));
    let drels = format!(
        r#"<Relationships xmlns="{RELS_NS}"><Relationship Id="rIdX" Type="{REL}/image" Target="{target}" TargetMode="External"/></Relationships>"#
    );
    package(&doc, &drels, &[])
}

fn package(document: &str, document_rels: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let rels = format!(
        r#"<Relationships xmlns="{RELS_NS}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="bin" ContentType="application/octet-stream"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", document.as_bytes()),
        ("word/_rels/document.xml.rels", document_rels.as_bytes()),
    ];
    parts.extend_from_slice(extra);
    for (name, bytes) in parts {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn multilingual_names_pass_strict() {
    if without_fonts() {
        return;
    }
    let names = "Łukasz Żmijewski\nGheorghe Ștefănescu\nKrejčí\nŞahın Gül\nLlŷr Grŵn\n王小明\n김민수\nสมชาย ใจดี\nराम शर्मा";
    let d = dir("names");
    let input = json_input(&d, names);
    for out in ["names.pdf", "names.png"] {
        let out = d.join(out);
        let r = convert(&input, &out, true);
        assert_eq!(r.status.code(), Some(0), "{}", stderr(&r));
        assert!(out.exists());
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn an_uncovered_code_point_fails_strict_with_a_report_and_no_file() {
    let c = uncovered();
    let cp = format!("U+{:04X}", c as u32);
    let d = dir("uncovered");
    let input = json_input(&d, &format!("a{c}b{c}{c}"));
    for out in ["out.pdf", "out.png"] {
        let out = d.join(out);
        let r = convert(&input, &out, true);
        assert_eq!(r.status.code(), Some(3), "{}", stderr(&r));
        let text = stderr(&r);
        assert!(text.contains(&format!("wordcraft-cli: strict: 1 character cannot be drawn: {cp} (×3)")), "{text}");
        let json = report_json(&r);
        assert_eq!(json["schema"], 1);
        assert_eq!(json["strict"], "failed");
        assert_eq!(json["missing_glyphs"][0]["cp"], cp.as_str());
        assert_eq!(json["missing_glyphs"][0]["count"], 3);
        assert!(!out.exists(), "{} was written", out.display());
        eprintln!("{text}");
        // Without --strict the same input converts, as before.
        let r = convert(&input, &out, false);
        assert_eq!(r.status.code(), Some(0), "{}", stderr(&r));
        assert!(out.exists());
        std::fs::remove_file(&out).unwrap();
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_real_png_in_a_header_passes_strict() {
    let d = dir("header-png");
    let input = d.join("in.docx");
    std::fs::write(&input, docx_with_header_picture(&png())).unwrap();
    for out in ["out.pdf", "out.png"] {
        let out = d.join(out);
        let r = convert(&input, &out, true);
        assert_eq!(r.status.code(), Some(0), "{}", stderr(&r));
        assert!(out.exists());
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_corrupt_header_picture_fails_strict_naming_the_part_and_the_header() {
    let d = dir("header-corrupt");
    let input = d.join("in.docx");
    std::fs::write(&input, docx_with_header_picture(b"corrupt bytes, not a picture")).unwrap();
    for out in ["out.pdf", "out.png"] {
        let out = d.join(out);
        let r = convert(&input, &out, true);
        assert_eq!(r.status.code(), Some(3), "{}", stderr(&r));
        let text = stderr(&r);
        assert!(text.contains("word/media/image2.bin (header2.xml): unsupported format"), "{text}");
        let json = report_json(&r);
        assert_eq!(json["strict"], "failed");
        assert_eq!(
            json["missing_pictures"][0],
            serde_json::json!({"part": "word/media/image2.bin", "in": "header2.xml", "reason": "unsupported format"})
        );
        assert!(!out.exists());
        eprintln!("{text}");
    }
    // Without --strict the grey box is drawn, as before.
    let out = d.join("lenient.pdf");
    assert_eq!(convert(&input, &out, false).status.code(), Some(0));
    assert!(out.exists());
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn an_external_picture_target_is_never_fetched_and_is_reported_under_strict() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let target = format!("http://{}/canary.png", listener.local_addr().unwrap());
    let d = dir("external");
    let input = d.join("in.docx");
    std::fs::write(&input, docx_with_external_picture(&target)).unwrap();

    let lenient = convert(&input, &d.join("lenient.pdf"), false);
    assert_eq!(lenient.status.code(), Some(0), "{}", stderr(&lenient));
    let out = d.join("strict.pdf");
    let strict = convert(&input, &out, true);
    assert_eq!(strict.status.code(), Some(3), "{}", stderr(&strict));
    let json = report_json(&strict);
    assert_eq!(json["missing_pictures"][0]["reason"], "external target");
    assert!(stderr(&strict).contains("external target"));
    assert!(!out.exists());

    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Ok((_, from)) => panic!("the converter connected to the external target from {from}"),
        Err(e) => panic!("{e}"),
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn bad_input_exits_1_with_the_usual_message_with_and_without_strict() {
    let d = dir("bad-input");
    let good = docx_with_header_picture(&png());
    let truncated = d.join("truncated.docx");
    std::fs::write(&truncated, &good[..good.len() / 2]).unwrap();
    let not_zip = d.join("not-zip.docx");
    std::fs::write(&not_zip, b"this is plainly not a zip archive").unwrap();
    for input in [&truncated, &not_zip] {
        let plain = convert(input, &d.join("a.pdf"), false);
        let strict = convert(input, &d.join("b.pdf"), true);
        assert_eq!(plain.status.code(), Some(1), "{}", stderr(&plain));
        assert_eq!(strict.status.code(), Some(1), "{}", stderr(&strict));
        assert_eq!(stderr(&plain), stderr(&strict));
        assert!(stderr(&plain).starts_with("wordcraft-cli: "), "{}", stderr(&plain));
        assert!(!d.join("a.pdf").exists() && !d.join("b.pdf").exists());
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn strict_with_another_output_type_exits_1_before_the_input_is_read() {
    let d = dir("wrong-output");
    let missing = d.join("does-not-exist.docx");
    for out in ["out.docx", "out.txt", "out.odt"] {
        let r = convert(&missing, &d.join(out), true);
        assert_eq!(r.status.code(), Some(1));
        let text = stderr(&r);
        assert!(text.contains("--strict needs a .pdf or .png output"), "{text}");
        assert!(!d.join(out).exists());
    }
    // Without --strict the same command fails on the missing input.
    let r = convert(&missing, &d.join("out.docx"), false);
    assert_eq!(r.status.code(), Some(1));
    assert!(!stderr(&r).contains("--strict"));
    std::fs::remove_dir_all(d).unwrap();
}

fn render(input: &Path, output: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).arg("render").arg(input).arg(output).args(args).output().unwrap()
}

/// Page 1 has `a{c}b`, page 2 plain text.
fn two_page_input(d: &Path, text: &str) -> PathBuf {
    let mut doc = Document::from_text(&format!("{text}\nSecond page"));
    doc.para_mut(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(1)).unwrap().props.page_break_before = Some(true);
    let path = d.join("two.json");
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    path
}

#[test]
fn render_strict_refuses_an_uncovered_character_on_the_page_with_no_stdout_or_file() {
    let c = uncovered();
    let cp = format!("U+{:04X}", c as u32);
    let d = dir("render-glyph");
    let input = two_page_input(&d, &format!("a{c}b"));
    let out = d.join("p1.png");
    let r = render(&input, &out, &["--strict"]);
    assert_eq!(r.status.code(), Some(3), "{}", stderr(&r));
    assert!(r.stdout.is_empty(), "{}", String::from_utf8_lossy(&r.stdout));
    assert!(!out.exists());
    let text = stderr(&r);
    assert!(text.contains(&format!("wordcraft-cli: strict: 1 character cannot be drawn: {cp} (×1)")), "{text}");
    assert_eq!(report_json(&r)["missing_glyphs"][0]["cp"], cp.as_str());
    eprintln!("{text}");

    // The character is not on page 2, and --scale does not matter to the check.
    let out2 = d.join("p2.png");
    let r = render(&input, &out2, &["--page", "2", "--strict", "--scale", "0.5"]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr(&r));
    assert!(out2.exists());
    assert!(String::from_utf8_lossy(&r.stdout).contains("\"path\""));
    // And the same page 1 without --strict renders, with the usual stdout.
    let out3 = d.join("lenient.png");
    let r = render(&input, &out3, &[]);
    assert_eq!(r.status.code(), Some(0), "{}", stderr(&r));
    assert!(out3.exists());
    let json: serde_json::Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(json["path"], out3.to_string_lossy().as_ref());
    assert!(json["width"].as_u64().is_some() && json["bytes"].as_u64().is_some());
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn render_strict_refuses_a_corrupt_header_picture_and_bad_input_still_exits_1() {
    let d = dir("render-pic");
    let input = d.join("in.docx");
    std::fs::write(&input, docx_with_header_picture(b"corrupt bytes, not a picture")).unwrap();
    let out = d.join("out.png");
    let r = render(&input, &out, &["--strict"]);
    assert_eq!(r.status.code(), Some(3), "{}", stderr(&r));
    assert!(r.stdout.is_empty());
    assert!(stderr(&r).contains("word/media/image2.bin (header2.xml)"), "{}", stderr(&r));
    assert!(!out.exists());

    let bad = d.join("bad.docx");
    std::fs::write(&bad, b"this is plainly not a zip archive").unwrap();
    let (plain, strict) = (render(&bad, &out, &[]), render(&bad, &out, &["--strict"]));
    assert_eq!((plain.status.code(), strict.status.code()), (Some(1), Some(1)));
    assert_eq!(stderr(&plain), stderr(&strict));
    std::fs::remove_dir_all(d).unwrap();
}
