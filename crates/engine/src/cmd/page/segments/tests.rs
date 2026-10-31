use super::*;
use wordcraft_doc::Document;

fn at(s: &mut Session, block: usize) {
    s.sel = Selection::caret(Pos { story: StoryRef::Body, path: Path::top(block), off: 0 });
}

#[test]
fn page_segments_partial_updates_preserve_numbering_and_private_stories() {
    let mut s = Session::new(Document::from_text("First\nSecond"));
    s.run("insert.footer", &json!({"text":"Keep me"})).unwrap();
    configure(&mut s, 0, "number", "upperRoman", "", json!(4), false);
    configure(&mut s, 1, "number", "decimal", "", json!(1), true);
    at(&mut s, 0);
    let count = s.doc.parts.len();
    let footer = s.doc.sections()[0].1.footers;
    for name in ["Preface", "Foreword", "Opening"] {
        s.run("layout.pageSegment", &json!({"label":{"name":name}})).unwrap();
        let section = s.doc.sections()[0].1;
        assert_eq!((section.page_num_format, section.page_num_start), (NumFormat::UpperRoman, Some(4)));
        assert_eq!(section.footers, footer);
        assert_eq!(s.doc.parts.len(), count);
    }
    s.run("layout.pageSegment", &json!({"label":{},"start":null})).unwrap();
    assert_eq!(s.doc.sections()[0].1.page_num_start, None);
    assert_eq!(s.doc.sections()[0].1.page_num_format, NumFormat::UpperRoman);
    assert_eq!(s.doc.last_section.page_num_start, Some(1));
    for format in [Value::Null, json!(4), json!(true)] {
        assert!(s.run("layout.pageSegment", &json!({"label":{},"format":format})).is_err());
    }
}

#[test]
fn page_segments_even_pages_reuse_the_existing_default_footer() {
    let mut s = Session::new(Document::from_text("First\u{000c}Second"));
    s.run("insert.footer", &json!({"text":"Confidential"})).unwrap();
    s.doc.settings.even_odd_headers = true;
    configure(&mut s, 0, "number", "decimal", "", json!(1), false);
    assert!(s.doc.last_section.footers.even.is_none());
    let layout = s.layout();
    assert_eq!(layout.pages.len(), 2);
    assert_eq!(layout.pages[0].footer_story, layout.pages[1].footer_story);
    let id = s.doc.last_section.footers.default.unwrap();
    assert!(s.doc.plain_text(StoryRef::Part(id)).contains("Confidential"));
}

#[test]
fn page_segments_distinct_pages_with_equal_labels_stay_in_generated_lists() {
    for (mark, generate, expected) in
        [("references.markEntry", "references.index", "Term, 1, 1"), ("references.markCitation", "references.tableOfAuthorities", "Term\t1, 1")]
    {
        let mut s = Session::new(Document::from_text("Term\nTerm\nOutput"));
        configure(&mut s, 0, "number", "decimal", "", json!(1), false);
        configure(&mut s, 1, "number", "decimal", "", json!(1), true);
        for block in [0, 0, 1] {
            let p = s.doc.para(StoryRef::Body, &Path::top(block)).unwrap();
            let start = p.text.find("Term").unwrap();
            s.sel = Selection {
                anchor: Pos { story: StoryRef::Body, path: Path::top(block), off: start },
                focus: Pos { story: StoryRef::Body, path: Path::top(block), off: start + 4 },
            };
            s.run(mark, &json!({})).unwrap();
        }
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run(generate, &json!({})).unwrap();
        let text = s.doc.plain_text(StoryRef::Body);
        assert!(text.contains(expected), "{generate}: {text}");
        assert!(!text.contains(&format!("{expected}, 1")), "same-page duplicates should still merge");
    }
}

fn docx_xml(doc: &Document, name: &str) -> String {
    use std::io::Read;
    let bytes = wordcraft_docx::write(doc).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
    xml
}

fn page_field(instr: &str) -> InlineObject {
    InlineObject::Field { instr: instr.into(), result: String::new(), locked: false, dirty: false, code: false, form: String::new() }
}

#[test]
fn page_segments_docx_shared_and_inherited_stories_have_section_specific_fallbacks() {
    let mut doc = Document::from_text("First\nSecond\nThird");
    let mut footer = Paragraph::new();
    footer.insert_object(0, page_field("PAGE"), &CharProps::default()).unwrap();
    let id = doc.add_part(PartKind::Footer, vec![std::sync::Arc::new(Block::Para(footer))]);
    for (block, name) in [(0, "Preface"), (1, "Appendix")] {
        let mut section = wordcraft_doc::section::SectionProps {
            page_label: Some(PageLabel { mode: wordcraft_doc::page_label::PageLabelMode::Text, text: name.into(), ..Default::default() }),
            ..Default::default()
        };
        if block == 0 {
            section.footers.default = Some(id);
        }
        doc.para_mut(StoryRef::Body, &Path::top(block)).unwrap().section = Some(Box::new(section));
    }
    // Third section explicitly shares the same part, but has ordinary numbering.
    doc.last_section.footers.default = Some(id);
    assert!(docx_xml(&doc, "word/footer1.xml").contains(">Preface</w:t>"));
    assert!(docx_xml(&doc, "word/footer2.xml").contains(">Appendix</w:t>"));
    let ordinary = docx_xml(&doc, "word/footer3.xml");
    assert!(ordinary.contains(" PAGE "));
    assert!(!ordinary.contains("WordCraft.PageLabel"));
    assert_eq!(doc.parts.len(), 1, "export does not mutate the source document");
}

#[test]
fn page_segments_docx_zero_uses_explicit_field_format() {
    for (format, instr, conditional, expected) in
        [(NumFormat::UpperRoman, r"PAGE \* ARABIC", false, ">0</w:t>"), (NumFormat::Decimal, r"PAGE \* ROMAN", true, ">Opening</w:t>")]
    {
        let mut doc = Document::new();
        doc.last_section.page_num_format = format;
        doc.last_section.page_num_start = Some(0);
        doc.last_section.page_label = Some(PageLabel { zero_label: "Opening".into(), ..Default::default() });
        doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().insert_object(0, page_field(instr), &CharProps::default()).unwrap();
        let xml = docx_xml(&doc, "word/document.xml");
        assert_eq!(xml.contains(" IF "), conditional, "{xml}");
        assert!(xml.contains(expected), "{xml}");
    }
}

#[test]
fn page_segments_docx_labels_keep_insertions_and_deletions() {
    use wordcraft_doc::page_label::PageLabelMode;
    use wordcraft_doc::{Revision, RevisionKind};
    for mode in [PageLabelMode::Hidden, PageLabelMode::Text, PageLabelMode::Number, PageLabelMode::TextNumber] {
        for deleted in [false, true] {
            let mut doc = Document::new();
            doc.last_section.page_num_format = NumFormat::UpperRoman;
            doc.last_section.page_label = Some(PageLabel { mode, text: "Part".into(), ..Default::default() });
            doc.revisions.push(Revision::new(if deleted { RevisionKind::Delete } else { RevisionKind::Insert }, "Reviewer", "2026-10-30T12:00:00Z"));
            let props = if deleted { CharProps { del: Some(0), ..Default::default() } } else { CharProps { ins: Some(0), ..Default::default() } };
            doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().insert_object(0, page_field("PAGE"), &props).unwrap();
            let xml = docx_xml(&doc, "word/document.xml");
            let tag = if deleted { "w:del" } else { "w:ins" };
            assert_eq!(xml.matches(&format!("<{tag} ")).count(), 1, "one complete revision: {xml}");
            assert!(xml.find(&format!("<{tag} ")).unwrap() < xml.find("<w:sdt ").unwrap());
            assert!(xml.find("</w:sdt>").unwrap() < xml.find(&format!("</{tag}>")).unwrap());
            if deleted {
                assert!(!xml.contains("<w:t "), "deleted labels must use delText");
            }
            let read = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
            let props = read.para(StoryRef::Body, &Path::top(0)).unwrap().props_at(0);
            assert_eq!(props.del.is_some(), deleted);
            assert_eq!(props.ins.is_some(), !deleted);
        }
    }
}

#[test]
fn page_segments_saved_pageref_keeps_number_and_case_switches() {
    let mut s = Session::new(Document::from_text("Target\nReference"));
    configure(&mut s, 0, "textNumber", "decimal", "part", json!(31), false);
    s.run("select.text", &json!({"text":"Target"})).unwrap();
    s.run("insert.bookmark", &json!({"name":"target"})).unwrap();
    let p = s.doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    p.insert_object(0, page_field(r"PAGEREF target \* CardText \* FirstCap"), &CharProps::default()).unwrap();
    s.run("references.updateFields", &json!({})).unwrap();
    assert!(
        s.doc
            .para(StoryRef::Body, &Path::top(1))
            .unwrap()
            .objects
            .iter()
            .any(|o| matches!(o, InlineObject::Field { result, .. } if result == "Part thirty-one"))
    );
}
fn configure(s: &mut Session, block: usize, mode: &str, format: &str, text: &str, start: Value, new: bool) {
    at(s, block);
    s.run(
        "layout.pageSegment",
        &json!({"new":new,"from":"cursor","label":{"name":format!("part {block}"),"mode":mode,"text":text},"format":format,"start":start}),
    )
    .unwrap();
}
fn labels(s: &mut Session) -> Vec<String> {
    let layout = s.layout();
    layout.pages.iter().map(|p| s.doc.sections()[p.section].1.page_label(p.number)).collect()
}

#[test]
fn page_segments_book_roundtrip_and_undo() {
    let mut s = Session::new(Document::from_text("Cover\nContents\nPreface\nChapter\nAppendix"));
    s.run("insert.footer", &json!({"text":"Confidential"})).unwrap();
    configure(&mut s, 0, "hidden", "decimal", "", json!(1), false);
    configure(&mut s, 1, "textNumber", "upperRoman", "TOC", json!(1), true);
    configure(&mut s, 2, "number", "upperLetter", "", json!(1), true);
    configure(&mut s, 3, "number", "decimal", "", json!(1), true);
    configure(&mut s, 4, "textNumber", "decimal", "Annex", json!(0), true);
    assert_eq!(labels(&mut s), ["", "TOC I", "A", "1", "Annex 0"]);
    assert_eq!(s.doc.body.len(), 5, "no duplicate empty paragraphs at existing boundaries");
    for (_, section) in s.doc.sections() {
        assert!(s.doc.plain_text(StoryRef::Part(section.footers.default.unwrap())).contains("Confidential"));
    }
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let reopened = wordcraft_docx::read(&bytes).unwrap();
    let mut reopened = Session::new(reopened);
    assert_eq!(labels(&mut reopened), labels(&mut s));
    let twice = wordcraft_docx::read(&wordcraft_docx::write(&reopened.doc).unwrap()).unwrap();
    assert_eq!(labels(&mut Session::new(twice)), labels(&mut s), "no doubled prefix on a second roundtrip");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.sections().len(), 4);
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(labels(&mut s), ["", "TOC I", "A", "1", "Annex 0"]);
}

#[test]
fn page_segments_skip_text_and_hidden_counters_and_keep_restarts_after_edits() {
    let mut s = Session::new(Document::from_text("One\nNotice\nTwo\nBody"));
    configure(&mut s, 0, "number", "upperLetter", "", json!(1), false);
    configure(&mut s, 1, "text", "decimal", "Notice", Value::Null, true);
    configure(&mut s, 2, "number", "upperLetter", "", Value::Null, true);
    configure(&mut s, 3, "number", "decimal", "", json!(1), true);
    assert_eq!(labels(&mut s), ["A", "Notice", "B", "1"]);
    at(&mut s, 1);
    s.run("text.insert", &json!({"text":"extra\u{000c}"})).unwrap();
    assert_eq!(labels(&mut s), ["A", "Notice", "Notice", "B", "1"]);
}

#[test]
fn page_segments_zero_and_invalid_parameters() {
    let mut s = Session::new(Document::from_text("One\u{000c}Two"));
    s.run("layout.pageSegment", &json!({"label":{"mode":"textNumber","text":"Part","zeroLabel":"Opening"},"format":"upperRoman","start":0})).unwrap();
    assert_eq!(labels(&mut s), ["Opening", "Part I"]);
    let before = serde_json::to_value(&s.doc).unwrap();
    for params in [
        json!({"label":{"mode":"unknown"}}),
        json!({"label":{"text":"a\nb"}}),
        json!({"label":{},"start":-1}),
        json!({"label":{},"section":9999}),
        json!({"label":{},"format":"bogus"}),
        json!({"label":{},"new":true,"from":"bogus"}),
    ] {
        assert!(s.run("layout.pageSegment", &params).is_err(), "{params}");
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
    }
}

#[test]
fn page_segments_table_boundary_is_refused_without_moving_the_table() {
    let mut s = Session::new(Document::new());
    s.doc.insert_block(StoryRef::Body, &Path::top(0), Block::Table(wordcraft_doc::Table::new(1, 1, 300.0))).unwrap();
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    let before = serde_json::to_value(&s.doc).unwrap();
    assert!(s.run("layout.pageSegment", &json!({"new":true,"from":"page","label":{},"start":1})).is_err());
    assert_eq!(serde_json::to_value(&s.doc).unwrap(), before);
}

#[test]
fn page_segments_page_boundary_and_shared_footer_target() {
    let mut s = Session::new(Document::from_text("First\u{000c}Second\nThird"));
    s.run("insert.footer", &json!({"text":"Keep me"})).unwrap();
    let id = s.doc.last_section.footers.default.unwrap();
    s.page_hint = 1;
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Part(id)));
    s.run("layout.pageSegment", &json!({"new":true,"from":"page","label":{"name":"Second","mode":"text","text":"Appendix"},"start":1})).unwrap();
    assert_eq!(s.doc.sections().len(), 2);
    assert_eq!(labels(&mut s), ["1", "Appendix"]);
    let section = s.doc.sections()[1].1.clone();
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Part(section.footers.default.unwrap())));
    s.page_hint = 1;
    s.run("layout.pageSegment", &json!({"label":{"mode":"text","text":"Changed"},"start":1})).unwrap();
    assert_eq!(labels(&mut s), ["1", "Changed"]);
}

#[test]
fn page_segments_new_footer_does_not_leak_into_a_later_empty_footer() {
    let mut s = Session::new(Document::from_text("First\nSecond"));
    s.doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::default());
    s.doc.last_section.start = SectionStart::NextPage;
    configure(&mut s, 0, "text", "decimal", "Only first", json!(1), false);
    let layout = s.layout();
    assert_eq!(layout.pages.len(), 2);
    assert!(!layout.pages[0].footer.is_empty());
    assert!(
        layout.pages[1].footer.iter().all(|item| match item {
            wordcraft_layout::Placed::Lines { para, .. } => para.glyphs.is_empty(),
            _ => true,
        }),
        "a later section with no footer must stay visually empty"
    );
    let before = s.doc.parts.len();
    at(&mut s, 1);
    s.run("layout.pageSegment", &json!({"label":null,"start":0,"format":"arabicAlpha"})).unwrap();
    assert_eq!(s.doc.parts.len(), before, "ordinary number formatting does not copy stories");
    assert_eq!(s.doc.last_section.page_num_format, NumFormat::ArabicAlpha, "existing imported formats remain editable");
}

#[test]
fn page_segments_add_labels_to_first_even_and_default_without_deleting_text() {
    let mut s = Session::new(Document::from_text("One\u{000c}Two\u{000c}Three"));
    s.doc.last_section.title_page = true;
    s.doc.settings.even_odd_headers = true;
    configure(&mut s, 0, "text", "decimal", "Notice", json!(1), false);
    let layout = s.layout();
    assert_eq!(layout.pages.len(), 3);
    for page in &layout.pages {
        assert!(page.footer_story.is_some());
        let glyphs: usize = page
            .footer
            .iter()
            .map(|item| match item {
                wordcraft_layout::Placed::Lines { para, .. } => para.glyphs.len(),
                _ => 0,
            })
            .sum();
        assert!(glyphs > 0, "all active slots show the literal label");
    }
}

#[test]
fn page_segments_references_use_section_formats_and_custom_labels() {
    // #3856: the same logical label in all generated lists, including legacy sections.
    for custom in [false, true] {
        let mut s = Session::new(Document::from_text("Heading\nPicture\nTerm\nCase\nOutput"));
        s.doc.last_section.page_num_format = NumFormat::UpperRoman;
        s.doc.last_section.page_num_start = Some(4);
        if custom {
            configure(&mut s, 0, "textNumber", "upperRoman", "Part", json!(4), false);
        }
        let expected = if custom { "Part IV" } else { "IV" };
        s.run("select.text", &json!({"text":"Heading"})).unwrap();
        s.run("styles.apply", &json!({"style":"Heading 1"})).unwrap();
        s.run("insert.bookmark", &json!({"name":"heading"})).unwrap();
        s.run("select.text", &json!({"text":"Picture"})).unwrap();
        s.run("references.caption", &json!({"label":"Figure","text":"Picture"})).unwrap();
        s.run("select.text", &json!({"text":"Term"})).unwrap();
        s.run("references.markEntry", &json!({})).unwrap();
        s.run("select.text", &json!({"text":"Case"})).unwrap();
        s.run("references.markCitation", &json!({})).unwrap();
        let source = s.doc.clone();
        for (command, text) in [
            ("references.toc", "Heading\t"),
            ("references.tableOfFigures", "Figure 1: Picture\t"),
            ("references.index", "Term, "),
            ("references.tableOfAuthorities", "Case\t"),
        ] {
            let mut generated = Session::new(source.clone());
            generated.run("caret.docEnd", &json!({})).unwrap();
            generated.run("text.newParagraph", &json!({})).unwrap();
            generated.run(command, &json!({})).unwrap();
            let body = generated.doc.plain_text(StoryRef::Body);
            assert!(body.contains(&format!("{text}{expected}")), "{command}: {body}");
        }
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("insert.crossReference", &json!({"to":"bookmark","target":"heading","show":"page"})).unwrap();
        let results: Vec<_> = s
            .doc
            .para_paths(StoryRef::Body)
            .iter()
            .flat_map(|path| &s.doc.para(StoryRef::Body, path).unwrap().objects)
            .filter_map(|o| match o {
                InlineObject::Field { instr, result, .. } if instr.starts_with("PAGEREF") => Some(result.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(results, [expected]);
        let layout = s.layout();
        let shown: Vec<_> = layout
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| match item {
                wordcraft_layout::Placed::Lines { para, .. } => Some(&para.shown),
                _ => None,
            })
            .flatten()
            .map(|x| x.1.as_str())
            .collect();
        assert!(shown.contains(&expected), "live PAGEREF must agree with its saved result: {shown:?}");
    }
}

#[test]
fn page_segments_index_range_uses_physical_order_across_a_restart() {
    let mut s = Session::new(Document::from_text("First\nLast\nIndex"));
    configure(&mut s, 0, "number", "upperRoman", "", json!(4), false);
    configure(&mut s, 1, "number", "decimal", "", json!(1), true);
    s.doc
        .para_mut(StoryRef::Body, &Path::top(0))
        .unwrap()
        .insert_object(0, InlineObject::BookmarkStart { name: "Span".into() }, &CharProps::default())
        .unwrap();
    let end = s.doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    end.insert_object(end.len(), InlineObject::BookmarkEnd { name: "Span".into() }, &CharProps::default()).unwrap();
    at(&mut s, 0);
    s.run("references.markEntry", &json!({"entry":"Term","bookmark":"Span"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("references.index", &json!({})).unwrap();
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(text.contains("Term, IV–1"), "range endpoints must not be compared as counters: {text}");
}

#[test]
fn page_segments_export_pdf_labels_and_docx_fallback_without_changing_locked_fields() {
    use std::io::Read;
    let mut s = Session::new(Document::from_text("Cover\nContents\nBody\nLegacy"));
    configure(&mut s, 0, "hidden", "decimal", "", json!(1), false);
    configure(&mut s, 1, "textNumber", "upperRoman", "TOC", json!(1), true);
    configure(&mut s, 2, "number", "decimal", "", json!(0), true);
    at(&mut s, 3);
    s.run("layout.pageSegment", &json!({"new":true,"from":"cursor","label":null,"start":7})).unwrap();
    let bytes = wordcraft_pdf::export(&s.doc, &wordcraft_pdf::PdfOptions { compress: false, ..Default::default() }).unwrap();
    let raw = String::from_utf8_lossy(&bytes);
    for expected in ["/PageLabels", "/P()", "/P(TOC I)", "/P(0)", "/P(7)"] {
        assert!(raw.contains(expected), "missing PDF label: {expected}: {:?}", raw.lines().filter(|l| l.contains("/P")).collect::<Vec<_>>());
    }
    let mut locked = Paragraph::new();
    locked
        .insert_object(
            0,
            InlineObject::Field { instr: "PAGE".into(), result: "Keep cached".into(), locked: true, dirty: false, code: false, form: String::new() },
            &CharProps::default(),
        )
        .unwrap();
    s.doc.insert_block(StoryRef::Body, &Path::top(4), Block::Para(locked)).unwrap();
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let mut all = String::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        if entry.name().ends_with(".xml") {
            entry.read_to_string(&mut all).unwrap();
        }
    }
    assert!(all.contains("wc:pageLabel="));
    assert!(all.contains("WordCraft.PageLabel"));
    assert!(all.contains(" IF ") && all.contains("PAGE \\* ARABIC"), "standard conditional fallback for Roman zero");
    assert!(all.contains("TOC I"));
    let read = wordcraft_docx::read(&bytes).unwrap();
    assert!(read.para_paths(StoryRef::Body).iter().any(|path| {
        read.para(StoryRef::Body, path)
            .unwrap()
            .objects
            .iter()
            .any(|o| matches!(o,InlineObject::Field {locked:true,result,..} if result=="Keep cached"))
    }));
}
