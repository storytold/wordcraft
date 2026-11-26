//! The fonts the laid-out text asks for that aren't installed, with the faces drawn instead
//! ([`DocLayout::substituted_fonts`]), read from what each paragraph recorded as it was shaped
//! ([`ParaLayout::substituted`](crate::ParaLayout::substituted)).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use wordcraft_fonts::FaceRef;

use crate::{DocLayout, Placed};

/// Most (font, face) pairs [`DocLayout::substituted_fonts`] returns: a hostile file naming a
/// new font in every paragraph can't make the list (and the status bar reading it) unbounded.
pub const MAX_SUBSTITUTED: usize = 1024;

impl DocLayout {
    /// The fonts the laid-out text asks for that aren't installed, each with the faces it was
    /// drawn in instead and whether its italic text was slanted there for want of an italic
    /// (`ParaLayout::substituted`), over every page's body, header, footer,
    /// notes, text boxes and table cells: the text this view shows (Web, Draft and Read views lay
    /// out no headers or footers). Chart and SmartArt labels, drawn apart from paragraphs, aren't
    /// included. Each (font, face) pair once, in the order first met, at most
    /// [`MAX_SUBSTITUTED`]; a pair listed is slanted when any of its text is.
    pub fn substituted_fonts(&self) -> Vec<(String, FaceRef, bool)> {
        let mut w = Walk { paras: HashSet::new(), pairs: HashMap::new(), out: Vec::new() };
        for p in &self.pages {
            for items in [&p.items, &p.header, &p.footer] {
                w.walk(items);
            }
        }
        w.out
    }
}

struct Walk<'a> {
    /// Paragraphs looked at: one split over pages is placed once per piece.
    paras: HashSet<usize>,
    /// (font, face id) pairs already in `out`, by index.
    pairs: HashMap<(&'a str, u32), usize>,
    out: Vec<(String, FaceRef, bool)>,
}

impl<'a> Walk<'a> {
    fn walk(&mut self, items: &'a [Placed]) {
        for it in items {
            match it {
                Placed::Lines { para, .. } if self.paras.insert(Arc::as_ptr(para) as usize) => {
                    for (font, face, slanted) in &para.substituted {
                        if let Some(&i) = self.pairs.get(&(font.as_str(), face.id())) {
                            self.out[i].2 |= *slanted;
                        } else if self.out.len() < MAX_SUBSTITUTED {
                            self.pairs.insert((font.as_str(), face.id()), self.out.len());
                            self.out.push((font.clone(), *face, *slanted));
                        }
                    }
                }
                Placed::Clip { items, .. } => self.walk(items),
                _ => {}
            }
        }
    }
}
