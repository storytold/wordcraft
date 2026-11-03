//! The no-edit loss gate: a document opened and saved without changes loses nothing. Each
//! sample below is saved without edits and compared, part by part, with what was read: every
//! element, every attribute, every formatting property's value and every word must still be
//! there. What may go is listed in [`ALLOWED`], each with its reason.
//!
//! The samples are written here, by us, for the purpose (see `kept`'s module docs): never a file
//! or text taken from someone else.

use std::collections::BTreeMap;

use crate::odt::xml::{Ev, events};
use crate::odt::{export, import};

/// What a save may lose, and why. Matched against the start of a loss (see [`inventory`]).
const ALLOWED: &[(&str, &str)] = &[];

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0" xmlns:wcx="urn:example:wordcraft-test" xmlns:form="urn:oasis:names:tc:opendocument:xmlns:form:1.0" office:version="1.3""#;

/// A sample package: `content.xml`, `styles.xml` and `meta.xml` bodies (inside their roots).
struct Sample {
    name: &'static str,
    content: &'static str,
    styles: &'static str,
    meta: &'static str,
}

fn package(s: &Sample) -> Vec<u8> {
    let parts = [
        ("mimetype", "application/vnd.oasis.opendocument.text".to_string()),
        ("content.xml", format!("<office:document-content {NS}>{}</office:document-content>", s.content)),
        ("styles.xml", format!("<office:document-styles {NS}>{}</office:document-styles>", s.styles)),
        ("meta.xml", format!("<office:document-meta {NS}><office:meta>{}</office:meta></office:document-meta>", s.meta)),
    ];
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in parts {
        z.start_file(name, o).unwrap();
        std::io::Write::write_all(&mut z, data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// A length's value in points to a tenth, else the value as written.
fn norm(v: &str) -> String {
    let v = v.trim();
    let units = [("cm", 72.0 / 2.54), ("mm", 72.0 / 25.4), ("in", 72.0), ("pt", 1.0), ("pc", 12.0), ("px", 0.75)];
    for (u, f) in units {
        if let Some(n) = v.strip_suffix(u).and_then(|n| n.parse::<f64>().ok()) {
            return format!("{:.1}pt", n * f);
        }
    }
    v.to_lowercase()
}

/// Attributes whose values are names or places (their presence counts, not their value).
fn nameish(a: &str) -> bool {
    ["name", "id", "ref", "href", "class", "index", "parent", "master", "list-style", "data-style"]
        .iter()
        .any(|n| a.ends_with(n) || a.contains("style-name"))
}

/// What a package holds, as counts: `el <q>`, `attr <el>@<a>`, `val <el>@<a>=<value>` (the
/// values of formatting properties), and `word <w>`.
fn inventory(bytes: &[u8]) -> BTreeMap<String, usize> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut inv = BTreeMap::new();
    let mut add = |k: String| *inv.entry(k).or_insert(0usize) += 1;
    for part in ["content.xml", "styles.xml", "meta.xml"] {
        let mut xml = Vec::new();
        let Ok(mut f) = z.by_name(part) else { continue };
        std::io::Read::read_to_end(&mut f, &mut xml).unwrap();
        let mut text = String::new();
        events(&xml, |ev| match ev {
            Ev::Start(q, a) => {
                add(format!("el {q}"));
                let props = q.ends_with("-properties");
                for (k, v) in a {
                    if k.starts_with("xmlns") {
                        continue;
                    }
                    if props && !nameish(&k) {
                        add(format!("val {q}@{k}={}", norm(&v)));
                    } else {
                        add(format!("attr {q}@{k}"));
                    }
                }
                if matches!(q.as_str(), "text:p" | "text:h" | "text:tab" | "text:s" | "text:line-break") {
                    text.push(' ');
                }
            }
            Ev::End(_) => {}
            Ev::Text(t) => text.push_str(&t),
        });
        if part == "content.xml" {
            for w in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
                add(format!("word {w}"));
            }
        }
    }
    inv
}

/// What `saved` lacks that `orig` has (none of it allowed): fewer of a word or an element, or
/// an attribute or a value it has none of.
fn losses(orig: &[u8], saved: &[u8]) -> Vec<String> {
    let (a, b) = (inventory(orig), inventory(saved));
    a.into_iter()
        // Fewer words or elements (a section of three), or an attribute or value gone.
        .filter(|(k, n)| b.get(k).copied().unwrap_or(0) < *n && (k.starts_with("word ") || k.starts_with("el ") || !b.contains_key(k)))
        .map(|(k, _)| k)
        .filter(|k| !ALLOWED.iter().any(|(p, _)| k.starts_with(p)))
        .collect()
}

fn check(s: &Sample) {
    let orig = package(s);
    let d = import(&orig).unwrap();
    let saved = export(&d).unwrap();
    let lost = losses(&orig, &saved);
    assert!(lost.is_empty(), "{}: lost on a save without edits:\n  {}", s.name, lost.join("\n  "));
    // Saved again: nothing more goes.
    let again = export(&import(&saved).unwrap()).unwrap();
    let lost = losses(&saved, &again);
    assert!(lost.is_empty(), "{}: lost on a second save:\n  {}", s.name, lost.join("\n  "));
}

/// Paragraphs and text WordCraft partly models: unknown elements, attributes and namespaces.
const TEXT: Sample = Sample {
    name: "text",
    content: r##"<office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="0.4in" wcx:spring="tight"/><style:text-properties fo:color="#336699" wcx:hue="warm"/></style:style><style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold" style:text-blinking="true"/></style:style></office:automatic-styles><office:body><office:text><text:h text:outline-level="1">Orchard notes</text:h><text:p text:style-name="P1" wcx:mood="calm">Pears ripen <text:span text:style-name="T1">late</text:span> in the year<text:hidden-text text:condition="ooow:1" text:string-value="quiet"/>, and <wcx:marker wcx:kind="star">apples</wcx:marker> early.</text:p><text:p>Second paragraph with a <text:note text:id="n1" text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>Footnote words.</text:p></text:note-body></text:note> note.</text:p></office:text></office:body>"##,
    styles: r##"<office:styles><style:style style:name="Standard" style:family="paragraph" style:class="text"/></office:styles>"##,
    meta: "",
};

/// Style definitions WordCraft doesn't use: an unused list style, note and line numbering
/// settings, an outline style, unknown style attributes.
const STYLES: Sample = Sample {
    name: "styles",
    content: r##"<office:body><office:text><text:p text:style-name="Standard">Only text.</text:p></office:text></office:body>"##,
    styles: r##"<office:styles><style:default-style style:family="paragraph"><style:paragraph-properties style:tab-stop-distance="0.5in" wcx:grid="on"/><style:text-properties fo:font-size="11pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text" wcx:origin="sample"><style:paragraph-properties fo:text-align="justify"/></style:style><style:style style:name="Marginal" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-style="italic"/></style:style><text:list-style style:name="Marginal"><text:list-level-style-number text:level="1" style:num-format="i" style:num-suffix=")"/></text:list-style><text:list-style style:name="Ticks"><text:list-level-style-bullet text:level="1" text:bullet-char="✓"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:margin-left="0.25in"/></style:list-level-properties></text:list-level-style-bullet></text:list-style><text:notes-configuration text:note-class="footnote" style:num-format="a" text:start-value="0" text:footnotes-position="page" text:start-numbering-at="document"/><text:linenumbering-configuration text:number-lines="false" text:offset="0.2in" style:num-format="1" text:number-position="left" text:increment="5"/></office:styles>"##,
    meta: "",
};

#[test]
fn a_save_without_edits_loses_nothing() {
    for s in SAMPLES {
        check(&s);
    }
}

#[test]
fn the_gate_sees_a_loss() {
    let with = Sample {
        name: "with",
        content: r##"<office:body><office:text><text:p wcx:a="1">Kept words<wcx:b/></text:p></office:text></office:body>"##,
        styles: "",
        meta: "",
    };
    let without =
        Sample { name: "without", content: r##"<office:body><office:text><text:p>Kept</text:p></office:text></office:body>"##, styles: "", meta: "" };
    let lost = losses(&package(&with), &package(&without));
    assert_eq!(lost, ["attr text:p@wcx:a", "el wcx:b", "word words"]);
}

/// Document properties WordCraft doesn't model.
const META: Sample = Sample {
    name: "meta",
    content: r##"<office:body><office:text><text:p>Body.</text:p></office:text></office:body>"##,
    styles: "",
    meta: r##"<meta:initial-creator>Sample Writer</meta:initial-creator><dc:title>Lossless sample</dc:title><dc:language>en-GB</dc:language><meta:editing-duration>PT1H2M3S</meta:editing-duration><meta:print-date>2026-01-02T03:04:05</meta:print-date><meta:printed-by>Sample Printer</meta:printed-by><meta:template xlink:type="simple" xlink:href="../plain.ott" xlink:title="Plain"/><meta:auto-reload meta:delay="PT5M"/><meta:hyperlink-behaviour office:target-frame-name="_blank"/><meta:user-defined meta:name="Reviewed" meta:value-type="boolean">true</meta:user-defined><meta:document-statistic meta:page-count="1" meta:paragraph-count="1" meta:word-count="1" meta:character-count="5"/>"##,
};

/// Declarations at the start of the text and the document's scripts.
const DECLS: Sample = Sample {
    name: "decls",
    content: r##"<office:scripts><office:event-listeners><wcx:listener wcx:on="load"/></office:event-listeners></office:scripts><office:body><office:text><text:variable-decls><text:variable-decl office:value-type="float" text:name="Count"/></text:variable-decls><text:sequence-decls><text:sequence-decl text:display-outline-level="0" text:name="Illustration"/><text:sequence-decl text:display-outline-level="1" text:separation-character="." text:name="Table"/></text:sequence-decls><text:user-field-decls><text:user-field-decl office:value-type="string" office:string-value="Ann" text:name="Reader"/></text:user-field-decls><text:p>Decls.</text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// Page layouts and master pages WordCraft partly models: the footnote separator, header
/// properties, a layout grid, a header with its own paragraph.
const PAGES: Sample = Sample {
    name: "pages",
    content: r##"<office:body><office:text><text:p>Page.</text:p></office:text></office:body>"##,
    styles: r##"<office:styles><style:style style:name="Standard" style:family="paragraph"/><style:style style:name="Header" style:family="paragraph" style:parent-style-name="Standard"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1" style:page-usage="mirrored"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" fo:margin-top="0.8in" fo:margin-bottom="0.8in" fo:margin-left="1in" fo:margin-right="1in" style:writing-mode="lr-tb" style:layout-grid-mode="none" style:footnote-max-height="0in"><style:footnote-sep style:width="0.01in" style:distance-before-sep="0.04in" style:distance-after-sep="0.04in" style:line-style="solid" style:adjustment="left" style:rel-width="25%" style:color="#000000"/></style:page-layout-properties><style:header-style><style:header-footer-properties fo:min-height="0.3in" fo:margin-bottom="0.1in" style:dynamic-spacing="false" wcx:band="1"/></style:header-style><style:footer-style/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p text:style-name="Header">Running words</text:p></style:header></style:master-page></office:master-styles>"##,
    meta: "",
};

/// A table WordCraft partly models: unknown table, row and cell attributes.
const TABLE: Sample = Sample {
    name: "table",
    content: r##"<office:automatic-styles><style:style style:name="Tab1" style:family="table"><style:table-properties style:width="4in" table:align="margins" wcx:look="grid"/></style:style><style:style style:name="Tab1.A" style:family="table-column"><style:table-column-properties style:column-width="2in"/></style:style><style:style style:name="Tab1.A1" style:family="table-cell"><style:table-cell-properties fo:padding="0.04in" fo:border="0.5pt solid #000000" wcx:shade="soft"/></style:style></office:automatic-styles><office:body><office:text><table:table table:name="Grid" table:style-name="Tab1" wcx:role="data"><table:table-column table:style-name="Tab1.A" table:number-columns-repeated="2"/><table:table-row><table:table-cell table:style-name="Tab1.A1" office:value-type="string"><text:p>One</text:p></table:table-cell><table:table-cell table:style-name="Tab1.A1" office:value-type="string"><text:p>Two</text:p></table:table-cell></table:table-row></table:table><text:p>After.</text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// Named sections, one protected.
const SECTIONS: Sample = Sample {
    name: "sections",
    content: r##"<office:automatic-styles><style:style style:name="Sect1" style:family="section"><style:section-properties style:editable="false" wcx:tint="none"><style:columns fo:column-count="1" fo:column-gap="0in"/></style:section-properties></style:style></office:automatic-styles><office:body><office:text><text:p>Before.</text:p><text:section text:style-name="Sect1" text:name="Terms"><text:p>Inside the terms.</text:p></text:section><text:section text:name="Locked" text:protected="true"><text:p>Inside the lock.</text:p></text:section><text:section text:style-name="Sect1" text:name="Grouped"><table:table table:name="Group"><table:table-column/><table:table-row><table:table-cell office:value-type="string"><text:p>A cell alone.</text:p></table:table-cell></table:table-row></table:table></text:section><text:section text:name="Outer"><text:section text:name="Inner"><table:table table:name="Agenda"><table:table-column/><table:table-row><table:table-cell office:value-type="string"><text:p>Item one.</text:p></table:table-cell></table:table-row></table:table><text:p>Notes under the agenda.</text:p></text:section><text:p>Closing the outer one.</text:p></text:section><text:p>After.</text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// A form with its own properties and event listeners, and a field placed in the text.
const FORMS: Sample = Sample {
    name: "forms",
    content: r##"<office:body><office:text><office:forms form:automatic-focus="false" form:apply-design-mode="false"><form:form form:name="Standard" form:apply-filter="true" form:command-type="table" office:target-frame="" wcx:origin="sample"><form:properties><form:property form:property-name="GroupBy" office:value-type="string" office:string-value=""/></form:properties><office:event-listeners><wcx:listener wcx:on="submit"/></office:event-listeners><form:text form:name="Reader" form:id="control1" xml:id="control1" form:control-implementation="ooo:com.sun.star.form.component.TextField" form:current-value="Ann"/></form:form></office:forms><text:p>Name: <draw:control draw:control="control1" svg:width="1.5in" svg:height="0.25in" text:anchor-type="as-char"/></text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// A table of contents and an alphabetical index with their settings and templates.
const INDEXES: Sample = Sample {
    name: "indexes",
    content: r##"<office:automatic-styles><style:style style:name="Sect9" style:family="section"><style:section-properties style:editable="false"><style:columns fo:column-count="1" fo:column-gap="0in"/></style:section-properties></style:style></office:automatic-styles><office:body><office:text><text:table-of-content text:style-name="Sect9" text:protected="true" text:name="Contents" wcx:kind="main"><text:table-of-content-source text:outline-level="2" text:use-index-marks="false"><text:index-title-template text:style-name="Contents_20_Heading">Contents</text:index-title-template><text:table-of-content-entry-template text:outline-level="1" text:style-name="Contents_20_1"><text:index-entry-chapter/><text:index-entry-text/><text:index-entry-tab-stop style:type="right" style:leader-char="."/><text:index-entry-page-number/></text:table-of-content-entry-template></text:table-of-content-source><text:index-body><text:index-title text:style-name="Sect9" text:name="Contents_Head"><text:p text:style-name="Contents_20_Heading">Contents</text:p></text:index-title><text:p text:style-name="Contents_20_1">Orchards<text:tab/>1</text:p></text:index-body></text:table-of-content><text:h text:outline-level="1">Orchards</text:h><text:p>Pears and <text:alphabetical-index-mark text:string-value="apples"/>apples.</text:p><text:alphabetical-index text:name="Words" text:protected="true"><text:alphabetical-index-source text:ignore-case="true" text:combine-entries="true" text:combine-entries-with-pp="false" fo:language="en" fo:country="GB"><text:index-title-template>Words</text:index-title-template><text:alphabetical-index-entry-template text:outline-level="1" text:style-name="Index_20_1"><text:index-entry-text/><text:index-entry-tab-stop style:type="right" style:leader-char="."/><text:index-entry-page-number/></text:alphabetical-index-entry-template></text:alphabetical-index-source><text:index-body><text:index-title text:name="Words_Head"><text:p>Words</text:p></text:index-title><text:p text:style-name="Index_20_1">apples<text:tab/>1</text:p></text:index-body></text:alphabetical-index></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// Every sample.
const SAMPLES: [Sample; 9] = [TEXT, STYLES, META, DECLS, TABLE, PAGES, SECTIONS, FORMS, INDEXES];

/// Whether `xml` is well-formed: every element closed, in order, once.
fn well_formed(xml: &[u8]) -> bool {
    let mut r = quick_xml::Reader::from_reader(xml);
    r.config_mut().check_end_names = true;
    let mut buf = Vec::new();
    let mut depth = 0i64;
    loop {
        buf.clear();
        match r.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(_)) => depth += 1,
            Ok(quick_xml::events::Event::End(_)) => depth -= 1,
            Ok(quick_xml::events::Event::Eof) => return depth == 0,
            Ok(_) => {}
            Err(_) => return false,
        }
        if depth < 0 {
            return false;
        }
    }
}

fn parts_well_formed(bytes: &[u8]) -> Result<(), String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).map_err(|e| e.to_string())?;
    for part in ["content.xml", "styles.xml", "meta.xml", "settings.xml", "META-INF/manifest.xml"] {
        let mut xml = Vec::new();
        let mut f = z.by_name(part).map_err(|e| format!("{part}: {e}"))?;
        std::io::Read::read_to_end(&mut f, &mut xml).map_err(|e| e.to_string())?;
        if !well_formed(&xml) {
            return Err(format!("{part} isn't well-formed:\n{}", String::from_utf8_lossy(&xml)));
        }
    }
    Ok(())
}

#[test]
fn edits_leave_kept_markup_well_formed() {
    // Whatever is edited (a paragraph that opens a section deleted, text typed into each
    // paragraph, a style changed), what is written stays well-formed and reads back.
    use wordcraft_doc::Pos;
    for s in SAMPLES {
        let d = import(&package(&s)).unwrap();
        let mut edits: Vec<wordcraft_doc::Document> = Vec::new();
        for i in 0..d.body.len() {
            let mut e = d.clone();
            if e.body.get(i).is_some_and(|b| b.as_para().is_some()) {
                e.insert_text(&Pos::body(i, 0), "Edited ", &Default::default()).unwrap();
            }
            edits.push(e);
            if d.body.len() > 1 {
                let mut e = d.clone();
                e.body.remove(i);
                edits.push(e);
            }
        }
        let mut styled = d.clone();
        if let Some(st) = styled.styles.styles.iter_mut().find(|s| s.id == "Normal") {
            st.chr.italic = Some(true);
        }
        edits.push(styled);
        for e in edits {
            let saved = export(&e).unwrap();
            if let Err(m) = parts_well_formed(&saved) {
                panic!("{}: {m}", s.name);
            }
            let again = export(&import(&saved).unwrap()).unwrap();
            assert!(parts_well_formed(&again).is_ok(), "{}", s.name);
        }
    }
}

#[test]
fn an_edited_index_is_written_from_the_model() {
    // Unedited, the index keeps its templates; once an entry changes, the edit is what's saved.
    use wordcraft_doc::Pos;
    let d = import(&package(&INDEXES)).unwrap();
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains("text:combine-entries-with-pp=\"false\""), "{c}");
    let i = d.body.iter().position(|b| b.as_para().is_some_and(|p| p.plain_text().starts_with("Orchards\t"))).unwrap();
    let mut e = d.clone();
    e.insert_text(&Pos::body(i, 0), "Edited ", &Default::default()).unwrap();
    let c = content_of(&export(&e).unwrap());
    assert!(c.contains("Edited Orchards"), "{c}");
    // The other index, unedited, is still as read.
    assert!(c.contains("text:combine-entries-with-pp=\"false\""), "{c}");
}

fn content_of(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name("content.xml").unwrap(), &mut s).unwrap();
    s
}
