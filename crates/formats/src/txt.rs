//! Plain text: UTF-8 (with or without BOM), UTF-16 LE/BE (BOM or sniffed), else the legacy
//! encoding the bytes look like (GBK/GB18030, Big5, Shift_JIS, EUC-KR, Windows code pages…;
//! #778), Windows-1252 when nothing fits better.
//! Export writes UTF-8 with CRLF line ends; list paragraphs get their label and a tab, table
//! cells are separated by tabs.

use wordcraft_doc::Document;

pub use wordcraft_doc::encoding::cp1252;

use crate::model::{FBlock, ListCounter, from_doc};

#[cfg(test)]
mod legacy_oracle_tests;

// Detection is shared with the other Craft apps. WordCraft's `cp1252` and encoding_rs agree on
// every byte (pinned by `legacy_oracle_tests`), so a Windows-1252 guess needs no local mapping.
pub use craft_mime::detect::{decode, is_text, legacy};

/// Decode bytes in a named encoding (a WHATWG label such as `gbk`, `gb18030`, `big5`,
/// `shift_jis`, `euc-kr`, `utf-16le`, `windows-1251`); `None` when the label isn't known.
pub fn decode_as(b: &[u8], label: &str) -> Option<String> {
    let enc = encoding_rs::Encoding::for_label(label.trim().as_bytes())?;
    Some(enc.decode(b).0.into_owned())
}

/// A document with one paragraph per line.
pub fn import(bytes: &[u8]) -> Document {
    let text = decode(bytes);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let clean: String = text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t' | '\u{000C}')).collect();
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
                for row in &t.rows {
                    let cells: Vec<String> = row
                        .iter()
                        .map(|c| {
                            if depth > 8 {
                                return String::new();
                            }
                            let mut v = Vec::new();
                            blocks_text(&c.blocks, &mut v, &mut ListCounter::default(), depth + 1);
                            v.join(" ")
                        })
                        .collect();
                    out.push(cells.join("\t"));
                }
            }
        }
    }
}

/// UTF-8 text, CRLF line ends.
pub fn export(doc: &Document) -> Vec<u8> {
    let Ok(shared) = wordcraft_docx::to_shared_model(doc) else {
        return Vec::new();
    };
    let flow = from_doc(&shared);
    let mut lines = Vec::new();
    blocks_text(&flow.blocks, &mut lines, &mut ListCounter::default(), 0);
    lines.join("\r\n").into_bytes()
}
