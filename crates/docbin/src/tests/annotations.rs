//! Comments (annotations, [MS-DOC] §2.3.4): the reference and text PLCs, the author table and
//! the annotation bookmarks, hand-built for these tests from the specification. No file
//! produced by Word is used.

use proptest::prelude::*;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Document, PartKind, StoryRef};

use super::{StructSpec, plc, read, struct_doc_with, structured_spec};

/// FIB pair indices in `FibRgFcLcb97` ([MS-DOC] §2.5.6).
const AND_REF: usize = 4;
const AND_TXT: usize = 5;
const ATN_OWNERS: usize = 36;
const ATN_BKMK: usize = 37;
const ATN_BKF: usize = 42;
const ATN_BKL: usize = 43;

/// An `ATRDPre10` (§2.9.7): initials, author index and bookmark tag (-1: no range).
fn atrd(initials: &str, ibst: u16, tag: i32) -> Vec<u8> {
    let units: Vec<u16> = initials.encode_utf16().collect();
    let mut v = (units.len() as u16).to_le_bytes().to_vec();
    let mut chars: Vec<u8> = units.iter().flat_map(|c| c.to_le_bytes()).collect();
    chars.resize(18, 0);
    v.extend(chars);
    v.extend(ibst.to_le_bytes());
    v.extend([0u8; 4]); // bitsNotUsed, grfNotused
    v.extend(tag.to_le_bytes());
    v
}

/// A `GrpXstAtnOwners`: one `Xst` (§2.9.353) per author.
fn owners(names: &[&str]) -> Vec<u8> {
    let mut v = Vec::new();
    for n in names {
        let units: Vec<u16> = n.encode_utf16().collect();
        v.extend((units.len() as u16).to_le_bytes());
        v.extend(units.iter().flat_map(|c| c.to_le_bytes()));
    }
    v
}

/// A `SttbfAtnBkmk` (§2.9.277): an extended STTB of empty strings, each followed by an `ATNBE`
/// (§2.9.4) carrying its tag.
fn atn_bkmk(tags: &[u32]) -> Vec<u8> {
    let mut v: Vec<u8> = [0xFFFFu16, tags.len() as u16, 10].iter().flat_map(|x| x.to_le_bytes()).collect();
    for t in tags {
        v.extend(0u16.to_le_bytes()); // cchData
        v.extend(0x0100u16.to_le_bytes()); // bmc
        v.extend(t.to_le_bytes()); // lTag
        v.extend((-1i32).to_le_bytes()); // lTagOld
    }
    v
}

/// A `Plcfbkf` for annotation bookmarks: start CPs (plus the limit) and one `FBKF` each, whose
/// `ibkl` is the index of its end in the `Plcfbkl`.
fn bkf(starts: &[u32], ibkl: &[u16], limit: u32) -> Vec<u8> {
    let cps: Vec<u32> = starts.iter().copied().chain([limit]).collect();
    let data: Vec<u8> = ibkl.iter().flat_map(|i| [i.to_le_bytes(), 0u16.to_le_bytes()].concat()).collect();
    plc(&cps, &data)
}

fn open(text: &'static str, comment_stories: &[&str], extra_pairs: Vec<(usize, Vec<u8>)>) -> Document {
    let ccp = text.chars().count() as u32;
    let spec = StructSpec { text, extra_pairs, ..structured_spec(text, ccp) };
    read(&struct_doc_with(&spec, comment_stories)).expect("opens")
}

fn body_para(doc: &Document) -> wordcraft_doc::Paragraph {
    doc.body.first().and_then(|b| b.as_para().cloned()).expect("a body paragraph")
}

/// "See this word": a comment by Ada Reviewer (AR) on "this", its reference mark (0x05) just
/// after the range, and its text "Nice" in the comment subdocument.
const TEXT: &str = "See this\u{5} word\r";
const STORIES: [&str; 2] = ["\u{5}Nice\r", "\r"];

fn ranged_pairs(tag_in_atrd: i32, tag_in_bkmk: u32) -> Vec<(usize, Vec<u8>)> {
    let ccp = TEXT.chars().count() as u32;
    vec![
        (AND_REF, plc(&[8, ccp], &atrd("AR", 0, tag_in_atrd))),
        // The text range [0, 6), then ccpAtn - 1 (the guard) and the ignored last CP.
        (AND_TXT, plc(&[0, 6, 7], &[])),
        (ATN_OWNERS, owners(&["Ada Reviewer"])),
        (ATN_BKMK, atn_bkmk(&[tag_in_bkmk])),
        (ATN_BKF, bkf(&[4], &[0], ccp)),
        (ATN_BKL, plc(&[8, ccp], &[])),
    ]
}

#[test]
fn a_comment_keeps_its_author_initials_text_and_range() {
    let doc = open(TEXT, &STORIES, ranged_pairs(7, 7));
    assert_eq!(doc.comments.len(), 1, "{:?}", doc.comments);
    let (&id, c) = doc.comments.iter().next().expect("one comment");
    assert_eq!(c.author, "Ada Reviewer");
    assert_eq!(c.initials, "AR");
    let part = doc.parts.get(&c.part).expect("the comment's part");
    assert_eq!(part.kind, PartKind::Comment);
    // The story's own reference mark is not part of the comment text.
    assert_eq!(doc.plain_text(StoryRef::Part(c.part)).trim_end(), "Nice");

    // The range covers "this"; the reference mark itself leaves nothing behind.
    let p = body_para(&doc);
    assert_eq!(p.text, "See \u{FFFC}this\u{FFFC} word");
    assert_eq!(p.objects, vec![InlineObject::CommentStart { id }, InlineObject::CommentEnd { id }]);
}

#[test]
fn a_comment_without_a_range_is_anchored_at_its_reference() {
    // lTagBkmk = -1: the comment is on an empty range, at its reference mark.
    let doc = open(TEXT, &STORIES, ranged_pairs(-1, 7));
    let (&id, _) = doc.comments.iter().next().expect("one comment");
    let p = body_para(&doc);
    assert_eq!(p.text, "See this\u{FFFC}\u{FFFC} word");
    assert_eq!(p.objects, vec![InlineObject::CommentStart { id }, InlineObject::CommentEnd { id }]);
}

#[test]
fn a_comment_whose_bookmark_is_missing_falls_back_to_its_reference() {
    // The tag names no annotation bookmark: the comment is kept, anchored at its reference.
    let doc = open(TEXT, &STORIES, ranged_pairs(9, 7));
    let (&id, _) = doc.comments.iter().next().expect("one comment");
    let p = body_para(&doc);
    assert_eq!(p.text, "See this\u{FFFC}\u{FFFC} word");
    assert_eq!(p.objects, vec![InlineObject::CommentStart { id }, InlineObject::CommentEnd { id }]);
}

#[test]
fn a_comment_whose_range_reaches_past_the_text_falls_back_to_its_reference() {
    // The bookmark ends at ccpText, where the walk stops: the range would never be closed.
    let ccp = TEXT.chars().count() as u32;
    let mut pairs = ranged_pairs(7, 7);
    if let Some((_, bkl)) = pairs.iter_mut().find(|(i, _)| *i == ATN_BKL) {
        *bkl = plc(&[ccp, ccp], &[]);
    }
    let doc = open(TEXT, &STORIES, pairs);
    let (&id, _) = doc.comments.iter().next().expect("one comment");
    let p = body_para(&doc);
    assert_eq!(p.text, "See this\u{FFFC}\u{FFFC} word");
    assert_eq!(p.objects, vec![InlineObject::CommentStart { id }, InlineObject::CommentEnd { id }]);
}

#[test]
fn two_comments_each_get_their_author_and_text() {
    let text = "One\u{5} two\u{5}\r";
    let ccp = text.chars().count() as u32;
    let pairs = vec![
        (AND_REF, plc(&[3, 8, ccp], &[atrd("A", 1, -1), atrd("B", 0, -1)].concat())),
        (AND_TXT, plc(&[0, 7, 14, 15], &[])),
        (ATN_OWNERS, owners(&["Bo", "Al"])),
    ];
    let doc = open(text, &["\u{5}First\r", "\u{5}Other\r", "\r"], pairs);
    let got: Vec<(String, String, String)> = doc
        .comments
        .values()
        .map(|c| (c.author.clone(), c.initials.clone(), doc.plain_text(StoryRef::Part(c.part)).trim_end().to_string()))
        .collect();
    assert_eq!(got, vec![("Al".into(), "A".into(), "First".into()), ("Bo".into(), "B".into(), "Other".into())]);
}

#[test]
fn an_author_index_past_the_table_leaves_the_author_empty() {
    let ccp = TEXT.chars().count() as u32;
    let pairs = vec![(AND_REF, plc(&[8, ccp], &atrd("AR", 5, -1))), (AND_TXT, plc(&[0, 6, 7], &[])), (ATN_OWNERS, owners(&["Ada Reviewer"]))];
    let doc = open(TEXT, &STORIES, pairs);
    let c = doc.comments.values().next().expect("one comment");
    assert_eq!(c.author, "");
    assert_eq!(c.initials, "AR");
}

#[test]
fn comment_texts_that_overlap_are_read_once() {
    // A hostile PlcfandTxt steps back: the third comment's text [2, 6) lies inside the first's.
    let text = "a\u{5}b\u{5}c\u{5}\r";
    let ccp = text.chars().count() as u32;
    let atrds = [atrd("A", 0, -1), atrd("B", 0, -1), atrd("C", 0, -1)].concat();
    let pairs = vec![(AND_REF, plc(&[1, 3, 5, ccp], &atrds)), (AND_TXT, plc(&[0, 6, 2, 6, 7], &[]))];
    let doc = open(text, &["\u{5}Nice\r", "\r"], pairs);
    assert_eq!(doc.comments.len(), 1);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Junk in every comment table (references, texts, authors, bookmarks) never panics.
    #[test]
    fn junk_comment_tables_never_panic(
        refs in proptest::collection::vec(any::<u8>(), 0..200),
        txts in proptest::collection::vec(any::<u8>(), 0..64),
        authors in proptest::collection::vec(any::<u8>(), 0..64),
        bkmk in proptest::collection::vec(any::<u8>(), 0..64),
        bkf in proptest::collection::vec(any::<u8>(), 0..64),
        bkl in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        let pairs = vec![(AND_REF, refs), (AND_TXT, txts), (ATN_OWNERS, authors), (ATN_BKMK, bkmk), (ATN_BKF, bkf), (ATN_BKL, bkl)];
        let ccp = TEXT.chars().count() as u32;
        let spec = StructSpec { text: TEXT, extra_pairs: pairs, ..structured_spec(TEXT, ccp) };
        let _ = read(&struct_doc_with(&spec, &STORIES));
    }

    /// Every truncation of a well-formed set of comment tables never panics.
    #[test]
    fn truncated_comment_tables_never_panic(cut in 0usize..40, which in 0usize..6) {
        let mut pairs = ranged_pairs(7, 7);
        if let Some((_, blob)) = pairs.get_mut(which) {
            blob.truncate(cut);
        }
        let ccp = TEXT.chars().count() as u32;
        let spec = StructSpec { text: TEXT, extra_pairs: pairs, ..structured_spec(TEXT, ccp) };
        let _ = read(&struct_doc_with(&spec, &STORIES));
    }
}
