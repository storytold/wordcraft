//! Synthetic structural controls for Leaf #277. No Word-generated fixtures.
use serde_json::json;
use std::sync::Arc;
use wordcraft_doc::props::{Align, Border, Borders, Rgb, VMerge};
use wordcraft_doc::{Block, CharProps, Document, InlineObject, Paragraph, PartKind, Path, Pos, StoryRef, Table, para_block};
use wordcraft_engine::{Selection, Session, io};

fn table(rows: usize, prefix: &str) -> Table {
    let mut t = Table::new(rows, 2, 300.0);
    t.props.fixed = true;
    for (ri, row) in t.rows.iter_mut().enumerate() {
        for (ci, cell) in row.cells.iter_mut().enumerate() {
            cell.blocks = vec![para_block(Paragraph::with_text(&format!("{prefix}-{ri}-{ci}"), CharProps::default()))];
        }
    }
    t
}

fn fixture(separator: bool) -> Session {
    let mut doc = Document::new();
    doc.body = vec![Arc::new(Block::Table(table(2, "first")))];
    if separator {
        doc.body.push(para_block(Paragraph::new()));
    }
    doc.body.extend([Arc::new(Block::Table(table(2, "second"))), para_block(Paragraph::with_text("after", CharProps::default()))]);
    let mut s = Session::new(doc);
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 1, 1, 0]), 3));
    s
}

fn join(s: &mut Session) {
    s.run("table.joinNext", &json!({})).unwrap();
}

fn assert_refused(s: &mut Session, reason: &str) {
    let before = s.doc.clone();
    let selection = s.sel.clone();
    let dirty = s.dirty;
    let available = s.run("table.joinNextStatus", &json!({})).unwrap();
    assert_eq!(available["available"], false);
    assert!(available["reason"].as_str().unwrap().contains(reason), "{available}");
    assert!(s.run("table.joinNext", &json!({})).is_err());
    assert_eq!(s.doc, before);
    assert_eq!(s.sel, selection);
    assert_eq!(s.dirty, dirty);
    assert!(!s.can_undo(), "refusal leaves no history");
}

#[test]
fn adjacent_tables_become_one_real_table_and_one_history_step() {
    for separator in [false, true] {
        let mut s = fixture(separator);
        let before = s.doc.clone();
        let selection = s.sel.clone();
        let available = s.run("table.joinNextStatus", &json!({})).unwrap();
        assert_eq!(available["available"], true);
        assert_eq!(s.doc, before);
        assert_eq!(s.sel, selection);
        assert!(!s.dirty);
        join(&mut s);
        assert_eq!(s.doc.body.len(), 2);
        let t = s.doc.table(StoryRef::Body, &Path::top(0)).unwrap();
        assert_eq!(t.rows.len(), 4);
        assert_eq!(&t.rows[..2], &before.table(StoryRef::Body, &Path::top(0)).unwrap().rows);
        assert_eq!(&t.rows[2..], &before.table(StoryRef::Body, &Path::top(if separator { 2 } else { 1 })).unwrap().rows);
        assert_eq!(s.sel.focus.path, Path(vec![0, 2, 0, 0]));
        let joined = s.doc.clone();
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc, before);
        assert_eq!(s.sel, selection);
        assert!(!s.can_undo());
        s.run("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc, joined);
    }
}

#[test]
fn split_then_join_restores_content_and_does_not_make_deletion_join_tables() {
    let mut s = fixture(false);
    s.doc.body.remove(1);
    let before = s.doc.clone();
    s.run("table.splitTable", &json!({})).unwrap();
    assert_eq!(s.doc.body.len(), 4);
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0]), 0));
    join(&mut s);
    assert_eq!(s.doc, before);
}

#[test]
fn join_preserves_merged_cells_nested_tables_and_direct_formatting() {
    let mut s = fixture(true);
    let t = s.doc.table_mut(StoryRef::Body, &Path::top(2)).unwrap();
    t.merge(0, 0, 0, 1);
    t.rows[0].cells[0].props.shading = Some(Rgb(30, 60, 90));
    // Keep the same table start origin while testing preserved direct cell properties.
    t.rows[0].cells[0].props.borders = Some(Borders { left: Some(Border::single(0.5)), ..Default::default() });
    t.rows[0].cells[0].props.margins = Some([2.0, 3.0, 4.0, 5.0]);
    t.rows[0].cells[0].blocks.insert(0, Arc::new(Block::Table(table(1, "nested"))));
    let before = t.clone();
    join(&mut s);
    let joined = s.doc.table(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(&joined.rows[2..], &before.rows);
    assert_eq!(s.sel.focus.path, Path(vec![0, 2, 0, 0, 0, 0, 0]));
}

#[test]
fn join_rejects_incompatible_settings_and_boundary_flags_without_history() {
    for n in 0..8 {
        let mut s = fixture(false);
        let t = s.doc.table_mut(StoryRef::Body, &Path::top(1)).unwrap();
        let why = match n {
            0 => {
                t.grid[0] += 1.0;
                "column grids"
            }
            1 => {
                t.props.align = Some(Align::Right);
                "table settings"
            }
            2 => {
                t.props.fixed = false;
                "table settings"
            }
            3 => {
                t.rows[0].props.header = true;
                "repeated header"
            }
            4 => {
                t.rows[0].cells[0].props.vmerge = VMerge::Continue;
                "vertical merge"
            }
            5 => {
                t.props.caption = Some("A caption".into());
                "captions"
            }
            6 => {
                t.props.float = Some(Default::default());
                "floating"
            }
            _ => {
                t.rows[0].cells[0].props.span = 2;
                "column grid"
            }
        };
        assert_refused(&mut s, why);
    }
}

#[test]
fn join_does_not_skip_real_or_meaningful_empty_content() {
    for n in 0..7 {
        let mut s = fixture(true);
        let p = s.doc.para_mut(StoryRef::Body, &Path::top(1)).unwrap();
        match n {
            0 => {
                p.insert_text(0, "caption", &CharProps::default()).unwrap();
            }
            1 => p.props.page_break_before = Some(true),
            2 => p.section = Some(Box::default()),
            3 => p.mark.bold = Some(true),
            4 => p.props.style = Some("Heading1".into()),
            5 => {
                p.insert_object(0, InlineObject::BookmarkStart { name: "keep".into() }, &CharProps::default()).unwrap();
            }
            _ => {
                s.doc.body.insert(1, para_block(Paragraph::new()));
            }
        }
        assert_refused(&mut s, if n == 6 { "no consecutive" } else { "plain empty" });
    }
}

#[test]
fn joining_does_not_cross_a_container_or_story() {
    let mut s = fixture(false);
    let part = s.doc.add_part(PartKind::Header, vec![Arc::new(Block::Table(table(1, "header"))), para_block(Paragraph::new())]);
    s.sel = Selection::caret(Pos::new(StoryRef::Part(part), Path(vec![0, 0, 0, 0]), 0));
    assert_refused(&mut s, "no consecutive");
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0]), 0));
    let cell = &mut s.doc.table_mut(StoryRef::Body, &Path::top(0)).unwrap().rows[0].cells[0];
    cell.blocks.insert(0, Arc::new(Block::Table(table(1, "nested"))));
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0, 0, 0, 0]), 0));
    assert_refused(&mut s, "plain empty");
}

#[test]
fn protected_tracked_and_controlled_content_is_unavailable() {
    for n in 0..5 {
        let mut s = fixture(false);
        let why = match n {
            0 => {
                s.doc.settings.protection = Some("readOnly".into());
                "protected"
            }
            1 => {
                s.doc.settings.track_changes = true;
                "tracked changes"
            }
            2 => {
                s.doc.revisions.push(Default::default());
                "tracked changes"
            }
            3 => {
                s.doc.para_mut(StoryRef::Body, &Path(vec![1, 0, 0, 0])).unwrap().mark.ins = Some(0);
                "tracked formatting"
            }
            _ => {
                s.doc
                    .para_mut(StoryRef::Body, &Path(vec![1, 0, 0, 0]))
                    .unwrap()
                    .insert_object(0, InlineObject::ControlEnd, &CharProps::default())
                    .unwrap();
                "content controls"
            }
        };
        assert_refused(&mut s, why);
    }
}

#[test]
fn availability_is_fresh_when_the_document_or_selection_changes() {
    let mut s = fixture(false);
    assert_eq!(s.run("table.joinNextStatus", &json!({})).unwrap()["available"], true);
    s.doc.body.insert(1, para_block(Paragraph::with_text("new intervening text", CharProps::default())));
    assert_refused(&mut s, "plain empty");
    s.sel = Selection::caret(Pos::body(1, 0));
    assert_refused(&mut s, "isn't in a table");
}

#[test]
fn docx_and_odt_reopen_as_one_table_with_kept_cell_content() {
    for name in ["joined.docx", "joined.odt"] {
        let mut s = fixture(true);
        join(&mut s);
        let bytes = io::save_bytes(name, &s.doc).unwrap();
        let reopened = io::open_bytes(name, &bytes).unwrap();
        let tables: Vec<_> = reopened.body.iter().filter_map(|b| b.as_table()).collect();
        assert_eq!(tables.len(), 1, "{name}");
        assert_eq!(tables[0].rows.len(), 4, "{name}");
        for (ri, row) in tables[0].rows.iter().enumerate() {
            for (ci, cell) in row.cells.iter().enumerate() {
                assert_eq!(cell.blocks[0].as_para().unwrap().text, format!("{}-{}-{ci}", if ri < 2 { "first" } else { "second" }, ri % 2), "{name}");
            }
        }
    }
}

#[test]
fn automatic_sizing_is_not_silently_frozen_or_redistributed() {
    let mut s = fixture(false);
    for i in [0, 1] {
        s.doc.table_mut(StoryRef::Body, &Path::top(i)).unwrap().props.fixed = false;
    }
    assert_refused(&mut s, "automatic");
    assert!(!s.doc.table(StoryRef::Body, &Path::top(0)).unwrap().props.fixed);
}

#[test]
fn preferred_cell_widths_must_not_replace_an_identical_declared_grid() {
    let mut s = fixture(false);
    for row in &mut s.doc.table_mut(StoryRef::Body, &Path::top(0)).unwrap().rows {
        row.cells[0].props.width = Some(100.0);
        row.cells[1].props.width = Some(200.0);
    }
    assert_refused(&mut s, "preferred cell widths");
}

#[test]
fn first_cell_properties_must_not_move_the_joined_table_origin() {
    let mut s = fixture(false);
    s.doc.table_mut(StoryRef::Body, &Path::top(1)).unwrap().rows[0].cells[0].props.borders =
        Some(Borders { left: Some(Border::single(4.0)), ..Default::default() });
    assert_refused(&mut s, "origins");
    let mut s = fixture(false);
    s.doc.settings.compat_mode = 14;
    s.doc.table_mut(StoryRef::Body, &Path::top(1)).unwrap().rows[0].cells[0].props.margins = Some([0.0, 20.0, 0.0, 5.0]);
    assert_refused(&mut s, "origins");
}

#[test]
fn boundary_borders_are_not_silently_changed() {
    let mut s = fixture(false);
    s.doc.table_mut(StoryRef::Body, &Path::top(1)).unwrap().rows[0].cells[0].props.borders =
        Some(Borders { top: Some(Border::single(2.0)), ..Default::default() });
    assert_refused(&mut s, "boundary differ");
    let mut s = fixture(false);
    for i in [0, 1] {
        s.doc.table_mut(StoryRef::Body, &Path::top(i)).unwrap().props.borders =
            Some(Borders { top: Some(Border::single(2.0)), ..Default::default() });
    }
    assert_refused(&mut s, "horizontal table borders");
}

#[test]
fn repeated_headers_and_vertical_merges_have_valid_joined_boundaries() {
    let mut s = fixture(false);
    for i in [0, 1] {
        s.doc.table_mut(StoryRef::Body, &Path::top(i)).unwrap().merge(0, 1, 0, 0);
    }
    s.doc.table_mut(StoryRef::Body, &Path::top(0)).unwrap().rows[0].props.header = true;
    let before = s.doc.clone();
    join(&mut s);
    let t = s.doc.table(StoryRef::Body, &Path::top(0)).unwrap();
    assert_eq!(t.vertical_extent(0, 0), Some((0, 2)));
    assert_eq!(t.vertical_extent(2, 0), Some((2, 2)), "the second merge does not continue the first");
    assert_eq!(t.rows.iter().map(|r| r.props.header).collect::<Vec<_>>(), vec![true, false, false, false]);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc, before);
}

fn push(p: &mut Paragraph, o: InlineObject) {
    p.insert_object(p.len(), o, &CharProps::default()).unwrap();
}

#[test]
fn relationships_embedded_media_and_reference_fields_follow_the_moved_rows() {
    let mut s = fixture(false);
    let comment_part = s.doc.add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Review", CharProps::default()))]);
    s.doc.comments.insert(7, wordcraft_doc::Comment { part: comment_part, author: "Synthetic Reviewer".into(), ..Default::default() });
    let note = s.doc.add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Synthetic note", CharProps::default()))]);
    let im = image::RgbaImage::from_pixel(2, 2, image::Rgba([40, 80, 120, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    im.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let key = s.doc.add_media(bytes.into_inner(), "png");
    let p = s.doc.para_mut(StoryRef::Body, &Path(vec![1, 0, 0, 0])).unwrap();
    p.insert_object(0, InlineObject::BookmarkStart { name: "MovedBookmark".into() }, &CharProps::default()).unwrap();
    push(p, InlineObject::BookmarkEnd { name: "MovedBookmark".into() });
    push(p, InlineObject::CommentStart { id: 7 });
    push(p, InlineObject::CommentEnd { id: 7 });
    push(p, InlineObject::NoteRef { kind: wordcraft_doc::para::NoteKind::Footnote, id: note, custom: String::new() });
    push(p, serde_json::from_value(json!({"type":"image","media":key,"w":12,"h":12,"alt":"Synthetic image"})).unwrap());
    let f = serde_json::from_value(json!({"type":"field","instr":"REF MovedBookmark","result":"cached"})).unwrap();
    push(s.doc.para_mut(StoryRef::Body, &Path::top(2)).unwrap(), f);
    let before = s.doc.clone();
    join(&mut s);
    assert_eq!(s.doc.parts, before.parts);
    assert_eq!(s.doc.comments, before.comments);
    assert_eq!(s.doc.media, before.media);
    assert_eq!(s.doc.para(StoryRef::Body, &Path(vec![0, 2, 0, 0])), before.para(StoryRef::Body, &Path(vec![1, 0, 0, 0])));
    s.run("references.updateFields", &json!({})).unwrap();
    let tail = s.doc.para(StoryRef::Body, &Path::top(1)).unwrap();
    assert!(tail.objects.iter().any(|o| matches!(o,InlineObject::Field{result,..} if result=="second-0-0")));
    let reopened = io::open_bytes("attached.docx", &io::save_bytes("attached.docx", &s.doc).unwrap()).unwrap();
    assert!(reopened.comments.contains_key(&7));
    let p = reopened.para(StoryRef::Body, &Path(vec![0, 2, 0, 0])).unwrap();
    assert!(p.objects.iter().any(|o| matches!(o,InlineObject::BookmarkStart{name} if name=="MovedBookmark")));
    assert!(p.objects.iter().any(|o| matches!(o, InlineObject::NoteRef { .. })));
    assert!(p.objects.iter().any(|o| matches!(o, InlineObject::Image { .. })));
}

#[test]
fn caption_formulas_and_imported_opaque_markup_are_declined() {
    let mut s = fixture(false);
    s.doc.body.insert(0, para_block(Paragraph::with_text("Table 1", CharProps::default()).styled("Caption")));
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![1, 0, 0, 0]), 0));
    assert_refused(&mut s, "captions");
    let mut s = fixture(false);
    let field = serde_json::from_value(json!({"type":"field","instr":"=SUM(ABOVE)","result":"0"})).unwrap();
    push(s.doc.para_mut(StoryRef::Body, &Path(vec![1, 0, 0, 0])).unwrap(), field);
    assert_refused(&mut s, "table formulas");
    let mut s = fixture(false);
    let saved = io::save_bytes("source.odt", &s.doc).unwrap();
    s = Session::new(io::open_bytes("source.odt", &saved).unwrap());
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0]), 0));
    assert_refused(&mut s, "preserved");
}

#[test]
fn multipage_join_keeps_headers_and_exports_one_parent_table() {
    let mut d = Document::new();
    let mut a = table(5, "first");
    a.rows[0].props.header = true;
    let mut b = table(70, "second");
    for row in &mut b.rows {
        for cell in &mut row.cells {
            cell.blocks[0] = para_block(Paragraph::with_text("Body words on several lines. ".repeat(4).as_str(), CharProps::default()));
        }
    }
    d.body = vec![Arc::new(Block::Table(a)), para_block(Paragraph::new()), Arc::new(Block::Table(b)), para_block(Paragraph::new())];
    let mut s = Session::new(d);
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0]), 0));
    join(&mut s);
    let pages = s.layout().pages.clone();
    assert!(pages.len() > 2);
    let count =
        pages.iter().filter(|p| p.items.iter().any(|i| matches!(i,wordcraft_layout::Placed::Lines{path,..} if path.0.starts_with(&[0,0])))).count();
    assert!(count > 1, "the first table's header repeats after the joined boundary");
    assert!(io::save_bytes("joined.pdf", &s.doc).unwrap().starts_with(b"%PDF-"));
    let d = io::open_bytes("joined.docx", &io::save_bytes("joined.docx", &s.doc).unwrap()).unwrap();
    assert_eq!(d.body.iter().filter(|b| b.as_table().is_some()).count(), 1);
    assert_eq!(d.table(StoryRef::Body, &Path::top(0)).unwrap().rows.len(), 75);
    if let Ok(dir) = std::env::var("WORDCRAFT_PROOF_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(format!("{dir}/joined-tables.pdf"), io::save_bytes("joined.pdf", &s.doc).unwrap()).unwrap();
    }
}

#[test]
fn repeated_headers_and_multipage_reflow_match_a_fresh_layout() {
    let mut s = fixture(false);
    let first = s.doc.table_mut(StoryRef::Body, &Path::top(0)).unwrap();
    first.rows[0].props.header = true;
    first.rows[0].props.height = Some(22.0);
    let second = s.doc.table_mut(StoryRef::Body, &Path::top(1)).unwrap();
    second.rows = table(70, "long").rows;
    for row in &mut second.rows {
        row.props.height = Some(25.0);
    }
    let _warm = s.layout();
    join(&mut s);
    let joined = s.doc.clone();
    let warm = s.layout();
    let cold = Session::new(joined.clone()).layout();
    assert!(warm.pages.len() > 1);
    assert_eq!(warm.pages.len(), cold.pages.len());
    assert_eq!(warm.index, cold.index);
    let header_pages: std::collections::BTreeSet<_> =
        warm.index.get(&(StoryRef::Body, Path(vec![0, 0, 0, 0]))).unwrap().iter().map(|(page, _)| *page).collect();
    assert_eq!(header_pages.len(), warm.pages.len(), "the first table's header repeats on each page");
    for (a, b) in warm.pages.iter().zip(&cold.pages) {
        assert_eq!(format!("{:?}", a.items), format!("{:?}", b.items));
    }
    // A PDF is validated independently of DOCX/ODT reopening.
    let pdf = io::save_bytes("joined.pdf", &joined).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.body.len(), 3);
    s.run("edit.redo", &json!({})).unwrap();
    assert_eq!(s.doc, joined);
    assert_eq!(s.layout().pages.len(), warm.pages.len());
}

#[test]
fn row_dependent_styles_are_rejected_instead_of_silently_restyled() {
    let mut s = fixture(false);
    for i in [0, 1] {
        s.doc.table_mut(StoryRef::Body, &Path::top(i)).unwrap().props.style = Some("PlainTable1".into());
    }
    assert_refused(&mut s, "row-dependent");
}

#[test]
fn two_nested_tables_join_only_inside_their_shared_cell() {
    let mut s = fixture(false);
    let cell = &mut s.doc.table_mut(StoryRef::Body, &Path::top(0)).unwrap().rows[0].cells[0];
    cell.blocks = vec![
        Arc::new(Block::Table(table(1, "nested-a"))),
        para_block(Paragraph::new()),
        Arc::new(Block::Table(table(1, "nested-b"))),
        para_block(Paragraph::new()),
    ];
    s.sel = Selection::caret(Pos::new(StoryRef::Body, Path(vec![0, 0, 0, 0, 0, 0, 0]), 0));
    let outside = s.doc.body[1].clone();
    join(&mut s);
    assert_eq!(s.doc.body[1], outside);
    let joined = s.doc.table(StoryRef::Body, &Path(vec![0, 0, 0, 0])).unwrap();
    assert_eq!(joined.rows.len(), 2);
    assert_eq!(s.sel.focus.path, Path(vec![0, 0, 0, 0, 1, 0, 0]));
}

#[test]
fn refused_join_preserves_existing_history_and_redo() {
    let mut s = fixture(false);
    s.run("format.bold", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.can_redo());
    s.doc.table_mut(StoryRef::Body, &Path::top(1)).unwrap().grid[0] += 1.0;
    let before = (s.doc.clone(), s.sel.clone(), s.undo_labels(), s.redo_label().map(str::to_string), s.dirty);
    assert!(s.run("table.joinNext", &json!({})).is_err());
    assert_eq!((s.doc.clone(), s.sel.clone(), s.undo_labels(), s.redo_label().map(str::to_string), s.dirty), before);
}

#[test]
fn an_inherited_page_break_does_not_make_a_removable_empty_separator() {
    let mut s = fixture(true);
    Arc::make_mut(&mut s.doc.styles).get_mut("Normal").unwrap().para.page_break_before = Some(true);
    assert_refused(&mut s, "plain empty");
}
