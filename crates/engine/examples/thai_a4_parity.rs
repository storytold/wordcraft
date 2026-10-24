//! Generate the issue #1548 A4 PDF/DOCX fixtures with installed Thai fonts.
//!
//! Run with `cargo run -p wordcraft-engine --example thai_a4_parity -- target/thai-a4-output`.

use std::error::Error;
use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use wordcraft_doc::props::Align;
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, Placed, layout};
use wordcraft_pdf::{PdfOptions, export_layout};

const ISSUE_TEXT: &str = "[A] การประชุมคณะกรรมการบริหารโครงการพัฒนาระบบสารสนเทศเพื่อการบริหารจัดการทรัพยากรบุคคลขององค์กรปกครองส่วนท้องถิ่นได้มีมติเห็นชอบให้ดำเนินการจัดซื้อจัดจ้างเครื่องคอมพิวเตอร์แม่ข่ายพร้อมอุปกรณ์จัดเก็บข้อมูลสำรองภายในปีงบประมาณพุทธศักราชสองพันห้าร้อยหกสิบเก้าเพื่อรองรับการขยายตัวของระบบงานและลดความเสี่ยงจากการสูญหายของข้อมูลสำคัญซึ่งอาจส่งผลกระทบต่อการให้บริการประชาชน";

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("target/thai-a4-output"));
    fs::create_dir_all(&output)?;
    let fonts = [("Angsana New", 16.0), ("Tahoma", 11.0), ("Sarabun", 16.0)];
    let db = wordcraft_fonts::FontDb::global();
    let mut report = String::from(
        "WordCraft Thai A4 export regression\nPaper: A4 portrait (595.276 × 841.890 pt)\nMargins: 25.4 mm / 72 pt on each side\nAlignment: explicit Thai Distributed\nCompatibility: local installed fonts, default print layout settings\n\n",
    );
    let mut produced = 0usize;
    for (family, size) in fonts {
        if !db.has_family(family) {
            let note = format!("SKIPPED {family} {size} pt: family is not installed\n");
            print!("{note}");
            report.push_str(&note);
            continue;
        }
        let requested = wordcraft_fonts::word::resolve(family, false, false);
        if requested.substituted || !requested.face.named(family) {
            return Err(io_error(format!("{family} is installed but resolved to {} {}", requested.face.family, requested.face.style)).into());
        }
        if !requested.face.covers('\u{0E01}') {
            return Err(io_error(format!("{family} has no Thai base-letter glyph; refusing fallback")).into());
        }

        let doc = make_document(family, size)?;
        let first = first_line(&doc, &layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default()))?;
        if !first.starts_with("[A] ") || first.trim() == "[A]" || !first.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c)) {
            return Err(io_error(format!("{family}: first line isolates [A] or has no Thai text: {first:?}")).into());
        }
        verify_used_faces(&doc, &layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default()), family)?;

        let first_save = wordcraft_docx::write(&doc)?;
        let reopened_once = wordcraft_docx::read(&first_save)?;
        verify_document(&reopened_once, family, size)?;
        let second_save = wordcraft_docx::write(&reopened_once)?;
        let reopened_twice = wordcraft_docx::read(&second_save)?;
        verify_document(&reopened_twice, family, size)?;
        let original_layout = layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
        let reopened_layout = layout(&reopened_twice, &mut LayoutCache::new(), &LayoutOptions::default());
        compare_layouts(&original_layout, &reopened_layout)?;
        verify_used_faces(&reopened_twice, &reopened_layout, family)?;

        let xml = document_xml(&second_save)?;
        if !xml.contains("w:val=\"thaiDistribute\"") {
            return Err(io_error(format!("{family}: DOCX XML lost w:jc thaiDistribute")).into());
        }
        let pdf = export_layout(&reopened_twice, &reopened_layout, &PdfOptions::default())?;
        if !pdf.starts_with(b"%PDF-") {
            return Err(io_error(format!("{family}: PDF header is invalid")).into());
        }
        let stem = slug(family);
        let pdf_path = output.join(format!("{stem}-a4.pdf"));
        let docx_path = output.join(format!("{stem}-a4.docx"));
        let xml_path = output.join(format!("{stem}-document.xml"));
        for path in [&pdf_path, &docx_path, &xml_path] {
            if path.exists() {
                return Err(io_error(format!("refusing to overwrite existing output {}", path.display())).into());
            }
        }
        fs::write(&docx_path, second_save)?;
        fs::write(&pdf_path, pdf)?;
        fs::write(&xml_path, xml)?;
        let paragraph = body_para(&reopened_layout)?;
        let note = format!(
            "PASS {family} {} pt: {} {}, Thai glyphs covered, {} lines / {} pages, first line {:?}\n  PDF: {}\n  DOCX: {} (saved, reopened and saved again)\n  XML: {} (contains thaiDistribute)\n",
            size,
            requested.face.family,
            requested.face.style,
            paragraph.lines.len(),
            reopened_layout.pages.len(),
            first,
            pdf_path.display(),
            docx_path.display(),
            xml_path.display()
        );
        print!("{note}");
        report.push_str(&note);
        produced += 1;
    }
    if produced == 0 {
        return Err(io_error("no requested Thai fonts are installed; no export fixtures were produced").into());
    }
    fs::write(output.join("README.txt"), report)?;
    Ok(())
}

fn make_document(family: &str, size: f32) -> Result<Document, Box<dyn Error>> {
    let mut doc = Document::from_text(ISSUE_TEXT);
    // DOCX page size is stored in twentieths of a point; these are the exact round-tripped
    // twip values for 210 × 297 mm (A4) in WordprocessingML.
    doc.last_section.page_w = 595.3;
    doc.last_section.page_h = 841.9;
    doc.last_section.landscape = false;
    doc.last_section.margin_top = 72.0;
    doc.last_section.margin_bottom = 72.0;
    doc.last_section.margin_left = 72.0;
    doc.last_section.margin_right = 72.0;
    doc.format_paragraphs(&Pos::body(0, 0), &Pos::body(0, 0), &|p| {
        p.align = Some(Align::ThaiDistribute);
    })?;
    doc.format_range(&Pos::body(0, 0), &Pos::body(0, ISSUE_TEXT.len()), &|c| {
        c.font = Some(family.to_string());
        c.font_cs = Some(family.to_string());
        c.size = Some(size);
        c.size_cs = Some(size);
    })?;
    Ok(doc)
}

fn verify_document(doc: &Document, family: &str, size: f32) -> Result<(), Box<dyn Error>> {
    if doc.plain_text(StoryRef::Body) != ISSUE_TEXT {
        return Err(io_error(format!("{family}: DOCX round trip changed the source text")).into());
    }
    if doc.last_section.page_w.round() != 595.0
        || doc.last_section.page_h.round() != 842.0
        || [doc.last_section.margin_top, doc.last_section.margin_bottom, doc.last_section.margin_left, doc.last_section.margin_right]
            .iter()
            .any(|m| (*m - 72.0).abs() > 0.01)
    {
        return Err(io_error(format!("{family}: DOCX round trip changed the A4 page setup")).into());
    }
    let para = doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).ok_or_else(|| io_error("paragraph 0 is missing"))?;
    if para.props.align != Some(Align::ThaiDistribute) {
        return Err(io_error(format!("{family}: DOCX round trip changed Thai Distributed alignment")).into());
    }
    let props = para.props_of_char(4);
    if props.font.as_deref() != Some(family) || props.font_cs.as_deref() != Some(family) || props.size != Some(size) || props.size_cs != Some(size) {
        return Err(io_error(format!("{family}: DOCX round trip changed font or size: {props:?}")).into());
    }
    Ok(())
}

fn verify_used_faces(doc: &Document, layout: &DocLayout, family: &str) -> Result<(), Box<dyn Error>> {
    let paragraph = body_para(layout)?;
    let source = doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).ok_or_else(|| io_error("paragraph 0 is missing"))?;
    let mut thai_glyphs = 0usize;
    for cluster in &paragraph.clusters {
        if cluster.kind != wordcraft_layout::para::ClKind::Text {
            continue;
        }
        let Some(text) = source.text.get(cluster.start..cluster.end) else { continue };
        let thai: Vec<char> = text.chars().filter(|c| ('\u{0E00}'..='\u{0E7F}').contains(c)).collect();
        if thai.is_empty() {
            continue;
        }
        let style = paragraph.styles.get(cluster.style as usize).ok_or_else(|| io_error("Thai cluster has no style"))?;
        if !style.face.named(family) || thai.iter().any(|c| !style.face.covers(*c)) {
            return Err(io_error(format!("{family}: Thai text used fallback face {} {}", style.face.family, style.face.style)).into());
        }
        let glyphs = paragraph.glyphs.get(cluster.g0 as usize..cluster.g1 as usize).ok_or_else(|| io_error("invalid Thai glyph range"))?;
        if glyphs.iter().any(|g| g.gid == 0) {
            return Err(io_error(format!("{family}: Thai text contains a .notdef glyph")).into());
        }
        thai_glyphs += thai.len();
    }
    if thai_glyphs == 0 {
        return Err(io_error(format!("{family}: layout contains no Thai glyphs")).into());
    }
    Ok(())
}

fn body_para(layout: &DocLayout) -> Result<&wordcraft_layout::ParaLayout, Box<dyn Error>> {
    layout
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Placed::Lines { story: StoryRef::Body, path, para, .. } if path.0 == [0] => Some(para.as_ref()),
            _ => None,
        })
        .ok_or_else(|| io_error("body paragraph 0 is absent from layout").into())
}

fn first_line<'a>(doc: &'a Document, layout: &DocLayout) -> Result<&'a str, Box<dyn Error>> {
    let paragraph = doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).ok_or_else(|| io_error("paragraph 0 is missing"))?;
    let line = body_para(layout)?.lines.first().ok_or_else(|| io_error("paragraph has no first line"))?;
    paragraph.text.get(line.start..line.stop).ok_or_else(|| io_error("first line has invalid source offsets").into())
}

fn compare_layouts(a: &DocLayout, b: &DocLayout) -> Result<(), Box<dyn Error>> {
    if a.pages.len() != b.pages.len() {
        return Err(io_error(format!("DOCX reopen changed page count: {} vs {}", a.pages.len(), b.pages.len())).into());
    }
    let x = body_para(a)?;
    let y = body_para(b)?;
    if x.lines.len() != y.lines.len() {
        return Err(io_error(format!("DOCX reopen changed line count: {} vs {}", x.lines.len(), y.lines.len())).into());
    }
    for (i, (left, right)) in x.lines.iter().zip(&y.lines).enumerate() {
        if (left.start, left.stop, left.end) != (right.start, right.stop, right.end) {
            return Err(io_error(format!(
                "DOCX reopen changed line {i} source span/end: {:?} vs {:?}",
                (left.start, left.stop, left.end),
                (right.start, right.stop, right.end)
            ))
            .into());
        }
        if left.xs.len() != right.xs.len() {
            return Err(io_error(format!("DOCX reopen changed line {i} glyph-position count: {} vs {}", left.xs.len(), right.xs.len())).into());
        }
        if let Some((n, (u, v))) = left.xs.iter().zip(&right.xs).enumerate().find(|(_, (u, v))| (*u - *v).abs() > 0.02) {
            return Err(io_error(format!("DOCX reopen changed line {i} glyph position {n}: {u} vs {v}")).into());
        }
    }
    Ok(())
}

fn document_xml(bytes: &[u8]) -> Result<String, Box<dyn Error>> {
    let mut package = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut part = package.by_name("word/document.xml")?;
    let mut xml = String::new();
    part.read_to_string(&mut xml)?;
    Ok(xml)
}

fn slug(family: &str) -> String {
    family.to_ascii_lowercase().replace(' ', "-")
}

fn io_error(message: impl Into<String>) -> std::io::Error {
    std::io::Error::other(message.into())
}
