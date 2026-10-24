use super::*;
use wordcraft_doc::{Document, SectionProps};

fn control() -> Document {
    let mut doc = Document::from_text("Contents\nStale entry\n\nIncluded\nExcluded\nUnrelated ");
    doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().props.style = Some("TOCHeading".into());
    let first = doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    first.props.style = Some("TOC1".into());
    first
        .insert_object(0, InlineObject::FieldStart { instr: r#"TOC \o "1-2" \h \z \u"#.into(), locked: false, dirty: false }, &CharProps::default())
        .unwrap();
    let end = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    end.insert_object(0, InlineObject::FieldEnd, &CharProps::default()).unwrap();
    end.section = Some(Box::new(SectionProps { page_num_start: Some(3), ..Default::default() }));
    doc.para_mut(StoryRef::Body, &Path::top(3)).unwrap().props.style = Some("Heading1".into());
    doc.styles.styles.push(wordcraft_doc::styles::Style {
        id: "Heading2NoTOC".into(),
        name: "Heading 2 (no TOC)".into(),
        based_on: Some("Normal".into()),
        ..Default::default()
    });
    doc.para_mut(StoryRef::Body, &Path::top(4)).unwrap().props.style = Some("Heading2NoTOC".into());
    let other = doc.para_mut(StoryRef::Body, &Path::top(5)).unwrap();
    other
        .insert_object(
            other.len(),
            InlineObject::Field {
                instr: "AUTHOR".into(),
                result: "Cached author".into(),
                locked: false,
                dirty: false,
                code: false,
                form: String::new(),
            },
            &CharProps::default(),
        )
        .unwrap();
    doc.core.creator = "Current author".into();
    doc.last_section.page_num_start = Some(1);
    doc
}

fn refresh_doc(doc: Document) -> Result<Session, CmdError> {
    let mut session = Session::new(doc);
    session.run("references.updateToc", &json!({"forPdf": true}))?;
    Ok(session)
}

#[test]
fn refresh_keeps_sections() {
    let doc = control();
    let original = serde_json::to_value(&doc).unwrap();
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let imported = wordcraft_docx::read(&bytes).unwrap();
    let sections: Vec<_> = imported.sections().into_iter().map(|(_, section)| section.clone()).collect();
    let session = refresh_doc(imported).unwrap();
    let (start, end) = toc_range(&session).unwrap();
    assert_eq!((start, end), (0, 1), "empty begin paragraph moves onto TOCHeading");
    assert_eq!(session.doc.body[1].as_para().unwrap().plain_text(), "Included\t1");
    assert!(session.doc.plain_text(StoryRef::Body).contains("Excluded"));
    assert!(!session.doc.body[1].as_para().unwrap().plain_text().contains("Excluded"));
    assert!(session.doc.plain_text(StoryRef::Body).contains("Cached author"), "no global field update");
    assert_eq!(session.doc.sections().into_iter().map(|(_, section)| section.clone()).collect::<Vec<_>>(), sections);
    let heading = session
        .doc
        .para_paths(StoryRef::Body)
        .into_iter()
        .find(|path| session.doc.para(StoryRef::Body, path).unwrap().plain_text() == "Included")
        .unwrap();
    let layout = crate::print_fields::prepare(&session.doc, &Default::default()).unwrap().layout;
    let caret = layout.caret(&Pos::new(StoryRef::Body, heading, 0)).unwrap();
    assert_eq!(layout.pages[caret.page].number, 1);
    assert_eq!(serde_json::to_value(&doc).unwrap(), original);
}

#[test]
fn default_keeps_cached_toc() {
    let doc = control();
    let before = serde_json::to_value(&doc).unwrap();
    let printed = crate::print_fields::prepare(&doc, &Default::default()).unwrap();
    assert!(printed.doc.plain_text(StoryRef::Body).contains("Stale entry"));
    assert_eq!(serde_json::to_value(&doc).unwrap(), before);
}

#[test]
fn absent_and_locked_are_kept() {
    let doc = Document::from_text("Only body");
    let before = serde_json::to_value(&doc).unwrap();
    assert_eq!(serde_json::to_value(refresh_doc(doc).unwrap().doc).unwrap(), before);
    for range in [false, true] {
        let mut doc = control();
        if range {
            if let InlineObject::FieldStart { locked, .. } = &mut doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().objects[0] {
                *locked = true;
            }
        } else {
            doc = Document::from_text("Cached");
            doc.para_mut(StoryRef::Body, &Path::top(0))
                .unwrap()
                .insert_object(
                    0,
                    InlineObject::Field {
                        instr: r#"TOC \o "1-2""#.into(),
                        result: "Stored".into(),
                        locked: true,
                        dirty: false,
                        code: false,
                        form: String::new(),
                    },
                    &CharProps::default(),
                )
                .unwrap();
        }
        let before = serde_json::to_value(&doc).unwrap();
        assert_eq!(serde_json::to_value(refresh_doc(doc).unwrap().doc).unwrap(), before);
    }
}

#[test]
fn figure_list_is_not_a_toc() {
    let mut doc = control();
    let mut figure = Paragraph::with_text("Stored figures", CharProps::default());
    figure
        .insert_object(
            0,
            InlineObject::Field {
                instr: r#"TOC \c "Figure""#.into(),
                result: "Cached figure page99".into(),
                locked: false,
                dirty: false,
                code: false,
                form: String::new(),
            },
            &CharProps::default(),
        )
        .unwrap();
    let saved = figure.clone();
    doc.body.push(para_block(figure));
    let session = refresh_doc(doc).unwrap();
    assert_eq!(session.doc.body.last().unwrap().as_para().unwrap(), &saved);
}

#[test]
fn unsupported_is_atomic() {
    for instr in [r#"TOC \b restricted"#, r#"TOC \o "1-3" \x"#, r#"TOC \o "1-3-5""#] {
        let mut doc = control();
        if let InlineObject::FieldStart { instr: code, .. } = &mut doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().objects[0] {
            *code = instr.into();
        }
        let before = serde_json::to_value(&doc).unwrap();
        let mut session = Session::new(doc);
        assert!(session.run("references.updateToc", &json!({"forPdf": true})).is_err());
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
    }
    let mut doc = control();
    let mut other = Paragraph::new();
    other
        .insert_object(
            0,
            InlineObject::Field {
                instr: r#"TOC \o "1-1""#.into(),
                result: "Second".into(),
                locked: false,
                dirty: false,
                code: false,
                form: String::new(),
            },
            &CharProps::default(),
        )
        .unwrap();
    doc.body.push(para_block(other));
    let before = serde_json::to_value(&doc).unwrap();
    let mut session = Session::new(doc);
    assert!(session.run("references.updateToc", &json!({"forPdf": true})).is_err());
    assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
}

#[test]
fn existing_switch_defaults() {
    for instr in [r"TOC \o \h \z \u", r#"TOC \t "Heading 1,1,""#, r#"TOC \o "1-3" \n"#] {
        assert!(check_switches(&FieldCode::parse(instr)).is_ok(), "{instr}");
    }
}

#[test]
fn outside_toc_text_is_kept() {
    for trailing in [false, true] {
        let mut doc = control();
        doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().section = None;
        if trailing {
            let end = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
            end.insert_text(end.len(), "KEEP trailing", &CharProps::default()).unwrap();
        } else {
            doc.insert_block(StoryRef::Body, &Path::top(3), Block::Para(Paragraph::with_text("KEEP following", CharProps::default()).styled("TOC1")))
                .unwrap();
        }
        let before = serde_json::to_value(&doc).unwrap();
        let mut session = Session::new(doc);
        assert!(session.run("references.updateToc", &json!({"forPdf": true})).is_err());
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
    }
}

#[test]
fn heading_section_is_rejected() {
    let mut doc = control();
    doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(SectionProps { page_w: 800.0, ..Default::default() }));
    let before = serde_json::to_value(&doc).unwrap();
    let mut session = Session::new(doc);
    assert!(session.run("references.updateToc", &json!({"forPdf": true})).is_err());
    assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
}

#[test]
fn content_locks_survive_import() {
    use wordcraft_doc::control::{ContentControl, ControlKind, ControlLock};
    for gallery in [false, true] {
        for lock in [ControlLock::ContentLocked, ControlLock::SdtContentLocked] {
            let mut doc = control();
            let kind = if gallery {
                ControlKind::Gallery { list: false, gallery: "Table of Contents".into(), category: String::new(), unique: false }
            } else {
                ControlKind::RichText
            };
            let ctl = ContentControl { block: true, lock, ..ContentControl::new(kind) };
            doc.para_mut(StoryRef::Body, &Path::top(0))
                .unwrap()
                .insert_object(0, InlineObject::ControlStart { control: Box::new(ctl) }, &CharProps::default())
                .unwrap();
            let end = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
            end.insert_object(end.len(), InlineObject::ControlEnd, &CharProps::default()).unwrap();
            let imported = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
            assert!(imported.control_ranges(StoryRef::Body).iter().any(|r| r.control.lock == lock));
            let before = serde_json::to_value(&imported).unwrap();
            assert_eq!(serde_json::to_value(refresh_doc(imported).unwrap().doc).unwrap(), before);
        }
    }
}

#[test]
fn inherited_outline_is_selected() {
    let mut doc = Document::from_text("Inherited\nExcluded\nExplicit");
    doc.styles.styles.push(wordcraft_doc::styles::Style {
        id: "OutlineBase".into(),
        name: "Outline Base".into(),
        para: wordcraft_doc::ParaProps { outline_level: Some(1), ..Default::default() },
        ..Default::default()
    });
    doc.styles.styles.push(wordcraft_doc::styles::Style {
        id: "Derived".into(),
        name: "Derived".into(),
        based_on: Some("OutlineBase".into()),
        ..Default::default()
    });
    for i in 0..3 {
        doc.para_mut(StoryRef::Body, &Path::top(i)).unwrap().props.style = Some("Derived".into());
    }
    doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().props.outline_level = Some(9);
    doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().props.outline_level = Some(9);
    let session = Session::new(doc);
    let entries = toc_entries(&session, &FieldCode::parse(r"TOC \u"));
    assert_eq!(entries.iter().map(|e| (&*e.text, e.level)).collect::<Vec<_>>(), [("Inherited", 1)]);
    let entries = toc_entries(&session, &FieldCode::parse(r#"TOC \u \t "Derived,3""#));
    assert_eq!(entries.iter().map(|e| e.level).collect::<Vec<_>>(), [2, 2, 2]);
}

#[derive(Clone, Copy)]
enum Tail {
    PageBreak,
    Bookmark,
}

fn tail_control(kind: Tail) -> Document {
    let mut doc = control();
    let end = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    end.props.space_before = Some(0.0);
    end.props.space_after = Some(0.0);
    end.props.line_spacing = Some(wordcraft_doc::props::LineSpacing::Exactly(1.0));
    let props = CharProps { size: Some(8.0), italic: Some(true), ..Default::default() };
    match kind {
        Tail::PageBreak => {
            end.insert_text(end.len(), "\u{000c}", &props).unwrap();
        }
        Tail::Bookmark => {
            end.insert_object(end.len(), InlineObject::BookmarkStart { name: "TOC".into() }, &props).unwrap();
            end.insert_object(end.len(), InlineObject::BookmarkEnd { name: "TOC".into() }, &props).unwrap();
        }
    }
    end.mark = CharProps { size: Some(19.0), bold: Some(true), ..Default::default() };
    doc
}

#[test]
fn tails_keep_formatting() {
    for kind in [Tail::PageBreak, Tail::Bookmark] {
        for section in [None, Some(SectionProps { page_num_start: Some(3), ..Default::default() })] {
            let mut doc = tail_control(kind);
            let end = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
            end.section = section.map(Box::new);
            let mut expected = end.clone();
            expected.delete(0, OBJ.len_utf8()).unwrap();
            let sections: Vec<_> = doc.sections().into_iter().map(|(_, section)| section.clone()).collect();
            let unaffected = [doc.body[4].clone(), doc.body[5].clone()];
            let source = serde_json::to_value(&doc).unwrap();
            let session = refresh_doc(doc.clone()).unwrap();
            let (start, last) = toc_range(&session).unwrap();
            assert_eq!((start, last), (0, 1), "detached tail permits the normal heading move");
            assert_eq!(session.doc.body[last + 1].as_para().unwrap(), &expected, "tail properties, runs, mark, objects and section");
            assert_eq!(session.doc.sections().into_iter().map(|(_, section)| section.clone()).collect::<Vec<_>>(), sections);
            for original in unaffected {
                assert_eq!(session.doc.body.iter().filter(|block| block.as_ref() == original.as_ref()).count(), 1);
            }
            let paths = session.doc.para_paths(StoryRef::Body);
            let headings: Vec<_> = paths.iter().filter(|path| session.doc.para(StoryRef::Body, path).unwrap().plain_text() == "Included").collect();
            assert_eq!(headings.len(), 1);
            let printed = crate::print_fields::prepare(&session.doc, &Default::default()).unwrap();
            let caret = printed.layout.caret(&Pos::new(StoryRef::Body, headings[0].clone(), 0)).unwrap();
            assert_eq!(session.doc.body[1].as_para().unwrap().plain_text(), format!("Included\t{}", printed.layout.pages[caret.page].number));
            assert_eq!(serde_json::to_value(&doc).unwrap(), source);
        }
    }
}

#[test]
fn unsafe_tails_are_atomic() {
    for text in ["\u{000c}KEEP", "\u{000c}\u{000c}", " "] {
        let mut doc = control();
        let end = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
        end.insert_text(end.len(), text, &CharProps::default()).unwrap();
        let before = serde_json::to_value(&doc).unwrap();
        let mut session = Session::new(doc);
        assert!(session.run("references.updateToc", &json!({"forPdf": true})).is_err());
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
    }
    for kind in [Tail::PageBreak, Tail::Bookmark] {
        let mut doc = tail_control(kind);
        doc.para_mut(StoryRef::Body, &Path::top(0)).unwrap().section = Some(Box::new(SectionProps { page_w: 800.0, ..Default::default() }));
        let before = serde_json::to_value(&doc).unwrap();
        let mut session = Session::new(doc);
        assert!(session.run("references.updateToc", &json!({"forPdf": true})).is_err(), "prepared heading move retains the section guard");
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
    }
    for name in ["", "different"] {
        let mut doc = tail_control(Tail::Bookmark);
        let end = doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
        end.objects[2] = InlineObject::BookmarkEnd { name: name.into() };
        let before = serde_json::to_value(&doc).unwrap();
        let mut session = Session::new(doc);
        assert!(session.run("references.updateToc", &json!({"forPdf": true})).is_err());
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
    }
    let mut doc = tail_control(Tail::PageBreak);
    doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap().props.style = Some("TOC1".into());
    let before = serde_json::to_value(&doc).unwrap();
    let mut session = Session::new(doc);
    assert!(session.run("references.updateToc", &json!({"forPdf": true})).is_err(), "TOC-styled detached tails could be consumed by the generator");
    assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
}

#[test]
fn internal_cache_is_bounded() {
    let link = InlineObject::Field {
        instr: r#"HYPERLINK \l "_Toc209691849""#.into(),
        result: "Hyperlinked stale entry".into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let mut doc = control();
    let para = doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    para.insert_object(para.len(), link.clone(), &CharProps::default()).unwrap();
    let session = refresh_doc(doc).unwrap();
    assert_eq!(session.doc.body[1].as_para().unwrap().plain_text(), "Included\t1");
    assert!(!session.doc.plain_text(StoryRef::Body).contains("Hyperlinked stale entry"));
    assert!(session.doc.plain_text(StoryRef::Body).contains("Cached author"));
    for instr in [
        r#"HYPERLINK "https://example.invalid""#,
        r#"HYPERLINK "external" \l "local""#,
        r#"HYPERLINK \l """#,
        r#"HYPERLINK \l "local" \t "frame""#,
        "AUTHOR",
    ] {
        let mut doc = control();
        let mut object = link.clone();
        if let InlineObject::Field { instr: field, .. } = &mut object {
            *field = instr.into();
        }
        let para = doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
        para.insert_object(para.len(), object, &CharProps::default()).unwrap();
        let before = serde_json::to_value(&doc).unwrap();
        let mut session = Session::new(doc);
        assert!(session.run("references.updateToc", &json!({"forPdf": true})).is_err());
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
    }
    let mut doc = control();
    let mut locked = link;
    if let InlineObject::Field { locked, .. } = &mut locked {
        *locked = true;
    }
    let para = doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
    para.insert_object(para.len(), locked, &CharProps::default()).unwrap();
    let before = serde_json::to_value(&doc).unwrap();
    assert_eq!(serde_json::to_value(refresh_doc(doc).unwrap().doc).unwrap(), before);
}

#[test]
fn numbered_style_keeps_one_line() {
    use wordcraft_doc::props::{NumRef, TabAlign, TabLeader, TabStop};
    for canonical in [false, true] {
        let mut doc = control();
        let width = doc.sections()[0].1.text_width();
        let mut style = doc.styles.get("TOC3").unwrap().clone();
        style.id = "TOC 3".into();
        style.name = "TOC 3".into();
        style.para.tabs = Some(vec![
            TabStop { pos: 36.0, align: TabAlign::Left, leader: TabLeader::None },
            TabStop { pos: width - 0.5, align: TabAlign::Right, leader: TabLeader::Dot },
        ]);
        style.chr.font = Some("Arial".into());
        style.chr.size = Some(12.0);
        if canonical {
            let mut preferred = style.clone();
            preferred.id = "TOC3".into();
            preferred.para.tabs.as_mut().unwrap()[0].pos = 48.0;
            *doc.styles.get_mut("TOC3").unwrap() = preferred;
        } else {
            doc.styles.styles.retain(|style| style.id != "TOC3");
        }
        doc.styles.styles.push(style);
        if let InlineObject::FieldStart { instr, .. } = &mut doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap().objects[0] {
            *instr = r#"TOC \o "3-3""#.into();
        }
        let num = doc.numbering.add_list(wordcraft_doc::ListKind::Numbered);
        let heading = doc.para_mut(StoryRef::Body, &Path::top(3)).unwrap();
        heading.delete(0, heading.len()).unwrap();
        heading.insert_text(0, "TANK DESCRIPTION", &CharProps::default()).unwrap();
        heading.props.style = Some("Heading3".into());
        heading.props.numbering = Some(NumRef { num, level: 0 });
        let imported = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
        assert_eq!(imported.styles.get("TOC3").is_some(), canonical);
        assert_eq!(imported.styles.get("TOC 3").unwrap().para.tabs.as_ref().unwrap()[0].pos, 36.0);
        let styles = imported.styles.clone();
        let session = refresh_doc(imported).unwrap();
        assert_eq!(session.doc.styles, styles, "no aliases or style definitions are created");
        let (start, end) = toc_range(&session).unwrap();
        assert_eq!((start, end), (0, 1));
        let entry = session.doc.body[1].as_para().unwrap();
        assert_eq!(entry.props.style.as_deref(), Some(if canonical { "TOC3" } else { "TOC 3" }));
        assert_eq!(entry.plain_text(), "1.\tTANK DESCRIPTION\t1");
        let resolved = session.doc.styles.resolve_char(entry.props.style.as_deref(), &CharProps::default());
        assert_eq!((resolved.font.as_str(), resolved.size), ("Arial", 12.0));
        let print = crate::print_fields::prepare(&session.doc, &Default::default()).unwrap();
        let title_off = entry.text.find('\t').unwrap() + 1;
        let page_off = entry.text.rfind('\t').unwrap() + 1;
        let number = print.layout.caret(&Pos::body(1, 0)).unwrap();
        let title = print.layout.caret(&Pos::body(1, title_off)).unwrap();
        let page = print.layout.caret(&Pos::body(1, page_off)).unwrap();
        let last = print.layout.caret(&Pos::body(1, entry.len())).unwrap();
        assert_eq!((number.page, number.top), (title.page, title.top));
        assert_eq!((title.page, title.top), (page.page, page.top));
        let section = session.doc.sections()[0].1;
        let left = if canonical { 48.0 } else { 36.0 };
        assert!((title.x - section.margin_left - left).abs() < 1.0, "title x={}", title.x);
        assert!(page.x > section.margin_left + width - 30.0, "page x={}", page.x);
        assert!((last.x - section.margin_left - width + 0.5).abs() < 1.0, "last x={}", last.x);
        let lines: usize = print
            .layout
            .pages
            .iter()
            .flat_map(|page| &page.items)
            .filter_map(|item| match item {
                wordcraft_layout::Placed::Lines { story: StoryRef::Body, path, l0, l1, .. } if *path == Path::top(1) => Some(l1 - l0),
                _ => None,
            })
            .sum();
        assert_eq!(lines, 1, "number, title and page occupy one physical line");
    }
}

#[test]
fn spaced_style_fallback_is_bounded() {
    let mut doc = control();
    doc.styles.styles.retain(|style| style.id != "TOC3");
    let mut style = doc.styles.get("TOC2").unwrap().clone();
    style.id = "TOC 3".into();
    style.builtin = false;
    doc.styles.styles.push(style);
    let mut session = Session::new(doc);
    assert_eq!(toc_entry_style(&session, 3), "TOC3", "custom aliases are not admitted");
    session.doc.styles.get_mut("TOC 3").unwrap().builtin = true;
    session.doc.styles.get_mut("TOC 3").unwrap().kind = wordcraft_doc::styles::StyleKind::Character;
    assert_eq!(toc_entry_style(&session, 3), "TOC3", "character styles are not admitted");
}
