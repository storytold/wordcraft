//! Which clip paths, masks, patterns, filters, markers and gradients an SVG's own `url(#…)` and
//! `href` links make refer to each other (#1910 review 6). usvg builds each of them by building
//! what they hold, and recurses without limit when that leads back to them, through more than
//! one hop or across kinds: a stack overflow, which aborts the process. This builds the graph of
//! those references (without recursion, and bounded) so the pre-check can refuse a loop, and can
//! count how often each one is built along the graph.
//!
//! The graph reads references more widely than usvg does (review 8), so that whichever way usvg
//! reads one, the edge is there. What usvg follows, and the edge that covers it:
//! - a property naming `url(#id)` (`fill`, `stroke`, `clip-path`, `mask`, `filter`, `marker-*`):
//!   every `url(` in every attribute, in any namespace and case, with spaces before the bracket,
//!   quotes, and each way the id may end ([`each_url`]);
//! - the same set by a style sheet rule or a `style` attribute: rules matched with usvg's own
//!   matcher, `style` read with its own tokenizer (a `url(` in text it skips counts as a property
//!   that passes down);
//! - a property inherited from an ancestor (paint and markers), and `inherit` (which also takes a
//!   clip path, mask or filter from the parent): every ancestor of the element built, inheritable
//!   properties only unless the file says `inherit` anywhere, then all of them;
//! - `href` (`<use>` copies, filter images, pattern, gradient and filter templates, text paths):
//!   every `href` of every element built is followed, in any namespace, and the target's own
//!   ancestors are read as above (a filter image builds its target where it stands). Not
//!   followed: a hyperlink (`<a>`), and a `<use>` or filter image of a clip path, mask, pattern,
//!   filter, marker or gradient, which usvg doesn't draw (only a special's own `href`, its
//!   template, leads to another);
//! - an `id`: every attribute called `id`, in any namespace, names its element.
//!
//! The only edge left out is the one usvg itself removes: a special's own fill, stroke, clip
//! path, mask or filter naming itself, in what it holds (not through a link or inheritance).

use std::collections::{HashMap, HashSet};

use super::SVG_NS;

type Node<'a, 'i> = usvg::roxmltree::Node<'a, 'i>;

/// Most clip paths, masks, patterns, filters and markers a picture may have.
const MAX_SPECIALS: usize = 1 << 19;
/// Most references between them.
const MAX_EDGES: usize = 1 << 22;
/// Most elements, links and style rules looked at while finding the references.
const MAX_STEPS: u64 = 1 << 25;
/// Most combinators one selector may have (the matcher recurses along them).
const MAX_COMBINATORS: usize = 32;
/// Most matching work the style sheets may cost: for each selector, the elements matched times
/// the ancestors it may try for each descendant combinator (about a second of usvg's matching).
pub(crate) const MAX_MATCH_WORK: u64 = 1 << 28;
/// Most characters in the style sheets that could be combinators (white space runs, `+`, `>`), a
/// backstop to the selectors measured: the matcher's recursion stays within the reader's stack.
const MAX_COMBINATOR_CHARS: u64 = 1 << 20;
/// Most rules naming a `url(` that are matched.
const MAX_URL_RULES: usize = 4096;
/// Most markers one loop may go through (usvg builds a marker inside itself at most once, so a
/// loop through k markers can be built k! ways; each is counted).
const MAX_LOOP_MARKERS: u32 = 6;
/// Most states the count walks through (each special, with the markers of its loop it is inside).
const MAX_STATES: usize = 1 << 21;
/// Longest chain of references usvg may follow, one inside another.
const MAX_HOPS: u32 = 32;

/// Why no graph was made.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Some of them lead back to themselves.
    Loop,
    /// More than the bounds above.
    TooBig,
    /// A style sheet selector that matching would recurse or backtrack too far on.
    Selectors,
    /// A chain of references longer than [`MAX_HOPS`].
    Deep,
}

/// The clip paths, masks, patterns, filters, markers and gradients of a picture, and who refers
/// to whom.
pub(crate) struct Graph<'a> {
    /// Each one's node id in the XML document.
    pub nodes: Vec<usize>,
    /// Whether each is a marker (drawn per vertex rather than per element).
    pub marker: Vec<bool>,
    /// The ways usvg may reach each one: a special with the markers of its loop that are being
    /// built around it. Every path through them is a way usvg may build it.
    pub states: States,
    /// Whether the file says `inherit` anywhere (then a clip path, mask or filter may pass from
    /// an element to what it holds, like paint).
    pub inherit: bool,
    /// The specials each id may name.
    by_id: HashMap<&'a str, Vec<u32>>,
    /// The lengths of the ids in the file.
    lengths: HashSet<usize>,
    /// The style sheet rules that name a `url(`.
    rules: Vec<Rule<'a>>,
    /// What [`Graph::urls_of`] gives for each element that names a `url(`, by node id (matching
    /// is done once per element, however many specials reach it; review 9 I-5).
    urls: HashMap<usize, Vec<(&'a str, Vec<&'a str>)>>,
}

/// The states of [`Graph::states`]: a graph without loops.
#[derive(Default)]
pub(crate) struct States {
    /// The special each state is.
    pub of: Vec<u32>,
    /// The states each one refers to.
    pub edges: Vec<Vec<u32>>,
    /// Every state after all that refer to it.
    pub order: Vec<u32>,
    /// The state each special is in when the picture itself names it.
    pub entry: Vec<u32>,
}

fn is_special(n: Node) -> bool {
    n.is_element()
        && matches!(n.tag_name().namespace(), None | Some(SVG_NS))
        && matches!(n.tag_name().name(), "marker" | "clipPath" | "mask" | "pattern" | "filter" | "linearGradient" | "radialGradient")
}

/// The properties an element passes on to what it holds (and a `url(` with no property read,
/// in `style` text the CSS reader skipped).
fn inherited(name: &str) -> bool {
    matches!(name, "fill" | "stroke" | "marker-start" | "marker-mid" | "marker-end" | "marker" | "")
}

/// The `id`s an element may go by: every attribute called `id`, in any namespace (usvg reads the
/// first one in the SVG, XLink or XML namespace or none).
pub(crate) fn ids<'a>(e: Node<'a, '_>) -> impl Iterator<Item = &'a str> {
    e.attributes().filter(|a| a.name() == "id").map(|a| a.value())
}

/// Each `url(#…)` in `v`, read more widely than svgtypes reads one: `url` in any case, white
/// space before and inside the bracket, quotes. `f` gets the ids it may name: the text after `#`
/// up to each place svgtypes may end it (a bracket, a quote, a space or other white space), and
/// each trimmed; only those as long as some id in the file (`lengths`), as no other names
/// anything. (Checking the length first keeps this linear: a long id is only ever hashed once
/// per length.)
pub(crate) fn each_url<'s>(v: &'s str, lengths: &HashSet<usize>, mut f: impl FnMut(&[&'s str])) {
    const END_KINDS: usize = 5;
    // The places an id may end: a bracket, a space, a quote, any white space, a bracket or space.
    let is_end = |kind: usize, c: char| match kind {
        0 => c == ')',
        1 => c == ' ',
        2 => c == '"' || c == '\'',
        3 => c.is_whitespace(),
        _ => c == ')' || c == ' ',
    };
    // Per kind: not looked for yet, or the next one found (None: none to the end).
    let mut next_end: [Option<Option<usize>>; END_KINDS] = [None; END_KINDS];
    let b = v.as_bytes();
    let mut i = 0usize;
    while let Some(p) = b.get(i..).and_then(|r| r.windows(3).position(|w| w.eq_ignore_ascii_case(b"url"))) {
        let mut j = i + p + 3;
        i = j;
        while b.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
            j += 1;
        }
        if b.get(j) != Some(&b'(') {
            continue;
        }
        j += 1;
        while b.get(j).is_some_and(|c| c.is_ascii_whitespace() || *c == b'"' || *c == b'\'') {
            j += 1;
        }
        if b.get(j) != Some(&b'#') {
            continue;
        }
        let start = j + 1;
        let Some(rest) = v.get(start..) else { continue };
        // Where each kind of end comes next. The `url(`s are read in order, so each search goes
        // on from where it found its last end: linear in all, however many `url(`s.
        let mut ends = [0usize; END_KINDS];
        for (k, (next, end)) in next_end.iter_mut().zip(ends.iter_mut()).enumerate() {
            if next.is_none_or(|n| n.is_some_and(|n| n < start)) {
                *next = Some(rest.find(|c: char| is_end(k, c)).map(|p| start + p));
            }
            *end = next.flatten().map_or(rest.len(), |n| n.saturating_sub(start));
        }
        // Every one is the start of `rest`: the same length, the same id.
        let mut found: Vec<&str> = Vec::with_capacity(2 * END_KINDS);
        for e in ends {
            let Some(id) = rest.get(..e) else { continue };
            for id in [id, id.trim_end()] {
                if lengths.contains(&id.len()) && !found.iter().any(|f| f.len() == id.len()) {
                    found.push(id);
                }
            }
        }
        f(&found);
    }
}

/// Whether `v` holds `inherit` (any case).
pub(crate) fn says_inherit(v: &str) -> bool {
    v.as_bytes().windows(7).any(|w| w.eq_ignore_ascii_case(b"inherit"))
}

/// An element, for the CSS selector matcher.
#[derive(Clone, Copy)]
struct Css<'a, 'i>(Node<'a, 'i>);

impl simplecss::Element for Css<'_, '_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.parent_element().map(Css)
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling_element().map(Css)
    }
    fn has_local_name(&self, name: &str) -> bool {
        self.0.tag_name().name() == name
    }
    fn attribute_matches(&self, local_name: &str, operator: simplecss::AttributeOperator<'_>) -> bool {
        self.0.attribute(local_name).is_some_and(|v| operator.matches(v))
    }
    fn pseudo_class_matches(&self, class: simplecss::PseudoClass<'_>) -> bool {
        matches!(class, simplecss::PseudoClass::FirstChild) && self.0.prev_sibling_element().is_none()
    }
}

/// A style sheet rule that names a `url(`: for each such declaration, the property and, for each
/// `url(`, the ids it may name.
struct Rule<'a> {
    selector: simplecss::Selector<'a>,
    urls: Vec<(&'a str, Vec<Vec<&'a str>>)>,
}

impl<'a> Graph<'a> {
    /// The specials `id` may name.
    pub(crate) fn named_by(&self, id: &str) -> &[u32] {
        self.by_id.get(id).map_or(&[], |v| v.as_slice())
    }

    /// Each `url(` an element names itself, as usvg may read it: in its attributes (any
    /// namespace), its `style` (read with simplecss' tokenizer, as usvg reads it; a `url(` in
    /// text the tokenizer skips comes with no property name) and the style sheet rules that
    /// match it. `f` gets the property and the ids it may name.
    pub(crate) fn urls_of(&self, e: Node<'a, '_>, mut f: impl FnMut(&str, &[&'a str])) {
        for (name, ids) in self.urls.get(&e.id().get_usize()).into_iter().flatten() {
            f(name, ids);
        }
    }

    /// [`Graph::urls_of`], read from the element.
    fn read_urls(&self, e: Node<'a, '_>, mut f: impl FnMut(&'a str, &[&'a str])) {
        for a in e.attributes() {
            if a.name() == "href" {
                continue;
            }
            if a.name() == "style" {
                let mut read = 0usize;
                for d in simplecss::DeclarationTokenizer::from(a.value()) {
                    each_url(d.value, &self.lengths, |ids| {
                        read += 1;
                        f(d.name, ids);
                    });
                }
                // Text the tokenizer skipped names some: all of them, then, with no property.
                let mut all = 0usize;
                each_url(a.value(), &self.lengths, |_| all += 1);
                if all > read {
                    each_url(a.value(), &self.lengths, |ids| f("", ids));
                }
            } else {
                each_url(a.value(), &self.lengths, |ids| f(a.name(), ids));
            }
        }
        for r in &self.rules {
            if r.selector.matches(&Css(e)) {
                for (name, urls) in &r.urls {
                    urls.iter().for_each(|ids| f(name, ids));
                }
            }
        }
    }

    /// Whether `name`, set on an element, may pass to what the element holds.
    pub(crate) fn passes_down(&self, name: &str) -> bool {
        self.inherit || inherited(name)
    }
}

/// The text of elements called `style` in any namespace (usvg reads sheets by local name, the
/// first text in each; every text is read here, each on its own as usvg reads it).
pub(crate) fn style_texts<'a>(xml: &'a usvg::roxmltree::Document<'_>) -> Vec<&'a str> {
    xml.descendants()
        .filter(|e| e.is_element() && e.tag_name().name() == "style")
        .flat_map(|e| e.descendants())
        .filter_map(|d| d.text().filter(|_| d.is_text()))
        .collect()
}

/// The style sheets of `xml` as usvg reads them: every text, parsed by simplecss (which usvg
/// matches with), each from its own start.
fn sheet<'a>(texts: &[&'a str]) -> simplecss::StyleSheet<'a> {
    let mut sheet = simplecss::StyleSheet::new();
    for t in texts {
        sheet.parse_more(t);
    }
    sheet
}

/// Characters in a sheet that could be combinators: runs of white space, `+` and `>`, anywhere
/// (comments and strings included, so nothing the reader parses is missed).
fn combinator_chars(text: &str) -> u64 {
    let (mut n, mut space) = (0u64, false);
    for c in text.bytes() {
        let ws = c.is_ascii_whitespace();
        if (ws && !space) || c == b'+' || c == b'>' {
            n += 1;
        }
        space = ws;
    }
    n
}

/// The combinators of `selector`, and how many of them are descendant combinators. Measured on
/// the selector simplecss parsed, written out again and read back with its tokenizer; if that
/// fails, every space counts as both (only ever more).
fn combinators(selector: &simplecss::Selector<'_>) -> (usize, u32) {
    let text = selector.to_string();
    let (mut all, mut descendants) = (0usize, 0u32);
    for tok in simplecss::SelectorTokenizer::from(text.as_str()) {
        match tok {
            Ok(simplecss::SelectorToken::DescendantCombinator) => {
                all += 1;
                descendants = descendants.saturating_add(1);
            }
            Ok(simplecss::SelectorToken::ChildCombinator | simplecss::SelectorToken::AdjacentCombinator) => all += 1,
            Ok(_) => {}
            Err(_) => {
                let spaces = text.bytes().filter(|c| *c == b' ').count();
                return (spaces, u32::try_from(spaces).unwrap_or(u32::MAX));
            }
        }
    }
    (all, descendants)
}

/// Refuses style sheets whose selectors usvg's matcher (simplecss) would overflow the stack on
/// (it recurses once per combinator) or take forever on (it tries every ancestor for each
/// descendant combinator). The selectors measured are the ones simplecss itself parses out of
/// every `style` element, in any namespace (review 8: a hand-made cut of the text can disagree
/// with it); a count of the characters that could be combinators is a backstop. Returns the work
/// of matching every selector against an element, by the element's depth: usvg matches each
/// element it builds, `<use>` copies included, so the caller bounds it again once it knows how
/// many (review 9 I-5).
pub(crate) fn check_css(xml: &usvg::roxmltree::Document<'_>) -> Result<Vec<u64>, Refusal> {
    let texts = style_texts(xml);
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let raw = texts.iter().fold(0u64, |a, t| a.saturating_add(combinator_chars(t)));
    if raw > MAX_COMBINATOR_CHARS {
        return Err(Refusal::Selectors);
    }
    // Elements by depth: matching a selector against an element at depth d tries at most d
    // ancestors for each descendant combinator.
    let (mut depth, mut at_depth) = (HashMap::<usize, usize>::new(), Vec::<u64>::new());
    for n in xml.descendants().filter(|n| n.is_element()) {
        let d = n.parent().and_then(|p| depth.get(&p.id().get_usize()).copied()).map_or(1, |d| d + 1);
        depth.insert(n.id().get_usize(), d);
        if at_depth.len() <= d {
            at_depth.resize(d + 1, 0);
        }
        if let Some(k) = at_depth.get_mut(d) {
            *k += 1;
        }
    }
    // The work of matching every selector against one element, by its depth.
    let mut per_depth = vec![0u64; at_depth.len()];
    let mut work = 0u64;
    for rule in &sheet(&texts).rules {
        let (all, descendants) = combinators(&rule.selector);
        if all > MAX_COMBINATORS {
            return Err(Refusal::Selectors);
        }
        for (d, (w, n)) in per_depth.iter_mut().zip(&at_depth).enumerate() {
            let one = (d as u64).saturating_pow(descendants);
            *w = w.saturating_add(one);
            work = work.saturating_add(n.saturating_mul(one));
        }
        if work > MAX_MATCH_WORK {
            return Err(Refusal::Selectors);
        }
    }
    Ok(per_depth)
}

/// A depth-first walk with an explicit stack over `edges` (leaving out nodes `skip` names).
/// Returns the nodes in the order they finish, and the edges that lead back to a node on the
/// walk's path (each closes a loop).
fn walk(edges: &[Vec<u32>], skip: &dyn Fn(usize) -> bool) -> (Vec<u32>, Vec<(usize, u32)>) {
    let n = edges.len();
    let mut colour = vec![0u8; n]; // 0 new, 1 on the path, 2 done
    let (mut order, mut back) = (Vec::with_capacity(n), Vec::new());
    for root in 0..n {
        if skip(root) || colour.get(root) != Some(&0) {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        if let Some(c) = colour.get_mut(root) {
            *c = 1;
        }
        while let Some(&(v, k)) = stack.last() {
            match edges.get(v).and_then(|e| e.get(k)).copied() {
                Some(t) => {
                    if let Some(top) = stack.last_mut() {
                        top.1 = k + 1;
                    }
                    if skip(t as usize) {
                        continue;
                    }
                    match colour.get(t as usize).copied() {
                        Some(0) => {
                            if let Some(c) = colour.get_mut(t as usize) {
                                *c = 1;
                            }
                            stack.push((t as usize, 0));
                        }
                        Some(1) => back.push((v, t)),
                        _ => {}
                    }
                }
                None => {
                    if let Some(c) = colour.get_mut(v) {
                        *c = 2;
                    }
                    order.push(v as u32);
                    stack.pop();
                }
            }
        }
    }
    (order, back)
}

/// The strongly connected parts of `edges` (Tarjan's, with an explicit stack): each node's part.
fn parts(edges: &[Vec<u32>]) -> Vec<u32> {
    const NEW: u32 = u32::MAX;
    let n = edges.len();
    let (mut index, mut low, mut part) = (vec![NEW; n], vec![0u32; n], vec![NEW; n]);
    let mut on = vec![false; n];
    let (mut next, mut parts) = (0u32, 0u32);
    let (mut held, mut calls): (Vec<usize>, Vec<(usize, usize)>) = (Vec::new(), Vec::new());
    let enter = |v: usize, next: &mut u32, held: &mut Vec<usize>, index: &mut [u32], low: &mut [u32], on: &mut [bool]| {
        if let (Some(i), Some(l), Some(o)) = (index.get_mut(v), low.get_mut(v), on.get_mut(v)) {
            (*i, *l, *o) = (*next, *next, true);
        }
        *next += 1;
        held.push(v);
    };
    for root in 0..n {
        if index.get(root) != Some(&NEW) {
            continue;
        }
        enter(root, &mut next, &mut held, &mut index, &mut low, &mut on);
        calls.push((root, 0));
        while let Some(&(v, k)) = calls.last() {
            match edges.get(v).and_then(|e| e.get(k)).map(|t| *t as usize) {
                Some(t) => {
                    if let Some(top) = calls.last_mut() {
                        top.1 = k + 1;
                    }
                    if index.get(t) == Some(&NEW) {
                        enter(t, &mut next, &mut held, &mut index, &mut low, &mut on);
                        calls.push((t, 0));
                    } else if on.get(t) == Some(&true) {
                        let it = index.get(t).copied().unwrap_or(NEW);
                        if let Some(l) = low.get_mut(v) {
                            *l = (*l).min(it);
                        }
                    }
                }
                None => {
                    calls.pop();
                    let lv = low.get(v).copied().unwrap_or(0);
                    if let Some(&(p, _)) = calls.last()
                        && let Some(l) = low.get_mut(p)
                    {
                        *l = (*l).min(lv);
                    }
                    if Some(&lv) == index.get(v) {
                        while let Some(w) = held.pop() {
                            if let (Some(o), Some(c)) = (on.get_mut(w), part.get_mut(w)) {
                                (*o, *c) = (false, parts);
                            }
                            if w == v {
                                break;
                            }
                        }
                        parts += 1;
                    }
                }
            }
        }
    }
    part
}

/// The ways usvg may reach each special (see [`Graph::states`]). usvg never builds a marker
/// inside itself, so a loop has to go through a marker to be built at all, and then through
/// each marker of it once at most along any one path: a state is a special together with the
/// markers of its loop already being built around it. Refuses a loop without a marker, a loop
/// through more than [`MAX_LOOP_MARKERS`], and a chain longer than [`MAX_HOPS`]. Each list of
/// `edges` is sorted.
fn states(edges: &[Vec<u32>], marker: &[bool]) -> Result<States, Refusal> {
    let n = edges.len();
    let is_marker = |v: usize| marker.get(v) == Some(&true);
    // A loop that avoids the markers goes on without end.
    if !walk(edges, &is_marker).1.is_empty() {
        return Err(Refusal::Loop);
    }
    let part = parts(edges);
    let mut size: HashMap<u32, u32> = HashMap::new();
    for p in &part {
        *size.entry(*p).or_default() += 1;
    }
    let looping = |v: usize| {
        let p = part.get(v).copied().unwrap_or(u32::MAX);
        size.get(&p).is_some_and(|s| *s > 1) || edges.get(v).is_some_and(|e| e.binary_search(&(v as u32)).is_ok())
    };
    // Each looping marker's bit among its part's markers.
    let mut bit = vec![0u32; n];
    let mut markers_in: HashMap<u32, u32> = HashMap::new();
    for v in (0..n).filter(|v| is_marker(*v) && looping(*v)) {
        let k = markers_in.entry(part.get(v).copied().unwrap_or(u32::MAX)).or_default();
        if *k >= MAX_LOOP_MARKERS {
            return Err(Refusal::Loop);
        }
        if let Some(b) = bit.get_mut(v) {
            *b = 1 << *k;
        }
        *k += 1;
    }
    let mask_of = |v: usize| bit.get(v).copied().unwrap_or(0);
    let mut st = States::default();
    let mut at: HashMap<(u32, u32), u32> = HashMap::new();
    let mut add = |key: (u32, u32), st: &mut States| -> Result<u32, Refusal> {
        if let Some(i) = at.get(&key) {
            return Ok(*i);
        }
        if st.of.len() >= MAX_STATES {
            return Err(Refusal::TooBig);
        }
        let i = st.of.len() as u32;
        at.insert(key, i);
        st.of.push(key.0);
        st.edges.push(Vec::new());
        Ok(i)
    };
    for v in 0..n {
        let e = add((v as u32, mask_of(v)), &mut st)?;
        st.entry.push(e);
    }
    let mut masks: Vec<u32> = (0..n).map(mask_of).collect();
    let (mut i, mut total) = (0usize, 0usize);
    while let Some(u) = st.of.get(i).map(|u| *u as usize) {
        let held = masks.get(i).copied().unwrap_or(0);
        let mut out = Vec::new();
        for t in edges.get(u).into_iter().flatten().map(|t| *t as usize) {
            let mask = if part.get(t) != part.get(u) {
                mask_of(t)
            } else if mask_of(t) != 0 {
                if held & mask_of(t) != 0 {
                    // usvg doesn't build a marker inside itself.
                    continue;
                }
                held | mask_of(t)
            } else {
                held
            };
            let before = st.of.len();
            let s = add((t as u32, mask), &mut st)?;
            if st.of.len() > before {
                masks.push(mask);
            }
            out.push(s);
        }
        total = total.saturating_add(out.len());
        if total > MAX_EDGES {
            return Err(Refusal::TooBig);
        }
        if let Some(slot) = st.edges.get_mut(i) {
            *slot = out;
        }
        i += 1;
    }
    let (mut order, back) = walk(&st.edges, &|_| false);
    if !back.is_empty() {
        return Err(Refusal::Loop);
    }
    order.reverse();
    // The longest chain, along the order.
    let mut hops = vec![0u32; st.of.len()];
    for &x in &order {
        let h = hops.get(x as usize).copied().unwrap_or(0);
        if h > MAX_HOPS {
            return Err(Refusal::Deep);
        }
        for &t in st.edges.get(x as usize).into_iter().flatten() {
            if let Some(ht) = hops.get_mut(t as usize) {
                *ht = (*ht).max(h + 1);
            }
        }
    }
    st.order = order;
    Ok(st)
}

/// The graph for `xml`. It reads references more widely than usvg does, so that whichever way
/// usvg reads one, the loop is there to find (see the module docs).
pub(crate) fn graph<'a>(xml: &'a usvg::roxmltree::Document<'_>) -> Result<Graph<'a>, Refusal> {
    let all: Vec<Node> = xml.descendants().collect();
    let len = all.iter().map(|n| n.id().get_usize() + 1).max().unwrap_or(0);
    let mut special_at = vec![u32::MAX; len];
    let (mut nodes, mut marker): (Vec<usize>, Vec<bool>) = (Vec::new(), Vec::new());
    let mut specials: Vec<Node> = Vec::new();
    let mut by_id: HashMap<&str, Vec<Node>> = HashMap::new();
    let mut spec_by_id: HashMap<&str, Vec<u32>> = HashMap::new();
    let mut lengths = HashSet::new();
    for n in &all {
        let special = is_special(*n);
        if special {
            if specials.len() >= MAX_SPECIALS {
                return Err(Refusal::TooBig);
            }
            if let Some(s) = special_at.get_mut(n.id().get_usize()) {
                *s = specials.len() as u32;
            }
            nodes.push(n.id().get_usize());
            marker.push(n.tag_name().name() == "marker");
        }
        if n.is_element() {
            for id in ids(*n) {
                lengths.insert(id.len());
                by_id.entry(id).or_default().push(*n);
                if special {
                    spec_by_id.entry(id).or_default().push(specials.len() as u32);
                }
            }
        }
        if special {
            specials.push(*n);
        }
    }
    let texts = style_texts(xml);
    // `inherit` takes even a clip path, mask or filter from the parent: if the file says it
    // anywhere, every property passes down.
    let inherit = texts.iter().any(|t| says_inherit(t)) || all.iter().any(|e| e.attributes().any(|a| says_inherit(a.value())));
    let mut g = Graph { nodes, marker, states: States::default(), inherit, by_id: spec_by_id, lengths, rules: Vec::new(), urls: HashMap::new() };
    let n = specials.len();
    // Style sheet rules that name ids, read as usvg reads them (its matcher is simplecss).
    for r in &sheet(&texts).rules {
        let urls: Vec<(&str, Vec<Vec<&str>>)> = r
            .declarations
            .iter()
            .filter_map(|d| {
                let mut v = Vec::new();
                each_url(d.value, &g.lengths, |ids| v.push(ids.to_vec()));
                (!v.is_empty()).then_some((d.name, v))
            })
            .collect();
        if urls.is_empty() {
            continue;
        }
        if g.rules.len() >= MAX_URL_RULES {
            return Err(Refusal::TooBig);
        }
        g.rules.push(Rule { selector: r.selector.clone(), urls });
    }
    let mut urls = HashMap::new();
    for e in all.iter().filter(|e| e.is_element()) {
        let mut v: Vec<(&str, Vec<&str>)> = Vec::new();
        g.read_urls(*e, |name, ids| v.push((name, ids.to_vec())));
        if !v.is_empty() {
            urls.insert(e.id().get_usize(), v);
        }
    }
    g.urls = urls;
    if n == 0 {
        return Ok(g);
    }
    // Every element a `href` of `e` (in any namespace) links to. A link (`<a>`) only names a
    // place to go to; usvg builds nothing from it.
    let links = |e: Node| -> Vec<Node> {
        let mut v = Vec::new();
        if e.tag_name().name() == "a" {
            return v;
        }
        for a in e.attributes().filter(|a| a.name() == "href") {
            if let Some(id) = svgtypes::IRI::from_str(a.value()).ok().map(|i| i.0) {
                v.extend(by_id.get(id).into_iter().flatten().copied());
            }
        }
        v
    };

    // Edges: from each one to the ones its content, ancestors, own attributes and links name.
    let mut edges: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut steps = 0u64;
    let mut total_edges = 0usize;
    // Visited per special: as what it holds itself, and as what it reaches through a link (where
    // usvg's fix for a special naming itself does not reach).
    let (mut plain, mut linked) = (vec![0u32; len], vec![0u32; len]);
    // Ancestors read per special (each element's once).
    let mut above = vec![0u32; len];
    for (si, s) in specials.iter().enumerate() {
        let mark = si as u32 + 1;
        let mut out = Vec::<u32>::new();
        // usvg removes a pattern's own fill or stroke, and a clip path's, mask's or filter's own
        // clip path, mask or filter, from what it holds when it names the element itself, so
        // such a link is no loop (an inherited one, or one reached through a link, is not
        // removed).
        let removed = |name: &str| match s.tag_name().name() {
            "pattern" => matches!(name, "fill" | "stroke"),
            "clipPath" => name == "clip-path",
            "mask" => name == "mask",
            "filter" => name == "filter",
            _ => false,
        };
        // The ids `e` names itself; only the properties that pass down to what an element holds
        // if `down`.
        let collect = |e: Node<'a, '_>, down: bool, exempt: bool, out: &mut Vec<u32>, steps: &mut u64| -> Result<(), Refusal> {
            *steps = steps.saturating_add(1).saturating_add(g.rules.len() as u64);
            if *steps > MAX_STEPS {
                return Err(Refusal::TooBig);
            }
            let before = out.len();
            g.urls_of(e, |name, ids| {
                if down && !g.passes_down(name) {
                    return;
                }
                for id in ids {
                    for t in g.named_by(id) {
                        if *t == si as u32 && !down && exempt && removed(name) {
                            continue;
                        }
                        if out.len() < MAX_EDGES {
                            out.push(*t);
                        }
                    }
                }
            });
            if out.len() >= MAX_EDGES {
                return Err(Refusal::TooBig);
            }
            *steps = steps.saturating_add((out.len() - before) as u64);
            Ok(())
        };
        // What `e` inherits from the elements around it (each read once per special).
        let mut ancestors = |e: Node<'a, '_>, out: &mut Vec<u32>, steps: &mut u64| -> Result<(), Refusal> {
            for a in e.ancestors().skip(1).filter(|a| a.is_element()) {
                match above.get_mut(a.id().get_usize()) {
                    Some(m) if *m == mark => break,
                    Some(m) => *m = mark,
                    None => {}
                }
                collect(a, true, false, out, steps)?;
            }
            Ok(())
        };
        ancestors(*s, &mut out, &mut steps)?;
        let mut stack: Vec<(Node, bool)> = vec![(*s, false)];
        if let Some(m) = plain.get_mut(s.id().get_usize()) {
            *m = mark;
        }
        while let Some((e, via_link)) = stack.pop() {
            collect(e, false, !via_link, &mut out, &mut steps)?;
            let mut next: Vec<(Node, bool)> = Vec::new();
            // Content it draws from elsewhere: every link is followed. A `<use>` copies its
            // target into the special, where the copy inherits from the `<use>` and usvg's fix
            // reaches it; anything else builds it where it stands, inheriting from the elements
            // around it.
            let copies = e.tag_name().name() == "use" && matches!(e.tag_name().namespace(), None | Some(SVG_NS));
            for t in links(e) {
                match special_at.get(t.id().get_usize()).copied().filter(|t| *t != u32::MAX) {
                    // A template (a pattern's, gradient's or filter's own `href`). A `<use>` or
                    // filter image of a special builds nothing: usvg only builds what is drawn.
                    Some(t) if e == *s => out.push(t),
                    Some(_) => {}
                    None => {
                        if !copies {
                            ancestors(t, &mut out, &mut steps)?;
                        }
                        next.push((t, via_link || !copies));
                    }
                }
            }
            for c in e.children() {
                if c.is_element()
                    && c.tag_name().name() != "style"
                    && matches!(c.tag_name().namespace(), None | Some(SVG_NS))
                    && special_at.get(c.id().get_usize()) == Some(&u32::MAX)
                {
                    next.push((c, via_link));
                }
            }
            for (c, link) in next {
                let seen = if link { linked.get_mut(c.id().get_usize()) } else { plain.get_mut(c.id().get_usize()) };
                if let Some(m) = seen
                    && *m != mark
                {
                    *m = mark;
                    steps = steps.saturating_add(1);
                    if steps > MAX_STEPS {
                        return Err(Refusal::TooBig);
                    }
                    stack.push((c, link));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        total_edges = total_edges.saturating_add(out.len());
        if total_edges > MAX_EDGES {
            return Err(Refusal::TooBig);
        }
        if let Some(slot) = edges.get_mut(si) {
            *slot = out;
        }
    }
    g.states = states(&edges, &g.marker)?;
    Ok(g)
}
