//! Round trips through every format, malformed input and fuzzing.

use std::sync::Arc;

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Align, Block, CharProps, Document, Paragraph, Table, para_block};

use crate::model::{self, FBlock, Kind};
use crate::{export, import};

/// A small PNG made with the image crate.
fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(8, 4, |x, _| image::Rgba([(x * 30) as u8, 80, 160, 255]));
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

/// Headings, formatted runs, a link, nested lists, a table, a quote, code and a picture.
fn sample() -> Document {
    let mut d = Document::new();
    d.core.title = "Sample Title".into();
    d.core.creator = "Ada Writer".into();
    let mut blocks = Vec::new();
    blocks.push(para_block(Paragraph::with_text("Main Heading", CharProps::default()).styled("Heading1")));
    let mut p = Paragraph::with_text("Plain ", CharProps::default());
    let n = p.len();
    p.insert_text(n, "bold", &CharProps { bold: Some(true), ..Default::default() }).unwrap();
    let n = p.len();
    p.insert_text(n, " and ", &CharProps::default()).unwrap();
    let n = p.len();
    p.insert_text(n, "italic", &CharProps { italic: Some(true), ..Default::default() }).unwrap();
    let n = p.len();
    p.insert_text(n, " then ", &CharProps::default()).unwrap();
    let n = p.len();
    p.insert_text(n, "a link", &CharProps { link: Some("https://example.com/x".into()), ..Default::default() }).unwrap();
    let n = p.len();
    p.insert_text(n, ".", &CharProps::default()).unwrap();
    blocks.push(para_block(p));
    blocks.push(para_block(Paragraph::with_text("Second Level", CharProps::default()).styled("Heading2")));
    let bul = d.numbering.add_list(wordcraft_doc::ListKind::Bullet);
    let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    for (t, nm, lvl) in [("first bullet", bul, 0u8), ("second bullet", bul, 0), ("nested bullet", bul, 1)] {
        let mut q = Paragraph::with_text(t, CharProps::default()).styled("ListParagraph");
        q.props.numbering = Some(wordcraft_doc::props::NumRef { num: nm, level: lvl });
        blocks.push(para_block(q));
    }
    blocks.push(para_block(Paragraph::with_text("Between lists", CharProps::default())));
    for t in ["step one", "step two"] {
        let mut q = Paragraph::with_text(t, CharProps::default()).styled("ListParagraph");
        q.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
        blocks.push(para_block(q));
    }
    let mut t = Table::new(2, 2, 400.0);
    for (r, c, s) in [(0, 0, "Name"), (0, 1, "Value"), (1, 0, "alpha"), (1, 1, "42")] {
        t.rows[r].cells[c].blocks = vec![para_block(Paragraph::with_text(s, CharProps::default()))];
    }
    blocks.push(Arc::new(Block::Table(t)));
    blocks.push(para_block(Paragraph::with_text("A wise quote", CharProps::default()).styled("Quote")));
    let mut c = Paragraph::with_text("Centered text", CharProps::default());
    c.props.align = Some(Align::Center);
    blocks.push(para_block(c));
    let key = d.add_media(png(), "png");
    let mut ip = Paragraph::with_text("Picture: ", CharProps::default());
    let n = ip.len();
    ip.insert_object(
        n,
        InlineObject::Image { media: key, w: 60.0, h: 30.0, alt: "tiny".into(), float: Default::default(), crop: [0.0; 4] },
        &CharProps::default(),
    )
    .unwrap();
    blocks.push(para_block(ip));
    blocks.push(para_block(Paragraph::with_text("The end", CharProps::default())));
    d.body = blocks;
    d
}

fn flat(doc: &Document) -> Vec<FBlock> {
    model::from_doc(doc).blocks
}

fn para_texts(doc: &Document) -> Vec<String> {
    fn walk(b: &[FBlock], out: &mut Vec<String>) {
        for x in b {
            match x {
                FBlock::Para(p) => {
                    let t = p.text();
                    if !t.trim().is_empty() {
                        out.push(t.trim().to_string());
                    }
                }
                FBlock::Table(t) => {
                    for r in &t.rows {
                        for c in r {
                            walk(&c.blocks, out);
                        }
                    }
                }
            }
        }
    }
    let mut v = Vec::new();
    walk(&flat(doc), &mut v);
    v
}

fn find_para<'a>(b: &'a [FBlock], text: &str) -> Option<&'a model::Para> {
    b.iter().find_map(|x| match x {
        FBlock::Para(p) if p.text().trim() == text => Some(p),
        _ => None,
    })
}

fn check_round_trip(ext: &str, rich: bool) {
    let d = sample();
    let bytes = export(ext, &d).expect("handled").expect("export");
    let back = import(ext, &bytes).expect("handled").unwrap_or_else(|e| panic!("{ext}: {e}"));
    let texts = para_texts(&back);
    for want in [
        "Main Heading",
        "Second Level",
        "first bullet",
        "second bullet",
        "nested bullet",
        "step one",
        "step two",
        "Name",
        "Value",
        "alpha",
        "42",
        "A wise quote",
        "Centered text",
        "The end",
    ] {
        assert!(texts.iter().any(|t| if rich { t.ends_with(want) } else { t.contains(want) }), "{ext}: missing {want:?} in {texts:?}");
    }
    let joined = texts.join("\n");
    assert!(joined.contains("Plain bold and italic then a link."), "{ext}: {joined}");
    if !rich {
        return;
    }
    let b = flat(&back);
    assert_eq!(find_para(&b, "Main Heading").map(|p| p.kind), Some(Kind::Heading(1)), "{ext}");
    assert_eq!(find_para(&b, "Second Level").map(|p| p.kind), Some(Kind::Heading(2)), "{ext}");
    assert_eq!(find_para(&b, "A wise quote").map(|p| p.kind), Some(Kind::Quote), "{ext}");
    if ext != "md" {
        assert_eq!(find_para(&b, "Centered text").and_then(|p| p.align), Some(Align::Center), "{ext}");
    }
    let fb = find_para(&b, "first bullet").and_then(|p| p.list).expect("list");
    assert!(!fb.ordered && fb.level == 0, "{ext}");
    let nb = find_para(&b, "nested bullet").and_then(|p| p.list).expect("nested");
    assert_eq!(nb.level, 1, "{ext}");
    let s1 = find_para(&b, "step one").and_then(|p| p.list).expect("numbered");
    assert!(s1.ordered, "{ext}");
    assert!(find_para(&b, "Between lists").is_some_and(|p| p.list.is_none()), "{ext}");
    let body = find_para(&b, "Plain bold and italic then a link.").expect("para");
    let has = |t: &str, f: &dyn Fn(&model::Fmt) -> bool| body.inlines.iter().any(|i| matches!(i, model::Inline::Text(x, fm) if x == t && f(fm)));
    assert!(has("bold", &|f| f.bold && !f.italic), "{ext}: {:?}", body.inlines);
    assert!(has("italic", &|f| f.italic && !f.bold), "{ext}");
    assert!(has("a link", &|f| f.link.as_deref() == Some("https://example.com/x")), "{ext}: {:?}", body.inlines);
    let table = b.iter().find_map(|x| if let FBlock::Table(t) = x { Some(t) } else { None }).expect("table");
    assert_eq!(table.rows.len(), 2, "{ext}");
    assert_eq!(table.rows[1].iter().map(|c| c.text()).collect::<Vec<_>>(), vec!["alpha", "42"], "{ext}");
}

#[test]
fn markdown_round_trip() {
    check_round_trip("md", true);
}

#[test]
fn html_round_trip() {
    check_round_trip("html", true);
    let html = String::from_utf8(export("html", &sample()).unwrap().unwrap()).unwrap();
    assert!(html.contains("data:image/png;base64,"));
    let back = import("html", html.as_bytes()).unwrap().unwrap();
    assert_eq!(back.media.len(), 1, "picture survives");
    assert_eq!(back.core.title, "Sample Title");
}

#[test]
fn rtf_round_trip() {
    check_round_trip("rtf", true);
    let back = import("rtf", &export("rtf", &sample()).unwrap().unwrap()).unwrap().unwrap();
    assert_eq!(back.media.len(), 1, "picture survives");
    assert_eq!(back.core.title, "Sample Title");
    assert_eq!(back.core.creator, "Ada Writer");
}

#[test]
fn odt_round_trip() {
    check_round_trip("odt", true);
    let bytes = export("odt", &sample()).unwrap().unwrap();
    assert!(bytes.starts_with(b"PK"));
    assert_eq!(bytes.get(30..38), Some(&b"mimetype"[..]), "mimetype is the first entry");
    let back = import("odt", &bytes).unwrap().unwrap();
    assert_eq!(back.media.len(), 1, "picture survives");
    assert_eq!(back.core.title, "Sample Title");
    assert_eq!(back.core.creator, "Ada Writer");
}

#[test]
fn txt_round_trip() {
    check_round_trip("txt", false);
    let d = import("txt", "a\r\nb\nc".as_bytes()).unwrap().unwrap();
    assert_eq!(d.plain_text(wordcraft_doc::StoryRef::Body), "a\nb\nc");
    // UTF-16 LE with BOM, UTF-16 without BOM, Windows-1252.
    let mut u16le = vec![0xFF, 0xFE];
    for u in "héllo".encode_utf16() {
        u16le.extend(u.to_le_bytes());
    }
    assert_eq!(import("txt", &u16le).unwrap().unwrap().plain_text(Default::default()), "héllo");
    let raw: Vec<u8> = "hello world".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
    assert_eq!(import("txt", &raw).unwrap().unwrap().plain_text(Default::default()), "hello world");
    assert_eq!(import("txt", &[0x63, 0x61, 0x66, 0xE9, 0x80]).unwrap().unwrap().plain_text(Default::default()), "café€");
}

#[test]
fn dispatch_unknown() {
    assert!(import("xyz", b"").is_none());
    assert!(export("docx", &Document::new()).is_none());
    assert!(import(".MD", b"# x").is_some());
}

#[test]
fn rtf_merged_cells_and_lists() {
    let mut d = Document::new();
    let mut t = Table::new(3, 2, 300.0);
    t.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("tall", CharProps::default()))];
    t.rows[0].cells[0].props.vmerge = wordcraft_doc::props::VMerge::Restart;
    t.rows[1].cells[0].props.vmerge = wordcraft_doc::props::VMerge::Continue;
    d.body = vec![Arc::new(Block::Table(t))];
    d.ensure_nonempty();
    for ext in ["rtf", "html", "odt"] {
        let back = import(ext, &export(ext, &d).unwrap().unwrap()).unwrap().unwrap();
        let Some(Block::Table(bt)) = back.body.first().map(|b| &**b) else { panic!("{ext}: no table") };
        assert_eq!(bt.rows.len(), 3, "{ext}");
        assert_eq!(bt.rows[0].cells[0].props.vmerge, wordcraft_doc::props::VMerge::Restart, "{ext}");
        assert_eq!(bt.rows[1].cells[0].props.vmerge, wordcraft_doc::props::VMerge::Continue, "{ext}");
    }
}

#[test]
fn markdown_text_shapes() {
    let md = String::from_utf8(export("md", &sample()).unwrap().unwrap()).unwrap();
    assert!(md.contains("# Main Heading"));
    assert!(md.contains("**bold**") && md.contains("*italic*"));
    assert!(md.contains("[a link](https://example.com/x)"));
    assert!(md.contains("- first bullet") && md.contains("    - nested bullet") && md.contains("1. step one"));
    assert!(md.contains("| Name | Value |"));
    assert!(md.contains("> A wise quote"));
    assert!(md.contains("![tiny]("));
}

#[test]
fn code_blocks_and_rules() {
    let d = import("md", b"```\nfn main() {}\n  indented\n```\n\n***\n\nafter").unwrap().unwrap();
    let b = flat(&d);
    assert!(matches!(&b[0], FBlock::Para(p) if p.kind == Kind::Code && p.text() == "fn main() {}"));
    assert!(matches!(&b[1], FBlock::Para(p) if p.kind == Kind::Code && p.text() == "  indented"));
    assert!(matches!(&b[2], FBlock::Para(p) if p.kind == Kind::Rule));
    for ext in ["md", "html", "rtf", "odt"] {
        let back = import(ext, &export(ext, &d).unwrap().unwrap()).unwrap().unwrap();
        let bb = flat(&back);
        assert!(bb.iter().any(|x| matches!(x, FBlock::Para(p) if p.kind == Kind::Code && p.text() == "  indented")), "{ext}: {bb:?}");
        assert!(bb.iter().any(|x| matches!(x, FBlock::Para(p) if p.kind == Kind::Rule)), "{ext}: {bb:?}");
    }
}

#[test]
fn deep_nesting_is_bounded() {
    let html = "<div><table><tr><td>".repeat(5000) + "x";
    let d = import("html", html.as_bytes()).unwrap().unwrap();
    assert!(d.plain_text(Default::default()).contains('x'));
    let rtf = format!("{{\\rtf1 {}x{}}}", "{\\b ".repeat(100_000), "}".repeat(100_000));
    let d = import("rtf", rtf.as_bytes()).unwrap().unwrap();
    assert!(d.plain_text(Default::default()).contains('x'));
    let md = ">".repeat(10_000) + " deep";
    assert!(import("md", md.as_bytes()).unwrap().is_ok());
    let md = (0..200).map(|i| format!("{}- item", "  ".repeat(i))).collect::<Vec<_>>().join("\n");
    assert!(import("md", md.as_bytes()).unwrap().is_ok());
    let md = "[".repeat(5000) + &"](x)".repeat(5000);
    assert!(import("md", md.as_bytes()).unwrap().is_ok());
    let md = "*a ".repeat(20_000);
    assert!(import("md", md.as_bytes()).unwrap().is_ok());
}

#[test]
fn garbage_never_panics() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    for len in [0usize, 1, 7, 64, 513, 4096] {
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        for ext in ["txt", "md", "html", "rtf", "odt"] {
            let _ = import(ext, &bytes);
        }
        let mut rtf = b"{\\rtf1".to_vec();
        rtf.extend(&bytes);
        let _ = import("rtf", &rtf);
        let mut odt = b"PK\x03\x04".to_vec();
        odt.extend(&bytes);
        let _ = import("odt", &odt);
    }
    // A zip that is not an ODF text document.
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.start_file("mimetype", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut z, b"application/zip").unwrap();
    let bytes = z.finish().unwrap().into_inner();
    assert!(import("odt", &bytes).unwrap().is_err());
}

#[test]
fn hostile_rtf_bits() {
    for s in [
        &b"{\\rtf1 \\u-1? \\u99999999999? \\'zz \\bin999999999 }"[..],
        b"{\\rtf1{\\pict\\pngblip 89504e47zz}}",
        b"{\\rtf1\\trowd\\cellx-5 \\intbl a\\cell\\row\\row\\cell}",
        b"{\\rtf1 }}}}}}{{{{",
        b"{\\rtf1\\fs-99999 \\cf999 \\f-3 x\\par}",
    ] {
        let _ = import("rtf", s).unwrap();
    }
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig { cases: 128, ..Default::default() })]

    #[test]
    fn fuzz_markdown(s in "[ -~\\n\\t*_`#>|\\[\\]()!~-]{0,300}") {
        let d = import("md", s.as_bytes()).unwrap().unwrap();
        let _ = export("md", &d).unwrap().unwrap();
    }

    #[test]
    fn fuzz_html(s in "(<(/)?(p|b|i|ul|ol|li|table|tr|td|th|pre|blockquote|a href=x|img src=data:,|br|h2|span style='color:red')>|[a-z &;#<>\"'=]){0,120}") {
        let d = import("html", s.as_bytes()).unwrap().unwrap();
        let _ = export("html", &d).unwrap().unwrap();
    }

    #[test]
    fn fuzz_rtf(s in "(\\{|\\}|\\\\[a-z]{1,8}-?[0-9]{0,4} ?|\\\\'[0-9a-f]{2}|\\\\\\*|[a-z ;]){0,150}") {
        let src = format!("{{\\rtf1 {s}");
        let d = import("rtf", src.as_bytes()).unwrap().unwrap();
        let _ = export("rtf", &d).unwrap().unwrap();
    }

    #[test]
    fn markdown_text_round_trips(words in proptest::collection::vec("[a-zA-Z0-9*_`\\[\\]<>#.!-]{1,8}", 1..8)) {
        let text = words.join(" ");
        let d = Document::from_text(&text);
        let md = export("md", &d).unwrap().unwrap();
        let back = import("md", &md).unwrap().unwrap();
        let got = back.plain_text(Default::default());
        proptest::prop_assert_eq!(got.trim(), text.trim(), "md: {}", String::from_utf8_lossy(&md));
    }
}

#[test]
fn html_img_sources_go_through_the_loader() {
    // Issue #98: an `img` named by a relative path was dropped, leaving only its alt text.
    let html = r#"<p><img src="logo.png" alt="Sample Logo" width="120" height="40"></p><p><img src="gone.png" alt="Missing"></p><p>Text after.</p>"#;
    let pic = png();
    let seen = std::cell::RefCell::new(Vec::new());
    let d = crate::html::import_with(html, &|src| {
        seen.borrow_mut().push(src.to_string());
        (src == "logo.png").then(|| pic.clone())
    });
    assert_eq!(*seen.borrow(), ["logo.png", "gone.png"]);
    assert_eq!(d.media.len(), 1);
    let text = d.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(text.contains("Missing") && !text.contains("Sample Logo"), "{text:?}");
    // Without a loader only data: URIs load.
    assert!(import("html", html.as_bytes()).unwrap().unwrap().media.is_empty());
}
