//! Settings a save used to drop: `word/settings.xml` is merged into the file's original, so
//! settings the model doesn't hold (compatibility options, rsids, document variables, proofing
//! state, "remove personal information"…) survive, and owned ones follow the model. Hand-written
//! fixtures (from the spec), no files produced by Word.

use std::collections::BTreeMap;
use std::io::{Read, Write};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zw.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        assert!(out.insert(f.name().to_string(), b).is_none(), "duplicate part {}", f.name());
    }
    out
}

fn text(files: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(files.get(name).unwrap_or_else(|| panic!("no {name} in {:?}", files.keys())).clone()).unwrap()
}

fn rels(list: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s =
        String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, ty, target) in list {
        s.push_str(&format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/>"#));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

/// A package: `body`, document relationships, root relationships after the main document's,
/// and extra parts.
fn package(body: &str, doc_rels: &[(&str, &str, &str)], root_rels: &[(&str, &str, &str)], extra: &[(&str, &str)]) -> Vec<u8> {
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R_NS}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body>{body}</w:body></w:document>"#
    );
    let mut root = vec![("rIdMain", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument", "word/document.xml")];
    root.extend_from_slice(root_rels);
    let mut entries: Vec<(&str, Vec<u8>)> =
        vec![("_rels/.rels", rels(&root)), ("word/document.xml", doc.into_bytes()), ("word/_rels/document.xml.rels", rels(doc_rels))];
    for (p, c) in extra {
        entries.push((p, c.as_bytes().to_vec()));
    }
    zip(&entries)
}

fn save(doc: &wordcraft_doc::Document) -> BTreeMap<String, Vec<u8>> {
    unzip(&wordcraft_docx::write(doc).expect("write"))
}

const SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" mc:Ignorable="w14 w15"><w:zoom w:percent="120"/><w:removePersonalInformation/><w:removeDateAndTime/><w:proofState w:spelling="clean" w:grammar="clean"/><w:mailMerge><w:mainDocumentType w:val="formLetters"/><w:dataSource r:id="rId9"/></w:mailMerge><w:trackRevisions/><w:defaultTabStop w:val="708"/><w:hyphenationZone w:val="425"/><w:characterSpacingControl w:val="doNotCompress"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/><w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/><w:compatSetting w:name="enableOpenTypeFeatures" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/></w:compat><w:docVars><w:docVar w:name="ClauseSet" w:val="B"/></w:docVars><w:rsids><w:rsidRoot w:val="00A1B2C3"/><w:rsid w:val="00A1B2C3"/></w:rsids><m:mathPr><m:mathFont m:val="Cambria Math"/><m:smallFrac m:val="0"/></m:mathPr><w:themeFontLang w:val="pl-PL"/><w:decimalSymbol w:val=","/><w:listSeparator w:val=";"/><w14:docId w14:val="1A2B3C4D"/></w:settings>"#;

fn with_settings(settings: &str) -> Vec<u8> {
    package("<w:p><w:r><w:t>Text</w:t></w:r></w:p>", &[("rId1", &format!("{REL}/settings"), "settings.xml")], &[], &[("word/settings.xml", settings)])
}

fn pos(s: &str, needle: &str) -> usize {
    s.find(needle).unwrap_or_else(|| panic!("{needle} missing from {s}"))
}

#[test]
fn unmodelled_settings_survive_a_save_and_owned_ones_follow_the_model() {
    let mut doc = wordcraft_docx::read(&with_settings(SETTINGS)).unwrap();
    assert!(doc.settings.track_changes);
    doc.settings.track_changes = false;
    let files = save(&doc);
    let s = text(&files, "word/settings.xml");
    // Kept byte for byte: everything the model doesn't hold, the original root and zoom.
    for kept in [
        r#"<w:zoom w:percent="120"/>"#,
        r#"<w:proofState w:spelling="clean" w:grammar="clean"/>"#,
        r#"w:name="overrideTableStyleFontSizeAndJustification""#,
        r#"w:name="enableOpenTypeFeatures""#,
        r#"<w:docVar w:name="ClauseSet" w:val="B"/>"#,
        r#"<w:rsidRoot w:val="00A1B2C3"/>"#,
        r#"<m:mathFont m:val="Cambria Math"/>"#,
        r#"<w:themeFontLang w:val="pl-PL"/>"#,
        r#"<w:decimalSymbol w:val=","/>"#,
        r#"<w:listSeparator w:val=";"/>"#,
        r#"<w14:docId w14:val="1A2B3C4D"/>"#,
        r#"mc:Ignorable="w14 w15""#,
    ] {
        assert!(s.contains(kept), "{kept} lost: {s}");
    }
    // Owned: Track Changes was turned off; one compatibility mode; the mail merge source (a
    // relationship that isn't carried) is dropped rather than left dangling.
    assert!(!s.contains("trackRevisions"), "{s}");
    assert_eq!(s.matches("compatibilityMode").count(), 1, "{s}");
    assert!(!s.contains("mailMerge") && !s.contains("rId9"), "{s}");
    // The privacy options are the model's (#1062): written once each, and gone when turned off.
    assert_eq!(s.matches("removePersonalInformation").count(), 1, "{s}");
    assert_eq!(s.matches("removeDateAndTime").count(), 1, "{s}");
    // CT_Settings order holds for kept and owned children.
    assert!(pos(&s, "removePersonalInformation") < pos(&s, "defaultTabStop"));
    assert!(pos(&s, "defaultTabStop") < pos(&s, "<w:compat>"));
    assert!(pos(&s, "<w:compat>") < pos(&s, "<w:rsids>"));
    // Turning Track Changes back on inserts it at its place.
    doc.settings.track_changes = true;
    let s = text(&save(&doc), "word/settings.xml");
    assert!(pos(&s, "proofState") < pos(&s, "<w:trackRevisions/>") && pos(&s, "<w:trackRevisions/>") < pos(&s, "defaultTabStop"), "{s}");
    let mut private = doc.clone();
    private.settings.remove_personal_info = false;
    private.settings.remove_date_time = false;
    let p = text(&save(&private), "word/settings.xml");
    assert!(!p.contains("removePersonalInformation") && !p.contains("removeDateAndTime"), "{p}");
    // A second generation is identical.
    let again = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(text(&save(&again), "word/settings.xml"), s);
}

#[test]
fn settings_written_by_wordcraft_keep_no_original() {
    let doc = wordcraft_doc::Document::new();
    let back = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert!(!back.passthrough.contains_key("wordcraft:settings.orig"), "{:?}", back.passthrough.keys());
    assert_eq!(text(&save(&back), "word/settings.xml"), text(&save(&doc), "word/settings.xml"));
}

/// Gap #5: `w:doNotTrackMoves` (ECMA-376 §17.15.1.29) turns Track moves off; it is read, written
/// back right after `w:trackRevisions` (in a merged part and a generated one), and left out
/// while moves are tracked.
#[test]
fn do_not_track_moves_round_trips() {
    let original = SETTINGS.replace("<w:trackRevisions/>", "<w:trackRevisions/><w:doNotTrackMoves/>");
    let mut doc = wordcraft_docx::read(&with_settings(&original)).unwrap();
    assert!(!doc.settings.track_moves);
    let s = text(&save(&doc), "word/settings.xml");
    assert_eq!(s.matches("<w:doNotTrackMoves/>").count(), 1, "{s}");
    assert!(pos(&s, "<w:trackRevisions/>") < pos(&s, "<w:doNotTrackMoves/>") && pos(&s, "<w:doNotTrackMoves/>") < pos(&s, "<w:defaultTabStop"));
    assert!(s.contains("ClauseSet"), "a merged part: {s}");
    doc.settings.track_moves = true;
    let s = text(&save(&doc), "word/settings.xml");
    assert!(!s.contains("doNotTrackMoves"), "{s}");

    // `w:val="false"` keeps moves tracked.
    let off = SETTINGS.replace("<w:trackRevisions/>", r#"<w:trackRevisions/><w:doNotTrackMoves w:val="false"/>"#);
    assert!(wordcraft_docx::read(&with_settings(&off)).unwrap().settings.track_moves);

    // A document of our own: absent by default, written when off, and read back.
    let mut doc = wordcraft_doc::Document::new();
    assert!(doc.settings.track_moves);
    assert!(!text(&save(&doc), "word/settings.xml").contains("doNotTrackMoves"));
    doc.settings.track_moves = false;
    let s = text(&save(&doc), "word/settings.xml");
    assert!(s.contains("<w:doNotTrackMoves/>"), "{s}");
    assert!(!wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap().settings.track_moves);
}

#[test]
fn settings_merge_does_not_carry_a_document_type_declaration() {
    for declaration in
        [r#"<!DOCTYPE w:settings [<!ENTITY unused "original">]>"#, r#"<!DOCTYPE w:settings SYSTEM "https://invalid.example/settings.dtd">"#]
    {
        let original = SETTINGS.replacen("<w:settings", &format!("{declaration}<w:settings"), 1);
        let doc = wordcraft_docx::read(&with_settings(&original)).unwrap();
        let s = text(&save(&doc), "word/settings.xml");
        assert!(!s.contains("<!DOCTYPE"), "{s}");
        // Falling back to the generated settings still writes what the model owns.
        assert!(s.contains(r#"<w:defaultTabStop w:val="708"/>"#), "{s}");
        let back = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
        assert_eq!(back.settings, doc.settings);
    }
}

#[test]
fn settings_merge_drops_unowned_relationships_by_namespace() {
    for child in [
        format!(r#"<w:saveThroughXslt xmlns:q="{REL}" q:id="missing"/>"#),
        format!(r#"<w:unknown><w:source xmlns:q="{REL}" q:embed="missing"/></w:unknown>"#),
        r#"<w:saveThroughXslt xmlns:q="http://purl.oclc.org/ooxml/officeDocument/relationships" q:id="missing"/>"#.into(),
        r#"<w:saveThroughXslt r:id = "missing"/>"#.into(),
        r#"<w:unknown><w:a xmlns:r="urn:original" r:id="ordinary"/><w:b r:id="missing"/></w:unknown>"#.into(),
        r#"<w:saveThroughXslt xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationship&#115;" r:id="missing"/>"#.into(),
        r#"<w:saveThroughXslt xmlns:q="http://purl.oclc.org/ooxml/officeDocument/relation&#x73;hips" q:id="missing"/>"#.into(),
    ] {
        let original = SETTINGS.replace("</w:settings>", &format!("{child}</w:settings>"));
        let doc = wordcraft_docx::read(&with_settings(&original)).unwrap();
        let s = text(&save(&doc), "word/settings.xml");
        assert!(!s.contains("missing"), "{child}: {s}");
        assert!(s.contains(r#"w:name="ClauseSet" w:val="B""#), "unrelated settings: {s}");
        assert_eq!(text(&save(&wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap()), "word/settings.xml"), s);
    }
}

#[test]
fn settings_merge_keeps_relationship_like_text_and_unrelated_attributes() {
    for child in [
        r#"<w:docVars><w:docVar w:name="Original" w:val="r:id=literal"/></w:docVars>"#,
        r#"<w:unknown xmlns:r="urn:original" r:id="ordinary"/>"#,
        r#"<w:unknown id="ordinary"><!-- r:id=comment --><?original r:id=instruction?></w:unknown>"#,
        r#"<w:unknown w:id="ordinary"/>"#,
        r#"<w:unknown xmlns:q="urn:original&amp;literal"><w:a q:id="ordinary"/></w:unknown>"#,
    ] {
        let original = SETTINGS.replace("</w:settings>", &format!("{child}</w:settings>"));
        let doc = wordcraft_docx::read(&with_settings(&original)).unwrap();
        let s = text(&save(&doc), "word/settings.xml");
        assert!(s.contains(child), "{child}: {s}");
    }
    let original = SETTINGS.replacen("<w:settings", "<!-- A harmless DOCTYPE mention --><?original preserve?><w:settings", 1);
    let doc = wordcraft_docx::read(&with_settings(&original)).unwrap();
    let s = text(&save(&doc), "word/settings.xml");
    assert!(s.contains("<!-- A harmless DOCTYPE mention --><?original preserve?>"), "{s}");
    assert!(s.contains("ClauseSet"), "{s}");
}

/// Both inputs stay below the reader's per-element namespace and nesting limits. Grow the
/// active bindings and the attributes eightfold: resolving each attribute by scanning the
/// bindings used to multiply those two costs.
#[test]
fn settings_merge_namespace_lookup_scales_with_the_settings_part() {
    use std::fmt::Write as _;

    let fixture = |levels: usize, count: usize| {
        let mut xml =
            format!(r#"<w:settings xmlns:w="{W}" xmlns:r="{REL}" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><w:unknown>"#);
        for level in 0..levels {
            xml.push_str("<w:n");
            for prefix in 0..120 {
                write!(xml, r#" xmlns:p{level}_{prefix}="urn:original:{level}:{prefix}""#).unwrap();
            }
            xml.push('>');
        }
        for _ in 0..count {
            xml.push_str(r#"<w:a w:val="Original"/>"#);
        }
        for _ in 0..levels {
            xml.push_str("</w:n>");
        }
        xml.push_str("</w:unknown></w:settings>");
        let mut doc = wordcraft_doc::Document::new();
        doc.passthrough.insert("wordcraft:settings.orig".into(), xml.into_bytes().into());
        doc
    };
    let small = fixture(24, 4_000);
    let big = fixture(192, 32_000);
    for (doc, count) in [(&small, 4_000), (&big, 32_000)] {
        let settings = text(&save(doc), "word/settings.xml");
        assert_eq!(settings.matches(r#"<w:a w:val="Original"/>"#).count(), count);
        assert!(settings.contains("<w:defaultTabStop"));
    }
    let (small_cost, big_cost, ratio) = super::content_control_save_cost::cost_ratio(
        8,
        || {
            wordcraft_docx::write(&small).unwrap();
        },
        || {
            wordcraft_docx::write(&big).unwrap();
        },
    );
    eprintln!("settings merge: {small_cost:.4}s -> {big_cost:.4}s, ratio {ratio:.1}");
    assert!(ratio < 24.0, "eight times the settings took {ratio:.1}x the time");
}
