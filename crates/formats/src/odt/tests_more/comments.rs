//! Comments (ODF 1.3 §14.1 `office:annotation`, §14.2 `office:annotation-end`): read into the
//! document's comments with their ranges, replies and resolved state, and written back.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Document, StoryRef};

const NS_C: &str = r#"xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0""#;

fn body(xml: &str) -> Vec<u8> {
    package(
        &format!(r#"<office:document-content {NS_T} {NS_C}><office:body><office:text>{xml}</office:text></office:body></office:document-content>"#),
        None,
    )
}

/// Each comment: (author, initials, date, resolved, parent's text, its text, the text it covers).
type Summary = (String, String, String, bool, Option<String>, String, String);

fn text_of(d: &Document, part: u32) -> String {
    d.parts
        .get(&part)
        .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(OBJ, "")).collect::<Vec<_>>().join("|"))
        .unwrap_or_default()
}

/// The body text between each comment's start and end.
fn covered(d: &Document, id: u32) -> String {
    let mut out = String::new();
    let mut inside = false;
    for b in d.body.iter() {
        let Some(p) = b.as_para() else { continue };
        let offs = p.object_offsets();
        let mut last = 0;
        for (k, o) in p.objects.iter().enumerate() {
            let at = offs.get(k).copied().unwrap_or(0);
            if inside {
                out.push_str(&p.text[last..at].replace(OBJ, ""));
            }
            match o {
                InlineObject::CommentStart { id: i } if *i == id => inside = true,
                InlineObject::CommentEnd { id: i } if *i == id => inside = false,
                _ => {}
            }
            last = at + OBJ.len_utf8();
        }
        if inside {
            out.push_str(&p.text[last.min(p.text.len())..].replace(OBJ, ""));
            out.push('|');
        }
    }
    out
}

fn comments(d: &Document) -> Vec<Summary> {
    d.comments
        .iter()
        .map(|(id, c)| {
            let parent = c.parent.and_then(|p| d.comments.get(&p)).map(|p| text_of(d, p.part));
            (c.author.clone(), c.initials.clone(), c.date.clone(), c.resolved, parent, text_of(d, c.part), covered(d, *id))
        })
        .collect()
}

const COMMENTS: &str = concat!(
    r#"<text:p>Before <office:annotation office:name="c1"><dc:creator>Ada Lovelace</dc:creator><dc:date>2026-10-31T10:00:00</dc:date><meta:creator-initials>AL</meta:creator-initials><text:p>First comment.</text:p><text:p>Second line.</text:p></office:annotation>commented words<office:annotation-end office:name="c1"/> and"#,
    r#"<office:annotation loext:resolved="true"><dc:creator>Grace</dc:creator><dc:date>0000-00-00T00:00:00</dc:date><text:p>A point.</text:p></office:annotation> after.</text:p>"#,
);

#[test]
fn comments_open_with_their_ranges() {
    let d = import(&body(COMMENTS)).unwrap();
    let got = comments(&d);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(
        got[0],
        (
            "Ada Lovelace".into(),
            "AL".into(),
            "2026-10-31T10:00:00Z".into(),
            false,
            None,
            "First comment.|Second line.".into(),
            "commented words".into()
        )
    );
    // A comment without a range covers nothing; LibreOffice's empty date is no date.
    assert_eq!(got[1], ("Grace".into(), String::new(), String::new(), true, None, "A point.".into(), String::new()));
    // The comment's text isn't body text.
    assert_eq!(d.plain_text(StoryRef::Body).trim(), "Before commented words and after.");
}

#[test]
fn a_comment_across_paragraphs_keeps_its_range() {
    let d = import(&body(r#"<text:p>a <office:annotation office:name="x"><dc:creator>A</dc:creator><text:p>c</text:p></office:annotation>one</text:p><text:p>two<office:annotation-end office:name="x"/> b</text:p>"#)).unwrap();
    let (id, _) = d.comments.iter().next().unwrap();
    assert_eq!(covered(&d, *id), "one|two");
}

#[test]
fn replies_follow_their_parent() {
    let xml = concat!(
        r#"<text:p><office:annotation office:name="p"><dc:creator>Ada</dc:creator><text:p>Question?</text:p></office:annotation>text<office:annotation-end office:name="p"/>"#,
        r#"<office:annotation loext:parent-name="p"><dc:creator>Alan</dc:creator><text:p>Answer.</text:p></office:annotation></text:p>"#,
    );
    let d = import(&body(xml)).unwrap();
    let got = comments(&d);
    assert_eq!(got.len(), 2);
    assert_eq!(got[1].4.as_deref(), Some("Question?"));
    assert_eq!(got[1].5, "Answer.");
    // A reply has no place in the text of its own.
    let reply_id = *d.comments.iter().find(|(_, c)| c.parent.is_some()).unwrap().0;
    let anchored = d
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .any(|o| matches!(o, InlineObject::CommentStart { id } if *id == reply_id));
    assert!(!anchored);
}

#[test]
fn comments_round_trip() {
    let xml = concat!(
        r#"<text:p>Before <office:annotation office:name="c1"><dc:creator>Ada Lovelace</dc:creator><dc:date>2026-10-31T10:00:00</dc:date><meta:creator-initials>AL</meta:creator-initials><text:p>First comment.</text:p></office:annotation>commented</text:p><text:p>words<office:annotation-end office:name="c1"/>"#,
        r#"<office:annotation loext:parent-name="c1"><dc:creator>Alan</dc:creator><text:p>Reply.</text:p></office:annotation> and"#,
        r#"<office:annotation loext:resolved="true"><dc:creator>Grace</dc:creator><text:p>A point.</text:p></office:annotation> after.</text:p>"#,
    );
    let d = import(&body(xml)).unwrap();
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(comments(&back), comments(&d));
    assert_eq!(back.plain_text(StoryRef::Body), d.plain_text(StoryRef::Body));
}

#[test]
fn unmatched_and_hostile_comments_do_not_panic() {
    for xml in [
        // An end without a start, a start whose end never comes, a nested annotation.
        r#"<text:p>a<office:annotation-end office:name="nope"/>b</text:p>"#.to_string(),
        r#"<text:p>a<office:annotation office:name="open"><text:p>c</text:p></office:annotation>b</text:p>"#.to_string(),
        r#"<text:p><office:annotation><text:p>x<office:annotation><text:p>y</text:p></office:annotation></text:p></office:annotation></text:p>"#
            .to_string(),
        r#"<text:p><office:annotation><dc:creator>"#.to_string(),
        format!(r#"<text:p><office:annotation><dc:creator>{}</dc:creator><text:p>long</text:p></office:annotation></text:p>"#, "n".repeat(100_000)),
        r#"<text:p><office:annotation loext:parent-name="missing"><text:p>orphan</text:p></office:annotation></text:p>"#.to_string(),
    ] {
        let d = import(&body(&xml)).unwrap();
        let again = import(&export(&d).unwrap()).unwrap();
        // Every comment kept is whole: its story exists and its range closes.
        for (id, c) in &again.comments {
            assert!(again.parts.contains_key(&c.part), "{xml}");
            let starts = again
                .body
                .iter()
                .filter_map(|b| b.as_para())
                .flat_map(|p| p.objects.iter())
                .filter(|o| matches!(o, InlineObject::CommentStart { id: i } if i == id))
                .count();
            let ends = again
                .body
                .iter()
                .filter_map(|b| b.as_para())
                .flat_map(|p| p.objects.iter())
                .filter(|o| matches!(o, InlineObject::CommentEnd { id: i } if i == id))
                .count();
            assert_eq!(starts, ends, "{xml}");
        }
    }
}

#[test]
fn a_reply_to_a_reply_is_kept() {
    let xml = concat!(
        r#"<text:p><office:annotation office:name="a"><dc:creator>A</dc:creator><text:p>Root</text:p></office:annotation>x<office:annotation-end office:name="a"/>"#,
        r#"<office:annotation office:name="b" loext:parent-name="a"><dc:creator>B</dc:creator><text:p>Reply</text:p></office:annotation>"#,
        r#"<office:annotation loext:parent-name="b"><dc:creator>C</dc:creator><text:p>Reply to reply</text:p></office:annotation></text:p>"#,
    );
    let d = import(&body(xml)).unwrap();
    assert_eq!(d.comments.len(), 3);
    // LibreOffice's threads are flat: every reply answers the comment that starts them.
    let back = import(&export(&d).unwrap()).unwrap();
    let got = comments(&back);
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(got[2].4.as_deref(), Some("Root"));
    assert_eq!(got[2].5, "Reply to reply");
}
