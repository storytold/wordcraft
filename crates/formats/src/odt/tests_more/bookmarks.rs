//! Bookmarks (ODF 1.3 §6.2.1 `text:bookmark`, `text:bookmark-start`/`-end`): a point or the text
//! between its start and end, in one paragraph or across several, kept as a WordCraft bookmark
//! over that text (what a reference to it shows) and written back the same way.

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::para::{InlineObject, OBJ};

/// Each bookmark: (name, the text it covers, paragraphs joined by '|').
fn bookmarks(d: &Document) -> Vec<(String, String)> {
    let mut open: Vec<(String, String)> = Vec::new();
    let mut done = Vec::new();
    for (i, b) in d.body.iter().enumerate() {
        let Some(p) = b.as_para() else { continue };
        if i > 0 {
            for o in &mut open {
                o.1.push('|');
            }
        }
        let mut k = 0;
        for ch in p.text.chars() {
            if ch == OBJ {
                match p.objects.get(k) {
                    Some(InlineObject::BookmarkStart { name }) => open.push((name.clone(), String::new())),
                    Some(InlineObject::BookmarkEnd { name }) => {
                        if let Some(at) = open.iter().position(|(n, _)| n == name) {
                            done.push(open.remove(at));
                        }
                    }
                    _ => {}
                }
                k += 1;
            } else {
                for o in &mut open {
                    o.1.push(ch);
                }
            }
        }
    }
    done.sort();
    done
}

fn open() -> Document {
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Before <text:bookmark-start text:name="words"/>important words<text:bookmark-end text:name="words"/> after<text:bookmark text:name="spot"/>.</text:p><text:p>From <text:bookmark-start text:name="span"/>here</text:p><text:p>to there<text:bookmark-end text:name="span"/>.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, None)).unwrap()
}

#[test]
fn a_bookmark_covers_its_text() {
    assert_eq!(
        bookmarks(&open()),
        [("span".to_string(), "here|to there".to_string()), ("spot".into(), String::new()), ("words".into(), "important words".into())]
    );
}

#[test]
fn bookmarks_round_trip() {
    let d = open();
    let bytes = export(&d).unwrap();
    let c = content_of(&bytes);
    assert!(c.contains(r#"<text:bookmark-start text:name="words"/>"#) && c.contains(r#"<text:bookmark text:name="spot"/>"#), "{c}");
    assert_eq!(bookmarks(&import(&bytes).unwrap()), bookmarks(&d));
}

#[test]
fn hostile_bookmarks_do_not_panic() {
    for body in [
        r#"<text:p><text:bookmark-end text:name="never"/>x</text:p>"#,
        r#"<text:p><text:bookmark-start text:name="open"/>no end</text:p>"#,
        r#"<text:p><text:bookmark-start text:name="a"/><text:bookmark-start text:name="a"/>twice<text:bookmark-end text:name="a"/></text:p>"#,
        r#"<text:p><text:bookmark-start/>nameless<text:bookmark-end/></text:p>"#,
    ] {
        let content =
            format!(r#"<office:document-content {NS_T}><office:body><office:text>{body}</office:text></office:body></office:document-content>"#);
        let d = import(&package(&content, None)).unwrap();
        let _ = export(&d).unwrap();
    }
}
