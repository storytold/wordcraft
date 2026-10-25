//! What a save keeps from a Word file beyond the document model (settings: `settings_merge.rs`):
//! package parts WordCraft doesn't write (carried verbatim),
//! comment durable ids and UTC dates, localized default styles, latent styles, list identities
//! and extended properties. Hand-written fixtures (from the spec), no files produced by Word.

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

fn pos(s: &str, needle: &str) -> usize {
    s.find(needle).unwrap_or_else(|| panic!("{needle} missing from {s}"))
}

const CT: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/webSettings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml"/><Override PartName="/word/stylesWithEffects.xml" ContentType="application/vnd.ms-word.stylesWithEffects+xml"/><Override PartName="/word/people.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml"/><Override PartName="/word/webextensions/taskpanes.xml" ContentType="application/vnd.ms-office.webextensiontaskpanes+xml"/><Override PartName="/word/webextensions/webextension1.xml" ContentType="application/vnd.ms-office.webextension+xml"/><Override PartName="/word/unknownPart.xml" ContentType="application/x-unknown+xml"/></Types>"#;

#[test]
fn unmodelled_parts_are_carried_with_relationships_and_content_types() {
    let web = r#"<w:webSettings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:optimizeForBrowser/><w:allowPNG/></w:webSettings>"#;
    let swe =
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="X"/></w:styles>"#;
    let people = r#"<w15:people xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:person w15:author="Ann"><w15:presenceInfo w15:providerId="None" w15:userId="Ann"/></w15:person></w15:people>"#;
    let panes = r#"<wetp:taskpanes xmlns:wetp="http://schemas.microsoft.com/office/webextensions/taskpanes/2010/11" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><wetp:taskpane dockstate="right" visibility="0" width="350" row="1"><wetp:webextensionref r:id="rId1"/></wetp:taskpane></wetp:taskpanes>"#;
    let ext = r#"<we:webextension xmlns:we="http://schemas.microsoft.com/office/webextensions/webextension/2010/11" id="{00000000-0000-0000-0000-000000000001}"><we:reference id="wa0" version="1.0.0.0" store="en-US" storeType="OMEX"/></we:webextension>"#;
    let unknown = r#"<x:thing xmlns:x="urn:example:future"/>"#;
    let thumb = "\u{ff}\u{d8}\u{ff}JPEG";
    let panes_rels =
        String::from_utf8(rels(&[("rId1", "http://schemas.microsoft.com/office/2011/relationships/webextension", "webextension1.xml")])).unwrap();
    let bytes = package(
        r#"<w:p><w:r><w:t>Body</w:t></w:r></w:p>"#,
        &[
            ("rId1", &format!("{REL}/webSettings"), "webSettings.xml"),
            ("rId2", "http://schemas.microsoft.com/office/2007/relationships/stylesWithEffects", "stylesWithEffects.xml"),
            ("rId3", "http://schemas.microsoft.com/office/2011/relationships/people", "people.xml"),
            ("rId4", "urn:example:future-relationship", "unknownPart.xml"),
        ],
        &[
            ("rIdT", "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail", "docProps/thumbnail.jpeg"),
            ("rIdW", "http://schemas.microsoft.com/office/2011/relationships/webextensiontaskpanes", "word/webextensions/taskpanes.xml"),
        ],
        &[
            ("[Content_Types].xml", CT),
            ("word/webSettings.xml", web),
            ("word/stylesWithEffects.xml", swe),
            ("word/people.xml", people),
            ("word/unknownPart.xml", unknown),
            ("docProps/thumbnail.jpeg", thumb),
            ("word/webextensions/taskpanes.xml", panes),
            ("word/webextensions/_rels/taskpanes.xml.rels", &panes_rels),
            ("word/webextensions/webextension1.xml", ext),
        ],
    );
    let doc = wordcraft_docx::read(&bytes).unwrap();
    let files = save(&doc);
    let input = unzip(&bytes);
    for part in [
        "word/webSettings.xml",
        "word/stylesWithEffects.xml",
        "word/people.xml",
        "word/unknownPart.xml",
        "docProps/thumbnail.jpeg",
        "word/webextensions/taskpanes.xml",
        "word/webextensions/webextension1.xml",
    ] {
        assert_eq!(files.get(part), input.get(part), "{part} not carried verbatim");
    }
    let doc_rels = text(&files, "word/_rels/document.xml.rels");
    for ty in ["/webSettings\"", "/stylesWithEffects\"", "/people\"", "urn:example:future-relationship"] {
        assert_eq!(doc_rels.matches(ty).count(), 1, "{ty}: {doc_rels}");
    }
    let root_rels = text(&files, "_rels/.rels");
    assert!(root_rels.contains(r#"Target="docProps/thumbnail.jpeg""#) && root_rels.contains("webextensiontaskpanes"), "{root_rels}");
    assert!(text(&files, "word/webextensions/_rels/taskpanes.xml.rels").contains("webextension1.xml"));
    let ct = text(&files, "[Content_Types].xml");
    for (part, ty) in [
        ("/word/webSettings.xml", "webSettings+xml"),
        ("/word/stylesWithEffects.xml", "stylesWithEffects+xml"),
        ("/word/people.xml", "people+xml"),
        ("/word/webextensions/taskpanes.xml", "webextensiontaskpanes+xml"),
        ("/docProps/thumbnail.jpeg", "image/jpeg"),
    ] {
        let entry = ct.split("<Override ").find(|o| o.contains(&format!(r#"PartName="{part}""#))).unwrap_or_else(|| panic!("{part}: {ct}"));
        assert!(entry.contains(ty), "{part}: {entry}");
    }
    // Saved again: the same parts, still once each (`unzip` rejects duplicates).
    let again = save(&wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap());
    assert_eq!(again.get("word/webSettings.xml"), input.get("word/webSettings.xml"));
    assert_eq!(text(&again, "word/_rels/document.xml.rels").matches("/webSettings\"").count(), 1);
}

#[test]
fn parts_the_writer_owns_or_the_body_refers_to_are_not_carried() {
    // A second styles part, and an unknown part the body refers to by id: neither is carried
    // (the first would contradict the written styles, the second's element isn't kept).
    let bytes = package(
        r#"<w:p><w:r><w:t>Body</w:t></w:r><w:customThing r:id="rId8"/></w:p>"#,
        &[("rId7", &format!("{REL}/styles"), "other-styles.xml"), ("rId8", "urn:example:referenced", "referenced.xml")],
        &[],
        &[
            ("word/other-styles.xml", r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#),
            ("word/referenced.xml", "<x/>"),
        ],
    );
    let files = save(&wordcraft_docx::read(&bytes).unwrap());
    assert!(!files.contains_key("word/other-styles.xml") && !files.contains_key("word/referenced.xml"), "{:?}", files.keys());
    assert_eq!(text(&files, "word/_rels/document.xml.rels").matches("/styles\"").count(), 1);
}

fn comments_package(with_ids: bool) -> Vec<u8> {
    let comments = r#"<w:comments xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:comment w:id="0" w:author="Ann" w:date="2026-01-02T10:00:00" w:initials="A"><w:p w14:paraId="11111111"><w:r><w:t>First</w:t></w:r></w:p></w:comment><w:comment w:id="1" w:author="Ben" w:date="2026-01-03T11:00:00" w:initials="B"><w:p w14:paraId="22222222"><w:r><w:t>Reply</w:t></w:r></w:p></w:comment></w:comments>"#;
    let ex = r#"<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="11111111" w15:done="0"/><w15:commentEx w15:paraId="22222222" w15:paraIdParent="11111111" w15:done="0"/></w15:commentsEx>"#;
    let ids = r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="11111111" w16cid:durableId="5A1B2C3D"/><w16cid:commentId w16cid:paraId="22222222" w16cid:durableId="0F00BA12"/></w16cid:commentsIds>"#;
    let cex = r#"<w16cex:commentsExtensible xmlns:w16cex="http://schemas.microsoft.com/office/word/2018/wordml/cex"><w16cex:commentExtensible w16cex:durableId="5A1B2C3D" w16cex:dateUtc="2026-01-02T09:00:00Z"/><w16cex:commentExtensible w16cex:durableId="0F00BA12" w16cex:dateUtc="2026-01-03T10:00:00Z"/></w16cex:commentsExtensible>"#;
    let body = r#"<w:p><w:commentRangeStart w:id="0"/><w:commentRangeStart w:id="1"/><w:r><w:t>Text</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:rPr><w:rStyle w:val="Odwoaniedokomentarza"/></w:rPr><w:commentReference w:id="0"/></w:r><w:commentRangeEnd w:id="1"/><w:r><w:rPr><w:rStyle w:val="Odwoaniedokomentarza"/></w:rPr><w:commentReference w:id="1"/></w:r></w:p>"#;
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normalny"><w:name w:val="Normal"/></w:style><w:style w:type="character" w:default="1" w:styleId="Domylnaczcionkaakapitu"><w:name w:val="Default Paragraph Font"/></w:style><w:style w:type="character" w:styleId="Odwoaniedokomentarza"><w:name w:val="annotation reference"/><w:basedOn w:val="Domylnaczcionkaakapitu"/><w:rPr><w:sz w:val="16"/></w:rPr></w:style></w:styles>"#;
    let mut doc_rels = vec![
        ("rId1", format!("{REL}/comments"), "comments.xml"),
        ("rId2", "http://schemas.microsoft.com/office/2011/relationships/commentsExtended".to_string(), "commentsExtended.xml"),
        ("rId5", format!("{REL}/styles"), "styles.xml"),
    ];
    let mut extra = vec![("word/comments.xml", comments), ("word/commentsExtended.xml", ex), ("word/styles.xml", styles)];
    if with_ids {
        doc_rels.push(("rId3", "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds".into(), "commentsIds.xml"));
        doc_rels.push(("rId4", "http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible".into(), "commentsExtensible.xml"));
        extra.push(("word/commentsIds.xml", ids));
        extra.push(("word/commentsExtensible.xml", cex));
    }
    let doc_rels: Vec<(&str, &str, &str)> = doc_rels.iter().map(|(a, b, c)| (*a, b.as_str(), *c)).collect();
    package(body, &doc_rels, &[], &extra)
}

/// (paragraph id → durable id) from commentsIds, and (durable id → UTC date) from
/// commentsExtensible, as written.
fn written_ids(files: &BTreeMap<String, Vec<u8>>) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let attr = |el: &str, name: &str| -> Option<String> { el.split(&format!("{name}=\"")).nth(1)?.split('"').next().map(str::to_string) };
    let ids = text(files, "word/commentsIds.xml")
        .split("<w16cid:commentId ")
        .skip(1)
        .filter_map(|e| Some((attr(e, "w16cid:paraId")?, attr(e, "w16cid:durableId")?)))
        .collect();
    let dates = text(files, "word/commentsExtensible.xml")
        .split("<w16cex:commentExtensible ")
        .skip(1)
        .filter_map(|e| Some((attr(e, "w16cex:durableId")?, attr(e, "w16cex:dateUtc").unwrap_or_default())))
        .collect();
    (ids, dates)
}

#[test]
fn comment_durable_ids_and_utc_dates_survive_and_match_the_written_paragraph_ids() {
    let doc = wordcraft_docx::read(&comments_package(true)).unwrap();
    let c: Vec<_> = doc.comments.values().collect();
    assert_eq!(c[0].durable_id.as_deref(), Some("5A1B2C3D"));
    assert_eq!(c[1].date_utc.as_deref(), Some("2026-01-03T10:00:00Z"));
    let files = save(&doc);
    let (ids, dates) = written_ids(&files);
    assert_eq!(ids.iter().map(|(_, d)| d.as_str()).collect::<Vec<_>>(), ["5A1B2C3D", "0F00BA12"]);
    // Each paragraph id is the one the comment's paragraph got in comments.xml.
    let comments = text(&files, "word/comments.xml");
    for (pid, _) in &ids {
        assert!(comments.contains(&format!(r#"w14:paraId="{pid}""#)), "{pid}: {comments}");
    }
    assert_eq!(dates, [("5A1B2C3D".into(), "2026-01-02T09:00:00Z".into()), ("0F00BA12".into(), "2026-01-03T10:00:00Z".into())]);
    let rels = text(&files, "word/_rels/document.xml.rels");
    assert!(rels.contains("relationships/commentsIds") && rels.contains("relationships/commentsExtensible"), "{rels}");
    // Read back: the same durable ids and dates.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(
        back.comments.values().map(|c| c.durable_id.clone()).collect::<Vec<_>>(),
        doc.comments.values().map(|c| c.durable_id.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        back.comments.values().map(|c| c.date_utc.clone()).collect::<Vec<_>>(),
        doc.comments.values().map(|c| c.date_utc.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn comments_without_durable_ids_get_unique_ones() {
    let doc = wordcraft_docx::read(&comments_package(false)).unwrap();
    let (ids, dates) = written_ids(&save(&doc));
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0].1, ids[1].1);
    for (_, d) in &ids {
        let v = u32::from_str_radix(d, 16).unwrap();
        assert!(v != 0 && v < 0x7FFF_FFFF, "{d}");
    }
    // Local dates without a zone aren't passed off as UTC.
    assert!(dates.iter().all(|(_, utc)| utc.is_empty()), "{dates:?}");
}

/// A local `w:date` with a `Z` (older files have no `dateUtc`) isn't saved as the UTC time either,
/// so it doesn't shift by the user's offset after a round trip.
#[test]
fn a_local_date_with_a_z_is_not_saved_as_utc() {
    let mut doc = wordcraft_docx::read(&comments_package(false)).unwrap();
    for c in doc.comments.values_mut() {
        c.date.push('Z');
    }
    let files = save(&doc);
    let (_, dates) = written_ids(&files);
    assert!(dates.iter().all(|(_, utc)| utc.is_empty()), "{dates:?}");
    let back = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert!(back.comments.values().all(|c| c.date_utc.is_none() && c.date.ends_with('Z')));
}

#[test]
fn comment_references_use_the_documents_annotation_reference_style() {
    let files = save(&wordcraft_docx::read(&comments_package(true)).unwrap());
    let body = text(&files, "word/document.xml");
    assert!(body.contains(r#"<w:rStyle w:val="Odwoaniedokomentarza"/>"#), "{body}");
    assert!(!body.contains(r#"w:val="CommentReference""#), "{body}");
    // A document without that style keeps the English id.
    let mut doc = wordcraft_doc::Document::new();
    doc.styles.styles.retain(|s| !s.name.eq_ignore_ascii_case("annotation reference"));
    let rebuilt = wordcraft_docx::read(&comments_package(true)).unwrap();
    doc.body = rebuilt.body.clone();
    doc.parts = rebuilt.parts.clone();
    doc.comments = rebuilt.comments.clone();
    assert!(text(&save(&doc), "word/document.xml").contains(r#"w:val="CommentReference""#));
}

#[test]
fn localized_default_styles_keep_their_default_flag() {
    let styles = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:latentStyles w:defLockedState="0" w:defUIPriority="99" w:defSemiHidden="0" w:defUnhideWhenUsed="0" w:defQFormat="0" w:count="376"><w:lsdException w:name="Normal" w:uiPriority="0" w:qFormat="1"/><w:lsdException w:name="heading 1" w:uiPriority="9" w:qFormat="1"/></w:latentStyles><w:style w:type="paragraph" w:default="1" w:styleId="Normalny"><w:name w:val="Normal"/></w:style><w:style w:type="character" w:default="1" w:styleId="Domylnaczcionkaakapitu"><w:name w:val="Default Paragraph Font"/></w:style><w:style w:type="table" w:default="1" w:styleId="Standardowy"><w:name w:val="Normal Table"/></w:style><w:style w:type="numbering" w:default="1" w:styleId="Bezlisty"><w:name w:val="No List"/></w:style></w:styles>"#;
    let bytes =
        package("<w:p><w:r><w:t>x</w:t></w:r></w:p>", &[("rId1", &format!("{REL}/styles"), "styles.xml")], &[], &[("word/styles.xml", styles)]);
    let doc = wordcraft_docx::read(&bytes).unwrap();
    let s = text(&save(&doc), "word/styles.xml");
    for id in ["Domylnaczcionkaakapitu", "Standardowy", "Bezlisty"] {
        assert!(s.contains(&format!(r#"w:default="1" w:styleId="{id}""#)) || s.contains(&format!(r#"w:styleId="{id}" w:default="1""#)), "{id}: {s}");
    }
    assert_eq!(s.matches(r#"w:default="1""#).count(), 4, "{s}");
    // Latent styles come back as they were, before the styles.
    assert!(s.contains(r#"<w:lsdException w:name="heading 1" w:uiPriority="9" w:qFormat="1"/>"#), "{s}");
    assert!(pos(&s, "<w:latentStyles") < pos(&s, "<w:style "), "{s}");
    // A new document still flags the English defaults.
    let fresh = text(&save(&wordcraft_doc::Document::new()), "word/styles.xml");
    assert!(fresh.contains(r#"w:styleId="Normal" w:default="1""#) || fresh.contains(r#"w:default="1" w:styleId="Normal""#), "{fresh}");
}

#[test]
fn list_identities_survive_and_a_copied_definition_gets_none() {
    let numbering = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:nsid w:val="1F2E3D4C"/><w:multiLevelType w:val="hybridMultilevel"/><w:tmpl w:val="0415000F"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#;
    let bytes = package(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#,
        &[("rId1", &format!("{REL}/numbering"), "numbering.xml")],
        &[],
        &[("word/numbering.xml", numbering)],
    );
    let mut doc = wordcraft_docx::read(&bytes).unwrap();
    let n = text(&save(&doc), "word/numbering.xml");
    assert!(n.contains(r#"<w:nsid w:val="1F2E3D4C"/><w:multiLevelType w:val="hybridMultilevel"/><w:tmpl w:val="0415000F"/>"#), "{n}");
    // A copy of the definition (as the engine makes when restarting a list) doesn't reuse the nsid.
    let mut copy = doc.numbering.abstracts[0].clone();
    copy.id = 7;
    doc.numbering.abstracts.push(copy);
    let n = text(&save(&doc), "word/numbering.xml");
    assert_eq!(n.matches("1F2E3D4C").count(), 1, "{n}");
}

#[test]
fn extended_properties_are_kept_without_words_counts_and_personal_ones_follow_the_privacy_setting() {
    let app = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Template>Normal.dotm</Template><TotalTime>42</TotalTime><Pages>107</Pages><Words>40000</Words><Characters>250000</Characters><Application>Microsoft Office Word</Application><DocSecurity>0</DocSecurity><Lines>2000</Lines><Paragraphs>1700</Paragraphs><ScaleCrop>false</ScaleCrop><Manager>M</Manager><Company>Firma</Company><LinksUpToDate>false</LinksUpToDate><CharactersWithSpaces>290000</CharactersWithSpaces><SharedDoc>false</SharedDoc><HyperlinkBase>https://example.com/</HyperlinkBase><HyperlinksChanged>false</HyperlinksChanged><AppVersion>16.0000</AppVersion></Properties>"#;
    let make = |settings: &str| {
        let mut entries = vec![("docProps/app.xml", app)];
        let mut doc_rels = vec![];
        let s;
        if !settings.is_empty() {
            s = settings.to_string();
            entries.push(("word/settings.xml", s.as_str()));
            doc_rels.push(("rId1", format!("{REL}/settings"), "settings.xml"));
        }
        let doc_rels: Vec<(&str, &str, &str)> = doc_rels.iter().map(|(a, b, c)| (*a, b.as_str(), *c)).collect();
        let b = package(
            "<w:p><w:r><w:t>x</w:t></w:r></w:p>",
            &doc_rels,
            &[("rIdA", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties", "docProps/app.xml")],
            &entries,
        );
        text(&save(&wordcraft_docx::read(&b).unwrap()), "docProps/app.xml")
    };
    let a = make("");
    for kept in [
        "<Template>Normal.dotm</Template>",
        "<Company>Firma</Company>",
        "<Manager>M</Manager>",
        "<HyperlinkBase>https://example.com/</HyperlinkBase>",
    ] {
        assert!(a.contains(kept), "{kept}: {a}");
    }
    for gone in ["Pages", "Characters", "Lines", "TotalTime", "AppVersion", "Microsoft Office Word", "40000"] {
        assert!(!a.contains(gone), "{gone}: {a}");
    }
    assert!(a.contains("<Application>WordCraft</Application>"));
    let private = make(SETTINGS);
    assert!(!private.contains("Company") && !private.contains("Manager"), "{private}");
    assert!(private.contains("<Template>Normal.dotm</Template>"));
}
