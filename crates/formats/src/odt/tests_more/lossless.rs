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
    // The picture a sample may show, and a formula object with its preview.
    for (name, data) in [
        ("META-INF/manifest.xml", MANIFEST.as_bytes()),
        ("Pictures/dot.png", DOT),
        ("Object 9/content.xml", FORMULA.as_bytes()),
        ("ObjectReplacements/Object 9", DOT),
    ] {
        z.start_file(name, o).unwrap();
        std::io::Write::write_all(&mut z, data).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// The samples' manifest (ODF 1.3 Part 2 §4): the formula object is one.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Pictures/dot.png" manifest:media-type="image/png"/><manifest:file-entry manifest:full-path="Object 9/" manifest:media-type="application/vnd.oasis.opendocument.formula"/><manifest:file-entry manifest:full-path="Object 9/content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="ObjectReplacements/Object 9" manifest:media-type="image/png"/></manifest:manifest>"#;

/// A formula object's content (MathML, ODF 1.3 §10.4.3): x².
const FORMULA: &str = r#"<?xml version="1.0" encoding="UTF-8"?><math xmlns="http://www.w3.org/1998/Math/MathML" display="block"><semantics><msup><mi>x</mi><mn>2</mn></msup><annotation encoding="StarMath 5.0">x^2</annotation></semantics></math>"#;

/// A one-pixel picture of our own.
const DOT: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4,
    0x89, 0, 0, 0, 0x0d, b'I', b'D', b'A', b'T', 0x78, 0xda, 0x63, 0xfc, 0xcf, 0xc0, 0x50, 0x0f, 0, 0x04, 0x85, 0x01, 0x80, 0x84, 0xa9, 0x8c, 0x21,
    0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
];

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
    content: r##"<office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="0.4in" wcx:spring="tight"/><style:text-properties fo:color="#336699" wcx:hue="warm"/></style:style><style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold" style:text-blinking="true"/></style:style></office:automatic-styles><office:body><office:text text:use-soft-page-breaks="true"><text:h text:outline-level="1">Orchard notes</text:h><text:p text:style-name="P1" wcx:mood="calm"><text:soft-page-break/>Pears ripen <text:span text:style-name="T1">late</text:span> in the year<text:hidden-text text:condition="ooow:1" text:string-value="quiet"/>, and <wcx:marker wcx:kind="star">apples</wcx:marker> early.</text:p><text:p>Second paragraph with a <text:note text:id="n1" text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>Footnote words.</text:p></text:note-body></text:note> note.</text:p></office:text></office:body>"##,
    styles: r##"<office:styles><style:style style:name="Standard" style:family="paragraph" style:class="text"/></office:styles>"##,
    meta: "",
};

/// Style definitions WordCraft doesn't use: an unused list style, note and line numbering
/// settings, an outline style, unknown style attributes.
const STYLES: Sample = Sample {
    name: "styles",
    content: r##"<office:body><office:text><text:p text:style-name="Standard">Only text.</text:p></office:text></office:body>"##,
    styles: r##"<office:styles><style:default-style style:family="paragraph"><style:paragraph-properties style:tab-stop-distance="0.5in" wcx:grid="on"/><style:text-properties fo:font-size="11pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text" wcx:origin="sample"><style:paragraph-properties fo:text-align="justify"/></style:style><style:style style:name="Marginal" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-style="italic"/></style:style><style:style style:name="Marginal" style:family="graphic"><style:graphic-properties svg:width="1.4in" fo:min-height="0.2in" style:horizontal-pos="left"/></style:style><text:list-style style:name="Marginal"><text:list-level-style-number text:level="1" style:num-format="i" style:num-suffix=")"/></text:list-style><text:list-style style:name="Ticks"><text:list-level-style-bullet text:level="1" text:bullet-char="✓"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:margin-left="0.25in"/></style:list-level-properties></text:list-level-style-bullet></text:list-style><text:notes-configuration text:note-class="footnote" style:num-format="a" text:start-value="0" text:footnotes-position="page" text:start-numbering-at="document"/><text:linenumbering-configuration text:number-lines="false" text:offset="0.2in" style:num-format="1" text:number-position="left" text:increment="5"/></office:styles>"##,
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
    content: r##"<office:automatic-styles><style:style style:name="Sect1" style:family="section"><style:section-properties style:editable="false" wcx:tint="none"><style:columns fo:column-count="1" fo:column-gap="0in"/></style:section-properties></style:style></office:automatic-styles><office:body><office:text><text:p>Before.</text:p><text:section text:style-name="Sect1" text:name="Terms"><text:p>Inside the terms.</text:p></text:section><text:section text:name="Locked" text:protected="true"><text:p>Inside the lock.</text:p></text:section><text:section text:style-name="Sect1" text:name="Grouped"><table:table table:name="Group"><table:table-column/><table:table-row><table:table-cell office:value-type="string"><text:p>A cell alone.</text:p></table:table-cell></table:table-row></table:table></text:section><text:section text:name="Outer"><text:section text:name="Inner"><table:table table:name="Agenda"><table:table-column/><table:table-row><table:table-cell office:value-type="string"><text:p>Item one.</text:p></table:table-cell></table:table-row></table:table><text:p>Notes under the agenda.</text:p></text:section><text:p>Closing the outer one.</text:p></text:section><text:p><draw:frame draw:name="Sender" text:anchor-type="paragraph" svg:width="3in" draw:z-index="0"><draw:text-box fo:min-height="1in"><text:section text:style-name="Sect1" text:name="Sender Address"><text:p>Ada Lovelace</text:p><text:p>12 Example Street</text:p></text:section><text:p>Below the address.</text:p></draw:text-box></draw:frame>A boxed sender.</text:p><text:p>After.</text:p></office:text></office:body>"##,
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

/// Pictures and shapes in the text: a cropped picture with its look (adjustments WordCraft
/// doesn't model), a wrapped one with a contour, a shape, a group, and a picture in a table.
const DRAWINGS: Sample = Sample {
    name: "drawings",
    content: r##"<office:automatic-styles><style:style style:name="fr1" style:family="graphic" style:parent-style-name="Graphics"><style:graphic-properties style:wrap="parallel" style:number-wrapped-paragraphs="no-limit" style:wrap-contour="true" style:wrap-contour-mode="outside" style:vertical-pos="top" style:vertical-rel="paragraph" style:horizontal-pos="center" style:horizontal-rel="paragraph" style:mirror="none" fo:clip="rect(0in, 0.1in, 0in, 0.1in)" draw:luminance="10%" draw:contrast="5%" draw:red="0%" draw:green="0%" draw:blue="0%" draw:gamma="100%" draw:color-inversion="false" draw:image-opacity="100%" draw:color-mode="standard" wcx:look="kept"/></style:style><style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:fill-color="#729fcf" style:run-through="background" style:wrap="run-through" draw:wrap-influence-on-position="once-concurrent"/></style:style></office:automatic-styles><office:body><office:text><text:p>A picture <draw:frame draw:style-name="fr1" draw:name="Dot" text:anchor-type="paragraph" svg:width="1in" svg:height="1in" draw:z-index="1"><draw:image xlink:href="Pictures/dot.png" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad" draw:mime-type="image/png"/><svg:title>A dot</svg:title><svg:desc>One pixel of our own.</svg:desc><draw:contour-polygon svg:width="1in" svg:height="1in" svg:viewBox="0 0 1000 1000" draw:points="0,0 1000,0 1000,1000 0,1000" draw:recreate-on-edit="true"/></draw:frame>beside the text.</text:p><text:p>A shape <draw:custom-shape draw:style-name="gr1" draw:name="Badge" text:anchor-type="char" svg:x="0.5in" svg:y="0in" svg:width="1in" svg:height="0.5in" draw:z-index="2"><draw:enhanced-geometry svg:viewBox="0 0 21600 21600" draw:type="hexagon" draw:modifiers="5400" draw:enhanced-path="M ?f0 0 L ?f1 0 21600 10800 ?f1 21600 ?f0 21600 0 10800 Z N"><draw:equation draw:name="f0" draw:formula="$0 "/><draw:equation draw:name="f1" draw:formula="right-$0"/><draw:handle draw:handle-position="$0 top" draw:handle-range-x-minimum="0" draw:handle-range-x-maximum="10800"/></draw:enhanced-geometry></draw:custom-shape>and a group <draw:g draw:style-name="gr1" text:anchor-type="paragraph" draw:z-index="3"><draw:rect draw:style-name="gr1" svg:x="0in" svg:y="0in" svg:width="0.5in" svg:height="0.5in"/><draw:ellipse draw:style-name="gr1" svg:x="0.5in" svg:y="0in" svg:width="0.5in" svg:height="0.5in"/></draw:g>after.</text:p><table:table table:name="Pictured"><table:table-column/><table:table-row><table:table-cell office:value-type="string"><text:p><draw:frame draw:name="Cell dot" text:anchor-type="as-char" svg:width="0.2in" svg:height="0.2in" draw:z-index="4"><draw:image xlink:href="Pictures/dot.png" xlink:type="simple"/></draw:frame> in a cell</text:p></table:table-cell></table:table-row></table:table></office:text></office:body>"##,
    styles: r##"<office:styles><style:style style:name="Graphics" style:family="graphic"><style:graphic-properties text:anchor-type="paragraph" svg:x="0in" svg:y="0in" style:wrap="dynamic" style:vertical-pos="top" style:vertical-rel="paragraph" style:horizontal-pos="center" style:horizontal-rel="paragraph"/></style:style></office:styles>"##,
    meta: "",
};

/// Comments in a paragraph: one over a range, with a reply, resolved, its text styled; one at a
/// point; with what WordCraft doesn't model (an attribute, a styled paragraph in the comment).
const COMMENTS: Sample = Sample {
    name: "comments",
    content: r##"<office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold" wcx:weight="kept"/></style:style></office:automatic-styles><office:body><office:text><text:p>Before <office:annotation office:name="Remark" loext:resolved="false" wcx:mark="kept"><dc:creator>Ada Lovelace</dc:creator><dc:date>2026-10-31T10:00:00</dc:date><meta:creator-initials>AL</meta:creator-initials><text:p>A <text:span text:style-name="T1">firm</text:span> remark.</text:p></office:annotation><office:annotation loext:parent-name="Remark" loext:resolved="true"><dc:creator>Alan Turing</dc:creator><dc:date>2026-10-31T11:00:00</dc:date><text:p>Agreed.</text:p></office:annotation>the words remarked on<office:annotation-end office:name="Remark"/> and a point<office:annotation><dc:creator>Grace Hopper</dc:creator><dc:date>2026-10-31T12:00:00</dc:date><text:p>Here.</text:p></office:annotation> after.</text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// Tracked changes: an insertion, a deletion (of text inserted before it, in part), a format
/// change, a change's own remark (`office:change-info` paragraphs), and what WordCraft doesn't
/// model on them.
const CHANGES: Sample = Sample {
    name: "changes",
    content: r##"<office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-style="italic"/></style:style></office:automatic-styles><office:body><office:text><text:tracked-changes text:track-changes="true"><text:changed-region xml:id="ct10" text:id="ct10" wcx:region="kept"><text:insertion><office:change-info><dc:creator>Ada Lovelace</dc:creator><dc:date>2026-10-30T09:00:00</dc:date><text:p>Added for clarity.</text:p></office:change-info></text:insertion></text:changed-region><text:changed-region xml:id="ct11" text:id="ct11"><text:insertion><office:change-info><dc:creator>Ada Lovelace</dc:creator><dc:date>2026-10-30T09:05:00</dc:date></office:change-info></text:insertion></text:changed-region><text:changed-region xml:id="ct12" text:id="ct12"><text:deletion><office:change-info><dc:creator>Alan Turing</dc:creator><dc:date>2026-10-30T10:00:00</dc:date></office:change-info><text:p>gone <text:change-start text:change-id="ct11"/>and new<text:change-end text:change-id="ct11"/></text:p></text:deletion></text:changed-region><text:changed-region xml:id="ct13" text:id="ct13"><text:format-change><office:change-info><dc:creator>Grace Hopper</dc:creator><dc:date>2026-10-30T11:00:00</dc:date></office:change-info></text:format-change></text:changed-region></text:tracked-changes><text:p>Some <text:change-start text:change-id="ct10"/>inserted <text:change-end text:change-id="ct10"/>words.</text:p><text:p>Some <text:change text:change-id="ct12"/>removed words.</text:p><text:p>A <text:change-start text:change-id="ct13"/><text:span text:style-name="T1">restyled</text:span><text:change-end text:change-id="ct13"/> word.</text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// A text box with styled text in a styled frame, and a shape holding text, each with what
/// WordCraft doesn't model.
const TEXT_BOXES: Sample = Sample {
    name: "text boxes",
    content: r##"<office:automatic-styles><style:style style:name="fr1" style:family="graphic" style:parent-style-name="Frame"><style:graphic-properties style:wrap="parallel" fo:padding="0.05in" fo:border="0.5pt solid #000000" style:shadow="none" draw:shadow-opacity="100%" wcx:box="kept"/></style:style><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:text-align="center" wcx:para="kept"/></style:style><style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:fill-color="#ffcc00" draw:textarea-vertical-align="middle"/></style:style></office:automatic-styles><office:body><office:text><draw:frame draw:style-name="fr1" draw:name="Sender" text:anchor-type="page" text:anchor-page-number="1" svg:x="1in" svg:y="1in" svg:width="3in" draw:z-index="2"><draw:text-box fo:min-height="0.5in"><text:p>Our address</text:p></draw:text-box></draw:frame><text:p>A box <draw:frame draw:style-name="fr1" draw:name="Aside" text:anchor-type="paragraph" svg:width="2in" draw:z-index="0"><draw:text-box fo:min-height="0.5in"><text:p text:style-name="P1">Inside the box.</text:p><text:p>A second line.</text:p></draw:text-box></draw:frame>beside the text.</text:p><text:p>A shape <draw:custom-shape draw:style-name="gr1" draw:name="Sign" text:anchor-type="paragraph" svg:width="1in" svg:height="1in" draw:z-index="1"><text:p>Stop</text:p><draw:enhanced-geometry svg:viewBox="0 0 21600 21600" draw:type="octagon" draw:modifiers="5000" draw:enhanced-path="M ?f7 0 X 0 ?f8 L 0 ?f9 Y ?f7 21600 L ?f10 21600 X 21600 ?f9 L 21600 ?f8 Y ?f10 0 Z N"/></draw:custom-shape>with words.</text:p></office:text></office:body>"##,
    styles: r##"<office:styles><style:style style:name="Frame" style:family="graphic"><style:graphic-properties text:anchor-type="paragraph" svg:x="0in" svg:y="0in" style:wrap="parallel" style:number-wrapped-paragraphs="no-limit" style:vertical-pos="top" style:vertical-rel="paragraph-content" style:horizontal-pos="center" style:horizontal-rel="paragraph-content"/></style:style></office:styles>"##,
    meta: "",
};

/// Bookmarks: at a point in a table's cell (with LibreOffice's relative column widths), and over
/// text from one paragraph into the next.
const BOOKMARKS: Sample = Sample {
    name: "bookmarks",
    content: r##"<office:automatic-styles><style:style style:name="Tab2" style:family="table"><style:table-properties style:width="6in" table:align="margins"/></style:style><style:style style:name="Tab2.A" style:family="table-column"><style:table-column-properties style:column-width="2in" style:rel-column-width="21845*"/></style:style><style:style style:name="Tab2.B" style:family="table-column"><style:table-column-properties style:column-width="4in" style:rel-column-width="43690*"/></style:style></office:automatic-styles><office:body><office:text><table:table table:name="Agenda" table:style-name="Tab2"><table:table-column table:style-name="Tab2.A"/><table:table-column table:style-name="Tab2.B"/><table:table-row><table:table-cell office:value-type="string"><text:p><text:bookmark text:name="Item"/>Item</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>Who</text:p></table:table-cell></table:table-row></table:table><text:p>From <text:bookmark-start text:name="Span"/>here</text:p><text:p>to there<text:bookmark-end text:name="Span"/>.</text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// A footnote with styled text, and a formula object with its preview picture.
const NOTES_AND_FORMULAS: Sample = Sample {
    name: "notes and formulas",
    content: r##"<office:automatic-styles><style:style style:name="T9" style:family="text"><style:text-properties fo:font-style="italic" wcx:note="kept"/></style:style><style:style style:name="fr9" style:family="graphic"><style:graphic-properties style:vertical-pos="middle" style:vertical-rel="text" draw:ole-draw-aspect="1"/></style:style></office:automatic-styles><office:body><office:text><text:p>A claim<text:note text:id="ftn0" text:note-class="footnote"><text:note-citation>1</text:note-citation><text:note-body><text:p>A <text:span text:style-name="T9">source</text:span> for it.</text:p></text:note-body></text:note> and a formula <draw:frame draw:style-name="fr9" draw:name="Formula 9" text:anchor-type="as-char" svg:width="0.3in" svg:height="0.2in" draw:z-index="0"><draw:object xlink:href="./Object 9" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/><draw:image xlink:href="./ObjectReplacements/Object 9" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame> in it.</text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// Lists as LibreOffice and OpenOffice write them: a label followed by a space, a picture
/// bullet, an unnumbered header, a nested level, and a list going on with the first by its id.
const LISTS: Sample = Sample {
    name: "lists",
    content: r##"<office:automatic-styles><text:list-style style:name="L1"><text:list-level-style-number text:level="1" style:num-prefix="(" style:num-suffix=")" style:num-format="a" text:start-value="3"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="space" fo:text-indent="-0.25in" fo:margin-left="0.5in"/></style:list-level-properties></text:list-level-style-number><text:list-level-style-number text:level="2" style:num-format="i" text:display-levels="2"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="nothing" fo:text-indent="-0.25in" fo:margin-left="0.75in"/></style:list-level-properties></text:list-level-style-number></text:list-style><text:list-style style:name="L2"><text:list-level-style-image text:level="1" xlink:href="Pictures/dot.png" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment" style:vertical-pos="middle" style:vertical-rel="line" fo:width="0.12in" fo:height="0.12in"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="0.5in" fo:text-indent="-0.25in" fo:margin-left="0.5in"/></style:list-level-properties></text:list-level-style-image></text:list-style><style:style style:name="P7" style:family="paragraph"><style:paragraph-properties wcx:item="kept"/></style:style></office:automatic-styles><office:body><office:text><text:list xml:id="list1" text:style-name="L1"><text:list-header><text:p>A heading not counted</text:p></text:list-header><text:list-item><text:p text:style-name="P7">First</text:p></text:list-item><text:list-item><text:p>Second</text:p><text:list><text:list-item><text:p>Inner</text:p></text:list-item></text:list></text:list-item></text:list><text:p>Between the lists.</text:p><text:list xml:id="list2" text:continue-list="list1" text:style-name="L1"><text:list-item><text:p>Third, going on</text:p></text:list-item></text:list><text:list xml:id="list3" text:style-name="L2"><text:list-item><text:p>Pictured</text:p></text:list-item></text:list></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// Index marks over text, a plugin in a frame, and a form's data source: what a check of the
/// schema's every element found no file holding.
const MARKS: Sample = Sample {
    name: "marks",
    content: r##"<office:body><office:text><office:forms><form:form form:name="Orders"><form:connection-resource xlink:href="sdbc:embedded:orders"/></form:form></office:forms><text:p>A <text:toc-mark-start text:id="t1" text:outline-level="2"/>marked heading<text:toc-mark-end text:id="t1"/>, a <text:user-index-mark-start text:id="u1" text:index-name="Terms"/>term<text:user-index-mark-end text:id="u1"/> and a <text:alphabetical-index-mark-start text:id="a1" text:key1="fruit"/>pear<text:alphabetical-index-mark-end text:id="a1"/>.</text:p><text:p>A plugin <draw:frame draw:name="Player" text:anchor-type="as-char" svg:width="1in" svg:height="1in"><draw:plugin xlink:href="https://example.org/clip" xlink:type="simple" draw:mime-type="application/x-example"><draw:param draw:name="loop" draw:value="true"/></draw:plugin></draw:frame> here.</text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// What ties paragraphs together: a comment over text from one paragraph into the next, an
/// insertion likewise, and a text box continuing in another's in a later paragraph.
const GROUPS: Sample = Sample {
    name: "groups",
    content: r##"<office:body><office:text><text:tracked-changes><text:changed-region xml:id="ct20" text:id="ct20"><text:insertion><office:change-info><dc:creator>Ada Lovelace</dc:creator><dc:date>2026-11-01T09:00:00</dc:date></office:change-info></text:insertion></text:changed-region></text:tracked-changes><text:p wcx:part="one">The remark <office:annotation office:name="Across" wcx:mark="kept"><dc:creator>Alan Turing</dc:creator><dc:date>2026-11-01T10:00:00</dc:date><text:p>Both paragraphs.</text:p></office:annotation>starts here</text:p><text:p wcx:part="two">and ends<office:annotation-end office:name="Across"/> here; <text:change-start text:change-id="ct20"/>an insertion</text:p><text:p wcx:part="three">going on<text:change-end text:change-id="ct20"/> to here.</text:p><text:p>A box <draw:frame draw:name="First" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:text-box draw:chain-next-name="Second"><text:p>Words flowing on.</text:p></draw:text-box></draw:frame>here.</text:p><text:p>Its next <draw:frame draw:name="Second" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:text-box/></draw:frame>there.</text:p></office:text></office:body>"##,
    styles: "",
    meta: "",
};

/// Every sample.
const SAMPLES: [Sample; 18] = [
    TEXT,
    STYLES,
    META,
    DECLS,
    TABLE,
    PAGES,
    SECTIONS,
    FORMS,
    INDEXES,
    DRAWINGS,
    COMMENTS,
    CHANGES,
    TEXT_BOXES,
    BOOKMARKS,
    NOTES_AND_FORMULAS,
    LISTS,
    MARKS,
    GROUPS,
];

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
        if let Some(st) = std::sync::Arc::make_mut(&mut styled.styles).styles.iter_mut().find(|s| s.id == "Normal") {
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

/// The declarations at the start of the text are written in the schema's order: variables,
/// sequences, then user fields.
#[test]
fn the_text_declarations_are_written_in_the_schemas_order() {
    let c = content_of(&export(&import(&package(&DECLS)).unwrap()).unwrap());
    let at = |el: &str| c.find(el).unwrap_or_else(|| panic!("no {el} in {c}"));
    assert!(at("<text:variable-decls") < at("<text:sequence-decls"), "{c}");
    assert!(at("<text:sequence-decls") < at("<text:user-field-decls"), "{c}");
}

#[test]
fn a_section_in_a_text_box_closes_in_it_whatever_is_edited() {
    // The paragraph closing the box's section edited (its wrapper gone): the section still
    // closes inside the box, before the box ends.
    let d = import(&package(&SECTIONS)).unwrap();
    for (id, part) in d.parts.iter().filter(|(_, p)| p.kind == wordcraft_doc::PartKind::TextBox) {
        for i in 0..part.blocks.len() {
            let mut e = d.clone();
            let Some(b) = e.parts.get_mut(id).and_then(|p| p.blocks.get_mut(i)) else { continue };
            let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) else { continue };
            // Restyled: no longer what was read.
            p.props.style = Some("Edited".into());
            p.touch();
            let saved = export(&e).unwrap();
            assert!(parts_well_formed(&saved).is_ok(), "{i}");
            let c = content_of(&saved);
            let in_box = c.split("<draw:text-box").nth(1).and_then(|r| r.split("</draw:text-box>").next()).unwrap_or("");
            assert_eq!(in_box.matches("<text:section ").count(), in_box.matches("</text:section>").count(), "{c}");
        }
    }
}

#[test]
fn a_picture_kept_as_read_is_written_with_its_paragraph_only() {
    // The picture a paragraph written as read shows is in the package; once that paragraph
    // and the table showing it are gone, it isn't (a deleted picture stays deleted).
    let d = import(&package(&DRAWINGS)).unwrap();
    let files = |bytes: &[u8]| {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        let names: Vec<String> = (0..z.len()).filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string())).collect();
        let dot = z.by_name("Pictures/dot.png").ok().map(|mut f| {
            let mut v = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut v).unwrap();
            v
        });
        (names, dot)
    };
    let saved = export(&d).unwrap();
    assert_eq!(files(&saved).1.as_deref(), Some(DOT));
    assert!(crate::odt::tests::entry(&saved, "META-INF/manifest.xml").contains(r#"manifest:full-path="Pictures/dot.png""#));
    let mut e = d.clone();
    e.body.retain(|b| b.as_para().is_none_or(|p| !p.text.starts_with("A picture")) && b.as_table().is_none());
    let saved = export(&e).unwrap();
    let (names, dot) = files(&saved);
    assert!(dot.is_none(), "{names:?}");
    assert!(!content_of(&saved).contains("dot.png"));
}

/// A picture that can't be read (its entry is corrupt) isn't pointed at by markup kept as read:
/// what showed it is written from the model, and the saved file links to nothing it hasn't got.
#[test]
fn markup_kept_as_read_never_links_to_a_picture_that_could_not_be_read() {
    let mut bytes = package(&DRAWINGS);
    let at = bytes.windows(DOT.len()).position(|w| w == DOT).unwrap();
    // The first copy is `Pictures/dot.png`'s: one byte changed fails its checksum.
    bytes[at + 20] ^= 0xff;
    let d = import(&bytes).unwrap();
    let saved = export(&d).unwrap();
    let c = content_of(&saved);
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(saved.clone())).unwrap();
    assert!(!c.contains("Pictures/dot.png") || z.by_name("Pictures/dot.png").is_ok(), "{c}");
    // The text round it stays.
    assert!(c.contains("A picture"), "{c}");
}

#[test]
fn a_paragraph_with_comments_is_written_as_read_while_they_are() {
    let d = import(&package(&COMMENTS)).unwrap();
    let as_read = |d: &wordcraft_doc::Document| content_of(&export(d).unwrap()).contains(r#"wcx:mark="kept""#);
    assert!(as_read(&d));
    // A comment's text edited, resolved, deleted with its reply, or answered: written from the
    // model, which has what changed.
    let first = |d: &wordcraft_doc::Document| d.comments.iter().find(|(_, c)| c.author == "Ada Lovelace").map(|(id, c)| (*id, c.clone())).unwrap();
    let mut e = d.clone();
    let (_, c) = first(&e);
    if let Some(wordcraft_doc::Block::Para(p)) = e.parts.get_mut(&c.part).and_then(|p| p.blocks.first_mut()).map(std::sync::Arc::make_mut) {
        p.props.style = Some("Edited".into());
        p.touch();
    }
    assert!(!as_read(&e));
    let mut e = d.clone();
    let (id, _) = first(&e);
    if let Some(c) = e.comments.get_mut(&id) {
        c.resolved = true;
    }
    assert!(!as_read(&e));
    assert!(content_of(&export(&e).unwrap()).contains(r#"loext:resolved="true""#));
    let mut e = d.clone();
    let (id, c) = first(&e);
    let blocks = e.parts.get(&c.part).unwrap().blocks.clone();
    let part = e.try_add_part(wordcraft_doc::PartKind::Comment, blocks).unwrap();
    e.comments.insert(999, wordcraft_doc::Comment { author: "New".into(), parent: Some(id), part, ..Default::default() });
    let c = content_of(&export(&e).unwrap());
    assert!(!c.contains(r#"wcx:mark="kept""#) && c.contains("<dc:creator>New</dc:creator>"), "{c}");
    // Every save stays well-formed and reads back with its comments.
    assert_eq!(import(&export(&e).unwrap()).unwrap().comments.len(), 4);
}

#[test]
fn a_comment_answered_in_another_paragraph_is_written_from_the_model() {
    // Its reply elsewhere in the file: the paragraph holding it isn't kept (WordCraft writes a
    // reply with what it answers), and the reply still names it.
    let content = COMMENTS.content.replace(
        r#"<office:annotation loext:parent-name="Remark" loext:resolved="true"><dc:creator>Alan Turing</dc:creator><dc:date>2026-10-31T11:00:00</dc:date><text:p>Agreed.</text:p></office:annotation>"#,
        "",
    ).replace(
        "</text:p></office:text>",
        r#"</text:p><text:p>Later <office:annotation loext:parent-name="Remark"><dc:creator>Alan Turing</dc:creator><text:p>Agreed.</text:p></office:annotation>on.</text:p></office:text>"#,
    );
    let leaked: &'static str = Box::leak(content.into_boxed_str());
    let d = import(&package(&Sample { name: "elsewhere", content: leaked, styles: "", meta: "" })).unwrap();
    let saved = export(&d).unwrap();
    let c = content_of(&saved);
    assert!(!c.contains(r#"wcx:mark="kept""#), "{c}");
    let again = import(&saved).unwrap();
    assert_eq!(again.comments.len(), 3);
    assert!(again.comments.values().any(|c| c.author == "Alan Turing" && c.parent.is_some()));
}

#[test]
fn a_change_accepted_is_written_from_the_model_the_others_as_read() {
    // The insertion accepted: its paragraph is written from the model and its region isn't
    // written; the other paragraphs keep theirs as read, and ids don't meet.
    let d = import(&package(&CHANGES)).unwrap();
    let mut e = d.clone();
    let i = e.body.iter().position(|b| b.as_para().is_some_and(|p| p.text.starts_with("Some inserted"))).unwrap();
    if let Some(wordcraft_doc::Block::Para(p)) = e.body.get_mut(i).map(std::sync::Arc::make_mut) {
        for r in &mut p.runs {
            r.props.ins = None;
        }
        p.normalize();
        p.touch();
    }
    let saved = export(&e).unwrap();
    assert!(parts_well_formed(&saved).is_ok());
    let c = content_of(&saved);
    assert!(!c.contains(r#"wcx:region="kept""#), "{c}");
    assert!(c.contains(r#"text:id="ct12""#) && c.contains(r#"text:id="ct11""#) && c.contains(r#"text:id="ct13""#), "{c}");
    let back = import(&saved).unwrap();
    assert_eq!(back.revisions.len(), d.revisions.len() - 1, "{:?}", back.revisions);
    // A change made in WordCraft next to the file's: its own id.
    let mut e = d.clone();
    e.settings.track_changes = true;
    if let Some(wordcraft_doc::Block::Para(p)) = e.body.get_mut(i).map(std::sync::Arc::make_mut) {
        p.props.style = Some("Edited".into());
        p.touch();
    }
    let c = content_of(&export(&e).unwrap());
    assert!(c.contains(r#"text:id="wcct1""#) && c.contains(r#"text:id="ct13""#), "{c}");
}

#[test]
fn a_text_box_edited_is_written_from_the_model() {
    // Its text edited: the paragraph showing the box is written from the model, with the edit.
    let d = import(&package(&TEXT_BOXES)).unwrap();
    assert!(content_of(&export(&d).unwrap()).contains(r#"wcx:box="kept""#));
    let mut e = d.clone();
    let id = e
        .parts
        .iter()
        .find(|(_, p)| {
            p.kind == wordcraft_doc::PartKind::TextBox && p.blocks.iter().any(|b| b.as_para().is_some_and(|p| p.text.starts_with("Inside")))
        })
        .map(|(id, _)| *id)
        .unwrap();
    let mut blocks = e.parts.get(&id).unwrap().blocks.clone();
    if let Some(wordcraft_doc::Block::Para(p)) = blocks.get_mut(1).map(std::sync::Arc::make_mut) {
        p.props.style = Some("Edited".into());
        p.touch();
    }
    if let Some(part) = e.parts.get_mut(&id) {
        part.blocks = blocks;
    }
    let saved = export(&e).unwrap();
    let c = content_of(&saved);
    assert!(!c.contains(r#"wcx:box="kept""#) && c.contains("Inside the box."), "{c}");
    assert!(parts_well_formed(&saved).is_ok());
}

#[test]
fn linked_text_boxes_keep_their_link() {
    // A box continuing in another (§19.112 `draw:chain-next-name`): both written from the model,
    // which names them, and the link survives.
    let content = r##"<office:body><office:text><text:p>First <draw:frame draw:name="One" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:text-box draw:chain-next-name="Two"><text:p>Text that flows on.</text:p></draw:text-box></draw:frame>box.</text:p><text:p>Second <draw:frame draw:name="Two" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:text-box/></draw:frame>box.</text:p></office:text></office:body>"##;
    let d = import(&package(&Sample { name: "linked", content, styles: "", meta: "" })).unwrap();
    let c = content_of(&export(&d).unwrap());
    let next = c.split("draw:chain-next-name=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or_default().to_string();
    assert!(!next.is_empty() && c.contains(&format!("draw:name=\"{next}\"")), "{c}");
}

#[test]
fn a_bookmark_kept_at_one_end_is_written_once() {
    // Either paragraph a bookmark spans edited: one start and one end, each once.
    let d = import(&package(&BOOKMARKS)).unwrap();
    for start in ["From", "to there"] {
        let mut e = d.clone();
        let i = e.body.iter().position(|b| b.as_para().is_some_and(|p| p.text.starts_with(start))).unwrap();
        if let Some(wordcraft_doc::Block::Para(p)) = e.body.get_mut(i).map(std::sync::Arc::make_mut) {
            p.props.style = Some("Edited".into());
            p.touch();
        }
        let c = content_of(&export(&e).unwrap());
        assert_eq!(c.matches(r#"<text:bookmark-start text:name="Span"/>"#).count(), 1, "{c}");
        assert_eq!(c.matches(r#"<text:bookmark-end text:name="Span"/>"#).count(), 1, "{c}");
        assert!(c.find("bookmark-start").unwrap_or(0) < c.find("bookmark-end").unwrap_or(0), "{c}");
    }
}

#[test]
fn a_note_or_formula_edited_is_written_from_the_model() {
    let d = import(&package(&NOTES_AND_FORMULAS)).unwrap();
    let saved = export(&d).unwrap();
    assert!(content_of(&saved).contains(r#"wcx:note="kept""#));
    let names = |b: &[u8]| zip::ZipArchive::new(std::io::Cursor::new(b.to_vec())).unwrap().file_names().map(String::from).collect::<Vec<_>>();
    assert!(names(&saved).iter().any(|n| n == "Object 9/content.xml"), "{:?}", names(&saved));
    let manifest = crate::odt::tests::entry(&saved, "META-INF/manifest.xml");
    assert!(
        manifest.contains(r#"manifest:full-path="Object 9/" manifest:media-type="application/vnd.oasis.opendocument.formula""#)
            && manifest.contains(r#"manifest:full-path="ObjectReplacements/Object 9""#),
        "{manifest}"
    );
    // The note's text edited.
    let mut e = d.clone();
    let id = e.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(id, _)| *id).unwrap();
    let mut blocks = e.parts.get(&id).unwrap().blocks.clone();
    if let Some(wordcraft_doc::Block::Para(p)) = blocks.first_mut().map(std::sync::Arc::make_mut) {
        p.props.style = Some("Edited".into());
        p.touch();
    }
    if let Some(part) = e.parts.get_mut(&id) {
        part.blocks = blocks;
    }
    let c = content_of(&export(&e).unwrap());
    assert!(!c.contains(r#"wcx:note="kept""#) && c.contains("for it."), "{c}");
    // The formula edited: written anew, and the old one's files aren't.
    let mut e = d.clone();
    for b in e.body.iter_mut() {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            for o in &mut p.objects {
                if let wordcraft_doc::para::InlineObject::Equation { linear, .. } = o {
                    *linear = "y^3".into();
                }
            }
            p.touch();
        }
    }
    let saved = export(&e).unwrap();
    assert!(!names(&saved).iter().any(|n| n.starts_with("Object 9")), "{:?}", names(&saved));
    assert!(parts_well_formed(&saved).is_ok());
}

#[test]
fn a_list_kept_as_read_is_gone_on_with_and_an_edited_one_written_anew() {
    let d = import(&package(&LISTS)).unwrap();
    let c = content_of(&export(&d).unwrap());
    // As read; the list going on with it (written from the model) names it.
    assert!(c.contains(r#"<text:list xml:id="list1" text:style-name="L1">"#) && c.contains(r#"text:label-followed-by="space""#), "{c}");
    let going_on = c.split("Between the lists.").nth(1).unwrap_or("");
    assert!(going_on.contains(r#"text:continue-list="list1""#), "{c}");
    // An item edited: the list written from the model, its numbering going on still.
    let mut e = d.clone();
    let i = e.body.iter().position(|b| b.as_para().is_some_and(|p| p.text.starts_with("Second"))).unwrap();
    if let Some(wordcraft_doc::Block::Para(p)) = e.body.get_mut(i).map(std::sync::Arc::make_mut) {
        p.props.style = Some("Edited".into());
        p.touch();
    }
    let saved = export(&e).unwrap();
    let c = content_of(&saved);
    assert!(!c.contains(r#"xml:id="list1""#) && c.contains("Second"), "{c}");
    assert!(parts_well_formed(&saved).is_ok());
    let going_on = c.split("Between the lists.").nth(1).unwrap_or("");
    let first = c.split("<text:list xml:id=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or_default();
    assert!(going_on.contains(&format!("text:continue-list=\"{first}\"")), "{c}");
}

#[test]
fn a_hidden_section_stays_hidden_however_it_is_written() {
    // §19.818 `text:display="none"`: what it holds is hidden text. Ending in a list, it isn't
    // written around its blocks; they are hidden text themselves.
    let content = r##"<office:automatic-styles><text:list-style style:name="L5"><text:list-level-style-bullet text:level="1" text:bullet-char="-"/></text:list-style></office:automatic-styles><office:body><office:text><text:p>Shown.</text:p><text:section text:name="Notes" text:display="none"><text:p>Hidden words.</text:p><text:list xml:id="l5" text:style-name="L5"><text:list-item><text:p>Hidden item.</text:p></text:list-item></text:list></text:section><text:p>Shown again.</text:p></office:text></office:body>"##;
    let d = import(&package(&Sample { name: "hidden", content, styles: "", meta: "" })).unwrap();
    let hidden = |d: &wordcraft_doc::Document, start: &str| {
        let p = d.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.starts_with(start)).unwrap();
        p.runs.iter().all(|r| r.props.hidden == Some(true))
    };
    assert!(hidden(&d, "Hidden words") && hidden(&d, "Hidden item") && !hidden(&d, "Shown again"));
    let back = import(&export(&d).unwrap()).unwrap();
    assert!(hidden(&back, "Hidden words") && hidden(&back, "Hidden item") && !hidden(&back, "Shown again"));
}

#[test]
fn a_section_ending_in_a_list_closes_before_the_list_goes_on() {
    // The list goes on after the section (the same list): written as read or from the model, the
    // section closes before it, so what follows isn't drawn into it (hidden with it).
    let content = r##"<office:automatic-styles><text:list-style style:name="L6"><text:list-level-style-number text:level="1" style:num-format="1"/></text:list-style></office:automatic-styles><office:body><office:text><text:section text:name="Hidden" text:display="none"><text:p>Hidden words.</text:p><text:list xml:id="l6" text:style-name="L6"><text:list-item><text:p>Hidden item.</text:p></text:list-item></text:list></text:section><text:list xml:id="l7" text:continue-list="l6" text:style-name="L6"><text:list-item><text:p>Shown item.</text:p></text:list-item></text:list><text:p>Shown after.</text:p></office:text></office:body>"##;
    let d = import(&package(&Sample { name: "list in section", content, styles: "", meta: "" })).unwrap();
    let check = |d: &wordcraft_doc::Document, sectioned: bool| {
        let saved = export(d).unwrap();
        assert!(parts_well_formed(&saved).is_ok());
        let c = content_of(&saved);
        if sectioned {
            let close = c.find("</text:section>").unwrap_or(usize::MAX);
            assert!(close < c.find("Shown item.").unwrap_or(0), "{c}");
            assert!(c.find("Hidden item.").unwrap_or(usize::MAX) < close, "{c}");
        }
        let back = import(&saved).unwrap();
        let hidden = |start: &str| {
            let p = back.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.starts_with(start)).unwrap();
            p.runs.iter().all(|r| r.props.hidden == Some(true))
        };
        assert!(hidden("Hidden item") && !hidden("Shown item") && !hidden("Shown after"), "{c}");
    };
    check(&d, true);
    // Everything edited: written from the model, without the section (its paragraphs aren't
    // as read), its text hidden still.
    let mut e = d.clone();
    for b in e.body.iter_mut() {
        if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b) {
            p.props.style = Some("Edited".into());
            p.touch();
        }
    }
    check(&e, false);
}

#[test]
fn a_list_without_an_id_is_kept_and_given_one() {
    // Older files' lists have no `xml:id`: kept as read, one is given, and the list going on with
    // it (by `text:continue-numbering`, read as the same list) names it when written from the model.
    let content = LISTS.content.replace(r#"<text:list xml:id="list1" text:style-name="L1">"#, r#"<text:list text:style-name="L1">"#).replace(
        r#"<text:list xml:id="list2" text:continue-list="list1" text:style-name="L1">"#,
        r#"<text:list text:continue-numbering="true" text:style-name="L1">"#,
    );
    let leaked: &'static str = Box::leak(content.into_boxed_str());
    let d = import(&package(&Sample { name: "older lists", content: leaked, styles: "", meta: "" })).unwrap();
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains(r#"<text:list xml:id="wclstk1" text:style-name="L1">"#) && c.contains(r#"text:label-followed-by="space""#), "{c}");
    let going_on = c.split("Between the lists.").nth(1).unwrap_or("");
    assert!(going_on.contains(r#"text:continue-list="wclstk1""#) || going_on.contains(r#"text:continue-numbering="true""#), "{c}");
}

#[test]
fn a_reference_mark_is_written_as_one_and_as_a_bookmark() {
    // §6.6.2: `text:reference-ref` names a reference mark; WordCraft reads it as a bookmark (its
    // references, written from the model, are `text:bookmark-ref`): both are written.
    let content = r##"<office:body><office:text><text:p>See <text:reference-mark-start text:name="intro"/>the introduction<text:reference-mark-end text:name="intro"/> and <text:reference-mark text:name="point"/>here.</text:p><text:p>As said in <text:reference-ref text:reference-format="text" text:ref-name="intro">the introduction</text:reference-ref>.</text:p></office:text></office:body>"##;
    let d = import(&package(&Sample { name: "reference marks", content, styles: "", meta: "" })).unwrap();
    // Unedited: as read.
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains(r#"<text:reference-mark-start text:name="intro"/>the introduction<text:reference-mark-end text:name="intro"/>"#), "{c}");
    // Its paragraph edited, written from the model: both.
    let mut d = d;
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.first_mut().map(std::sync::Arc::make_mut) {
        p.props.style = Some("Edited".into());
        p.touch();
    }
    let saved = export(&d).unwrap();
    assert!(parts_well_formed(&saved).is_ok());
    let c = content_of(&saved);
    assert!(c.contains(r#"<text:reference-mark-start text:name="intro"/>"#) && c.contains(r#"<text:reference-mark-end text:name="intro"/>"#), "{c}");
    assert!(c.contains(r#"<text:reference-mark text:name="point"/>"#) && c.contains(r#"text:name="intro"/>"#), "{c}");
    assert!(c.contains(r#"<text:bookmark-start text:name="intro"/>"#) || c.contains(r#"<text:bookmark text:name="intro"/>"#), "{c}");
}

#[test]
fn a_section_in_a_table_cell_is_kept_in_it() {
    // A section in a cell (StarOffice's specification template holds hidden ones): the table
    // edited, written from the model, the section around the cell's paragraphs still, closed in
    // the cell, whatever in the cell is edited.
    let content = r##"<office:body><office:text><table:table table:name="Spec"><table:table-column table:number-columns-repeated="2"/><table:table-row><table:table-cell office:value-type="string"><text:p>Status</text:p></table:table-cell><table:table-cell office:value-type="string"><text:section text:name="Status" text:display="none"><text:p>Preliminary words.</text:p><text:p>Standard words.</text:p></text:section></table:table-cell></table:table-row></table:table><text:p>After the table.</text:p></office:text></office:body>"##;
    let d = import(&package(&Sample { name: "cell section", content, styles: "", meta: "" })).unwrap();
    let edit = |d: &wordcraft_doc::Document, which: &str| {
        let mut e = d.clone();
        for b in e.body.iter_mut() {
            if let wordcraft_doc::Block::Table(t) = std::sync::Arc::make_mut(b) {
                for r in &mut t.rows {
                    for c in &mut r.cells {
                        for b in c.blocks.iter_mut() {
                            if let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b)
                                && p.text.starts_with(which)
                            {
                                p.props.style = Some("Edited".into());
                                p.touch();
                            }
                        }
                    }
                }
            }
        }
        e
    };
    for e in [edit(&d, "Status"), edit(&d, "Standard")] {
        let saved = export(&e).unwrap();
        assert!(parts_well_formed(&saved).is_ok());
        let c = content_of(&saved);
        let cell = c.split("<table:table-cell").nth(2).and_then(|r| r.split("</table:table-cell>").next()).unwrap_or("");
        assert_eq!(cell.matches("<text:section ").count(), cell.matches("</text:section>").count(), "{c}");
        assert!(!c.split("</table:table>").nth(1).unwrap_or("").contains("</text:section>"), "{c}");
        let back = import(&saved).unwrap();
        let shown = back.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.starts_with("After")).unwrap();
        assert!(shown.runs.iter().all(|r| r.props.hidden != Some(true)));
    }
    // Only the first cell edited: the section is written around the second cell's paragraphs.
    let c = content_of(&export(&edit(&d, "Status")).unwrap());
    let cell = c.split("<table:table-cell").nth(2).unwrap_or("");
    assert!(cell.contains(r#"<text:section text:name="Status" text:display="none">"#), "{c}");
}

#[test]
fn a_section_opened_by_an_empty_paragraph_is_kept_and_opened_once() {
    // LibreOffice's agenda templates: a section starting at an empty paragraph, with empty twins
    // elsewhere. Kept; and a copy of that paragraph doesn't open it again.
    let content = r##"<office:body><office:text><text:p/><text:section text:name="Topics"><text:p/><text:p>A topic.</text:p></text:section><text:p/><text:p>After.</text:p></office:text></office:body>"##;
    let d = import(&package(&Sample { name: "twins", content, styles: "", meta: "" })).unwrap();
    let c = content_of(&export(&d).unwrap());
    assert_eq!(c.matches(r#"<text:section text:name="Topics">"#).count(), 1, "{c}");
    let topic = c.find("A topic.").unwrap_or(0);
    assert!(c.find("<text:section").unwrap_or(usize::MAX) < topic && topic < c.find("</text:section>").unwrap_or(0), "{c}");
    // The empty paragraph opening it copied to the end.
    let mut e = d.clone();
    let i = e.body.iter().position(|b| b.as_para().is_some_and(|p| p.text.starts_with("A topic"))).unwrap();
    if let Some(opener) = e.body.get(i - 1).cloned() {
        e.body.push(opener);
    }
    let saved = export(&e).unwrap();
    assert!(parts_well_formed(&saved).is_ok());
    let c = content_of(&saved);
    assert_eq!(c.matches("<text:section ").count(), 1, "{c}");
    assert_eq!(c.matches("</text:section>").count(), 1, "{c}");
}

#[test]
fn a_group_edited_anywhere_is_written_from_the_model_whole() {
    // A paragraph of a group edited: every paragraph of it written from the model, which names
    // what ties them afresh, consistently.
    let d = import(&package(&GROUPS)).unwrap();
    assert!(content_of(&export(&d).unwrap()).contains(r#"wcx:part="two""#));
    for (start, kept) in [
        ("The remark", "wcx:part"),
        ("and ends", "wcx:part"),
        ("going on", "wcx:part"),
        ("A box", r#"draw:name="First""#),
        ("Its next", r#"draw:name="First""#),
    ] {
        let mut e = d.clone();
        let i = e.body.iter().position(|b| b.as_para().is_some_and(|p| p.text.starts_with(start))).unwrap();
        if let Some(wordcraft_doc::Block::Para(p)) = e.body.get_mut(i).map(std::sync::Arc::make_mut) {
            p.props.style = Some("Edited".into());
            p.touch();
        }
        let saved = export(&e).unwrap();
        assert!(parts_well_formed(&saved).is_ok(), "{start}");
        let c = content_of(&saved);
        assert!(!c.contains(kept), "{start}: {c}");
        // What ties them still does: one comment from its start to its end, one insertion, a box
        // naming the next one there is.
        let back = import(&saved).unwrap();
        assert_eq!(back.comments.len(), 1, "{start}");
        let names: Vec<&str> = c.split(r#"office:name=""#).skip(1).filter_map(|r| r.split('"').next()).collect();
        assert!(names.len() == 2 && names[0] == names[1], "{start}: {c}");
        let next = c.split(r#"draw:chain-next-name=""#).nth(1).and_then(|r| r.split('"').next()).unwrap_or("-");
        assert!(c.contains(&format!(r#"draw:name="{next}""#)), "{start}: {c}");
    }
}
