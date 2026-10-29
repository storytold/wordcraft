//! Office Theme files (`.thmx`, #1869): an OOXML package whose main part is a DrawingML theme
//! (ECMA-376 Part 1 §20.1.6), written and read back, and the theme part of a `.docx`.

use std::io::{Cursor, Read, Write};

use wordcraft_doc::props::Rgb;
use wordcraft_doc::{Document, THEME_COLORS};
use wordcraft_docx::{Theme, read_theme, write_thmx};

fn sample_theme() -> Theme {
    let mut colors = THEME_COLORS;
    for (i, c) in colors.iter_mut().enumerate() {
        *c = Rgb(i as u8 * 10, 0x40, 0xFF - i as u8);
    }
    Theme { name: "Harbour".into(), colors, major_font: "Georgia".into(), minor_font: "Verdana".into() }
}

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut b = Vec::new();
            f.read_to_end(&mut b).unwrap();
            (f.name().to_string(), b)
        })
        .collect()
}

fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (n, b) in entries {
        zw.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b.as_bytes()).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

#[test]
fn thmx_written_then_read_gives_the_same_theme() {
    let t = sample_theme();
    let bytes = write_thmx(&t).unwrap();
    assert_eq!(read_theme(&bytes).unwrap(), t);
}

#[test]
fn thmx_package_has_the_theme_as_its_main_part() {
    let parts = unzip(&write_thmx(&sample_theme()).unwrap());
    let get = |n: &str| String::from_utf8(parts.iter().find(|(p, _)| p == n).unwrap_or_else(|| panic!("no {n}")).1.clone()).unwrap();
    let ct = get("[Content_Types].xml");
    assert!(ct.contains(r#"PartName="/theme/theme/theme1.xml""#), "{ct}");
    assert!(ct.contains("application/vnd.openxmlformats-officedocument.theme+xml"), "{ct}");
    let rels = get("_rels/.rels");
    assert!(rels.contains("relationships/officeDocument"), "{rels}");
    assert!(rels.contains(r#"Target="theme/theme/theme1.xml""#), "{rels}");
    let theme = get("theme/theme/theme1.xml");
    for tag in ["<a:theme ", "<a:clrScheme", "<a:fontScheme", "<a:fmtScheme", "name=\"Harbour\"", "typeface=\"Georgia\"", "<a:folHlink>"] {
        assert!(theme.contains(tag), "{tag} missing in {theme}");
    }
}

#[test]
fn hand_built_thmx_is_read_per_spec() {
    let theme = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Meadow">
 <a:themeElements>
  <a:clrScheme name="Meadow Colours">
   <a:dk1><a:sysClr val="windowText" lastClr="111111"/></a:dk1>
   <a:lt1><a:sysClr val="window" lastClr="FEFEFE"/></a:lt1>
   <a:dk2><a:srgbClr val="222222"/></a:dk2>
   <a:lt2><a:srgbClr val="EEEEEE"/></a:lt2>
   <a:accent1><a:srgbClr val="AA0001"/></a:accent1>
   <a:accent2><a:srgbClr val="AA0002"/></a:accent2>
   <a:accent3><a:srgbClr val="AA0003"/></a:accent3>
   <a:accent4><a:srgbClr val="AA0004"/></a:accent4>
   <a:accent5><a:srgbClr val="AA0005"/></a:accent5>
   <a:accent6><a:srgbClr val="AA0006"/></a:accent6>
   <a:hlink><a:srgbClr val="0000EE"/></a:hlink>
   <a:folHlink><a:srgbClr val="551A8B"/></a:folHlink>
  </a:clrScheme>
  <a:fontScheme name="Meadow Fonts">
   <a:majorFont><a:latin typeface="Palatino"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>
   <a:minorFont><a:latin typeface="Optima"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont>
  </a:fontScheme>
  <a:fmtScheme name="Meadow"><a:fillStyleLst/><a:lnStyleLst/><a:effectStyleLst/><a:bgFillStyleLst/></a:fmtScheme>
 </a:themeElements>
</a:theme>"#;
    let bytes = zip_of(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/theme/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="theme/theme/theme1.xml"/></Relationships>"#,
        ),
        ("theme/theme/theme1.xml", theme),
    ]);
    let t = read_theme(&bytes).unwrap();
    assert_eq!(t.name, "Meadow");
    assert_eq!(t.major_font, "Palatino");
    assert_eq!(t.minor_font, "Optima");
    assert_eq!(t.colors[0], Rgb(0x11, 0x11, 0x11));
    assert_eq!(t.colors[1], Rgb(0xFE, 0xFE, 0xFE));
    assert_eq!(t.colors[4], Rgb(0xAA, 0, 1));
    assert_eq!(t.colors[9], Rgb(0xAA, 0, 6));
    assert_eq!(t.colors[11], Rgb(0x55, 0x1A, 0x8B));
}

#[test]
fn docx_theme_part_is_read_as_a_theme() {
    let mut d = Document::default();
    let t = sample_theme();
    d.settings.theme_name = t.name.clone();
    d.settings.theme_colors = t.colors.to_vec();
    d.settings.major_font = t.major_font.clone();
    d.settings.minor_font = t.minor_font.clone();
    let bytes = wordcraft_docx::write(&d).unwrap();
    assert_eq!(read_theme(&bytes).unwrap(), t);
    // And `Theme::of` takes the same from the document.
    assert_eq!(Theme::of(&d), t);
}

#[test]
fn hostile_theme_files_give_errors() {
    assert!(read_theme(b"not a zip").is_err());
    assert!(read_theme(&[]).is_err());
    // A package with no theme part.
    let empty = zip_of(&[("_rels/.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#)]);
    assert!(read_theme(&empty).is_err());
    // Main part that is not a theme.
    let other = zip_of(&[
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="../../../etc/x.xml"/></Relationships>"#,
        ),
        ("etc/x.xml", "<root/>"),
    ]);
    assert!(read_theme(&other).is_err());
    // Missing colours fall back to the defaults, a broken one is skipped.
    let partial = zip_of(&[
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="theme/theme/theme1.xml"/></Relationships>"#,
        ),
        (
            "theme/theme/theme1.xml",
            r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="x"><a:accent1><a:srgbClr val="ZZZZZZ"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#,
        ),
    ]);
    let t = read_theme(&partial).unwrap();
    assert_eq!(t.colors, THEME_COLORS);
}

fn zip_bytes(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (n, b) in entries {
        zw.start_file(n.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// A theme part is a few kilobytes: one that unpacks past `MAX_THEME_PART` is refused before it
/// is all inflated (a small zip bomb in the themes folder would otherwise cost hundreds of MB).
#[test]
fn theme_part_unpacking_past_the_cap_is_refused() {
    let cap = wordcraft_docx::MAX_THEME_PART;
    let mut parts = unzip(&write_thmx(&sample_theme()).unwrap());
    let theme = parts.iter_mut().find(|(n, _)| n == "theme/theme/theme1.xml").unwrap();
    let xml = String::from_utf8(theme.1.clone()).unwrap();
    // Padding inside the root element: still a valid theme, just far too big.
    theme.1 = xml.replacen("<a:themeElements>", &format!("{}<a:themeElements>", " ".repeat(cap + 1)), 1).into_bytes();
    let bomb = zip_bytes(&parts);
    assert!(bomb.len() < cap / 10, "the zip itself is small ({} bytes)", bomb.len());
    let e = read_theme(&bomb).unwrap_err();
    assert!(e.to_string().contains("limit"), "{e}");
}

/// Only the parts that lead to the theme are unpacked: a document's big pictures don't count
/// against the theme caps.
#[test]
fn big_unrelated_parts_dont_stop_a_document_theme() {
    let mut d = Document::default();
    d.settings.theme_colors = sample_theme().colors.to_vec();
    let mut parts = unzip(&wordcraft_docx::write(&d).unwrap());
    parts.push(("word/media/big.bin".into(), vec![0u8; wordcraft_docx::MAX_THEME_PART * 3]));
    let t = read_theme(&zip_bytes(&parts)).unwrap();
    assert_eq!(t.colors, sample_theme().colors);
}
