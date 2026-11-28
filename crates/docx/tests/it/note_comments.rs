//! Comments anchored inside footnotes and endnotes keep their ranges on read. All markup here is
//! synthetic, written from ECMA-376 §17.13.4 and §17.11.

use wordcraft_doc::para::{NoteKind, OBJ};
use wordcraft_doc::{Document, InlineObject};

use crate::fixtures::zip;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const DOC_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes" Target="endnotes.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#;

fn note(tag: &str, mark: &str, comment: u32, text: &str) -> String {
    format!(
        r#"<w:{tag} w:id="1"><w:p><w:r><w:{mark}/></w:r><w:commentRangeStart w:id="{comment}"/><w:r><w:t xml:space="preserve"> {text}</w:t></w:r><w:commentRangeEnd w:id="{comment}"/><w:r><w:commentReference w:id="{comment}"/></w:r></w:p></w:{tag}>"#
    )
}

fn read_doc() -> Document {
    let doc = format!(
        r#"<w:document {W_NS}><w:body><w:p><w:r><w:t>Body</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r><w:r><w:endnoteReference w:id="1"/></w:r></w:p></w:body></w:document>"#
    );
    let footnotes = format!(r#"<w:footnotes {W_NS}>{}</w:footnotes>"#, note("footnote", "footnoteRef", 7, "Foot."));
    let endnotes = format!(r#"<w:endnotes {W_NS}>{}</w:endnotes>"#, note("endnote", "endnoteRef", 8, "End."));
    let comments = format!(
        r#"<w:comments {W_NS}><w:comment w:id="7" w:author="Ann" w:initials="A"><w:p><w:r><w:t>On the footnote</w:t></w:r></w:p></w:comment><w:comment w:id="8" w:author="Bob" w:initials="B"><w:p><w:r><w:t>On the endnote</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let bytes = zip(&[
        ("_rels/.rels", ROOT_RELS.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/_rels/document.xml.rels", DOC_RELS.as_bytes()),
        ("word/footnotes.xml", footnotes.as_bytes()),
        ("word/endnotes.xml", endnotes.as_bytes()),
        ("word/comments.xml", comments.as_bytes()),
    ]);
    wordcraft_docx::read(&bytes).unwrap()
}

/// comments.xml has to be read before the notes: a note's `w:commentRangeStart` resolves its id
/// against the comments already read, and was dropped when the notes came first.
#[test]
fn comment_ranges_inside_notes_are_kept() {
    let d = read_doc();
    let body = d.body[0].as_para().unwrap();
    let refs: Vec<(NoteKind, u32)> = body
        .objects
        .iter()
        .filter_map(|o| match o {
            InlineObject::NoteRef { kind, id, .. } => Some((*kind, *id)),
            _ => None,
        })
        .collect();
    assert_eq!(refs.len(), 2, "{:?}", body.objects);
    for ((kind, id), (author, text)) in refs.into_iter().zip([("Ann", " Foot."), ("Bob", " End.")]) {
        let p = d.parts.get(&id).unwrap().blocks[0].as_para().unwrap();
        let comment = match &p.objects[..] {
            [InlineObject::NoteRef { kind: k, id: n, .. }, InlineObject::CommentStart { id: c }, InlineObject::CommentEnd { id: e }]
                if *k == kind && *n == id && c == e =>
            {
                *c
            }
            o => panic!("{kind:?} {id}: {o:?}"),
        };
        assert_eq!(d.comments.get(&comment).unwrap().author, author);
        assert_eq!(p.text.chars().filter(|c| *c == OBJ).count(), 3);
        assert_eq!(p.plain_text(), text);
    }
}
