//! Copying and pasting footnote references (#6713): the copy brings the notes, a paste makes new
//! ones of its own.

use super::*;
use crate::StoryRef;
use crate::para::NoteKind;

/// Bytes an object takes in a paragraph's text (U+FFFC).
const OBJ_LEN: usize = crate::para::OBJ.len_utf8();

fn doc(lines: &[&str]) -> Document {
    Document::from_text(&lines.join("\n"))
}

fn note_ref(id: u32) -> InlineObject {
    InlineObject::NoteRef { kind: NoteKind::Footnote, id, custom: String::new() }
}

/// A footnote story with `text`, starting with the note's own reference mark as the Footnote
/// command makes it. Returns its id; nothing refers to it yet.
fn make_note(d: &mut Document, text: &str) -> u32 {
    let id = d.add_part(PartKind::Footnote, Vec::new());
    let mut note = Paragraph::new();
    note.insert_object(0, note_ref(id), &CharProps::default()).unwrap();
    let len = note.len();
    note.insert_text(len, &format!(" {text}"), &CharProps::default()).unwrap();
    d.set_story(StoryRef::Part(id), vec![para_block(note)]).unwrap();
    id
}

/// A footnote with `text` anchored at byte `at` of the first body paragraph. Returns its id.
fn add_note(d: &mut Document, at: usize, text: &str) -> u32 {
    let id = make_note(d, text);
    d.insert_object(&Pos::body(0, at), note_ref(id), &CharProps::default()).unwrap();
    id
}

/// The note references of the paragraphs of `story`, in reading order.
fn refs_in(d: &Document, story: StoryRef) -> Vec<u32> {
    let mut out = Vec::new();
    for path in d.para_paths(story) {
        let Some(p) = d.para(story, &path) else { continue };
        for o in &p.objects {
            if let InlineObject::NoteRef { id, .. } = o {
                out.push(*id);
            }
        }
    }
    out
}

fn body_note_ids(d: &Document) -> Vec<u32> {
    refs_in(d, StoryRef::Body)
}

/// The id a note's own reference mark names (its story's first paragraph).
fn mark_id(d: &Document, note: u32) -> Option<u32> {
    refs_in(d, StoryRef::Part(note)).first().copied()
}

fn frag_refs(f: &Fragment) -> usize {
    let mut n = 0;
    for b in &f.blocks {
        each_para(b, 0, &mut |p| n += p.objects.iter().filter(|o| matches!(o, InlineObject::NoteRef { .. })).count());
    }
    n
}

fn box_shape(story: u32) -> InlineObject {
    InlineObject::Shape {
        kind: crate::para::ShapeKind::TextBox,
        w: 144.0,
        h: 72.0,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}

/// A copy that holds a footnote reference brings the note, and pasting it in the same document
/// makes a note of its own (not a second reference to the first one's story).
#[test]
fn copied_footnote_reference_pastes_as_a_new_note() {
    let mut d = doc(&["Alpha beta gamma"]);
    let a = add_note(&mut d, 5, "first note");
    let frag = d.copy_range(&Pos::body(0, 0), &Pos::body(0, 5 + OBJ_LEN));
    assert_eq!(frag.parts.keys().copied().collect::<Vec<_>>(), vec![a], "the note rides along");
    let at = d.end_of(StoryRef::Body);
    d.insert_fragment(&at, &frag).unwrap();
    let ids = body_note_ids(&d);
    assert_eq!(ids.len(), 2, "two references now");
    assert_eq!(ids[0], a);
    assert_ne!(ids[1], a, "the pasted reference has a story of its own");
    assert!(d.plain_text(StoryRef::Part(ids[1])).contains("first note"));
    assert!(d.plain_text(StoryRef::Part(a)).contains("first note"), "the original stays");
    assert_eq!(mark_id(&d, ids[1]), Some(ids[1]), "the copy's own mark names the copy");
    assert_eq!(mark_id(&d, a), Some(a));
}

/// Into another document the copy's story id means nothing (and may be another note's).
#[test]
fn copied_footnote_pastes_into_another_document_without_touching_its_notes() {
    let mut src = doc(&["Alpha beta"]);
    let a = add_note(&mut src, 5, "from the source gone");
    // Tracked changes in the source note: " gone" deleted, "source" inserted, "the" reformatted
    src.revisions.push(crate::Revision::new(crate::RevisionKind::Delete, "Source", ""));
    src.revisions.push(crate::Revision::new(crate::RevisionKind::Insert, "Source", ""));
    let note = src.para_mut(StoryRef::Part(a), &Path::top(0)).unwrap();
    let start = OBJ_LEN + " from the source".len();
    let end = note.len();
    note.format(start, end, &|c| c.del = Some(0)).unwrap();
    note.format(OBJ_LEN + " from the ".len(), start, &|c| c.ins = Some(1)).unwrap();
    note.format(OBJ_LEN + " from ".len(), OBJ_LEN + " from the".len(), &|c| c.fmt_change = crate::props::PropChange::boxed(1, CharProps::default()))
        .unwrap();
    let mut dst = doc(&["Other"]);
    let theirs = add_note(&mut dst, 5, "already here");
    assert_eq!(a, theirs, "the ids collide");
    let frag = src.copy_range(&Pos::body(0, 0), &Pos::body(0, 5 + OBJ_LEN));
    let at = dst.end_of(StoryRef::Body);
    dst.insert_fragment(&at, &frag).unwrap();
    let ids = body_note_ids(&dst);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], theirs);
    assert_ne!(ids[1], theirs);
    assert!(dst.plain_text(StoryRef::Part(theirs)).contains("already here"));
    assert!(!dst.plain_text(StoryRef::Part(theirs)).contains("from the source"));
    assert!(dst.plain_text(StoryRef::Part(ids[1])).contains("from the source"));
    assert_eq!(dst.parts[&ids[1]].kind, PartKind::Footnote);
    // The source's revision ids mean nothing here: the note comes as reviewed
    let copied = dst.para(StoryRef::Part(ids[1]), &Path::top(0)).unwrap();
    assert!(!copied.text.contains("gone"));
    assert!(copied.runs.iter().map(|r| &r.props).chain([&copied.mark]).all(|c| c.ins.is_none() && c.del.is_none() && c.fmt_change.is_none()));
}

/// A note referenced only from deleted text of a copied note is not copied: nothing would show it.
#[test]
fn note_named_in_deleted_note_text_is_not_copied() {
    let mut src = doc(&["Alpha beta"]);
    let outer = add_note(&mut src, 5, "outer");
    let inner = make_note(&mut src, "inner");
    src.revisions.push(crate::Revision::new(crate::RevisionKind::Delete, "Source", ""));
    let note = src.para_mut(StoryRef::Part(outer), &Path::top(0)).unwrap();
    let end = note.len();
    note.insert_object(end, note_ref(inner), &CharProps { del: Some(0), ..CharProps::default() }).unwrap();
    let frag = src.copy_range(&Pos::body(0, 0), &Pos::body(0, 5 + OBJ_LEN));
    let mut dst = doc(&["Other"]);
    let parts_before = dst.parts.len();

    let at = dst.end_of(StoryRef::Body);
    dst.insert_fragment(&at, &frag).unwrap();

    let ids = body_note_ids(&dst);
    assert_eq!(ids.len(), 1);
    assert_eq!(dst.parts.len(), parts_before + 1, "only the outer note gets a story");
    assert!(dst.plain_text(StoryRef::Part(ids[0])).contains("outer"));
}

/// A selection that ends before the reference, or starts after it, copies no note; one that
/// splits the reference's bytes doesn't panic.
#[test]
fn selection_beside_a_footnote_reference_copies_no_note() {
    let mut d = doc(&["Alpha beta"]);
    add_note(&mut d, 5, "note");
    let len = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();
    assert!(d.copy_range(&Pos::body(0, 0), &Pos::body(0, 5)).parts.is_empty());
    assert!(d.copy_range(&Pos::body(0, 5 + OBJ_LEN), &Pos::body(0, len)).parts.is_empty());
    assert_eq!(d.copy_range(&Pos::body(0, 5), &Pos::body(0, 5 + OBJ_LEN)).parts.len(), 1);
    for inside in 6..5 + OBJ_LEN {
        let _ = d.copy_range(&Pos::body(0, 0), &Pos::body(0, inside));
        let _ = d.copy_range(&Pos::body(0, inside), &Pos::body(0, len));
    }
    // Pasting a copy with no note adds no story.
    let before = d.parts.len();
    let frag = d.copy_range(&Pos::body(0, 5 + OBJ_LEN), &Pos::body(0, len));
    d.insert_fragment(&Pos::body(0, 0), &frag).unwrap();
    assert_eq!(d.parts.len(), before);
}

/// Text copied inside a note's own story leaves the note's own mark behind: pasting it would name
/// the note it came from.
#[test]
fn copy_inside_a_note_drops_the_notes_mark() {
    let mut d = doc(&["Alpha beta"]);
    let a = add_note(&mut d, 5, "note text");
    let story = StoryRef::Part(a);
    let frag = d.copy_range(&Pos { story, path: Path::top(0), off: 0 }, &d.end_of(story));
    assert!(frag.parts.is_empty());
    assert_eq!(frag_refs(&frag), 0, "no reference in the copy");
    assert!(frag.plain_text().contains("note text"));
    let before = d.parts.len();
    let at = d.end_of(StoryRef::Body);
    d.insert_fragment(&at, &frag).unwrap();
    assert_eq!(d.parts.len(), before, "pasting it makes no note");
    assert_eq!(body_note_ids(&d), vec![a], "and adds no reference");
    // Into another document its id means nothing, so it must not arrive.
    let mut dst = doc(&["Other"]);
    let theirs = add_note(&mut dst, 5, "theirs");
    let at = dst.end_of(StoryRef::Body);
    dst.insert_fragment(&at, &frag).unwrap();
    assert_eq!(body_note_ids(&dst), vec![theirs]);
}

/// A paste into a note's story leaves the references out: notes hold no notes.
#[test]
fn paste_into_a_note_leaves_the_references_out() {
    let mut d = doc(&["Alpha beta"]);
    let a = add_note(&mut d, 5, "note text");
    let frag = d.copy_range(&Pos::body(0, 0), &Pos::body(0, 5 + OBJ_LEN));
    assert_eq!(frag.parts.len(), 1);
    let before = d.parts.len();
    let at = d.end_of(StoryRef::Part(a));
    d.insert_fragment(&at, &frag).unwrap();
    assert_eq!(d.parts.len(), before);
    assert_eq!(refs_in(&d, StoryRef::Part(a)), vec![a], "only the note's own mark");
    assert_eq!(body_note_ids(&d), vec![a]);
    assert!(d.plain_text(StoryRef::Part(a)).contains("Alpha"), "the text still went in");
}

#[test]
fn note_reference_removal_scales_with_paragraph_length() {
    let cost = |count: usize| {
        let mut p = Paragraph::new();
        let bold = CharProps { bold: Some(true), ..CharProps::default() };
        for _ in 0..count {
            p.insert_object(p.len(), note_ref(1), &CharProps::default()).unwrap();
            p.insert_text(p.len(), "?", &bold).unwrap();
        }
        p.insert_object(p.len(), note_ref(2), &CharProps::default()).unwrap();
        let mut block = Block::Para(p);
        let steps = crate::work::count(|| {
            rewrite_note_refs(&mut block, &|id| if id == 1 { NoteMove::Remove } else { NoteMove::To(3) });
        });
        let p = block.as_para().unwrap();
        assert_eq!(p.text, format!("{}{OBJ}", "?".repeat(count), OBJ = crate::para::OBJ));
        assert_eq!(p.objects, vec![note_ref(3)]);
        assert_eq!(p.runs.len(), 2);
        assert_eq!(p.runs[0].len, "?".len() * count);
        assert_eq!(p.runs[0].props, bold);
        steps
    };
    assert!(cost(256) <= 5 * cost(64));
}

/// Several references to one note, and many notes: each note is copied once, every reference
/// follows its own copy, and the paste stays in step with the copy.
#[test]
fn many_notes_and_repeated_references_paste_cleanly() {
    let mut d = doc(&["x"]);
    let first = add_note(&mut d, 0, "n0");
    // A second reference to the same note.
    d.insert_object(&Pos::body(0, 0), note_ref(first), &CharProps::default()).unwrap();
    let notes = MAX_PASTED_BOXES + 1;
    for i in 1..notes {
        add_note(&mut d, 0, &format!("n{i}"));
    }
    // Each new reference went first, so the two to the first note come last.
    let len = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();
    let frag = d.copy_range(&Pos::body(0, 0), &Pos::body(0, len));
    assert_eq!(frag.parts.len(), notes);
    let parts_before = d.parts.len();
    let at = d.end_of(StoryRef::Body);
    let (_, omitted) = d.insert_fragment_with_note_report(&at, &frag).unwrap();
    assert_eq!(omitted, 0);
    assert_eq!(d.parts.len(), parts_before + notes);
    let ids = body_note_ids(&d);
    assert_eq!(ids.len(), (notes + 1) * 2);
    let (old, new) = ids.split_at(notes + 1);
    assert_eq!(&old[notes - 1..], &[first, first]);
    let old: std::collections::BTreeSet<_> = old.iter().copied().collect();
    assert!(new.iter().all(|id| !old.contains(id)), "no reference of the paste names an old note");
    assert_eq!(new[notes - 1], new[notes], "the two references to one note still name one story");
    for id in new {
        assert_eq!(mark_id(&d, *id), Some(*id));
    }
}

/// A Spike piece with a note keeps it when pieces are appended and pasted.
#[test]
fn appended_pieces_keep_their_notes() {
    let mut d = doc(&["Alpha beta"]);
    let a = add_note(&mut d, 5, "kept");
    let mut spike = d.copy_range(&Pos::body(0, 0), &Pos::body(0, 5 + OBJ_LEN));
    spike.append_piece(d.copy_range(&Pos::body(0, 0), &Pos::body(0, 5 + OBJ_LEN)));
    assert_eq!(spike.parts.len(), 2, "two pieces, two notes");
    assert!(spike.parts.values().all(|p| p.kind == PartKind::Footnote));
    let at = d.end_of(StoryRef::Body);
    d.insert_fragment(&at, &spike).unwrap();
    let refs = body_note_ids(&d);
    assert_eq!(refs.len(), 3);
    assert_eq!(refs[0], a);
    assert_ne!(refs[1], refs[2], "each piece's note is its own");
    for id in &refs[1..] {
        assert!(d.plain_text(StoryRef::Part(*id)).contains("kept"));
        assert_eq!(mark_id(&d, *id), Some(*id));
    }
}

/// A note that refers to another note, and a text box with a note in it, come along whole and
/// every reference follows its copy.
#[test]
fn notes_in_notes_and_in_text_boxes_follow_their_copies() {
    let mut d = doc(&["Alpha"]);
    let a = add_note(&mut d, 5, "outer");
    let b = make_note(&mut d, "inner");
    let end_of_a = d.end_of(StoryRef::Part(a));
    d.insert_object(&end_of_a, note_ref(b), &CharProps::default()).unwrap();
    let c = make_note(&mut d, "boxed");
    let mut in_box = Paragraph::with_text("In a box", CharProps::default());
    in_box.insert_object(0, note_ref(c), &CharProps::default()).unwrap();
    let story = d.add_part(PartKind::TextBox, vec![para_block(in_box)]);
    d.insert_object(&Pos::body(0, 0), box_shape(story), &CharProps::default()).unwrap();
    let len = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();

    let frag = d.copy_range(&Pos::body(0, 0), &Pos::body(0, len));
    for id in [a, b, c, story] {
        assert!(frag.parts.contains_key(&id), "part {id} was copied");
    }
    let old_parts: Vec<u32> = d.parts.keys().copied().collect();
    let at = d.end_of(StoryRef::Body);
    d.insert_fragment(&at, &frag).unwrap();

    let new_box = refs_in(&d, StoryRef::Body);
    let new_a = new_box.last().copied().unwrap();
    assert!(!old_parts.contains(&new_a));
    let a_refs = refs_in(&d, StoryRef::Part(new_a));
    assert_eq!(a_refs.len(), 2, "the mark and the inner note");
    assert_eq!(a_refs[0], new_a);
    let new_b = a_refs[1];
    assert!(!old_parts.contains(&new_b));
    assert!(d.plain_text(StoryRef::Part(new_b)).contains("inner"));
    // The pasted box's story names a copy of its note, not the source's.
    let box_story = d
        .para_paths(StoryRef::Body)
        .iter()
        .filter_map(|p| d.para(StoryRef::Body, p))
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let InlineObject::Shape { story, .. } = o { *story } else { None })
        .find(|s| !old_parts.contains(s))
        .unwrap();
    let box_refs = refs_in(&d, StoryRef::Part(box_story));
    assert_eq!(box_refs.len(), 1);
    assert!(!old_parts.contains(&box_refs[0]));
    assert!(d.plain_text(StoryRef::Part(box_refs[0])).contains("boxed"));
    // The originals are as they were.
    assert_eq!(refs_in(&d, StoryRef::Part(story)), vec![c]);
    assert_eq!(refs_in(&d, StoryRef::Part(a)), vec![a, b]);
}

/// A note whose story can't be made (the paste's budget is spent) leaves its references out
/// rather than naming nothing, and charges the budget.
#[test]
fn notes_past_the_paste_budget_leave_no_dangling_reference() {
    let mut d = doc(&["Alpha beta"]);
    let note = add_note(&mut d, 5, "note");
    let frag = d.copy_range(&Pos::body(0, 0), &Pos::body(0, 5 + OBJ_LEN));
    let mut blocks = frag.blocks.clone();
    let mut parts = frag.parts.clone();
    let before = d.parts.len();
    let mut budget = BoxBudget::new();
    budget.bytes = 0;
    assert_eq!(d.adopt_notes(&mut blocks, &mut parts, &mut budget), 1);
    assert_eq!(d.parts.len(), before, "no note was made");
    assert_eq!(budget.skipped, 0);
    assert!(!parts.contains_key(&note), "a rejected note has no blocks to clone during rewriting");
    let Some(Block::Para(p)) = blocks.first() else { panic!() };
    assert!(p.objects.is_empty(), "the reference is gone");
    assert_eq!(p.text, "Alpha");
    // Exhausted text box slots still leave room for the note.
    let (mut blocks, mut parts) = (frag.blocks.clone(), frag.parts.clone());
    let mut budget = BoxBudget::new();
    budget.stories = 0;
    let bytes = budget.bytes;
    assert_eq!(d.adopt_notes(&mut blocks, &mut parts, &mut budget), 0);
    assert_eq!(d.parts.len(), before + 1);
    assert_eq!(budget.stories, 0);
    assert!(budget.bytes < bytes);
}

/// A note cut from the document and pasted back keeps its story (no unreferenced second copy is
/// left); one copied while still referenced gets a story of its own.
#[test]
fn cut_and_paste_back_reuses_the_notes_story() {
    let mut d = doc(&["Alpha beta"]);
    let a = add_note(&mut d, 5, "moves");
    let len = d.para(StoryRef::Body, &Path::top(0)).unwrap().len();
    let frag = d.copy_range(&Pos::body(0, 0), &Pos::body(0, len));
    d.delete_range(&Pos::body(0, 0), &Pos::body(0, len)).unwrap();
    assert!(body_note_ids(&d).is_empty());
    let parts_before = d.parts.len();
    d.insert_fragment(&Pos::body(0, 0), &frag).unwrap();
    assert_eq!(d.parts.len(), parts_before, "no second story");
    assert_eq!(body_note_ids(&d), vec![a]);
    assert_eq!(mark_id(&d, a), Some(a));
    // Pasting it again while it is referenced makes a note of its own.
    let at = d.end_of(StoryRef::Body);
    d.insert_fragment(&at, &frag).unwrap();
    let ids = body_note_ids(&d);
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    // A story that changed since the copy isn't taken for it.
    let end = d.end_of(StoryRef::Body);
    d.delete_range(&Pos::body(0, 0), &end).unwrap();
    assert!(body_note_ids(&d).is_empty());
    let mut dst = d.clone();
    let story_end = dst.end_of(StoryRef::Part(a));
    dst.para_mut(StoryRef::Part(a), &story_end.path).unwrap().insert_text(story_end.off, " edited", &CharProps::default()).unwrap();
    dst.insert_fragment(&Pos::body(0, 0), &frag).unwrap();
    let pasted = body_note_ids(&dst);
    assert_eq!(pasted.len(), 1);
    assert_ne!(pasted[0], a, "the edited story stays as it was");
    assert!(!dst.plain_text(StoryRef::Part(pasted[0])).contains("edited"));
}

/// Reusing the old note would also reuse the story shown by a changed nested text box.
#[test]
fn cut_note_with_an_edited_nested_box_restores_the_copied_box_text() {
    let mut d = doc(&["Alpha beta"]);
    let note = add_note(&mut d, 5, "note");
    let inner = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::with_text("copied text", CharProps::default()))]);
    let mut outside = Paragraph::new();
    outside.insert_object(0, box_shape(inner), &CharProps::default()).unwrap();
    let outer = d.add_part(PartKind::TextBox, vec![para_block(outside)]);
    let in_note = d.end_of(StoryRef::Part(note));
    d.insert_object(&in_note, box_shape(outer), &CharProps::default()).unwrap();

    let start = Pos::body(0, 0);
    let end = d.end_of(StoryRef::Body);
    let fragment = d.copy_range(&start, &end);
    d.delete_range(&start, &end).unwrap();
    let mut unchanged = d.clone();
    unchanged.insert_fragment(&start, &fragment).unwrap();
    assert_eq!(body_note_ids(&unchanged), [note], "an unchanged cut can still reuse the original note");

    d.set_story(StoryRef::Part(inner), vec![para_block(Paragraph::with_text("changed box", CharProps::default()))]).unwrap();
    d.insert_fragment(&start, &fragment).unwrap();
    let [pasted] = body_note_ids(&d).try_into().unwrap();
    assert_ne!(pasted, note);
    let box_in = |id| d.para(StoryRef::Part(id), &Path::top(0)).unwrap().objects.iter().flat_map(InlineObject::text_boxes).next().unwrap();
    assert_eq!(d.plain_text(StoryRef::Part(box_in(box_in(pasted)))), "copied text");
    assert_eq!(d.plain_text(StoryRef::Part(inner)), "changed box");
}

#[test]
fn cut_note_follows_the_copied_or_removed_nested_note() {
    for in_box in [false, true] {
        let mut d = doc(&["Alpha"]);
        let outer = add_note(&mut d, 5, "outer");
        let inner = make_note(&mut d, "copied text");
        let target = if in_box {
            let story = d.add_part(PartKind::TextBox, vec![para_block(Paragraph::new())]);
            d.insert_object(&d.end_of(StoryRef::Part(outer)), box_shape(story), &CharProps::default()).unwrap();
            story
        } else {
            outer
        };
        d.insert_object(&d.end_of(StoryRef::Part(target)), note_ref(inner), &CharProps::default()).unwrap();
        let start = Pos::body(0, 0);
        let end = d.end_of(StoryRef::Body);
        let frag = d.copy_range(&start, &end);
        d.delete_range(&start, &end).unwrap();
        let end_of_inner = d.end_of(StoryRef::Part(inner));
        d.para_mut(StoryRef::Part(inner), &end_of_inner.path).unwrap().insert_text(end_of_inner.off, " edited", &CharProps::default()).unwrap();

        let mut limited = d.clone();
        d.insert_fragment(&start, &frag).unwrap();
        let [pasted] = body_note_ids(&d).try_into().unwrap();
        assert_ne!(pasted, outer, "the outer note needs a new target");
        let pasted_source = if in_box {
            let p = d.para(StoryRef::Part(pasted), &Path::top(0)).unwrap();
            p.objects.iter().flat_map(InlineObject::text_boxes).next().unwrap()
        } else {
            pasted
        };
        let pasted_inner = *refs_in(&d, StoryRef::Part(pasted_source)).last().unwrap();
        assert_ne!(pasted_inner, inner);
        assert!(d.plain_text(StoryRef::Part(pasted_inner)).ends_with("copied text"));
        assert!(d.plain_text(StoryRef::Part(inner)).ends_with(" edited"));
        assert_eq!(refs_in(&d, StoryRef::Part(target)).last(), Some(&inner));

        let mut blocks = frag.blocks.clone();
        let mut parts = frag.parts.clone();
        let mut budget = BoxBudget::new();
        let outer_blocks = &parts[&outer].blocks;
        budget.bytes = outer_blocks.iter().map(|b| block_cost(b).0 + std::mem::size_of::<Arc<Block>>()).sum();
        assert_eq!(limited.adopt_notes(&mut blocks, &mut parts, &mut budget), 1);
        assert!(!parts.contains_key(&inner), "the nested note exceeds the remaining byte budget");
        let Some(Block::Para(p)) = blocks.first() else { panic!() };
        let new_outer = p
            .objects
            .iter()
            .find_map(|o| match o {
                InlineObject::NoteRef { id, .. } => Some(*id),
                _ => None,
            })
            .unwrap();
        assert_ne!(new_outer, outer);
        assert_eq!(refs_in(&limited, StoryRef::Part(new_outer)), [new_outer]);
    }
}

/// An image object naming `key`.
fn image_ref(key: &str) -> InlineObject {
    InlineObject::Image {
        media: key.into(),
        w: 10.0,
        h: 10.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    }
}

/// A document whose first paragraph ends in a footnote holding an image `image1.png` of `bytes`.
fn doc_with_note_image(bytes: u8) -> Document {
    let mut d = doc(&["Alpha"]);
    let note = add_note(&mut d, 5, "pictured");
    let at = d.end_of(StoryRef::Part(note));
    d.insert_object(&at, image_ref("image1.png"), &CharProps::default()).unwrap();
    d.media.insert("image1.png".into(), Arc::new(vec![bytes; 3]));
    d
}

/// The bytes of the image in the note a body reference of `d` names.
fn note_image_bytes(d: &Document, note: u32) -> Option<Vec<u8>> {
    let p = d.para(StoryRef::Part(note), &Path::top(0))?;
    p.objects.iter().find_map(|o| match o {
        InlineObject::Image { media, .. } => d.media.get(media).map(|b| b.to_vec()),
        _ => None,
    })
}

/// The Spike keeps the pictures of the notes it collects, also when two pieces come from
/// documents that both call their picture `image1.png` (review of #6740).
#[test]
fn appended_pieces_keep_the_pictures_of_their_notes() {
    let one = doc_with_note_image(1);
    let two = doc_with_note_image(2);
    let mut spike = one.copy_range(&Pos::body(0, 0), &Pos::body(0, 5 + OBJ_LEN));
    spike.append_piece(two.copy_range(&Pos::body(0, 0), &Pos::body(0, 5 + OBJ_LEN)));

    let mut d = doc(&["Target"]);
    let at = d.end_of(StoryRef::Body);
    d.insert_fragment(&at, &spike).unwrap();
    let refs = body_note_ids(&d);
    assert_eq!(refs.len(), 2);
    assert_eq!(note_image_bytes(&d, refs[0]), Some(vec![1; 3]));
    assert_eq!(note_image_bytes(&d, refs[1]), Some(vec![2; 3]));
}
