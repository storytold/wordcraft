//! Synthetic regressions for the Mac paragraph-formatting matrix in #5964.

use super::*;

/// Copy Formatting and the brush distinguish partial and whole paragraph sources and targets.
#[test]
fn format_painter_paragraph_selection_matrix() {
    use wordcraft_doc::props::Align;
    for brush in [false, true] {
        for (source, mark) in [("Source", false), ("Source alpha beta.", false), ("Source alpha beta.", true)] {
            for target in ["Target", "middle", "Target first middle last."] {
                let mut s = s();
                run(&mut s, "document.setText", json!({"text": "Source alpha beta.\nTarget first middle last."}));
                run(&mut s, "select.text", json!({"text": "Source alpha beta."}));
                run(&mut s, "para.alignCenter", json!({}));
                run(&mut s, "select.text", json!({"text": source}));
                if mark {
                    s.sel.focus = Pos { story: StoryRef::Body, path: s.doc.para_paths(StoryRef::Body)[1].clone(), off: 0 };
                }
                run(&mut s, if brush { "edit.formatPainter" } else { "edit.copyFormat" }, json!({}));
                run(&mut s, "select.text", json!({"text": target}));
                run(&mut s, "edit.pasteFormat", json!({}));
                let expected = source == "Source alpha beta." && (brush || target == "Target first middle last.");
                assert_eq!(
                    s.doc.para_at(&s.sel.focus).unwrap().props.align == Some(Align::Center),
                    expected,
                    "brush={brush}, source={source:?}, mark={mark}, target={target:?}"
                );
            }
        }
    }
}

/// Paste Formatting applies captured paragraph properties at carets and across touched paragraphs.
#[test]
fn paste_format_paragraph_carets_and_cross_paragraph_targets() {
    use wordcraft_doc::props::Align;
    for source_kind in ["partial", "whole", "caret"] {
        for cross in [false, true] {
            let mut s = s();
            run(&mut s, "document.setText", json!({"text": "Source alpha beta.\nTarget first middle last.\nSecond target paragraph."}));
            run(&mut s, "select.text", json!({"text": "Source alpha beta."}));
            run(&mut s, "para.alignCenter", json!({}));
            if source_kind == "partial" {
                run(&mut s, "select.text", json!({"text": "Source"}));
            } else if source_kind == "caret" {
                s.sel = crate::Selection::caret(Pos { off: 3, ..s.sel.anchor.clone() });
            }
            run(&mut s, "edit.copyFormat", json!({}));
            if cross {
                // select.text searches inside a paragraph; construct the cross-paragraph
                // range explicitly, matching the native first-word to second-word selection.
                let paths = s.doc.para_paths(StoryRef::Body);
                s.sel = crate::Selection {
                    anchor: Pos { story: StoryRef::Body, path: paths[1].clone(), off: 7 },
                    focus: Pos { story: StoryRef::Body, path: paths[2].clone(), off: 13 },
                };
            } else {
                run(&mut s, "select.text", json!({"text": "middle"}));
                s.sel = crate::Selection::caret(s.sel.anchor.clone());
            }
            let (a, b) = s.sel.ordered();
            run(&mut s, "edit.pasteFormat", json!({}));
            for p in [&a, &b] {
                assert_eq!(
                    s.doc.para_at(p).unwrap().props.align == Some(Align::Center),
                    source_kind != "partial",
                    "source_kind={source_kind}, cross={cross}"
                );
            }
            if source_kind != "partial" {
                run(&mut s, "edit.undo", json!({}));
                assert_ne!(s.doc.para_at(&a).unwrap().props.align, Some(Align::Center), "paragraph changes are undoable");
            }
        }
    }
}
