use super::*;

#[test]
fn compare_marks_changes() {
    let mut s = Session::new(Document::from_text("The cat sat.\nKeep this.\nOld line."));
    let r = s.run("review.compare", &json!({"text": "The dog sat.\nKeep this.\nNew line here."})).unwrap();
    assert!(r["changes"].as_u64().unwrap() >= 2);
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The dog sat.\nKeep this.\nNew line here.");
}

/// A document's body as compared: text, runs (without revision marks), objects (pictures by
/// their bytes, not their media key), paragraph style and list definition, tables cell by cell.
fn shape(doc: &Document) -> Value {
    fn blocks(doc: &Document, bl: &[Arc<Block>]) -> Value {
        Value::Array(bl.iter().map(|b| block(doc, b)).collect())
    }
    fn block(doc: &Document, b: &Block) -> Value {
        match b {
            Block::Para(p) => {
                let runs: Vec<Value> = p
                    .runs
                    .iter()
                    .map(|r| json!([r.len, serde_json::to_value(CharProps { ins: None, del: None, fmt_change: None, ..r.props.clone() }).unwrap()]))
                    .collect();
                let objects: Vec<Value> = p
                    .objects
                    .iter()
                    .map(|o| {
                        let mut v = serde_json::to_value(o).unwrap();
                        if let InlineObject::Image { media, .. } = o {
                            v["media"] = json!(doc.media.get(media).map(|d| d.to_vec()));
                        }
                        v
                    })
                    .collect();
                let list = p
                    .props
                    .numbering
                    .as_ref()
                    .map(|n| json!([n.level, doc.numbering.abstract_of(n.num).map(|a| serde_json::to_value(&a.levels).unwrap())]));
                json!({"text": p.text, "runs": runs, "objects": objects, "style": p.props.style, "list": list})
            }
            Block::Table(t) => Value::Array(t.rows.iter().map(|r| Value::Array(r.cells.iter().map(|c| blocks(doc, &c.blocks)).collect())).collect()),
        }
    }
    blocks(doc, &doc.body)
}

/// Compare `original` with `revised`, then check Accept All gives the revised document and
/// Reject All the original. Returns the compare result and a session on the compared document.
fn compare_round_trip(original: &Document, revised: &Document) -> (Value, Session) {
    let mut s = Session::new(original.clone());
    let r = compare_with(&mut s, revised.clone(), &json!({"author": "Cmp"})).unwrap();
    let result = s.doc.clone();
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(shape(&s.doc), shape(revised), "Accept All gives the revised document");
    let mut s = Session::new(result.clone());
    s.run("review.rejectAll", &json!({})).unwrap();
    assert_eq!(shape(&s.doc), shape(original), "Reject All gives the original document");
    (r, Session::new(result))
}

fn picture(media: &str) -> InlineObject {
    InlineObject::Image { media: media.into(), w: 20.0, h: 10.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None }
}

fn para_with_object(text: &str, obj: InlineObject) -> Arc<Block> {
    let mut p = Paragraph::with_text(text, CharProps::default());
    let end = p.len();
    p.insert_object(end, obj, &CharProps::default()).unwrap();
    para_block(p)
}

fn para_mut(b: &mut Arc<Block>) -> &mut Paragraph {
    match Arc::make_mut(b) {
        Block::Para(p) => p,
        Block::Table(_) => panic!("not a paragraph"),
    }
}

/// #1317: a picture the revised document stores under a key the original uses for another
/// picture shows the revised picture where it is inserted, the original one where kept.
#[test]
fn compare_imports_pictures_whose_keys_collide() {
    let mut original = Document::from_text("Intro\nOutro");
    original.media.insert("image1.png".into(), Arc::new(vec![1, 1, 1]));
    original.body.insert(1, para_with_object("Kept ", picture("image1.png")));
    let mut revised = Document::from_text("Intro\nOutro");
    revised.media.insert("image2.png".into(), Arc::new(vec![1, 1, 1]));
    revised.media.insert("image1.png".into(), Arc::new(vec![2, 2, 2]));
    revised.body.insert(1, para_with_object("Kept ", picture("image2.png")));
    revised.body.insert(2, para_with_object("New ", picture("image1.png")));
    let (_, s) = compare_round_trip(&original, &revised);
    let bytes = |b: &Block| match b.as_para().and_then(|p| p.objects.first()) {
        Some(InlineObject::Image { media, .. }) => s.doc.media.get(media).map(|d| d.to_vec()),
        _ => None,
    };
    assert_eq!(bytes(&s.doc.body[1]), Some(vec![1, 1, 1]), "the kept picture is the original's");
    assert_eq!(bytes(&s.doc.body[2]), Some(vec![2, 2, 2]), "the inserted picture is the revised one");
}

/// #1317: one changed word in a paragraph with a picture, a field and a bold run: only the
/// word is marked; the objects and the formatting survive.
#[test]
fn compare_changed_paragraph_keeps_objects_and_formatting() {
    let build = |word: &str| {
        let mut d = Document::from_text("");
        d.media.insert("image1.png".into(), Arc::new(vec![7, 7]));
        let mut p = Paragraph::with_text("Hello ", CharProps::default());
        let n = p.len();
        p.insert_text(n, "brave", &CharProps { bold: Some(true), ..Default::default() }).unwrap();
        let n = p.len();
        p.insert_text(n, &format!(" {word} "), &CharProps::default()).unwrap();
        let n = p.len();
        p.insert_object(n, picture("image1.png"), &CharProps::default()).unwrap();
        let n = p.len();
        p.insert_text(n, " page ", &CharProps::default()).unwrap();
        let n = p.len();
        let field = InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() };
        p.insert_object(n, field, &CharProps::default()).unwrap();
        d.body = vec![para_block(p)];
        d
    };
    let (_, s) = compare_round_trip(&build("world"), &build("planet"));
    let p = s.doc.body[0].as_para().unwrap();
    let marked = |f: fn(&CharProps) -> bool| -> String { p.run_ranges().filter(|(_, c)| f(c)).map(|(r, _)| p.text[r].to_string()).collect() };
    assert_eq!(marked(|c| c.del.is_some()), "world ");
    assert_eq!(marked(|c| c.ins.is_some()), "planet ");
    assert_eq!(p.objects.len(), 2, "the picture and the field are kept");
    assert_eq!(marked(|c| c.bold == Some(true)), "brave");
}

/// #1317: whole paragraphs and tables added or removed are tracked with their paragraph
/// marks and rows, so resolving leaves no empty paragraphs or tables behind.
#[test]
fn compare_tracks_whole_paragraphs_and_tables() {
    let table = || {
        let mut t = wordcraft_doc::Table::new(2, 2, 200.0);
        for (i, c) in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).enumerate() {
            *c = wordcraft_doc::table::Cell::with_text(&format!("cell {i}"));
        }
        Arc::new(Block::Table(t))
    };
    let mut original = Document::from_text("Intro\nRemoved paragraph\nMiddle\nOutro");
    original.body.insert(2, table());
    let revised = Document::from_text("Intro\nMiddle\nAdded paragraph\nOutro\nTail paragraph");
    compare_round_trip(&original, &revised);
    // The other way round: a paragraph removed at the end, a table added.
    let mut revised = Document::from_text("Intro\nMiddle");
    revised.body.insert(1, table());
    let original = Document::from_text("Intro\nMiddle\nLast one");
    compare_round_trip(&original, &revised);
}

/// #1317: styles and lists only the revised document defines come with its content, and a
/// list id the original uses for another list doesn't take over.
#[test]
fn compare_imports_styles_and_lists() {
    use wordcraft_doc::numbering::ListKind;
    let mut original = Document::from_text("Intro\nItem one");
    let mine = original.numbering.add_list(ListKind::Bullet);
    para_mut(&mut original.body[1]).props.numbering = Some(NumRef { num: mine, level: 0 });
    let mut revised = Document::from_text("Intro\nItem one\nFancy text");
    let theirs = revised.numbering.add_list(ListKind::Numbered);
    assert_eq!(mine, theirs, "the two lists share an id");
    revised.styles.upsert(wordcraft_doc::styles::Style {
        id: "Fancy".into(),
        name: "Fancy".into(),
        kind: wordcraft_doc::styles::StyleKind::Paragraph,
        chr: CharProps { italic: Some(true), ..Default::default() },
        ..Default::default()
    });
    para_mut(&mut revised.body[1]).props.numbering = Some(NumRef { num: theirs, level: 0 });
    let fancy = para_mut(&mut revised.body[2]);
    fancy.props.style = Some("Fancy".into());
    fancy.props.numbering = Some(NumRef { num: theirs, level: 0 });
    let (_, s) = compare_round_trip(&original, &revised);
    assert_eq!(s.doc.styles.get("Fancy").and_then(|st| st.chr.italic), Some(true));
}

/// #1317: a footnote the revised document adds comes with its own story; a reference to a
/// note the revised document lacks is left out and reported, never pointed at one of ours.
#[test]
fn compare_imports_notes_and_reports_what_it_drops() {
    use wordcraft_doc::{PartKind, para::NoteKind};
    let mut original = Document::from_text("Intro");
    let ours = original.try_add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Our note", CharProps::default()))]).unwrap();
    let mut revised = Document::from_text("Intro");
    let theirs = revised.try_add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("Their note", CharProps::default()))]).unwrap();
    assert_eq!(ours, theirs, "the two notes share an id");
    revised.body.push(para_with_object("Noted", InlineObject::NoteRef { kind: NoteKind::Footnote, id: theirs, custom: String::new() }));
    revised.body.push(para_with_object("Lost", InlineObject::NoteRef { kind: NoteKind::Footnote, id: 999, custom: String::new() }));
    let mut s = Session::new(original);
    let r = compare_with(&mut s, revised, &json!({})).unwrap();
    let note = s.doc.body[1].as_para().and_then(|p| match p.objects.first() {
        Some(InlineObject::NoteRef { id, .. }) => Some(*id),
        _ => None,
    });
    let note = note.expect("the note reference is kept");
    assert_ne!(note, ours);
    assert_eq!(s.doc.plain_text(StoryRef::Part(note)), "Their note");
    assert_eq!(s.doc.plain_text(StoryRef::Part(ours)), "Our note");
    assert!(s.doc.body[2].as_para().is_some_and(|p| p.objects.is_empty() && p.text == "Lost"));
    assert_eq!(r["dropped"][0]["what"], "noteReference");
    assert_eq!(r["dropped"][0]["count"], 1);
}

/// A document with a footnote, a comment and a text box, and a first paragraph to edit.
fn rich_doc(first: &str) -> Document {
    use wordcraft_doc::{Comment, PartKind, para::NoteKind};
    let plain = CharProps::default();
    let mut d = Document::from_text(&format!("{first}\nOutro"));
    let note = d.try_add_part(PartKind::Footnote, vec![para_block(Paragraph::with_text("A note", plain.clone()))]).unwrap();
    d.body.insert(1, para_with_object("Noted", InlineObject::NoteRef { kind: NoteKind::Footnote, id: note, custom: String::new() }));
    let part = d.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("A comment", plain.clone()))]).unwrap();
    d.comments.insert(0, Comment { author: "Ann".into(), part, ..Default::default() });
    let mut p = Paragraph::new();
    p.insert_object(0, InlineObject::CommentStart { id: 0 }, &plain).unwrap();
    let n = p.len();
    p.insert_text(n, "Commented", &plain).unwrap();
    let n = p.len();
    p.insert_object(n, InlineObject::CommentEnd { id: 0 }, &plain).unwrap();
    d.body.insert(2, para_block(p));
    let story = d.try_add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("Boxed", plain))]).unwrap();
    let shape = InlineObject::Shape {
        kind: Default::default(),
        w: 50.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.body.insert(3, para_with_object("Box ", shape));
    d
}

/// Text marked inserted or deleted in the body (with paragraph marks as "¶").
fn tracked(doc: &Document, ins: bool) -> String {
    let mut out = String::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for (r, c) in p.run_ranges() {
            if (ins && c.ins.is_some()) || (!ins && c.del.is_some()) {
                out.push_str(&p.text[r]);
            }
        }
        if (ins && p.mark.ins.is_some()) || (!ins && p.mark.del.is_some()) {
            out.push('¶');
        }
    }
    out
}

/// #1317 review: footnotes, comments and text boxes the two documents share are no change,
/// and comparing doesn't leave copies of them behind.
#[test]
fn compare_shared_notes_comments_and_text_boxes_are_unchanged() {
    let original = rich_doc("Intro words here");
    let mut s = Session::new(original.clone());
    let r = compare_with(&mut s, original.clone(), &json!({})).unwrap();
    assert_eq!(r["changes"], 0);
    assert_eq!((tracked(&s.doc, true), tracked(&s.doc, false)), (String::new(), String::new()));
    assert_eq!((s.doc.parts.len(), s.doc.comments.len()), (original.parts.len(), 1));
    // One word changed elsewhere: only that word is marked.
    let mut s = Session::new(original.clone());
    let r = compare_with(&mut s, rich_doc("Intro lines here"), &json!({})).unwrap();
    assert_eq!(r["changes"], 1);
    assert_eq!((tracked(&s.doc, true), tracked(&s.doc, false)), ("lines ".to_string(), "words ".to_string()));
    assert_eq!((s.doc.parts.len(), s.doc.comments.len()), (original.parts.len(), 1));
}

fn cells_table() -> Arc<Block> {
    let mut t = wordcraft_doc::Table::new(2, 2, 200.0);
    for (i, c) in t.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).enumerate() {
        *c = wordcraft_doc::table::Cell::with_text(&format!("cell {i}"));
    }
    Arc::new(Block::Table(t))
}

/// Select the whole table at body index `i` (2 × 2).
fn select_table(s: &mut Session, i: u32) {
    let end = s.doc.para(StoryRef::Body, &wordcraft_doc::Path(vec![i, 1, 1, 0])).map(|p| p.len()).unwrap();
    s.sel = Selection {
        anchor: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![i, 0, 0, 0]), off: 0 },
        focus: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![i, 1, 1, 0]), off: end },
    };
}

/// #1317 review: a table deletion rejected (or an insertion accepted) by selection stays
/// resolved, so a later Accept All (Reject All) keeps the table.
#[test]
fn row_marks_resolved_by_selection_survive_resolve_all() {
    let mut original = Document::from_text("Intro\nOutro");
    original.body.insert(1, cells_table());
    let mut s = Session::new(original);
    compare_with(&mut s, Document::from_text("Intro\nOutro"), &json!({})).unwrap();
    select_table(&mut s, 1);
    s.run("review.reject", &json!({})).unwrap();
    s.run("review.acceptAll", &json!({})).unwrap();
    assert!(matches!(s.doc.body.get(1).map(|b| &**b), Some(Block::Table(_))), "the kept table stays");
    let mut revised = Document::from_text("Intro\nOutro");
    revised.body.insert(1, cells_table());
    let mut s = Session::new(Document::from_text("Intro\nOutro"));
    compare_with(&mut s, revised, &json!({})).unwrap();
    select_table(&mut s, 1);
    s.run("review.accept", &json!({})).unwrap();
    s.run("review.rejectAll", &json!({})).unwrap();
    assert!(matches!(s.doc.body.get(1).map(|b| &**b), Some(Block::Table(_))), "the accepted table stays");
}

/// #1317 review: a table whose text is the same but whose formatting changed is tracked, so
/// Reject All gives back the original table.
#[test]
fn compare_tracks_a_table_whose_formatting_changed() {
    let mut original = Document::from_text("Intro\nOutro");
    original.body.insert(1, cells_table());
    let mut revised = original.clone();
    if let Block::Table(t) = Arc::make_mut(&mut revised.body[1]) {
        let p = match Arc::make_mut(&mut t.rows[0].cells[0].blocks[0]) {
            Block::Para(p) => p,
            Block::Table(_) => panic!("not a paragraph"),
        };
        let n = p.len();
        p.format(0, n, &|c| c.bold = Some(true)).unwrap();
    }
    compare_round_trip(&original, &revised);
}

/// #1317 review: a chart from the revised document whose package part has the same name as a
/// different part of the original's can't be brought over: its kept source is left out and
/// reported, never pointed at the original's part.
#[test]
fn compare_drops_embedded_parts_whose_names_collide() {
    use wordcraft_doc::graphic::{Embedded, EmbeddedRef, Graphic};
    let chart = |items: usize| InlineObject::Graphic {
        w: 10.0,
        h: 10.0,
        alt: String::new(),
        float: Default::default(),
        graphic: Arc::new(Graphic {
            w: items as f32,
            source: Some(Arc::new(Embedded {
                xml: "<a:graphic/>".into(),
                refs: vec![EmbeddedRef { id: "rId1".into(), kind: "chart".into(), target: "word/charts/chart1.xml".into(), external: false }],
            })),
            ..Default::default()
        }),
    };
    let mut original = Document::from_text("Intro\nOutro");
    original.passthrough.insert("word/charts/chart1.xml".into(), Arc::new(b"A".to_vec()));
    original.body.insert(1, para_with_object("Old chart ", chart(1)));
    let mut revised = Document::from_text("Intro\nOutro");
    revised.passthrough.insert("word/charts/chart1.xml".into(), Arc::new(b"B".to_vec()));
    revised.body.insert(1, para_with_object("Old chart ", chart(1)));
    revised.body.insert(2, para_with_object("New chart ", chart(2)));
    let mut s = Session::new(original);
    let r = compare_with(&mut s, revised, &json!({})).unwrap();
    let source = |i: usize| match s.doc.body[i].as_para().and_then(|p| p.objects.first()) {
        Some(InlineObject::Graphic { graphic, .. }) => graphic.source.is_some(),
        _ => panic!("no chart"),
    };
    assert!(source(1), "the original's chart keeps its part");
    assert!(!source(2), "the inserted chart doesn't point at the original's part");
    assert!(r["dropped"].as_array().unwrap().iter().any(|d| d["what"] == "embeddedObject"), "{r}");
}

/// #1317 review (#11): a comment id at the top of the range in the original isn't overwritten
/// by a comment the revised document brings.
#[test]
fn compare_never_overwrites_a_comment_at_the_last_id() {
    let mut original = Document::from_text("Intro");
    let part = original.try_add_part(wordcraft_doc::PartKind::Comment, vec![para_block(Paragraph::with_text("Mine", CharProps::default()))]).unwrap();
    original.comments.insert(u32::MAX, wordcraft_doc::Comment { author: "Me".into(), part, ..Default::default() });
    let mut s = Session::new(original);
    compare_with(&mut s, rich_doc("Intro"), &json!({})).unwrap();
    assert_eq!(s.doc.comments.get(&u32::MAX).map(|c| c.author.as_str()), Some("Me"));
}

/// #1317 (N1): a comment whose range grew stays one comment, its start and end both there,
/// after Accept All.
#[test]
fn compare_keeps_a_comment_whose_range_grew_one_comment() {
    use wordcraft_doc::{Comment, PartKind};
    let build = |grown: bool| {
        let plain = CharProps::default();
        let mut d = Document::from_text("Intro");
        let part = d.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Remark", plain.clone()))]).unwrap();
        d.comments.insert(0, Comment { author: "Ann".into(), part, ..Default::default() });
        let mut p = Paragraph::with_text("A ", plain.clone());
        let items: Vec<(Option<InlineObject>, &str)> = if grown {
            vec![(Some(InlineObject::CommentStart { id: 0 }), "B C"), (Some(InlineObject::CommentEnd { id: 0 }), "")]
        } else {
            vec![(Some(InlineObject::CommentStart { id: 0 }), "B "), (Some(InlineObject::CommentEnd { id: 0 }), "C")]
        };
        for (obj, text) in items {
            if let Some(o) = obj {
                let n = p.len();
                p.insert_object(n, o, &plain).unwrap();
            }
            let n = p.len();
            p.insert_text(n, text, &plain).unwrap();
        }
        d.body.push(para_block(p));
        d
    };
    let mut s = Session::new(build(false));
    compare_with(&mut s, build(true), &json!({})).unwrap();
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.comments.len(), 1, "one comment");
    let id = *s.doc.comments.keys().next().unwrap();
    let p = s.doc.body[1].as_para().unwrap();
    let anchors: Vec<&InlineObject> =
        p.objects.iter().filter(|o| matches!(o, InlineObject::CommentStart { .. } | InlineObject::CommentEnd { .. })).collect();
    assert_eq!(anchors, [&InlineObject::CommentStart { id }, &InlineObject::CommentEnd { id }]);
    assert_eq!(p.plain_text(), "A B C");
}

/// #1317 (N2): rejecting the insertion of a row that holds a nested table resolves that row
/// once: the next row and its text stay.
#[test]
fn rejecting_a_row_with_a_nested_table_keeps_the_next_row() {
    let mut outer = wordcraft_doc::Table::new(2, 2, 200.0);
    for (i, c) in outer.rows.iter_mut().flat_map(|r| r.cells.iter_mut()).enumerate() {
        *c = wordcraft_doc::table::Cell::with_text(&format!("cell {i}"));
    }
    let mut inner = wordcraft_doc::Table::new(1, 1, 50.0);
    inner.rows[0].cells[0] = wordcraft_doc::table::Cell::with_text("nested");
    outer.rows[0].cells[1].blocks.insert(0, Arc::new(Block::Table(inner)));
    let mut revised = Document::from_text("Intro\nOutro");
    revised.body.insert(1, Arc::new(Block::Table(outer)));
    let mut s = Session::new(Document::from_text("Intro\nOutro"));
    compare_with(&mut s, revised, &json!({})).unwrap();
    let end = s.doc.para(StoryRef::Body, &wordcraft_doc::Path(vec![1, 0, 1, 1])).map(|p| p.len()).unwrap();
    s.sel = Selection {
        anchor: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, 0, 0, 0]), off: 0 },
        focus: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, 0, 1, 1]), off: end },
    };
    s.run("review.reject", &json!({})).unwrap();
    let t = s.doc.body.get(1).and_then(|b| b.as_table()).expect("the table stays");
    assert_eq!(t.rows.len(), 1);
    assert_eq!(block_text(&s.doc.body[1], 0), "cell 2\tcell 3");
}

/// A 2 × 2 table whose first column is one cell merged down both rows.
fn vmerged_table() -> Arc<Block> {
    use wordcraft_doc::props::VMerge;
    use wordcraft_doc::table::Cell;
    let mut t = wordcraft_doc::Table::new(2, 2, 200.0);
    t.rows[0].cells[0] = Cell::with_text("merged");
    t.rows[0].cells[0].props.vmerge = VMerge::Restart;
    t.rows[0].cells[1] = Cell::with_text("top");
    t.rows[1].cells[0].props.vmerge = VMerge::Continue;
    t.rows[1].cells[1] = Cell::with_text("bottom");
    Arc::new(Block::Table(t))
}

/// Select row `r` of the 2 × 2 table at body index 1.
fn select_row(s: &mut Session, r: u32) {
    let end = s.doc.para(StoryRef::Body, &wordcraft_doc::Path(vec![1, r, 1, 0])).map(|p| p.len()).unwrap();
    s.sel = Selection {
        anchor: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, r, 0, 0]), off: 0 },
        focus: Pos { story: StoryRef::Body, path: wordcraft_doc::Path(vec![1, r, 1, 0]), off: end },
    };
}

/// #1317 review: a row removed by resolving its change hands a vertical merge that starts in
/// it to the row below, which never keeps a merge with no start.
#[test]
fn resolving_a_row_keeps_vertical_merges_valid() {
    use wordcraft_doc::props::VMerge;
    let first_col = |s: &Session| {
        let t = s.doc.body.get(1).and_then(|b| b.as_table()).expect("the table stays");
        t.rows.iter().map(|r| r.cells[0].props.vmerge).collect::<Vec<_>>()
    };
    let mut revised = Document::from_text("Intro\nOutro");
    revised.body.insert(1, vmerged_table());
    // Reject All after the lower row was accepted by selection.
    let mut s = Session::new(Document::from_text("Intro\nOutro"));
    compare_with(&mut s, revised.clone(), &json!({})).unwrap();
    select_row(&mut s, 1);
    s.run("review.accept", &json!({})).unwrap();
    s.run("review.rejectAll", &json!({})).unwrap();
    assert_eq!(first_col(&s), [VMerge::None]);
    // The upper row rejected by selection.
    let mut s = Session::new(Document::from_text("Intro\nOutro"));
    compare_with(&mut s, revised, &json!({})).unwrap();
    select_row(&mut s, 0);
    s.run("review.reject", &json!({})).unwrap();
    assert_eq!(first_col(&s), [VMerge::None]);
}

/// #1317 review: comment marks inside a text box and inside a footnote the revised document
/// brings point at the revised comment's copy, never at the original comment with its id.
#[test]
fn compare_gives_comment_marks_in_text_boxes_and_notes_our_ids() {
    use wordcraft_doc::{Comment, PartKind, para::NoteKind};
    let plain = CharProps::default();
    let commented = |text: &str| {
        let mut p = Paragraph::new();
        p.insert_object(0, InlineObject::CommentStart { id: 0 }, &plain).unwrap();
        let n = p.len();
        p.insert_text(n, text, &plain).unwrap();
        let n = p.len();
        p.insert_object(n, InlineObject::CommentEnd { id: 0 }, &plain).unwrap();
        para_block(p)
    };
    let mut original = Document::from_text("Intro");
    let part = original.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Mine", plain.clone()))]).unwrap();
    original.comments.insert(0, Comment { author: "Me".into(), part, ..Default::default() });
    let mut revised = Document::from_text("Intro");
    let part = revised.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Theirs", plain.clone()))]).unwrap();
    revised.comments.insert(0, Comment { author: "Ann".into(), part, ..Default::default() });
    let story = revised.try_add_part(PartKind::TextBox, vec![commented("Boxed")]).unwrap();
    let shape = InlineObject::Shape {
        kind: Default::default(),
        w: 50.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    revised.body.push(para_with_object("Box ", shape));
    let note = revised.try_add_part(PartKind::Footnote, vec![commented("Noted")]).unwrap();
    revised.body.push(para_with_object("Note", InlineObject::NoteRef { kind: NoteKind::Footnote, id: note, custom: String::new() }));
    let mut s = Session::new(original);
    compare_with(&mut s, revised, &json!({})).unwrap();
    let first = |b: &Arc<Block>| b.as_para().and_then(|p| p.objects.first().cloned());
    let story = match first(&s.doc.body[1]) {
        Some(InlineObject::Shape { story: Some(id), .. }) => id,
        o => panic!("no text box: {o:?}"),
    };
    let note = match first(&s.doc.body[2]) {
        Some(InlineObject::NoteRef { id, .. }) => id,
        o => panic!("no note: {o:?}"),
    };
    for id in [story, note] {
        let marks: Vec<u32> = s.doc.parts[&id]
            .blocks
            .iter()
            .filter_map(|b| b.as_para())
            .flat_map(|p| p.objects.iter())
            .filter_map(|o| match o {
                InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(marks.len(), 2, "story {id} keeps both marks");
        for m in marks {
            assert_eq!(s.doc.comments.get(&m).map(|c| c.author.as_str()), Some("Ann"), "story {id}");
        }
    }
    assert_eq!(s.doc.comments.get(&0).map(|c| c.author.as_str()), Some("Me"));
    // A comment whose own text holds a mark of itself ends: the inner mark is left out.
    let mut revised = Document::from_text("Intro");
    let part = revised.try_add_part(PartKind::Comment, vec![commented("Loop")]).unwrap();
    revised.comments.insert(0, Comment { author: "Ann".into(), part, ..Default::default() });
    revised.body.push(commented("Text"));
    let mut s = Session::new(Document::from_text("Intro"));
    let r = compare_with(&mut s, revised, &json!({})).unwrap();
    assert_eq!(s.doc.comments.len(), 1);
    assert_eq!(r["dropped"][0]["what"], "commentAnchor", "{r}");
}

/// #1317 (N4): a row inserted or deleted whole is listed as a change, even with empty cells.
#[test]
fn row_insertions_are_listed_as_changes() {
    let mut revised = Document::from_text("Intro\nOutro");
    revised.body.insert(1, Arc::new(Block::Table(wordcraft_doc::Table::new(1, 2, 200.0))));
    let mut s = Session::new(Document::from_text("Intro\nOutro"));
    compare_with(&mut s, revised, &json!({})).unwrap();
    let list = s.run("review.changes", &json!({})).unwrap();
    assert!(list.as_array().unwrap().iter().any(|c| c["kind"] == "insert"), "{list}");
}

#[test]
fn compare_options_ignore_case_and_white_space() {
    let original = "The Cat sat.\nTwo  spaces here.";
    let revised = "The cat sat.\nTwo spaces here.";
    // By default both differences are changes.
    let mut s = Session::new(Document::from_text(original));
    let r = s.run("review.compare", &json!({"text": revised, "author": "Reviewer"})).unwrap();
    assert_eq!(r["changes"], 2);
    assert!(s.doc.revisions.iter().any(|x| x.author == "Reviewer"));
    // Ignoring case and white space: no changes, and the result reads as the revised text.
    let mut s = Session::new(Document::from_text(original));
    let r = s.run("review.compare", &json!({"text": revised, "caseChanges": false, "whiteSpace": false})).unwrap();
    assert_eq!(r["changes"], 0);
    assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
    // Character level marks only the changed letters.
    let mut s = Session::new(Document::from_text("colour"));
    s.run("review.compare", &json!({"text": "color", "level": "character"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "u");
    // Two dates that happen to share digits in a different order ("01.09.2026" vs
    // "19.10.2026") must come out as one clean replacement, not a scrambled chain of
    // single-character deletions and insertions wherever a digit coincidentally recurs.
    let mut s = Session::new(Document::from_text("zawarta w dniu 01.09.2026 pomiędzy:"));
    s.run("review.compare", &json!({"text": "zawarta w dniu 19.10.2026 pomiędzy:", "level": "character"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "01.09");
    assert_eq!(inserted, "19.10");
    assert_eq!(p.run_ranges().filter(|(_, c)| c.del.is_some()).count(), 1, "one clean deleted run, not several scattered ones");
    assert_eq!(p.run_ranges().filter(|(_, c)| c.ins.is_some()).count(), 1, "one clean inserted run, not several scattered ones");
    // Formatting differences become formatting changes.
    let mut s = Session::new(Document::from_text("Same words."));
    let mut rev = Document::from_text("Same words.");
    rev.format_range(&rev.start_of(StoryRef::Body), &rev.end_of(StoryRef::Body), &|c| c.bold = Some(true)).unwrap();
    let path = std::env::temp_dir().join(format!("wc-compare-{}.docx", std::process::id()));
    crate::io::save_path(&path, &rev).unwrap();
    let r = s.run("review.compare", &json!({"path": path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(r["changes"], 1);
    assert!(s.doc.revisions.iter().any(|x| x.kind == RevisionKind::Format));
}

#[test]
fn collapse_noise_matches_merges_short_interior_runs_but_keeps_prefix_suffix() {
    // "01.09" vs "19.10": a pure LCS finds a scrambled chain of single-character matches
    // ('1', '.', '0') threaded through what a reader expects to be one clean replacement.
    let ops = lcs_ops(&['0', '1', '.', '0', '9'], &['1', '9', '.', '1', '0'], &mut { u64::MAX });
    assert!(ops.contains(&Op::Same), "the raw LCS does find spurious matches here, or this test proves nothing");
    assert_eq!(collapse_noise_matches(&ops), vec![Op::Del, Op::Del, Op::Del, Op::Del, Op::Del, Op::Ins, Op::Ins, Op::Ins, Op::Ins, Op::Ins]);
    // A plain single-character replacement flanked by a real common prefix and suffix has no
    // interior match to begin with (the DP's middle is pure delete/insert), so it's untouched.
    let ops = lcs_ops(&['a', 'b', 'c', 'x', 'd', 'e', 'f'], &['a', 'b', 'c', 'y', 'd', 'e', 'f'], &mut { u64::MAX });
    assert_eq!(collapse_noise_matches(&ops), ops);
}

#[test]
fn compare_at_word_level_still_matches_a_whole_unchanged_word() {
    // `collapse_noise_matches` only runs for character-level compares (wired up in
    // `word_diff`): a single matched *word* between two edits is a real, meaningful match
    // (unlike a single matched character), and must never be folded into a replacement.
    let mut s = Session::new(Document::from_text("the cat sat"));
    s.run("review.compare", &json!({"text": "a cat ran"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    assert!(
        p.run_ranges().any(|(r, c)| c.del.is_none() && c.ins.is_none() && p.text[r].trim() == "cat"),
        "the unchanged word must survive as plain text"
    );
}

#[test]
fn compare_bounds_total_diff_work() {
    // Many long, slightly different paragraphs: each pair alone fits one comparison table,
    // but the work over all pairs must stay bounded. Pairs past the budget are compared
    // whole (deleted and inserted), and every pair still counts as a change.
    let words = 3000;
    let paras = 12;
    let doc = |rev: bool| -> String {
        (0..paras)
            .map(|p| (0..words).map(|w| if w % 10 == 0 && rev { format!("x{p}w{w}") } else { format!("p{p}w{w}") }).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (original, revised) = (doc(false), doc(true));
    let mut s = Session::new(Document::from_text(&original));
    let r = s.run("review.compare", &json!({"text": revised})).unwrap();
    assert_eq!(r["changes"], paras);
    // A pair diffed word by word keeps most of its text unmarked; one replaced whole keeps
    // only its common start and end.
    let word_diffed = |b: &Arc<Block>| {
        let p = b.as_para().unwrap();
        let same: usize = p.run_ranges().filter(|(_, c)| c.ins.is_none() && c.del.is_none()).map(|(r, _)| r.len()).sum();
        same * 2 > p.text.len()
    };
    assert!(word_diffed(&s.doc.body[0]));
    let diffed = s.doc.body.iter().filter(|b| word_diffed(b)).count() as u64;
    let cells = (words as u64) * (words as u64);
    assert!(diffed * cells <= 50_000_000 + cells, "{diffed} pairs diffed word by word");
    assert!(!word_diffed(s.doc.body.last().unwrap()));
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
}

#[test]
fn macros_record_and_run() {
    let mut s = Session::new(Document::new());
    s.run("tools.recordMacro", &json!({"name": "hi"})).unwrap();
    s.run("text.insert", &json!({"text": "Hi "})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("tools.recordMacro", &json!({})).unwrap();
    s.run("tools.macros", &json!({"run": "hi"})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hi Hi ");
    s.run("edit.repeat", &json!({})).unwrap();
}

#[test]
fn autocorrect_on_space() {
    let mut s = Session::new(Document::new());
    for t in ["teh", " ", "Teh", " ", "(c)", " "] {
        s.run("text.insert", &json!({"text": t})).unwrap();
    }
    // The first word of a sentence is capitalised too.
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The The © ");
}

#[test]
fn protection_blocks_edits() {
    let mut s = Session::new(Document::from_text("locked"));
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    // The "none" mode is the shorthand for stopping the protection, and it doesn't stop a
    // protected document: only the explicit stop does (#1327 follow-up).
    assert!(s.run("review.restrict", &json!({"mode": "none"})).is_err());
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    assert!(s.run("text.insert", &json!({"text": "x"})).is_ok());
}

#[test]
fn accessibility_and_quick_parts() {
    let mut s = Session::new(crate::sample::sample_document());
    let r = s.run("file.accessibility", &json!({})).unwrap();
    assert!(r["issues"].is_array());
    s.run("select.text", &json!({"text": "Membership"})).unwrap();
    s.run("insert.quickParts", &json!({"save": "m"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.quickParts", &json!({"insert": "m"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).ends_with("Membership"));
}

/// #407 Language: the selection's language and "do not check", the default language and
/// the detect preference; a bad tag is refused.
#[test]
fn proofing_language_options() {
    let mut s = Session::new(Document::from_text("Bonjour"));
    s.run("select.all", &json!({})).unwrap();
    s.run("review.language", &json!({"lang": "fr-FR", "noProof": true, "detect": false})).unwrap();
    let rc = s.doc.para_at(&Pos::body(0, 1)).map(|p| p.props_at(1).clone()).unwrap();
    assert_eq!((rc.lang.as_deref(), rc.no_proof), (Some("fr-FR"), Some(true)));
    assert!(!s.prefs.detect_language);
    s.run("review.language", &json!({"lang": "de-DE", "default": true})).unwrap();
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("de-DE"));
    assert!(s.run("review.language", &json!({"lang": "<script>"})).is_err());
}

/// The inspector's removals delete straight from the document, so they aren't tracked: on a
/// document that tracks changes the text would go with no revision to show for it, and the
/// revision trail would be broken behind the user's back. The removal is refused instead,
/// and the document is left as it was.
#[test]
fn inspect_removals_are_refused_while_the_document_tracks_changes() {
    let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
    s.run("select.text", &json!({"text": "Hidden"})).unwrap();
    s.run("format.hidden", &json!({"value": true})).unwrap();
    // Tracking without Restrict Editing isolates the Inspector guard from the protected
    // document check added separately in #3285.
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    assert!(s.doc.settings.track_changes, "the document doesn't track changes");
    let before = s.doc.clone();
    let err = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap_err();
    assert!(matches!(err, CmdError::Disabled(_)), "{err}");
    assert_eq!(s.doc, before, "the document changed despite the refusal");
    // Reading the document stays allowed.
    let found = s.run("file.inspect", &json!({})).unwrap();
    assert_eq!(found["found"]["hiddenText"], true, "{found}");
}

/// A document that doesn't track changes is inspected as before: the hidden text goes.
#[test]
fn inspect_removals_work_when_the_document_does_not_track_changes() {
    let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
    s.run("select.text", &json!({"text": "Hidden"})).unwrap();
    s.run("format.hidden", &json!({"value": true})).unwrap();
    assert!(!s.doc.settings.track_changes);
    let out = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap();
    assert_eq!(out["residual"], json!([]), "{out}");
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(!text.contains("Hidden"), "the hidden text is still there: {text}");
    assert!(text.contains("Visible text."), "{text}");
}
