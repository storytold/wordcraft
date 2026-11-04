//! ODT export from the full document model (ODF 1.3), the way the DOCX writer works: named
//! styles with their inheritance, one master page per section with its headers and footers,
//! direct formatting as automatic styles. See `docs/odt-writer.md` for the module map and what
//! each part covers.
//!
//! The writer grows in parts (#2669). [`can_write`] says whether it writes everything a document
//! holds; `odt::export` sends any other document to the exchange-flow writer
//! (`odt::export_flow`), so no part makes an export lose what the flow writer kept.
//!
//! [`Ctx`] is the state shared by the parts: the automatic style registries (one for
//! `content.xml`, one for `styles.xml`, which holds the automatic styles of headers and footers),
//! the fonts used, the pictures to package and the page start waiting for the next block.
//! Stories (header, footer, cell) are written by [`text::blocks`].

mod lists;
mod package;
mod pages;
mod styles;
mod tables;
#[cfg(test)]
mod tests;
mod text;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use wordcraft_doc::Document;

/// Element properties (`fo:margin-top` → `6pt`), written in key order.
pub(super) type Props = BTreeMap<&'static str, String>;

/// ` k="v"` for every property.
pub(super) fn attrs(p: &Props) -> String {
    p.iter().map(|(k, v)| format!(" {k}=\"{}\"", super::x(v))).collect()
}

/// A length in points, as ODF writes it (`12pt`, `0.5pt`).
pub(super) fn pt(v: f32) -> String {
    super::pt(if v.is_finite() { v } else { 0.0 })
}

/// Which file the story being written goes to: automatic styles of headers and footers live in
/// `styles.xml`, those of the body in `content.xml` (ODF 1.3 Part 3 §3.15.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Target {
    Content,
    Styles,
}

/// Automatic styles of one file: their XML in order of creation, deduplicated by content.
#[derive(Default)]
struct Auto {
    names: HashMap<String, String>,
    xml: String,
    counts: HashMap<String, usize>,
}

/// A page change waiting for the next body block: the master page it starts and the page
/// number it restarts at (ODF puts both on the block's style).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PageStart {
    pub master: String,
    pub number: Option<u32>,
}

/// A packaged file: path in the zip, bytes, media type.
pub(super) struct Packaged {
    pub path: String,
    pub data: Arc<Vec<u8>>,
    pub mime: String,
}

pub(super) struct Ctx<'d> {
    pub doc: &'d Document,
    pub target: Target,
    auto: [Auto; 2],
    /// Font families used (`style:font-face` declarations).
    pub fonts: BTreeSet<String>,
    /// Pictures to package, and the path each media key went to.
    pub files: Vec<Packaged>,
    media_paths: HashMap<String, String>,
    /// Set by the body walk at a section start; the next body paragraph or table takes it.
    pub page_start: Option<PageStart>,
    /// Numbered names (`Image3`, `Table2`) by prefix.
    names: HashMap<&'static str, usize>,
    pub lists: lists::State,
    /// Story nesting (cells), bounded.
    pub depth: usize,
    /// A page (`"page"`) or column break ending the last paragraph, for the next block.
    pub break_next: Option<&'static str>,
    /// Word's list counters over the list items written so far, in document order.
    pub counters: wordcraft_doc::numbering::Counters,
}

/// Deepest nesting of stories written (tables in cells).
pub(super) const MAX_DEPTH: usize = 32;

impl<'d> Ctx<'d> {
    fn new(doc: &'d Document) -> Self {
        Ctx {
            doc,
            target: Target::Content,
            auto: Default::default(),
            fonts: BTreeSet::new(),
            files: Vec::new(),
            media_paths: HashMap::new(),
            page_start: None,
            names: HashMap::new(),
            lists: Default::default(),
            depth: 0,
            break_next: None,
            counters: Default::default(),
        }
    }

    fn registry(&mut self) -> &mut Auto {
        let [content, styles] = &mut self.auto;
        match self.target {
            Target::Content => content,
            Target::Styles => styles,
        }
    }

    /// An automatic style element of the current file, deduplicated: `make` gets the new name
    /// and returns the element. Names start with `prefix` (`M` + prefix in `styles.xml`).
    pub fn auto(&mut self, prefix: &str, key: &str, make: impl FnOnce(&str) -> String) -> String {
        let full = format!("{prefix}\u{1}{key}");
        let lead = if self.target == Target::Styles { "M" } else { "" };
        let reg = self.registry();
        if let Some(n) = reg.names.get(&full) {
            return n.clone();
        }
        let count = reg.counts.entry(prefix.to_string()).or_insert(0);
        *count += 1;
        let name = format!("{lead}{prefix}{count}");
        reg.xml.push_str(&make(&name));
        reg.names.insert(full, name.clone());
        name
    }

    /// An automatic `style:style` of `family` with extra attributes `head` (` style:parent-style-name="…"`)
    /// and child elements `inner`.
    pub fn auto_style(&mut self, family: &str, head: &str, inner: &str) -> String {
        let prefix = match family {
            "paragraph" => "P",
            "text" => "T",
            "table" => "Tb",
            "table-column" => "Co",
            "table-cell" => "Ce",
            "section" => "Se",
            _ => "X",
        };
        let key = format!("{family}\u{1}{head}\u{1}{inner}");
        self.auto(prefix, &key, |name| format!("<style:style style:name=\"{name}\" style:family=\"{family}\"{head}>{inner}</style:style>"))
    }

    /// The next numbered name with `prefix` (`Image1`, `Image2`…), unique in the document.
    pub fn name(&mut self, prefix: &'static str) -> String {
        let n = self.names.entry(prefix).or_insert(0);
        *n += 1;
        format!("{prefix}{n}")
    }

    /// Declare font `family` and return the name styles refer to it by.
    pub fn font(&mut self, family: &str) -> String {
        let f = family.trim().to_string();
        self.fonts.insert(f.clone());
        f
    }

    /// The package path of picture `media` (a `Document::media` key), packaging it the first
    /// time; `None` when the document has no such media.
    pub fn picture(&mut self, media: &str) -> Option<String> {
        if let Some(p) = self.media_paths.get(media) {
            return Some(p.clone());
        }
        let data = self.doc.media.get(media)?.clone();
        let ext = match crate::model::sniff_image(&data) {
            Some(e) => e.to_string(),
            None => media.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default(),
        };
        let safe: String = ext.chars().filter(char::is_ascii_alphanumeric).take(8).collect();
        let path = format!("Pictures/image{}.{}", self.files.len() + 1, if safe.is_empty() { "bin" } else { &safe });
        self.files.push(Packaged { path: path.clone(), data, mime: crate::model::mime_of(&ext).to_string() });
        self.media_paths.insert(media.to_string(), path.clone());
        Some(path)
    }

    /// Write a nested story with `f`, unless nesting is already too deep.
    pub fn nested(&mut self, out: &mut String, f: impl FnOnce(&mut Self, &mut String)) {
        if self.depth >= MAX_DEPTH {
            return;
        }
        self.depth += 1;
        f(self, out);
        self.depth -= 1;
    }
}

/// Whether this writer writes everything `doc` holds that the exchange-flow writer would: all but
/// tracked changes (or Track Changes on), which only the exchange-flow writer writes yet.
pub(crate) fn can_write(doc: &Document) -> bool {
    doc.revisions.is_empty() && !doc.settings.track_changes && styles::names_unique(doc)
}

/// Write `doc` as an `.odt` package.
pub(crate) fn write(doc: &Document) -> Result<Vec<u8>, String> {
    let mut ctx = Ctx::new(doc);
    let plan = pages::plan(doc);
    let mut body = String::new();
    text::body(&mut ctx, &plan, &mut body);
    ctx.target = Target::Styles;
    let (layouts, masters) = pages::master_styles(&mut ctx, &plan);
    let common = styles::common(&mut ctx);
    let [content_auto, styles_auto] = std::mem::take(&mut ctx.auto);
    let fonts = package::font_decls(&ctx.fonts);
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {}>{fonts}<office:automatic-styles>{}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>\n",
        super::NS,
        content_auto.xml
    );
    let styles = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {}>{fonts}<office:styles>{common}</office:styles><office:automatic-styles>{layouts}{}</office:automatic-styles><office:master-styles>{masters}</office:master-styles></office:document-styles>\n",
        super::NS,
        styles_auto.xml
    );
    package::zip(doc, &content, &styles, &ctx.files)
}
