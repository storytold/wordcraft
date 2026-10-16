//! One equation as a standalone OMML fragment, in and out, for the clipboard and the equation
//! editor. These reuse the document reader and writer (`read::math`, `write::math`), so a
//! fragment means exactly what the same `m:oMath` means inside a `.docx`.

use wordcraft_doc::math::{Math, MathJc};

use crate::xml::{El, NAMESPACES, W};

/// Largest OMML fragment read, bytes.
pub const MAX_OMML: usize = 4 << 20;

/// The namespaces an OMML fragment uses: math, and WordprocessingML for run properties.
fn ns_attrs() -> Vec<(String, String)> {
    NAMESPACES.iter().filter(|(p, _)| matches!(*p, "m" | "w")).map(|(p, u)| (format!("xmlns:{p}"), (*u).to_string())).collect()
}

/// An OMML fragment as an equation and whether it is a display one: an `m:oMathPara`
/// (display, its first `m:oMath`) or an `m:oMath` (inline), at the root or wrapped in other
/// elements. Namespace declarations may be missing: the `m:` and `w:` prefixes are bound to
/// the OMML and WordprocessingML namespaces unless the fragment binds them itself. `None` when
/// the text is not well-formed XML, holds no equation or is larger than [`MAX_OMML`].
pub fn equation_from_omml(xml: &str) -> Option<(Math, bool)> {
    if xml.len() > MAX_OMML {
        return None;
    }
    let body = xml.trim().strip_prefix('\u{FEFF}').unwrap_or(xml.trim());
    // An XML declaration cannot sit inside the wrapper.
    let body = match body.strip_prefix("<?xml") {
        Some(rest) => rest.split_once("?>").map(|(_, r)| r).unwrap_or(rest),
        None => body,
    };
    let decl: String = ns_attrs().iter().map(|(k, v)| format!(" {k}=\"{v}\"")).collect();
    let wrapped = format!("<wcx{decl}>{body}</wcx>");
    let root = crate::xml::parse(wrapped.as_bytes()).ok()?;
    if let Some(para) = find(&root, "m:oMathPara") {
        let jc = crate::read::math::para_jc(para);
        let m = para.child("m:oMath")?;
        return Some((crate::read::math::read_omath(m, jc), true));
    }
    find(&root, "m:oMath").map(|m| (crate::read::math::read_omath(m, MathJc::default()), false))
}

/// The element itself when it is `name`, else its first descendant with that name.
fn find<'a>(e: &'a El, name: &str) -> Option<&'a El> {
    if e.name == name { Some(e) } else { e.find(name) }
}

/// An equation as a standalone OMML fragment: `m:oMath`, inside `m:oMathPara` when `display`,
/// with the `m:` and `w:` namespaces declared on the root. `linear` is the equation's linear
/// text, used when `math` has no tree (as in the document writer).
pub fn equation_to_omml(linear: &str, display: bool, math: &Math) -> String {
    let mut w = W::default();
    // No separate run colour: the fragment keeps the colour its own OMML carries.
    let color = crate::write::math::omml_color(&math.omml);
    crate::write::math::write_equation(&mut w, linear, display, math, color);
    declare_ns(&w.s)
}

/// Add the namespace declarations the fragment's root start tag lacks.
fn declare_ns(xml: &str) -> String {
    let Some(start) = xml.find('<') else { return xml.to_string() };
    let rest = xml.get(start + 1..).unwrap_or("");
    let name_len = rest.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(rest.len());
    let tag_end = rest.find('>').unwrap_or(rest.len());
    let tag = rest.get(..tag_end).unwrap_or("");
    let decl: String = ns_attrs().iter().filter(|(k, _)| !tag.contains(&format!("{k}="))).map(|(k, v)| format!(" {k}=\"{v}\"")).collect();
    let at = start + 1 + name_len;
    format!("{}{decl}{}", xml.get(..at).unwrap_or(""), xml.get(at..).unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::math::{MNode, parse_linear, to_linear};

    fn math(linear: &str) -> Math {
        Math { nodes: parse_linear(linear), ..Default::default() }
    }

    #[test]
    fn fragments_declare_their_namespaces_and_read_back() {
        for (lin, display) in [("x=(-b±√(b^2-4ac))/2a", true), ("a/b", false), ("∑_(i=1)^n▒i", true)] {
            let m = math(lin);
            let xml = equation_to_omml(lin, display, &m);
            let root = if display { "<m:oMathPara xmlns:" } else { "<m:oMath xmlns:" };
            assert!(xml.starts_with(root), "{xml}");
            assert_eq!(xml.matches("xmlns:m=").count(), 1, "{xml}");
            let (back, d) = equation_from_omml(&xml).unwrap();
            assert_eq!(d, display);
            assert_eq!(back.nodes, m.nodes, "{lin}");
        }
    }

    #[test]
    fn reads_bare_prefixes_wrappers_and_declarations() {
        let bare = "<m:oMath><m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f></m:oMath>";
        let (m, display) = equation_from_omml(bare).unwrap();
        assert!(!display);
        assert!(matches!(m.nodes.as_slice(), [MNode::Frac { .. }]), "{:?}", m.nodes);
        let declared = format!(
            "<?xml version=\"1.0\"?>\n<m:oMathPara xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\"><m:oMathParaPr><m:jc m:val=\"left\"/></m:oMathParaPr>{bare}</m:oMathPara>"
        );
        let (m, display) = equation_from_omml(&declared).unwrap();
        assert!(display);
        assert_eq!(m.jc, MathJc::Left);
        assert_eq!(to_linear(&m.nodes), "1/2");
    }

    #[test]
    fn junk_is_none_never_a_panic() {
        for s in ["", "x", "<m:oMath>", "<p>no math</p>", "<m:oMath></m:r></m:oMath>", "<<<", "<?xml", "<m:oMathPara/>"] {
            let _ = equation_from_omml(s);
        }
        assert!(equation_from_omml("<p>no math</p>").is_none());
        assert!(equation_from_omml(&"<m:e>".repeat(2000)).is_none());
        assert!(equation_from_omml(&" ".repeat(MAX_OMML + 1)).is_none());
    }
}
