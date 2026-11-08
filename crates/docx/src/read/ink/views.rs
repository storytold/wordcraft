//! Local trace-view selection, retaining the selected tree for nested one-based indices.

use super::{Context, El, InkReader, MAX_DEPTH, spend};

/// A resolved tree of selected trace ranges; geometry is decoded only after slicing.
enum Selection<'a> {
    /// One inclusive range in the original trace, with its resolved context.
    Trace { el: &'a El, context: Context<'a>, from: usize, to: usize },
    /// A retained group boundary, required by colon-separated nested selectors.
    Group(Vec<Selection<'a>>),
}

impl<'a> InkReader<'a, '_> {
    /// Resolve and draw definition-backed views; views of already-present traces are annotations,
    /// not additional paint operations (important for translucent ink and recognition groupings).
    pub(super) fn view(&mut self, el: &'a El, context: Context<'a>, depth: usize) -> Option<()> {
        let selection = self.selection(el, context, depth)?;
        let stream_tails = std::mem::take(&mut self.tails);
        let result = self.draw_selection(selection);
        self.tails = stream_tails;
        result
    }

    /// Resolve references and contexts into a bounded selection tree, detecting cycles by depth.
    fn selection(&mut self, el: &'a El, inherited: Context<'a>, depth: usize) -> Option<Selection<'a>> {
        spend(self.work, 1)?;
        if depth > MAX_DEPTH {
            return None;
        }
        let context = self.context(el, inherited, 0)?;
        match el.name.as_str() {
            "ink:trace" => {
                let text = el.text();
                spend(self.work, text.len().div_ceil(16))?;
                let to = text.trim().trim_end_matches(',').split(',').count();
                Some(Selection::Trace { el, context, from: 1, to })
            }
            "ink:traceGroup" => {
                let mut children = Vec::new();
                for child in el.els() {
                    if matches!(child.name.as_str(), "ink:annotation" | "ink:annotationXML") {
                        continue;
                    }
                    children.push(self.selection(child, context.clone(), depth + 1)?);
                }
                (!children.is_empty()).then_some(Selection::Group(children))
            }
            "ink:traceView" => {
                let id = el.attr("traceDataRef")?.strip_prefix('#')?;
                let target = *self.ids.get(id)?;
                let context = self.reference_context(target, context, 0)?;
                let selection = self.selection(target, context, depth + 1)?;
                selection.slice(&indices(el.attr("from"), self.work)?, &indices(el.attr("to"), self.work)?, self.work)
            }
            _ => None,
        }
    }

    /// A jump to a nested definition must not discard its enclosing groups' context.
    /// Ordinary child traversal already supplies that context; only reference jumps use this.
    fn reference_context(&mut self, el: &'a El, inherited: Context<'a>, depth: usize) -> Option<Context<'a>> {
        spend(self.work, 1)?;
        if depth > MAX_DEPTH {
            return None;
        }
        let Some(parent) = self.definition_groups.get(&(el as *const El)).copied() else {
            return Some(inherited);
        };
        let inherited = self.reference_context(parent, inherited, depth + 1)?;
        self.context(parent, inherited, 0)
    }

    /// Decode selected definition traces without duplicating original top-level ink.
    fn draw_selection(&mut self, selection: Selection<'a>) -> Option<()> {
        spend(self.work, 1)?;
        match selection {
            Selection::Trace { el, context, from, to } => {
                if self.definition_traces.contains(&(el as *const El)) {
                    self.trace(el, &context, Some((from, to)))?;
                }
            }
            Selection::Group(children) => {
                for child in children {
                    self.draw_selection(child)?;
                }
            }
        }
        Some(())
    }
}

impl Selection<'_> {
    /// Cut a range while retaining nested groups so further views index the selected tree.
    fn slice(self, from: &[usize], to: &[usize], work: &mut usize) -> Option<Self> {
        spend(work, 1)?;
        match self {
            Self::Trace { el, context, from: base, to: end } => {
                if from.len() > 1 || to.len() > 1 {
                    return None;
                }
                let len = end.checked_sub(base)?.checked_add(1)?;
                let first = from.first().copied().unwrap_or(1);
                let last = to.first().copied().unwrap_or(len);
                if first == 0 || last < first || last > len {
                    return None;
                }
                Some(Self::Trace { el, context, from: base.checked_add(first - 1)?, to: base.checked_add(last - 1)? })
            }
            Self::Group(children) => {
                let first = from.first().copied().unwrap_or(1);
                let last = to.first().copied().unwrap_or(children.len());
                if first == 0 || last < first || last > children.len() {
                    return None;
                }
                let mut selected = Vec::new();
                for (i, child) in children.into_iter().enumerate() {
                    let n = i + 1;
                    if n < first || n > last {
                        continue;
                    }
                    let f = if n == first { from.get(1..).unwrap_or(&[]) } else { &[] };
                    let t = if n == last { to.get(1..).unwrap_or(&[]) } else { &[] };
                    selected.push(child.slice(f, t, work)?);
                }
                Some(Self::Group(selected))
            }
        }
    }
}

/// Parse at most MAX_DEPTH one-based selector components without accepting zero or signs.
fn indices(value: Option<&str>, work: &mut usize) -> Option<Vec<usize>> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Some(Vec::new());
    };
    // The same definition-backed view can be selected many times. Reserve before
    // scanning digits so long leading-zero indices cannot bypass the part's work cap.
    spend(work, value.len().div_ceil(16))?;
    let mut out = Vec::new();
    for part in value.split(':') {
        if out.len() >= MAX_DEPTH || part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n = part.parse::<usize>().ok()?;
        if n == 0 {
            return None;
        }
        out.push(n);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::indices;

    /// Selector input is charged before parsing; refused reservations spend no work.
    #[test]
    fn selector_bytes_are_reserved_before_scanning() {
        let selector = format!("{}1", "0".repeat(64));
        let cost = selector.len().div_ceil(16);
        let mut refused = cost - 1;
        assert!(indices(Some(&selector), &mut refused).is_none());
        assert_eq!(refused, cost - 1, "a refused reservation spends no work");
        let mut exact = cost;
        assert_eq!(indices(Some(&selector), &mut exact), Some(vec![1]));
        assert_eq!(exact, 0);
        let mut invalid = cost;
        assert!(indices(Some(&format!("{}x", "0".repeat(64))), &mut invalid).is_none());
        assert_eq!(invalid, 0, "accepted work remains charged when parsing fails");
    }
}
