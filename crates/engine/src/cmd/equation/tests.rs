use super::*;
use crate::Session;
use wordcraft_doc::Document;

fn session() -> Session {
    let mut s = Session::new(Document::new());
    s.run("document.setText", &json!({"text": ""})).unwrap();
    s
}

fn linear(s: &Session) -> String {
    s.math.as_ref().and_then(|m| s.doc.para_at(&m.at).and_then(|p| p.object_at(m.at.off))).map(|o| o.plain_text().to_string()).unwrap_or_default()
}

#[test]
fn type_an_equation_like_word() {
    let mut s = session();
    s.run("insert.equation", &json!({})).unwrap();
    assert!(s.math.is_some(), "editing the new equation");
    // An empty line gives a display equation.
    assert!(matches!(s.doc.para_at(&s.math.as_ref().unwrap().at).unwrap().object_at(0), Some(InlineObject::Equation { display: true, .. })));
    for t in ["x", "=", "(-b\\pm", " ", "\\sqrt", " ", "(b^2-4ac))/2a", " "] {
        s.run("equation.type", &json!({"text": t})).unwrap();
    }
    assert_eq!(linear(&s), "x=(-b±√(b^2-4ac))/2a");
    let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
    assert!(matches!(nodes.get(1), Some(MNode::Frac { .. })), "{nodes:#?}");
    // One undo step for the typing.
    s.run("equation.exit", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert!(linear(&s).is_empty());
}

#[test]
fn structures_symbols_and_navigation() {
    let mut s = session();
    s.run("equation.insertStructure", &json!({"id": "frac.stacked"})).unwrap();
    s.run("equation.type", &json!({"text": "1"})).unwrap();
    s.run("equation.move", &json!({"dir": "down"})).unwrap();
    s.run("equation.insertSymbol", &json!({"char": "π"})).unwrap();
    assert_eq!(linear(&s), "1/π");
    s.run("equation.move", &json!({"dir": "right"})).unwrap();
    s.run("equation.insertStructure", &json!({"id": "nary.sumLimits"})).unwrap();
    s.run("equation.type", &json!({"text": "k=1"})).unwrap();
    s.run("equation.move", &json!({"dir": "next"})).unwrap();
    s.run("equation.type", &json!({"text": "n"})).unwrap();
    s.run("equation.move", &json!({"dir": "next"})).unwrap();
    s.run("equation.type", &json!({"text": "k"})).unwrap();
    assert_eq!(linear(&s), "1/π∑_(k=1)^n▒k");
    let r = s.run("equation.get", &json!({})).unwrap();
    assert!(r["latex"].as_str().unwrap().contains("\\sum"));
    // Moving right past the end leaves the equation.
    for _ in 0..20 {
        if s.math.is_none() {
            break;
        }
        let _ = s.run("equation.move", &json!({"dir": "right"}));
    }
    assert!(s.math.is_none());
}

#[test]
fn numbering_conversion_and_display() {
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "E=mc^2"})).unwrap();
    s.run("equation.type", &json!({"text": "#(1)"})).unwrap();
    s.run("equation.enter", &json!({})).unwrap();
    assert_eq!(linear(&s), "E=mc^2#(1)");
    s.run("equation.number", &json!({"value": false})).unwrap();
    assert_eq!(linear(&s), "E=mc^2");
    s.run("equation.number", &json!({})).unwrap();
    assert_eq!(linear(&s), "E=mc^2#");
    s.run("equation.convert", &json!({"to": "linear"})).unwrap();
    s.run("equation.convert", &json!({"to": "professional"})).unwrap();
    assert_eq!(linear(&s), "E=mc^2#");
    s.run("equation.display", &json!({"value": false})).unwrap();
    let r = s.run("equation.get", &json!({})).unwrap();
    assert_eq!(r["display"], false);
    // LaTeX input.
    s.run("equation.set", &json!({"latex": "\\frac{a}{b}"})).unwrap();
    assert_eq!(linear(&s), "a/b");
    // Backspacing an equation empty, then once more, removes it.
    let mut n = 0;
    while s.math.is_some() && n < 50 {
        s.run("equation.backspace", &json!({})).unwrap();
        n += 1;
    }
    assert!(s.math.is_none());
    assert_eq!(s.doc.para_at(&s.sel.focus).unwrap().objects.len(), 0);
}

#[test]
fn gallery_lists_everything() {
    let mut s = session();
    let g = s.run("equation.gallery", &json!({})).unwrap();
    assert_eq!(g["structures"].as_array().unwrap().len(), 11);
    assert!(g["symbols"].as_array().unwrap().len() >= 8);
    for b in crate::math_gallery::BUILT_INS {
        let mut s = session();
        s.run("insert.equation", &json!({"builtin": b.0, "edit": false})).unwrap();
    }
}

/// A session on a DOCX whose only paragraph is `<m:oMath>{math}</m:oMath>`.
fn imported(math: &str) -> Session {
    use std::io::Write;
    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    let m = "http://schemas.openxmlformats.org/officeDocument/2006/math";
    let parts = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string(),
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
        ),
        ("word/document.xml", format!(r#"<w:document xmlns:w="{w}" xmlns:m="{m}"><w:body><w:p><m:oMath>{math}</m:oMath></w:p><w:sectPr/></w:body></w:document>"#)),
    ];
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in parts {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
    }
    let bytes = zw.finish().unwrap().into_inner();
    let mut s = Session::new(wordcraft_docx::read(&bytes).unwrap());
    s.run("equation.edit", &json!({})).unwrap();
    s
}

fn document_xml(s: &Session) -> String {
    use std::io::Read;
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml
}

#[test]
fn removing_numbers_keeps_every_row() {
    // #444: a numbered two-row array lost its second row when its numbers were removed.
    for numbered in ["█(a#(1)@b#(2))", "█(a@b#(2))"] {
        let mut s = session();
        s.run("insert.equation", &json!({"linear": numbered})).unwrap();
        s.run("equation.number", &json!({"value": false})).unwrap();
        assert_eq!(linear(&s), "█(a@b)", "{numbered}");
        let mut back = Session::new(wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap());
        assert_eq!(back.run("equation.get", &json!({})).unwrap()["linear"], "█(a@b)", "{numbered} after save and reopen");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(linear(&s), numbered, "Undo restores the numbers");
    }
    // A single numbered row stands alone again.
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "a#(1)"})).unwrap();
    s.run("equation.number", &json!({"value": false})).unwrap();
    assert_eq!(linear(&s), "a");
}

#[test]
fn colour_survives_save_and_reopen() {
    // #1467: a coloured equation came back black after saving and reopening.
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "(a+b)/c"})).unwrap();
    s.run("equation.exit", &json!({})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    s.run("format.color", &json!({"color": "C00000"})).unwrap();
    assert!(document_xml(&s).contains(r#"<w:color w:val="C00000"/></w:rPr><m:t"#), "the math runs carry the colour");
    let mut back = Session::new(wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap());
    back.run("equation.edit", &json!({})).unwrap();
    let at = back.math.as_ref().unwrap().at.clone();
    let p = back.doc.para_at(&at).unwrap();
    let (_, props) = p.run_ranges().find(|(r, _)| r.contains(&at.off)).unwrap();
    assert_eq!(props.color, Some(wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::props::Rgb(0xC0, 0, 0))), "the equation's colour is back");
    let Some(InlineObject::Equation { math, .. }) = p.object_at(at.off) else { panic!() };
    let mut nodes = math.nodes.clone();
    let mut runs = 0;
    wordcraft_doc::math_edit::each_run_mut(&mut nodes, &mut |r| {
        runs += 1;
        assert_eq!(r.color, None, "the colour is the equation's, not stuck on each run");
    });
    assert!(runs > 0);
    // Automatic again: no colour written.
    back.run("select.all", &json!({})).unwrap();
    back.run("format.color", &json!({"color": "auto"})).unwrap();
    assert!(!document_xml(&back).contains("C00000"));
}

#[test]
fn imported_colour_keeps_source_omml_until_recoloured() {
    // #1467: lifting a Word equation's colour must not drop the OMML the model can't hold.
    let r = r#"<w:rPr><w:color w:val="FF0000"/></w:rPr>"#;
    let ctrl = r#"<m:ctrlPr><w:rPr><w:color w:val="00FF00"/></w:rPr></m:ctrlPr>"#;
    let mut s =
        imported(&format!("<m:f><m:fPr>{ctrl}</m:fPr><m:num><m:r>{r}<m:t>a</m:t></m:r></m:num><m:den><m:r>{r}<m:t>b</m:t></m:r></m:den></m:f>"));
    assert!(document_xml(&s).contains("00FF00"), "untouched: source OMML written back");
    s.run("select.all", &json!({})).unwrap();
    s.run("format.color", &json!({"color": "0000FF"})).unwrap();
    let xml = document_xml(&s);
    assert!(xml.contains("0000FF") && !xml.contains("FF0000"), "recoloured: runs rewritten");
}

#[test]
fn navigation_keeps_imported_omml() {
    // #446: a Left move without any change dropped the fraction's imported control colour.
    let red = r#"<m:f><m:fPr><m:ctrlPr><w:rPr><w:color w:val="FF0000"/></w:rPr></m:ctrlPr></m:fPr><m:num><m:r><m:t>a</m:t></m:r></m:num><m:den><m:r><m:t>b</m:t></m:r></m:den></m:f>"#;
    let mut s = imported(red);
    s.run("equation.move", &json!({"dir": "left"})).unwrap();
    assert!(document_xml(&s).contains(r#"w:val="FF0000""#), "the control colour survives navigation");
    // A genuine edit still writes the new content.
    s.run("equation.type", &json!({"text": "x"})).unwrap();
    assert_eq!(linear(&s), "a/bx");
    assert!(document_xml(&s).contains(">bx<"));
}

/// A Left move that builds up `a/b` makes the document dirty, advances layout and IME revisions,
/// reports a classified transformed change, and Undo restores the literal run; plain navigation
/// leaves no undo step.
#[test]
fn navigation_build_up_is_an_undoable_edit() {
    // #447: building `a/b` up on a Left move left the document clean and Undo did nothing.
    let mut s = imported("<m:r><m:t>a/b</m:t></m:r>");
    assert!(!s.dirty);
    let layout_rev = s.rev();
    let ime_rev = revision(&mut s);
    s.run("equation.move", &json!({"dir": "left"})).unwrap();
    // Every command replaces the last outcome; sample the edit before the read-only query.
    let outcome = s.ime_edit_outcome();
    assert_eq!(s.run("equation.get", &json!({})).unwrap()["latex"], "\\frac{a}{b}");
    assert!(s.dirty, "build-up marks the document dirty");
    assert!(s.rev() > layout_rev, "build-up refreshes the layout revision");
    assert!(outcome.classified && outcome.content_changed && outcome.state_changed && outcome.transformed, "{outcome:?}");
    assert!(revision(&mut s) > ime_rev, "build-up advances the unavailable IME revision");
    s.run("edit.undo", &json!({})).unwrap();
    s.run("equation.edit", &json!({})).unwrap();
    assert_eq!(s.run("equation.get", &json!({})).unwrap()["latex"], "a/b", "Undo restores the plain run");
    // Plain navigation is no edit.
    let mut s = imported("<m:r><m:t>abc</m:t></m:r>");
    s.run("equation.move", &json!({"dir": "left"})).unwrap();
    assert!(!s.dirty);
    assert_eq!(s.undo_depth(), 0);
}

#[test]
fn latex_text_keeps_its_spaces() {
    // #448: `\text{for all x}` came out as `forallx`.
    for (tex, want) in
        [("\\text{for all x}", "\\text{for all x}"), ("\\text{a b}", "\\text{a b}"), ("\\text{ab}", "\\text{ab}"), ("x + y", "x+y"), ("x+y", "x+y")]
    {
        let mut s = session();
        s.run("insert.equation", &json!({"latex": tex})).unwrap();
        assert_eq!(s.run("equation.get", &json!({})).unwrap()["latex"], want, "{tex}");
    }
}

#[test]
fn typed_multi_letter_scripts_stay_whole() {
    // #1609, Word for Mac 16.113.4: `V_dd` + Space leaves "dd" entirely in the subscript.
    for typed in ["V_dd", "V_(dd)"] {
        let mut s = session();
        s.run("insert.equation", &json!({})).unwrap();
        s.run("equation.type", &json!({"text": typed})).unwrap();
        s.run("equation.type", &json!({"text": " "})).unwrap();
        let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
        assert_eq!(nodes.len(), 1, "{typed}: {nodes:#?}");
        assert!(matches!(&nodes[0], MNode::Script { sub, .. } if matches!(sub.as_slice(), [MNode::Run(r)] if r.text == "dd")), "{typed}: {nodes:#?}");
    }
    let mut s = session();
    s.run("insert.equation", &json!({})).unwrap();
    s.run("equation.type", &json!({"text": "x^ab "})).unwrap();
    let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
    assert!(matches!(&nodes[0], MNode::Script { sup, .. } if matches!(sup.as_slice(), [MNode::Run(r)] if r.text == "ab")), "{nodes:#?}");
}

#[test]
fn a_built_script_keeps_the_letter_typed_after_it() {
    // #1609 review: `x_i y ` built x_{iy}: the letter after a script is its own item.
    let mut s = session();
    s.run("insert.equation", &json!({})).unwrap();
    s.run("equation.type", &json!({"text": "x_i y "})).unwrap();
    let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
    assert_eq!(nodes.len(), 2, "{nodes:#?}");
    assert!(matches!(&nodes[0], MNode::Script { sub, .. } if matches!(sub.as_slice(), [MNode::Run(r)] if r.text == "i")), "{nodes:#?}");
    assert!(matches!(&nodes[1], MNode::Run(r) if r.text.trim() == "y"), "{nodes:#?}");
    let mut s = session();
    s.run("insert.equation", &json!({})).unwrap();
    s.run("equation.type", &json!({"text": "a_i b_j "})).unwrap();
    let nodes = nodes_of(&s, &s.math.as_ref().unwrap().at).unwrap();
    let subs: Vec<&str> = nodes
        .iter()
        .filter_map(|n| match n {
            MNode::Script { sub, .. } => match sub.as_slice() {
                [MNode::Run(r)] => Some(r.text.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(subs, ["i", "j"], "{nodes:#?}");
    assert!(nodes.iter().all(|n| matches!(n, MNode::Script { .. }) || matches!(n, MNode::Run(r) if r.text.trim().is_empty())), "{nodes:#?}");
}

/// Returns the session's current IME state revision, allocating a new one when the caret state changed.
fn revision(s: &mut Session) -> u64 {
    s.ime_state_revision().expect("an IME state revision")
}

/// Clears the stored node tree of the equation currently being edited, leaving its linear text
/// and lazy state in place.
fn clear_nodes(s: &mut Session) {
    let at = s.math.as_ref().expect("editing").at.clone();
    if let Some(InlineObject::Equation { math, .. }) = s.doc.para_mut(at.story, &at.path).expect("paragraph").object_at_mut(at.off) {
        math.nodes.clear();
    }
}

/// Backspace at the start of an equation is classified with no content or state change,
/// allocates no revision and leaves the linear text intact.
#[test]
fn equation_backspace_at_start_is_a_classified_noop() {
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "x+y"})).unwrap();
    s.run("equation.move", &json!({"dir": "home"})).unwrap();
    let before = revision(&mut s);
    s.run("equation.backspace", &json!({})).unwrap();
    let outcome = s.ime_edit_outcome();
    assert!(outcome.classified);
    assert!(!outcome.content_changed && !outcome.state_changed, "{outcome:?}");
    assert_eq!(revision(&mut s), before, "a no-op allocates no revision");
    assert_eq!(linear(&s), "x+y");
}

/// Materializing an imported equation's lazy nodes is a state change only: the outcome is
/// classified, content is unchanged, and the revision advances.
#[test]
fn lazy_normalization_is_state_not_content() {
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "x+1"})).unwrap();
    clear_nodes(&mut s);
    // Editing parses the local tree but leaves the stored equation lazy.
    s.run("equation.edit", &json!({})).unwrap();
    s.run("equation.move", &json!({"dir": "home"})).unwrap();
    let before = revision(&mut s);
    s.run("equation.backspace", &json!({})).unwrap();
    let outcome = s.ime_edit_outcome();
    assert!(outcome.classified);
    assert!(!outcome.content_changed, "the normalized tree was already logical content");
    assert!(outcome.state_changed, "materializing the cached nodes is a state change");
    assert!(revision(&mut s) > before, "state changes advance the revision");
}

/// Empty typing is a classified no-op in built-up and linear form, without a new revision.
#[test]
fn empty_equation_type_is_a_classified_noop_in_both_modes() {
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "x"})).unwrap();
    let before = revision(&mut s);
    s.run("equation.type", &json!({"text": ""})).unwrap();
    assert!(s.ime_edit_outcome().classified);
    assert!(!s.ime_edit_outcome().content_changed && !s.ime_edit_outcome().state_changed);
    assert_eq!(revision(&mut s), before, "built-up empty type is a no-op");
    s.run("equation.convert", &json!({"to": "linear"})).unwrap();
    let before = revision(&mut s);
    s.run("equation.type", &json!({"text": ""})).unwrap();
    assert!(!s.ime_edit_outcome().content_changed && !s.ime_edit_outcome().state_changed);
    assert_eq!(revision(&mut s), before, "linear empty type is a no-op");
}

/// Typing `a/b ` or `\alpha ` reports both a content change and a transformed automatic rewrite.
#[test]
fn automatic_equation_rewrites_are_transformed() {
    for text in ["a/b ", "\\alpha "] {
        let mut s = session();
        s.run("insert.equation", &json!({})).unwrap();
        s.run("equation.type", &json!({"text": text})).unwrap();
        let outcome = s.ime_edit_outcome();
        assert!(outcome.content_changed, "{text}: {outcome:?}");
        assert!(outcome.transformed, "{text}: {outcome:?}");
    }
}

/// Justification and display changes advance the revision and report a state, not content,
/// change.
#[test]
fn equation_style_changes_are_state_changes_without_content() {
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "x"})).unwrap();
    let before = revision(&mut s);
    s.run("equation.justify", &json!({"jc": "left"})).unwrap();
    let outcome = s.ime_edit_outcome();
    assert!(outcome.classified && outcome.state_changed && !outcome.content_changed, "{outcome:?}");
    assert!(revision(&mut s) > before);
    let before = revision(&mut s);
    s.run("equation.display", &json!({"value": false})).unwrap();
    assert!(s.ime_edit_outcome().state_changed && !s.ime_edit_outcome().content_changed);
    assert!(revision(&mut s) > before);
}

/// Converting every equation reports one classified effect that merges their content and state
/// changes.
#[test]
fn convert_all_merges_every_equations_effect() {
    let mut s = session();
    s.run("document.setText", &json!({"text": "\r"})).unwrap();
    s.run("insert.equation", &json!({"linear": "a/b"})).unwrap();
    s.run("equation.convert", &json!({"to": "linear"})).unwrap();
    s.run("equation.exit", &json!({})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.equation", &json!({"linear": "c/d"})).unwrap();
    s.run("equation.exit", &json!({})).unwrap();
    s.run("equation.convert", &json!({"to": "professional", "all": true})).unwrap();
    let outcome = s.ime_edit_outcome();
    assert!(outcome.classified && outcome.content_changed && outcome.state_changed, "{outcome:?}");
}

/// A bounded deletion around the caret removes the requested units in one undo step and reports
/// a content change.
#[test]
fn equation_delete_surrounding_is_one_transaction_and_one_undo_step() {
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "abcd"})).unwrap();
    let depth = s.undo_depth();
    s.run("equation.deleteSurrounding", &json!({"before": 2, "after": 0})).unwrap();
    assert_eq!(linear(&s), "ab");
    assert_eq!(s.undo_depth(), depth + 1, "one undo step, not one per unit");
    assert!(s.ime_edit_outcome().classified && s.ime_edit_outcome().content_changed);
}

/// An impossible bounded deletion applies no prefix, adds no undo step, keeps the session and
/// caret, and leaves no classified outcome.
#[test]
fn equation_delete_surrounding_refuses_partial_progress() {
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "ab"})).unwrap();
    let session_token = s.math.as_ref().unwrap().session;
    let at = s.math.as_ref().unwrap().at.clone();
    let depth = s.undo_depth();
    let err = s.run("equation.deleteSurrounding", &json!({"before": 3, "after": 0})).unwrap_err();
    assert!(err.to_string().contains("cannot delete"), "{err}");
    assert_eq!(linear(&s), "ab", "the whole request is refused, no prefix applied");
    assert_eq!(s.undo_depth(), depth, "no undo step for a refused transaction");
    let m = s.math.as_ref().expect("still editing");
    assert_eq!(m.session, session_token);
    assert_eq!(m.at, at);
    assert!(!s.ime_edit_outcome().classified, "a refusal has no outcome");
    assert_eq!(s.last_command.as_ref().map(|(id, _)| id.as_str()), Some("equation.deleteSurrounding"), "only the pre-dispatch Repeat record remains");
}

/// Zero or oversized counts and deletions from an empty equation are refused without mutation;
/// exactly deleting all text leaves the empty equation object.
#[test]
fn equation_delete_surrounding_bounds_and_empty_equations() {
    let mut s = session();
    s.run("insert.equation", &json!({})).unwrap();
    assert!(s.run("equation.deleteSurrounding", &json!({"before": 0, "after": 0})).is_err(), "zero counts are refused");
    assert!(s.run("equation.deleteSurrounding", &json!({"before": 1, "after": 0})).is_err(), "an empty equation refuses");
    assert!(s.math.is_some(), "the empty equation object stays");
    s.run("equation.type", &json!({"text": "a"})).unwrap();
    let depth = s.undo_depth();
    assert!(s.run("equation.deleteSurrounding", &json!({"before": 65, "after": 0})).is_err());
    assert!(s.run("equation.deleteSurrounding", &json!({"before": 0, "after": 65})).is_err());
    assert_eq!(linear(&s), "a");
    assert_eq!(s.undo_depth(), depth);
    s.run("equation.deleteSurrounding", &json!({"before": 1, "after": 0})).unwrap();
    assert_eq!(linear(&s), "", "exactly deleting the last unit leaves the equation object");
    assert!(s.math.is_some());
}

/// A forward deletion that would only step into another argument is refused whole, leaving the
/// text and undo depth unchanged (the caret is not asserted here).
#[test]
fn equation_delete_surrounding_refuses_a_caret_only_step() {
    let mut s = session();
    s.run("insert.equation", &json!({"latex": "\\frac{a}{b}"})).unwrap();
    // At the end of the numerator a forward delete only steps into the denominator: the
    // tree does not change, so a request that would report progress is refused whole.
    s.math.as_mut().expect("editing").pos = MathPos { path: vec![(0, 0)], off: 1 };
    let before = linear(&s);
    let depth = s.undo_depth();
    let err = s.run("equation.deleteSurrounding", &json!({"before": 0, "after": 1})).unwrap_err();
    assert!(err.to_string().contains("no progress"), "{err}");
    assert_eq!(linear(&s), before);
    assert_eq!(s.undo_depth(), depth);
}

/// A deletion that really erases on one side cannot buy a caret-only step on the other:
/// the mixed request is refused whole, so a following IME commit cannot land in the
/// argument the platform never asked about.
#[test]
fn equation_delete_surrounding_refuses_a_mixed_caret_only_step() {
    let mut s = session();
    s.run("insert.equation", &json!({"latex": "\\frac{a}{b}"})).unwrap();
    // At the end of the numerator. `before: 1` erases `a`, but `after: 1` would only step
    // into the denominator (the report's mixed-direction reproducer).
    s.math.as_mut().expect("editing").pos = MathPos { path: vec![(0, 0)], off: 1 };
    let before = linear(&s);
    let pos = s.math.as_ref().expect("editing").pos.clone();
    let depth = s.undo_depth();
    let err = s.run("equation.deleteSurrounding", &json!({"before": 1, "after": 1})).unwrap_err();
    assert!(err.to_string().contains("no progress"), "{err}");
    assert_eq!(linear(&s), before, "the successful-looking prefix is not applied");
    assert_eq!(s.undo_depth(), depth, "no undo step for a refused mixed request");
    assert_eq!(s.math.as_ref().expect("editing").pos, pos, "the caret stays where the request found it");
}

/// Typing more than the cap inserts exactly `MAX_EQUATION_TYPE_CHARS` characters in both linear
/// and built-up form.
#[test]
fn equation_type_caps_linear_and_built_up_input() {
    for linear_mode in [false, true] {
        let mut s = session();
        s.run("insert.equation", &json!({})).unwrap();
        if linear_mode {
            s.run("equation.convert", &json!({"to": "linear"})).unwrap();
        }
        let text = "a".repeat(crate::MAX_EQUATION_TYPE_CHARS + 5);
        s.run("equation.type", &json!({"text": text})).unwrap();
        assert_eq!(linear(&s).chars().count(), crate::MAX_EQUATION_TYPE_CHARS, "linear={linear_mode}");
    }
}

/// Deleting one character forward removes it and moves the math caret to the start of the
/// remaining text.
#[test]
fn equation_delete_surrounding_moves_the_caret_for_a_one_step_delete() {
    let mut s = session();
    s.run("insert.equation", &json!({"linear": "abc"})).unwrap();
    s.run("equation.move", &json!({"dir": "home"})).unwrap();
    s.run("equation.deleteSurrounding", &json!({"after": 1, "before": 0})).unwrap();
    assert_eq!(linear(&s), "bc");
    assert_eq!(s.math.as_ref().unwrap().pos, MathPos::new(Vec::new(), 0));
}
