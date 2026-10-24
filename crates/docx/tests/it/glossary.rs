//! The glossary document (ECMA-376 §17.12: building blocks, Quick Parts, AutoText): kept byte
//! for byte while unchanged, read into `Document::glossary`, and written from the model once
//! edited. Hand-written fixtures (from the spec), no files produced by Word.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Arc;

use wordcraft_doc::{Block, BuildingBlock, CharProps, Document, Paragraph, para_block};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const GLOSSARY_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument";
const GLOSSARY_CT: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml";

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

fn rels(list: &[(&str, &str, &str)]) -> String {
    let mut s =
        String::from(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, ty, target) in list {
        s.push_str(&format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/>"#));
    }
    s.push_str("</Relationships>");
    s
}

fn content_types(extra: &[(&str, &str)]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
    );
    for (part, ct) in extra {
        s.push_str(&format!(r#"<Override PartName="{part}" ContentType="{ct}"/>"#));
    }
    s.push_str("</Types>");
    s
}

/// A package with body text, a main document relating to `doc_rels`, and `extra` parts.
fn package(doc_rels: &[(&str, &str, &str)], extra: &[(&str, String)]) -> Vec<u8> {
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R_NS}"><w:body><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#
    );
    let root = rels(&[("rIdMain", &format!("{REL}/officeDocument"), "word/document.xml")]);
    let mut entries: Vec<(&str, Vec<u8>)> = vec![
        ("_rels/.rels", root.into_bytes()),
        ("word/document.xml", doc.into_bytes()),
        ("word/_rels/document.xml.rels", rels(doc_rels).into_bytes()),
    ];
    for (p, c) in extra {
        entries.push((p, c.as_bytes().to_vec()));
    }
    zip(&entries)
}

fn glossary_xml(doc_parts: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:glossaryDocument xmlns:w="{W}" xmlns:r="{R_NS}"><w:docParts>{doc_parts}</w:docParts></w:glossaryDocument>"#
    )
}

const SIGNATURE: &str = r#"<w:docPart><w:docPartPr><w:name w:val="Signature"/><w:style w:val="Normal"/><w:category><w:name w:val="General"/><w:gallery w:val="docParts"/></w:category><w:types><w:type w:val="normal"/></w:types><w:behaviors><w:behavior w:val="content"/></w:behaviors><w:description w:val="A closing line"/><w:guid w:val="{0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0}"/></w:docPartPr><w:docPartBody><w:p><w:r><w:t>Kind regards, Ann</w:t></w:r></w:p></w:docPartBody></w:docPart>"#;
const PRICES: &str = r#"<w:docPart><w:docPartPr><w:name w:val="Price table"/><w:category><w:name w:val="Sales"/><w:gallery w:val="autoTxt"/></w:category><w:types><w:type w:val="bbPlcHdr"/><w:type w:val="autoExp"/><w:type w:val="madeUp"/></w:types><w:behaviors><w:behavior w:val="p"/><w:behavior w:val="pg"/></w:behaviors><w:guid w:val="{11111111-2222-3333-4444-555555555555}"/></w:docPartPr><w:docPartBody><w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Item</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Price</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>Prices exclude tax.</w:t></w:r></w:p></w:docPartBody></w:docPart>"#;
const GLOSSARY_STYLES: &str = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>"#;
const GLOSSARY_WEB: &str =
    r#"<w:webSettings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:optimizeForBrowser/></w:webSettings>"#;

/// A document whose glossary holds a Quick Part and an AutoText entry (with a table), with the
/// glossary's own styles and web settings.
fn with_glossary() -> Vec<u8> {
    package(
        &[("rId7", GLOSSARY_REL, "glossary/document.xml")],
        &[
            (
                "[Content_Types].xml",
                content_types(&[
                    ("/word/glossary/document.xml", GLOSSARY_CT),
                    ("/word/glossary/styles.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"),
                    ("/word/glossary/webSettings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml"),
                ]),
            ),
            ("word/glossary/document.xml", glossary_xml(&format!("{SIGNATURE}{PRICES}"))),
            (
                "word/glossary/_rels/document.xml.rels",
                rels(&[("rId1", &format!("{REL}/styles"), "styles.xml"), ("rId2", &format!("{REL}/webSettings"), "webSettings.xml")]),
            ),
            ("word/glossary/styles.xml", GLOSSARY_STYLES.to_string()),
            ("word/glossary/webSettings.xml", GLOSSARY_WEB.to_string()),
        ],
    )
}

fn save(doc: &Document) -> BTreeMap<String, Vec<u8>> {
    unzip(&wordcraft_docx::write(doc).expect("write"))
}

fn block_text(b: &Block) -> String {
    match b {
        Block::Para(p) => p.text.clone(),
        Block::Table(t) => {
            t.rows.iter().flat_map(|r| r.cells.iter()).flat_map(|c| c.blocks.iter()).map(|b| block_text(b)).collect::<Vec<_>>().join("|")
        }
    }
}

fn entry_text(e: &BuildingBlock) -> String {
    e.blocks.iter().map(|b| block_text(b)).collect::<Vec<_>>().join("\n")
}

/// The glossary part a saved package's main document relates to.
fn glossary_target(files: &BTreeMap<String, Vec<u8>>) -> Option<String> {
    let doc_rels = text(files, "word/_rels/document.xml.rels");
    assert!(doc_rels.matches("/glossaryDocument\"").count() <= 1, "{doc_rels}");
    let at = doc_rels.find("/glossaryDocument\"")?;
    let rest = doc_rels.get(at..)?;
    let t = rest.find("Target=\"")? + "Target=\"".len();
    let end = rest.get(t..)?.find('"')?;
    Some(format!("word/{}", rest.get(t..t + end)?))
}

#[test]
fn unchanged_glossary_is_carried_byte_for_byte() {
    let bytes = with_glossary();
    let mut doc = wordcraft_docx::read(&bytes).unwrap();
    doc.body.push(para_block(Paragraph::with_text("Added after opening", CharProps::default())));
    let files = save(&doc);
    let input = unzip(&bytes);
    assert_eq!(glossary_target(&files).as_deref(), Some("word/glossary/document.xml"));
    for part in ["word/glossary/document.xml", "word/glossary/styles.xml", "word/glossary/webSettings.xml"] {
        assert_eq!(files.get(part), input.get(part), "{part} not carried verbatim");
    }
    let grels = text(&files, "word/glossary/_rels/document.xml.rels");
    assert!(grels.contains("styles.xml") && grels.contains("webSettings.xml"), "{grels}");
    let ct = text(&files, "[Content_Types].xml");
    assert!(ct.contains(&format!(r#"PartName="/word/glossary/document.xml" ContentType="{GLOSSARY_CT}""#)), "{ct}");
    assert!(ct.contains(r#"PartName="/word/glossary/styles.xml""#), "{ct}");
    let again = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(again.glossary.len(), 2);
    assert!(again.plain_text(wordcraft_doc::StoryRef::Body).contains("Added after opening"));
}

#[test]
fn glossary_entries_are_read_into_the_model() {
    let doc = wordcraft_docx::read(&with_glossary()).unwrap();
    let g = &doc.glossary;
    assert_eq!(g.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Signature", "Price table"]);
    let sig = &g[0];
    assert_eq!((sig.gallery.as_str(), sig.category.as_str()), ("docParts", "General"));
    assert_eq!(sig.description, "A closing line");
    assert_eq!(sig.guid, "{0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0}");
    assert_eq!(
        (sig.style.as_str(), sig.types.as_slice(), sig.behaviors.as_slice()),
        ("Normal", &["normal".to_string()][..], &["content".to_string()][..])
    );
    assert_eq!(entry_text(sig), "Kind regards, Ann");
    let prices = &g[1];
    assert_eq!((prices.gallery.as_str(), prices.category.as_str()), ("autoTxt", "Sales"));
    // Types and behaviours outside the spec's lists aren't kept.
    assert_eq!(prices.types, ["bbPlcHdr", "autoExp"]);
    assert_eq!(prices.behaviors, ["p", "pg"]);
    assert!(matches!(&*prices.blocks[0], Block::Table(_)), "{:?}", prices.blocks);
    assert_eq!(entry_text(prices), "Item|Price\nPrices exclude tax.");
    // The glossary's text isn't the document's.
    assert!(!doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Kind regards"));
}

#[test]
fn edited_glossary_is_written_from_the_model() {
    let mut doc = wordcraft_docx::read(&with_glossary()).unwrap();
    doc.glossary[0].name = "Sign-off".into();
    let num = Arc::make_mut(&mut doc.numbering).add_list(wordcraft_doc::ListKind::Bullet);
    let mut listed = Paragraph::with_text("Second line", CharProps::default());
    listed.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
    doc.glossary.push(BuildingBlock {
        name: "Address".into(),
        gallery: "docParts".into(),
        category: "General".into(),
        blocks: vec![para_block(Paragraph::with_text("1 Main Street", CharProps::default())), para_block(listed)],
        ..Default::default()
    });
    let files = save(&doc);
    let target = glossary_target(&files).expect("a glossary relationship");
    let g = text(&files, &target);
    assert!(g.contains("w:glossaryDocument") && g.contains("Sign-off") && g.contains("1 Main Street"), "{g}");
    assert!(!files.values().any(|b| String::from_utf8_lossy(b).contains(r#"w:val="Signature""#)), "the old glossary is gone");
    let ct = text(&files, "[Content_Types].xml");
    assert!(ct.contains(&format!(r#"PartName="/{target}" ContentType="{GLOSSARY_CT}""#)), "{ct}");
    let again = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(again.glossary.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Sign-off", "Price table", "Address"]);
    assert_eq!(entry_text(&again.glossary[1]), "Item|Price\nPrices exclude tax.");
    assert_eq!(again.glossary[1].gallery, "autoTxt");
    assert_eq!(again.glossary[0].guid, "{0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0}");
    assert_eq!(again.glossary[0].description, "A closing line");
    assert_eq!(entry_text(&again.glossary[2]), "1 Main Street\nSecond line");
    // Style, types and behaviours survive the rewrite.
    for (a, b) in again.glossary.iter().zip(&doc.glossary) {
        assert_eq!((&a.style, &a.types, &a.behaviors), (&b.style, &b.types, &b.behaviors), "{}", a.name);
    }
    assert_eq!(again.glossary[1].types, ["bbPlcHdr", "autoExp"]);
    assert_eq!(again.glossary[1].behaviors, ["p", "pg"]);
    assert_eq!(again.glossary[0].style, "Normal");
    // A list in an entry has its definitions beside the glossary (its own numbering part).
    let grels = text(&files, &glossary_rels(&target));
    assert!(grels.contains("/numbering\""), "{grels}");
    let numbering = grels.split("/numbering\" Target=\"").nth(1).and_then(|r| r.split('"').next()).expect("numbering target");
    let numbering = text(&files, &format!("word/{numbering}"));
    assert!(numbering.contains("w:abstractNum") && numbering.contains(&format!(r#"w:numId="{num}""#)), "{numbering}");
    // Saving the reopened document again keeps its (now unchanged) glossary verbatim.
    let twice = save(&again);
    assert_eq!(glossary_target(&twice).as_deref(), Some(target.as_str()));
    assert_eq!(twice.get(&target), files.get(&target));
}

/// The relationships part of a part in `word/`.
fn glossary_rels(part: &str) -> String {
    let (dir, file) = part.rsplit_once('/').unwrap();
    format!("{dir}/_rels/{file}.rels")
}

#[test]
fn an_unchanged_glossary_whose_bytes_werent_kept_is_written_from_the_model() {
    let mut doc = wordcraft_docx::read(&with_glossary()).unwrap();
    // The verbatim copy is gone (over the kept-part caps, say): the model is all there is.
    doc.passthrough.remove("word/glossary/document.xml");
    let files = save(&doc);
    let target = glossary_target(&files).expect("a glossary relationship");
    assert!(text(&files, &target).contains("Kind regards, Ann"));
    let again = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(again.glossary.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Signature", "Price table"]);
}

#[test]
fn an_external_glossary_relationship_is_passed_over() {
    let doc_rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId6" Type="{GLOSSARY_REL}" Target="https://example.com/g.xml" TargetMode="External"/><Relationship Id="rId7" Type="{GLOSSARY_REL}" Target="glossary/document.xml"/></Relationships>"#
    );
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}" xmlns:r="{R_NS}"><w:body><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#
    );
    let bytes = zip(&[
        ("[Content_Types].xml", content_types(&[("/word/glossary/document.xml", GLOSSARY_CT)]).into_bytes()),
        ("_rels/.rels", rels(&[("rIdMain", &format!("{REL}/officeDocument"), "word/document.xml")]).into_bytes()),
        ("word/document.xml", doc.into_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.into_bytes()),
        ("word/glossary/document.xml", glossary_xml(SIGNATURE).into_bytes()),
    ]);
    let doc = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(doc.glossary.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Signature"]);
    // The same glossary is the one kept: saved unchanged, it's written back as it was.
    let files = save(&doc);
    assert_eq!(files.get("word/glossary/document.xml"), unzip(&bytes).get("word/glossary/document.xml"));
}

#[test]
fn a_document_without_glossary_writes_none() {
    let files = save(&Document::new());
    assert_eq!(glossary_target(&files), None);
    assert!(!text(&files, "[Content_Types].xml").contains("glossary"));
    // Removing every entry removes the glossary.
    let mut doc = wordcraft_docx::read(&with_glossary()).unwrap();
    doc.glossary.clear();
    let files = save(&doc);
    assert_eq!(glossary_target(&files), None);
    assert!(!files.keys().any(|k| k.contains("glossary")), "{:?}", files.keys());
    assert!(!text(&files, "[Content_Types].xml").contains("glossary"));
    assert!(wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap().glossary.is_empty());
}

#[test]
fn a_new_entry_in_a_new_document_makes_a_glossary() {
    let mut doc = Document::new();
    doc.glossary.push(BuildingBlock {
        name: "Greeting".into(),
        gallery: "docParts".into(),
        category: "General".into(),
        blocks: vec![para_block(Paragraph::with_text("Hello there", CharProps::default()))],
        ..Default::default()
    });
    let again = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
    assert_eq!(again.glossary.len(), 1);
    assert_eq!(again.glossary[0].name, "Greeting");
    assert_eq!(entry_text(&again.glossary[0]), "Hello there");
    assert!(again.plain_text(wordcraft_doc::StoryRef::Body).trim().is_empty());
}

/// Hostile glossaries open without a panic; entries that can't be read are skipped.
#[test]
fn hostile_glossaries_never_panic() {
    let ct = content_types(&[("/word/glossary/document.xml", GLOSSARY_CT)]);
    let one = |g: String| {
        package(&[("rId7", GLOSSARY_REL, "glossary/document.xml")], &[("[Content_Types].xml", ct.clone()), ("word/glossary/document.xml", g)])
    };
    let read = |bytes: Vec<u8>| -> Document {
        let doc = wordcraft_docx::read(&bytes).unwrap();
        // Saving and reopening must not panic either.
        let _ = wordcraft_docx::read(&wordcraft_docx::write(&doc).unwrap()).unwrap();
        doc
    };
    // No body, an empty body, no name, no properties: skipped.
    let doc = read(one(glossary_xml(
        r#"<w:docPart><w:docPartPr><w:name w:val="NoBody"/></w:docPartPr></w:docPart><w:docPart><w:docPartPr><w:name w:val="Empty"/></w:docPartPr><w:docPartBody/></w:docPart><w:docPart><w:docPartPr/><w:docPartBody><w:p><w:r><w:t>nameless</w:t></w:r></w:p></w:docPartBody></w:docPart><w:docPart><w:docPartBody><w:p/></w:docPartBody></w:docPart>"#,
    )));
    assert!(doc.glossary.iter().all(|e| !e.name.is_empty() && !e.blocks.is_empty()), "{:?}", doc.glossary);
    assert!(doc.glossary.iter().all(|e| e.name != "NoBody"));
    // Duplicate names: both read, file order.
    let doc = read(one(glossary_xml(&format!("{SIGNATURE}{SIGNATURE}"))));
    assert_eq!(doc.glossary.len(), 2);
    // Huge counts are capped.
    let many: String = (0..12_000)
        .map(|i| format!(r#"<w:docPart><w:docPartPr><w:name w:val="e{i}"/></w:docPartPr><w:docPartBody><w:p/></w:docPartBody></w:docPart>"#))
        .collect();
    let doc = read(one(glossary_xml(&many)));
    assert!(doc.glossary.len() <= 10_000, "{}", doc.glossary.len());
    // Deep nesting (tables in tables).
    let deep = format!(
        r#"<w:docPart><w:docPartPr><w:name w:val="Deep"/></w:docPartPr><w:docPartBody>{}<w:p/>{}</w:docPartBody></w:docPart>"#,
        "<w:tbl><w:tr><w:tc>".repeat(400),
        "</w:tc></w:tr></w:tbl>".repeat(400)
    );
    let _ = read(one(glossary_xml(&deep)));
    // Not a glossary at all, or a wrong content type: nothing read.
    let doc = read(one(format!(r#"<w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>x</w:t></w:r></w:p></w:body></w:document>"#)));
    assert!(doc.glossary.is_empty());
    let wrong = package(
        &[("rId7", GLOSSARY_REL, "glossary/document.xml")],
        &[
            ("[Content_Types].xml", content_types(&[("/word/glossary/document.xml", "image/png")])),
            ("word/glossary/document.xml", glossary_xml(SIGNATURE)),
        ],
    );
    assert!(read(wrong).glossary.is_empty());
    // Broken XML.
    let _ = read(one("<w:glossaryDocument><w:docParts><w:docPart>".into()));
    // A glossary relationship pointing at the main document, or a missing part.
    for target in ["document.xml", "missing.xml", "../../../../etc/passwd"] {
        let doc = read(package(&[("rId7", GLOSSARY_REL, target)], &[("[Content_Types].xml", ct.clone())]));
        assert!(doc.glossary.is_empty(), "{target}");
        assert!(!doc.plain_text(wordcraft_doc::StoryRef::Body).contains("Kind regards"));
    }
    // A glossary relating to itself, or with a glossary of its own.
    for own in
        [rels(&[("rId1", GLOSSARY_REL, "document.xml")]), rels(&[("rId1", GLOSSARY_REL, "inner.xml"), ("rId2", GLOSSARY_REL, "../document.xml")])]
    {
        let bytes = package(
            &[("rId7", GLOSSARY_REL, "glossary/document.xml")],
            &[
                ("[Content_Types].xml", content_types(&[("/word/glossary/document.xml", GLOSSARY_CT), ("/word/glossary/inner.xml", GLOSSARY_CT)])),
                ("word/glossary/document.xml", glossary_xml(SIGNATURE)),
                ("word/glossary/_rels/document.xml.rels", own),
                ("word/glossary/inner.xml", glossary_xml(PRICES)),
            ],
        );
        let doc = read(bytes);
        assert_eq!(doc.glossary.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Signature"]);
    }
    // Two glossary relationships: the first counts.
    let two = package(
        &[("rId7", GLOSSARY_REL, "glossary/document.xml"), ("rId8", GLOSSARY_REL, "glossary/document.xml")],
        &[("[Content_Types].xml", ct.clone()), ("word/glossary/document.xml", glossary_xml(SIGNATURE))],
    );
    let doc = read(two);
    assert_eq!(doc.glossary.len(), 1);
}
