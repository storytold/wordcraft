//! Theme colour references of WordprocessingML colours through WordCraft's own document model
//! (ECMA-376 Part 1, 5th edition: §17.3.2.6 `w:color`, §17.3.2.40 `w:u`, §17.3.4 `CT_Border`,
//! §17.3.5 `CT_Shd`): the reader's records of them cross the model bridge both ways, so a save
//! from WordCraft writes them back (storytold/wordcraft#6256). The packages are written here
//! from strings, for these tests; their stored colours differ from what the references resolve
//! to, so a save that drops a reference, or replaces a stored colour, shows.

use std::collections::BTreeMap;

use wordcraft_doc::props::{Border, TextColor};
use wordcraft_doc::{Document, Rgb};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A twelve-colour theme of our own.
fn theme() -> String {
    theme_with("336699")
}

/// [`theme`] with another first accent colour.
fn theme_with(accent1: &str) -> String {
    let slots = [
        ("dk1", "142638"),
        ("lt1", "E9F1F7"),
        ("dk2", "314457"),
        ("lt2", "DDE6EF"),
        ("accent1", accent1),
        ("accent2", "8A4B32"),
        ("accent3", "28725A"),
        ("accent4", "77558B"),
        ("accent5", "B27935"),
        ("accent6", "426E87"),
        ("hlink", "3359AD"),
        ("folHlink", "775577"),
    ];
    let colours: String = slots.iter().map(|(n, v)| format!(r#"<a:{n}><a:srgbClr val="{v}"/></a:{n}>"#)).collect();
    let fonts: String = ["majorFont", "minorFont"]
        .iter()
        .map(|n| format!(r#"<a:{n}><a:latin typeface="Liberation Sans"/><a:ea typeface=""/><a:cs typeface=""/></a:{n}>"#))
        .collect();
    let fill = r#"<a:solidFill><a:srgbClr val="E9F1F7"/></a:solidFill>"#.repeat(3);
    let line = r#"<a:ln w="12700"><a:solidFill><a:srgbClr val="142638"/></a:solidFill></a:ln>"#.repeat(3);
    let effects = "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3);
    format!(
        r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Synthetic colour reference probe"><a:themeElements><a:clrScheme name="Synthetic twelve">{colours}</a:clrScheme><a:fontScheme name="Synthetic fonts">{fonts}</a:fontScheme><a:fmtScheme name="Synthetic format"><a:fillStyleLst>{fill}</a:fillStyleLst><a:lnStyleLst>{line}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst>{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#
    )
}

const STYLES: &str = r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="SyntheticThemeStyle"><w:name w:val="Synthetic Theme Style"/><w:basedOn w:val="Normal"/><w:rPr><w:color w:val="445566" w:themeColor="accent6" w:themeTint="33"/></w:rPr></w:style>"#;

/// A run, paragraph, table, cell and page border and shading each in a theme colour, every one
/// with a tint or shade; two shadings with a theme colour on both channels; and a run in a fixed
/// colour and one in the automatic colour beside them.
const BODY: &str = r#"<w:p><w:pPr><w:pBdr><w:top w:val="single" w:sz="8" w:color="456789" w:themeColor="accent5" w:themeTint="66"/></w:pBdr><w:shd w:val="clear" w:color="102030" w:fill="203040" w:themeColor="accent4" w:themeShade="80" w:themeFill="accent3" w:themeFillTint="99"/></w:pPr><w:r><w:rPr><w:color w:val="A1B2C3" w:themeColor="accent1" w:themeShade="BF"/><w:u w:val="single" w:color="112233" w:themeColor="accent2" w:themeTint="66"/><w:bdr w:val="single" w:sz="8" w:color="234567" w:themeColor="accent3" w:themeShade="80"/><w:shd w:val="clear" w:color="auto" w:fill="345678" w:themeFill="accent2" w:themeFillTint="80"/></w:rPr><w:t>Synthetic themed run</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="SyntheticThemeStyle"/></w:pPr><w:r><w:t>Synthetic styled run</w:t></w:r></w:p><w:p><w:r><w:rPr><w:color w:val="123456"/></w:rPr><w:t>Literal control</w:t></w:r><w:r><w:rPr><w:color w:val="auto"/></w:rPr><w:t>Automatic control</w:t></w:r></w:p><w:tbl><w:tblPr><w:tblW w:w="4800" w:type="dxa"/><w:tblBorders><w:left w:val="single" w:sz="8" w:color="56789A" w:themeColor="accent6" w:themeShade="AA"/></w:tblBorders><w:shd w:val="clear" w:color="auto" w:fill="607080" w:themeFill="accent5" w:themeFillShade="CC"/></w:tblPr><w:tblGrid><w:gridCol w:w="4800"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="4800" w:type="dxa"/><w:tcBorders><w:bottom w:val="single" w:sz="8" w:color="6789AB" w:themeColor="accent1" w:themeTint="80"/></w:tcBorders><w:shd w:val="clear" w:color="304050" w:fill="405060" w:themeColor="accent5" w:themeTint="40" w:themeFill="accent4" w:themeFillShade="80"/></w:tcPr><w:p><w:r><w:t>Synthetic shaded cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/><w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:pgBorders><w:bottom w:val="single" w:sz="8" w:color="789ABC" w:themeColor="accent2" w:themeShade="99"/></w:pgBorders></w:sectPr>"#;

/// A package of `body` and `styles` with the theme above.
fn package(body: &str, styles: &str) -> Vec<u8> {
    let decl = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
    let parts = [
        (
            "_rels/.rels".to_string(),
            format!(
                r#"{decl}<Relationships xmlns="{PKG_RELS}"><Relationship Id="rMain" Type="{REL}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        ("word/document.xml".to_string(), format!(r#"{decl}<w:document xmlns:w="{W}" xmlns:r="{REL}"><w:body>{body}</w:body></w:document>"#)),
        ("word/styles.xml".to_string(), format!(r#"{decl}<w:styles xmlns:w="{W}">{styles}</w:styles>"#)),
        ("word/settings.xml".to_string(), format!(r#"{decl}<w:settings xmlns:w="{W}"/>"#)),
        (
            "word/_rels/document.xml.rels".to_string(),
            format!(
                r#"{decl}<Relationships xmlns="{PKG_RELS}"><Relationship Id="rStyles" Type="{REL}/styles" Target="styles.xml"/><Relationship Id="rSettings" Type="{REL}/settings" Target="settings.xml"/><Relationship Id="rTheme" Type="{REL}/theme" Target="theme/theme1.xml"/></Relationships>"#
            ),
        ),
        (
            "[Content_Types].xml".to_string(),
            format!(
                r#"{decl}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>"#
            ),
        ),
        ("word/theme/theme1.xml".to_string(), format!("{decl}{}", theme())),
    ];
    let mut zw = craft_zip::Writer::new();
    for (name, xml) in &parts {
        zw.add(name, xml.as_bytes(), craft_zip::Compression::Deflated).unwrap();
    }
    zw.finish().unwrap()
}

/// `name` parsed with its WordprocessingML names prefixed `w:`, whatever prefix the part uses.
fn part(pkg: &[u8], name: &str) -> craft_xml::El {
    let mut zip = craft_zip::Archive::open(pkg, craft_zip::Limits { max_entries: 10_000, max_total_bytes: 64 << 20 }).unwrap();
    let bytes = zip.read(name, 16 << 20).unwrap().unwrap_or_else(|| panic!("{name} is missing"));
    let canonical = craft_xml::Canonical::new(&[("w", W)]);
    let options = craft_xml::Options { names: craft_xml::Names::Canonical(&canonical), ..Default::default() };
    options.parse(&bytes).unwrap()
}

/// The colour attributes of `e`: its fixed colours (`w:val`, `w:color`, `w:fill`) and its theme
/// references (`w:theme…`).
fn colour_attrs(e: &craft_xml::El) -> BTreeMap<String, String> {
    ["w:val", "w:color", "w:fill", "w:themeColor", "w:themeTint", "w:themeShade", "w:themeFill", "w:themeFillTint", "w:themeFillShade"]
        .iter()
        .filter_map(|a| Some((a.to_string(), e.attr(a)?.to_string())))
        .collect()
}

fn nth<'a>(e: &'a craft_xml::El, name: &'a str, i: usize) -> &'a craft_xml::El {
    e.children(name).nth(i).unwrap_or_else(|| panic!("no {name} #{i} in {}", e.name))
}

/// Each colour owner of the fixture, found by its place in the document, with its colour
/// attributes.
fn owners(pkg: &[u8]) -> Vec<(&'static str, BTreeMap<String, String>)> {
    let doc = part(pkg, "word/document.xml");
    let body = doc.child("w:body").unwrap();
    let p0 = nth(body, "w:p", 0);
    let ppr = p0.child("w:pPr").unwrap();
    let rpr = nth(p0, "w:r", 0).child("w:rPr").unwrap();
    let controls = nth(body, "w:p", 2);
    let tblpr = body.child("w:tbl").unwrap().child("w:tblPr").unwrap();
    let tcpr = body.child("w:tbl").unwrap().child("w:tr").unwrap().child("w:tc").unwrap().child("w:tcPr").unwrap();
    let styles = part(pkg, "word/styles.xml");
    let style = styles.children("w:style").find(|s| s.attr("w:styleId") == Some("SyntheticThemeStyle")).unwrap();
    let path = |e: &craft_xml::El, names: &[&str]| e.path(names).map(colour_attrs).unwrap_or_default();
    vec![
        ("run colour", path(rpr, &["w:color"])),
        ("run underline", path(rpr, &["w:u"])),
        ("run border", path(rpr, &["w:bdr"])),
        ("run shading", path(rpr, &["w:shd"])),
        ("paragraph shading", path(ppr, &["w:shd"])),
        ("paragraph top border", path(ppr, &["w:pBdr", "w:top"])),
        ("table shading", path(tblpr, &["w:shd"])),
        ("table left border", path(tblpr, &["w:tblBorders", "w:left"])),
        ("cell shading", path(tcpr, &["w:shd"])),
        ("cell bottom border", path(tcpr, &["w:tcBorders", "w:bottom"])),
        ("page bottom border", path(body, &["w:sectPr", "w:pgBorders", "w:bottom"])),
        ("style colour", path(style, &["w:rPr", "w:color"])),
        ("fixed colour", path(nth(controls, "w:r", 0), &["w:rPr", "w:color"])),
        ("automatic colour", path(nth(controls, "w:r", 1), &["w:rPr", "w:color"])),
    ]
}

/// The colours the document is shown in, at the same owners.
fn shown(d: &Document) -> Vec<(&'static str, Option<TextColor>)> {
    let paras: Vec<_> = d.body.iter().filter_map(|b| b.as_para()).collect();
    let table = d.body.iter().find_map(|b| b.as_table()).unwrap();
    let cell = &table.rows[0].cells[0];
    let run = &paras[0].runs[0].props;
    let rgb = |c: Option<Rgb>| c.map(TextColor::Rgb);
    let line = |b: Option<Border>| b.and_then(|b| b.color).map(TextColor::Rgb);
    let style = d.styles.get("SyntheticThemeStyle").unwrap();
    vec![
        ("run colour", run.color),
        ("run underline", rgb(run.underline_color)),
        ("run border", line(run.border)),
        ("run shading", rgb(run.shading)),
        ("paragraph shading", rgb(paras[0].props.shading)),
        ("paragraph top border", line(paras[0].props.borders.and_then(|b| b.top))),
        ("table shading", rgb(table.props.shading)),
        ("table left border", line(table.props.borders.and_then(|b| b.left))),
        ("cell shading", rgb(cell.props.shading)),
        ("cell bottom border", line(cell.props.borders.and_then(|b| b.bottom))),
        ("page bottom border", line(d.last_section.page_borders.and_then(|b| b.bottom))),
        ("style colour", style.chr.color),
        ("fixed colour", paras[2].runs[0].props.color),
        ("automatic colour", paras[2].runs[1].props.color),
    ]
}

/// Opened and saved in WordCraft twice: every theme reference, its tint or shade and the stored
/// colour beside it is saved as read, on its own colour, and the colours are shown as before.
#[test]
fn theme_colour_references_survive_two_saves_from_wordcraft() {
    let input = package(BODY, STYLES);
    let expected = owners(&input);
    let mut doc = wordcraft_docx::read(&input).unwrap();
    let paint = shown(&doc);
    assert_ne!(paint[0].1, Some(TextColor::Rgb(Rgb(0xA1, 0xB2, 0xC3))), "the run is shown in its theme colour, not the stored one");
    for save in 1..=2 {
        let saved = wordcraft_docx::write(&doc).unwrap();
        for ((name, want), (_, have)) in expected.iter().zip(owners(&saved)) {
            assert_eq!(&have, want, "{name}, save {save}");
        }
        doc = wordcraft_docx::read(&saved).unwrap();
        assert_eq!(shown(&doc), paint, "save {save}");
    }
}

/// A new theme's colours show where the document's colours follow its theme colours, and the
/// save still writes the references and the colours stored beside them; fixed colours stay.
#[test]
fn a_new_theme_recolours_what_follows_it_and_keeps_the_references() {
    let input = package(BODY, STYLES);
    let mut doc = wordcraft_docx::read(&input).unwrap();
    let before = shown(&doc);
    doc.settings.theme_colors[4] = Rgb(0x99, 0x33, 0x66);
    doc.refresh_theme_colors();
    let after = shown(&doc);
    let accent1 = wordcraft_doc::props::ThemeRef { color: wordcraft_doc::props::ThemeColor::Accent1, tint: None, shade: Some(0xBF) };
    assert_eq!(after[0].1, accent1.resolve(&doc.settings.theme_colors, None).map(TextColor::Rgb), "the run follows accent1");
    assert_ne!(after[0].1, before[0].1);
    let unchanged: Vec<_> = before.iter().zip(&after).filter(|(b, a)| b == a).map(|(b, _)| b.0).collect();
    assert!(unchanged.contains(&"fixed colour") && unchanged.contains(&"paragraph shading"), "{unchanged:?}");
    let saved = wordcraft_docx::write(&doc).unwrap();
    for ((name, want), (_, have)) in owners(&input).iter().zip(owners(&saved)) {
        assert_eq!(&have, want, "{name}");
    }
    assert_eq!(shown(&wordcraft_docx::read(&saved).unwrap()), after);
}
