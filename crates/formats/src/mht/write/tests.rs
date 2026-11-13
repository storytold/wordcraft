use super::super::{Part, collect, split_headers};
use super::*;
use wordcraft_doc::{CharProps, InlineObject, Pos, StoryRef};

/// A second picture, different from [`crate::tests::png`]: a 1×1 PNG.
const DOT: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// A document of `text`, then one paragraph per picture showing it.
fn with_pictures(text: &str, pictures: &[&[u8]]) -> Document {
    let mut d = Document::from_text(text);
    for (n, bytes) in pictures.iter().enumerate() {
        let key = d.add_media(bytes.to_vec(), "png");
        let at = d.body.len();
        d.body.push(wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("", CharProps::default())));
        let image = InlineObject::Image {
            media: key,
            w: 12.0,
            h: 6.0,
            alt: format!("picture {n}"),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
            fx: Default::default(),
        };
        d.insert_object(&Pos::body(at, 0), image, &CharProps::default()).unwrap();
    }
    d
}

/// The message's own headers and its leaf parts, read the way [`super::super::import`] reads them.
fn parts(message: &[u8]) -> (Vec<(String, String)>, Vec<Part>) {
    let (headers, body) = split_headers(message);
    let mut out = Vec::new();
    collect(&headers, body, &mut out, 0);
    (headers, out)
}

fn text(p: &Part) -> String {
    String::from_utf8(p.body.to_vec()).unwrap()
}

/// What decoding gives back for `data` encoded by [`quoted_printable`]: every line break a CRLF.
fn crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (n, line) in data.split(|&b| b == b'\n').enumerate() {
        if n > 0 {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(line.strip_suffix(b"\r").unwrap_or(line));
    }
    out
}

/// The text of the `Subject` header's encoded words.
fn decoded_subject(message: &str) -> Option<String> {
    let (headers, _) = split_headers(message.as_bytes());
    let value = headers.iter().find(|(k, _)| k == "subject").map(|(_, v)| v.clone())?;
    let mut bytes = Vec::new();
    for word in value.split_whitespace() {
        let b64 = word.strip_prefix("=?utf-8?B?").and_then(|w| w.strip_suffix("?=")).unwrap();
        let chunk = crate::model::base64_decode(b64).unwrap();
        // Each word is whole characters (RFC 2047 §5).
        assert!(std::str::from_utf8(&chunk).is_ok(), "{word}");
        bytes.extend(chunk);
    }
    Some(String::from_utf8(bytes).unwrap())
}

#[test]
fn the_page_is_the_first_part_and_each_picture_a_part_beside_it_once() {
    let png = crate::tests::png();
    let dot = crate::model::base64_decode(DOT).unwrap();
    // The first picture shown twice, then another.
    let d = with_pictures("Pictures", &[&png, &dot, &png]);
    let message = export(&d);
    let (headers, parts) = parts(&message);
    let content_type = &headers.iter().find(|(k, _)| k == "content-type").unwrap().1;
    assert!(content_type.starts_with("multipart/related; type=\"text/html\";"), "{content_type}");
    assert!(content_type.contains(&format!("boundary=\"{BOUNDARY}\"")), "{content_type}");
    assert_eq!(parts.len(), 3, "the page and two distinct pictures");
    let page = &parts[0];
    assert_eq!(page.content_type, "text/html; charset=\"utf-8\"");
    assert_eq!(page.location.as_deref(), Some(PAGE_LOCATION));
    let html = text(page);
    assert!(!html.contains("data:"), "no picture left in the page: {html}");
    // Named by Content-ID: Chrome and Edge show no `file:` part, Word links rather than keeps one
    // at the root of `file:///`.
    assert_eq!(html.matches("<img src=\"cid:image001.png\"").count(), 2, "{html}");
    assert_eq!(html.matches("<img src=\"cid:image002.png\"").count(), 1, "{html}");
    assert!(!html.contains("document_files"), "{html}");
    // The picture's other attributes stay.
    assert!(html.contains("<img src=\"cid:image002.png\" alt=\"picture 1\" width=\"16\" height=\"8\">"), "{html}");
    for (part, name, bytes) in [(&parts[1], "image001.png", &png), (&parts[2], "image002.png", &dot)] {
        assert_eq!(part.content_type, "image/png");
        assert_eq!(part.id.as_deref(), Some(name), "the Content-ID the page names");
        assert_eq!(part.location.as_deref(), Some(format!("file:///document_files/{name}").as_str()));
        assert_eq!(part.body.as_slice(), bytes.as_slice(), "{name}");
    }
    // Opened again, the pictures are where they were: the first one shown twice.
    let back = crate::mht::import(&message);
    let shown: Vec<Vec<u8>> = back
        .body
        .iter()
        .filter_map(|b| match &**b {
            wordcraft_doc::Block::Para(p) => p.objects.iter().find_map(|o| match o {
                InlineObject::Image { media, .. } => back.media.get(media).map(|m| m.to_vec()),
                _ => None,
            }),
            _ => None,
        })
        .collect();
    assert_eq!(shown, vec![png.clone(), dot, png]);
    assert_eq!(back.plain_text(StoryRef::Body).lines().next(), Some("Pictures"));
}

#[test]
fn every_line_is_short_ascii_and_ends_in_crlf() {
    let long = "Dobar dan, gospođo — 日本語のテキスト = \"quoted\"\t".repeat(40);
    let d = with_pictures(&format!("{long}\nshort line \nlast"), &[&crate::tests::png()]);
    let message = export(&d);
    assert!(message.is_ascii());
    let s = String::from_utf8(message).unwrap();
    assert!(s.ends_with("--\r\n"), "{s}");
    for (i, _) in s.match_indices('\n') {
        assert_eq!(s.as_bytes().get(i.wrapping_sub(1)), Some(&b'\r'), "a bare LF at {i}");
    }
    for line in s.split("\r\n") {
        assert!(line.len() <= MAX_LINE, "{} characters: {line}", line.len());
        assert!(!line.ends_with([' ', '\t']), "white space ends {line:?}");
    }
}

#[test]
fn quoted_printable_decodes_back_to_the_bytes() {
    let samples: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"=".to_vec(),
        b"a=b==c".to_vec(),
        b"trailing space \nand tab\t\r\n".to_vec(),
        b"  \n\t\n".to_vec(),
        b"lone \r carriage return".to_vec(),
        "č".repeat(200).into_bytes(),
        "x".repeat(75).into_bytes(),
        "x".repeat(76).into_bytes(),
        format!("{}=", "x".repeat(74)).into_bytes(),
        format!("{} {}", "x".repeat(74), "y".repeat(3)).into_bytes(),
        (0u8..=255).collect(),
        b"\n\n\n".to_vec(),
    ];
    for data in samples {
        let mut encoded = String::new();
        quoted_printable(&data, &mut encoded);
        assert!(encoded.is_ascii(), "{encoded}");
        for line in encoded.split("\r\n") {
            assert!(line.len() <= MAX_LINE, "{line}");
            assert!(!line.ends_with([' ', '\t']), "{line:?}");
            // Every `=` starts an escape or a soft line break: no `=_`, so no boundary.
            assert!(!line.contains("=_"), "{line}");
        }
        assert_eq!(super::super::quoted_printable(encoded.as_bytes()), crlf(&data), "{encoded}");
    }
}

#[test]
fn text_holding_the_boundary_does_not_end_a_part() {
    let tricky = format!("--{BOUNDARY}\n--{BOUNDARY}--\n<img src=\"data:image/png;base64,{DOT}\">");
    let d = Document::from_text(&tricky);
    let message = String::from_utf8(export(&d)).unwrap();
    let delimiters = message.split("\r\n").filter(|l| l.starts_with(&format!("--{BOUNDARY}"))).count();
    assert_eq!(delimiters, 2, "the page's delimiter and the closing one: {message}");
    let (_, parts) = parts(message.as_bytes());
    assert_eq!(parts.len(), 1, "text that reads like a picture is no picture");
    let back = crate::mht::import(message.as_bytes());
    assert_eq!(back.plain_text(StoryRef::Body), d.plain_text(StoryRef::Body));
    assert!(back.media.is_empty());
}

#[test]
fn the_title_is_the_subject_in_encoded_words() {
    let mut d = Document::from_text("x");
    d.core.title = "Izvještaj\r\nBcc: someone@example.com".into();
    let message = String::from_utf8(export(&d)).unwrap();
    // The message's headers end at the first empty line (the page's own `<title>` keeps its text).
    let head = message.split("\r\n\r\n").next().unwrap();
    assert!(!head.contains("\r\nBcc:"), "a line break in the title starts no header: {head}");
    assert_eq!(decoded_subject(&message).as_deref(), Some("Izvještaj  Bcc: someone@example.com"));
    // A long title in several scripts: words of whole characters, each line short enough.
    let long = "Квартальный отчёт 年度報告書 📈 ".repeat(12);
    let s = subject(&long).unwrap();
    for line in s.split("\r\n").filter(|l| !l.is_empty()) {
        assert!(line.len() <= 78, "{} characters: {line}", line.len());
    }
    assert_eq!(decoded_subject(&s).as_deref(), Some(long.chars().take(MAX_SUBJECT_CHARS).collect::<String>().trim()));
    // No title, no subject.
    for empty in ["", " \t\r\n"] {
        assert_eq!(subject(empty), None);
    }
    assert!(!String::from_utf8(export(&Document::from_text("x"))).unwrap().contains("Subject:"));
}

#[test]
fn only_base64_data_pictures_become_parts() {
    // A page given by hand: a picture by URL, one percent-encoded, one with no closing quote.
    let html = format!(
        "<p><img src=\"https://example.test/a.png\"><img src=\"data:image/png,%89PNG\">\
         <img src=\"data:image/png;base64,{DOT}\" alt=\"dot\"><img src=\"data:image/png;base64,{DOT}"
    );
    let message = export_html(&html, "");
    let (_, parts) = parts(&message);
    assert_eq!(parts.len(), 2, "the page and the one well-formed picture");
    let page = text(&parts[0]);
    assert!(page.contains("<img src=\"https://example.test/a.png\"><img src=\"data:image/png,%89PNG\">"), "{page}");
    assert!(page.contains("<img src=\"cid:image001.png\" alt=\"dot\">"), "{page}");
    assert!(page.ends_with(&format!("<img src=\"data:image/png;base64,{DOT}")), "{page}");
    assert_eq!(extension("image/jpeg"), "jpg");
    assert_eq!(extension("application/octet-stream"), "bin");
    assert_eq!(data_uri("data:image/png;base64,ab\"c"), None);
    assert_eq!(data_uri("data:;base64,abc"), None);
    assert_eq!(data_uri("data:image/png;base64,"), None);
}

#[test]
fn a_picture_shown_ten_thousand_times_is_one_part() {
    let html = format!("<p>{}</p>", format!("<img src=\"data:image/png;base64,{DOT}\" alt=\"\">").repeat(10_000));
    let message = export_html(&html, "Many");
    assert_eq!(parts(&message).1.len(), 2);
    // The page keeps its 10,000 pictures, by name: about the size of their tags.
    let page_len = text(&parts(&message).1[0]).len();
    assert!(page_len < 10_000 * 50, "{page_len}");
    assert_eq!(export_html(&html, "Many"), message, "the same page gives the same bytes");
}
