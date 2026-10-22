//! The ODT writer against LibreOffice: synthetic documents are saved as `.docx` and `.odt`,
//! LibreOffice renders both to PDF, and the two PDFs must have the same pages and lines in the
//! same places (`pdftotext -bbox-layout`). LibreOffice laying out the DOCX is the reference: the
//! ODT should give it everything the DOCX does.
//!
//! Ignored by default: run with `cargo test -p wordcraft-engine --test odt_lo_render -- --ignored`
//! where LibreOffice (`soffice`, or `WORDCRAFT_SOFFICE`) and poppler's `pdftotext` are installed.
//! `WORDCRAFT_LO_KEEP=DIR` keeps the files and PDFs there.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::Command;

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{CharProps, ParaProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::section::SectionProps;
use wordcraft_doc::styles::{Style, StyleKind};
use wordcraft_doc::{Document, Paragraph, PartKind, para_block};
use wordcraft_engine::io;

fn soffice() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("WORDCRAFT_SOFFICE") {
        return Some(PathBuf::from(p));
    }
    let mac = PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice");
    if mac.exists() {
        return Some(mac);
    }
    Command::new("soffice").arg("--version").output().ok().filter(|o| o.status.success()).map(|_| PathBuf::from("soffice"))
}

fn has_pdftotext() -> bool {
    Command::new("pdftotext").arg("-v").output().is_ok()
}

/// One text line of a PDF: page, top left corner, text.
#[derive(Debug, Clone)]
struct Line {
    page: usize,
    x: f64,
    y: f64,
    text: String,
}

fn attr(tag: &str, name: &str) -> Option<f64> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    tag.get(at..)?.split('"').next()?.parse().ok()
}

fn lines(pdf: &Path) -> Vec<Line> {
    let out = Command::new("pdftotext").args(["-bbox-layout".as_ref(), pdf.as_os_str(), "-".as_ref()]).output().unwrap();
    let xml = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut v = Vec::new();
    for (page, body) in xml.split("<page ").skip(1).enumerate() {
        for line in body.split("<line").skip(1) {
            let line = line.split("</line>").next().unwrap_or("");
            let (x, y) = (attr(line, "xMin").unwrap_or(0.0), attr(line, "yMin").unwrap_or(0.0));
            let words: Vec<&str> = line.split("<word").skip(1).filter_map(|w| w.split('>').nth(1)?.split("</word").next()).collect();
            v.push(Line { page, x, y, text: words.join(" ") });
        }
    }
    v
}

/// Render `docs` (name, document) as `.docx` and `.odt` through LibreOffice in one run; the PDFs
/// of each, `(docx, odt)`, by name.
fn render(soffice: &Path, docs: &[(&str, &Document)]) -> (PathBuf, Vec<(PathBuf, PathBuf)>) {
    let dir = match std::env::var("WORDCRAFT_LO_KEEP") {
        Ok(d) => PathBuf::from(d),
        Err(_) => std::env::temp_dir().join(format!("wordcraft-odt-lo-{}", std::process::id())),
    };
    std::fs::create_dir_all(&dir).unwrap();
    let mut files = Vec::new();
    for (name, doc) in docs {
        for ext in ["docx", "odt"] {
            let p = dir.join(format!("{name}_{ext}.{ext}"));
            std::fs::write(&p, io::save_bytes(&format!("x.{ext}"), doc).unwrap()).unwrap();
            files.push(p);
        }
    }
    // One LibreOffice process with its own profile, so it neither waits for nor disturbs a
    // running one.
    let profile = format!("-env:UserInstallation=file://{}", dir.join("profile").display());
    let st = Command::new(soffice)
        .args([profile.as_str(), "--headless", "--norestore", "--convert-to", "pdf", "--outdir"])
        .arg(&dir)
        .args(&files)
        .output()
        .unwrap();
    assert!(st.status.success(), "soffice: {}", String::from_utf8_lossy(&st.stderr));
    let pdfs = docs.iter().map(|(name, _)| (dir.join(format!("{name}_docx.pdf")), dir.join(format!("{name}_odt.pdf")))).collect();
    (dir, pdfs)
}

/// Pages and line positions of the two PDFs: same page count, every reference line found on the
/// same page with the same text, at most `tol` points off either way.
fn compare(name: &str, docx: &Path, odt: &Path, tol: f64) -> Result<(), String> {
    compare_lines(name, lines(docx), lines(odt), tol)
}

fn compare_lines(name: &str, a: Vec<Line>, b: Vec<Line>, tol: f64) -> Result<(), String> {
    let pages = |l: &[Line]| l.iter().map(|l| l.page + 1).max().unwrap_or(0);
    let mut errs = Vec::new();
    if pages(&a) != pages(&b) {
        errs.push(format!("pages: docx {} odt {}", pages(&a), pages(&b)));
    }
    // Lines are matched in order, so a sentence repeated on a page matches its own copy.
    let mut next = 0;
    for (i, l) in a.iter().enumerate() {
        // Words are compared without the spaces between them and with a tab leader's dots as
        // one: pdftotext splits a word from the leader after it in one PDF and not in the
        // other, and a leader 0.5 pt longer has a dot more.
        let norm = |t: &str| {
            let mut n = String::new();
            for c in t.chars().filter(|c| *c != ' ') {
                if !(c == '.' && n.ends_with("..")) {
                    n.push(c);
                }
            }
            n
        };
        let same = |a: &str, b: &str| norm(a) == norm(b);
        // A drawing's text may come before or after the paragraph it is anchored in.
        let here = |m: &&Line| m.page == l.page && same(&m.text, &l.text);
        let found = b.iter().enumerate().skip(next).find(|(_, m)| here(m)).or_else(|| b.iter().enumerate().find(|(_, m)| here(m)));
        if let Some((j, _)) = found {
            next = j + 1;
        }
        match found.map(|(_, m)| m) {
            Some(m) if (m.y - l.y).abs() <= tol && (m.x - l.x).abs() <= tol => {}
            Some(m) => errs.push(format!("line {i} {:?} p{}: at {:.1},{:.1} vs {:.1},{:.1}", l.text, l.page + 1, l.x, l.y, m.x, m.y)),
            None => errs.push(format!("line {i} {:?} p{} y {:.1}: not on that page", l.text, l.page + 1, l.y)),
        }
    }
    if errs.is_empty() { Ok(()) } else { Err(format!("{name}: {} of {} lines differ\n{}", errs.len(), a.len(), errs.join("\n"))) }
}

fn plain(t: &str) -> Paragraph {
    Paragraph::with_text(t, CharProps::default())
}

/// Synthetic body text: `n` sentences.
fn prose(seed: usize, n: usize) -> String {
    const WORDS: [&str; 16] = [
        "river", "lantern", "harbor", "meadow", "copper", "window", "orchard", "signal", "valley", "thread", "garden", "pillar", "canvas", "ladder",
        "summit", "beacon",
    ];
    (0..n)
        .map(|s| {
            let k = 8 + (seed + s) % 7;
            let words: Vec<&str> = (0..k).map(|w| WORDS[(seed * 7 + s * 3 + w * 5) % WORDS.len()]).collect();
            let mut t = words.join(" ");
            t.replace_range(0..1, &t[0..1].to_uppercase());
            t + "."
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Three sections (portrait, landscape restarting the page numbers, portrait), headers with a
/// different first page, page number footers, named styles with spacing and inheritance, tab
/// stops with leaders, a page break inside a paragraph.
fn sections_and_styles() -> Document {
    let mut d = Document::new();
    d.styles.upsert(Style {
        id: "Lead".into(),
        name: "Lead Paragraph".into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        para: ParaProps { space_before: Some(18.0), space_after: Some(6.0), indent_left: Some(24.0), ..Default::default() },
        chr: CharProps { size: Some(13.0), ..Default::default() },
        ..Default::default()
    });
    d.styles.upsert(Style {
        id: "LeadTight".into(),
        name: "Lead Tight".into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Lead".into()),
        para: ParaProps { space_after: Some(0.0), contextual_spacing: Some(true), ..Default::default() },
        ..Default::default()
    });
    d.styles.upsert(Style {
        id: "Entry".into(),
        name: "Entry".into(),
        kind: StyleKind::Paragraph,
        based_on: Some("Normal".into()),
        para: ParaProps {
            indent_left: Some(36.0),
            tabs: Some(vec![TabStop { pos: 400.0, align: TabAlign::Right, leader: TabLeader::Dot }]),
            ..Default::default()
        },
        ..Default::default()
    });
    let field = |instr: &str, result: &str| InlineObject::Field {
        instr: instr.into(),
        result: result.into(),
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
    };
    let head = d.add_part(PartKind::Header, vec![para_block(plain("Quarterly field notes"))]);
    let head_first = d.add_part(PartKind::Header, vec![para_block(plain("Opening page header"))]);
    let mut foot = plain("Page ");
    let n = foot.len();
    foot.insert_object(n, field("PAGE", "1"), &CharProps::default()).unwrap();
    let foot = d.add_part(PartKind::Footer, vec![para_block(foot)]);
    let mut wide_foot = plain("Wide pages, page ");
    let n = wide_foot.len();
    wide_foot.insert_object(n, field("PAGE", "1"), &CharProps::default()).unwrap();
    let wide_foot = d.add_part(PartKind::Footer, vec![para_block(wide_foot)]);

    let mut body = vec![para_block(plain("Field notes").styled("Heading1"))];
    for i in 0..5 {
        body.push(para_block(plain(&prose(i, 6))));
    }
    for i in 0..3 {
        body.push(para_block(plain(&prose(10 + i, 2)).styled("Lead")));
    }
    for i in 0..3 {
        body.push(para_block(plain(&prose(20 + i, 1)).styled("LeadTight")));
    }
    for (k, (a, b)) in [("Harbor survey", "12"), ("Orchard counts", "27"), ("Signal log", "41")].into_iter().enumerate() {
        body.push(para_block(plain(&format!("{}. {a}\t{b}", k + 1)).styled("Entry")));
    }
    body.push(para_block(plain(&format!("{}\u{000C}{}", prose(30, 2), prose(31, 3)))));
    let mut first = SectionProps { title_page: true, ..SectionProps::default() };
    first.headers.default = Some(head);
    first.headers.first = Some(head_first);
    first.footers.default = Some(foot);
    let mut end = plain("Closing words of the upright part.");
    end.section = Some(Box::new(first));
    body.push(para_block(end));

    body.push(para_block(plain("Wide part").styled("Heading2")));
    for i in 0..4 {
        body.push(para_block(plain(&prose(40 + i, 7))));
    }
    let mut wide = SectionProps { page_num_start: Some(1), ..SectionProps::default() };
    wide.set_landscape(true);
    wide.footers.default = Some(wide_foot);
    let mut end = plain("Closing words of the wide part.");
    end.section = Some(Box::new(wide));
    body.push(para_block(end));

    body.push(para_block(plain("Back upright").styled("Heading2")));
    for i in 0..3 {
        body.push(para_block(plain(&prose(50 + i, 5))));
    }
    d.last_section = SectionProps::default();
    d.body = body;
    d
}

/// Fonts LibreOffice ships, everywhere: a missing font is substituted differently for the two
/// formats, which would measure the substitution rather than the writer.
fn with_shipped_fonts(mut d: Document) -> Document {
    d.styles.default_chr.font = Some("Liberation Serif".into());
    for s in &mut d.styles.styles {
        if s.chr.font.is_some() {
            s.chr.font = Some("Liberation Sans".into());
        }
    }
    d
}

#[test]
#[ignore = "needs LibreOffice (soffice or WORDCRAFT_SOFFICE) and poppler's pdftotext"]
fn odt_lays_out_like_the_docx_in_libreoffice() {
    let (Some(soffice), true) = (soffice(), has_pdftotext()) else {
        eprintln!("skipped: LibreOffice or pdftotext not installed");
        return;
    };
    let a = with_shipped_fonts(sections_and_styles());
    let docs = [("sections", &a)];
    let (dir, pdfs) = render(&soffice, &docs);
    let mut errs = Vec::new();
    for ((name, _), (docx, odt)) in docs.iter().zip(&pdfs) {
        if let Err(e) = compare(name, docx, odt, 1.0) {
            errs.push(e);
        }
    }
    if std::env::var("WORDCRAFT_LO_KEEP").is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(errs.is_empty(), "{}", errs.join("\n\n"));
}
