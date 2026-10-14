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

/// #1707: Insert ▸ Date & Time and DATE/TIME fields use the local civil date, time and
/// weekday, not UTC's.
#[test]
fn local_civil_date_follows_the_utc_offset() {
    let pic = "dddd d MMMM yyyy HH:mm";
    let show = |unix: u64, off: i64| {
        let (y, m, d, hms, days) = crate::cmd::local_civil(unix, off);
        format_date_parts(pic, y, m as usize, d, hms, days)
    };
    // 2026-10-30T23:00:00Z in New Zealand (UTC+13) is Saturday 31 October, noon.
    assert_eq!(show(1_793_401_200, 13 * 3600), "Saturday 31 October 2026 12:00");
    // 2026-11-01T02:00:00Z at UTC-5 is still Saturday 31 October, 21:00.
    assert_eq!(show(1_793_498_400, -5 * 3600), "Saturday 31 October 2026 21:00");
    // Offset 0 is UTC.
    assert_eq!(show(1_793_401_200, 0), "Friday 30 October 2026 23:00");
    // Near the epoch with a negative offset: the day before 1970, no overflow.
    assert_eq!(show(0, -3600), "Wednesday 31 December 1969 23:00");
    // Hostile values never panic.
    let _ = crate::cmd::local_civil(u64::MAX, i64::MAX);
    let _ = crate::cmd::local_civil(u64::MAX, i64::MIN);
    let _ = crate::cmd::local_civil(0, i64::MIN);
    // The machine's zone is a real offset (within ±26 h) and asking never panics.
    for unix in [0, 1_793_401_200, u64::MAX] {
        assert!(crate::cmd::local_offset_secs(unix).abs() <= 26 * 3600);
    }
}
