use serde_json::json;
use wordcraft_doc::{Document, StoryRef, section::NumFormat};
use wordcraft_engine::Session;
use wordcraft_formats::{
    model::{self, FBlock, Flow, Fmt, FmtChange, ListInfo, Para, RevMark},
    rtf,
};

fn changed_list(old: Option<bool>, new: Option<bool>, level: u8) -> Document {
    let list = |ordered| ListInfo { ordered, level, start: 1 };
    let mut item = Para {
        list: new.map(list),
        fmt_change: Some(Box::new(FmtChange {
            mark: RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None },
            old: Para { list: old.map(list), ..Default::default() },
        })),
        ..Default::default()
    };
    item.push_text("Item", &Fmt::default());
    let mut next = Para { list: new.map(list), ..Default::default() };
    next.push_text("Next", &Fmt::default());
    model::to_doc(&Flow { blocks: vec![FBlock::Para(item), FBlock::Para(next)], ..Default::default() })
}

fn lists(doc: &Document) -> Vec<Option<(bool, u8)>> {
    doc.para_paths(StoryRef::Body)
        .iter()
        .map(|path| {
            doc.para(StoryRef::Body, path).unwrap().props.numbering.map(|n| {
                let def = doc.numbering.level(n.num, n.level).expect("list definition exists");
                (def.format != NumFormat::Bullet, n.level)
            })
        })
        .collect()
}

// Prior properties must not consume one reader list-table slot per changed paragraph.
fn many_changed_lists(old_ordered: bool) {
    let mut item = Para {
        list: Some(ListInfo { ordered: !old_ordered, level: 2, start: 1 }),
        fmt_change: Some(Box::new(FmtChange {
            mark: RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None },
            old: Para { list: Some(ListInfo { ordered: old_ordered, level: 2, start: 1 }), ..Default::default() },
        })),
        ..Default::default()
    };
    item.push_text("Changed", &Fmt::default());
    let mut blocks = vec![FBlock::Para(item); 4096];
    let mut separator = Para::default();
    separator.push_text("Separator", &Fmt::default());
    blocks.push(FBlock::Para(separator));
    let mut later = Para { list: Some(ListInfo { ordered: true, level: 0, start: 1 }), ..Default::default() };
    later.push_text("Independent numbered item", &Fmt::default());
    blocks.push(FBlock::Para(later));
    let mut doc = model::to_doc(&Flow { blocks, ..Default::default() });
    for saves in 1..=2 {
        doc = rtf::import(rtf::export(&doc).as_bytes()).unwrap();
        let current = lists(&doc);
        assert_eq!(current.len(), 4098);
        assert!(current[..4096].iter().all(|li| *li == Some((!old_ordered, 2))));
        assert_eq!(current[4096], None);
        if !old_ordered {
            assert_eq!(current[4097], Some((true, 0)), "independent list, saves={saves}");
        }
        for (command, ordered) in [("review.rejectAll", old_ordered), ("review.acceptAll", !old_ordered)] {
            let mut s = Session::new(doc.clone());
            s.run(command, &json!({})).unwrap();
            let restored = lists(&s.doc);
            assert!(restored[..4096].iter().all(|li| *li == Some((ordered, 2))), "{command}, old_ordered={old_ordered}, saves={saves}");
            assert_eq!(&restored[4096..], &[None, Some((true, 0))]);
            assert_eq!(s.doc.plain_text(StoryRef::Body), doc.plain_text(StoryRef::Body));
            assert!(s.run("review.changes", &json!({})).unwrap().as_array().unwrap().is_empty());
        }
    }
}

#[test]
fn rtf_many_prior_numbered_lists_keep_their_kind() {
    many_changed_lists(true);
}

#[test]
fn rtf_many_prior_bulleted_lists_preserve_later_numbering() {
    many_changed_lists(false);
}

#[test]
fn rtf_prior_list_accept_and_reject_after_save_reopen() {
    for (old, new) in [
        (Some(true), Some(false)),
        (Some(false), Some(true)),
        (Some(true), None),
        (Some(false), None),
        (None, Some(true)),
        (None, Some(false)),
        (Some(true), Some(true)),
        (Some(false), Some(false)),
    ] {
        for level in [0, 2, 8] {
            let mut doc = changed_list(old, new, level);
            // The unsaved document is the control; repeat the save/reopen to catch drift.
            for saves in 0..=2 {
                for (command, expected) in [("review.rejectAll", old), ("review.acceptAll", new)] {
                    let mut s = Session::new(doc.clone());
                    s.run(command, &json!({})).unwrap();
                    assert_eq!(
                        lists(&s.doc),
                        vec![expected.map(|ordered| (ordered, level)), new.map(|ordered| (ordered, level))],
                        "{command}, old={old:?}, new={new:?}, level={level}, saves={saves}"
                    );
                    assert_eq!(s.doc.plain_text(StoryRef::Body).trim_end(), "Item\nNext");
                    assert!(s.run("review.changes", &json!({})).unwrap().as_array().unwrap().is_empty());
                }
                doc = rtf::import(rtf::export(&doc).as_bytes()).unwrap();
            }
        }
    }
}

#[test]
fn rtf_mixed_nested_list_kinds_survive_accept_and_reject() {
    for first_ordered in [false, true] {
        for level in [1, 2, 8] {
            let list = |ordered| Some(ListInfo { ordered, level, start: 1 });
            let mut first = Para {
                list: list(first_ordered),
                fmt_change: Some(Box::new(FmtChange {
                    mark: RevMark { author: "Alice".into(), date: "2024-03-15T10:30:00Z".into(), id: None },
                    old: Para { list: list(!first_ordered), ..Default::default() },
                })),
                ..Default::default()
            };
            first.push_text("Changed", &Fmt::default());
            let mut next = Para { list: list(!first_ordered), ..Default::default() };
            next.push_text("Unchanged", &Fmt::default());
            let mut doc = model::to_doc(&Flow { blocks: vec![FBlock::Para(first), FBlock::Para(next)], ..Default::default() });
            assert_eq!(lists(&doc), vec![Some((first_ordered, level)), Some((!first_ordered, level))], "flow conversion");
            // Give the second paragraph its own real definition: constructing a mixed flow
            // must not mask an export bug by coalescing its kinds before the first save.
            let next_kind = if first_ordered { wordcraft_doc::numbering::ListKind::Bullet } else { wordcraft_doc::numbering::ListKind::Numbered };
            let next_num = doc.numbering.add_list(next_kind);
            let next_path = doc.para_paths(StoryRef::Body)[1].clone();
            doc.para_mut(StoryRef::Body, &next_path).unwrap().props.numbering = Some(wordcraft_doc::props::NumRef { num: next_num, level });
            for saves in 0..=2 {
                assert_eq!(lists(&doc), vec![Some((first_ordered, level)), Some((!first_ordered, level))], "current, saves={saves}");
                for (command, expected_first) in [("review.acceptAll", first_ordered), ("review.rejectAll", !first_ordered)] {
                    let mut session = Session::new(doc.clone());
                    session.run(command, &json!({})).unwrap();
                    assert_eq!(
                        lists(&session.doc),
                        vec![Some((expected_first, level)), Some((!first_ordered, level))],
                        "{command}, first_ordered={first_ordered}, level={level}, saves={saves}"
                    );
                    assert_eq!(session.doc.plain_text(StoryRef::Body).trim_end(), "Changed\nUnchanged");
                    assert!(session.run("review.changes", &json!({})).unwrap().as_array().unwrap().is_empty());
                }
                doc = rtf::import(rtf::export(&doc).as_bytes()).unwrap();
            }
        }
    }
}

#[test]
fn rtf_nested_kind_switch_keeps_parent_numbering() {
    let blocks = [(true, 0), (false, 1), (true, 1), (false, 1), (true, 1), (true, 0)]
        .into_iter()
        .map(|(ordered, level)| {
            let mut p = Para { list: Some(ListInfo { ordered, level, start: 1 }), ..Default::default() };
            p.push_text("Item", &Fmt::default());
            FBlock::Para(p)
        })
        .collect();
    let mut doc = model::to_doc(&Flow { blocks, ..Default::default() });
    for saves in 0..=2 {
        let mut counters = wordcraft_doc::numbering::Counters::default();
        let labels: Vec<_> = doc
            .para_paths(StoryRef::Body)
            .iter()
            .map(|path| {
                let n = doc.para(StoryRef::Body, path).unwrap().props.numbering.unwrap();
                counters.next_label(&doc.numbering, n.num, n.level).unwrap().0
            })
            .collect();
        assert_eq!(labels[0], "1.", "saves={saves}");
        assert_eq!(labels[2], "a.", "saves={saves}");
        assert_eq!(labels[4], "a.", "saves={saves}");
        assert_eq!(labels[5], "2.", "saves={saves}");
        doc = rtf::import(rtf::export(&doc).as_bytes()).unwrap();
    }
}
