use super::*;
use wordcraft_doc::Pos;

/// A document of `pages` pages, two paragraphs each (the first one starts its page through Page
/// Break Before), with a centred page number in the footer.
fn book(pages: usize) -> Session {
    let text: Vec<String> = (1..=pages).flat_map(|n| [format!("Page {n} first paragraph"), format!("Page {n} second paragraph")]).collect();
    let mut s = Session::new(Document::from_text(&text.join("\n")));
    for n in 1..pages {
        s.doc.para_mut(StoryRef::Body, &Path::top(n * 2)).unwrap().props.page_break_before = Some(true);
    }
    s.touch();
    s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
    s.run("insert.footer", &json!({"preset": "pageNumber"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(s.layout().pages.len(), pages);
    s
}

fn shown(s: &mut Session) -> Vec<bool> {
    let l = s.layout();
    l.pages.iter().map(|pg| footer_visible(&s.doc, pg)).collect()
}

fn numbers(s: &mut Session) -> Vec<u32> {
    s.layout().pages.iter().map(|pg| pg.number).collect()
}

fn hide(s: &mut Session, page: usize) -> CmdResult {
    s.run("layout.pageFooterVisible", &json!({"page": page, "visible": false}))
}

fn show(s: &mut Session, page: usize) -> CmdResult {
    s.run("layout.pageFooterVisible", &json!({"page": page, "visible": true}))
}

fn only(n: usize, pages: usize) -> Vec<bool> {
    (1..=pages).map(|i| i != n).collect()
}

#[test]
fn hide_a_middle_page_leaves_every_other_page_alone() {
    let mut s = book(5);
    let before = s.doc.clone();
    let r = hide(&mut s, 3).unwrap();
    assert_eq!((r["page"].as_u64(), r["visible"].as_bool(), r["changed"].as_bool()), (Some(3), Some(false), Some(true)));
    assert_eq!(shown(&mut s), only(3, 5));
    assert_eq!(numbers(&mut s), vec![1, 2, 3, 4, 5], "numbering goes on");
    // The page is a section of its own: breaks on the last paragraph before it and on its last.
    let secs = s.doc.sections();
    assert_eq!(secs.iter().map(|(e, _)| *e).collect::<Vec<_>>(), vec![3, 5, 9]);
    assert!(secs[1].1.footers.default.is_some() && secs[1].1.page_num_start.is_none());
    assert_eq!(secs[0].1.start, SectionStart::NextPage);
    // One undo step puts everything back.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc, before);
    assert_eq!(shown(&mut s), vec![true; 5]);
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(shown(&mut s), only(3, 5));
}

#[test]
fn hiding_twice_adds_no_section_breaks_and_showing_restores_the_footer() {
    let mut s = book(5);
    hide(&mut s, 3).unwrap();
    let sections = s.doc.sections().len();
    assert_eq!(hide(&mut s, 3).unwrap()["changed"], false);
    assert_eq!(s.doc.sections().len(), sections);
    // Shown again, hidden again: the page stays a section of its own, no more breaks.
    let r = show(&mut s, 3).unwrap();
    assert_eq!(r["changed"], true);
    assert_eq!(shown(&mut s), vec![true; 5]);
    assert_eq!(numbers(&mut s), vec![1, 2, 3, 4, 5]);
    let parts = s.doc.parts.len();
    hide(&mut s, 3).unwrap();
    show(&mut s, 3).unwrap();
    assert_eq!(s.doc.sections().len(), sections, "still the same breaks");
    assert_eq!(s.doc.parts.len(), parts, "no footers pile up");
    // Another page next to it needs its own breaks only.
    hide(&mut s, 4).unwrap();
    assert_eq!(shown(&mut s), only(4, 5));
}

#[test]
fn toggle_flips_the_footer_only_when_asked_for() {
    let mut s = book(4);
    s.run("layout.pageFooterVisible", &json!({"page": 2, "toggle": true})).unwrap();
    assert_eq!(shown(&mut s), only(2, 4));
    s.run("layout.pageFooterVisible", &json!({"page": 2, "toggle": true})).unwrap();
    assert_eq!(shown(&mut s), vec![true; 4]);
    // The caret's page when none is given.
    s.run("select.text", &json!({"text": "Page 3 second"})).unwrap();
    s.run("layout.pageFooterVisible", &json!({"toggle": true})).unwrap();
    assert_eq!(shown(&mut s), only(3, 4));
}

#[test]
fn without_visible_or_toggle_nothing_is_guessed() {
    let mut s = book(3);
    let before = s.doc.clone();
    let cases = [
        json!({"page": 2}),
        json!({}),
        json!({"page": 2, "visible": null}),
        json!({"page": 2, "toggle": false}),
        json!({"page": 2, "visible": true, "toggle": true}),
        json!({"page": 2, "visible": false, "toggle": true}),
        json!({"page": 2, "toggle": "true"}),
        json!({"page": 2, "toggle": 1}),
    ];
    for args in cases {
        let e = s.run("layout.pageFooterVisible", &args).unwrap_err();
        assert!(matches!(e, CmdError::Params(_)), "{args}: {e:?}");
        assert_eq!(s.doc, before, "{args}");
    }
    // An explicit value is idempotent: the same call twice ends in the same state.
    hide(&mut s, 2).unwrap();
    assert_eq!(hide(&mut s, 2).unwrap()["changed"], false);
    assert_eq!(shown(&mut s), only(2, 3));
    // toggle: false next to an explicit visible is just the explicit call.
    s.run("layout.pageFooterVisible", &json!({"page": 2, "visible": true, "toggle": false})).unwrap();
    assert_eq!(shown(&mut s), vec![true; 3]);
}

#[test]
fn the_last_and_the_one_before_it() {
    let mut s = book(4);
    hide(&mut s, 4).unwrap();
    assert_eq!(shown(&mut s), only(4, 4));
    hide(&mut s, 3).unwrap();
    assert_eq!(shown(&mut s), vec![true, true, false, false]);
    show(&mut s, 3).unwrap();
    assert_eq!(shown(&mut s), only(4, 4));
    show(&mut s, 4).unwrap();
    assert_eq!(shown(&mut s), vec![true; 4]);
    assert_eq!(numbers(&mut s), vec![1, 2, 3, 4]);
}

#[test]
fn the_first_page_uses_different_first_page() {
    let mut s = book(3);
    hide(&mut s, 1).unwrap();
    assert_eq!(shown(&mut s), vec![false, true, true]);
    assert_eq!(s.doc.sections().len(), 1, "no section break for the first page");
    assert!(s.doc.last_section.title_page);
    assert_eq!(numbers(&mut s), vec![1, 2, 3]);
    show(&mut s, 1).unwrap();
    assert_eq!(shown(&mut s), vec![true; 3]);
    assert!(!s.doc.last_section.title_page, "Different First Page goes off again");
    assert_eq!(s.doc.last_section.footers.first, None);
}

#[test]
fn a_header_stays_when_the_first_page_footer_is_hidden() {
    let mut s = book(3);
    s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
    s.run("insert.header", &json!({"text": "Running head"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    let l = s.layout();
    assert!(l.pages.iter().all(|pg| header_visible(&s.doc, pg)));
    hide(&mut s, 1).unwrap();
    let l = s.layout();
    assert!(l.pages.iter().all(|pg| header_visible(&s.doc, pg)), "headers are not touched");
    assert_eq!(shown(&mut s), vec![false, true, true]);
}

#[test]
fn a_page_that_starts_or_ends_inside_a_paragraph_is_refused_and_nothing_changes() {
    let long = "word ".repeat(2500);
    let mut s = Session::new(Document::from_text(&format!("Intro\n{long}\nEnd")));
    s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
    s.run("insert.footer", &json!({"preset": "pageNumber"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    let pages = s.layout().pages.len();
    assert!(pages >= 3, "{pages} pages");
    let before = s.doc.clone();
    let err = hide(&mut s, 2).unwrap_err();
    assert!(matches!(err, CmdError::Disabled(_)), "{err:?}");
    assert!(err.to_string().contains("middle of a paragraph"), "{err}");
    assert_eq!(s.doc, before);
    let info = page_footers(&mut s);
    assert_eq!(info.len(), pages);
    assert!(info[1].blocked.is_some() && info[0].blocked.is_none());
    assert_eq!(info[0].blocked_code, None);
    assert_eq!(info[1].blocked_code, Some(BlockedReason::StartsMidBlock));
    assert!(err.to_string().ends_with("[starts_mid_block]"), "{err}");
    let listed = s.run("layout.pageFooters", &json!({})).unwrap();
    assert_eq!(listed["pages"][1]["blockedCode"], "starts_mid_block");
    assert!(listed["pages"][0]["blockedCode"].is_null() && listed["pages"][0]["blocked"].is_null());
}

#[test]
fn bad_pages_are_params_errors() {
    let mut s = book(2);
    for page in [0u64, 3, 1_000_000] {
        let e = s.run("layout.pageFooterVisible", &json!({"page": page, "visible": false})).unwrap_err();
        assert!(matches!(e, CmdError::Params(_)), "{page}: {e:?}");
    }
}

#[test]
fn an_unreadable_page_is_an_error_and_never_means_the_caret_page() {
    let mut s = book(3);
    // The caret is on page 3, so a fallback would change page 3.
    s.run("select.text", &json!({"text": "Page 3 second"})).unwrap();
    let before = s.doc.clone();
    for page in [json!(-1), json!("2"), json!(true), json!(1.5), json!(2.8), json!(2.0), json!(null), json!([2]), json!({})] {
        for args in [json!({"page": page.clone(), "visible": false}), json!({"page": page.clone(), "toggle": true})] {
            let e = s.run("layout.pageFooterVisible", &args).unwrap_err();
            assert!(matches!(e, CmdError::Params(_)), "{page}: {e:?}");
            assert_eq!(s.doc, before, "{page}: a refused call changes nothing");
        }
    }
    assert_eq!(shown(&mut s), vec![true; 3]);
}

#[test]
fn pages_out_of_range_are_params_errors_and_change_nothing() {
    let mut s = book(3);
    s.run("select.text", &json!({"text": "Page 3 second"})).unwrap();
    let before = s.doc.clone();
    for page in [json!(0), json!(4), json!(u64::MAX), json!(1_000_000)] {
        let e = s.run("layout.pageFooterVisible", &json!({"page": page.clone(), "visible": false})).unwrap_err();
        assert!(matches!(e, CmdError::Params(_)), "{page}: {e:?}");
        assert!(e.to_string().contains("pages are numbered from 1 to 3"), "{e}");
        assert_eq!(s.doc, before);
    }
}

#[test]
fn resolve_page_covers_every_form_and_the_empty_document() {
    let caret = || Some(2);
    assert_eq!(resolve_page(None, 5, caret).unwrap(), (2, true));
    assert_eq!(resolve_page(Some(&json!(5)), 5, caret).unwrap(), (4, false));
    assert_eq!(resolve_page(None, 2, caret).unwrap(), (1, true), "a caret past the end is clamped");
    assert_eq!(resolve_page(None, 5, || None).unwrap(), (0, true));
    for count in [0, 3] {
        for bad in [json!(null), json!("1"), json!(-1), json!(0.5), json!(1.0)] {
            assert!(resolve_page(Some(&bad), count, caret).is_err(), "{bad} with {count} pages");
        }
    }
    // No pages at all: the error is the same with or without a page, and the caret is not asked.
    for page in [None, Some(json!(1)), Some(json!(0))] {
        let e = resolve_page(page.as_ref(), 0, || panic!("the caret must not be consulted")).unwrap_err();
        assert!(matches!(e, CmdError::Failed(_)) && e.to_string().contains("no pages"), "{e:?}");
    }
}

#[test]
fn the_result_names_the_page_its_number_and_where_it_came_from() {
    let mut s = book(6);
    // Two chapters: the last page displays 1 again.
    s.doc.para_mut(StoryRef::Body, &Path::top(10)).unwrap().props.page_break_before = None;
    s.sel = crate::Selection::caret(Pos::body(10, 0));
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("layout.pageNumberFormat", &json!({"start": 1})).unwrap();
    let r = s.run("layout.pageFooterVisible", &json!({"page": 6, "visible": false})).unwrap();
    assert_eq!((r["page"].as_u64(), r["number"].as_u64(), r["pageFrom"].as_str()), (Some(6), Some(1), Some("argument")));
    // No page: the caret's, and the result says so.
    s.run("select.text", &json!({"text": "Page 2 second"})).unwrap();
    let r = s.run("layout.pageFooterVisible", &json!({"visible": false})).unwrap();
    assert_eq!((r["page"].as_u64(), r["number"].as_u64(), r["pageFrom"].as_str()), (Some(2), Some(2), Some("caret")));
    // Even when nothing changes.
    let r = s.run("layout.pageFooterVisible", &json!({"visible": false})).unwrap();
    assert_eq!((r["changed"].as_bool(), r["pageFrom"].as_str()), (Some(false), Some("caret")));
}

#[test]
fn a_refusal_on_the_caret_page_names_that_page() {
    let long = "word ".repeat(2500);
    let mut s = Session::new(Document::from_text(&format!("Intro\n{long}\nEnd")));
    s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
    s.run("insert.footer", &json!({"preset": "pageNumber"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run("select.text", &json!({"text": "End"})).unwrap();
    let last = s.layout().pages.len();
    let before = s.doc.clone();
    let e = s.run("layout.pageFooterVisible", &json!({"visible": false})).unwrap_err();
    let text = e.to_string();
    assert!(text.contains(&format!("page {last} is the page the caret is on")), "{text}");
    assert!(text.contains('['), "{text}");
    assert_eq!(s.doc, before);
    // A page the caller named carries no such note.
    let e = s.run("layout.pageFooterVisible", &json!({"page": 2, "visible": false})).unwrap_err();
    assert!(!e.to_string().contains("the caret is on"), "{e}");
}

#[test]
fn a_page_without_a_footer_gets_the_neighbours_when_shown() {
    let mut s = book(3);
    s.run("insert.removeFooter", &json!({})).unwrap();
    assert_eq!(shown(&mut s), vec![false; 3]);
    show(&mut s, 2).unwrap();
    assert_eq!(shown(&mut s), vec![false, true, false]);
    assert_eq!(numbers(&mut s), vec![1, 2, 3]);
}

#[test]
fn restarted_numbering_and_other_sections_are_kept() {
    // Two chapters; the second restarts at 1.
    let mut s = book(6);
    s.doc.para_mut(StoryRef::Body, &Path::top(10)).unwrap().props.page_break_before = None;
    s.sel = crate::Selection::caret(Pos::body(10, 0));
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("layout.pageNumberFormat", &json!({"start": 1})).unwrap();
    let before_numbers = numbers(&mut s);
    assert_eq!(before_numbers, vec![1, 2, 3, 4, 5, 1]);
    hide(&mut s, 5).unwrap();
    assert_eq!(shown(&mut s), vec![true, true, true, true, false, true]);
    assert_eq!(numbers(&mut s), before_numbers);
    hide(&mut s, 2).unwrap();
    assert_eq!(numbers(&mut s), before_numbers);
    assert_eq!(shown(&mut s), vec![true, false, true, true, false, true]);
    // The last chapter is a section of its own with its own restart.
    assert_eq!(s.doc.last_section.page_num_start, Some(1));
}

#[test]
fn even_and_odd_pages_each_keep_their_footer() {
    let mut s = book(6);
    s.run("layout.differentOddEven", &json!({"value": true})).unwrap();
    hide(&mut s, 4).unwrap();
    assert_eq!(shown(&mut s), only(4, 6));
    hide(&mut s, 3).unwrap();
    assert_eq!(shown(&mut s), vec![true, true, false, false, true, true]);
    show(&mut s, 4).unwrap();
    assert_eq!(shown(&mut s), only(3, 6));
}

#[test]
fn survives_docx_and_odt() {
    let mut s = book(5);
    hide(&mut s, 3).unwrap();
    hide(&mut s, 1).unwrap();
    let want = vec![false, true, false, true, true];
    assert_eq!(shown(&mut s), want);
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let mut t = Session::new(back);
    assert_eq!(shown(&mut t), want, "docx");
    assert_eq!(numbers(&mut t), vec![1, 2, 3, 4, 5]);
    // ODT export does not write headers and footers at all yet (docs/file-format-parity.md, #863),
    // so only the structure is checked there: the same sections, pages and page numbers.
    let odt = crate::io_ext::save("odt", &s.doc, &crate::io::ExportOptions::default()).expect("odt is a save format").unwrap();
    let mut t = Session::new(crate::io::open_bytes("book.odt", &odt).unwrap());
    assert_eq!(t.doc.sections().len(), s.doc.sections().len(), "odt sections");
    assert_eq!(numbers(&mut t), vec![1, 2, 3, 4, 5], "odt pages");
}

#[test]
fn the_footer_map_lists_every_page() {
    let mut s = book(3);
    hide(&mut s, 2).unwrap();
    let map = page_footers(&mut s);
    assert_eq!(map.iter().map(|i| (i.page, i.footer)).collect::<Vec<_>>(), vec![(1, true), (2, false), (3, true)]);
    let r = s.run("layout.pageFooters", &json!({})).unwrap();
    assert_eq!(r["pages"].as_array().map(Vec::len), Some(3));
    assert_eq!(r["pages"][1]["footer"], false);
}

#[test]
fn page_break_paragraph_forms_are_fine_and_tables_at_the_edge_are_refused() {
    // Word's page breaks: a paragraph holding the break, or one ending with it.
    for text in ["A\u{000C}\nB\u{000C}\nC\u{000C}\nD", "A\n\u{000C}\nB\n\u{000C}\nC\n\u{000C}\nD"] {
        let mut s = Session::new(Document::from_text(text));
        s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
        s.run("insert.footer", &json!({"preset": "pageNumber"})).unwrap();
        s.run("insert.closeHeader", &json!({})).unwrap();
        assert!(page_footers(&mut s).iter().all(|i| i.blocked.is_none()), "{text:?}");
        hide(&mut s, 3).unwrap();
        assert_eq!(shown(&mut s), only(3, 4), "{text:?}");
        assert_eq!(numbers(&mut s), vec![1, 2, 3, 4]);
        // No blank page after a save in either format: the section break replaces the manual one.
        let t = Session::new(wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap());
        let mut t = t;
        assert_eq!(numbers(&mut t), vec![1, 2, 3, 4], "docx {text:?}");
        let odt = crate::io_ext::save("odt", &s.doc, &crate::io::ExportOptions::default()).expect("odt is a save format").unwrap();
        let mut t = Session::new(crate::io::open_bytes("book.odt", &odt).unwrap());
        assert_eq!(numbers(&mut t), vec![1, 2, 3, 4], "odt {text:?}");
    }
    // A table next to the page: no paragraph to hold a section break.
    let mut s = book(3);
    let table = wordcraft_doc::Table::new(1, 1, 400.0);
    s.doc.body.insert(4, std::sync::Arc::new(Block::Table(table)));
    s.touch();
    let before = s.doc.clone();
    let info = page_footers(&mut s);
    assert!(info[1].blocked.as_deref().is_some_and(|m| m.contains("table")), "{:?}", info[1].blocked);
    assert!(info[2].blocked.as_deref().is_some_and(|m| m.contains("table")), "{:?}", info[2].blocked);
    assert_eq!(info[1].blocked_code, Some(BlockedReason::EndsWithTable));
    assert_eq!(info[2].blocked_code, Some(BlockedReason::AfterTable));
    let e = hide(&mut s, 2).unwrap_err();
    assert!(matches!(e, CmdError::Disabled(_)) && e.to_string().contains("table"), "{e}");
    assert!(e.to_string().ends_with("[ends_with_table]"), "{e}");
    assert_eq!(s.doc, before, "nothing changed");
}

/// Page breaks typed inside one paragraph ("A⏎B⏎C"): a page that starts and ends right after
/// such a break is cut out of the paragraph as a page break followed by a new paragraph.
#[test]
fn pages_made_by_page_breaks_inside_one_paragraph_can_be_hidden() {
    let mut s = Session::new(Document::from_text("Apples\u{000C}Bananas\u{000C}Cherries\u{000C}Dates"));
    s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
    s.run("insert.footer", &json!({"preset": "pageNumber"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(s.doc.body.len(), 1);
    assert!(page_footers(&mut s).iter().all(|i| i.blocked.is_none()), "{:?}", page_footers(&mut s));
    hide(&mut s, 3).unwrap();
    assert_eq!(shown(&mut s), only(3, 4));
    assert_eq!(numbers(&mut s), vec![1, 2, 3, 4]);
    assert_eq!(s.doc.body.len(), 3, "cut before and after the page");
    let text: Vec<String> = (0..3).map(|i| s.doc.para(StoryRef::Body, &Path::top(i)).unwrap().text.clone()).collect();
    assert_eq!(text, ["Apples\u{000C}Bananas", "Cherries", "Dates"], "the section breaks replace the page breaks around Cherries");
    // The text is the same apart from that one page break, and every page keeps its words.
    let t = s.doc.plain_text(StoryRef::Body);
    for word in ["Apples", "Bananas", "Cherries", "Dates"] {
        assert!(t.contains(word));
    }
    show(&mut s, 3).unwrap();
    assert_eq!(shown(&mut s), vec![true; 4]);
    // Undone in one step.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(shown(&mut s), only(3, 4));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.body.len(), 1);
    assert_eq!(shown(&mut s), vec![true; 4]);
}

#[test]
fn a_heading_or_list_item_is_not_cut_in_two() {
    for style in ["Heading1", "ListParagraph"] {
        let mut s = Session::new(Document::from_text("Apples\u{000C}Bananas\u{000C}Cherries"));
        s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.style = Some(style.into());
        if style == "ListParagraph" {
            s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: 1, level: 0 });
        }
        s.touch();
        s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
        s.run("insert.footer", &json!({"preset": "pageNumber"})).unwrap();
        s.run("insert.closeHeader", &json!({})).unwrap();
        let before = s.doc.clone();
        let e = hide(&mut s, 2).unwrap_err();
        assert!(matches!(e, CmdError::Disabled(_)), "{style}: {e:?}");
        assert_eq!(s.doc, before, "{style}");
    }
}

/// A document whose first page already has its own first-page footer (Different First Page).
fn book_with_own_first_footer(pages: usize) -> Session {
    let mut s = book(pages);
    let own = vec![para_block(Paragraph::with_text("Own first page footer", Default::default()).styled("Footer"))];
    let id = s.doc.try_add_part(PartKind::Footer, own).unwrap();
    s.doc.last_section.title_page = true;
    s.doc.last_section.footers.first = Some(id);
    s.touch();
    s
}

fn first_page_footer(s: &mut Session) -> Option<String> {
    let l = s.layout();
    let blocks = l.pages.first().and_then(|pg| shown_blocks(&s.doc, pg.footer_story))?;
    Some(blocks.iter().filter_map(|b| if let Block::Para(p) = &**b { Some(p.text.clone()) } else { None }).collect::<Vec<_>>().join("\n"))
}

#[test]
fn showing_the_first_page_again_brings_back_its_own_footer() {
    let mut s = book_with_own_first_footer(3);
    let own = Some("Own first page footer".to_string());
    assert_eq!(first_page_footer(&mut s), own);
    hide(&mut s, 1).unwrap();
    assert_eq!(shown(&mut s), vec![false, true, true]);
    show(&mut s, 1).unwrap();
    assert_eq!(shown(&mut s), vec![true; 3]);
    assert_eq!(first_page_footer(&mut s), own, "the page shows its own footer, not a neighbour's");
}

#[test]
fn a_visible_that_is_not_a_boolean_is_a_params_error_and_changes_nothing() {
    let mut s = book(3);
    let before = s.doc.clone();
    for visible in [json!("false"), json!("yes"), json!(0), json!(1), json!(2.5), json!([]), json!({}), json!([true])] {
        let e = s.run("layout.pageFooterVisible", &json!({"page": 2, "visible": visible.clone()})).unwrap_err();
        assert!(matches!(e, CmdError::Params(_)), "{visible}: {e:?}");
        assert_eq!(s.doc, before, "{visible}");
    }
    assert_eq!(shown(&mut s), vec![true; 3]);
}

#[test]
fn blocked_codes_are_stable_strings() {
    let all = [
        (BlockedReason::StartsMidBlock, "starts_mid_block"),
        (BlockedReason::EndsMidBlock, "ends_mid_block"),
        (BlockedReason::ContinuousBreak, "continuous_break"),
        (BlockedReason::AfterTable, "after_table"),
        (BlockedReason::EndsWithTable, "ends_with_table"),
        (BlockedReason::NoText, "no_text"),
    ];
    for (reason, code) in all {
        assert_eq!(reason.code(), code);
    }
}

/// `Intro`, a long paragraph that starts its own page and ends in the middle of a page, `End`.
fn long_book() -> Session {
    let long = "word ".repeat(2500);
    let mut s = Session::new(Document::from_text(&format!("Intro\n{long}\nEnd")));
    s.doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.page_break_before = Some(true);
    s.touch();
    s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
    s.run("insert.footer", &json!({"preset": "pageNumber"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    s
}

#[test]
fn the_blocked_reasons_in_the_listing_match_what_the_pages_do() {
    let mut s = long_book();
    let map = page_footers(&mut s);
    assert!(map.len() >= 4, "{} pages", map.len());
    assert_eq!(map[0].blocked_code, None, "the first page of a section is never blocked");
    assert_eq!(map[1].blocked_code, Some(BlockedReason::EndsMidBlock), "starts its paragraph but runs on");
    let codes: Vec<_> = map.iter().map(|i| i.blocked_code).collect();
    assert!(codes.contains(&Some(BlockedReason::EndsMidBlock)), "{codes:?}");
    assert!(codes.contains(&Some(BlockedReason::StartsMidBlock)), "{codes:?}");
    for info in &map {
        assert_eq!(info.blocked.is_some(), info.blocked_code.is_some(), "text and code come together: {info:?}");
        // A blocked page is refused by the command with the same code.
        if let Some(code) = info.blocked_code {
            let e = hide(&mut s, info.page).unwrap_err();
            assert!(e.to_string().ends_with(&format!("[{}]", code.code())), "{e}");
        }
    }
}

#[test]
fn a_continuous_break_inside_a_page_blocks_it_with_its_own_code() {
    let mut s = book(3);
    s.sel = crate::Selection::caret(Pos::body(3, 0));
    s.run("layout.break", &json!({"kind": "continuous"})).unwrap();
    let before = s.doc.clone();
    let map = page_footers(&mut s);
    assert_eq!(map[1].blocked_code, Some(BlockedReason::ContinuousBreak), "{map:?}");
    let e = hide(&mut s, 2).unwrap_err();
    assert!(matches!(e, CmdError::Disabled(_)) && e.to_string().ends_with("[continuous_break]"), "{e}");
    assert_eq!(s.doc, before);
}

#[test]
fn page_footer_info_is_the_listing_entry_of_one_page() {
    let mut s = book(4);
    hide(&mut s, 2).unwrap();
    let map = page_footers(&mut s);
    for (i, want) in map.iter().enumerate() {
        assert_eq!(page_footer_info(&mut s, i).as_ref(), Some(want), "page {}", i + 1);
    }
    assert_eq!(page_footer_info(&mut s, 1).map(|i| (i.page, i.number, i.footer)), Some((2, 2, false)));
    assert_eq!(page_footer_info(&mut s, 4), None, "past the last page");
    assert_eq!(page_footer_info(&mut s, usize::MAX), None);
}

#[test]
fn the_listing_shows_headers_numbers_and_the_footer_of_every_page() {
    let mut s = book(6);
    s.sel = crate::Selection::caret(s.doc.start_of(StoryRef::Body));
    s.run("insert.header", &json!({"text": "Running head"})).unwrap();
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.doc.para_mut(StoryRef::Body, &Path::top(10)).unwrap().props.page_break_before = None;
    s.sel = crate::Selection::caret(Pos::body(10, 0));
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    s.run("layout.pageNumberFormat", &json!({"start": 1})).unwrap();
    hide(&mut s, 3).unwrap();
    let map = page_footers(&mut s);
    assert_eq!(map.iter().map(|i| i.page).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(map.iter().map(|i| i.number).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5, 1], "page and number differ after a restart");
    assert_eq!(map.iter().map(|i| i.footer).collect::<Vec<_>>(), vec![true, true, false, true, true, true]);
    assert!(map.iter().all(|i| i.header), "headers are listed too");
}

#[test]
fn hiding_a_first_page_with_its_own_footer_keeps_it_aside_and_undo_restores_the_file() {
    let mut s = book_with_own_first_footer(3);
    let before = s.doc.clone();
    hide(&mut s, 1).unwrap();
    assert_eq!(first_page_footer(&mut s), None, "hidden");
    // The own footer is not lost: showing the page puts it back, and a second round does too.
    show(&mut s, 1).unwrap();
    assert_eq!(first_page_footer(&mut s).as_deref(), Some("Own first page footer"));
    let parts = s.doc.parts.len();
    hide(&mut s, 1).unwrap();
    show(&mut s, 1).unwrap();
    assert_eq!(first_page_footer(&mut s).as_deref(), Some("Own first page footer"));
    assert_eq!(s.doc.parts.len(), parts, "no footers pile up");
    // Hidden, then undone: the document is exactly as it was.
    hide(&mut s, 1).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(first_page_footer(&mut s).as_deref(), Some("Own first page footer"));
    drop(before);
}

#[test]
fn another_section_keeps_its_first_page_footer_when_the_first_page_is_hidden() {
    // Two chapters, each with Different First Page and its own first-page footer text.
    let mut s = book_with_own_first_footer(6);
    s.doc.para_mut(StoryRef::Body, &Path::top(10)).unwrap().props.page_break_before = None;
    s.sel = crate::Selection::caret(Pos::body(10, 0));
    s.run("layout.break", &json!({"kind": "nextPage"})).unwrap();
    let second = vec![para_block(Paragraph::with_text("Chapter two opener", Default::default()).styled("Footer"))];
    let id = s.doc.try_add_part(PartKind::Footer, second).unwrap();
    s.doc.last_section.title_page = true;
    s.doc.last_section.footers.first = Some(id);
    s.touch();
    let texts = |s: &mut Session| {
        let l = s.layout();
        l.pages
            .iter()
            .map(|pg| {
                shown_blocks(&s.doc, pg.footer_story).map(|b| {
                    b.iter().filter_map(|x| if let Block::Para(p) = &**x { Some(p.text.clone()) } else { None }).collect::<Vec<_>>().join("")
                })
            })
            .collect::<Vec<_>>()
    };
    let before = texts(&mut s);
    assert_eq!(before.last().cloned().flatten().as_deref(), Some("Chapter two opener"));
    hide(&mut s, 1).unwrap();
    let after = texts(&mut s);
    assert_eq!(after[0], None);
    assert_eq!(&after[1..], &before[1..], "every other page, the second chapter's opener included, shows what it showed");
}
