//! Calculations through `.docx`: Word sees ordinary equations in tagged content controls and the
//! quoted values as locked fields; WordCraft reads its hidden store back and the calculations are
//! live again. An equation edited in Word is flagged, never overwritten.

use std::collections::BTreeMap;

use wordcraft_calc::docx::{export, import};
use wordcraft_doc::calc::{CalcRegion, CalcSettings, CalcState, CalcTable, ColumnError, QuoteSpec, UnitSystem, calc_field_instr};
use wordcraft_doc::math::Math;
use wordcraft_doc::{Block, CharProps, Document, InlineObject, Paragraph, para_block};

/// The sources of the test document, region ids 1, 2, 3… in order. Region 3 is a display
/// equation. Variety on purpose: units, a fraction, scientific notation, a nested subscript, an
/// absolute temperature, a root, a user function.
const SOURCES: [&str; 10] = [
    "P := 1500 lbf",
    "A_b := 2.5 in^2",
    "σ := P/A_b = ? ksi",
    "d := 1.5e-7 m = ?",
    "k_t_max := 3.2",
    "T := 20 °C = ? °F",
    "x := √(4 m^2) = ? m",
    "f(a, b) := a^2 + b",
    "z := f(2, 3) = ?",
    "w := 3 ft + 2 in = ? in",
];

fn calc_eq(id: u64, source: &str, display: bool) -> InlineObject {
    InlineObject::Equation {
        linear: String::new(),
        display,
        math: Math::default(),
        calc: Some(Box::new(CalcRegion {
            result_format: None,
            worksheet_order: None,
            id,
            source: source.into(),
            unit: None,
            precision: None,
            numbered: false,
            state: CalcState::Fresh,
        })),
    }
}

/// One region per paragraph, then a paragraph quoting σ, recalculated.
fn report() -> Document {
    let mut d = Document::new();
    d.body = SOURCES
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut p = Paragraph::with_text("Step ", CharProps::default());
            let off = p.text.len();
            p.insert_object(off, calc_eq(i as u64 + 1, s, i == 2), &CharProps::default()).unwrap();
            para_block(p)
        })
        .collect();
    let mut p = Paragraph::with_text("The stress is ", CharProps::default());
    let off = p.text.len();
    let field = InlineObject::Field {
        instr: calc_field_instr(&QuoteSpec { name: "σ".into(), unit: Some("ksi".into()), precision: Some(2), index: None }),
        result: String::new(),
        locked: true,
        dirty: false,
        code: false,
        form: String::new(),
    };
    p.insert_object(off, field, &CharProps::default()).unwrap();
    d.body.push(para_block(p));
    d.calc_settings = Some(CalcSettings {
        result_format: None,
        system: UnitSystem::UsCustomary,
        precision: 2,
        manual: false,
        g: wordcraft_doc::calc::GMeans::Gram,
        origin: 0,
    });
    wordcraft_calc::recalc(&mut d, &|_| None);
    d
}

fn save(doc: &Document) -> Vec<u8> {
    let (mut copy, store) = export(doc);
    copy.calc_store = Some(store);
    wordcraft_docx::write(&copy).unwrap()
}

fn open(bytes: &[u8]) -> Document {
    let mut doc = wordcraft_docx::read(bytes).unwrap();
    let store = doc.calc_store.take();
    import(&mut doc, store);
    doc
}

/// Every object in the body, table cells included, in document order.
fn objects(doc: &Document) -> Vec<InlineObject> {
    fn walk(blocks: &[std::sync::Arc<Block>], out: &mut Vec<InlineObject>) {
        for b in blocks {
            match &**b {
                Block::Para(p) => out.extend(p.objects.iter().cloned()),
                Block::Table(t) => t.rows.iter().flat_map(|r| r.cells.iter()).for_each(|c| walk(&c.blocks, out)),
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.body, &mut out);
    out
}

/// (region, linear) of every calculation region in the body.
fn regions(doc: &Document) -> Vec<(CalcRegion, String)> {
    objects(doc)
        .into_iter()
        .filter_map(|o| match o {
            InlineObject::Equation { linear, calc: Some(c), .. } => Some((*c, linear)),
            _ => None,
        })
        .collect()
}

fn region(doc: &Document, id: u64) -> (CalcRegion, String) {
    regions(doc).into_iter().find(|(r, _)| r.id == id).unwrap_or_else(|| panic!("no region {id}"))
}

fn field_result(doc: &Document) -> String {
    objects(doc)
        .into_iter()
        .find_map(|o| match o {
            InlineObject::Field { result, .. } => Some(result),
            _ => None,
        })
        .unwrap()
}

fn has_markers(doc: &Document) -> bool {
    objects(doc).iter().any(|o| matches!(o, InlineObject::ControlStart { .. } | InlineObject::ControlEnd))
}

fn parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 10_000, max_total_bytes: 64 << 20 }).unwrap();
    let names: Vec<_> = archive.names().map(|name| name.into_owned()).collect();
    names
        .into_iter()
        .map(|name| {
            let bytes = archive.read(&name, 64 << 20).unwrap().unwrap();
            (name, bytes)
        })
        .collect()
}

fn zip(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut archive = craft_zip::Writer::new();
    for (name, bytes) in parts {
        archive.add(name, bytes, craft_zip::Compression::Deflated).unwrap();
    }
    archive.finish().unwrap()
}

fn document_xml(bytes: &[u8]) -> String {
    String::from_utf8(parts(bytes).remove("word/document.xml").unwrap()).unwrap()
}

/// `bytes` with `word/document.xml` changed by `f`.
fn edit_document(bytes: &[u8], f: impl Fn(String) -> String) -> Vec<u8> {
    let mut p = parts(bytes);
    let xml = String::from_utf8(p.remove("word/document.xml").unwrap()).unwrap();
    p.insert("word/document.xml".into(), f(xml).into_bytes());
    zip(&p)
}

/// The name the store part was saved under.
fn store_part(p: &BTreeMap<String, Vec<u8>>) -> String {
    p.keys().find(|k| k.eq_ignore_ascii_case("customXml/wordcraftcalc.xml")).cloned().expect("store part")
}

/// `bytes` with the store part's JSON replaced by `json`.
fn with_store_json(bytes: &[u8], json: &str) -> Vec<u8> {
    let mut p = parts(bytes);
    let name = store_part(&p);
    let esc = json.replace('&', "&amp;").replace('<', "&lt;");
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><wordcraftCalc xmlns="urn:wordcraft:calc" kind="wordcraft-calc-v1"><calcStore>{esc}</calcStore></wordcraftCalc>"#
    );
    p.insert(name, xml.into_bytes());
    zip(&p)
}

/// The `w:sdt` … `</w:sdt>` holding the region with `id`.
fn sdt_of(xml: &str, id: u64) -> String {
    let tag = format!(r#"<w:tag w:val="wordcraft-calc:{id}"/>"#);
    let at = xml.find(&tag).unwrap();
    let start = xml[..at].rfind("<w:sdt>").unwrap();
    let end = at + xml[at..].find("</w:sdt>").unwrap() + "</w:sdt>".len();
    xml[start..end].to_string()
}

#[test]
fn calc_round_trips_through_docx() {
    let doc = report();
    let before = doc.clone();
    for (r, linear) in regions(&doc) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
    let bytes = save(&doc);
    assert_eq!(doc, before, "saving changes nothing in the live document");

    let back = open(&bytes);
    assert!(!has_markers(&back), "the tagged controls are WordCraft's bookkeeping: gone once read");
    let got = regions(&back);
    assert_eq!(got.len(), SOURCES.len());
    for (i, (r, linear)) in got.iter().enumerate() {
        let (orig, orig_linear) = region(&doc, i as u64 + 1);
        assert_eq!(r.id, orig.id);
        assert_eq!(r.source, SOURCES[i]);
        assert_eq!(r.state, CalcState::Fresh, "{} came back as {:?} ({linear})", r.source, r.state);
        assert_eq!(linear, &orig_linear);
    }
    assert_eq!(field_result(&back), field_result(&doc));
    assert!(field_result(&back).contains("ksi"), "{}", field_result(&back));
    assert_eq!(back.calc_settings, doc.calc_settings);
    assert!(back.calc_store.is_none());
    // A second round trip is just as clean.
    let again = open(&save(&back));
    assert_eq!(regions(&again), regions(&back));
}

#[test]
fn word_sees_plain_equations() {
    let bytes = save(&report());
    let xml = document_xml(&bytes);
    let mut maths = 0;
    for (at, _) in xml.match_indices("<m:oMath>") {
        maths += 1;
        let before = &xml[..at];
        let open = before.rfind("<w:sdtContent>").expect("inside a content control");
        assert!(!before[open..].contains("</w:sdtContent>"), "equation outside a control at {at}");
        let tag = &before[before.rfind("<w:tag w:val=\"").unwrap()..];
        assert!(tag.starts_with("<w:tag w:val=\"wordcraft-calc:"), "{}", &tag[..40.min(tag.len())]);
    }
    assert_eq!(maths, SOURCES.len());
    assert!(xml.contains("<m:oMathPara>"), "the display equation stays a display equation");
    for id in 1..=SOURCES.len() as u64 {
        let sdt = sdt_of(&xml, id);
        assert!(sdt.contains(r#"<w:alias w:val="Calculation"/>"#), "{sdt}");
        assert!(!sdt.contains("<w:lock"), "reviewers may edit: {sdt}");
        assert!(sdt.contains("<m:oMath>"));
    }
    assert!(xml.contains("WCCALC"));
    assert!(xml.contains(r#"w:fldLock="1""#), "quoted values are locked fields");
    // Word shows the result, not a source: no `?` or `:=` anywhere in what it sees.
    assert!(!xml.contains(":="));
    let p = parts(&bytes);
    let store = String::from_utf8(p[&store_part(&p)].clone()).unwrap();
    assert!(store.contains("wordcraft-calc-v1") && store.contains("σ := P/A_b = ? ksi"), "{store}");
}

#[test]
fn plain_document_has_no_store() {
    let doc = Document::from_text("Just text");
    let (copy, store) = export(&doc);
    assert!(store.is_empty());
    assert_eq!(copy, doc);
    let p = parts(&save(&doc));
    assert!(!p.keys().any(|k| k.to_ascii_lowercase().contains("wordcraftcalc")));
}

#[test]
fn edited_in_word_is_flagged() {
    let doc = report();
    let bytes = edit_document(&save(&doc), |xml| {
        assert!(xml.contains(">P≔1500<"));
        xml.replace(">P≔1500<", ">P≔1600<")
    });
    let back = open(&bytes);
    let (p, linear) = region(&back, 1);
    assert_eq!(p.state, CalcState::EditedOutside);
    assert_eq!(p.source, "P := 1500 lbf", "the source is kept for the author to resolve");
    assert!(linear.contains("1600"), "Word's version is shown: {linear}");
    assert!(linear.ends_with(" ⚠"), "the edited region is marked: {linear}");
    // σ uses P: neither Word's 1600 nor the stored 1500 may give it a number.
    let (sigma, linear) = region(&back, 3);
    assert_eq!(sigma.state, CalcState::Error("depends on a calculation edited in Word — resolve it".into()), "{linear}");
    assert!(!linear.contains("ksi") && linear.contains('⚠'), "{linear}");
    assert_eq!(field_result(&back), "?", "a quoted value of a dependent shows no number");
    // Everything that doesn't use P is current.
    for (r, linear) in regions(&back).into_iter().filter(|(r, _)| ![1, 3].contains(&r.id)) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
}

#[test]
fn the_edited_mark_never_reaches_word() {
    let bytes = edit_document(&save(&report()), |xml| xml.replace(">P≔1500<", ">P≔1600<"));
    let once = open(&bytes);
    assert!(region(&once, 1).1.ends_with(" ⚠"));
    let xml = document_xml(&save(&once));
    let sdt = sdt_of(&xml, 1);
    assert!(sdt.contains("1600") && !sdt.contains('⚠'), "Word gets its own equation back, unmarked: {sdt}");
    // Reopened, it is marked once, not twice.
    let twice = open(&save(&once));
    let (_, linear) = region(&twice, 1);
    assert_eq!(linear.matches('⚠').count(), 1, "{linear}");
}

#[test]
fn edited_outside_stays_flagged_after_resave() {
    let bytes = edit_document(&save(&report()), |xml| xml.replace(">P≔1500<", ">P≔1600<"));
    let once = open(&bytes);
    assert_eq!(region(&once, 1).0.state, CalcState::EditedOutside);
    // Saved again untouched: Word's text now matches what is written, but it is still Word's.
    let twice = open(&save(&once));
    let (p, linear) = region(&twice, 1);
    assert_eq!(p.state, CalcState::EditedOutside);
    assert!(linear.contains("1600"), "{linear}");
}

#[test]
fn deleted_in_word_makes_dependents_error() {
    let bytes = edit_document(&save(&report()), |xml| {
        let sdt = sdt_of(&xml, 2);
        xml.replace(&sdt, "")
    });
    let back = open(&bytes);
    assert!(regions(&back).iter().all(|(r, _)| r.id != 2), "the deleted region stays deleted");
    let (sigma, linear) = region(&back, 3);
    assert_eq!(sigma.state, CalcState::Error("A_b isn't defined".into()), "{linear}");
    assert!(linear.contains('⚠'), "{linear}");
}

#[test]
fn no_store_means_plain() {
    let (copy, _) = export(&report());
    // The controls are written, the store isn't.
    let bytes = wordcraft_docx::write(&copy).unwrap();
    assert!(document_xml(&bytes).contains("wordcraft-calc:1"));
    let back = open(&bytes);
    assert!(regions(&back).is_empty());
    assert!(!has_markers(&back));
    let equations = objects(&back).iter().filter(|o| matches!(o, InlineObject::Equation { .. })).count();
    assert_eq!(equations, SOURCES.len(), "the equations stay, as plain math");
}

#[test]
fn hostile_store_ignored() {
    let bytes = save(&report());
    let huge = {
        let mut s = String::from(r#"{"settings":{},"regions":["#);
        for i in 0..1_000_000u64 {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(r#"{{"id":{i},"source":"","fingerprint":0}}"#));
        }
        s + "]}"
    };
    let many = {
        let entries: Vec<String> = (0..100_001u64).map(|i| format!(r#"{{"id":{i},"source":"a","fingerprint":0}}"#)).collect();
        format!(r#"{{"settings":{{}},"regions":[{}]}}"#, entries.join(","))
    };
    for json in ["{not json", "[]", r#"{"regions": 7}"#, huge.as_str(), many.as_str()] {
        let back = open(&with_store_json(&bytes, json));
        assert!(regions(&back).is_empty(), "store {:.40} should be ignored", json);
        assert!(!has_markers(&back));
    }
}

#[test]
fn duplicate_or_missing_store_entries_leave_those_regions_plain() {
    let doc = report();
    let (_, store) = export(&doc);
    let mut regions_json: Vec<serde_json::Value> = store.regions.iter().map(|r| serde_json::to_value(r).unwrap()).collect();
    // Region 1 twice (with different sources: which is right can't be known), region 2 missing.
    let mut dup = regions_json[0].clone();
    dup["source"] = "P := 1 lbf".into();
    regions_json.push(dup);
    regions_json.remove(1);
    let json = serde_json::json!({ "settings": store.settings, "regions": regions_json }).to_string();
    let back = open(&with_store_json(&save(&doc), &json));
    let ids: Vec<u64> = regions(&back).iter().map(|(r, _)| r.id).collect();
    assert!(!ids.contains(&1) && !ids.contains(&2), "{ids:?}");
    assert!(ids.contains(&3));
    assert!(!has_markers(&back));
}

#[test]
fn a_copy_made_in_word_comes_back_plain() {
    // A reviewer copied P's equation (control and all) into another paragraph: the first keeps
    // the calculation, the copy is plain math.
    let bytes = edit_document(&save(&report()), |xml| {
        let sdt = sdt_of(&xml, 1);
        let end = xml.find("</w:body>").unwrap();
        let sect = xml[..end].rfind("<w:sectPr").unwrap();
        format!("{}<w:p>{sdt}</w:p>{}", &xml[..sect], &xml[sect..])
    });
    let back = open(&bytes);
    assert_eq!(regions(&back).iter().filter(|(r, _)| r.id == 1).count(), 1);
    assert_eq!(region(&back, 1).0.state, CalcState::Fresh);
    let last = back.body.iter().rev().find_map(|b| b.as_para().filter(|p| !p.objects.is_empty())).unwrap();
    assert!(matches!(last.objects.as_slice(), [InlineObject::Equation { calc: None, .. }]), "{:?}", last.objects);
}

#[test]
fn tables_and_stale_results_are_saved_as_shown() {
    let mut doc = report();
    // A region in a table cell.
    let mut cell_para = Paragraph::with_text("", CharProps::default());
    cell_para.insert_object(0, calc_eq(50, "y := 2 × P = ? lbf", false), &CharProps::default()).unwrap();
    let cell = wordcraft_doc::Cell { blocks: vec![para_block(cell_para)], ..Default::default() };
    let table = wordcraft_doc::Table { rows: vec![wordcraft_doc::Row { cells: vec![cell], ..Default::default() }], ..Default::default() };
    doc.body.insert(0, std::sync::Arc::new(Block::Table(table)));
    doc.body.insert(1, para_block(Paragraph::new()));
    // A region in error.
    let mut bad = Paragraph::with_text("", CharProps::default());
    bad.insert_object(0, calc_eq(60, "e := q + 1 = ?", false), &CharProps::default()).unwrap();
    doc.body.insert(1, para_block(bad));
    wordcraft_calc::recalc(&mut doc, &|_| None);
    assert!(matches!(region(&doc, 60).0.state, CalcState::Error(_)));
    wordcraft_calc::mark_stale(&mut doc);
    let xml = document_xml(&save(&doc));
    assert!(xml.contains("wordcraft-calc:50"));
    assert!(xml.contains('≈'), "a stale result is written stale");
    let errored = sdt_of(&xml, 60);
    assert!(errored.contains('⚠') && errored.contains(r#"w:val="C00000""#), "an error is written as its red mark: {errored}");
    assert!(errored.contains(">e≔q+1<") && !errored.contains("≔q+1="), "and no value: {errored}");
    let back = open(&save(&doc));
    let (y, _) = region(&back, 50);
    assert_eq!(y.source, "y := 2 × P = ? lbf");
}

#[test]
fn a_store_renamed_by_word_is_found() {
    // Word may save the custom XML parts it keeps under its own names (`customXml/item1.xml`).
    let doc = report();
    let mut p = parts(&save(&doc));
    let old = store_part(&p);
    let bytes = p.remove(&old).unwrap();
    p.insert("customXml/item1.xml".into(), bytes);
    for (name, data) in p.iter_mut() {
        if name.ends_with(".rels") || name == "[Content_Types].xml" {
            *data = String::from_utf8(data.clone()).unwrap().replace("wordcraftcalc.xml", "item1.xml").into_bytes();
        }
    }
    let renamed = zip(&p);
    let back = open(&renamed);
    assert_eq!(regions(&back).len(), SOURCES.len());
    assert!(regions(&back).iter().all(|(r, _)| r.state == CalcState::Fresh));
    // Saved again, there is one store, not the old one beside the new.
    let again = parts(&save(&back));
    let stores = again.values().filter(|d| d.windows(17).any(|w| w == b"wordcraft-calc-v1")).count();
    assert_eq!(stores, 1, "{:?}", again.keys().collect::<Vec<_>>());
}

#[test]
fn word_reserializing_an_untouched_equation_is_no_edit() {
    // Word splits and rewrites runs as it saves; the equation's text and structure are the same.
    let bytes = edit_document(&save(&report()), |xml| {
        let run = r#"<m:t xml:space="preserve">P≔1500</m:t></m:r>"#;
        assert!(xml.contains(run));
        let split = r#"<m:t>P≔</m:t></m:r><m:r><w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/></w:rPr><m:t>1500</m:t></m:r>"#;
        xml.replace(run, split)
    });
    let back = open(&bytes);
    assert_eq!(region(&back, 1).0.state, CalcState::Fresh);
}

#[test]
fn stores_are_proper_custom_xml_data_stores() {
    // Word keeps a custom XML part as a data store when it has its properties part (itemProps):
    // both WordCraft stores get one, related from the store, with its content type.
    let mut doc = report();
    doc.fit_state = Some(wordcraft_doc::FitState { cum_delta: 1.0, ..Default::default() });
    let bytes = save(&doc);
    let p = parts(&bytes);
    let types = String::from_utf8(p["[Content_Types].xml"].clone()).unwrap();
    for (store, ns) in [("wordcraftcalc.xml", "urn:wordcraft:calc"), ("wordcraftfit.xml", "urn:wordcraft:fit")] {
        let item = p.keys().find(|k| k.to_ascii_lowercase().ends_with(&format!("customxml/{store}"))).unwrap_or_else(|| panic!("{store}"));
        let (dir, file) = item.rsplit_once('/').unwrap();
        let rels = String::from_utf8(p[&format!("{dir}/_rels/{file}.rels")].clone()).unwrap();
        assert!(rels.contains("/relationships/customXmlProps"), "{rels}");
        let target = rels.split("Target=\"").nth(1).unwrap().split('"').next().unwrap();
        let props_name = format!("{dir}/{target}");
        let props = String::from_utf8(p[&props_name].clone()).unwrap();
        assert!(props.contains("datastoreItem") && props.contains("itemID=\"{") && props.contains(&format!("schemaRef ds:uri=\"{ns}\"")), "{props}");
        assert!(
            types.contains(&format!(
                "PartName=\"/{props_name}\" ContentType=\"application/vnd.openxmlformats-officedocument.customXmlProperties+xml\""
            )),
            "{types}"
        );
    }
    let back = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(back.fit_state, doc.fit_state);
    assert_eq!(regions(&open(&bytes)).len(), SOURCES.len());
    // Saved again: still one of each, each with its properties.
    let again = parts(&save(&open(&bytes)));
    let props = again.values().filter(|d| d.windows(13).any(|w| w == b"datastoreItem")).count();
    assert_eq!(props, 2, "{:?}", again.keys().collect::<Vec<_>>());
}

/// Every internal relationship target in the package that names no part: `(rels part, target)`.
fn dangling(p: &BTreeMap<String, Vec<u8>>) -> Vec<(String, String)> {
    let has = |name: &str| p.keys().any(|k| k.eq_ignore_ascii_case(name));
    let mut out = Vec::new();
    for (name, data) in p.iter().filter(|(k, _)| k.ends_with(".rels")) {
        let base = name.split("_rels/").next().unwrap_or("");
        for rel in String::from_utf8_lossy(data).split("<Relationship ").skip(1) {
            if rel.contains("TargetMode=\"External\"") {
                continue;
            }
            let target = rel.split("Target=\"").nth(1).and_then(|t| t.split('"').next()).unwrap_or("");
            let mut path: Vec<&str> = if target.starts_with('/') { Vec::new() } else { base.split('/').filter(|s| !s.is_empty()).collect() };
            for seg in target.split('/').filter(|s| !s.is_empty()) {
                match seg {
                    ".." => {
                        path.pop();
                    }
                    "." => {}
                    s => path.push(s),
                }
            }
            if !has(&path.join("/")) {
                out.push((name.clone(), target.to_string()));
            }
        }
    }
    out
}

/// The `ds:itemID`s of every properties part in the package, one entry per part.
fn item_ids(p: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    p.values()
        .filter_map(|d| {
            let s = String::from_utf8_lossy(d);
            s.contains("datastoreItem").then(|| s.split("itemID=\"").nth(1).and_then(|t| t.split('"').next()).unwrap_or("").to_string())
        })
        .collect()
}

fn with_fit(mut doc: Document) -> Document {
    doc.fit_state = Some(wordcraft_doc::FitState { cum_delta: 1.0, ..Default::default() });
    doc
}

#[test]
fn dropping_the_stores_leaves_no_trace() {
    let bytes = save(&with_fit(report()));
    let mut back = open(&bytes);
    assert!(back.fit_state.is_some() && !regions(&back).is_empty());
    // Taken with everything that came with it: its properties part, manifest lines and relationship.
    for (k, v) in &back.passthrough {
        let k = k.to_ascii_lowercase();
        assert!(!k.contains("wordcraftcalc") && !k.contains("wordcraftfit"), "{k}");
        let text = String::from_utf8_lossy(v).to_ascii_lowercase();
        assert!(!k.starts_with("wordcraft:") || !(text.contains("wordcraftcalc") || text.contains("wordcraftfit")), "{k}: {text}");
    }
    // Every calculation made plain, Fit One Page undone, default settings.
    for b in back.body.iter_mut() {
        if let Block::Para(p) = std::sync::Arc::make_mut(b) {
            for o in &mut p.objects {
                if let InlineObject::Equation { calc, .. } = o {
                    *calc = None;
                }
            }
        }
    }
    back.fit_state = None;
    back.calc_settings = None;
    let saved = save(&back);
    let p = parts(&saved);
    assert!(item_ids(&p).is_empty(), "{:?}", p.keys().collect::<Vec<_>>());
    assert!(!p.keys().any(|k| k.to_ascii_lowercase().contains("wordcraft")), "{:?}", p.keys().collect::<Vec<_>>());
    assert!(dangling(&p).is_empty(), "{:?}", dangling(&p));
    let types = String::from_utf8(p["[Content_Types].xml"].clone()).unwrap();
    assert!(!types.contains("wordcraft"), "{types}");
    wordcraft_docx::read(&saved).unwrap();
}

#[test]
fn stores_renamed_by_word_are_written_once() {
    // Word saves the custom XML it keeps as `item1.xml` + `itemProps1.xml`, keeping their ids.
    let mut p = parts(&save(&with_fit(report())));
    let renames = [
        ("wordcraftcalcProps.xml", "itemProps1.xml"),
        ("wordcraftcalc.xml", "item1.xml"),
        ("wordcraftfitProps.xml", "itemProps2.xml"),
        ("wordcraftfit.xml", "item2.xml"),
    ];
    let rename = |s: &str| renames.iter().fold(s.to_string(), |s, (a, b)| s.replace(a, b));
    p = p
        .into_iter()
        .map(|(k, v)| {
            let v = if k.ends_with(".rels") || k == "[Content_Types].xml" { rename(&String::from_utf8(v).unwrap()).into_bytes() } else { v };
            (rename(&k), v)
        })
        .collect();
    assert!(
        p.contains_key("customxml/itemProps1.xml") || p.keys().any(|k| k.eq_ignore_ascii_case("customxml/itemProps1.xml")),
        "{:?}",
        p.keys().collect::<Vec<_>>()
    );
    let renamed = zip(&p);
    let back = open(&renamed);
    assert_eq!(regions(&back).len(), SOURCES.len());
    assert!(!back.passthrough.keys().any(|k| k.eq_ignore_ascii_case("customxml/itemProps1.xml")), "the renamed store's properties go with it");
    let saved = parts(&save(&back));
    let mut ids = item_ids(&saved);
    ids.sort();
    let mut unique = ids.clone();
    unique.dedup();
    assert_eq!(ids, unique, "no itemID twice: {:?}", saved.keys().collect::<Vec<_>>());
    assert_eq!(ids.len(), 2, "one properties part per store: {:?}", saved.keys().collect::<Vec<_>>());
    assert!(dangling(&saved).is_empty(), "{:?}", dangling(&saved));
    assert_eq!(regions(&open(&zip(&saved))).len(), SOURCES.len());
}

const FIT_ITEM_ID: &str = "{3B8E2C14-7A5D-4F69-9E01-2D6C8B4A7F53}";

/// The package `save` writes for `doc`, with the fit store moved to Word's names
/// (`item2.xml` + `itemProps2.xml`), its JSON payload changed by `payload`.
fn fit_renamed_by_word(doc: &Document, payload: impl Fn(String) -> String) -> Vec<u8> {
    let renames = [("wordcraftfitProps.xml", "itemProps2.xml"), ("wordcraftfit.xml", "item2.xml")];
    let rename = |s: &str| renames.iter().fold(s.to_string(), |s, (a, b)| s.replace(a, b));
    let p: BTreeMap<String, Vec<u8>> = parts(&save(doc))
        .into_iter()
        .map(|(k, v)| {
            let k = rename(&k);
            let v = if k.ends_with(".rels") || k == "[Content_Types].xml" {
                rename(&String::from_utf8(v).unwrap()).into_bytes()
            } else if k.ends_with("item2.xml") {
                payload(String::from_utf8(v).unwrap()).into_bytes()
            } else {
                v
            };
            (k, v)
        })
        .collect();
    assert!(p.keys().any(|k| k.ends_with("itemProps2.xml")), "{:?}", p.keys().collect::<Vec<_>>());
    zip(&p)
}

fn assert_one_props_per_id(p: &BTreeMap<String, Vec<u8>>) {
    let mut ids = item_ids(p);
    ids.sort();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "an itemID twice: {:?}", p.keys().collect::<Vec<_>>());
    assert!(dangling(p).is_empty(), "{:?}", dangling(p));
}

#[test]
fn a_fit_store_renamed_by_word_is_read_and_written_once() {
    let doc = with_fit(report());
    let mut back = open(&fit_renamed_by_word(&doc, |x| x));
    assert_eq!(back.fit_state, doc.fit_state, "found by its content");
    back.fit_state = Some(wordcraft_doc::FitState { cum_delta: 2.0, ..Default::default() });
    let saved = save(&back);
    let p = parts(&saved);
    assert_one_props_per_id(&p);
    assert_eq!(item_ids(&p).iter().filter(|id| *id == FIT_ITEM_ID).count(), 1);
    assert_eq!(wordcraft_docx::read(&saved).unwrap().fit_state, back.fit_state);
}

#[test]
fn a_kept_copy_of_a_store_never_duplicates_its_id() {
    // A renamed fit store WordCraft can't read (broken) stays as Word's data, until a fit store is
    // written: then the copy with the same data store id goes.
    let doc = with_fit(report());
    let bytes = fit_renamed_by_word(&doc, |x| x.replace("<fitState>", "<fitState>{broken"));
    let mut back = open(&bytes);
    assert!(back.fit_state.is_none());
    back.fit_state = Some(wordcraft_doc::FitState { cum_delta: 3.0, ..Default::default() });
    let saved = save(&back);
    let p = parts(&saved);
    assert_one_props_per_id(&p);
    assert_eq!(item_ids(&p).iter().filter(|id| *id == FIT_ITEM_ID).count(), 1);
    assert!(!p.keys().any(|k| k.ends_with("item2.xml")), "{:?}", p.keys().collect::<Vec<_>>());
    assert_eq!(wordcraft_docx::read(&saved).unwrap().fit_state, back.fit_state);
}

#[test]
fn a_broken_fit_store_under_its_own_name_is_dropped() {
    let mut p = parts(&save(&with_fit(report())));
    let name = p.keys().find(|k| k.to_ascii_lowercase().ends_with("customxml/wordcraftfit.xml")).cloned().unwrap();
    let broken = String::from_utf8(p[&name].clone()).unwrap().replace("<fitState>", "<fitState>{broken");
    p.insert(name, broken.into_bytes());
    let back = open(&zip(&p));
    assert!(back.fit_state.is_none());
    assert!(!back.passthrough.keys().any(|k| k.to_ascii_lowercase().contains("wordcraftfit")), "{:?}", back.passthrough.keys().collect::<Vec<_>>());
    let saved = parts(&save(&back));
    assert!(!item_ids(&saved).iter().any(|id| id == FIT_ITEM_ID));
    assert!(dangling(&saved).is_empty(), "{:?}", dangling(&saved));
}

#[test]
fn vector_results_round_trip_as_word_matrices() {
    // A literal of many long elements, built cell by cell (Word keeps a matrix of up to 1000 rows).
    let long: Vec<String> = (0..1000).map(|k| format!("{}.5e-{} m", k % 10, k % 9 + 7)).collect();
    let long = format!("E := [{}]", long.join(", "));
    let sources = [
        "D := [0.840, 1.050] in = ? mm",
        "x := 0..59 =",
        "v := [2 in, 3 lbf] =",
        "M := [1, 2; 3, 4] =",
        "S := [1e-9, 2e9] =",
        "i := 0..9999",
        "r := transpose(i) =",
        "Mx := mmul(x, transpose(x)) =",
        long.as_str(),
    ];
    let mut doc = Document::new();
    doc.body = sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut p = Paragraph::with_text("", CharProps::default());
            p.insert_object(0, calc_eq(i as u64 + 1, s, false), &CharProps::default()).unwrap();
            para_block(p)
        })
        .collect();
    wordcraft_calc::recalc(&mut doc, &|_| None);
    let bytes = save(&doc);
    let xml = document_xml(&bytes);
    assert!(xml.contains("<m:m>") || xml.contains("<m:m "), "a vector result is a Word matrix");
    assert!(xml.contains('⋮') && xml.contains("(60 rows)"), "the cut is in Word's equation too");
    assert!(xml.contains('⋯') && xml.contains('⋱'), "so are a column cut and the corner");
    assert!(xml.contains("(10000 columns)") && xml.contains("(60 rows × 60 columns)"));
    let back = open(&bytes);
    for (i, (r, linear)) in regions(&back).iter().enumerate() {
        assert_eq!(r.state, CalcState::Fresh, "{} came back as {:?} ({linear})", sources[i], r.state);
        assert_eq!(linear, &region(&doc, i as u64 + 1).1);
    }
}

// ---------------------------------------------------------------------------------------------
// Calculation data tables
// ---------------------------------------------------------------------------------------------

/// The B31.3 pipe table: headings, then three rows of values.
const PIPES: [[&str; 3]; 4] = [["NPS (in)", "D_o (in)", "t_nom (in)"], ["1/2", "0.840", "0.109"], ["3/4", "1.050", "0.113"], ["1", "1.315", "0.133"]];

/// The id the pipe table is marked with: not 1, so it can't pass for a region's id by chance.
const PIPE_TABLE: u64 = 7;

/// The B31.3 example: the pipe table marked as calculation data (block 0), then regions 1 to 7
/// (blocks 1 to 7), recalculated. The margin is `margin`, not `m`: `m` is the metre.
fn pipe_doc() -> Document {
    let sources =
        ["P := 1000 psi", "S := 20000 psi", "E := 1", "Y := 0.4", "t_min := P·D_o/(2·(S·E + P·Y)) =", "margin := t_nom - t_min =", "min(margin) ="];
    let table = wordcraft_doc::Table {
        rows: PIPES
            .iter()
            .map(|r| wordcraft_doc::Row { cells: r.iter().map(|t| wordcraft_doc::Cell::with_text(t)).collect(), ..Default::default() })
            .collect(),
        grid: vec![100.0; 3],
        calc: Some(Box::new(CalcTable { id: PIPE_TABLE, errors: Vec::new() })),
        ..Default::default()
    };
    let mut d = Document::new();
    d.body = vec![std::sync::Arc::new(Block::Table(table))];
    for (i, s) in sources.iter().enumerate() {
        let mut p = Paragraph::with_text("", CharProps::default());
        p.insert_object(0, calc_eq(i as u64 + 1, s, false), &CharProps::default()).unwrap();
        d.body.push(para_block(p));
    }
    wordcraft_calc::recalc(&mut d, &|_| None);
    d
}

/// The table at body block `at`.
fn table_at(doc: &Document, at: usize) -> &wordcraft_doc::Table {
    doc.body[at].as_table().unwrap_or_else(|| panic!("block {at} isn't a table"))
}

/// Each cell's text (its paragraphs' text, trimmed), row by row.
fn cell_texts(t: &wordcraft_doc::Table) -> Vec<Vec<String>> {
    t.rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.final_text()).collect::<Vec<_>>().join("\n").trim().to_string())
                .collect()
        })
        .collect()
}

/// The `w:tbl` … `</w:tbl>` of the first table in `xml`.
fn first_tbl(xml: &str) -> &str {
    let start = xml.find("<w:tbl>").expect("a table");
    let end = start + xml[start..].find("</w:tbl>").unwrap() + "</w:tbl>".len();
    &xml[start..end]
}

/// `bytes` opened in WordCraft, with the report of bringing its calculations back.
fn open_with_report(bytes: &[u8]) -> (Document, wordcraft_calc::RecalcReport) {
    let mut doc = wordcraft_docx::read(bytes).unwrap();
    let store = doc.calc_store.take();
    let report = import(&mut doc, store);
    (doc, report)
}

#[test]
fn data_table_round_trips_through_docx() {
    let doc = pipe_doc();
    assert!(region(&doc, 7).1.ends_with("=0.087 \"in\""), "{}", region(&doc, 7).1);
    let before = doc.clone();
    let bytes = save(&doc);
    assert_eq!(doc, before, "saving changes nothing in the live document");

    // Word gets the table in a content control tagged with its id, holding exactly the typed text.
    let xml = document_xml(&bytes);
    let tag = format!(r#"<w:tag w:val="wordcraft-calc-table:{PIPE_TABLE}"/>"#);
    let at = xml.find(&tag).unwrap_or_else(|| panic!("no {tag}"));
    let sdt_start = xml[..at].rfind("<w:sdt>").unwrap();
    let content = at + xml[at..].find("<w:sdtContent>").unwrap() + "<w:sdtContent>".len();
    assert!(xml[content..].starts_with("<w:tbl>"), "the control's content starts with the table: {}", &xml[sdt_start..content + 40]);
    let tbl = first_tbl(&xml);
    assert!(xml[content + tbl.len()..].starts_with("</w:sdtContent></w:sdt>"), "and ends with it");
    for text in PIPES.iter().flatten() {
        assert!(tbl.contains(&format!(">{text}<")), "{text} in {tbl}");
    }
    let p = parts(&bytes);
    let store = String::from_utf8(p[&store_part(&p)].clone()).unwrap().replace("&quot;", "\"");
    assert!(store.contains(&format!(r#""tables":[{{"id":{PIPE_TABLE},"fingerprint":"#)), "{store}");

    let (back, report) = open_with_report(&bytes);
    assert_eq!(report.tables_edited, 0);
    let t = table_at(&back, 0);
    assert_eq!(t.calc.as_deref(), Some(&CalcTable { id: PIPE_TABLE, errors: Vec::new() }), "marked again with the same id");
    assert!(t.controls.is_empty(), "the control is WordCraft's bookkeeping: gone once read");
    assert!(!has_markers(&back));
    assert_eq!(cell_texts(t), PIPES.map(|r| r.map(String::from).to_vec()).to_vec());
    let got: Vec<(u64, CalcState, String)> = regions(&back).into_iter().map(|(r, l)| (r.id, r.state, l)).collect();
    let want: Vec<(u64, CalcState, String)> = regions(&doc).into_iter().map(|(r, l)| (r.id, r.state, l)).collect();
    assert_eq!(got, want, "the results are the same");
    assert!(want.iter().all(|(_, s, _)| *s == CalcState::Fresh));
    // A second round trip is just as clean.
    let (again, report) = open_with_report(&save(&back));
    assert_eq!(report.tables_edited, 0);
    assert_eq!(table_at(&again, 0).calc, t.calc);
    assert_eq!(regions(&again), regions(&back));
}

#[test]
fn word_sees_a_plain_table() {
    let marked = pipe_doc();
    let mut plain = marked.clone();
    if let Block::Table(t) = std::sync::Arc::make_mut(&mut plain.body[0]) {
        t.calc = None;
    }
    let marked_xml = document_xml(&save(&marked));
    let plain_xml = document_xml(&save(&plain));
    assert!(!plain_xml.contains("wordcraft-calc-table:"));
    // The table itself is written exactly as an unmarked one: no field codes, no extra runs.
    let tbl = first_tbl(&marked_xml);
    assert_eq!(tbl, first_tbl(&plain_xml));
    for code in ["w:fldChar", "w:instrText", "w:fldSimple", "<w:sdt>", "WCCALC", "<m:oMath"] {
        assert!(!tbl.contains(code), "{code} in {tbl}");
    }
    // The only addition is the control around it, which reviewers may edit.
    let at = marked_xml.find(tbl).unwrap();
    let sdt = &marked_xml[marked_xml[..at].rfind("<w:sdt>").unwrap()..at];
    assert!(sdt.ends_with("</w:sdtPr><w:sdtContent>"), "{sdt}");
    assert!(sdt.contains("wordcraft-calc-table:") && !sdt.contains("<w:lock"), "{sdt}");
    assert!(marked_xml[at + tbl.len()..].starts_with("</w:sdtContent></w:sdt>"));
    let (before, after) = (&marked_xml[..at], &marked_xml[at + tbl.len()..]);
    let (plain_at, _) = plain_xml.match_indices(tbl).next().unwrap();
    assert_eq!(
        format!("{}{}", &before[..before.len() - sdt.len()], &after["</w:sdtContent></w:sdt>".len()..]),
        format!("{}{}", &plain_xml[..plain_at], &plain_xml[plain_at + tbl.len()..]),
        "nothing else differs"
    );
}

#[test]
fn table_edited_in_word_is_used_and_noticed() {
    let bytes = save(&pipe_doc());
    // A reviewer changes t_nom of the 3/4 pipe.
    let edited = edit_document(&bytes, |xml| {
        assert_eq!(xml.matches(">0.113<").count(), 1);
        xml.replace(">0.113<", ">0.120<")
    });
    let (back, report) = open_with_report(&edited);
    assert_eq!(report.tables_edited, 1);
    assert_eq!(report.edited, 0, "no equation was edited");
    let t = table_at(&back, 0);
    assert_eq!(t.calc.as_deref(), Some(&CalcTable { id: PIPE_TABLE, errors: Vec::new() }), "still data: the new value is an input");
    assert_eq!(cell_texts(t)[2][2], "0.120");
    let (min, linear) = region(&back, 7);
    assert_eq!(min.state, CalcState::Fresh, "{linear}");
    assert!(linear.ends_with("=0.088 \"in\""), "the new value is used: {linear}");
    // Saved again, the edit is WordCraft's own data now: not noticed a second time.
    assert_eq!(open_with_report(&save(&back)).1.tables_edited, 0);

    // A reviewer adds a row and leaves it empty: an error naming the row, never a 0.
    let empty_row = "<w:tr><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc><w:tc><w:p/></w:tc></w:tr>";
    let edited = edit_document(&bytes, |xml| xml.replacen("</w:tbl>", &format!("{empty_row}</w:tbl>"), 1));
    let (back, report) = open_with_report(&edited);
    assert_eq!(report.tables_edited, 1);
    let t = table_at(&back, 0);
    assert_eq!(t.rows.len(), 5);
    let errors = &t.calc.as_deref().expect("still data").errors;
    assert!(errors.contains(&ColumnError { col: Some(2), message: "t_nom row 4 isn't a number".into() }), "{errors:?}");
    let (min, linear) = region(&back, 7);
    assert!(matches!(min.state, CalcState::Error(_)), "{linear}");
    assert!(!linear.contains("0.08"), "no number from the table in error: {linear}");
}

#[test]
fn table_without_store_stays_plain() {
    let (copy, _) = export(&pipe_doc());
    // The control is written, the store isn't.
    let bytes = wordcraft_docx::write(&copy).unwrap();
    assert!(document_xml(&bytes).contains(&format!("wordcraft-calc-table:{PIPE_TABLE}")));
    let (back, report) = open_with_report(&bytes);
    assert_eq!(report.tables_edited, 0);
    let t = table_at(&back, 0);
    assert_eq!(t.calc, None, "a plain table");
    assert!(t.controls.is_empty(), "with its control removed");
    assert_eq!(cell_texts(t), PIPES.map(|r| r.map(String::from).to_vec()).to_vec());
    assert!(!has_markers(&back));
}

#[test]
fn hostile_table_entries_leave_tables_plain() {
    let doc = pipe_doc();
    let bytes = save(&doc);
    let (_, store) = export(&doc);
    let regions_json = serde_json::to_value(&store.regions).unwrap();
    let table = serde_json::to_value(store.tables[0]).unwrap();
    // The table's id stored twice: which entry is right can't be known.
    let twice = serde_json::json!({ "settings": store.settings, "regions": regions_json, "tables": [table, table] }).to_string();
    // Tables count toward the bound on stored entries.
    let over = {
        let entries: Vec<String> = (0..100_000u64).map(|i| format!(r#"{{"id":{},"source":"a","fingerprint":0}}"#, i + 100)).collect();
        format!(r#"{{"settings":{{}},"regions":[{}],"tables":[{table}]}}"#, entries.join(","))
    };
    // A fingerprint is only ever "edited or not": a wrong one is no reason to drop the table.
    let mut wrong = table.clone();
    wrong["fingerprint"] = 0.into();
    let wrong = serde_json::json!({ "settings": store.settings, "regions": regions_json, "tables": [wrong] }).to_string();
    for (json, marked) in [(twice.as_str(), false), (over.as_str(), false), (wrong.as_str(), true), (r#"{"tables": 7}"#, false)] {
        let (back, report) = open_with_report(&with_store_json(&bytes, json));
        let t = table_at(&back, 0);
        assert_eq!(t.calc.is_some(), marked, "store {json:.60}");
        assert!(t.controls.is_empty() && !has_markers(&back), "store {json:.60}");
        assert_eq!(report.tables_edited, usize::from(marked), "store {json:.60}");
    }
}

#[test]
fn text_typed_inside_the_table_control_in_word_is_kept() {
    // Word lets a reviewer type a paragraph after the table, inside its control.
    let edited = edit_document(&save(&pipe_doc()), |xml| {
        assert_eq!(xml.matches("</w:tbl></w:sdtContent>").count(), 1);
        xml.replace("</w:tbl></w:sdtContent>", "</w:tbl><w:p><w:r><w:t>Checked</w:t></w:r></w:p></w:sdtContent>")
    });
    let (back, report) = open_with_report(&edited);
    assert_eq!(report.tables_edited, 0);
    let t = table_at(&back, 0);
    assert_eq!(t.calc.as_deref().map(|c| c.id), Some(PIPE_TABLE));
    assert!(t.controls.is_empty() && !has_markers(&back), "{:?}", t.controls);
    assert!(back.body[1].as_para().is_some_and(|p| p.final_text() == "Checked"));
    assert!(region(&back, 7).1.ends_with("=0.087 \"in\""));
    assert!(!back.has_unbalanced_controls());
}

#[test]
fn origin_round_trips_through_the_store() {
    let mut doc = pipe_doc();
    doc.calc_settings = Some(CalcSettings { origin: 1, ..CalcSettings::default() });
    let mut p = Paragraph::with_text("", CharProps::default());
    p.insert_object(0, calc_eq(8, "D_o[1] =", false), &CharProps::default()).unwrap();
    doc.body.push(para_block(p));
    wordcraft_calc::recalc(&mut doc, &|_| None);
    assert!(region(&doc, 8).1.ends_with("=0.84 \"in\""), "{}", region(&doc, 8).1);
    let back = open(&save(&doc));
    assert_eq!(back.calc_settings.as_ref().map(|s| s.origin), Some(1));
    assert_eq!(region(&back, 8).1, region(&doc, 8).1);
}

/// A document with a solve block: a guess, `Given`, one equation and the region that closes it.
fn block_doc() -> Document {
    let mut d = Document::new();
    d.body = ["x := 1", "Given", "x^2 == 2", "x_s := Find(x) = ?"]
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut p = Paragraph::with_text("", CharProps::default());
            p.insert_object(0, calc_eq(i as u64 + 1, s, false), &CharProps::default()).unwrap();
            para_block(p)
        })
        .collect();
    wordcraft_calc::recalc(&mut d, &|_| None);
    d
}

#[test]
fn block_survives_word() {
    let doc = block_doc();
    for (r, linear) in regions(&doc) {
        assert_eq!(r.state, CalcState::Fresh, "{} → {linear}", r.source);
    }
    let solved = region(&doc, 4).1;
    assert!(solved.contains("1.4"), "the block solved before saving: {solved}");

    // Saved and reopened: the same sources, the same results.
    let bytes = save(&doc);
    let back = open(&bytes);
    assert!(!has_markers(&back));
    for (i, source) in ["x := 1", "Given", "x^2 == 2", "x_s := Find(x) = ?"].iter().enumerate() {
        let (r, linear) = region(&back, i as u64 + 1);
        assert_eq!(&r.source, source);
        assert_eq!(r.state, CalcState::Fresh, "{source} came back as {:?} ({linear})", r.state);
        assert_eq!(linear, region(&doc, i as u64 + 1).1, "{source}");
    }
    assert_eq!(regions(&open(&save(&back))), regions(&back), "a second round trip is just as clean");

    // The equation edited in Word: it is flagged, and the region that closes its block fails.
    let edited = edit_document(&bytes, |xml| {
        let sdt = sdt_of(&xml, 3);
        let changed = sdt.replacen(r#"xml:space="preserve">2</m:t></m:r></m:oMath>"#, r#"xml:space="preserve">3</m:t></m:r></m:oMath>"#, 1);
        assert_ne!(sdt, changed, "the equation's right side is found: {sdt}");
        xml.replace(&sdt, &changed)
    });
    let back = open(&edited);
    let (eq, linear) = region(&back, 3);
    assert_eq!(eq.state, CalcState::EditedOutside);
    assert_eq!(eq.source, "x^2 == 2", "the source is kept for the author to resolve");
    assert!(linear.ends_with(" ⚠"), "the edited equation is marked: {linear}");
    let (closing, linear) = region(&back, 4);
    assert_eq!(closing.state, CalcState::Error("depends on a calculation edited in Word — resolve it".into()), "{linear}");
    assert!(!linear.contains("1.4") && linear.contains('⚠'), "{linear}");
    // The guess and the Given are untouched.
    for id in [1, 2] {
        assert_eq!(region(&back, id).0.state, CalcState::Fresh);
    }
}
