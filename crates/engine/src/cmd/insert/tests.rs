use super::*;

/// A 4 × 4 RGB TIFF that says its strips are JPEG (they aren't): its header still reads.
fn jpeg_strip_tiff() -> Vec<u8> {
    let mut t = Vec::new();
    image::RgbImage::new(4, 4).write_to(&mut std::io::Cursor::new(&mut t), image::ImageFormat::Tiff).unwrap();
    assert_eq!(&t[..4], b"II*\0");
    let ifd = u32::from_le_bytes(t[4..8].try_into().unwrap()) as usize;
    let n = usize::from(u16::from_le_bytes([t[ifd], t[ifd + 1]]));
    let e = (0..n).map(|i| ifd + 2 + 12 * i).find(|&e| t[e..e + 2] == 259u16.to_le_bytes()).unwrap();
    t[e + 8..e + 10].copy_from_slice(&7u16.to_le_bytes());
    t
}

/// #3645: a TIFF with JPEG strips is never decoded, so it is neither inserted nor swapped in;
/// before, its readable header was all insertion looked at.
#[test]
fn tiffs_with_jpeg_strips_are_refused() {
    let tiff = jpeg_strip_tiff();
    assert_eq!(wordcraft_render::image_size(&tiff), Some((4, 4)), "its header reads");
    let mut s = Session::new(wordcraft_doc::Document::new());
    let e = s.run("insert.picture", &json!({"data": base64_encode(&tiff)})).unwrap_err().to_string();
    assert!(e.contains("TIFF"), "{e}");
    assert!(s.doc.media.is_empty());
    let mut plain = Vec::new();
    image::RgbImage::new(4, 4).write_to(&mut std::io::Cursor::new(&mut plain), image::ImageFormat::Tiff).unwrap();
    s.run("insert.picture", &json!({"data": base64_encode(&plain)})).unwrap();
    let e = s.run("picture.change", &json!({"data": base64_encode(&tiff)})).unwrap_err().to_string();
    assert!(e.contains("TIFF"), "{e}");
    assert_eq!(s.doc.media.len(), 1);
}

/// Closing the header or footer (Esc, the ribbon) puts the caret back where it was in the
/// body, not on page 1 (#3331).
#[test]
fn close_header_returns_to_the_body_selection() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "One"})).unwrap();
    s.run("insert.pageBreak", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Two words here"})).unwrap();
    s.run("select.text", &json!({"text": "words"})).unwrap();
    let before = s.sel.clone();
    // A header written to stays.
    s.run("insert.editHeader", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Head"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(s.sel, before);
    // An empty one made by a double-click on page 2 is taken away again.
    s.run("insert.editFooter", &json!({"page": 1})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(s.sel, before);
    assert_eq!(s.doc.last_section.footers, Default::default());
}

/// #960: editing the header of a page that has none creates the one that page shows (first
/// page, even page or default); typing goes into it, and closing an unwritten one takes it
/// away again so the saved file stays as it was.
#[test]
fn edit_header_on_a_page_creates_its_slot() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "One"})).unwrap();
    s.run("insert.pageBreak", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Two"})).unwrap();
    assert_eq!(s.layout().pages.len(), 2);
    // Plain document: the default header, then closing it unwritten leaves no header.
    let r = s.run("insert.editHeader", &json!({"page": 1})).unwrap();
    let id = r["story"].as_u64().unwrap() as u32;
    assert_eq!(s.sel.focus.story, StoryRef::Part(id));
    assert_eq!(s.doc.last_section.headers.default, Some(id));
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(s.doc.last_section.headers, Default::default());
    assert!(!s.doc.parts.contains_key(&id));
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert!(back.parts.values().all(|p| !matches!(p.kind, PartKind::Header | PartKind::Footer)), "no empty header saved");
    // Written footer stays.
    let r = s.run("insert.editFooter", &json!({"page": 0})).unwrap();
    let foot = r["story"].as_u64().unwrap() as u32;
    s.run("text.insert", &json!({"text": "Foot"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(s.doc.last_section.footers.default, Some(foot));
    assert_eq!(s.layout().pages[1].footer_story, Some(foot));
    // Different first page: page 0's header is the first-page one, page 1's the default.
    s.run("layout.differentFirstPage", &json!({"value": true})).unwrap();
    let first = s.run("insert.editHeader", &json!({"page": 0})).unwrap()["story"].as_u64().unwrap() as u32;
    s.run("text.insert", &json!({"text": "Cover"})).unwrap();
    assert_eq!(s.doc.last_section.headers.first, Some(first));
    assert_eq!(s.doc.last_section.headers.default, None);
    // Odd & even: page 1 (page number 2) gets the even header.
    s.run("layout.differentOddEven", &json!({"value": true})).unwrap();
    let even = s.run("insert.editHeader", &json!({"page": 1})).unwrap()["story"].as_u64().unwrap() as u32;
    s.run("text.insert", &json!({"text": "Even"})).unwrap();
    assert_eq!(s.doc.last_section.headers.even, Some(even));
    // A page that shows a header edits that one; a junk page is an error, not a panic.
    assert_eq!(s.run("insert.editHeader", &json!({"page": 0})).unwrap()["story"].as_u64(), Some(first as u64));
    assert!(s.run("insert.editHeader", &json!({"page": 99})).is_err());
    assert!(s.run("insert.editHeader", &json!({"page": u64::MAX})).is_err());
}

/// #2826: with Different First Page on, Insert › Header and Edit Header with the caret on
/// page 1 give page 1 its own header; page 2 keeps the default one.
#[test]
fn different_first_page_header_is_reachable_from_page_one() {
    let mut s = two_pages();
    s.run("insert.header", &json!({"text": "Head"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    let head = s.doc.last_section.headers.default.unwrap();
    s.run("layout.differentFirstPage", &json!({"value": true})).unwrap();
    assert_eq!(s.layout().pages[0].header_story, None, "page 1 shows the empty first-page header");
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    let first = s.run("insert.header", &json!({"preset": "title"})).unwrap()["story"].as_u64().unwrap() as u32;
    assert_ne!(first, head);
    assert_eq!(s.doc.last_section.headers.first, Some(first));
    let pages = s.layout().pages.clone();
    assert_eq!((pages[0].header_story, pages[1].header_story), (Some(first), Some(head)));
    // Edit Footer from page 1 (no page given): page 1's own footer, not the default.
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    let foot = s.run("insert.editFooter", &json!({})).unwrap()["story"].as_u64().unwrap() as u32;
    assert_eq!((s.doc.last_section.footers.first, s.doc.last_section.footers.default), (Some(foot), None));
}

fn two_pages() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "One"})).unwrap();
    s.run("insert.pageBreak", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Two"})).unwrap();
    s
}

fn headers(s: &Session) -> usize {
    s.doc.parts.values().filter(|p| matches!(p.kind, PartKind::Header | PartKind::Footer)).count()
}

/// #960 review: making a header is an edit — a read-only document refuses it (an existing
/// header can still be gone into), and Undo takes a made one away again.
#[test]
fn creating_a_header_is_an_undoable_protected_edit() {
    let mut s = two_pages();
    s.doc.settings.protection = Some("readOnly".into());
    assert!(s.run("insert.editHeader", &json!({"page": 0})).is_err(), "read only: no new header");
    assert!(s.run("insert.editFooter", &json!({})).is_err(), "read only: no new footer");
    assert_eq!(headers(&s), 0);
    s.doc.settings.protection = None;
    let id = s.run("insert.editHeader", &json!({"page": 1})).unwrap()["story"].as_u64().unwrap() as u32;
    assert!(s.doc.parts.contains_key(&id));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(headers(&s), 0, "undo takes the new header away");
    assert_eq!(s.doc.last_section.headers, Default::default());
    // Going into one that is there is no edit: allowed read-only, and no undo step.
    let id = s.run("insert.editHeader", &json!({"page": 0})).unwrap()["story"].as_u64().unwrap() as u32;
    s.run("text.insert", &json!({"text": "Head"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    let steps = s.undo_labels().len();
    s.doc.settings.protection = Some("readOnly".into());
    assert_eq!(s.run("insert.editHeader", &json!({"page": 1})).unwrap()["story"].as_u64(), Some(id as u64));
    assert_eq!(s.undo_labels().len(), steps);
}

/// #960 review: a new header still empty isn't saved, even while it is open; and one an Undo
/// brings back after closing is taken away on the next close.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_empty_new_header_is_never_saved() {
    let dir = std::env::temp_dir().join(format!("wordcraft-hf960-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.docx");
    let mut s = two_pages();
    s.run("insert.editHeader", &json!({"page": 0})).unwrap();
    s.run("file.save", &json!({"path": path.to_string_lossy()})).unwrap();
    let back = wordcraft_docx::read(&std::fs::read(&path).unwrap()).unwrap();
    assert!(back.parts.values().all(|p| p.kind != PartKind::Header), "no empty header saved");
    assert_eq!(headers(&s), 1, "still open for typing");
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(headers(&s), 0);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(headers(&s), 1, "undo brings it back");
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(headers(&s), 0, "and the next close takes it away again");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #960 review: with no header anywhere, one made on a later section's page goes in the first
/// section, linked through to the later ones (Word's "Link to Previous" default); with Odd &
/// Even on and only a default header, an even page edits the one it shows.
#[test]
fn new_header_links_to_previous_and_even_pages_edit_what_they_show() {
    let mut s = two_pages();
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Three"})).unwrap();
    let pages = s.layout().pages.len();
    assert!(pages >= 3 && s.layout().pages[pages - 1].section == 1, "a second section");
    let id = s.run("insert.editHeader", &json!({"page": pages - 1})).unwrap()["story"].as_u64().unwrap() as u32;
    s.run("text.insert", &json!({"text": "All"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert!(s.layout().pages.iter().all(|p| p.header_story == Some(id)), "every page shows it");
    assert_eq!(s.doc.sections()[0].1.headers.default, Some(id));
    s.run("layout.differentOddEven", &json!({"value": true})).unwrap();
    assert_eq!(s.layout().pages[1].header_story, Some(id));
    assert_eq!(s.run("insert.editHeader", &json!({"page": 1})).unwrap()["story"].as_u64(), Some(id as u64));
    assert_eq!(s.doc.sections()[0].1.headers.even, None, "nothing made");
}

#[test]
fn base64_round_trip() {
    let data = b"hello world!?";
    assert_eq!(base64_decode(&base64_encode(data)).unwrap(), data);
    assert!(base64_decode("!!!").is_none());
}

/// Every bookmark mark in the body, in order: (paragraph, start or end, name).
fn bookmark_marks(s: &Session) -> Vec<(usize, bool, String)> {
    let mut out = Vec::new();
    for (i, path) in s.doc.para_paths(StoryRef::Body).iter().enumerate() {
        let p = s.doc.para(StoryRef::Body, path).unwrap();
        for off in p.object_offsets() {
            match p.object_at(off) {
                Some(InlineObject::BookmarkStart { name }) => out.push((i, true, name.clone())),
                Some(InlineObject::BookmarkEnd { name }) => out.push((i, false, name.clone())),
                _ => {}
            }
        }
    }
    out
}

/// Insert › Bookmark › Delete (#1236): a bookmark across two paragraphs loses both its marks,
/// the text and the selection stay where they were, and one undo brings it back. Go To
/// selects the bookmark's text, as in Word.
#[test]
fn deleting_a_bookmark_removes_its_start_and_end() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Alpha beta\ngamma delta"));
    s.sel = Selection { anchor: Pos::body(0, 6), focus: Pos::body(1, 5) };
    s.run("insert.bookmark", &json!({"name": "Across"})).unwrap();
    assert_eq!(bookmark_marks(&s), vec![(0, true, "Across".into()), (1, false, "Across".into())]);
    s.run("edit.goto", &json!({"bookmark": "Across"})).unwrap();
    let picked = s.selected_text();
    assert!(picked.starts_with("beta") && picked.ends_with("gamma"), "{picked:?}");
    s.run("select.text", &json!({"text": "delta"})).unwrap();
    s.run("insert.deleteBookmark", &json!({"name": "Across"})).unwrap();
    assert!(bookmark_marks(&s).is_empty(), "{:?}", bookmark_marks(&s));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Alpha beta\ngamma delta");
    assert_eq!(s.selected_text(), "delta", "the selection stays on its text");
    assert!(s.run("insert.deleteBookmark", &json!({"name": "Across"})).is_err(), "gone");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(bookmark_marks(&s).len(), 2, "one undo step");
}

/// Adding a bookmark under a name in use moves it, and the old end goes too — it was only
/// looked for in the start's paragraph, so a bookmark across paragraphs left a stray end.
#[test]
fn adding_a_bookmark_again_moves_all_of_it() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("Alpha beta\ngamma delta"));
    s.sel = Selection { anchor: Pos::body(0, 6), focus: Pos::body(1, 5) };
    s.run("insert.bookmark", &json!({"name": "Spot"})).unwrap();
    s.run("select.text", &json!({"text": "delta"})).unwrap();
    s.run("insert.bookmark", &json!({"name": "Spot"})).unwrap();
    assert_eq!(bookmark_marks(&s), vec![(1, true, "Spot".into()), (1, false, "Spot".into())]);
    assert_eq!(crate::cmd::references::bookmark_text(&s.doc, "Spot").as_deref(), Some("delta"));
}

/// #3491: Bookmark names are validated by Unicode character count and character classes.
#[test]
fn bookmark_name_validation_rules() {
    // Valid names:
    assert!(bookmark_name_ok("Mark1"));
    assert!(bookmark_name_ok("A"));
    assert!(bookmark_name_ok("Alpha_Beta_123"));
    // Valid leading underscore (Word hidden bookmarks, e.g. _Toc, _Ref, _Hidden):
    assert!(bookmark_name_ok("_Hidden"));
    assert!(bookmark_name_ok("_foo"));
    // Valid 27-character Unicode Cyrillic name (51 UTF-8 bytes):
    assert!(bookmark_name_ok("быстрый_доступ_к_документам"));
    // Valid 40-character name:
    let exactly_40 = "a".repeat(40);
    assert!(bookmark_name_ok(&exactly_40));
    let exactly_40_unicode = "я".repeat(40);
    assert!(bookmark_name_ok(&exactly_40_unicode));

    // Invalid: empty
    assert!(!bookmark_name_ok(""));
    // Invalid: leading digit
    assert!(!bookmark_name_ok("1mark"));
    // Invalid: symbols / punctuation
    assert!(!bookmark_name_ok("a&b"));
    assert!(!bookmark_name_ok("a!b"));
    assert!(!bookmark_name_ok("a-b"));
    assert!(!bookmark_name_ok("a.b"));
    assert!(!bookmark_name_ok("foo bar"));
    // Invalid: more than 40 characters
    assert!(!bookmark_name_ok(&"a".repeat(41)));
    assert!(!bookmark_name_ok(&"я".repeat(41)));

    // Verification through `insert.bookmark` command
    let mut s = Session::new(wordcraft_doc::Document::from_text("Test content"));
    assert!(s.run("insert.bookmark", &json!({"name": "быстрый_доступ_к_документам"})).is_ok());
    assert!(s.run("insert.bookmark", &json!({"name": "_Hidden"})).is_ok());
    assert!(s.run("insert.bookmark", &json!({"name": "1mark"})).is_err());
    assert!(s.run("insert.bookmark", &json!({"name": "a&b"})).is_err());
    assert!(s.run("insert.bookmark", &json!({"name": "a!b"})).is_err());
}

/// Insert › Symbol with a font: only the symbol takes it, the caret's font carries on after.
#[test]
fn a_mixed_symbol_string_keeps_symbols_and_text_apart() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("a"));
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.symbol", &json!({"char": "\u{F041}A", "font": "My Custom Font"})).unwrap();
    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(p.props_of_char(1).sym_font.as_deref(), Some("My Custom Font"), "the private-use glyph");
    assert_eq!(p.props_of_char(4).sym_font, None, "the plain letter");
    assert_eq!(p.props_of_char(4).font.as_deref(), Some("My Custom Font"));
}

#[test]
fn an_astral_symbol_stays_text_in_its_font() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("a"));
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.symbol", &json!({"char": "\u{1F600}", "font": "Wingdings"})).unwrap();
    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(p.props_of_char(1).sym_font, None, "w:sym can't hold it");
    assert_eq!(p.props_of_char(1).font.as_deref(), Some("Wingdings"));
}

#[test]
fn symbol_takes_its_own_font() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("ab"));
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.symbol", &json!({"char": "\u{2665}", "font": "DejaVu Sans"})).unwrap();
    s.run("text.insert", &json!({"text": "c"})).unwrap();
    let p = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(p.text, "ab\u{2665}c");
    assert_eq!(p.props_of_char(2).font.as_deref(), Some("DejaVu Sans"), "the symbol");
    assert_eq!(p.props_of_char(5).font, None, "typing after it (byte offsets)");
    assert!(s.run("insert.symbol", &json!({"char": ""})).is_err());
}

#[test]
fn date_pictures() {
    // Word accepts either case for the day and year instructions.
    assert_eq!(format_date_parts("MM/DD/YYYY", 2026, 10, 21, (17, 27, 3), 0), "10/21/2026");
    assert_eq!(format_date_parts("D MMMM YYYY", 2026, 10, 21, (17, 27, 3), 0), "21 October 2026");
    // Both 12-hour instructions pad a single-digit hour; only the 12-hour one wraps at noon.
    assert_eq!(format_date_parts("hh:mm", 2026, 10, 21, (17, 27, 3), 0), "05:27");
    assert_eq!(format_date_parts("h:mm", 2026, 10, 21, (17, 27, 3), 0), "5:27");
    assert_eq!(format_date_parts("HH:mm", 2026, 10, 21, (5, 7, 3), 0), "05:07");
    let d = format_date("yyyy-MM-dd");
    assert_eq!(d.len(), 10);
    assert!(format_date("MMMM d, yyyy").contains(", "));
    let t = format_date("h:mm:ss am/pm");
    assert!(t.ends_with(" AM") || t.ends_with(" PM"), "{t}");
    assert_eq!(t.matches(':').count(), 2, "{t}");
    use super::super::references::date_picture;
    assert_eq!(date_picture(r#"DATE \@ "MMMM d, yyyy" \* MERGEFORMAT"#).as_deref(), Some("MMMM d, yyyy"));
    assert_eq!(date_picture(r#"TIME \@ HH:mm"#).as_deref(), Some("HH:mm"));
    assert_eq!(date_picture("DATE"), None);
}

/// #4847 & #3639: Unlinking a header in Section 2 allows it to be edited independently
/// without altering Section 1's header; re-linking restores inheritance from Section 1.
/// Also verifies that commands executed while in a header story correctly target Section 2
/// instead of corrupting Section 1 (#3639).
#[test]
fn link_to_previous_unlinks_and_relinks_across_sections() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Section 1 text"})).unwrap();
    s.run("insert.header", &json!({"text": "Header 1"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();

    // Insert section break (next page)
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Section 2 text"})).unwrap();

    assert_eq!(s.doc.sections().len(), 2);
    // Section 1 cannot link to previous
    s.run("caret.docStart", &json!({})).unwrap();
    assert!(s.run("layout.linkToPrevious", &json!({})).is_err());

    // Section 2 starts out linked to Section 1
    s.run("caret.docEnd", &json!({})).unwrap();
    assert!(crate::cmd::page::is_hf_linked(&s));

    // Edit header on page 1 (which is Section 2)
    s.run("insert.editHeader", &json!({"page": 1})).unwrap();
    assert_eq!(crate::cmd::page::section_index_for_hf(&s), 1);

    // #3639: hf.position from Section 2's header modifies Section 2, not Section 1
    s.run("hf.position", &json!({"header": 45.0})).unwrap();
    assert_ne!(s.doc.sections()[0].1.header, 45.0, "section 1 header distance must not change");
    assert_eq!(s.doc.last_section.header, 45.0, "section 2 header distance changed");

    // Unlink Section 2's header
    let res = s.run("layout.linkToPrevious", &json!({"linked": false})).unwrap();
    assert_eq!(res["linked"], false);
    assert!(!crate::cmd::page::is_hf_linked(&s));

    // Modify Section 2's header text
    s.run("select.all", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Header 2 Independent"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();

    // Section 1's header should still be "Header 1"
    let s1_header_id = s.doc.sections()[0].1.headers.default.unwrap();
    let s1_text = s.doc.plain_text(StoryRef::Part(s1_header_id));
    assert!(s1_text.contains("Header 1"), "section 1 header was preserved: {s1_text}");

    // Section 2's header is "Header 2 Independent"
    let s2_header_id = s.doc.last_section.headers.default.unwrap();
    assert_ne!(s1_header_id, s2_header_id);
    let s2_text = s.doc.plain_text(StoryRef::Part(s2_header_id));
    assert!(s2_text.contains("Header 2 Independent"), "section 2 header changed: {s2_text}");

    // Now re-link Section 2's header to previous
    s.run("insert.editHeader", &json!({"page": 1})).unwrap();
    let res = s.run("layout.linkToPrevious", &json!({"linked": true})).unwrap();
    assert_eq!(res["linked"], true);
    assert!(crate::cmd::page::is_hf_linked(&s));
    assert_eq!(s.doc.last_section.headers.default, None);
}

#[test]
fn insert_header_in_linked_section_preserves_link_and_targets_inherited() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Section 1"})).unwrap();
    s.run("insert.header", &json!({"text": "Header 1"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();

    // Section break to create Section 2
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Section 2"})).unwrap();
    assert!(crate::cmd::page::is_hf_linked(&s));
    assert_eq!(s.doc.last_section.headers.default, None);

    // Running insert.header while in Section 2 should resolve the inherited header, not unlink Section 2
    s.run("insert.header", &json!({"text": "Header 1 Updated"})).unwrap();
    assert!(crate::cmd::page::is_hf_linked(&s));
    assert_eq!(s.doc.last_section.headers.default, None, "Section 2 must not be silently unlinked");

    let s1_header_id = s.doc.sections()[0].1.headers.default.unwrap();
    let s1_text = s.doc.plain_text(StoryRef::Part(s1_header_id));
    assert!(s1_text.contains("Header 1 Updated"));
}

#[test]
fn relinking_when_previous_has_no_header_moves_caret_to_body() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Section 1 without header"})).unwrap();

    // Section break to Section 2 (neither section has a header yet)
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Section 2"})).unwrap();

    // Unlink Section 2 and give it a header
    s.run("layout.linkToPrevious", &json!({"linked": false})).unwrap();
    assert!(!crate::cmd::page::is_hf_linked(&s));
    s.run("insert.editHeader", &json!({})).unwrap();
    let s2_id = s.doc.last_section.headers.default.unwrap();
    assert!(matches!(s.sel.focus.story, StoryRef::Part(id) if id == s2_id));

    // Re-link Section 2 to previous: previous has no header, so caret moves back to Body
    let res = s.run("layout.linkToPrevious", &json!({"linked": true})).unwrap();
    assert_eq!(res["linked"], true);
    assert!(crate::cmd::page::is_hf_linked(&s));
    assert_eq!(s.doc.last_section.headers.default, None);
    assert_eq!(s.sel.focus.story, StoryRef::Body, "caret must not be orphaned in cleared part");
}

#[test]
fn link_to_previous_is_idempotent_when_explicitly_setting_same_state() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Section 1"})).unwrap();
    s.run("insert.header", &json!({"text": "Header 1"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();

    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Section 2"})).unwrap();

    // Explicit linked: true when already linked is a no-op
    let res = s.run("layout.linkToPrevious", &json!({"linked": true})).unwrap();
    assert_eq!(res["linked"], true);
    assert!(crate::cmd::page::is_hf_linked(&s));

    // Unlink Section 2
    s.run("layout.linkToPrevious", &json!({"linked": false})).unwrap();
    s.run("insert.editHeader", &json!({})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Custom Content"})).unwrap();
    let original_part = s.doc.last_section.headers.default.unwrap();

    // Explicit linked: false again must not overwrite existing independent content
    let res = s.run("layout.linkToPrevious", &json!({"linked": false})).unwrap();
    assert_eq!(res["linked"], false);
    let after_part = s.doc.last_section.headers.default.unwrap();
    assert_eq!(original_part, after_part);
    let text = s.doc.plain_text(StoryRef::Part(after_part));
    assert!(text.contains("Custom Content"));
}

#[test]
fn edit_header_with_invalid_page_preserves_page_hint() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.page_hint = 0;
    assert!(s.run("insert.editHeader", &json!({"page": 999})).is_err());
    assert_eq!(s.page_hint, 0, "page_hint must not be modified when page lookup fails");
}

#[test]
fn hf_nav_synchronizes_page_hint() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Section 1"})).unwrap();
    s.run("insert.header", &json!({"text": "Header 1"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();

    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Section 2"})).unwrap();
    s.run("layout.linkToPrevious", &json!({"linked": false})).unwrap();
    s.run("insert.editHeader", &json!({"page": 1})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Header 2"})).unwrap();

    // Start in Header 1 (page 0)
    s.run("insert.editHeader", &json!({"page": 0})).unwrap();
    assert_eq!(s.page_hint, 0);

    // Navigate to next header
    s.run("hf.next", &json!({})).unwrap();
    assert_eq!(s.page_hint, 1, "page_hint must synchronize with destination page in hf_nav");
}

#[test]
fn even_page_without_even_header_reuses_default() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("layout.differentOddEven", &json!({"value": true})).unwrap();
    s.run("text.insert", &json!({"text": "Page 1"})).unwrap();
    s.run("insert.header", &json!({"text": "Default Header"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();

    // Page break to Page 2 (even page)
    s.run("insert.pageBreak", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Page 2"})).unwrap();

    // hf_part for even page should resolve to default header when even header is unset
    let part_id = super::header_footer::hf_part(&mut s, true);
    assert_eq!(Some(part_id), s.doc.last_section.headers.default);
    assert_eq!(s.doc.last_section.headers.even, None);
}

#[test]
fn is_hf_linked_uses_caret_page_slot() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Section 1"})).unwrap();
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Section 2"})).unwrap();

    // Enable Different First Page on Section 2
    s.run("layout.differentFirstPage", &json!({"value": true})).unwrap();

    // Unlink default header on Section 2
    s.doc.last_section.headers.default = Some(s.doc.add_part(wordcraft_doc::PartKind::Header, vec![]));
    s.doc.last_section.headers.first = None; // First page is still linked

    // Caret is on Section 2's first page (which is page 1)
    s.run("caret.docEnd", &json!({})).unwrap();
    s.layout();
    assert!(crate::cmd::page::is_hf_linked(&s), "is_hf_linked must check first-page slot for caret on first page");
}

#[test]
fn even_page_resolves_inherited_even_header_before_default() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("layout.differentOddEven", &json!({"value": true})).unwrap();
    s.run("text.insert", &json!({"text": "Sec 1 Page 1"})).unwrap();
    s.run("insert.header", &json!({"text": "S1 Default"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();

    // S1 Page 2 (even) has its own even header
    s.run("insert.pageBreak", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Sec 1 Page 2"})).unwrap();
    let s1_even_id = s.doc.add_part(wordcraft_doc::PartKind::Header, vec![]);
    s.doc.section_mut_at(0).headers.even = Some(s1_even_id);

    // Section 2: unlinked default header, but even header linked (None)
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Sec 2 Page 3"})).unwrap();
    s.run("layout.linkToPrevious", &json!({"linked": false})).unwrap();
    s.run("insert.editHeader", &json!({"page": 2})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "S2 Default"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();

    // Section 2 Page 4 (even page in section 2)
    s.run("insert.pageBreak", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "Sec 2 Page 4"})).unwrap();

    assert_eq!(s.doc.last_section.headers.even, None);

    s.page_hint = 3;
    let part_id = super::header_footer::hf_part(&mut s, true);
    assert_eq!(part_id, s1_even_id, "even page must resolve inherited even header instead of own default");
}

#[test]
fn hf_nav_preserves_page_hint_for_inherited_part() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "Section 1"})).unwrap();
    s.run("insert.header", &json!({"text": "Header 1"})).unwrap();
    s.run("insert.footer", &json!({"text": "Footer 1"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();

    // Section 2 (inherits Header 1 and Footer 1)
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("text.insert", &json!({"text": "Section 2"})).unwrap();

    let footer_id = s.doc.sections()[0].1.footers.default.unwrap();
    s.page_hint = 1;
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Part(footer_id)));

    // Navigate to previous part (which cycles to Header 1)
    s.run("hf.previous", &json!({})).unwrap();
    assert_eq!(s.page_hint, 1, "page_hint must be preserved on Section 2 when moving between inherited parts");
}
