//! Optional, section-local page labels. A hidden label is not a page numbered zero.

use crate::section::NumFormat;
use serde::{Deserialize, Serialize};

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageLabelMode {
    Hidden,
    Text,
    #[default]
    Number,
    TextNumber,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PageLabel {
    pub name: String,
    pub mode: PageLabelMode,
    pub text: String,
    pub separator: String,
    /// The complete label for zero in Roman/alphabetic formats (empty hides it).
    pub zero_label: String,
}

impl Default for PageLabel {
    fn default() -> Self {
        Self { name: String::new(), mode: PageLabelMode::Number, text: String::new(), separator: " ".into(), zero_label: String::new() }
    }
}

impl PageLabel {
    pub fn counts(&self) -> bool {
        matches!(self.mode, PageLabelMode::Number | PageLabelMode::TextNumber)
    }

    pub fn format(&self, format: NumFormat, number: u32) -> String {
        if self.mode == PageLabelMode::Hidden {
            return String::new();
        }
        if self.mode == PageLabelMode::Text {
            return self.text.clone();
        }
        if number == 0 && matches!(format, NumFormat::UpperRoman | NumFormat::LowerRoman | NumFormat::UpperLetter | NumFormat::LowerLetter) {
            return self.zero_label.clone();
        }
        let number = format.format(number);
        if self.mode == PageLabelMode::TextNumber && !self.text.is_empty() { format!("{}{}{number}", self.text, self.separator) } else { number }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        for (value, limit) in [(&self.name, 256), (&self.text, 256), (&self.separator, 16), (&self.zero_label, 256)] {
            if value.chars().take(limit + 1).count() > limit {
                return Err("page label text is too long");
            }
            if value.chars().any(char::is_control) {
                return Err("page labels cannot contain control characters");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_keep_text_zero_and_visibility_independent() {
        let mut label = PageLabel { mode: PageLabelMode::TextNumber, text: "Anlage α".into(), ..Default::default() };
        assert_eq!(label.format(NumFormat::Decimal, 0), "Anlage α 0");
        assert_eq!(label.format(NumFormat::UpperRoman, 1), "Anlage α I");
        assert_eq!(label.format(NumFormat::UpperLetter, 1), "Anlage α A");
        assert_eq!(label.format(NumFormat::UpperRoman, 0), "");
        label.zero_label = "Auftakt".into();
        assert_eq!(label.format(NumFormat::LowerLetter, 0), "Auftakt");
        label.mode = PageLabelMode::Text;
        assert_eq!(label.format(NumFormat::UpperRoman, 0), "Anlage α");
        label.mode = PageLabelMode::Hidden;
        assert_eq!(label.format(NumFormat::Decimal, 42), "");
        assert!(!label.counts());
    }
    #[test]
    fn label_validation_is_bounded_and_unicode_aware() {
        let mut label = PageLabel { text: "ä".repeat(256), ..Default::default() };
        assert!(label.validate().is_ok());
        label.text.push('ä');
        assert!(label.validate().is_err());
        label.text = "a\nb".into();
        assert!(label.validate().is_err());
    }
}
