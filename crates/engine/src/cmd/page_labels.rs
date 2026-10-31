//! References use displayed labels; page ranges compare physical positions.
use wordcraft_doc::{Document, Pos, page_label::PageLabel, section::NumFormat};
use wordcraft_layout::{DocLayout, Page};

/// Snapshot section labels once per generated list, rather than scan the document per entry.
pub(crate) struct PageLabels(Vec<(NumFormat, Option<PageLabel>)>);

impl PageLabels {
    pub(crate) fn new(doc: &Document) -> Self {
        Self(doc.sections().iter().map(|(_, sec)| (sec.page_num_format, sec.page_label.clone())).collect())
    }

    fn label(&self, page: &Page) -> String {
        let Some((format, label)) = self.0.get(page.section) else { return page.number.to_string() };
        label.as_ref().map_or_else(
            || format.format(page.number),
            |label| {
                let text = label.format(*format, page.number);
                if text.is_empty() { label.name.clone() } else { text }
            },
        )
    }

    pub(crate) fn at(&self, layout: &DocLayout, pos: &Pos) -> (usize, String) {
        layout.caret(pos).and_then(|caret| layout.pages.get(caret.page).map(|page| (caret.page, self.label(page)))).unwrap_or((0, "1".into()))
    }
}
