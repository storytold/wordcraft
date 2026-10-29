use super::cache::{Key, LayoutCache, Totals, has_page_fields, has_total_fields, hash_of};
use super::floats::{EarlyFloats, WrapArea};
use super::headers::HfLinks;
use super::placed::Placed;
use super::{
    Anchor, Arc, Block, CharProps, Counters, DocLayout, Document, FieldCtx, Float, HashMap, HashSet, HideDeleted, InlineObject, LayoutOptions, Level,
    ParaLayout, Paragraph, Rect, Rgb, SectionProps, StoryRef, ViewMode, Wrap, contour, math, para, styleref, textbox,
};

/// A layout pass: finished, or to be done again with the page totals it found, or with the
/// floats placed above their anchors it found ([`EarlyFloats`]).
pub(crate) enum Pass {
    Done(DocLayout),
    Again(Totals),
    Rewrap(EarlyFloats),
}

/// Walk state while paginating.
pub(crate) struct Ctx<'a> {
    pub(crate) doc: &'a Document,
    pub(crate) cache: &'a mut LayoutCache,
    pub(crate) opts: &'a LayoutOptions,
    pub(crate) counters: Counters,
    pub(crate) fields: FieldCtx,
    pub(crate) notes_hash: u64,
    pub(crate) numbers: HashMap<u32, Arc<ParaLayout>>,
    /// Automatic equation numbers so far.
    pub(crate) eq_count: u32,
    /// Bounds laying out text boxes inside text boxes, for the whole layout.
    pub(crate) boxes: wordcraft_doc::BoxBudget,
    /// Linked text boxes (Create Link).
    pub(crate) chains: textbox::Chains,
    /// The current section's document grid (body text only: none for headers and footers).
    pub(crate) grid: para::Grid,
    /// Whether tracked deletions are left out in the story being laid out
    /// ([`LayoutOptions::hides_deleted`]): the body's, or the story of the [`layout_box`] at work.
    pub(crate) hide_deleted: HideDeleted,
    /// Whether body text (or its notes) laid out so far shows a page total (`NUMPAGES`,
    /// `SECTIONPAGES`), which is only known once the whole body is paginated.
    pub(crate) uses_totals: bool,
    /// Whether to note `uses_totals` now (off while laying out headers and footers).
    pub(crate) track_totals: bool,
    /// The width a top-level table's percentage width is a share of, when it isn't the box's:
    /// an inherited header or footer's tables follow the section that defines it.
    pub(crate) pct_base: Option<f32>,
    /// Empty body paragraphs with hidden marks or continuous section breaks, in order:
    /// they take no room (see [`place_para`]).
    pub(crate) quiet_marks: Vec<usize>,
    /// Empty paragraphs closing a continuous section right after another section's mark, in
    /// order: each takes its line while it fits, then stays at the page's bottom (#3907).
    pub(crate) stacked_marks: Vec<usize>,
    /// Whether even pages are mirrored (mirrored margins, on pages): see [`mirror_even_pages`].
    pub(crate) mirror: bool,
    /// Cache-key hashes of the table-style text formatting cells were laid out with
    /// ([`Ctx::table_key`]).
    pub(crate) table_keys: Vec<(para::CellText, u64)>,
    /// While a header or footer with tables is laid out: its page, for floats in its cells placed
    /// on the page (see [`with_pin`]).
    pub(crate) pin: Option<Pin>,
    pub(crate) footer_wraps: HashMap<(usize, u32, bool), Vec<WrapArea>>,
    pub(crate) header_tables: HashMap<(usize, u32, bool), Vec<Rect>>,
    /// Heights of text boxes that resize to fit their text, by (story, width), worked out once
    /// per layout ([`textbox::fitted_height`]).
    pub(crate) fitted: HashMap<(u32, u32), Option<f32>>,
    /// Each body float's latest placement: on which page and with what wrap area, when it is
    /// above its anchor paragraph with text before it ([`EarlyFloats`]).
    pub(crate) floats_above: HashMap<(usize, usize), Option<(usize, WrapArea)>>,
    /// The document's sections with their headers and footers linked ([`HfLinks`]).
    pub(crate) hf: Arc<HfLinks<'a>>,
    /// Pictures' transparency, decoded for Tight and Through wrapping ([`Ctx::wrap_outline`]).
    pub(crate) alphas: contour::Alphas,
}

struct ParaEnvInput<'a> {
    width: f32,
    label: Option<(String, Level)>,
    table: Option<&'a para::CellText>,
    exclusions: &'a [para::Exclusion],
    eq_number: u32,
    typing_word: Option<(usize, usize)>,
    fitted: &'a [(usize, f32)],
}

/// A header or footer (`story`) laid out for page `number` of a section (`sect`, `mirror`): its
/// table cells' floats placed on the page rather than in their cell ([`Float::outside_cell`],
/// Word's picture watermark in a header that starts with a table), drawn behind the header's
/// text (`behind`) or in front, in page coordinates. Once each (`seen`): a cell may be laid out
/// more than once.
pub(crate) struct Pin {
    pub(crate) story: StoryRef,
    pub(crate) sect: SectionProps,
    pub(crate) number: u32,
    pub(crate) mirror: bool,
    pub(crate) behind: Vec<Placed>,
    pub(crate) front: Vec<Placed>,
    pub(crate) seen: HashSet<(Vec<u32>, usize)>,
}

/// Whether a float's position on both axes is measured on the page (from its edges or margins),
/// not from its column, paragraph, character or line: one that can be placed on the page from
/// anywhere.
pub(crate) fn page_anchored(f: &Float) -> bool {
    use Anchor::*;
    matches!(f.h_rel, Page | Margin | LeftMargin | RightMargin | InsideMargin | OutsideMargin)
        && matches!(f.v_rel, Page | Margin | TopMargin | BottomMargin | InsideMargin | OutsideMargin)
}

/// Run `f`, a layout of header or footer `id` for page `number` of `sect`, with the floats in its
/// table cells that are placed on the page put there (see [`Pin`]). Returns `f`'s result and those
/// floats' items, in page coordinates: behind the header's text, and in front.
pub(crate) fn with_pin<T>(ctx: &mut Ctx, id: u32, sect: &SectionProps, number: u32, f: impl FnOnce(&mut Ctx) -> T) -> (T, Vec<Placed>, Vec<Placed>) {
    let tables = ctx.doc.parts.get(&id).is_some_and(|p| p.blocks.iter().any(|b| matches!(**b, Block::Table(_))));
    if !tables {
        return (f(ctx), Vec::new(), Vec::new());
    }
    let pin = Pin {
        story: StoryRef::Part(id),
        sect: sect.clone(),
        number,
        mirror: ctx.mirror,
        behind: Vec::new(),
        front: Vec::new(),
        seen: HashSet::new(),
    };
    let outer = ctx.pin.replace(pin);
    let out = f(ctx);
    let pin = std::mem::replace(&mut ctx.pin, outer);
    let (behind, front) = pin.map(|p| (p.behind, p.front)).unwrap_or_default();
    (out, behind, front)
}

/// Picture outlines by media name, bytes (address and length) and crop ([`LayoutCache`]), each
/// with a weak reference to those bytes. The cache outlives the documents it is used with, and
/// an address names the same bytes only while their allocation lives: the weak reference keeps
/// it (not the bytes in it), so a picture loaded later can't be given a freed picture's address
/// and length and be taken for it.
pub(crate) type Outlines = HashMap<(String, usize, usize, [u32; 4]), (std::sync::Weak<Vec<u8>>, Option<Arc<Vec<(f32, f32)>>>)>;

/// Picture `bytes`'s outline within `crop`, kept in `outlines` under `key`. One refused for this
/// layout's decoding budget wraps as its rectangle and isn't kept: a later layout may decode it.
pub(crate) fn picture_outline(
    outlines: &mut Outlines,
    alphas: &mut contour::Alphas,
    key: (String, usize, usize, [u32; 4]),
    bytes: &Arc<Vec<u8>>,
    crop: [f32; 4],
) -> Option<Arc<Vec<(f32, f32)>>> {
    if let Some((_, o)) = outlines.get(&key) {
        return o.clone();
    }
    let outline = alphas.outline(bytes, crop).ok()?.map(Arc::new);
    if outlines.len() >= 256 {
        outlines.clear();
    }
    outlines.insert(key, (Arc::downgrade(bytes), outline.clone()));
    outline
}

impl Ctx<'_> {
    /// The outline Tight and Through wrapping follow around `o` (see [`contour::wrap_outline`]);
    /// pictures' are worked out once per picture and crop.
    pub(crate) fn wrap_outline(&mut self, o: &InlineObject) -> Option<Arc<Vec<(f32, f32)>>> {
        let (doc, cache, alphas) = (self.doc, &mut *self.cache, &mut self.alphas);
        contour::wrap_outline(o, |media, crop| {
            let bytes = doc.media.get(media)?;
            let key = (media.to_string(), Arc::as_ptr(bytes) as usize, bytes.len(), crop.map(f32::to_bits));
            picture_outline(&mut cache.outlines, alphas, key, bytes, crop)
        })
    }

    /// A laid-out line number (cached).
    pub(crate) fn number_para(&mut self, n: u32) -> Arc<ParaLayout> {
        if let Some(p) = self.numbers.get(&n) {
            return p.clone();
        }
        let mut p = Paragraph::with_text(
            &n.to_string(),
            CharProps { size: Some(10.0), color: Some(wordcraft_doc::TextColor::Rgb(Rgb(0x60, 0x60, 0x60))), ..Default::default() },
        );
        p.props.space_after = Some(0.0);
        p.props.space_before = Some(0.0);
        let env = para::ParaEnv {
            doc: self.doc,
            width: 60.0,
            label: None,
            fields: &self.fields,
            show_hidden: false,
            field_codes: false,
            hide_deleted: Default::default(),
            table: None,
            proofing: None,
            typing_word: None,
            numeral: self.opts.numeral,
            exclusions: &[],
            eq_number: 0,
            read_line_spacing: None,
            grid: para::Grid::default(),
            fit_objects: false,
            fitted: &[],
        };
        let pl = Arc::new(para::layout_para(&p, &env));
        self.numbers.insert(n, pl.clone());
        pl
    }

    /// The list label of `p`, advancing the list counters: call it once per paragraph, in
    /// document order (only for paragraphs that will show).
    /// Whether `p` gets a list label ([`Ctx::next_label`] is `Some`), without counting it.
    pub(crate) fn has_label(&self, p: &Paragraph) -> bool {
        p.props
            .numbering
            .or_else(|| self.doc.styles.resolve_para(&p.props).numbering)
            .filter(|n| n.num != 0)
            .is_some_and(|n| self.doc.numbering.shows_label(n.num, n.level))
    }
    pub(crate) fn next_label(&mut self, p: &Paragraph) -> Option<(String, Level)> {
        let n = p.props.numbering.or_else(|| self.doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0)?;
        self.counters.next_label(&self.doc.numbering, n.num, n.level)
    }

    /// Lay out `p` with its already counted `label`, so laying a paragraph out again (around
    /// floating objects, or after it moved) doesn't advance the list counters twice.
    pub(crate) fn para_labelled(
        &mut self,
        p: &Paragraph,
        width: f32,
        table: Option<&para::CellText>,
        exclusions: &[para::Exclusion],
        label: Option<(String, Level)>,
    ) -> Arc<ParaLayout> {
        let fitted = self.fitted_boxes(p);
        let (key, eq_number) = self.para_key(p, width, table, exclusions, label.as_ref(), &fitted);
        self.cache.used.insert(key.clone());
        if let Some(pl) = self.cache.paras.get(&key) {
            self.cache.hits += 1;
            return pl.clone();
        }
        self.cache.misses += 1;
        let typing = key.typing;
        let pl = Arc::new(para::layout_para(
            p,
            &self.para_env(ParaEnvInput { width, label, table, exclusions, eq_number, typing_word: typing, fitted: &fitted }),
        ));
        self.cache.paras.insert(key, pl.clone());
        pl
    }

    /// `p` laid out on its own, as the paragraph after a kept one is measured before it is placed
    /// (no wrap areas, proofing, typed word or resized text boxes), through the cache when
    /// [`Ctx::para_labelled`] would lay it out with those same inputs: it is then not laid out a
    /// second time when it is placed. Counts nothing (list labels, equation numbers, page totals).
    /// `None` when the inputs differ.
    pub(crate) fn para_ahead(&mut self, p: &Paragraph, width: f32, label: Option<(String, Level)>) -> Option<Arc<ParaLayout>> {
        let resized = p
            .objects
            .iter()
            .any(|o| matches!(o, InlineObject::Shape { story: Some(_), float, .. } if float.wrap == Wrap::Inline && float.rot == 0.0));
        if self.opts.proofing || self.boxes.inside() || resized || self.hide_deleted != self.opts.hides_deleted(StoryRef::Body) {
            return None;
        }
        let kept = (self.eq_count, self.uses_totals);
        let (key, eq_number) = self.para_key(p, width, None, &[], label.as_ref(), &[]);
        (self.eq_count, self.uses_totals) = kept;
        if key.typing.is_some() {
            return None;
        }
        self.cache.used.insert(key.clone());
        if let Some(pl) = self.cache.paras.get(&key) {
            self.cache.hits += 1;
            return Some(pl.clone());
        }
        self.cache.misses += 1;
        let pl = Arc::new(para::layout_para(
            p,
            &self.para_env(ParaEnvInput { width, label, table: None, exclusions: &[], eq_number, typing_word: None, fitted: &[] }),
        ));
        self.cache.paras.insert(key, pl.clone());
        Some(pl)
    }

    /// What `measure` reads from `p` laid out as [`Ctx::para_labelled`] would, remembering only
    /// the result: a table's columns measure every cell once at an unbounded width, and keeping
    /// those layouts would double what a long table holds (and walk them all again on each pass).
    pub(crate) fn para_measured(
        &mut self,
        p: &Paragraph,
        width: f32,
        table: Option<&para::CellText>,
        label: Option<(String, Level)>,
        measure: impl FnOnce(&ParaLayout) -> (f32, f32),
    ) -> (f32, f32) {
        let fitted = self.fitted_boxes(p);
        let (key, eq_number) = self.para_key(p, width, table, &[], label.as_ref(), &fitted);
        self.cache.used.insert(key.clone());
        if let Some(m) = self.cache.measures.get(&key) {
            self.cache.hits += 1;
            return *m;
        }
        self.cache.misses += 1;
        let typing = key.typing;
        let m = measure(&para::layout_para(
            p,
            &self.para_env(ParaEnvInput { width, label, table, exclusions: &[], eq_number, typing_word: typing, fitted: &fitted }),
        ));
        self.cache.measures.insert(key, m);
        m
    }

    /// The heights of `p`'s inline text boxes that resize to fit their text, by object index
    /// ([`para::ParaEnv::fitted`]).
    fn fitted_boxes(&mut self, p: &Paragraph) -> Vec<(usize, f32)> {
        let mut out = Vec::new();
        for (k, o) in p.objects.iter().enumerate() {
            let InlineObject::Shape { story: Some(id), w, float, .. } = o else { continue };
            if float.wrap != Wrap::Inline || float.rot != 0.0 {
                continue;
            }
            let key = (*id, w.to_bits());
            let h = match self.fitted.get(&key) {
                Some(h) => *h,
                None => {
                    let h = textbox::fitted_height(self, *id, *w);
                    self.fitted.insert(key, h);
                    h
                }
            };
            if let Some(h) = h {
                out.push((k, h));
            }
        }
        out
    }

    /// The cache key of `p` laid out with these inputs, and its first automatic equation number
    /// (advancing the count past its equations).
    fn para_key(
        &mut self,
        p: &Paragraph,
        width: f32,
        table: Option<&para::CellText>,
        exclusions: &[para::Exclusion],
        label: Option<&(String, Level)>,
        fitted: &[(usize, f32)],
    ) -> (Key, u32) {
        let page = if has_page_fields(p) { Some(self.fields.page_key()) } else { None };
        if self.track_totals && !self.uses_totals && has_total_fields(p) {
            self.uses_totals = true;
        }
        // Automatic equation numbers count through the document.
        let eq_here: u32 = p
            .objects
            .iter()
            .map(|o| match o {
                InlineObject::Equation { math, display, .. } => math::auto_numbers(math, *display),
                _ => 0,
            })
            .sum();
        let eq_number = self.eq_count;
        self.eq_count = self.eq_count.saturating_add(eq_here);
        let key = Key {
            rev: p.rev,
            width: width.to_bits(),
            label: label.map(|(t, l)| format!("{t}|{}|{}|{:?}|{:?}|{:?}|{:?}", l.indent, l.hanging, l.suffix, l.align, l.tab, l.chr)),
            page_from: if page.is_some() && !self.fields.page_from.is_empty() { hash_of(&self.fields.page_from) } else { 0 },
            page,
            table: table.map(|t| self.table_key(t)).unwrap_or(0),
            notes: if p.objects.iter().any(|o| matches!(o, InlineObject::NoteRef { .. })) { self.notes_hash } else { 0 },
            excl: if exclusions.is_empty() { 0 } else { hash_of(&format!("{exclusions:?}")) },
            eq: if eq_here > 0 { eq_number } else { 0 },
            read_spacing: if self.opts.view == ViewMode::Read { self.opts.read_line_spacing.map(f32::to_bits) } else { None },
            proof: if self.opts.proofing && !wordcraft_proof::external::is_empty() && !p.text.trim().is_empty() {
                wordcraft_proof::external::stamp(&para::proof_text(p), &para::external_language(self.doc, p, table.map(|t| &t.chr)))
            } else {
                0
            },
            grid: self.grid.key(),
            fit: self.boxes.inside(),
            fitted: if fitted.is_empty() { 0 } else { hash_of(&format!("{fitted:?}")) },
            hide_deleted: self.hide_deleted,
            sref: if self.fields.styleref.is_some() && styleref::has_styleref(p) { self.fields.styleref_key(p) } else { 0 },
            typing: self.opts.typing_word.filter(|t| t.rev == p.rev).map(|t| (t.start, t.end)),
        };
        (key, eq_number)
    }

    /// The cache-key hash of a cell's table-style text formatting. Hashing it means formatting it
    /// as text, which a long table would otherwise do for each of its paragraphs on every pass; a
    /// document's tables only give their text a few different formats.
    fn table_key(&mut self, t: &para::CellText) -> u64 {
        if let Some((_, h)) = self.table_keys.iter().find(|(c, _)| c == t) {
            return *h;
        }
        let h = hash_of(&format!("{t:?}"));
        if self.table_keys.len() >= 64 {
            self.table_keys.clear();
        }
        self.table_keys.push((t.clone(), h));
        h
    }

    fn para_env<'b>(&'b self, input: ParaEnvInput<'b>) -> para::ParaEnv<'b> {
        let ParaEnvInput { width, label, table, exclusions, eq_number, typing_word, fitted } = input;
        para::ParaEnv {
            doc: self.doc,
            width,
            label,
            fields: &self.fields,
            show_hidden: self.opts.show_hidden,
            field_codes: self.opts.field_codes,
            hide_deleted: self.hide_deleted,
            table,
            proofing: self.opts.proofing.then_some(self.opts.proof),
            typing_word,
            numeral: self.opts.numeral,
            exclusions,
            eq_number,
            read_line_spacing: if self.opts.view == ViewMode::Read { self.opts.read_line_spacing } else { None },
            grid: self.grid,
            fit_objects: self.boxes.inside(),
            fitted,
        }
    }
}

/// Lay out with percentage-width tables sized by `base` (see [`HfLinks::defining_width`]).
pub(crate) fn with_pct_base<T>(ctx: &mut Ctx, base: Option<f32>, f: impl FnOnce(&mut Ctx) -> T) -> T {
    let before = std::mem::replace(&mut ctx.pct_base, base);
    let out = f(ctx);
    ctx.pct_base = before;
    out
}
