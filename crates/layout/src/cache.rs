use super::ctx::Outlines;
use super::placed::Placed;
use super::{
    Arc, Blocks, Document, Hash, HashMap, HashSet, Hasher, HideDeleted, InlineObject, LayoutOptions, Page, ParaLayout, Paragraph, SectionProps,
    StoryRef, ViewMode, fields, styleref,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub(crate) rev: u64,
    pub(crate) width: u32,
    pub(crate) label: Option<String>,
    pub(crate) page: Option<(u32, u32, u32, u32, u64, wordcraft_doc::section::NumFormat)>,
    /// The pages a split paragraph's fields are on after its first (`FieldCtx::page_from`).
    pub(crate) page_from: u64,
    pub(crate) table: u64,
    pub(crate) notes: u64,
    pub(crate) excl: u64,
    pub(crate) eq: u32,
    pub(crate) read_spacing: Option<u32>,
    /// The external checker's result shown in this paragraph ([`wordcraft_proof::external`]):
    /// a new result redraws only its own paragraph.
    pub(crate) proof: u64,
    pub(crate) grid: (u32, u32, u32, bool),
    pub(crate) fit: bool,
    /// Heights of the paragraph's inline boxes that resize to fit their text.
    pub(crate) fitted: u64,
    pub(crate) hide_deleted: HideDeleted,
    /// STYLEREF results in headers and footers ([`FieldCtx::styleref_key`]).
    pub(crate) sref: u64,
    /// The word being typed in this paragraph, left without a spelling squiggle.
    pub(crate) typing: Option<(usize, usize)>,
}

/// Memoised paragraph layouts.
#[derive(Default)]
pub struct LayoutCache {
    pub(crate) paras: HashMap<Key, Arc<ParaLayout>>,
    /// Narrowest and widest content of paragraphs measured for table autofit ([`Ctx::para_measured`]).
    pub(crate) measures: HashMap<Key, (f32, f32)>,
    /// Outlines of pictures' opaque pixels (Tight and Through wrapping), by media key, the
    /// media's bytes (address and length) and crop. Kept from one layout to the next, so each
    /// entry holds a weak reference to its bytes: their allocation isn't freed while the entry is
    /// kept, so no other picture's bytes can come to have its address ([`Outlines`]).
    pub(crate) outlines: Outlines,
    pub(crate) env: u64,
    pub(crate) used: HashSet<Key>,
    /// The STYLEREF style index of the last pass and what it was built from: the body blocks
    /// (shared, so an edited block is a different pointer) and `env` (styles and numbering). Kept
    /// while both are unchanged, so headers using STYLEREF don't rebuild it on every pass.
    pub(crate) styleref: Option<(Blocks, u64, Arc<styleref::StyleIndex>)>,
    pub hits: u64,
    pub misses: u64,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.paras.len()
    }
    pub fn is_empty(&self) -> bool {
        self.paras.is_empty()
    }
    pub fn clear(&mut self) {
        self.paras.clear();
        self.measures.clear();
        self.outlines.clear();
        self.styleref = None;
    }

    /// The STYLEREF style index for `doc`, rebuilt only when the body or the styles changed.
    pub(crate) fn style_index(&mut self, doc: &Document) -> Arc<styleref::StyleIndex> {
        if let Some((body, env, ix)) = &self.styleref
            && *env == self.env
            && body.len() == doc.body.len()
            && body.iter().zip(doc.body.iter()).all(|(a, b)| Arc::ptr_eq(a, b))
        {
            return ix.clone();
        }
        let ix = Arc::new(styleref::StyleIndex::build(doc));
        self.styleref = Some((doc.body.to_vec(), self.env, ix.clone()));
        ix
    }
}

pub(crate) fn hash_of<T: Hash>(t: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

pub(crate) fn env_hash(doc: &Document, opts: &LayoutOptions) -> u64 {
    let s = serde_json::to_string(&(&doc.styles, &doc.numbering, doc.settings.default_tab, &doc.settings.footnote_format)).unwrap_or_default();
    // Outline view lays paragraphs out changed (no indents; plain without text formatting).
    let outline = (opts.view == ViewMode::Outline).then(|| opts.outline.as_ref().is_some_and(|o| o.plain));
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    opts.show_hidden.hash(&mut h);
    opts.hide_deleted.hash(&mut h);
    opts.hide_marked.hash(&mut h);
    opts.deletions_in_balloons.hash(&mut h);
    opts.proofing.hash(&mut h);
    opts.numeral.hash(&mut h);
    opts.field_codes.hash(&mut h);
    opts.proof.hash(&mut h);
    wordcraft_proof::dictionary_generation().hash(&mut h);
    hyphenation_key(doc).hash(&mut h);
    outline.hash(&mut h);
    opts.filename.hash(&mut h);
    opts.filepath.hash(&mut h);
    h.finish()
}

/// The hyphenation settings, hashable (the zone as its bits).
fn hyphenation_key(doc: &Document) -> (bool, u32, bool, u32) {
    let s = &doc.settings;
    (s.auto_hyphenation, s.hyphenation_zone.to_bits(), s.hyphenate_caps, s.consecutive_hyphen_limit)
}

pub(crate) fn has_page_fields(p: &Paragraph) -> bool {
    p.objects.iter().any(|o| match o {
        InlineObject::Field { instr, .. } => {
            matches!(fields::field_name(instr).as_str(), "PAGE" | "NUMPAGES" | "SECTIONPAGES" | "SECTION" | "PAGEREF")
        }
        _ => false,
    })
}

/// Does `p` show a PAGE field, whose result is the number of the page it lands on?
pub(crate) fn has_page_number_fields(p: &Paragraph) -> bool {
    p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if fields::field_name(instr) == "PAGE"))
}

/// Does `p` show a page total, which depends on how many pages the document has?
pub(crate) fn has_total_fields(p: &Paragraph) -> bool {
    p.objects.iter().any(|o| match o {
        InlineObject::Field { instr, .. } => matches!(fields::field_name(instr).as_str(), "NUMPAGES" | "SECTIONPAGES"),
        _ => false,
    })
}

/// Page totals: of the document, and of each section (by index).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Totals {
    pub(crate) pages: u32,
    sections: Vec<u32>,
}

impl Totals {
    /// What a first pass assumes: one page.
    pub(crate) fn first_guess(sections: usize) -> Totals {
        Totals { pages: 1, sections: vec![1; sections] }
    }
    /// Count each section on every page containing its body, including a shared continuous
    /// start page. The page owner still selects headers and footers; empty sections count one.
    pub(crate) fn of(pages: &[Page], sections: &[(usize, &SectionProps)]) -> Totals {
        let mut per = vec![0u32; sections.len()];
        for page in pages {
            let mut present = HashSet::from([page.section]);
            for item in &page.items {
                let path = match item {
                    Placed::Lines { story: StoryRef::Body, path, .. }
                    | Placed::Image { story: StoryRef::Body, path, .. }
                    | Placed::Graphic { story: StoryRef::Body, path, .. }
                    | Placed::Object { story: StoryRef::Body, path, .. }
                    | Placed::Cell { story: StoryRef::Body, table: path, .. } => path,
                    _ => continue,
                };
                let Some(&block) = path.0.first() else { continue };
                present.insert(sections.partition_point(|(end, _)| *end < block as usize));
            }
            for section in present {
                if let Some(n) = per.get_mut(section) {
                    *n = n.saturating_add(1);
                }
            }
        }
        for n in &mut per {
            *n = (*n).max(1);
        }
        Totals { pages: u32::try_from(pages.len()).unwrap_or(u32::MAX).max(1), sections: per }
    }
    pub(crate) fn section(&self, si: usize) -> u32 {
        self.sections.get(si).copied().unwrap_or(1)
    }
}

/// The most extra layout passes for page totals in the body: a total that changes the page
/// count (its digits pushing text to another page) is tried again, this many times at most.
pub(crate) const MAX_TOTAL_PASSES: usize = 3;
