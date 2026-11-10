use super::{
    model, numbering,
    xml::{Attrs, get},
};
use std::collections::{HashMap, HashSet};
use wordcraft_doc::section::NumFormat;

/// Bounded counters and value references for body sequence fields.
#[derive(Default)]
pub(super) struct Sequences {
    /// Last calculated value; None marks an unsupported formula in the chain.
    values: HashMap<String, Option<u32>>,
    /// Names requiring unsupported chapter-aware numbering.
    chapter: HashSet<String>,
    /// Calculated labels indexed by reference name.
    references: HashMap<String, String>,
}

impl Sequences {
    pub fn declare(&mut self, a: &Attrs) {
        if get(a, "text:display-outline-level").is_some_and(|v| v != "0")
            && self.chapter.len() < 16384
            && let Some(name) = get(a, "text:name").filter(|s| s.len() <= 256)
        {
            self.chapter.insert(name.into());
        }
    }
    pub fn render(&mut self, a: &Attrs) -> Option<String> {
        let name = get(a, "text:name")?;
        if name.len() > 256 || self.values.len() >= 16384 || self.chapter.contains(name) {
            return None;
        }
        let old = self.values.get(name).copied().unwrap_or(Some(0));
        let formula = get(a, "text:formula").and_then(|s| s.strip_prefix("ooow:"));
        let value = formula.and_then(|f| evaluate(f.trim(), name, old));
        self.values.insert(name.to_string(), value);
        let value = value?;
        let kind = numbering::format(get(a, "style:num-format").unwrap_or("1"))?;
        let sequential = get(a, "style:num-letter-sync") != Some("true");
        if matches!(kind, NumFormat::UpperRoman | NumFormat::LowerRoman) && value > 50_000 {
            return None;
        }
        if !sequential && matches!(kind, NumFormat::UpperLetter | NumFormat::LowerLetter) && value > 780 {
            return None;
        }
        let label = if sequential && matches!(kind, NumFormat::UpperLetter | NumFormat::LowerLetter) {
            wordcraft_doc::numbering::sequential_letters(value, kind == NumFormat::UpperLetter)
        } else {
            kind.format(value)
        };
        if self.references.len() < 16384
            && let Some(name) = get(a, "text:ref-name").filter(|s| s.len() <= 256)
        {
            self.references.insert(name.into(), label.clone());
        }
        Some(label)
    }
    pub fn reference(&self, a: &Attrs) -> Option<String> {
        if get(a, "text:reference-format") != Some("value") {
            return None;
        }
        self.references.get(get(a, "text:ref-name")?).cloned()
    }
}

fn evaluate(formula: &str, name: &str, previous: Option<u32>) -> Option<u32> {
    if let Ok(value) = formula.parse::<u32>() {
        return Some(value);
    }
    let rest = formula.strip_prefix(name)?.trim();
    let value = previous?;
    if rest.is_empty() {
        return Some(value);
    }
    if let Some(add) = rest.strip_prefix('+') {
        value.checked_add(add.trim().parse().ok()?)
    } else if let Some(sub) = rest.strip_prefix('-') {
        value.checked_sub(sub.trim().parse().ok()?)
    } else {
        None
    }
}

pub(super) fn unsupported() {
    model::dropped(model::Dropped::OdtSequence);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(values: &[(&str, &str)]) -> Attrs {
        values.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
    }

    #[test]
    fn formulas_and_value_references_use_the_calculated_number() {
        let mut sequences = Sequences::default();
        let first = attrs(&[("text:name", "Figure"), ("text:formula", "ooow:28"), ("style:num-format", "A"), ("text:ref-name", "first")]);
        assert_eq!(sequences.render(&first).as_deref(), Some("AB"));
        assert_eq!(sequences.reference(&attrs(&[("text:ref-name", "first"), ("text:reference-format", "value")])).as_deref(), Some("AB"));
        let next = attrs(&[("text:name", "Figure"), ("text:formula", "ooow:Figure+1"), ("style:num-format", "i")]);
        assert_eq!(sequences.render(&next).as_deref(), Some("xxix"));
        assert_eq!(evaluate("Figure-2", "Figure", Some(29)), Some(27));
        assert_eq!(evaluate("Figure+1", "Figure", Some(u32::MAX)), None);
        assert_eq!(evaluate("Figure-1", "Figure", Some(0)), None);
        assert_eq!(evaluate("Figure*2", "Figure", Some(2)), None);
    }
}
