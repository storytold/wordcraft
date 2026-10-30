use std::io::{self, Write};

use wordcraft_doc::Document;

/// Native files must fit the existing whole-document reader ceiling. Shared run metadata
/// can expand during JSON serialization, so charge actual escaped bytes before allocating.
pub(super) fn save(doc: &Document, limit: u64) -> Result<Vec<u8>, String> {
    let mut out = Buffer { bytes: Vec::new(), limit: usize::try_from(limit).unwrap_or(usize::MAX) };
    serde_json::to_writer_pretty(&mut out, doc).map_err(|e| e.to_string())?;
    Ok(out.bytes)
}

struct Buffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let needed = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| io::Error::other("native JSON exceeds the document file size limit"))?;
        if needed > self.bytes.capacity() {
            let capacity = self.bytes.capacity().saturating_mul(2).max(4096).max(needed).min(self.limit);
            self.bytes.try_reserve_exact(capacity - self.bytes.len()).map_err(|_| io::Error::other("not enough memory to save native JSON"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{CharProps, Pos};

    fn document() -> Document {
        let mut doc = Document::new();
        let hint: std::sync::Arc<str> = "Original \"hint\" Ω. ".repeat(512).into();
        for i in 0..32 {
            let end = doc.body[0].as_para().unwrap().len();
            doc.insert_text(
                &Pos::body(0, end),
                "word ",
                &CharProps { link: Some("#target".into()), link_tooltip: Some(hint.clone()), bold: Some(i % 2 == 0), ..Default::default() },
            )
            .unwrap();
        }
        doc
    }

    #[test]
    fn native_json_buffer_refuses_before_growing_or_retaining_extra_bytes() {
        let mut out = Buffer { bytes: Vec::new(), limit: 10 };
        out.write_all(b"0123456789").unwrap();
        assert!(out.write_all(b"extra").is_err());
        assert_eq!(out.bytes, b"0123456789");
        assert!(out.bytes.capacity() <= 10);
        assert!(Buffer { bytes: Vec::new(), limit: 0 }.write_all(b"x").is_err());
    }

    #[test]
    fn native_json_refuses_repeated_hint_expansion_before_exceeding_its_budget() {
        let doc = document();
        let before = doc.clone();
        assert!(save(&doc, 4096).is_err());
        assert_eq!(doc, before);
    }

    #[test]
    fn native_json_keeps_exact_pretty_bytes_at_the_budget_boundary() {
        let doc = document();
        let expected = serde_json::to_vec_pretty(&doc).unwrap();
        assert_eq!(save(&doc, expected.len() as u64).unwrap(), expected);
        assert!(save(&doc, expected.len() as u64 - 1).is_err());
    }
}
