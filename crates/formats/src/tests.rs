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
        InlineObject::Image { media: key, w: 60.0, h: 30.0, alt: "tiny".into(), float: Default::default(), crop: [0.0; 4], ole: None },
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

/// Two bullets directly followed by two numbered items.
fn bullets_then_numbers() -> Document {
    let mut d = Document::new();
    let bul = d.numbering.add_list(wordcraft_doc::ListKind::Bullet);
    let num = d.numbering.add_list(wordcraft_doc::ListKind::Numbered);
    let mut blocks = Vec::new();
    for (t, nm) in [("dot one", bul), ("dot two", bul), ("num one", num), ("num two", num)] {
        let mut q = Paragraph::with_text(t, CharProps::default()).styled("ListParagraph");
        q.props.numbering = Some(wordcraft_doc::props::NumRef { num: nm, level: 0 });
        blocks.push(para_block(q));
    }
    d.body = blocks;
    d
}

#[test]
fn numbered_list_after_bullets_keeps_its_kind() {
    let d = bullets_then_numbers();
    for ext in ["odt", "rtf"] {
        let bytes = export(ext, &d).expect("handled").expect("export");
        let back = import(ext, &bytes).expect("handled").expect("import");
        let b = flat(&back);
        let kind = |t: &str| find_para(&b, t).and_then(|p| p.list).map(|l| l.ordered);
        assert_eq!(kind("dot two"), Some(false), "{ext}");
        assert_eq!(kind("num one"), Some(true), "{ext}");
        assert_eq!(kind("num two"), Some(true), "{ext}");
    }
    let md = String::from_utf8(export("md", &d).expect("handled").expect("export")).unwrap();
    assert!(md.contains("1. num one") && md.contains("2. num two"), "{md}");
}

#[test]
fn markdown_list_items_that_start_like_blocks_keep_their_text() {
    for text in ["---", "--", "1. Intro", "# Agenda", "ordinary item"] {
        let mut d = Document::new();
        let bul = d.numbering.add_list(wordcraft_doc::ListKind::Bullet);
        let mut p = Paragraph::with_text(text, CharProps::default()).styled("ListParagraph");
        p.props.numbering = Some(wordcraft_doc::props::NumRef { num: bul, level: 0 });
        d.body = vec![para_block(p)];
        let bytes = export("md", &d).unwrap().unwrap();
        let blocks = flat(&import("md", &bytes).unwrap().unwrap());
        let [FBlock::Para(p)] = blocks.as_slice() else { panic!("{text}: {}", String::from_utf8_lossy(&bytes)) };
        assert_eq!((p.text().as_str(), p.kind, p.list), (text, Kind::Normal, Some(model::ListInfo { ordered: false, level: 0, start: 1 })));
    }
}

#[test]
fn markdown_round_trip() {
    check_round_trip("md", true);
}

/// A code line of backticks must not be mistaken for the closing fence when the file comes
/// back: the writer's fence has to be one backtick longer than any content line.
#[test]
fn markdown_code_fence_outgrows_its_content() {
    let line = "`".repeat(100);
    let doc = import("md", format!("    {line}\n").as_bytes()).unwrap().unwrap();
    assert_eq!(doc.plain_text(Default::default()).trim(), line);
    let out = export("md", &doc).unwrap().unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains(&"`".repeat(101)), "the fence is longer than the content: {text}");
    let back = import("md", text.as_bytes()).unwrap().unwrap();
    assert_eq!(back.plain_text(Default::default()).trim(), line, "the code line survives a round trip");
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
fn html_ruby_round_trip() {
    // #1681: a phonetic guide exports as `<ruby>` and opens back as one ruby object, its base
    // text still in the paragraph.
    let props = CharProps::default();
    let mut p = Paragraph::with_text("I read ", props.clone());
    let n = p.len();
    p.insert_object(n, model::ruby_object("漢字".into(), "かんじ".into(), 20.0), &props).unwrap();
    let n = p.len();
    p.insert_text(n, " daily.", &props).unwrap();
    let mut d = Document::new();
    d.body = vec![para_block(p)];

    let html = String::from_utf8(export("html", &d).unwrap().unwrap()).unwrap();
    assert!(html.contains("<ruby>漢字<rt>かんじ</rt></ruby>"), "{html}");
    let back = import("html", html.as_bytes()).unwrap().unwrap();
    assert_eq!(back.plain_text(Default::default()), "I read 漢字 daily.");
    let ruby = back.body.iter().find_map(|b| match &**b {
        Block::Para(p) => p.objects.iter().find_map(|o| match o {
            InlineObject::Ruby { base, ruby, .. } => Some((base.as_str(), ruby.as_str())),
            _ => None,
        }),
        _ => None,
    });
    assert_eq!(ruby, Some(("漢字", "かんじ")));
}

#[test]
fn html_ruby_at_the_paragraph_start_keeps_a_leading_space() {
    // #1681: the ruby's base text goes through the text escaper with the paragraph-start state
    // taken before its formatting tags, so a leading space becomes `&nbsp;` instead of a space
    // HTML may collapse.
    let props = CharProps { bold: Some(true), ..Default::default() };
    let mut p = Paragraph::with_text("", CharProps::default());
    let n = p.len();
    p.insert_object(n, model::ruby_object(" 漢字".into(), "かんじ".into(), 20.0), &props).unwrap();
    let mut d = Document::new();
    d.body = vec![para_block(p)];

    let html = String::from_utf8(export("html", &d).unwrap().unwrap()).unwrap();
    assert!(html.contains("<strong><ruby>&nbsp;漢字<rt>かんじ</rt></ruby></strong>"), "{html}");
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
fn latex_round_trip() {
    check_round_trip("tex", true);
    let tex = String::from_utf8(export("tex", &sample()).unwrap().unwrap()).unwrap();
    assert!(tex.starts_with("% Written by WordCraft") && tex.contains("\\documentclass{article}"), "{tex}");
    assert!(tex.contains("\\section*{Main Heading}") && tex.contains("\\subsection*{Second Level}"), "{tex}");
    assert!(tex.contains("\\textbf{bold}") && tex.contains("\\textit{italic}"), "{tex}");
    assert!(tex.contains("\\href{https://example.com/x}{a link}"), "{tex}");
    assert!(tex.contains("\\begin{itemize}") && tex.contains("\\begin{enumerate}") && tex.contains("\\begin{quote}"), "{tex}");
    assert!(tex.contains("\\begin{tabular}") && tex.contains("\\begin{center}"), "{tex}");
    assert!(tex.contains("\\textit{[Picture: tiny]}"), "pictures leave their alternative text: {tex}");
    assert!(tex.contains("pdftitle={Sample Title}") && tex.contains("pdfauthor={Ada Writer}"), "{tex}");
    assert!(tex.trim_end().ends_with("\\end{document}"));
    // Exported LaTeX parses back to the same metadata.
    let back = import("tex", tex.as_bytes()).unwrap().unwrap();
    assert_eq!(back.core.title, "", "pdftitle isn't \\title");
    for ext in ["tex", "latex", "ltx", ".TEX"] {
        assert!(import(ext, b"x").is_some() && export(ext, &Document::new()).is_some(), "{ext}");
    }
}

#[test]
fn page_break_char_splits_paragraph() {
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("num two\u{c}Page two", CharProps::default()))];
    let b = flat(&d);
    assert!(matches!(&b[..], [FBlock::Para(a), FBlock::Para(c)] if a.text() == "num two" && !a.page_break && c.text() == "Page two" && c.page_break));
    let txt = String::from_utf8(export("txt", &d).unwrap().unwrap()).unwrap();
    assert!(!txt.contains("twoPage"), "{txt:?}");
    let odt = import("odt", &export("odt", &d).unwrap().unwrap()).unwrap().unwrap();
    let b = flat(&odt);
    assert!(b.iter().any(|x| matches!(x, FBlock::Para(p) if p.text().trim() == "Page two" && p.page_break)));
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
fn txt_export_writes_symbol_font_text_as_unicode() {
    let mut d = Document::from_text("Done J\u{F0FC}");
    d.format_range(&wordcraft_doc::Pos::body(0, 5), &wordcraft_doc::Pos::body(0, 9), &|c| c.font = Some("Wingdings".into())).unwrap();
    let out = export("txt", &d).unwrap().unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "Done \u{263A}\u{2713}");
}

/// #1080: `txt` export (and so `wordcraft-cli text`) keeps list labels, so a list item reads
/// apart from an ordinary paragraph.
#[test]
fn txt_export_keeps_list_labels() {
    let mut d = Document::new();
    let bul = d.numbering.add_list(wordcraft_doc::ListKind::Bullet);
    let mut p = Paragraph::with_text("alpha", CharProps::default());
    p.props.numbering = Some(wordcraft_doc::props::NumRef { num: bul, level: 0 });
    d.body = vec![para_block(p), para_block(Paragraph::with_text("plain", CharProps::default()))];
    let out = String::from_utf8(export("txt", &d).unwrap().unwrap()).unwrap();
    let mut lines = out.split("\r\n");
    assert!(lines.next().is_some_and(|l| l.contains("alpha") && !l.starts_with("alpha")), "{out:?}");
    assert_eq!(lines.next(), Some("plain"));
}

/// #569: a file with an extension we don't know opens as text when it is text (a playlist), and
/// a binary file is told apart.
#[test]
fn text_sniffing() {
    use crate::txt::is_text;
    assert!(is_text(b""));
    assert!(is_text(b"#EXTM3U\r\n#EXTINF:123,Artist - Title\r\nC:\\Music\\song.mp3\r\n"));
    assert!(is_text("#EXTM3U\n#EXTINF:-1,Мелодия — ünïcødé\nhttp://x/y.mp3\n".as_bytes()));
    assert!(is_text(&[0x23, 0x45, 0x58, 0x54, 0xE9, 0x0D, 0x0A, 0x1A])); // Windows-1252, DOS ^Z
    assert!(is_text(&[0xEF, 0xBB, 0xBF, b'a']));
    let u16le: Vec<u8> = "#EXTM3U\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    assert!(is_text(&u16le));
    assert!(!is_text(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
    assert!(!is_text(b"PK\x03\x04\x14\0\x06\0"));
    assert!(!is_text(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n"));
    assert!(!is_text(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 3, 4, 5, 6, 7, 8]));
    // Odd-length bytes with zeros mixed in: binary, not UTF-16.
    assert!(!is_text(&[1, 0, 2, 0, 3]));
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
    for ext in ["md", "html", "rtf", "odt", "tex"] {
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
    let tex = "{\\textbf{".repeat(50_000) + "x";
    assert!(import("tex", tex.as_bytes()).unwrap().unwrap().plain_text(Default::default()).contains('x'));
    let tex = "\\begin{itemize}\\item ".repeat(10_000) + "deep";
    assert!(import("tex", tex.as_bytes()).unwrap().unwrap().plain_text(Default::default()).contains("deep"));
    let tex = "\\begin{tabular}{l}".repeat(2_000) + "cell";
    assert!(import("tex", tex.as_bytes()).unwrap().is_ok());
    let tex = format!("${}x{}$", "\\frac{".repeat(10_000), "}".repeat(10_000));
    assert!(import("tex", tex.as_bytes()).unwrap().is_ok());
    // `*{n}{…}` column specs: nested 100k deep (the repeat recursed without a limit and overflowed
    // the stack), and repeats multiplying past what a table holds (64^30 columns).
    let tex = format!("\\begin{{tabular}}{{{}l{}}}x\\end{{tabular}}", "*{1}{".repeat(100_000), "}".repeat(100_000));
    assert!(import("tex", tex.as_bytes()).unwrap().unwrap().plain_text(Default::default()).contains('x'));
    let tex = format!("\\begin{{tabular}}{{{}p{{1cm}}{}}}x\\end{{tabular}}", "*{64}{".repeat(30), "}".repeat(30));
    assert!(import("tex", tex.as_bytes()).unwrap().unwrap().plain_text(Default::default()).contains('x'));
    // An equation of unmatched `(`: finding each group rescanned to the end of the input, so
    // exporting it was quadratic (50k took seconds; 200k would take minutes).
    let started = std::time::Instant::now();
    let latex = crate::latex::linear_to_latex(&"(".repeat(200_000));
    assert_eq!(latex.len(), 200_000);
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    // Brackets that do match, nested 100k deep: each group was copied out before looking for
    // the `/` of a fraction, which was quadratic as well.
    let started = std::time::Instant::now();
    let nested = format!("{}x{}", "(".repeat(100_000), ")".repeat(100_000));
    assert_eq!(crate::latex::linear_to_latex(&nested).len(), 200_001);
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    assert_eq!(crate::latex::linear_to_latex("(a)/(b)"), "\\frac{a}{b}");
    assert_eq!(crate::latex::linear_to_latex("((a)/(b))/(c)"), "\\frac{\\frac{a}{b}}{c}");
    // `\begin{` never closed: the name was looked for through the whole rest of the file.
    let started = std::time::Instant::now();
    let tex = "\\begin{".repeat(300_000);
    assert!(import("tex", tex.as_bytes()).unwrap().is_ok());
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}

/// What a document says in an equation or a bookmark name is not written to the exported file as
/// TeX commands: compiling the export must not read files or run programs the document names.
#[test]
fn latex_export_carries_no_commands_from_the_document() {
    let tex = crate::latex::linear_to_latex("\\input secret.txt \\immediate\\write18\\bgroup id\\egroup \\csname x\\endcsname");
    for cmd in ["\\input", "\\immediate", "\\write", "\\bgroup", "\\egroup", "\\csname", "\\endcsname"] {
        assert!(!tex.contains(cmd), "{cmd} in {tex}");
    }
    assert!(tex.contains("\\backslash input"), "{tex}");
    // `^^5c` is how TeX spells a backslash: two carets never come out side by side.
    for linear in ["a^^5cinput secret.txt ", "a^^^^5cinput", "x^(^^5cinput)", "^^5cinput"] {
        assert!(!crate::latex::linear_to_latex(linear).contains("^^"), "{linear}");
    }
    let deep = format!("{}^^5cinput x{}", "x^(".repeat(40), ")".repeat(40));
    assert!(!crate::latex::linear_to_latex(&deep).contains("^^"));
    // Symbols, functions and font switches are still commands.
    assert_eq!(crate::latex::linear_to_latex("\\alpha+\\sin x+\\mathbb R"), "\\alpha+\\sin x+\\mathbb R");
    assert_eq!(crate::latex::linear_to_latex("x^2^3"), "x^2^3");
    // The same through a whole file, and for a bookmark name.
    let d = crate::latex::import("Text $\\input secret.txt $ and \\hypertarget{^^5cinput secret.txt ^^5c}{here}.".as_bytes());
    let out = String::from_utf8(export("tex", &d).unwrap().unwrap()).unwrap();
    assert!(!out.contains("\\input") && !out.contains("^^"), "{out}");
}

/// Many labels in a row, with or without spaces between, are read in linear time (looking back
/// past them for the last text made a megabyte of labels take tens of seconds), and labels
/// separated by text are all kept.
#[test]
fn latex_labels_in_a_row_are_read_in_linear_time() {
    let started = std::time::Instant::now();
    for src in ["\\label{a} ".repeat(100_000) + "x", "\\label{a}".repeat(100_000) + "x", "\\hypertarget{a}{} ".repeat(100_000)] {
        let d = crate::latex::import(src.as_bytes());
        assert!(d.body.len() <= 2);
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    let d = crate::latex::import("\\label{one}First \\label{two}second \\label{three}third.".as_bytes());
    let p = d.para_at(&wordcraft_doc::Pos::body(0, 0)).unwrap();
    let names: Vec<&str> = p
        .objects
        .iter()
        .filter_map(|o| if let wordcraft_doc::InlineObject::BookmarkStart { name } = o { Some(name.as_str()) } else { None })
        .collect();
    assert_eq!(names, ["one", "two", "three"]);
    assert_eq!(d.plain_text(wordcraft_doc::StoryRef::Body).replace('\u{FFFC}', ""), "First second third.");
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
        for ext in ["txt", "md", "html", "rtf", "odt", "tex"] {
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
    // #1653: a flood of multibyte code-page bytes (leads and trails, raw and escaped) decodes as a
    // stream and never panics or grows a buffer with the input.
    let mut rtf = b"{\\rtf1\\ansi\\ansicpg932 ".to_vec();
    rtf.resize(rtf.len() + 200_000, 0x93);
    rtf.extend_from_slice(b"\\'fa\\par\\'93");
    assert!(import("rtf", &rtf).unwrap().is_ok());
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
    fn fuzz_latex(s in "(\\\\(begin|end)\\{(itemize|tabular|verbatim|quote|center|x)\\}|\\\\[a-zA-Z]{1,8}\\*?|\\\\[^a-zA-Z]|[{}\\[\\]$&%~^_#]|\n\n|[a-z ]){0,150}") {
        let d = import("tex", s.as_bytes()).unwrap().unwrap();
        let _ = export("tex", &d).unwrap().unwrap();
    }

    #[test]
    fn latex_text_round_trips(words in proptest::collection::vec("[a-zA-Z0-9{}$&%#_~^\\\\<>|`'.!-]{1,8}", 1..8)) {
        let text = words.join(" ");
        let d = Document::from_text(&text);
        let tex = export("tex", &d).unwrap().unwrap();
        let back = import("tex", &tex).unwrap().unwrap();
        let got = back.plain_text(Default::default());
        proptest::prop_assert_eq!(got.trim(), text.trim(), "tex: {}", String::from_utf8_lossy(&tex));
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
        (src == "logo.png").then(|| std::sync::Arc::new(pic.clone()))
    });
    assert_eq!(*seen.borrow(), ["logo.png", "gone.png"]);
    assert_eq!(d.media.len(), 1);
    let text = d.plain_text(wordcraft_doc::StoryRef::Body);
    assert!(text.contains("Missing") && !text.contains("Sample Logo"), "{text:?}");
    // Without a loader only data: URIs load.
    assert!(import("html", html.as_bytes()).unwrap().unwrap().media.is_empty());
}

#[test]
fn chart_alt_text_is_an_image_alt_never_body_text() {
    let mut p = Paragraph::with_text("See below", CharProps::default());
    let chart = InlineObject::Graphic { w: 100.0, h: 50.0, alt: "Sales".into(), float: Default::default(), graphic: Arc::new(Default::default()) };
    p.insert_object(4, chart, &CharProps::default()).unwrap();
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    let text = |ext: &str| String::from_utf8(export(ext, &d).unwrap().unwrap()).unwrap();
    assert!(text("html").contains(r#"<img alt="Sales">"#), "{}", text("html"));
    assert!(text("md").contains("![Sales]()"), "{}", text("md"));
    for ext in ["txt", "rtf", "tex"] {
        assert!(!text(ext).contains("Sales"), "{ext}: {}", text(ext));
    }
    assert_eq!(para_texts(&d), ["See below"]);
}

/// A picture marked decorative exports with an empty alt text, though it keeps its description,
/// so screen readers skip it; the same picture not marked decorative keeps its alt text.
#[test]
fn decorative_picture_exports_with_empty_alt_text() {
    use wordcraft_doc::para::{AltText, Float};
    let doc = |decorative: bool| {
        let mut d = Document::new();
        let media = d.add_media(png(), "png");
        let float = Float { alt: AltText { decorative, ..Default::default() }, ..Default::default() };
        let pic = InlineObject::Image { media, w: 40.0, h: 20.0, alt: "Blue bar".into(), float, crop: [0.0; 4], ole: None };
        let mut p = Paragraph::with_text("Logo: ", CharProps::default());
        let n = p.len();
        p.insert_object(n, pic, &CharProps::default()).unwrap();
        d.body = vec![para_block(p)];
        d
    };
    let text = |d: &Document, ext: &str| String::from_utf8(export(ext, d).unwrap().unwrap()).unwrap();
    let (plain, decorative) = (doc(false), doc(true));
    assert!(text(&plain, "html").contains(r#"alt="Blue bar""#), "{}", text(&plain, "html"));
    assert!(text(&plain, "md").contains("![Blue bar]("), "{}", text(&plain, "md"));
    let (html, md) = (text(&decorative, "html"), text(&decorative, "md"));
    assert!(html.contains(r#"alt="""#) && !html.contains("Blue bar"), "{html}");
    assert!(md.contains("![](") && !md.contains("Blue bar"), "{md}");
}

/// Hostile Markdown and HTML that rescanned the rest of the text from every `[`, `![`, `<` or `&`
/// took quadratic time: a few hundred KB froze the importer for tens of seconds.
#[test]
fn hostile_importer_input_takes_linear_time() {
    use std::time::{Duration, Instant};
    let cases: [(&str, String); 5] = [
        ("md", "*[".repeat(50_000)),
        ("md", "![a](b".repeat(100_000)),
        ("md", "[a](".repeat(100_000)),
        ("md", "<a".repeat(200_000)),
        ("html", "&#".repeat(500_000)),
    ];
    for (ext, text) in cases {
        let t = Instant::now();
        let d = import(ext, text.as_bytes()).unwrap().unwrap();
        let _ = d.plain_text(Default::default());
        assert!(t.elapsed() < Duration::from_secs(5), "{ext} {:?}... took {:?}", &text[..8], t.elapsed());
    }
}

/// Backtick runs that never close, each a different length, sent every code span and every link
/// bracket scan over them looking for a closing run through the rest of the text: superlinear
/// time on hostile Markdown.
#[test]
fn unmatched_backtick_runs_take_linear_time() {
    use std::time::{Duration, Instant};
    // Runs of 1, 2, … 2000 backticks (2 MB), none of them closed.
    let runs = (1..=2000).map(|n| "`".repeat(n)).collect::<Vec<_>>().join(" ");
    let cases = [runs.clone(), format!("{}{runs}{}", "[".repeat(8), "]".repeat(8)), format!("[{runs}")];
    for text in cases {
        let t = Instant::now();
        let d = import("md", text.as_bytes()).unwrap().unwrap();
        let plain = d.plain_text(Default::default());
        let elapsed = t.elapsed();
        eprintln!("{:?}... took {elapsed:?}", &text[..4]);
        assert!(elapsed < Duration::from_secs(5), "{:?}... took {elapsed:?}", &text[..4]);
        assert_eq!(plain.matches('`').count(), text.matches('`').count(), "every backtick stays text");
    }
}

/// Code spans and brackets around backticks read as before the backtick runs were indexed.
#[test]
fn code_spans_and_links_around_backticks_read_as_before() {
    // An escaped backtick before a span, a span holding a shorter run, a run nothing closes, a `]`
    // inside a code span in link text, and link text with a backtick that nothing closes.
    let flow = crate::markdown::parse("\\``e` `` a ` b `` then ``` and `c` [x `]` y](http://a.org) [p ` q](http://b.org) ``f");
    let FBlock::Para(p) = &flow.blocks[0] else { panic!("{:?}", flow.blocks) };
    let spans: Vec<(String, bool, Option<String>)> = p
        .inlines
        .iter()
        .filter_map(|i| if let model::Inline::Text(t, f) = i { Some((t.clone(), f.code, f.link.as_deref().map(str::to_string))) } else { None })
        .collect();
    let code: Vec<&str> = spans.iter().filter(|s| s.1).map(|s| s.0.as_str()).collect();
    assert_eq!(code, ["e", "a ` b", "c", "]"], "{spans:?}");
    let linked = |url: &str| spans.iter().filter(|s| s.2.as_deref() == Some(url)).map(|s| s.0.as_str()).collect::<String>();
    assert_eq!(linked("http://a.org"), "x ] y");
    assert_eq!(linked("http://b.org"), "p ` q");
    assert_eq!(p.text(), "`e a ` b then ``` and c x ] y p ` q ``f");
}

/// An unescaped `&` in a LaTeX link address is part of the address.
#[test]
fn latex_href_keeps_ampersand() {
    let d = import("tex", br"\href{https://e.com/a?b=1&c=2}{link}").unwrap().unwrap();
    let links: Vec<Option<String>> = flat(&d)
        .iter()
        .filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None })
        .flat_map(|p| p.inlines.iter())
        .filter_map(|i| if let model::Inline::Text(_, f) = i { Some(f.link.as_deref().map(str::to_string)) } else { None })
        .collect();
    assert_eq!(links, [Some("https://e.com/a?b=1&c=2".to_string())]);

    let d = import("tex", br"\url{https://e.com/a?b=1&c=2}").unwrap().unwrap();
    let texts: Vec<(String, Option<String>)> = flat(&d)
        .iter()
        .filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None })
        .flat_map(|p| p.inlines.iter())
        .filter_map(|i| if let model::Inline::Text(t, f) = i { Some((t.to_string(), f.link.as_deref().map(str::to_string))) } else { None })
        .collect();
    let url = "https://e.com/a?b=1&c=2".to_string();
    assert_eq!(texts, [(url.clone(), Some(url))]);
}

/// A `[` nothing closes, bracketed text that is not a link and links themselves used up the bracket
/// scans' budget as if each had read the rest of the text: later links in the paragraph were plain
/// text.
#[test]
fn brackets_do_not_turn_later_links_into_text() {
    let links = |md: &str| {
        let flow = crate::markdown::parse(md);
        let FBlock::Para(p) = &flow.blocks[0] else { panic!("{:?}", flow.blocks) };
        let mut urls: Vec<String> =
            p.inlines.iter().filter_map(|i| if let model::Inline::Text(_, f) = i { f.link.as_deref().map(str::to_string) } else { None }).collect();
        urls.dedup();
        urls
    };
    // An interval nothing closes, before a link.
    assert_eq!(links("Intervals [0, 1) and [docs](http://x.org)"), ["http://x.org"]);
    // References early in a long paragraph, a link at its end.
    let refs = (1..=12).map(|n| format!("[{n}]")).collect::<Vec<_>>().join(" ");
    assert_eq!(links(&format!("{refs} {}[docs](http://x.org)", "word ".repeat(4000))), ["http://x.org"]);
    // Many links through one paragraph.
    let urls: Vec<String> = (0..60).map(|n| format!("http://x.org/{n}")).collect();
    assert_eq!(links(&urls.iter().map(|u| format!("[link]({u}) {}", "word ".repeat(20))).collect::<String>()), urls);
}

/// The bounded scans must not change how ordinary links, images, tags and entities read.
#[test]
fn bounded_scans_keep_ordinary_markdown_and_entities() {
    let md =
        import("md", b"See [the *docs*](https://example.com/a(b) and ![alt](x.png) & &amp; <b>bold</b> <https://example.com>.\n").unwrap().unwrap();
    let text = md.plain_text(Default::default());
    assert!(text.contains("the docs"), "{text}");
    assert!(text.contains("& &"), "{text}");
    let html = import("html", "<p>&lt;&amp;&#65;&eacute; &nosuchentity; &#x41;</p>".as_bytes()).unwrap().unwrap();
    assert_eq!(html.plain_text(Default::default()).trim(), "<&Aé &nosuchentity; A");
    // An entity name longer than the scan window is left alone, as before.
    let long = format!("<p>&{};</p>", "a".repeat(40));
    let t = import("html", long.as_bytes()).unwrap().unwrap().plain_text(Default::default());
    assert!(t.contains(&"a".repeat(40)), "{t}");
}

/// A document's hyperlink can be `javascript:` (WordCraft refuses to follow it); the exported
/// HTML and Markdown must not hand it to a browser either. Web, mail, bookmark and relative links
/// still export as links.
#[test]
fn exported_links_keep_only_safe_targets() {
    let links = [
        ("script", "javascript:alert(1)"),
        ("split", "java\nscript:alert(1)"),
        ("vb", "vbscript:MsgBox(1)"),
        ("data", "data:text/html,<b>x</b>"),
        ("web", "https://ok.example/x"),
        ("mail", "mailto:a@b.example"),
        ("bookmark", "#top"),
        ("relative", "other.html"),
    ];
    let mut d = Document::new();
    let mut p = Paragraph::with_text("", CharProps::default());
    for (text, l) in links {
        let n = p.len();
        p.insert_text(n, text, &CharProps { link: Some(l.to_string()), ..Default::default() }).unwrap();
        let n = p.len();
        p.insert_text(n, " ", &CharProps::default()).unwrap();
    }
    d.body = vec![para_block(p)];
    let html = String::from_utf8(export("html", &d).unwrap().unwrap()).unwrap();
    let md = String::from_utf8(export("md", &d).unwrap().unwrap()).unwrap();
    for bad in ["javascript", "vbscript", "data:"] {
        assert!(!html.to_ascii_lowercase().contains(bad), "html: {html}");
        assert!(!md.to_ascii_lowercase().replace('\n', "").contains(bad), "md: {md}");
    }
    for (text, _) in links {
        assert!(html.contains(text), "html keeps the text {text:?}: {html}");
        assert!(md.contains(text), "md keeps the text {text:?}: {md}");
    }
    for (text, l) in &links[4..] {
        assert!(html.contains(&format!("<a href=\"{l}\">{text}</a>")), "html: {html}");
        assert!(md.contains(&format!("[{text}]({l})")), "md: {md}");
    }
    for text in ["script", "split", "vb", "data"] {
        assert!(!html.contains(&format!(">{text}</a>")), "html: {html}");
        assert!(!md.contains(&format!("[{text}](")), "md: {md}");
    }
}

/// A link target with any whitespace (a no-break or ideographic space too) is written in `<…>`,
/// so the reader, which ends a bare destination at the first whitespace, reads it back whole.
#[test]
fn markdown_link_targets_with_unicode_spaces_read_back() {
    for l in ["https://e.com/a\u{a0}b", "https://e.com/a\u{3000}b", "https://e.com/a b"] {
        let mut d = Document::new();
        let mut p = Paragraph::with_text("", CharProps::default());
        p.insert_text(0, "go", &CharProps { link: Some(l.to_string()), ..Default::default() }).unwrap();
        d.body = vec![para_block(p)];
        let md = String::from_utf8(export("md", &d).unwrap().unwrap()).unwrap();
        let sp = spans(&crate::markdown::parse(&md));
        assert_eq!(sp.iter().find(|(t, _)| t == "go").and_then(|(_, f)| f.link.as_deref()), Some(l), "md: {md}");
    }
}

#[test]
fn hidden_drawings_are_left_out_of_exports() {
    let mut d = wordcraft_doc::Document::from_text("x");
    let key = d.add_media(vec![0x89, b'P', b'N', b'G'], "png");
    for hidden in [false, true] {
        let float = wordcraft_doc::para::Float { hidden, ..Default::default() };
        let image = wordcraft_doc::InlineObject::Image { media: key.clone(), w: 10.0, h: 10.0, alt: "Logo".into(), float, crop: [0.0; 4], ole: None };
        d.insert_object(&wordcraft_doc::Pos::body(0, 0), image, &Default::default()).unwrap();
    }
    // Two pictures, one hidden: one comes out.
    assert_eq!(crate::html::export(&d).matches("<img").count(), 1);
}

/// Every text span of the flow, with its formatting.
fn spans(flow: &model::Flow) -> Vec<(String, model::Fmt)> {
    flow.blocks
        .iter()
        .filter_map(|b| if let FBlock::Para(p) = b { Some(p) } else { None })
        .flat_map(|p| p.inlines.iter())
        .filter_map(|i| if let model::Inline::Text(t, f) = i { Some((t.clone(), f.clone())) } else { None })
        .collect()
}

/// A font name or link target too long for any real one is dropped, its text kept: every run
/// carried its own copy, so one long name times many runs was gigabytes (#3713).
#[test]
fn overlong_font_names_and_links_are_dropped() {
    let long = "x".repeat(1 << 20);
    let rtf = format!("{{\\rtf1{{\\fonttbl{{\\f0 Arial;}}{{\\f1 {long};}}}}\\f1 {}}}", "{\\b a}a".repeat(1000));
    let flow = crate::rtf::parse(rtf.as_bytes()).unwrap();
    let sp = spans(&flow);
    assert!(sp.iter().map(|s| s.0.len()).sum::<usize>() >= 2000);
    assert!(sp.iter().all(|(_, f)| f.font.is_none()), "an overlong RTF font name is no font");
    // A real one still is.
    let flow = crate::rtf::parse(b"{\\rtf1{\\fonttbl{\\f0 Arial;}{\\f1 Georgia;}}\\f1 a}").unwrap();
    assert_eq!(spans(&flow)[0].1.font.as_deref(), Some("Georgia"));

    let html =
        format!("<p><span style=\"font-family:'{long}'\"><a href=\"https://e.com/{long}\">kept</a></span> <font face=\"{long}\">too</font></p>");
    let sp = spans(&crate::html::parse(&html));
    assert!(sp.iter().any(|(t, _)| t.contains("kept")) && sp.iter().any(|(t, _)| t.contains("too")));
    assert!(sp.iter().all(|(_, f)| f.font.is_none() && f.link.is_none()), "{:?}", sp.iter().map(|s| &s.0).collect::<Vec<_>>());

    let sp = spans(&crate::markdown::parse(&format!("[kept](https://e.com/{long})")));
    assert_eq!(sp.len(), 1);
    assert_eq!((sp[0].0.as_str(), sp[0].1.link.is_none()), ("kept", true));
}

/// Spans of one link share its target, and so do references to one definition: a long
/// definition and many references no longer copy the destination into each (#3713).
#[test]
fn spans_share_their_link_target() {
    let dest = format!("https://e.com/{}", "x".repeat(4000));
    let md = format!("{}\n\n[r]: {dest}\n", "[a][r] **[b][r]** ".repeat(10_000));
    let sp = spans(&crate::markdown::parse(&md));
    let links: Vec<_> = sp.iter().filter_map(|(_, f)| f.link.as_ref()).collect();
    assert_eq!(links.len(), 20_000);
    assert!(links.iter().all(|l| Arc::ptr_eq(l, links[0])), "one copy of the destination");

    let html = format!("<a href=\"{dest}\">{}</a>", "<b>a</b>a".repeat(1000));
    let sp = spans(&crate::html::parse(&html));
    assert!(sp.len() >= 2000);
    let first = sp[0].1.link.clone().unwrap();
    assert!(sp.iter().all(|(_, f)| f.link.as_ref().is_some_and(|l| Arc::ptr_eq(l, &first))));
}

/// The document's runs own their link and font strings; past a budget further runs keep their
/// text but lose them, so a flow of shared names can't expand into gigabytes (#3713).
#[test]
fn run_link_and_font_copies_are_budgeted() {
    let link: Arc<str> = format!("https://e.com/{}", "x".repeat(model::MAX_LINK - 20)).into();
    let mut p = model::Para::default();
    for i in 0..20_000 {
        let f = model::Fmt { bold: i % 2 == 0, link: Some(link.clone()), ..Default::default() };
        p.push_text("a", &f);
    }
    model::begin_import();
    let (doc, warned) = crate::test_log::warnings(|| model::to_doc(&model::Flow { blocks: vec![FBlock::Para(p)], ..Default::default() }));
    let paras: Vec<_> = doc.body.iter().filter_map(|b| if let Block::Para(p) = &**b { Some(p) } else { None }).collect();
    assert_eq!(paras.iter().map(|p| p.text.len()).sum::<usize>(), 20_000, "all text kept");
    let runs: Vec<_> = paras.iter().flat_map(|p| p.runs.iter()).collect();
    let kept: usize = runs.iter().filter_map(|r| r.props.link.as_ref()).map(String::len).sum();
    assert!(kept > 0 && kept <= 64 << 20, "{kept}");
    // The runs that lose theirs are said to, once for the document (review of #3873).
    assert_eq!(warned.iter().filter(|w| w.contains("later runs lose them")).count(), 1, "{warned:?}");
}

/// Every limit that leaves a font name, link or style name out says so, once per kind per
/// document rather than once per item (review of #3873; the tracker complains of silent loss).
#[test]
fn dropped_names_and_links_are_logged_once_per_document() {
    let long = "x".repeat(model::MAX_LINK + 1);
    let html = format!(
        "<p><span style=\"font-family:'{long}'\"><a href=\"https://e.com/{long}\">a</a></span> <font face=\"{long}\">b</font> <a href=\"https://e.com/{long}\">c</a></p>"
    );
    let count = |w: &[String], what: &str| w.iter().filter(|l| l.contains(what)).count();
    for _ in 0..2 {
        // Each import is its own document: the second is reported too.
        let (doc, warned) = crate::test_log::warnings(|| crate::html::import(&html));
        assert!(doc.body.len() == 1, "the text is kept");
        assert_eq!(count(&warned, "font names longer"), 1, "{warned:?}");
        assert_eq!(count(&warned, "links longer"), 1, "{warned:?}");
    }
    let (_, warned) = crate::test_log::warnings(|| crate::html::import("<p><a href=\"https://e.com\" style=\"font-family:Georgia\">fine</a></p>"));
    assert!(warned.is_empty(), "nothing left out, nothing said: {warned:?}");

    let md = format!("[a](https://e.com/{long}) [b](https://e.com/{long})");
    let (_, warned) = crate::test_log::warnings(|| crate::markdown::import(&md));
    assert_eq!(count(&warned, "links longer"), 1, "{warned:?}");

    let rtf =
        format!("{{\\rtf1{{\\fonttbl{{\\f0 Arial;}}{{\\f1 {long};}}{{\\f2 {long};}}}}{{\\stylesheet{{\\s1 {long};}}{{\\s2 {long};}}}}\\f1 a\\f2 b}}");
    let (_, warned) = crate::test_log::warnings(|| crate::rtf::import(rtf.as_bytes()).unwrap());
    assert_eq!(count(&warned, "font names longer"), 1, "{warned:?}");
    assert_eq!(count(&warned, "style names longer"), 1, "{warned:?}");

    let tex = format!("\\href{{https://e.com/{long}}}{{a}} \\url{{https://e.com/{long}}}");
    let (_, warned) = crate::test_log::warnings(|| crate::latex::import(tex.as_bytes()));
    assert_eq!(count(&warned, "links longer"), 1, "{warned:?}");
}

/// #1910: an SVG picture is written as PNG where the format can't hold SVG (RTF, ODT) and as
/// SVG where it can (HTML).
mod svg_pictures {
    use super::*;

    const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="20" height="20" fill="#ff0000"/></svg>"##;

    fn svg_doc() -> Document {
        let mut d = Document::new();
        let media = d.add_media(SVG.to_vec(), "svg");
        let mut p = Paragraph::with_text("Logo: ", CharProps::default());
        let n = p.len();
        let obj = InlineObject::Image { media, w: 30.0, h: 15.0, alt: "logo".into(), float: Default::default(), crop: [0.0; 4], ole: None };
        p.insert_object(n, obj, &CharProps::default()).unwrap();
        d.body = vec![para_block(p)];
        d
    }

    fn picture_media(d: &Document) -> Vec<u8> {
        d.body
            .iter()
            .filter_map(|b| b.as_para())
            .flat_map(|p| &p.objects)
            .find_map(|o| match o {
                InlineObject::Image { media, .. } => d.media.get(media).map(|m| m.to_vec()),
                _ => None,
            })
            .expect("a picture")
    }

    #[test]
    fn rtf_gets_a_png() {
        let rtf = String::from_utf8(export("rtf", &svg_doc()).unwrap().unwrap()).unwrap();
        assert!(rtf.contains("\\pngblip"), "the SVG is written as a PNG");
        let back = import("rtf", rtf.as_bytes()).unwrap().unwrap();
        assert!(picture_media(&back).starts_with(b"\x89PNG"));
    }

    #[test]
    fn odt_gets_a_png() {
        let odt = export("odt", &svg_doc()).unwrap().unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&odt)).unwrap();
        let names: Vec<String> = z.file_names().map(str::to_string).collect();
        assert!(names.iter().any(|n| n.starts_with("Pictures/") && n.ends_with(".png")), "{names:?}");
        assert!(!names.iter().any(|n| n.ends_with(".svg")), "{names:?}");
        let mut manifest = String::new();
        std::io::Read::read_to_string(&mut z.by_name("META-INF/manifest.xml").unwrap(), &mut manifest).unwrap();
        assert!(!manifest.contains("image/svg+xml"));
        let back = import("odt", &odt).unwrap().unwrap();
        assert!(picture_media(&back).starts_with(b"\x89PNG"));
    }

    #[test]
    fn html_keeps_the_svg() {
        let html = String::from_utf8(export("html", &svg_doc()).unwrap().unwrap()).unwrap();
        assert!(html.contains("src=\"data:image/svg+xml;base64,"), "the SVG is embedded as is");
    }

    #[test]
    fn unreadable_svg_is_left_out_of_rtf_not_written_broken() {
        let mut d = svg_doc();
        for m in d.media.values_mut() {
            *m = Arc::new(b"<svg".to_vec());
        }
        let rtf = String::from_utf8(export("rtf", &d).unwrap().unwrap()).unwrap();
        assert!(!rtf.contains("\\pict"));
        assert!(import("rtf", rtf.as_bytes()).unwrap().is_ok());
    }
}
