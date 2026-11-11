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
fn toggle_without_visible() {
    let mut s = book(4);
    s.run("layout.pageFooterVisible", &json!({"page": 2})).unwrap();
    assert_eq!(shown(&mut s), only(2, 4));
    s.run("layout.pageFooterVisible", &json!({"page": 2})).unwrap();
    assert_eq!(shown(&mut s), vec![true; 4]);
    // The caret's page when none is given.
    s.run("select.text", &json!({"text": "Page 3 second"})).unwrap();
    s.run("layout.pageFooterVisible", &json!({})).unwrap();
    assert_eq!(shown(&mut s), only(3, 4));
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
fn an_unreadable_page_is_an_error_and_a_null_page_means_the_caret_page() {
    let mut s = book(2);
    let before = shown(&mut s);
    for page in [json!(-1), json!("2"), json!(true), json!(1.5), json!(2.8)] {
        let e = s.run("layout.pageFooterVisible", &json!({"page": page.clone(), "visible": false})).unwrap_err();
        assert!(matches!(e, CmdError::Params(_)), "{page}: {e:?}");
    }
    assert_eq!(shown(&mut s), before, "a refused call changes nothing");
    assert!(s.run("layout.pageFooterVisible", &json!({"page": null, "visible": true})).is_ok());
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
    let e = hide(&mut s, 2).unwrap_err();
    assert!(matches!(e, CmdError::Disabled(_)) && e.to_string().contains("table"), "{e}");
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
