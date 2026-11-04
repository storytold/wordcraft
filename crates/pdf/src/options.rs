use super::{NumeralMode, WriteError, imposition};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PdfError {
    #[error("the document has no pages to export")]
    NoPages,
    #[error("page {0} does not exist")]
    BadPage(usize),
    #[error("PDF writing failed: {0}")]
    Write(String),
}

impl From<WriteError> for PdfError {
    fn from(error: WriteError) -> Self {
        Self::Write(error.to_string())
    }
}

/// Export options.
#[derive(Clone, Debug)]
pub struct PdfOptions {
    /// Physical sheet layout used only by printing.
    pub imposition: imposition::Imposition,
    /// Repeat complete sheet plans (padding never spills into another copy).
    pub sheet_copies: u32,
    /// Collate physical sheet sides when repeating a print job.
    pub sheet_collate: bool,
    /// Pages to export (0-based layout page indexes, in output order); `None` = all.
    pub pages: Option<Vec<usize>>,
    /// Title / author overrides (default: the document properties).
    pub title: Option<String>,
    pub author: Option<String>,
    /// Write a structure tree (tagged PDF, best effort).
    pub tagged: bool,
    /// Show tracked changes as markup (coloured, underlined / struck through).
    pub include_markup: bool,
    /// Print Markup's comments: each page is scaled down to make room on its right for the
    /// comments in balloons joined to their anchors (Word's "Document showing markup").
    pub comments: bool,
    /// Compress content streams.
    pub compress: bool,
    /// Write hyperlinks and in-document links as link annotations. A print job leaves them out:
    /// they print nothing, and a printer driver can't fit a page with annotations to the paper
    /// (#4734).
    pub links: bool,
    /// Print on this paper (width, height in points) instead of each page's own size: the page
    /// is scaled to fit and centred, the paper turned to the page's orientation (Scale to paper
    /// size, #2108). `None` keeps the document's page sizes.
    pub paper: Option<(f32, f32)>,
    /// How [`export`] shows digits (the Numeral setting; callers resolve `System`, see
    /// [`NumeralMode::resolve`]). [`export_layout`] keeps the layout it is given.
    pub numeral: NumeralMode,
}

impl Default for PdfOptions {
    /// Keep ordinary document export unchanged unless a print layout is requested.
    fn default() -> Self {
        PdfOptions {
            imposition: Default::default(),
            sheet_copies: 1,
            sheet_collate: true,
            pages: None,
            title: None,
            author: None,
            tagged: true,
            include_markup: false,
            comments: false,
            compress: true,
            links: true,
            paper: None,
            numeral: NumeralMode::Arabic,
        }
    }
}
