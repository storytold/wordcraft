use super::{run, s};
use serde_json::json;
use wordcraft_doc::Pos;
use wordcraft_doc::props::Ligatures;

#[test]
fn format_kerning_and_ligatures_commands() {
    let mut s = s();
    run(&mut s, "text.insert", json!({"text": "office affinity"}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.kerning", json!({"value": false}));
    assert_eq!(run(&mut s, "format.state", json!({}))["kerning"], false);
    run(&mut s, "format.kerning", json!({"minSize": 14}));
    let st = run(&mut s, "format.state", json!({}));
    assert_eq!(st["kerning"], true, "enabled even below the threshold");
    assert_eq!(st["kernMinSize"], 14.0);
    let depth = s.undo_depth();
    run(&mut s, "format.kerning", json!({"value": true}));
    assert_eq!(s.undo_depth(), depth, "same threshold is not another edit");
    run(&mut s, "format.kerning", json!({}));
    assert_eq!(run(&mut s, "format.state", json!({}))["kernMinSize"], 0.0);
    run(&mut s, "format.kerning", json!({}));
    assert_eq!(run(&mut s, "format.state", json!({}))["kernMinSize"], 1.0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "format.state", json!({}))["kerning"], false);
    run(&mut s, "format.kerning", json!({"minSize": 0.5}));
    assert_eq!(run(&mut s, "format.state", json!({}))["kernMinSize"], 0.5);
    for mode in Ligatures::ALL {
        run(&mut s, "format.ligatures", json!({"mode": mode.as_ooxml()}));
        assert_eq!(run(&mut s, "format.state", json!({}))["ligatures"], mode.as_ooxml());
        let depth = s.undo_depth();
        run(&mut s, "format.ligatures", json!({"mode": mode.as_ooxml()}));
        assert_eq!(s.undo_depth(), depth);
    }
    run(&mut s, "format.set", json!({"props": {"ligatures": "none"}}));
    assert_eq!(run(&mut s, "format.state", json!({}))["ligatures"], "none");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "format.state", json!({}))["ligatures"], "all");
    s.doc
        .format_range(&Pos::body(0, 0), &Pos::body(0, 6), &|c| {
            c.kern = Some(0.0);
            c.ligatures = Some(Ligatures::None);
        })
        .unwrap();
    let st = run(&mut s, "format.state", json!({}));
    assert!(st["kerning"].is_null());
    assert!(st["kernMinSize"].is_null());
    assert!(st["ligatures"].is_null());
    let snapshot = serde_json::to_value(&s.doc).unwrap();
    let depth = s.undo_depth();
    for (command, params) in [
        ("format.ligatures", json!({"mode": "bogus"})),
        ("format.ligatures", json!({})),
        ("format.ligatures", json!({"mode": 9})),
        ("format.kerning", json!({"minSize": "bad"})),
        ("format.kerning", json!({"value": "bad"})),
        ("format.set", json!({"props": {"ligatures": "bogus"}})),
        ("format.set", json!({"props": {"ligatures": {"standard": true}}})),
        ("format.set", json!({"props": {"fmtChange": {"rev": 0, "old": {"ligatures": {"standard": true}}}}})),
        ("styles.modify", json!({"style": "Normal", "chr": {"ligatures": {"standard": true}}})),
        ("styles.modify", json!({"style": "Normal", "chr": {"fmtChange": {"rev": 0, "old": {"ligatures": {}}}}})),
    ] {
        assert!(s.run(command, &params).is_err(), "{command}: {params}");
        assert_eq!(serde_json::to_value(&s.doc).unwrap(), snapshot);
        assert_eq!(s.undo_depth(), depth);
    }
    run(&mut s, "format.clear", json!({}));
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert!(p.runs.iter().all(|r| r.props.ligatures.is_none() && r.props.kern.is_none()));
}

#[test]
fn legacy_json_ligatures_open_and_save_as_names() {
    let legacy = json!({
        "body": [{"kind": "para", "text": "office", "runs": [{"len": 6, "props": {"ligatures": {"standard": true}}}]}],
        "styles": {"defaultChr": {"ligatures": {"contextual": true}}}
    });
    let doc = crate::io::open_bytes("legacy.json", &serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(doc.plain_text(wordcraft_doc::StoryRef::Body), "office");
    assert_eq!(doc.para_at(&Pos::body(0, 0)).unwrap().runs[0].props.ligatures, Some(Ligatures::Standard));
    assert_eq!(doc.styles.default_chr.ligatures, Some(Ligatures::Contextual));
    let saved = crate::io::save_bytes("updated.json", &doc).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(json["body"][0]["runs"][0]["props"]["ligatures"], "standard");
    assert_eq!(json["styles"]["defaultChr"]["ligatures"], "contextual");
    let reopened = crate::io::open_bytes("updated.json", &saved).unwrap();
    assert_eq!(reopened, doc);
}
