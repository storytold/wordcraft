//! Defaults for new documents (File › Options › Defaults): the styles and page setup a person
//! adopts from a document. They live in the app's preferences (`UiState::defaults`, saved in
//! `ui.json`) and are applied by [`Document::apply_defaults`] when a blank document is made —
//! never to a document being opened, which keeps its own styles and page.

use serde::{Deserialize, Serialize};

use crate::Document;
use crate::props::Rgb;
use crate::section::{MAX_PAGE_SIDE, MIN_PAGE_SIDE, SectionProps};
use crate::styles::{Style, StyleSheet};

/// The page setup adopted for new documents.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PageDefaults {
    pub page_w: f32,
    pub page_h: f32,
    pub landscape: bool,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    /// Mirror margins (a document-wide setting, like the page colour).
    pub mirror: bool,
    /// The sheet's own colour; `None` leaves the theme's paper.
    pub page_color: Option<Rgb>,
}

impl Default for PageDefaults {
    fn default() -> Self {
        let s = SectionProps::default();
        PageDefaults {
            page_w: s.page_w,
            page_h: s.page_h,
            landscape: s.landscape,
            margin_top: s.margin_top,
            margin_bottom: s.margin_bottom,
            margin_left: s.margin_left,
            margin_right: s.margin_right,
            mirror: false,
            page_color: None,
        }
    }
}

impl PageDefaults {
    /// The page setup a document's section has (its size, orientation and margins), with the
    /// document's mirror margins and page colour.
    pub fn from_section(s: &SectionProps, mirror: bool, page_color: Option<Rgb>) -> Self {
        PageDefaults {
            page_w: s.page_w,
            page_h: s.page_h,
            landscape: s.landscape,
            margin_top: s.margin_top,
            margin_bottom: s.margin_bottom,
            margin_left: s.margin_left,
            margin_right: s.margin_right,
            mirror,
            page_color,
        }
    }

    /// Keep the values inside the page model's limits: a hand-edited or older `ui.json` must not
    /// put an impossible page into every new document.
    pub fn sanitize(&mut self) {
        let side = |v: f32, fallback: f32| if v.is_finite() { v.clamp(MIN_PAGE_SIDE, MAX_PAGE_SIDE) } else { fallback };
        let default = SectionProps::default();
        self.page_w = side(self.page_w, default.page_w);
        self.page_h = side(self.page_h, default.page_h);
        self.margin_top = self.margin_top.clamp(0.0, self.page_h);
        self.margin_bottom = self.margin_bottom.clamp(0.0, self.page_h);
        self.margin_left = self.margin_left.clamp(0.0, self.page_w);
        self.margin_right = self.margin_right.clamp(0.0, self.page_w);
        // Margins that leave no text room are brought back to the app's margins.
        if self.margin_left + self.margin_right > self.page_w - 18.0 || self.margin_top + self.margin_bottom > self.page_h - 18.0 {
            let d = SectionProps::default();
            self.margin_top = d.margin_top;
            self.margin_bottom = d.margin_bottom;
            self.margin_left = d.margin_left;
            self.margin_right = d.margin_right;
        }
    }

    fn apply(&self, doc: &mut Document) {
        let mut p = *self;
        p.sanitize();
        doc.last_section.page_w = p.page_w;
        doc.last_section.page_h = p.page_h;
        doc.last_section.landscape = p.landscape;
        doc.last_section.margin_top = p.margin_top;
        doc.last_section.margin_bottom = p.margin_bottom;
        doc.last_section.margin_left = p.margin_left;
        doc.last_section.margin_right = p.margin_right;
        // The header/footer distances stay on the page.
        doc.last_section.header = doc.last_section.header.clamp(0.0, p.page_h);
        doc.last_section.footer = doc.last_section.footer.clamp(0.0, p.page_h);
        doc.settings.mirror_margins = p.mirror;
        doc.settings.page_color = p.page_color;
    }
}

/// Defaults for new documents: a whole style sheet (every style adopted), individual style
/// definitions, and the page setup. Empty means the app's own built-in defaults.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocumentDefaults {
    /// A whole sheet adopted from a document (`adopt all styles`); replaces the built-in sheet.
    pub sheet: Option<StyleSheet>,
    /// Style definitions adopted one by one, merged over the sheet (or the built-in sheet).
    pub styles: Vec<Style>,
    /// The page setup for new documents; `None` keeps the app's regional paper.
    pub page: Option<PageDefaults>,
}

impl DocumentDefaults {
    pub fn is_empty(&self) -> bool {
        self.sheet.is_none() && self.styles.is_empty() && self.page.is_none()
    }

    /// Keep the defaults inside sane limits (a hand-edited or older `ui.json`).
    pub fn sanitize(&mut self) {
        const MAX_STYLES: usize = 5000;
        self.styles.retain(|s| !s.id.is_empty() && s.id.chars().count() <= 200);
        self.styles.truncate(MAX_STYLES);
        if let Some(sheet) = &mut self.sheet {
            sheet.styles.truncate(MAX_STYLES);
        }
        if let Some(page) = &mut self.page {
            page.sanitize();
        }
    }

    /// The style sheet these defaults stand for: the adopted sheet (all styles), or the built-in
    /// sheet with the adopted styles merged over it. The UI edits and previews this.
    pub fn styles_sheet(&self) -> StyleSheet {
        let mut sheet = self.sheet.clone().unwrap_or_default();
        if self.sheet.is_none() {
            for style in &self.styles {
                match sheet.styles.iter_mut().find(|s| s.id == style.id) {
                    Some(slot) => *slot = style.clone(),
                    None => sheet.styles.push(style.clone()),
                }
            }
        }
        sheet
    }

    /// The ids this default defines, in order (the sheet's, or the adopted ones).
    pub fn style_ids(&self) -> Vec<String> {
        match &self.sheet {
            Some(sheet) => sheet.styles.iter().map(|s| s.id.clone()).collect(),
            None => self.styles.iter().map(|s| s.id.clone()).collect(),
        }
    }
}

impl Document {
    /// Start this (new) document from `defaults`: the adopted styles and page setup. An empty
    /// default leaves the document as it is. Existing documents are never passed here.
    pub fn apply_defaults(&mut self, defaults: &DocumentDefaults) {
        let mut d = defaults.clone();
        d.sanitize();
        if let Some(sheet) = d.sheet {
            self.styles = sheet;
        }
        for style in d.styles {
            match self.styles.styles.iter_mut().find(|s| s.id == style.id) {
                Some(slot) => *slot = style,
                None => self.styles.styles.push(style),
            }
        }
        if let Some(page) = d.page {
            page.apply(self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_replace_styles_and_page_of_a_new_document() {
        let mut d = Document::new();
        let mut style = d.styles.styles.iter().find(|s| s.id == "Normal").cloned().unwrap();
        style.chr.size = Some(15.0);
        let mut page = PageDefaults::from_section(&d.last_section, false, Some(Rgb(0x18, 0x18, 0x18)));
        page.margin_top = 40.0;
        page.page_h = 600.0 * 72.0 / 2.54;
        let defaults = DocumentDefaults { sheet: None, styles: vec![style], page: Some(page) };
        d.apply_defaults(&defaults);
        let normal = d.styles.styles.iter().find(|s| s.id == "Normal").unwrap();
        assert_eq!(normal.chr.size, Some(15.0));
        assert_eq!(d.last_section.margin_top, 40.0);
        assert_eq!(d.last_section.page_h, page.page_h);
        assert_eq!(d.settings.page_color, Some(Rgb(0x18, 0x18, 0x18)));
        // Other styles keep the built-in sheet.
        assert!(d.styles.styles.iter().any(|s| s.id == "Heading1"));
    }

    #[test]
    fn an_empty_default_leaves_a_document_alone() {
        let mut d = Document::new();
        let before = d.clone();
        d.apply_defaults(&DocumentDefaults::default());
        assert_eq!(d.styles, before.styles);
        assert_eq!(d.last_section, before.last_section);
    }

    #[test]
    fn junk_defaults_are_brought_back_into_limits() {
        let mut d = Document::new();
        let page = PageDefaults { page_w: f32::NAN, page_h: 1e30, margin_left: -5.0, margin_right: 1e9, ..Default::default() };
        let defaults = DocumentDefaults { sheet: None, styles: vec![], page: Some(page) };
        d.apply_defaults(&defaults);
        assert!(d.last_section.page_w.is_finite() && d.last_section.page_w >= MIN_PAGE_SIDE);
        assert_eq!(d.last_section.page_h, MAX_PAGE_SIDE);
        assert!(d.last_section.margin_left >= 0.0);
        assert!(d.last_section.margin_left + d.last_section.margin_right < d.last_section.page_w);
    }
}
