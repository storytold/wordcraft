//! Drawing a chart part as the DOCX reader draws one.
//!
//! The reader's chart drawing takes a chart part in a package, so the part goes into the
//! smallest package that shows it: one paragraph with the chart in line at the size asked for,
//! and a theme with the document's colours (a chart's scheme colours are the theme's). Reading
//! that package draws the chart exactly as opening a document with it would.

use wordcraft_doc::graphic::GraphicItem;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::Rgb;

/// Largest drawing size, points (the model's own limit for a chart).
const MAX_SIDE: f32 = 2000.0;

/// What the chart part `part` draws as, `w` × `h` points, in the theme colours `theme` (dk1 lt1
/// dk2 lt2 accent1..6 hlink folHlink). Empty when it can't be drawn.
pub fn draw_part(part: &[u8], theme: &[Rgb], w: f32, h: f32) -> Vec<GraphicItem> {
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) || part.len() > super::MAX_PART {
        return Vec::new();
    }
    let (w, h) = (w.min(MAX_SIDE), h.min(MAX_SIDE));
    let Ok(package) = package(part, theme, w, h) else { return Vec::new() };
    let Ok(doc) = crate::read(&package) else { return Vec::new() };
    let mut items = None;
    doc.objects_in_reading_order(&mut |o| {
        if items.is_none()
            && let InlineObject::Graphic { graphic, .. } = o
        {
            items = Some(graphic.items.clone());
        }
    });
    items.unwrap_or_default()
}

fn package(part: &[u8], theme: &[Rgb], w: f32, h: f32) -> Result<Vec<u8>, craft_zip::Error> {
    let emu = |pt: f32| (f64::from(pt) * 12_700.0).round() as i64;
    let document = format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
            r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
            r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
            r#"xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" "#,
            r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
            r#"<w:body><w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">"#,
            r#"<wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="1" name="Chart 1"/>"#,
            r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">"#,
            r#"<c:chart r:id="rIdChart"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
            r#"<w:sectPr><w:pgSz w:w="{pw}" w:h="{ph}"/><w:pgMar w:top="0" w:right="0" w:bottom="0" w:left="0" w:header="0" w:footer="0" w:gutter="0"/></w:sectPr>"#,
            r#"</w:body></w:document>"#
        ),
        cx = emu(w),
        cy = emu(h),
        // A page the chart fits on, so it is never shrunk to the text width.
        pw = ((w + 72.0) * 20.0).ceil() as i64,
        ph = ((h + 72.0) * 20.0).ceil() as i64,
    );
    let rels = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Id="rIdChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/>"#,
        r#"<Relationship Id="rIdTheme" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>"#,
        r#"</Relationships>"#
    );
    let root_rels = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#,
        r#"</Relationships>"#
    );
    let types = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
        r#"<Default Extension="xml" ContentType="application/xml"/>"#,
        r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
        r#"<Override PartName="/word/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#,
        r#"<Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>"#,
        r#"</Types>"#
    );
    let mut z = craft_zip::Writer::new();
    z.add("[Content_Types].xml", types.as_bytes(), craft_zip::Compression::Stored)?;
    z.add("_rels/.rels", root_rels.as_bytes(), craft_zip::Compression::Stored)?;
    z.add("word/document.xml", document.as_bytes(), craft_zip::Compression::Stored)?;
    z.add("word/_rels/document.xml.rels", rels.as_bytes(), craft_zip::Compression::Stored)?;
    z.add("word/charts/chart1.xml", part, craft_zip::Compression::Stored)?;
    z.add("word/theme/theme1.xml", theme_xml(theme).as_bytes(), craft_zip::Compression::Stored)?;
    z.finish()
}

/// A theme part with only a colour scheme: `theme`'s colours, Office's for any it lacks.
fn theme_xml(theme: &[Rgb]) -> String {
    const NAMES: [&str; 12] = ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];
    const OFFICE: [Rgb; 12] = [
        Rgb(0x00, 0x00, 0x00),
        Rgb(0xFF, 0xFF, 0xFF),
        Rgb(0x0E, 0x28, 0x41),
        Rgb(0xE8, 0xE8, 0xE8),
        Rgb(0x15, 0x60, 0x82),
        Rgb(0xE9, 0x71, 0x32),
        Rgb(0x19, 0x6B, 0x24),
        Rgb(0x0F, 0x9E, 0xD5),
        Rgb(0xA0, 0x2B, 0x93),
        Rgb(0x4E, 0xA7, 0x2E),
        Rgb(0x46, 0x78, 0x86),
        Rgb(0x96, 0x60, 0x7D),
    ];
    let mut s = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Chart"><a:themeElements><a:clrScheme name="Chart">"#
    ));
    for (i, (name, fallback)) in NAMES.iter().zip(OFFICE).enumerate() {
        let c = theme.get(i).copied().unwrap_or(fallback);
        s.push_str(&format!(r#"<a:{name}><a:srgbClr val="{:02X}{:02X}{:02X}"/></a:{name}>"#, c.0, c.1, c.2));
    }
    s.push_str("</a:clrScheme></a:themeElements></a:theme>");
    s
}
