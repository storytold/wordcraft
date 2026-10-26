//! `word/settings.xml` merged into the file's original settings part.
//!
//! The model owns only some settings (`super::settings_xml`). Regenerating the part from the
//! model alone drops everything else Word stores there: compatibility options, rsids, document
//! variables, proofing state, the colour scheme mapping, "remove personal information on save"…
//! So when a file brought settings the model doesn't hold ([`has_unowned`]; the reader keeps its
//! bytes under `SETTINGS_ORIG`), the writer merges instead ([`merge`]): the original root start
//! tag (namespace declarations, `mc:Ignorable`) and every child the model doesn't own are kept
//! byte for byte; owned children come from the generated part (or go, when the model turned
//! them off); owned children the original lacks are inserted at their CT_Settings position; in
//! `w:compat` only the `compatibilityMode` setting is replaced. Kept children that refer to a
//! relationship (`r:id`, e.g. a mail merge data source) are dropped: their relationships aren't
//! carried. Anything unexpected returns `None`, and the generated part is written as before.

/// Children of `w:settings` the model writes (`super::settings_xml`).
const OWNED: &[&str] = &[
    "w:removePersonalInformation",
    "w:removeDateAndTime",
    "w:displayBackgroundShape",
    "w:mirrorMargins",
    "w:attachedTemplate",
    "w:linkStyles",
    "w:trackRevisions",
    "w:doNotTrackMoves",
    "w:documentProtection",
    "w:defaultTabStop",
    "w:autoHyphenation",
    "w:consecutiveHyphenLimit",
    "w:hyphenationZone",
    "w:doNotHyphenateCaps",
    "w:evenAndOddHeaders",
    "w:drawingGridHorizontalSpacing",
    "w:drawingGridVerticalSpacing",
    "w:characterSpacingControl",
    "w:footnotePr",
    "w:endnotePr",
    "m:mathPr",
];

/// Owned children whose original, when there is one, is kept as is.
const KEEP_ORIGINAL: &[&str] = &["m:mathPr", "w:displayBackgroundShape"];

/// CT_Settings' sequence (ECMA-376 §17.15.1.78), for placing inserted children.
const ORDER: &[&str] = &[
    "w:writeProtection",
    "w:view",
    "w:zoom",
    "w:removePersonalInformation",
    "w:removeDateAndTime",
    "w:doNotDisplayPageBoundaries",
    "w:displayBackgroundShape",
    "w:printPostScriptOverText",
    "w:printFractionalCharacterWidth",
    "w:printFormsData",
    "w:embedTrueTypeFonts",
    "w:embedSystemFonts",
    "w:saveSubsetFonts",
    "w:saveFormsData",
    "w:mirrorMargins",
    "w:alignBordersAndEdges",
    "w:bordersDoNotSurroundHeader",
    "w:bordersDoNotSurroundFooter",
    "w:gutterAtTop",
    "w:hideSpellingErrors",
    "w:hideGrammaticalErrors",
    "w:activeWritingStyle",
    "w:proofState",
    "w:formsDesign",
    "w:attachedTemplate",
    "w:linkStyles",
    "w:stylePaneFormatFilter",
    "w:stylePaneSortMethod",
    "w:documentType",
    "w:mailMerge",
    "w:revisionView",
    "w:trackRevisions",
    "w:doNotTrackMoves",
    "w:doNotTrackFormatting",
    "w:documentProtection",
    "w:autoFormatOverride",
    "w:styleLockTheme",
    "w:styleLockQFSet",
    "w:defaultTabStop",
    "w:autoHyphenation",
    "w:consecutiveHyphenLimit",
    "w:hyphenationZone",
    "w:doNotHyphenateCaps",
    "w:showEnvelope",
    "w:summaryLength",
    "w:clickAndTypeStyle",
    "w:defaultTableStyle",
    "w:evenAndOddHeaders",
    "w:bookFoldRevPrinting",
    "w:bookFoldPrinting",
    "w:bookFoldPrintingSheets",
    "w:drawingGridHorizontalSpacing",
    "w:drawingGridVerticalSpacing",
    "w:displayHorizontalDrawingGridEvery",
    "w:displayVerticalDrawingGridEvery",
    "w:doNotUseMarginsForDrawingGridOrigin",
    "w:drawingGridHorizontalOrigin",
    "w:drawingGridVerticalOrigin",
    "w:doNotShadeFormData",
    "w:noPunctuationKerning",
    "w:characterSpacingControl",
    "w:printTwoOnOne",
    "w:strictFirstAndLastChars",
    "w:noLineBreaksAfter",
    "w:noLineBreaksBefore",
    "w:savePreviewPicture",
    "w:doNotValidateAgainstSchema",
    "w:saveInvalidXml",
    "w:ignoreMixedContent",
    "w:alwaysShowPlaceholderText",
    "w:doNotDemarcateInvalidXml",
    "w:saveXmlDataOnly",
    "w:useXSLTWhenSaving",
    "w:saveThroughXslt",
    "w:showXMLTags",
    "w:alwaysMergeEmptyNamespace",
    "w:updateFields",
    "w:hdrShapeDefaults",
    "w:footnotePr",
    "w:endnotePr",
    "w:compat",
    "w:docVars",
    "w:rsids",
    "m:mathPr",
    "w:attachedSchema",
    "w:themeFontLang",
    "w:clrSchemeMapping",
    "w:doNotIncludeSubdocsInStats",
    "w:doNotAutoCompressPictures",
    "w:forceUpgrade",
    "w:captions",
    "w:readModeInkLockDown",
    "w:smartTagType",
    "sl:schemaLibrary",
    "w:shapeDefaults",
    "w:doNotEmbedSmartTags",
    "w:decimalSymbol",
    "w:listSeparator",
];

fn rank(name: &str) -> usize {
    ORDER.iter().position(|n| *n == name).unwrap_or(ORDER.len())
}

/// A child element of the root: its qualified name and byte span.
struct Child {
    name: String,
    start: usize,
    end: usize,
}

/// Byte index of the `>` that ends the tag at the start of `s`, skipping quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// The root start tag's span end and the root's direct children, or `None` for anything the
/// scanner doesn't expect (CDATA, DOCTYPE, a different root).
fn children(xml: &str, root: &str) -> Option<(usize, usize, Vec<Child>)> {
    let open = xml.find(&format!("<{root}"))?;
    let open_end = open + tag_end(xml.get(open..)?)? + 1;
    if xml.get(open_end - 2..open_end)? == "/>" {
        return None;
    }
    let close = xml.rfind(&format!("</{root}>"))?;
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut i = open_end;
    let mut cur: Option<(String, usize)> = None;
    while i < close {
        let rest = xml.get(i..close)?;
        let Some(lt) = rest.find('<') else { break };
        let at = i + lt;
        let tail = xml.get(at..close)?;
        if tail.starts_with("<!--") {
            i = at + tail.find("-->")? + 3;
            continue;
        }
        if tail.starts_with("<![CDATA[") || tail.starts_with("<!") {
            return None;
        }
        if tail.starts_with("<?") {
            i = at + tail.find("?>")? + 2;
            continue;
        }
        let gt = at + tag_end(tail)? + 1;
        let tag = xml.get(at..gt)?;
        if tag.starts_with("</") {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                let (name, s) = cur.take()?;
                out.push(Child { name, start: s, end: gt });
            }
        } else {
            let name: String = tag.get(1..)?.chars().take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').collect();
            let selfclose = tag.ends_with("/>");
            if depth == 0 {
                if selfclose {
                    out.push(Child { name, start: at, end: gt });
                } else {
                    cur = Some((name, at));
                    depth = 1;
                }
            } else if !selfclose {
                depth += 1;
            }
        }
        i = gt;
    }
    if depth != 0 {
        return None;
    }
    Some((open_end, close, out))
}

/// The original `w:compat` with its `compatibilityMode` taken from the generated one.
fn merge_compat(orig: &str, generated: &str) -> Option<String> {
    let mode = |s: &str| -> Option<(usize, usize)> {
        let at = s.find("w:name=\"compatibilityMode\"")?;
        let tag_start = s.get(..at)?.rfind('<')?;
        let tag_end = tag_start + s.get(tag_start..)?.find('>')? + 1;
        Some((tag_start, tag_end))
    };
    match (mode(orig), mode(generated)) {
        (Some((a, b)), Some((c, d))) => Some(format!("{}{}{}", orig.get(..a)?, generated.get(c..d)?, orig.get(b..)?)),
        (None, Some((c, d))) => {
            // No mode in the original: add the generated one before `</w:compat>`, or expand a self-closing `<w:compat/>`.
            if let Some(close) = orig.rfind("</w:compat>") {
                Some(format!("{}{}{}", orig.get(..close)?, generated.get(c..d)?, orig.get(close..)?))
            } else if let Some(slash) = orig.rfind("/>") {
                let open = orig.get(..slash)?.trim_end();
                Some(format!("{open}>{}</w:compat>", generated.get(c..d)?))
            } else {
                None
            }
        }
        _ => Some(orig.to_string()),
    }
}

/// Does `original` (a settings part) hold anything the model doesn't write? Then the reader keeps
/// it for [`merge`]. False for parts WordCraft wrote itself, and for anything it can't scan.
pub fn has_unowned(original: &[u8]) -> bool {
    let Some(xml) = std::str::from_utf8(original).ok() else { return false };
    let Some((_, _, kids)) = children(xml, "w:settings") else { return false };
    kids.iter().any(|k| match k.name.as_str() {
        "w:zoom" => false,
        // Anything in `w:compat` beyond its own tag and the compatibility mode.
        "w:compat" => xml.get(k.start..k.end).is_some_and(|c| {
            let mode = usize::from(c.contains("w:name=\"compatibilityMode\""));
            c.matches("<w:").count().saturating_sub(1 + mode) > 0
        }),
        n => !OWNED.contains(&n),
    })
}

/// The merged settings part, or `None` to fall back to `generated`.
pub fn merge(original: &[u8], generated: &[u8]) -> Option<Vec<u8>> {
    let orig = std::str::from_utf8(original).ok()?;
    let gen_s = std::str::from_utf8(generated).ok()?;
    let (o_open, o_close, o_kids) = children(orig, "w:settings")?;
    let (_, _, g_kids) = children(gen_s, "w:settings")?;
    // Prefixes the generated children use must be declared by the original root.
    let root_tag = orig.get(..o_open)?;
    for p in ["w", "m", "r"] {
        if g_kids.iter().any(|k| k.name.starts_with(&format!("{p}:"))) && !root_tag.contains(&format!("xmlns:{p}=")) {
            return None;
        }
    }
    let g_text = |k: &Child| gen_s.get(k.start..k.end).map(str::to_string);
    let mut used = vec![false; g_kids.len()];
    let mark = |name: &str, used: &mut Vec<bool>| -> Option<usize> {
        let gi = g_kids.iter().enumerate().position(|(i, g)| g.name == name && !used.get(i).copied().unwrap_or(true))?;
        if let Some(u) = used.get_mut(gi) {
            *u = true;
        }
        Some(gi)
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for k in &o_kids {
        let text = orig.get(k.start..k.end)?;
        if KEEP_ORIGINAL.contains(&k.name.as_str()) {
            // The model reads these only partly (math settings) or writes them only in one case
            // (the background shape flag, with a page colour): the original wins.
            mark(&k.name, &mut used);
            out.push((k.name.clone(), text.to_string()));
        } else if OWNED.contains(&k.name.as_str()) {
            if let Some(gi) = mark(&k.name, &mut used) {
                out.push((k.name.clone(), g_text(g_kids.get(gi)?)?));
            }
        } else if k.name == "w:compat" {
            let merged = match mark("w:compat", &mut used) {
                Some(gi) => merge_compat(text, &g_text(g_kids.get(gi)?)?)?,
                None => text.to_string(),
            };
            out.push((k.name.clone(), merged));
        } else if !text.contains("r:id=") {
            out.push((k.name.clone(), text.to_string()));
        }
    }
    for (gi, g) in g_kids.iter().enumerate() {
        let owned = OWNED.contains(&g.name.as_str()) || g.name == "w:compat";
        if used.get(gi).copied().unwrap_or(true) || !owned {
            continue;
        }
        let r = rank(&g.name);
        let at = out.iter().position(|(n, _)| rank(n) > r).unwrap_or(out.len());
        out.insert(at, (g.name.clone(), g_text(g)?));
    }
    let mut s = String::with_capacity(orig.len() + 256);
    s.push_str(root_tag);
    for (_, t) in &out {
        s.push_str(t);
    }
    s.push_str(orig.get(o_close..)?);
    Some(s.into_bytes())
}

/// Did `written` (a settings part) keep everything of `original` the model doesn't hold? Every
/// child [`merge`] keeps byte for byte must be there byte for byte, and `w:compat` must be the
/// original's but for its compatibility mode (a self-closing `<w:compat/>` the merge expanded to
/// hold the mode counts as kept). False when a child went (one referring to a relationship, or
/// all of them when the merge fell back) or `written` can't be scanned; true for an `original`
/// [`has_unowned`] wouldn't keep. A check for callers that refuse to lose settings silently.
pub fn kept_unowned(original: &[u8], written: &[u8]) -> bool {
    if !has_unowned(original) {
        return true;
    }
    let (Ok(orig), Ok(out)) = (std::str::from_utf8(original), std::str::from_utf8(written)) else { return false };
    let (Some((_, _, o_kids)), Some((_, _, w_kids))) = (children(orig, "w:settings"), children(out, "w:settings")) else {
        return false;
    };
    let text = |xml: &str, k: &Child| xml.get(k.start..k.end).map(str::to_string);
    let without_mode = |c: &str| -> String {
        let c = match c.find("w:name=\"compatibilityMode\"") {
            Some(at) => match (c.get(..at).and_then(|h| h.rfind('<')), c.get(at..).and_then(|t| t.find('>'))) {
                (Some(start), Some(len)) => format!("{}{}", c.get(..start).unwrap_or(""), c.get(at + len + 1..).unwrap_or("")),
                _ => c.to_string(),
            },
            None => c.to_string(),
        };
        // An empty block reads the same self-closing or not.
        let open = match c.find('>') {
            Some(gt) if c.get(gt + 1..) == Some("</w:compat>") => c.get(..gt),
            Some(_) if c.ends_with("/>") => c.get(..c.len() - 2),
            _ => None,
        };
        open.map_or(c.clone(), |o| format!("{}/>", o.trim_end()))
    };
    let mut used = vec![false; w_kids.len()];
    for k in &o_kids {
        let name = k.name.as_str();
        if OWNED.contains(&name) && !KEEP_ORIGINAL.contains(&name) {
            continue;
        }
        let Some(want) = text(orig, k) else { return false };
        let want = if name == "w:compat" { without_mode(&want) } else { want };
        let found = w_kids.iter().enumerate().position(|(i, w)| {
            !used[i] && w.name == k.name && text(out, w).is_some_and(|t| if name == "w:compat" { without_mode(&t) == want } else { t == want })
        });
        match found {
            Some(i) => used[i] = true,
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEN: &str = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:zoom w:percent="100"/><w:defaultTabStop w:val="720"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="u" w:val="15"/></w:compat></w:settings>"#;

    #[test]
    fn odd_or_hostile_originals_fall_back_or_merge_without_panicking() {
        let odd = [
            "",
            "<w:settings",
            "<w:settings/>",
            "<x:other/>",
            r#"<w:settings xmlns:w="w"><w:a><w:b></w:a></w:settings>"#,
            r#"<w:settings xmlns:w="w"><![CDATA[x]]></w:settings>"#,
            r#"<w:settings xmlns:w="w"></w:x></w:settings>"#,
            "<w:settings xmlns:w=\"w\">\u{1F600}<w:rsids/></w:settings>",
            r#"<w:settings xmlns:w="w"><!-- c --><?pi x?><w:rsids/></w:settings>"#,
        ];
        for o in odd {
            let _ = merge(o.as_bytes(), GEN.as_bytes());
            let _ = has_unowned(o.as_bytes());
        }
        assert!(merge(&[0xff, 0xfe], GEN.as_bytes()).is_none());
        // Missing declarations for a prefix the generated part uses: fall back.
        assert!(merge(br#"<w:settings xmlns:x="x"><w:rsids/></w:settings>"#, GEN.as_bytes()).is_none());
    }

    #[test]
    fn a_gt_inside_an_attribute_value_does_not_hide_the_kept_settings() {
        for v in [r#"w:val="x>y""#, "w:val='x>y'"] {
            let orig = format!(
                r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:docVar w:name="a" {v}/><w:docVar w:name="b" {v}></w:docVar></w:settings>"#
            );
            assert!(has_unowned(orig.as_bytes()), "{v}");
            let out = String::from_utf8(merge(orig.as_bytes(), GEN.as_bytes()).unwrap()).unwrap();
            assert!(out.contains(r#"w:name="a""#) && out.contains(r#"w:name="b""#), "{v}");
        }
    }

    #[test]
    fn a_compat_block_without_a_mode_gets_the_generated_one() {
        let orig = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:compat><w:useFELayout/></w:compat></w:settings>"#;
        let out = String::from_utf8(merge(orig.as_bytes(), GEN.as_bytes()).unwrap()).unwrap();
        assert!(out.contains("<w:useFELayout/>") && out.contains("compatibilityMode"), "{out}");
        assert!(has_unowned(orig.as_bytes()));
        assert!(!has_unowned(GEN.as_bytes()));
        assert!(kept_unowned(orig.as_bytes(), &merge(orig.as_bytes(), GEN.as_bytes()).unwrap()));
    }

    #[test]
    fn a_self_closing_compat_block_gets_the_generated_mode() {
        let orig = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:compat/><w:rsids><w:rsidRoot w:val="00112233"/></w:rsids></w:settings>"#;
        let out = String::from_utf8(merge(orig.as_bytes(), GEN.as_bytes()).unwrap()).unwrap();
        assert!(out.contains("<w:compat><w:compatSetting") && out.contains("</w:compat>"), "{out}");
        assert!(out.contains("<w:rsidRoot w:val=\"00112233\"/>"), "{out}");
    }

    #[test]
    fn kept_unowned_sees_a_lost_or_changed_child() {
        let orig = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:zoom w:percent="90"/><w:rsids><w:rsidRoot w:val="1"/></w:rsids><w:defaultTabStop w:val="708"/><w:compat><w:useFELayout/><w:compatSetting w:name="compatibilityMode" w:uri="u" w:val="14"/></w:compat></w:settings>"#;
        let merged = merge(orig.as_bytes(), GEN.as_bytes()).unwrap();
        assert!(kept_unowned(orig.as_bytes(), &merged), "{}", String::from_utf8_lossy(&merged));
        // The generated part alone (a merge that fell back) loses the rsids and the zoom.
        assert!(!kept_unowned(orig.as_bytes(), GEN.as_bytes()));
        // A changed compat option is a loss; a changed mode isn't.
        let compat = String::from_utf8(merged.clone()).unwrap().replace("<w:useFELayout/>", "");
        assert!(!kept_unowned(orig.as_bytes(), compat.as_bytes()));
        // A child referring to a relationship goes with the merge: that is a loss too.
        let mail = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:mailMerge><w:dataSource r:id="rId1"/></w:mailMerge></w:settings>"#;
        assert!(!kept_unowned(mail.as_bytes(), &merge(mail.as_bytes(), GEN.as_bytes()).unwrap()));
        assert!(!kept_unowned(orig.as_bytes(), &[0xff]));
        assert!(kept_unowned(GEN.as_bytes(), b""));
    }

    #[test]
    fn kept_unowned_takes_an_expanded_self_closing_compat_block() {
        let orig = r#"<w:settings xmlns:w="w" xmlns:m="m" xmlns:r="r"><w:compat/><w:rsids><w:rsidRoot w:val="00112233"/></w:rsids></w:settings>"#;
        let merged = merge(orig.as_bytes(), GEN.as_bytes()).unwrap();
        assert!(kept_unowned(orig.as_bytes(), &merged), "{}", String::from_utf8_lossy(&merged));
        // An option added to the block is a change, not the mode.
        let added = String::from_utf8(merged).unwrap().replace("<w:compat>", "<w:compat><w:useFELayout/>");
        assert!(!kept_unowned(orig.as_bytes(), added.as_bytes()));
    }
}
