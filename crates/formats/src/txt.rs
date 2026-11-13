//! Plain text: UTF-8 (with or without BOM), UTF-16 LE/BE (BOM or sniffed), else the legacy
//! encoding the bytes look like (GBK/GB18030, Big5, Shift_JIS, EUC-KR, Windows code pages…;
//! #778), Windows-1252 when nothing fits better.
//! Export writes UTF-8 with CRLF line ends; list paragraphs get their label and a tab, table
//! cells are separated by tabs.

use wordcraft_doc::Document;

pub use wordcraft_doc::encoding::cp1252;

use crate::model::{FBlock, ListCounter, from_doc};

fn utf16(b: &[u8], le: bool) -> String {
    let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
    String::from_utf16_lossy(&units)
}

fn utf32(b: &[u8], le: bool) -> String {
    let units: Vec<char> = b
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| {
            let u = if le { u32::from_le_bytes(*c) } else { u32::from_be_bytes(*c) };
            char::from_u32(u).unwrap_or('\u{FFFD}')
        })
        .collect();
    units.into_iter().collect()
}

/// UTF-16 without a BOM, told by its zero bytes: `Some(true)` little-endian, `Some(false)`
/// big-endian.
fn unmarked_utf16(b: &[u8]) -> Option<bool> {
    if b.len() < 4 || !b.len().is_multiple_of(2) {
        return None;
    }
    let sample = b.get(..b.len().min(512))?;
    let even_zero = sample.iter().step_by(2).filter(|x| **x == 0).count();
    let odd_zero = sample.iter().skip(1).step_by(2).filter(|x| **x == 0).count();
    let half = sample.len() / 2;
    if odd_zero * 10 >= half * 4 && even_zero * 10 < half {
        return Some(true);
    }
    if even_zero * 10 >= half * 4 && odd_zero * 10 < half {
        return Some(false);
    }
    None
}

/// Does `b` look like plain text, so a file with an extension WordCraft doesn't know (a
/// playlist, a log, a config file…) can open as text the way Word's text import opens it?
///
/// Text has a BOM, or reads as UTF-16 by its zero bytes, or has no NUL byte and next to no
/// other control characters in its first 64 KB. PDFs are refused by their signature (they
/// start as text). An empty file is text.
pub fn is_text(b: &[u8]) -> bool {
    if b.starts_with(&[0xEF, 0xBB, 0xBF])
        || b.starts_with(&[0xFF, 0xFE, 0x00, 0x00])
        || b.starts_with(&[0x00, 0x00, 0xFE, 0xFF])
        || b.starts_with(&[0xFF, 0xFE])
        || b.starts_with(&[0xFE, 0xFF])
    {
        return true;
    }
    if b.starts_with(b"%PDF-") {
        return false;
    }
    let sample = b.get(..b.len().min(64 << 10)).unwrap_or(b);
    // Tab, line feed, form feed, carriage return, DOS end-of-file (^Z) and escape appear in text;
    // other controls in more than 1 of 100 characters mean binary.
    let odd = |c: u32| c == 0 || (c < 0x20 && !matches!(c, 0x09 | 0x0A | 0x0C | 0x0D | 0x1A | 0x1B)) || c == 0x7F;
    if let Some(le) = unmarked_utf16(sample) {
        let units = sample.as_chunks::<2>().0;
        let n = units.iter().filter(|c| odd(u32::from(if le { u16::from_le_bytes(**c) } else { u16::from_be_bytes(**c) }))).count();
        return n.saturating_mul(100) <= units.len();
    }
    if sample.contains(&0) {
        return false;
    }
    sample.iter().filter(|c| odd(u32::from(**c))).count().saturating_mul(100) <= sample.len()
}

/// Decode text bytes: BOMs first, then UTF-16 without BOM (by its zero bytes), UTF-8, and the
/// legacy encoding the bytes look like ([`legacy`]) as the last resort.
pub fn decode(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE, 0x00, 0x00]) {
        return utf32(rest, true);
    }
    if let Some(rest) = b.strip_prefix(&[0x00, 0x00, 0xFE, 0xFF]) {
        return utf32(rest, false);
    }
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    if let Some(le) = unmarked_utf16(b) {
        return utf16(b, le);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => legacy(b),
    }
}

/// Most bytes looked at to guess a legacy encoding.
const SNIFF: usize = 1 << 20;

/// Bytes that aren't UTF-8 in the legacy encoding they look like (chardetng, the detector
/// Firefox uses for unlabelled pages): GBK/GB18030 for Simplified Chinese, Big5, Shift_JIS or
/// EUC-JP, EUC-KR, or a single-byte Windows / ISO code page; Windows-1252 as before when the
/// guess is a single-byte Western one.
pub fn legacy(b: &[u8]) -> String {
    let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    det.feed(b.get(..SNIFF).unwrap_or(b), b.len() <= SNIFF);
    let enc = det.guess(None, chardetng::Utf8Detection::Deny);
    if enc == encoding_rs::WINDOWS_1252 {
        return b.iter().map(|c| cp1252(*c)).collect();
    }
    enc.decode_without_bom_handling(b).0.into_owned()
}

/// Decode bytes in a named encoding (a WHATWG label such as `gbk`, `gb18030`, `big5`,
/// `shift_jis`, `euc-kr`, `utf-16le`, `windows-1251`); `None` when the label isn't known.
pub fn decode_as(b: &[u8], label: &str) -> Option<String> {
    let clean = label.trim().to_ascii_lowercase();
    match clean.as_str() {
        "utf-32" | "utf-32le" => return Some(utf32(b.strip_prefix(&[0xFF, 0xFE, 0x00, 0x00]).unwrap_or(b), true)),
        "utf-32be" => return Some(utf32(b.strip_prefix(&[0x00, 0x00, 0xFE, 0xFF]).unwrap_or(b), false)),
        _ => {}
    }
    let enc = encoding_rs::Encoding::for_label(clean.as_bytes())?;
    Some(enc.decode(b).0.into_owned())
}

/// A document with one paragraph per line.
pub fn import(bytes: &[u8]) -> Document {
    let text = decode(bytes);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let clean: String = text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t' | '\u{000B}' | '\u{000C}' | '\u{000E}')).collect();
    Document::from_text(&clean)
}

/// [`import`] onto `page`: plain text gives no page size (#1381).
pub fn import_on(bytes: &[u8], page: &wordcraft_doc::SectionProps) -> Document {
    let mut doc = import(bytes);
    doc.last_section = page.clone();
    doc
}

fn blocks_text(blocks: &[FBlock], out: &mut Vec<String>, lists: &mut ListCounter, depth: usize) {
    for b in blocks {
        match b {
            FBlock::Para(p) => {
                let mut line = String::new();
                match p.list {
                    Some(li) => {
                        line.push_str(&"\t".repeat(li.level as usize));
                        line.push_str(&lists.label(li));
                        line.push('\t');
                    }
                    None => lists.reset(),
                }
                line.push_str(&p.shown_text().replace('\n', "\r\n").replace(['\u{000C}', '\u{000E}'], ""));
                out.push(line);
            }
            FBlock::Table(t) => {
                lists.reset();
                let cols = t.cols();
                for row in &t.rows {
                    let mut cells: Vec<String> = Vec::new();
                    for c in row {
                        if c.covered {
                            cells.push(String::new());
                        } else if depth > 8 {
                            cells.push(String::new());
                        } else {
                            let mut v = Vec::new();
                            blocks_text(&c.blocks, &mut v, &mut ListCounter::default(), depth + 1);
                            let text = v.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
                            cells.push(text);
                        }
                        for _ in 1..c.colspan.clamp(1, 63) {
                            cells.push(String::new());
                        }
                    }
                    cells.resize(cols, String::new());
                    out.push(cells.join("\t"));
                }
            }
        }
    }
}

/// UTF-8 text, CRLF line ends.
pub fn export(doc: &Document) -> Vec<u8> {
    let flow = from_doc(doc);
    let mut lines = Vec::new();
    blocks_text(&flow.blocks, &mut lines, &mut ListCounter::default(), 0);
    lines.join("\r\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cell, Fmt, Para};
    use wordcraft_doc::StoryRef;

    #[test]
    fn test_utf32_bom_decode_and_sniffing() {
        let mut u32le = vec![0xFF, 0xFE, 0x00, 0x00];
        for c in "Hello 🦀".chars() {
            u32le.extend((c as u32).to_le_bytes());
        }
        assert!(is_text(&u32le));
        assert_eq!(decode(&u32le), "Hello 🦀");
        assert_eq!(decode_as(&u32le, "utf-32"), Some("Hello 🦀".into()));

        let mut u32be = vec![0x00, 0x00, 0xFE, 0xFF];
        for c in "World 🌍".chars() {
            u32be.extend((c as u32).to_be_bytes());
        }
        assert!(is_text(&u32be));
        assert_eq!(decode(&u32be), "World 🌍");
        assert_eq!(decode_as(&u32be, "utf-32be"), Some("World 🌍".into()));
    }

    #[test]
    fn test_import_preserves_soft_breaks() {
        let raw = b"Line 1\x0BSoft break\x0ECol break";
        let doc = import(raw);
        let text = doc.plain_text(StoryRef::Body);
        assert_eq!(text, "Line 1\u{000B}Soft break\u{000E}Col break");
    }

    #[test]
    fn test_table_export_colspan_covered_and_newline_cleaning() {
        let mut t = crate::model::FTable::default();
        let mut c1 = Cell::default();
        c1.colspan = 2;
        let mut p1 = Para::default();
        p1.push_text("Merged\nHeader\tItem", &Fmt::default());
        c1.blocks.push(FBlock::Para(p1));

        let mut c2 = Cell::default();
        let mut p2 = Para::default();
        p2.push_text("Col 3", &Fmt::default());
        c2.blocks.push(FBlock::Para(p2));

        let mut c3 = Cell::default();
        c3.covered = true;

        let mut c4 = Cell::default();
        let mut p4 = Para::default();
        p4.push_text("Row 2 Col 2", &Fmt::default());
        c4.blocks.push(FBlock::Para(p4));

        let mut c5 = Cell::default();
        let mut p5 = Para::default();
        p5.push_text("Row 2 Col 3", &Fmt::default());
        c5.blocks.push(FBlock::Para(p5));

        t.rows.push(vec![c1, c2]);
        t.rows.push(vec![c3, c4, c5]);

        let mut out = Vec::new();
        let blocks = vec![FBlock::Table(t)];
        blocks_text(&blocks, &mut out, &mut ListCounter::default(), 0);

        assert_eq!(out.len(), 2);
        // Row 1: Merged Header Item (colspan 2) -> "Merged Header Item\t\tCol 3"
        assert_eq!(out[0], "Merged Header Item\t\tCol 3");
        // Row 2: Covered cell is empty -> "\tRow 2 Col 2\tRow 2 Col 3"
        assert_eq!(out[1], "\tRow 2 Col 2\tRow 2 Col 3");
    }
}
