//! Tests of the full-model ODT writer, on synthetic documents built through the model API. The
//! package is read back by a small normalizer: styles of both files with their parents resolved
//! to the properties in effect, paragraphs with their text, master pages with their headers and
//! footers.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};
use std::sync::Arc;

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{CharProps, LineSpacing, NumRef, TabAlign, TabLeader, TabStop, VMerge};
use wordcraft_doc::section::SectionProps;
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{Block, Document, ListKind, Paragraph, PartKind, Revision, RevisionKind, Table, para_block};

use super::super::{Ev, events, get, local};

/// A written package, read back.
pub(crate) struct Odt {
    pub content: String,
    pub styles: String,
    pub settings: String,
    pub manifest: String,
    pub meta: String,
    /// Entry names in zip order.
    pub entries: Vec<String>,
    /// Styles of both files by (family, name): parent and own properties.
    map: HashMap<(String, String), (Option<String>, BTreeMap<String, String>)>,
}

/// A paragraph read back: its element (`p`/`h`), style and text.
#[derive(Debug, Clone)]
pub(crate) struct Para {
    pub tag: String,
    pub style: String,
    pub text: String,
    pub attrs: BTreeMap<String, String>,
}

impl Odt {
    /// `doc` written through `odt::export`, which must take this writer for it.
    pub fn new(doc: &Document) -> Odt {
        assert!(super::can_write(doc), "the document goes to the exchange-flow writer");
        Odt::read(&crate::odt::export(doc).unwrap())
    }

    pub fn read(bytes: &[u8]) -> Odt {
        conformance(bytes);
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let entries: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
        let mut part = |n: &str| {
            let mut s = String::new();
            if let Ok(mut f) = z.by_name(n) {
                f.read_to_string(&mut s).unwrap();
            }
            s
        };
        let (content, styles, settings, manifest, meta) =
            (part("content.xml"), part("styles.xml"), part("settings.xml"), part("META-INF/manifest.xml"), part("meta.xml"));
        let mut map = HashMap::new();
        for xml in [&styles, &content] {
            read_styles(xml, &mut map);
        }
        Odt { content, styles, settings, manifest, meta, entries, map }
    }

    /// The properties in effect for style `name` of `family`: the default style's, then each
    /// parent's, then its own (its attributes too, like `style:master-page-name`).
    pub fn props(&self, family: &str, name: &str) -> BTreeMap<String, String> {
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            let Some((parent, props)) = self.map.get(&(family.to_string(), n.clone())) else { break };
            if chain.len() > 32 {
                break;
            }
            chain.push(props);
            cur = parent.clone();
        }
        let mut out = self.map.get(&(family.to_string(), String::new())).map(|(_, p)| p.clone()).unwrap_or_default();
        for p in chain.into_iter().rev() {
            out.extend(p.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        out
    }

    /// Paragraphs of the body.
    pub fn paras(&self) -> Vec<Para> {
        paras(&self.content)
    }

    /// The body paragraph whose text is `text`.
    pub fn para(&self, text: &str) -> Para {
        self.paras().into_iter().find(|p| p.text == text).unwrap_or_else(|| panic!("no paragraph {text:?} in {:?}", self.paras()))
    }

    /// The properties in effect for the body paragraph whose text is `text`.
    pub fn para_props(&self, text: &str) -> BTreeMap<String, String> {
        self.props("paragraph", &self.para(text).style)
    }
}

/// Every package the tests write is checked as an ODF 1.3 package: `mimetype` first and stored,
/// the manifest listing exactly the files there are. With `ODF_JING` (a jing jar; Java on `PATH`
/// or in `JAVA`) and `ODF_SCHEMA_DIR` (the OASIS `OpenDocument-v1.3-schema.rng` and
/// `OpenDocument-v1.3-manifest-schema.rng`), each part is also validated against the schema as
/// an extended document: foreign elements and attributes (`loext:`…) are dropped first.
fn conformance(bytes: &[u8]) {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
    assert_eq!(names.first().map(String::as_str), Some("mimetype"));
    assert_eq!(z.by_index(0).unwrap().compression(), zip::CompressionMethod::Stored, "mimetype is stored");
    let mut read = |n: &str| {
        let mut s = String::new();
        z.by_name(n).unwrap().read_to_string(&mut s).unwrap();
        s
    };
    let manifest = read("META-INF/manifest.xml");
    let mut listed = Vec::new();
    events(manifest.as_bytes(), |e| {
        if let Ev::Start(n, a) = e
            && n == "manifest:file-entry"
            && let Some(p) = get(&a, "manifest:full-path")
        {
            listed.push(p.to_string());
        }
    });
    for n in names.iter().filter(|n| !n.ends_with('/') && *n != "mimetype" && *n != "META-INF/manifest.xml") {
        assert!(listed.contains(n), "{n} is not in the manifest");
    }
    for p in listed.iter().filter(|p| *p != "/") {
        assert!(names.iter().any(|n| n == p || (p.ends_with('/') && n.starts_with(p.as_str()))), "{p} is listed but missing");
    }
    let (Some(jing), Some(dir)) = (std::env::var_os("ODF_JING"), std::env::var_os("ODF_SCHEMA_DIR")) else {
        return;
    };
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let tmp = std::env::temp_dir().join(format!("odf-schema-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&tmp).unwrap();
    let mut parts = Vec::new();
    for n in names.iter().filter(|n| ["content.xml", "styles.xml", "meta.xml", "settings.xml"].iter().any(|p| n.rsplit('/').next() == Some(p))) {
        let Some(xml) = extended(&read(n)) else { continue };
        let path = tmp.join(n.replace('/', "__"));
        std::fs::write(&path, xml).unwrap();
        parts.push(path);
    }
    let java = std::env::var_os("JAVA").unwrap_or_else(|| "java".into());
    let dir = std::path::PathBuf::from(dir);
    let jing = |schema: &str, files: &[std::path::PathBuf]| {
        let out = std::process::Command::new(&java).arg("-jar").arg(&jing).arg("-i").arg(dir.join(schema)).args(files).output().unwrap();
        assert!(out.status.success(), "{schema}:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    };
    jing("OpenDocument-v1.3-schema.rng", &parts);
    let m = tmp.join("manifest.xml");
    std::fs::write(&m, &manifest).unwrap();
    jing("OpenDocument-v1.3-manifest-schema.rng", &[m]);
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `xml` without foreign elements and attributes (ODF 1.3 part 3, 3.17), or `None` for a
/// formula object's MathML, which the schema doesn't constrain.
fn extended(xml: &str) -> Option<String> {
    const ODF: &str = "office style text table draw fo xlink dc meta number svg chart dr3d math form script config presentation anim smil db xforms xhtml grddl xml xmlns";
    let odf = |q: &str| q.split_once(':').is_some_and(|(p, _)| ODF.split(' ').any(|n| n == p));
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let (mut out, mut skip, mut root) = (String::new(), 0usize, true);
    let mut math = false;
    events(xml.as_bytes(), |e| match e {
        Ev::Start(n, a) => {
            if root {
                math = local(&n) == "math";
                root = false;
            }
            if skip > 0 || !odf(&n) {
                skip += 1;
                return;
            }
            out.push('<');
            out.push_str(&n);
            for (k, v) in a.iter().filter(|(k, _)| odf(k)) {
                out.push_str(&format!(" {k}=\"{}\"", esc(v)));
            }
            out.push('>');
        }
        Ev::End(n) => {
            if skip > 0 {
                skip -= 1;
            } else {
                out.push_str(&format!("</{n}>"));
            }
        }
        Ev::Text(t) if skip == 0 => out.push_str(&esc(&t)),
        Ev::Text(_) => {}
    });
    (!math).then_some(out)
}

fn read_styles(xml: &str, map: &mut HashMap<(String, String), (Option<String>, BTreeMap<String, String>)>) {
    let mut cur: Option<((String, String), Option<String>, BTreeMap<String, String>)> = None;
    let mut sub = String::new();
    events(xml.as_bytes(), |ev| match ev {
        Ev::Start(q, a) => match q.as_str() {
            "style:style" | "style:default-style" | "style:page-layout" | "text:list-style" => {
                let family = match q.as_str() {
                    "style:page-layout" => "page-layout",
                    "text:list-style" => "list",
                    _ => get(&a, "style:family").unwrap_or(""),
                };
                let name = get(&a, "style:name").unwrap_or("").to_string();
                let own =
                    a.iter().filter(|(k, _)| !matches!(k.as_str(), "style:name" | "style:family" | "style:parent-style-name")).cloned().collect();
                cur = Some(((family.to_string(), name), get(&a, "style:parent-style-name").map(String::from), own));
            }
            "style:header-style" | "style:footer-style" => sub = format!("{}:", local(&q).trim_end_matches("-style")),
            "style:tab-stop" => {
                if let Some((_, _, p)) = cur.as_mut() {
                    let t =
                        format!("{}{}", get(&a, "style:position").unwrap_or(""), get(&a, "style:type").map(|t| format!(":{t}")).unwrap_or_default());
                    let e = p.entry("tabs".into()).or_default();
                    if !e.is_empty() {
                        e.push(' ');
                    }
                    e.push_str(&t);
                }
            }
            q if q.ends_with("-properties") || q.starts_with("text:list-level-style") || q == "style:list-level-label-alignment" => {
                if let Some((_, _, p)) = cur.as_mut() {
                    let lvl =
                        if q.starts_with("text:list-level-style") { format!("{}/", get(&a, "text:level").unwrap_or("")) } else { String::new() };
                    for (k, v) in &a {
                        p.insert(format!("{sub}{lvl}{k}"), v.clone());
                    }
                }
            }
            _ => {}
        },
        Ev::End(q) => match q.as_str() {
            "style:style" | "style:default-style" | "style:page-layout" | "text:list-style" => {
                if let Some((k, parent, p)) = cur.take() {
                    map.insert(k, (parent, p));
                }
            }
            "style:header-style" | "style:footer-style" => sub.clear(),
            _ => {}
        },
        Ev::Text(_) => {}
    });
}

/// Paragraphs of `xml` in document order (nested ones after their container's start).
pub(crate) fn paras(xml: &str) -> Vec<Para> {
    let mut out: Vec<Para> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut in_body = false;
    events(xml.as_bytes(), |ev| match ev {
        Ev::Start(q, a) => match q.as_str() {
            "office:body" => in_body = true,
            // Deleted text in change regions isn't in the body.
            "text:tracked-changes" => in_body = false,
            "text:p" | "text:h" if in_body => {
                open.push(out.len());
                out.push(Para {
                    tag: local(&q).into(),
                    style: get(&a, "text:style-name").unwrap_or("").into(),
                    text: String::new(),
                    attrs: a.into_iter().collect(),
                });
            }
            "text:s" => {
                if let Some(p) = open.last().and_then(|i| out.get_mut(*i)) {
                    let n = get(&a, "text:c").and_then(|c| c.parse().ok()).unwrap_or(1);
                    p.text.push_str(&" ".repeat(n));
                }
            }
            "text:tab" | "text:line-break" => {
                if let Some(p) = open.last().and_then(|i| out.get_mut(*i)) {
                    p.text.push(if q == "text:tab" { '\t' } else { '\n' });
                }
            }
            _ => {}
        },
        Ev::End(q) => {
            if q == "text:p" || q == "text:h" {
                open.pop();
            } else if q == "text:tracked-changes" {
                in_body = true;
            }
        }
        Ev::Text(t) => {
            if let Some(p) = open.last().and_then(|i| out.get_mut(*i)) {
                p.text.push_str(&t);
            }
        }
    });
    out
}

fn doc_of(paras: Vec<Paragraph>) -> Document {
    let mut d = Document::new();
    d.body = paras.into_iter().map(para_block).collect();
    d
}

fn plain(t: &str) -> Paragraph {
    Paragraph::with_text(t, CharProps::default())
}

// ---------------------------------------------------------------------------------------------
// Which writer

#[test]
fn documents_it_cannot_write_whole_go_to_the_flow_writer() {
    let base = doc_of(vec![plain("Text")]);
    assert!(super::can_write(&base));
    let flow = |d: &Document| crate::odt::export_flow(d).unwrap();
    // Tracked changes, or Track Changes on: the flow writer keeps them.
    let mut tracked = base.clone();
    tracked.settings.track_changes = true;
    assert!(!super::can_write(&tracked));
    assert_eq!(crate::odt::export(&tracked).unwrap().len(), flow(&tracked).len());
    let x = Odt::read(&crate::odt::export(&tracked).unwrap());
    assert!(x.content.contains("<text:tracked-changes text:track-changes=\"true\">"), "{}", x.content);
    let mut revised = base.clone();
    revised.revisions.push(Revision::new(RevisionKind::Insert, "A", ""));
    assert!(!super::can_write(&revised));
    // What later parts of #2669 write: sections, headers.
    let mut sections = base.clone();
    let mut end1 = plain("One");
    end1.section = Some(Box::new(SectionProps::default()));
    sections.body.insert(0, para_block(end1));
    assert!(!super::can_write(&sections));
    let mut header = base.clone();
    let h = header.add_part(PartKind::Header, vec![para_block(plain("Head"))]);
    header.last_section.headers.default = Some(h);
    assert!(!super::can_write(&header));
    // Named styles, lists and tables are written.
    assert!(super::can_write(&styled_doc()));
    assert!(super::can_write(&lists_doc()));
}

// ---------------------------------------------------------------------------------------------
// Package

#[test]
fn package_parts_and_settings() {
    let mut d = doc_of(vec![plain("Hello")]);
    d.core.title = "A title".into();
    d.core.creator = "Someone".into();
    let o = Odt::new(&d);
    assert_eq!(o.entries.first().map(String::as_str), Some("mimetype"));
    for p in ["content.xml", "styles.xml", "meta.xml", "settings.xml"] {
        assert!(o.manifest.contains(&format!("manifest:full-path=\"{p}\"")), "{p}: {}", o.manifest);
    }
    // The set name is a QName: it only counts with `xmlns:ooo` declared.
    assert!(o.settings.contains("xmlns:ooo=\"http://openoffice.org/2004/office\""));
    assert!(o.settings.contains("config:name=\"ooo:configuration-settings\""));
    assert!(o.settings.contains("<config:config-item config:name=\"TabsRelativeToIndent\" config:type=\"boolean\">false</config:config-item>"));
    // Justified lines shrink their spaces before breaking, as Word 2013 does; in an earlier
    // compatibility mode they don't.
    let shrink = |o: &Odt| {
        o.settings.contains("<config:config-item config:name=\"JustifyLinesWithShrinking\" config:type=\"boolean\">true</config:config-item>")
    };
    assert!(shrink(&o));
    let mut old = d.clone();
    old.settings.compat_mode = 12;
    assert!(!shrink(&Odt::new(&old)));
    assert!(o.meta.contains("<dc:title>A title</dc:title>") && o.meta.contains("<meta:initial-creator>Someone</meta:initial-creator>"));
    // Fonts used are declared in both files.
    assert!(o.styles.contains("<style:font-face style:name=\"Aptos\""), "{}", o.styles);
    assert!(o.content.contains("<style:font-face style:name=\"Aptos\""));
}

#[test]
fn the_page_is_the_sections() {
    let mut d = doc_of(vec![plain("Text")]);
    d.last_section.set_landscape(true);
    d.last_section.margin_top = 90.0;
    let o = Odt::new(&d);
    assert!(o.styles.contains("<style:master-page style:name=\"Standard\" style:page-layout-name=\"pm1\"/>"), "{}", o.styles);
    let l = o.props("page-layout", "pm1");
    assert_eq!((l.get("fo:page-width").unwrap().as_str(), l.get("style:print-orientation").unwrap().as_str()), ("792pt", "landscape"));
    assert_eq!(l.get("fo:margin-top").map(String::as_str), Some("90pt"));
}

// ---------------------------------------------------------------------------------------------
// Styles

#[test]
fn styles_whose_names_normalize_alike_go_to_the_flow_writer() {
    let mut d = doc_of(vec![plain("Text")]);
    for id in ["A B", "A_B"] {
        d.styles.upsert(Style { id: id.into(), name: id.into(), kind: StyleKind::Paragraph, ..Default::default() });
    }
    assert!(!super::can_write(&d));
    // The same id in the other family is a different ODF name space.
    let mut d = doc_of(vec![plain("Text")]);
    d.styles.upsert(Style { id: "A B".into(), name: "A".into(), kind: StyleKind::Paragraph, ..Default::default() });
    d.styles.upsert(Style { id: "A_B".into(), name: "A".into(), kind: StyleKind::Character, ..Default::default() });
    assert!(super::can_write(&d));
}

/// The parent of style `name` of `family` as written, if any.
fn parent_of(o: &Odt, family: &str, name: &str) -> Option<String> {
    o.map.get(&(family.to_string(), name.to_string())).and_then(|(p, _)| p.clone())
}

#[test]
fn a_based_on_loop_is_not_written_as_a_loop() {
    let mut d = doc_of(vec![plain("Looped").styled("LoopA")]);
    for (id, on) in [("LoopA", "LoopB"), ("LoopB", "LoopA"), ("Self", "Self")] {
        d.styles.upsert(Style { id: id.into(), name: id.into(), kind: StyleKind::Paragraph, based_on: Some(on.into()), ..Default::default() });
    }
    // The default paragraph style based on a heading that is based on it.
    if let Some(n) = d.styles.get_mut("Normal") {
        n.based_on = Some("Heading1".into());
    }
    let o = Odt::new(&d);
    assert_eq!(o.para("Looped").style, "LoopA");
    // Following style:parent-style-name from any style ends, without coming back.
    for (family, name) in o.map.keys() {
        let mut seen = vec![name.clone()];
        let mut cur = parent_of(&o, family, name);
        while let Some(p) = cur {
            assert!(!seen.contains(&p), "{family} style {name} is its own ancestor: {seen:?} then {p}");
            seen.push(p.clone());
            cur = parent_of(&o, family, &p);
        }
    }
    assert_eq!(parent_of(&o, "paragraph", "Self"), None);
    // The loop is cut at one edge; the style outside the loop keeps its parent.
    assert_eq!(parent_of(&o, "paragraph", "Heading1"), None, "Heading1's parent Normal is based on it");
    assert_eq!(parent_of(&o, "paragraph", "Heading2").as_deref(), Some("Standard"));
}

#[test]
fn style_names_are_ncnames_and_display_names_are_escaped() {
    let display = r#"Quote "A" <b> & 'c'"#;
    let mut d = doc_of(vec![plain("Odd").styled("Q\"<&>'x"), plain("Run")]);
    d.styles.upsert(Style { id: "Q\"<&>'x".into(), name: display.into(), kind: StyleKind::Paragraph, ..Default::default() });
    d.styles.upsert(Style { id: "C&\"".into(), name: display.into(), kind: StyleKind::Character, ..Default::default() });
    let mut run = plain("");
    run.insert_text(0, "Run", &CharProps { style: Some("C&\"".into()), ..Default::default() }).unwrap();
    d.body[1] = para_block(run);
    let o = Odt::new(&d);
    let p = o.para("Odd").style;
    assert_eq!(p, "Q_____x");
    // Letters of any script are kept; characters that are alphanumeric but not name characters
    // (superscript two, circled one) are not, nor a digit first.
    assert_eq!(super::styles::name("Überschrift²①"), "Überschrift__");
    assert_eq!(super::styles::name("2col"), "_2col");
    for (family, name) in [("paragraph", p.as_str()), ("text", "C__")] {
        let own = &o.map[&(family.to_string(), name.to_string())].1;
        assert_eq!(own.get("style:display-name").map(String::as_str), Some(display), "{family} {name}");
    }
    assert!(o.styles.contains("style:display-name=\"Quote &quot;A&quot; &lt;b&gt; &amp; &apos;c&apos;\""), "{}", o.styles);
}

/// A style chain: "Base" sets spacing and a font, "Derived" (based on it) only the space after,
/// a character style "Loud", tab stops with a clear.
fn styled_doc() -> Document {
    let mut d = Document::new();
    let base = Style {
        id: "Base".into(),
        name: "Base Text".into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        para: wordcraft_doc::props::ParaProps {
            space_before: Some(12.0),
            space_after: Some(4.0),
            contextual_spacing: Some(true),
            tabs: Some(vec![
                TabStop { pos: 72.0, align: TabAlign::Left, leader: TabLeader::None },
                TabStop { pos: 144.0, align: TabAlign::Right, leader: TabLeader::Dot },
            ]),
            ..Default::default()
        },
        chr: CharProps { font: Some("Serif Face".into()), size: Some(13.0), ..Default::default() },
        ..Default::default()
    };
    let derived = Style {
        id: "Derived".into(),
        name: "Derived Text".into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Base".into()),
        para: wordcraft_doc::props::ParaProps {
            space_after: Some(20.0),
            tabs: Some(vec![TabStop { pos: 72.0, align: TabAlign::Clear, leader: TabLeader::None }]),
            ..Default::default()
        },
        chr: CharProps { italic: Some(true), ..Default::default() },
        ..Default::default()
    };
    let loud = Style {
        id: "Loud".into(),
        name: "Loud".into(),
        kind: StyleKind::Character,
        chr: CharProps { bold: Some(true), ..Default::default() },
        ..Default::default()
    };
    for s in [base, derived, loud] {
        d.styles.upsert(s);
    }
    let mut p = plain("In derived ").styled("Derived");
    let n = p.len();
    p.insert_text(
        n,
        "loud red",
        &CharProps {
            style: Some("Loud".into()),
            color: Some(wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::Rgb(200, 0, 0))),
            ..Default::default()
        },
    )
    .unwrap();
    let mut direct = plain("Direct spacing").styled("Derived");
    direct.props.space_before = Some(30.0);
    direct.props.line_spacing = Some(LineSpacing::Exactly(18.0));
    d.body = vec![para_block(plain("Opening")), para_block(p), para_block(direct), para_block(plain("Plain"))];
    d
}

#[test]
fn named_styles_keep_their_chain() {
    let o = Odt::new(&styled_doc());
    let p = o.para_props("In derived loud red");
    assert_eq!(o.para("In derived loud red").style, "Derived", "no direct formatting: the named style itself");
    assert!(
        o.styles.contains("style:name=\"Derived\" style:display-name=\"Derived Text\" style:family=\"paragraph\" style:parent-style-name=\"Base\""),
        "{}",
        o.styles
    );
    assert!(
        o.styles.contains("style:name=\"Base\" style:display-name=\"Base Text\" style:family=\"paragraph\" style:parent-style-name=\"Standard\"")
    );
    assert_eq!(p.get("style:font-name").map(String::as_str), Some("Serif Face"));
    assert_eq!(p.get("fo:font-style").map(String::as_str), Some("italic"));
    assert_eq!(p.get("fo:font-size").map(String::as_str), Some("13pt"));
    // The default paragraph style is Standard, with the model's display name.
    assert!(o.styles.contains("style:name=\"Standard\" style:display-name=\"Normal\""));
    assert_eq!(o.para("Plain").style, "Standard");
}

#[test]
fn partial_spacing_is_completed_from_the_chain() {
    let o = Odt::new(&styled_doc());
    let derived = o.props("paragraph", "Derived");
    // Derived sets only the space after; the space before and contextual spacing come from Base,
    // written on Derived itself (LibreOffice would take them from the default style).
    assert!(o.styles.contains("style:name=\"Derived\""));
    let own = o.styles.split("style:name=\"Derived\"").nth(1).and_then(|s| s.split("</style:style>").next()).unwrap();
    assert!(
        own.contains("fo:margin-top=\"12pt\"") && own.contains("fo:margin-bottom=\"20pt\"") && own.contains("style:contextual-spacing=\"true\""),
        "{own}"
    );
    assert_eq!(derived.get("fo:margin-bottom").map(String::as_str), Some("20pt"));
    // Direct spacing on a paragraph is completed from its style.
    let direct = o.styles.len();
    assert!(direct > 0);
    let p = o.para("Direct spacing");
    let auto = o.content.split(&format!("style:name=\"{}\"", p.style)).nth(1).and_then(|s| s.split("</style:style>").next()).unwrap();
    assert!(
        auto.contains("fo:margin-top=\"30pt\"") && auto.contains("fo:margin-bottom=\"20pt\"") && auto.contains("style:contextual-spacing=\"true\""),
        "{auto}"
    );
    assert_eq!(o.para_props("Direct spacing").get("fo:line-height").map(String::as_str), Some("18pt"));
}

#[test]
fn tabs_are_written_merged() {
    let o = Odt::new(&styled_doc());
    // Base: 72 left and 144 right with dots; Derived clears 72.
    assert_eq!(o.props("paragraph", "Base").get("tabs").map(String::as_str), Some("72pt 144pt:right"));
    let own = o.styles.split("style:name=\"Derived\"").nth(1).and_then(|s| s.split("</style:style>").next()).unwrap();
    assert!(own.contains("<style:tab-stops><style:tab-stop style:position=\"144pt\" style:type=\"right\" style:leader-style=\"dotted\" style:leader-text=\".\"/></style:tab-stops>"), "{own}");
}

#[test]
fn runs_get_character_styles_and_direct_formatting() {
    let o = Odt::new(&styled_doc());
    let span = o.content.split("loud red").next().and_then(|s| s.rsplit("text:style-name=\"").next()).and_then(|s| s.split('"').next()).unwrap();
    let t = o.props("text", span);
    assert_eq!(t.get("fo:font-weight").map(String::as_str), Some("bold"), "from the character style");
    assert_eq!(t.get("fo:color").map(String::as_str), Some("#c80000"));
    assert!(o.content.contains(&format!("style:name=\"{span}\" style:family=\"text\" style:parent-style-name=\"Loud\"")), "{}", o.content);
}

#[test]
fn headings_are_text_h_with_their_level() {
    let o = Odt::new(&doc_of(vec![plain("A heading").styled("Heading2"), plain("Body")]));
    assert_eq!(o.para("A heading").tag, "h");
    assert_eq!(o.para("A heading").attrs.get("text:outline-level").map(String::as_str), Some("2"));
    assert_eq!(o.para("Body").tag, "p");
}

// ---------------------------------------------------------------------------------------------
// Paragraph content

#[test]
fn paragraph_content() {
    let mut p = plain("  two  spaces\tthen ");
    let n = p.len();
    p.insert_text(n, "a link", &CharProps { link: Some("https://example.com/".into()), ..Default::default() }).unwrap();
    let n = p.len();
    p.insert_object(n, InlineObject::BookmarkStart { name: "here".into() }, &CharProps::default()).unwrap();
    let n = p.len();
    p.insert_text(n, "\u{000C}after the break", &CharProps::default()).unwrap();
    let o = Odt::new(&doc_of(vec![p]));
    assert!(o.content.contains("<text:s text:c=\"2\"/>two <text:s text:c=\"1\"/>spaces<text:tab/>then "), "{}", o.content);
    assert!(o.content.contains("<text:a xlink:type=\"simple\" xlink:href=\"https://example.com/\">a link</text:a>"));
    assert!(o.content.contains("<text:bookmark-start text:name=\"here\"/>"));
    // The page break splits the paragraph; the rest starts on a new page.
    assert_eq!(o.para_props("after the break").get("fo:break-before").map(String::as_str), Some("page"));
    assert!(!o.para_props("  two  spaces\tthen a link").contains_key("fo:break-before"));
}

#[test]
fn direct_formatting_is_automatic_styles() {
    let mut p = plain("Spaced ");
    p.props.space_before = Some(30.0);
    p.props.line_spacing = Some(LineSpacing::Exactly(18.0));
    p.props.tabs = Some(vec![TabStop { pos: 144.0, align: TabAlign::Right, leader: TabLeader::Dot }]);
    let n = p.len();
    p.insert_text(
        n,
        "red",
        &CharProps { bold: Some(true), color: Some(wordcraft_doc::props::TextColor::Rgb(wordcraft_doc::Rgb(200, 0, 0))), ..Default::default() },
    )
    .unwrap();
    let o = Odt::new(&doc_of(vec![p, plain("Plain")]));
    assert_eq!(o.para("Plain").style, "Standard", "no direct formatting: the default style itself");
    let pp = o.para_props("Spaced red");
    assert_eq!(pp.get("fo:line-height").map(String::as_str), Some("18pt"));
    assert_eq!(pp.get("tabs").map(String::as_str), Some("144pt:right"));
    // The space above set alone is written with the space below and contextual spacing.
    let auto =
        o.content.split(&format!("style:name=\"{}\"", o.para("Spaced red").style)).nth(1).and_then(|s| s.split("</style:style>").next()).unwrap();
    assert!(auto.contains("fo:margin-top=\"30pt\"") && auto.contains("fo:margin-bottom=") && auto.contains("style:contextual-spacing="), "{auto}");
    let span = o.content.split(">red<").next().and_then(|s| s.rsplit("text:style-name=\"").next()).and_then(|s| s.split('"').next()).unwrap();
    let t = o.props("text", span);
    assert_eq!((t.get("fo:font-weight").map(String::as_str), t.get("fo:color").map(String::as_str)), (Some("bold"), Some("#c80000")));
}

#[test]
fn the_default_paragraph_style_is_standard() {
    let mut d = doc_of(vec![plain("Text")]);
    let normal = d.styles.styles.iter_mut().find(|s| s.id == "Normal").unwrap();
    normal.para.space_after = Some(7.0);
    normal.chr.size = Some(14.0);
    let o = Odt::new(&d);
    assert!(o.styles.contains("style:name=\"Standard\" style:display-name=\"Normal\""), "{}", o.styles);
    let p = o.para_props("Text");
    assert_eq!((p.get("fo:margin-bottom").map(String::as_str), p.get("fo:font-size").map(String::as_str)), (Some("7pt"), Some("14pt")));
}

#[test]
fn pictures_are_packaged() {
    let mut d = Document::new();
    let key = d.add_media(vec![0x89, b'P', b'N', b'G', 1, 2, 3], "png");
    let mut p = plain("Pic: ");
    let n = p.len();
    p.insert_object(
        n,
        InlineObject::Image { media: key, w: 40.0, h: 20.0, alt: "a dot".into(), float: Default::default(), crop: [0.0; 4], ole: None },
        &CharProps::default(),
    )
    .unwrap();
    d.body = vec![para_block(p)];
    let o = Odt::new(&d);
    assert!(o.entries.iter().any(|e| e == "Pictures/image1.png"), "{:?}", o.entries);
    assert!(o.manifest.contains("manifest:full-path=\"Pictures/image1.png\" manifest:media-type=\"image/png\""));
    assert!(o.content.contains("svg:width=\"40pt\" svg:height=\"20pt\""));
    assert!(o.content.contains("<svg:desc>a dot</svg:desc>"));
}

// ---------------------------------------------------------------------------------------------
// Lists and tables (as the Flow writer wrote them)

/// A numbered list: one, its nested one.a, two.
fn lists_doc() -> Document {
    let mut d = Document::new();
    let num = d.numbering.add_list(ListKind::Numbered);
    let item = |t: &str, level: u8| {
        let mut p = plain(t).styled("ListParagraph");
        p.props.numbering = Some(NumRef { num, level });
        p
    };
    d.body = vec![para_block(item("one", 0)), para_block(item("one.a", 1)), para_block(item("two", 0)), para_block(plain("after"))];
    d
}

#[test]
fn lists_nest() {
    let o = Odt::new(&lists_doc());
    let lists: Vec<&str> = o.content.split("<text:list ").skip(1).map(|s| s.split('>').next().unwrap_or("")).collect();
    assert_eq!(lists.len(), 1, "{}", o.content);
    let style = lists[0].split("text:style-name=\"").nth(1).and_then(|s| s.split('"').next()).unwrap();
    let ls = o.props("list", style);
    assert_eq!(ls.get("1/style:num-format").map(String::as_str), Some("1"));
    assert_eq!(ls.get("2/style:num-format").map(String::as_str), Some("a"));
    assert!(
        o.content.contains(">one</text:p><text:list><text:list-item><text:p text:style-name=\"ListParagraph\">one.a"),
        "nested level: {}",
        o.content
    );
    assert!(o.content.contains(">two</text:p></text:list-item></text:list>"), "{}", o.content);
}

#[test]
fn a_list_keeps_the_number_it_starts_at() {
    let mut d = lists_doc();
    // `lists_doc` adds the only list.
    d.numbering.abstracts.last_mut().unwrap().levels[0].start = 5;
    let o = Odt::new(&d);
    assert!(o.content.contains("text:level=\"1\" style:num-suffix=\".\" style:num-format=\"1\" text:start-value=\"5\""), "{}", o.content);
    assert_eq!(o.content.matches("text:start-value").count(), 1, "levels starting at 1 say nothing");
}

#[test]
fn tables_spans_and_header_rows() {
    let mut d = Document::new();
    let mut t = Table::new(3, 3, 300.0);
    t.rows[0].cells[0].blocks = vec![para_block(plain("tall"))];
    t.rows[0].cells[0].props.vmerge = VMerge::Restart;
    t.rows[1].cells[0].props.vmerge = VMerge::Continue;
    t.rows[0].cells[1].props.span = 2;
    t.rows[0].cells.pop();
    t.rows[0].cells[1].blocks = vec![para_block(plain("wide"))];
    t.rows[0].props.header = true;
    d.body = vec![Arc::new(Block::Table(t)), para_block(plain("after"))];
    let o = Odt::new(&d);
    assert!(o.content.contains("table:number-rows-spanned=\"2\""), "{}", o.content);
    assert!(o.content.contains("table:number-columns-spanned=\"2\""));
    assert_eq!(o.content.matches("<table:covered-table-cell/>").count(), 2);
    assert_eq!(o.content.matches("<table:table-row>").count(), 3);
    let header = o.content.split("<table:table-header-rows>").nth(1).and_then(|s| s.split("</table:table-header-rows>").next()).unwrap();
    assert!(header.contains(">wide<") && header.matches("<table:table-row>").count() == 1, "{header}");
    assert_eq!(o.para("tall").style, "Standard");
}

#[test]
fn only_header_rows_repeat_not_the_first_row_look() {
    // `look.header_row` is the table style's first-row formatting (`w:firstRow`); only rows marked
    // `w:tblHeader` repeat on each page, as in Word.
    let mut t = Table::new(2, 1, 100.0);
    t.props.look.header_row = true;
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(t.clone()))];
    assert!(!Odt::new(&d).content.contains("<table:table-header-rows>"));
    t.rows[0].props.header = true;
    d.body = vec![Arc::new(Block::Table(t))];
    assert!(Odt::new(&d).content.contains("<table:table-header-rows>"));
}

#[test]
fn a_table_ending_a_cell_is_followed_by_a_paragraph() {
    let mut outer = Table::new(1, 1, 200.0);
    outer.rows[0].cells[0].blocks = vec![Arc::new(Block::Table(Table::new(1, 1, 100.0)))];
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Table(outer))];
    let o = Odt::new(&d);
    assert!(o.content.contains("</table:table><text:p"), "{}", o.content);
}

#[test]
fn a_break_ending_a_paragraph_moves_the_next_block() {
    // Word draws the mark of a paragraph that ends with a page break beside the break: the next
    // paragraph (or table) starts the new page, no empty paragraph above it.
    let mut d = Document::new();
    d.body = vec![
        para_block(plain("Before\u{000C}")),
        para_block(plain("Next page")),
        para_block(plain("\u{000C}")),
        Arc::new(Block::Table(Table::new(1, 1, 100.0))),
        para_block(plain("mid\u{000C}line")),
    ];
    let o = Odt::new(&d);
    assert_eq!(o.para_props("Next page").get("fo:break-before").map(String::as_str), Some("page"));
    assert!(!o.para_props("Before").contains_key("fo:break-before"));
    assert_eq!(o.paras().iter().filter(|p| p.text == "Before" || p.text == "Next page").count(), 2, "no paragraph split off");
    let ts = o.content.split("<table:table table:name=\"Table1\" table:style-name=\"").nth(1).and_then(|s| s.split('"').next()).unwrap();
    assert_eq!(o.props("table", ts).get("fo:break-before").map(String::as_str), Some("page"));
    // A break inside a paragraph still splits it.
    assert_eq!(o.para_props("line").get("fo:break-before").map(String::as_str), Some("page"));
}

#[test]
fn widow_control_is_on_unless_turned_off() {
    // The model resolves an unset widow control as on; ODF's default is off.
    let mut d = doc_of(vec![plain("Text")]);
    d.styles.default_para.widow_control = None;
    let o = Odt::new(&d);
    let def = o.props("paragraph", "");
    assert_eq!((def.get("fo:widows").map(String::as_str), def.get("fo:orphans").map(String::as_str)), (Some("2"), Some("2")));
    d.styles.default_para.widow_control = Some(false);
    assert_eq!(Odt::new(&d).props("paragraph", "").get("fo:widows").map(String::as_str), Some("0"));
}

#[test]
fn hidden_text_is_left_out() {
    // The exchange-flow writer drops hidden runs; text the author hid must not reach the package.
    let mut d = Document::new();
    let key = d.add_media(vec![0x89, b'P', b'N', b'G', 1, 2, 3], "png");
    let mut p = plain("Shown ");
    let hidden = CharProps { hidden: Some(true), ..Default::default() };
    let n = p.len();
    p.insert_text(n, "secret", &hidden).unwrap();
    let n = p.len();
    let pic = |alt: &str| InlineObject::Image {
        media: key.clone(),
        w: 40.0,
        h: 20.0,
        alt: alt.into(),
        float: Default::default(),
        crop: [0.0; 4],
        ole: None,
    };
    p.insert_object(n, pic("hidden picture"), &hidden).unwrap();
    let n = p.len();
    p.insert_text(n, " end", &CharProps::default()).unwrap();
    let n = p.len();
    p.insert_object(n, pic("kept"), &CharProps::default()).unwrap();
    d.body = vec![para_block(p)];
    let o = Odt::new(&d);
    assert!(!o.content.contains("secret") && !o.content.contains("hidden picture"), "{}", o.content);
    assert!(!o.content.contains("text:display"), "{}", o.content);
    assert!(o.content.contains("<svg:desc>kept</svg:desc>"), "objects after a hidden one keep their place: {}", o.content);
    assert!(o.paras().iter().any(|p| p.text.starts_with("Shown  end")), "{:?}", o.paras());
}

#[test]
fn decorative_pictures_have_no_description() {
    // Screen readers skip a decorative picture: it exports with no description, found or not.
    let mut d = Document::new();
    let key = d.add_media(vec![0x89, b'P', b'N', b'G', 1, 2, 3], "png");
    let mut p = plain("Pic: ");
    for media in [key, "missing.png".into()] {
        let mut pic = InlineObject::Image { media, w: 40.0, h: 20.0, alt: "ornament".into(), float: Default::default(), crop: [0.0; 4], ole: None };
        pic.float_mut().unwrap().alt.decorative = true;
        let n = p.len();
        p.insert_object(n, pic, &CharProps::default()).unwrap();
    }
    d.body = vec![para_block(p)];
    let o = Odt::new(&d);
    assert!(o.content.contains("<draw:image"), "{}", o.content);
    assert!(!o.content.contains("ornament"), "{}", o.content);
}
