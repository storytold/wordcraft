//! Equations as MathML (W3C MathML Core / Presentation MathML) via `craft-math`.

pub use craft_math::mathml::{NS, UNICODEMATH};

pub fn to_mathml(nodes: &[wordcraft_doc::math::MNode], display: bool, linear: &str) -> String {
    let linear = if linear.is_empty() { wordcraft_doc::math::to_linear(nodes) } else { linear.to_string() };
    linear_to_mathml(&linear, display)
}

pub fn linear_to_mathml(linear: &str, display: bool) -> String {
    let tree = craft_math::math::parse_linear(linear);
    craft_math::mathml::to_mathml(&tree, display, linear)
}

pub fn parse(html_or_math: &str) -> Option<Vec<wordcraft_doc::math::MNode>> {
    let ml = craft_math::mathml::parse(html_or_math)?;
    let tree = craft_math::mathml::to_tree(&ml);
    let linear = craft_math::math::to_linear(&tree);
    Some(wordcraft_doc::math::parse_linear(&linear))
}
