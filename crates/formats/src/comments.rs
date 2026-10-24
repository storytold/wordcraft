//! Print Markup for the text formats: HTML, Markdown and plain text copies end with the
//! document's comments, numbered in document order, each with its author, the text it comments
//! on, what it says and its replies (like the notes at the end of a printed document).

use std::collections::{BTreeMap, HashMap, HashSet};

use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Document, PartKind, StoryRef};

/// Longest quoted anchor text (characters).
const MAX_QUOTE: usize = 200;
const MAX_COMMENTS: usize = 5_000;
const MAX_REPLIES: usize = 200;
const MAX_PARENT_CHAIN: usize = 64;

/// One comment as written out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub author: String,
    /// The text it comments on (shortened), or empty.
    pub quote: String,
    pub text: String,
    /// (author, text) of each reply, oldest first.
    pub replies: Vec<(String, String)>,
}

fn root(doc: &Document, id: u32) -> u32 {
    let mut cur = id;
    for _ in 0..MAX_PARENT_CHAIN {
        match doc.comments.get(&cur).and_then(|c| c.parent) {
            Some(p) if p != cur && doc.comments.contains_key(&p) => cur = p,
            _ => break,
        }
    }
    cur
}

fn author(a: &str) -> String {
    if a.trim().is_empty() { "Author".to_string() } else { a.trim().to_string() }
}

fn text_of(doc: &Document, part: u32) -> String {
    doc.plain_text(StoryRef::Part(part)).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The document's top-level comments: those anchored in the body in document order, then the
/// rest; replies go under the comment they answer.
pub fn notes(doc: &Document) -> Vec<Note> {
    if doc.comments.is_empty() {
        return Vec::new();
    }
    // Anchor order and the text between each comment's start and end.
    let mut order: Vec<u32> = Vec::new();
    let mut seen = HashSet::new();
    let mut quotes: HashMap<u32, String> = HashMap::new();
    let mut open: Vec<u32> = Vec::new();
    let stories = std::iter::once(StoryRef::Body).chain(
        doc.parts
            .iter()
            .filter_map(|(&k, p)| if p.kind != PartKind::Comment { Some(StoryRef::Part(k)) } else { None }),
    );
    for story in stories {
        for path in doc.para_paths(story) {
            let Some(p) = doc.para(story, &path) else { continue };
            for (off, c) in p.text.char_indices() {
                if c == OBJ {
                    match p.object_at(off) {
                        Some(InlineObject::CommentStart { id }) => {
                            if seen.insert(*id) {
                                order.push(*id);
                            }
                            if !open.contains(id) && open.len() < 64 {
                                open.push(*id);
                            }
                        }
                        Some(InlineObject::CommentEnd { id }) => open.retain(|x| x != id),
                        _ => {}
                    }
                    continue;
                }
                for id in &open {
                    let q = quotes.entry(*id).or_default();
                    if q.chars().count() <= MAX_QUOTE {
                        q.push(c);
                    }
                }
            }
            for id in &open {
                quotes.entry(*id).or_default().push(' ');
            }
        }
    }
    for id in doc.comments.keys() {
        if seen.insert(*id) {
            order.push(*id);
        }
    }
    let mut replies: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for id in doc.comments.keys() {
        let r = root(doc, *id);
        if r != *id {
            let v = replies.entry(r).or_default();
            if v.len() < MAX_REPLIES {
                v.push(*id);
            }
        }
    }
    let mut out = Vec::new();
    for id in order {
        let Some(c) = doc.comments.get(&id) else { continue };
        if root(doc, id) != id {
            continue;
        }
        let quote = quotes.get(&id).map(|q| q.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
        let quote = if quote.chars().count() > MAX_QUOTE { format!("{}…", quote.chars().take(MAX_QUOTE).collect::<String>()) } else { quote };
        let replies =
            replies.get(&id).into_iter().flatten().filter_map(|r| doc.comments.get(r)).map(|r| (author(&r.author), text_of(doc, r.part))).collect();
        out.push(Note { author: author(&c.author), quote, text: text_of(doc, c.part), replies });
        if out.len() >= MAX_COMMENTS {
            break;
        }
    }
    out
}

/// `bytes` (a copy in format `ext`) with the comments written at its end.
pub fn append(ext: &str, doc: &Document, bytes: Vec<u8>) -> Vec<u8> {
    let notes = notes(doc);
    if notes.is_empty() {
        return bytes;
    }
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" | "xhtml" => {
            let mut s = match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(e) => return e.into_bytes(),
            };
            let lower = s.to_ascii_lowercase();
            let at = lower.rfind("</body>").or_else(|| lower.rfind("</html>")).unwrap_or(s.len());
            s.insert_str(at, &html(&notes));
            s.into_bytes()
        }
        "md" | "markdown" => {
            let mut s = bytes;
            let md = markdown(&notes);
            let md_bytes = md.as_bytes();
            let needed = if s.ends_with(b"\n\n") {
                &md_bytes[2..]
            } else if s.ends_with(b"\n") {
                &md_bytes[1..]
            } else {
                md_bytes
            };
            s.extend_from_slice(needed);
            s
        }
        "txt" | "text" => {
            let mut s = bytes;
            let tx = text(&notes);
            let tx_bytes = tx.as_bytes();
            let needed = if s.ends_with(b"\r\n\r\n") {
                &tx_bytes[4..]
            } else if s.ends_with(b"\r\n") {
                &tx_bytes[2..]
            } else if s.ends_with(b"\n\n") {
                &tx_bytes[4..]
            } else if s.ends_with(b"\n") {
                &tx_bytes[2..]
            } else {
                tx_bytes
            };
            s.extend_from_slice(needed);
            s
        }
        _ => bytes,
    }
}

fn html(notes: &[Note]) -> String {
    use crate::html::esc;
    let mut s = String::from("<section class=\"comments\">\n<h2>Comments</h2>\n<ol>\n");
    for (i, n) in notes.iter().enumerate() {
        s.push_str(&format!("<li id=\"comment-{}\"><p><strong>{}</strong>", i + 1, esc(&n.author)));
        if !n.quote.is_empty() {
            s.push_str(&format!(" on <q>{}</q>", esc(&n.quote)));
        }
        s.push_str(&format!(": {}</p>", esc(&n.text)));
        if !n.replies.is_empty() {
            s.push_str("<ul>");
            for (a, t) in &n.replies {
                s.push_str(&format!("<li><strong>{}</strong>: {}</li>", esc(a), esc(t)));
            }
            s.push_str("</ul>");
        }
        s.push_str("</li>\n");
    }
    s.push_str("</ol>\n</section>\n");
    s
}

fn markdown(notes: &[Note]) -> String {
    use crate::markdown::escape;
    let mut s = String::from("\n\n## Comments\n\n");
    for (i, n) in notes.iter().enumerate() {
        s.push_str(&format!("{}. **{}**", i + 1, escape(&n.author)));
        if !n.quote.is_empty() {
            s.push_str(&format!(" on “{}”", escape(&n.quote)));
        }
        s.push_str(&format!(": {}\n", escape(&n.text)));
        for (a, t) in &n.replies {
            s.push_str(&format!("    - **{}**: {}\n", escape(a), escape(t)));
        }
    }
    s
}

fn text(notes: &[Note]) -> String {
    let mut s = String::from("\r\n\r\nComments\r\n");
    for (i, n) in notes.iter().enumerate() {
        s.push_str(&format!("\r\n[{}] {}", i + 1, n.author));
        if !n.quote.is_empty() {
            s.push_str(&format!(" on “{}”", n.quote));
        }
        s.push_str(&format!(": {}\r\n", n.text));
        for (a, t) in &n.replies {
            s.push_str(&format!("    {a}: {t}\r\n"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::para_block;
    use wordcraft_doc::props::CharProps;
    use wordcraft_doc::{Comment, Document, Paragraph, PartKind};

    fn make_test_doc() -> Document {
        let mut doc = Document::new();
        let mut p = Paragraph::new();
        p.insert_text(0, "Hello ", &CharProps::default()).unwrap();
        p.insert_object(p.len(), InlineObject::CommentStart { id: 1 }, &CharProps::default()).unwrap();
        p.insert_text(p.len(), "world", &CharProps::default()).unwrap();
        p.insert_object(p.len(), InlineObject::CommentEnd { id: 1 }, &CharProps::default()).unwrap();
        p.insert_text(p.len(), "!", &CharProps::default()).unwrap();
        doc.body = vec![para_block(p)];

        // Comment 1 text
        let mut cp = Paragraph::new();
        cp.insert_text(0, "First comment", &CharProps::default()).unwrap();
        let part1 = doc.add_part(PartKind::Comment, vec![para_block(cp)]);
        doc.comments.insert(
            1,
            Comment {
                author: "Alice".into(),
                part: part1,
                ..Default::default()
            },
        );

        // Reply to comment 1
        let mut rp = Paragraph::new();
        rp.insert_text(0, "Reply text", &CharProps::default()).unwrap();
        let part2 = doc.add_part(PartKind::Comment, vec![para_block(rp)]);
        doc.comments.insert(
            2,
            Comment {
                author: "Bob".into(),
                parent: Some(1),
                part: part2,
                ..Default::default()
            },
        );

        doc
    }

    #[test]
    fn test_notes_extraction() {
        let doc = make_test_doc();
        let n = notes(&doc);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].author, "Alice");
        assert_eq!(n[0].quote, "world");
        assert_eq!(n[0].text, "First comment");
        assert_eq!(n[0].replies, vec![("Bob".into(), "Reply text".into())]);
    }

    #[test]
    fn test_notes_in_footnote_part() {
        let mut doc = Document::new();
        let mut p = Paragraph::new();
        p.insert_text(0, "Main text", &CharProps::default()).unwrap();
        doc.body = vec![para_block(p)];

        // Footnote with a comment anchor
        let mut fp = Paragraph::new();
        fp.insert_text(0, "Note ", &CharProps::default()).unwrap();
        fp.insert_object(fp.len(), InlineObject::CommentStart { id: 10 }, &CharProps::default()).unwrap();
        fp.insert_text(fp.len(), "anchored inside", &CharProps::default()).unwrap();
        fp.insert_object(fp.len(), InlineObject::CommentEnd { id: 10 }, &CharProps::default()).unwrap();
        let _fn_part = doc.add_part(PartKind::Footnote, vec![para_block(fp)]);

        let mut cp = Paragraph::new();
        cp.insert_text(0, "Comment on footnote", &CharProps::default()).unwrap();
        let c_part = doc.add_part(PartKind::Comment, vec![para_block(cp)]);
        doc.comments.insert(
            10,
            Comment {
                author: "Carol".into(),
                part: c_part,
                ..Default::default()
            },
        );

        let n = notes(&doc);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].quote, "anchored inside");
        assert_eq!(n[0].text, "Comment on footnote");
    }

    #[test]
    fn test_append_html_case_insensitive_and_extensions() {
        let doc = make_test_doc();
        let html_bytes = b"<html><BODY><p>Hello</p></BODY></html>".to_vec();
        let res = append(".HTML", &doc, html_bytes);
        let s = String::from_utf8(res).unwrap();
        assert!(s.contains("<section class=\"comments\">"), "Must append comments");
        assert!(s.contains("</section>\n</BODY></html>"), "Must insert before closing </BODY>");

        // Test without body tag, only html tag
        let html_no_body = b"<html><p>Fragment</p></html>".to_vec();
        let res2 = append("html", &doc, html_no_body);
        let s2 = String::from_utf8(res2).unwrap();
        assert!(s2.contains("</section>\n</html>"), "Must insert before closing </html>");
    }

    #[test]
    fn test_append_markdown_and_txt_newline_normalization() {
        let doc = make_test_doc();
        let md_with_newlines = b"# Heading\n\nContent\n\n".to_vec();
        let res_md = append(".md", &doc, md_with_newlines);
        let s_md = String::from_utf8(res_md).unwrap();
        assert!(!s_md.contains("\n\n\n\n## Comments"), "Should not have excess blank lines");
        assert!(s_md.contains("Content\n\n## Comments\n\n1. **Alice** on “world”: First comment"));

        let txt_bytes = b"Line 1\r\nLine 2\r\n\r\n".to_vec();
        let res_txt = append("TEXT", &doc, txt_bytes);
        let s_txt = String::from_utf8(res_txt).unwrap();
        assert!(!s_txt.contains("\r\n\r\n\r\n\r\nComments"), "Should not have excess CRLF");
        assert!(s_txt.contains("Comments\r\n\r\n[1] Alice on “world”: First comment"));
    }
}
