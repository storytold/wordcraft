//! A comment's first paragraph starts with its annotation reference mark (`w:annotationRef`),
//! one per comment, through any number of saves (#3541). Original synthetic packages.

use std::io::{Read, Write};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A package that comments on one body range per id in `ids`, with the given `w:comment`
/// elements and styles.
fn docx(ids: &[u32], comments: &str, styles: &str) -> Vec<u8> {
    let rels = "http://schemas.openxmlformats.org/package/2006/relationships";
    let root = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{rels}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let doc_rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{rels}"><Relationship Id="rId1" Type="{REL}/comments" Target="comments.xml"/><Relationship Id="rId2" Type="{REL}/styles" Target="styles.xml"/></Relationships>"#
    );
    let ranges: String = ids
        .iter()
        .map(|i| {
            format!(
                r#"<w:p><w:commentRangeStart w:id="{i}"/><w:r><w:t>Range {i}</w:t></w:r><w:commentRangeEnd w:id="{i}"/><w:r><w:commentReference w:id="{i}"/></w:r></w:p>"#
            )
        })
        .collect();
    let parts = [
        ("_rels/.rels", root),
        ("word/_rels/document.xml.rels", doc_rels),
        (
            "word/document.xml",
            format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{ranges}</w:body></w:document>"#),
        ),
        ("word/comments.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments xmlns:w="{W}">{comments}</w:comments>"#)),
        ("word/styles.xml", format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}">{styles}</w:styles>"#)),
    ];
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in &parts {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

fn save(bytes: &[u8]) -> Vec<u8> {
    wordcraft_docx::write(&wordcraft_docx::read(bytes).unwrap()).unwrap()
}

/// The `w:comment` elements of a written `comments.xml`, in order.
fn comments(xml: &str) -> Vec<&str> {
    xml.match_indices("<w:comment ")
        .map(|(at, _)| {
            let end = at + xml[at..].find("</w:comment>").unwrap_or_else(|| panic!("unclosed comment: {xml}")) + "</w:comment>".len();
            &xml[at..end]
        })
        .collect()
}

/// The first run of a written comment.
fn first_run(comment: &str) -> &str {
    let at = comment.find("<w:r>").unwrap_or_else(|| panic!("no run: {comment}"));
    let end = at + comment[at..].find("</w:r>").unwrap() + "</w:r>".len();
    &comment[at..end]
}

#[test]
fn a_comment_keeps_its_annotation_reference_mark_through_two_saves() {
    // The shape of the report: a bare mark run, then the comment's text.
    let input = docx(
        &[0],
        r#"<w:comment w:id="0" w:author="Rook Vale" w:initials="RV" w:date="2026-02-03T04:05:06Z"><w:p><w:r><w:annotationRef/></w:r><w:r><w:t>Check the figure</w:t></w:r></w:p></w:comment>"#,
        "",
    );
    let mark = r#"<w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:annotationRef/></w:r>"#;
    let once = save(&input);
    let written = part(&once, "word/comments.xml");
    assert_eq!(written.matches("<w:annotationRef/>").count(), 1, "{written}");
    let [comment] = comments(&written)[..] else { panic!("one comment: {written}") };
    assert_eq!(first_run(comment), mark, "{comment}");
    assert!(comment.contains("Check the figure"), "{comment}");
    // Read back, the mark is not text: the second save writes it once again, not twice.
    let twice = save(&once);
    let again = part(&twice, "word/comments.xml");
    assert_eq!(again.matches("<w:annotationRef/>").count(), 1, "{again}");
    let [comment] = comments(&again)[..] else { panic!("one comment: {again}") };
    assert_eq!(first_run(comment), mark, "{comment}");
    assert!(comment.contains("Check the figure"), "{comment}");
}

#[test]
fn every_comment_starts_with_one_mark_in_the_documents_annotation_reference_style() {
    // A localized id for the annotation reference style; the second comment, of two paragraphs,
    // comes without a mark.
    let styles = r#"<w:style w:type="character" w:styleId="Kommentarzeichen"><w:name w:val="annotation reference"/><w:rPr><w:sz w:val="16"/></w:rPr></w:style>"#;
    let input = docx(
        &[1, 2],
        concat!(
            r#"<w:comment w:id="1" w:author="Rook Vale" w:initials="RV" w:date="2026-02-03T04:05:06Z"><w:p>"#,
            r#"<w:r><w:rPr><w:rStyle w:val="Kommentarzeichen"/></w:rPr><w:annotationRef/></w:r><w:r><w:t>First remark</w:t></w:r></w:p></w:comment>"#,
            r#"<w:comment w:id="2" w:author="Mira Holt" w:initials="MH" w:date="2026-02-03T04:05:07Z">"#,
            r#"<w:p><w:r><w:t>Second remark</w:t></w:r></w:p><w:p><w:r><w:t>Its second paragraph</w:t></w:r></w:p></w:comment>"#,
        ),
        styles,
    );
    let mark = r#"<w:r><w:rPr><w:rStyle w:val="Kommentarzeichen"/></w:rPr><w:annotationRef/></w:r>"#;
    let written = part(&save(&input), "word/comments.xml");
    let [first, second] = comments(&written)[..] else { panic!("two comments: {written}") };
    assert_eq!(first_run(first), mark, "{first}");
    assert_eq!(first.matches("<w:annotationRef/>").count(), 1, "{first}");
    assert_eq!(first_run(second), mark, "{second}");
    // Only the first paragraph carries it.
    assert_eq!(second.matches("<w:annotationRef/>").count(), 1, "{second}");
    let tail = &second[second.find("Second remark").unwrap()..];
    assert!(tail.contains("Its second paragraph") && !tail.contains("annotationRef"), "{second}");
}
