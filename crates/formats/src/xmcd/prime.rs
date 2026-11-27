use std::collections::HashMap;

use base64::Engine as _;
use craft_zip::{Archive, Budget, Limits};
use wordcraft_doc::{Document, SectionProps};

use super::{Node, import_worksheet, position, read_xml};

const POINTS_PER_DIP: f64 = 0.75;
const MAX_XML: u64 = 64 * 1024 * 1024;
const MAX_PART: u64 = 16 * 1024 * 1024;

pub(super) fn namespace(uri: &[u8]) -> u8 {
    match uri {
        b"http://schemas.mathsoft.com/worksheet50" => 1,
        b"http://schemas.mathsoft.com/math50" => 2,
        b"http://schemas.microsoft.com/winfx/2006/xaml/presentation" => 3,
        b"clr-namespace:Ptc.Wpf;assembly=Ptc.Core" => 4,
        b"http://schemas.ptc.com/mathcad/settings/calculation10" | b"http://schemas.ptc.com/mathcad/settings/presentation10" => 5,
        b"http://schemas.openxmlformats.org/package/2006/relationships" => 6,
        _ => 0,
    }
}

pub(super) fn attribute(key: &str) -> bool {
    matches!(
        key,
        "Id" | "Target"
            | "TargetMode"
            | "Type"
            | "actualWidth"
            | "actualHeight"
            | "display-width"
            | "display-height"
            | "precision"
            | "labels"
            | "solve-block-category"
            | "paper-code"
            | "orientation"
            | "page-margin"
            | "margin-type"
            | "base-unit-system"
            | "TextAlignment"
            | "FontFamily"
            | "FontSize"
            | "FontWeight"
            | "FontStyle"
            | "Text"
            | "TextIndent"
            | "Margin"
    )
}

fn read_part(archive: &mut Archive<'_>, path: &str, max: u64) -> Result<Option<Node>, String> {
    archive.read(path, max).map_err(|e| e.to_string())?.map(|bytes| read_xml(&bytes, true)).transpose()
}

pub(crate) fn import(bytes: &[u8], page: Option<&SectionProps>) -> Result<Document, String> {
    let limits = Limits { max_entries: crate::MAX_ZIP_ENTRIES, max_total_bytes: 128 * 1024 * 1024 };
    let mut archive = Archive::open(bytes, limits).map_err(|error| format!("Invalid Mathcad Prime package: {error}"))?;
    if archive.has_duplicate_names() {
        return Err("Mathcad Prime package has duplicate part names".into());
    }
    let mut worksheet = read_part(&mut archive, "mathcad/worksheet.xml", MAX_XML)?
        .filter(|node| node.ns == 1 && node.name == "worksheet")
        .ok_or("Expected a Mathcad Prime worksheet")?;
    let relationships = read_part(&mut archive, "mathcad/_rels/worksheet.xml.rels", MAX_PART)?.unwrap_or_default();
    let targets: HashMap<_, _> = relationships.children.iter().filter_map(relationship).collect();
    let mut binary = HashMap::new();
    let mut flow_budget = Budget::new("Mathcad Prime text", 16 * 1024 * 1024);
    let regions =
        worksheet.children.iter_mut().find(|node| node.ns == 1 && node.name == "regions").ok_or("Mathcad Prime worksheet has no regions")?;
    normalize_regions(regions, &mut archive, &targets, &mut binary, &mut flow_budget)?;
    let mut settings = node("settings");
    if let Some(mut calculation) =
        read_part(&mut archive, "mathcad/settings/calculation.xml", MAX_PART)?.filter(|node| node.ns == 5 && node.name == "calculation")
    {
        reclassify_settings(&mut calculation);
        settings.children.push(calculation);
    }
    let presentation = read_part(&mut archive, "mathcad/settings/presentation.xml", MAX_PART)?;
    let section = normalize_page(presentation.as_ref(), page, &mut settings);
    worksheet.children.push(settings);
    worksheet.children.push(Node { children: binary.into_values().collect(), ..node("binaryContent") });
    import_worksheet(worksheet, Some(&section), &|_| None)
}

fn node(name: &str) -> Node {
    Node { ns: 1, name: name.into(), ..Node::default() }
}

fn relationship(rel: &Node) -> Option<(&str, String)> {
    if rel.ns != 6 || rel.name != "Relationship" || !matches!(rel.attr("TargetMode"), "" | "Internal") {
        return None;
    }
    let id = rel.attr("Id");
    let target = rel.attr("Target");
    let path = target.strip_prefix('/').map_or_else(|| format!("mathcad/{target}"), str::to_owned);
    if id.is_empty()
        || !path.starts_with("mathcad/")
        || path.split('/').any(|part| matches!(part, "" | "." | ".."))
        || path.contains(['\\', ':', '%', '?', '#'])
    {
        return None;
    }
    Some((id, path))
}

fn normalize_regions(
    regions: &mut Node,
    archive: &mut Archive<'_>,
    targets: &HashMap<&str, String>,
    binary: &mut HashMap<String, Node>,
    flow_budget: &mut Budget,
) -> Result<(), String> {
    for region in regions.children.iter_mut().filter(|node| node.ns == 1 && node.name == "region") {
        for (to, fallback) in [("top", "top"), ("left", "left"), ("width", "actualWidth"), ("height", "actualHeight")] {
            let key = if region.attr(to).is_empty() { fallback } else { to };
            if !region.attr(key).is_empty() {
                region.attrs.insert(to.into(), (position(region, key) * POINTS_PER_DIP).to_string());
            }
        }
        if let Some(contents) = region.children.iter_mut().find(|part| part.ns == 1 && matches!(part.name.as_str(), "Area" | "solveblock")) {
            let solve = contents.name == "solveblock";
            contents.name = "area".into();
            if let Some(nested) = contents.children.iter_mut().find(|part| part.ns == 1 && part.name == "regions") {
                normalize_regions(nested, archive, targets, binary, flow_budget)?;
                if solve {
                    add_given(nested);
                }
            }
        }
        if let Some(text) = region.children.iter_mut().find(|part| part.ns == 1 && part.name == "text") {
            let target = targets.get(text.attr("item-idref")).filter(|path| path.ends_with(".XamlPackage"));
            let flow = match target {
                Some(path) => read_flow(archive, path, flow_budget)?,
                None => None,
            };
            if let Some(flow) = flow {
                let mut paragraphs = Vec::new();
                flow_paragraphs(&flow, &FlowStyle::default(), &mut paragraphs);
                text.children = paragraphs;
            } else {
                text.name = "unavailable text".into();
            }
        }
        if let Some(png) = region.child(1, "picture").and_then(|picture| picture.child(1, "png")) {
            let id = png.attr("item-idref").to_owned();
            let dimensions = [("width", position(png, "display-width")), ("height", position(png, "display-height"))];
            for (key, size) in dimensions {
                if position(region, key) <= 0.0 && size > 0.0 {
                    region.attrs.insert(key.into(), (size * POINTS_PER_DIP).to_string());
                }
            }
            if let Some(target) = targets.get(id.as_str()).filter(|target| target.ends_with(".png")) {
                if !binary.contains_key(&id)
                    && let Some(bytes) = archive.read(target, MAX_PART).map_err(|e| e.to_string())?
                {
                    binary.insert(
                        id.clone(),
                        Node {
                            attrs: HashMap::from([("item-id".into(), id.clone())]),
                            text: base64::engine::general_purpose::STANDARD.encode(bytes),
                            ..node("item")
                        },
                    );
                }
                region.children.push(Node { attrs: HashMap::from([("item-idref".into(), id)]), ..node("rendering") });
            }
        }
        for part in &mut region.children {
            normalize_math(part, false);
        }
    }
    Ok(())
}

fn add_given(regions: &mut Node) {
    if !regions.children.iter().any(|region| region.attr("solve-block-category") == "solver" || region.child(1, "math").is_some_and(contains_solver))
    {
        return;
    }
    let Some(first) = regions
        .children
        .iter()
        .filter(|region| {
            region.attr("solve-block-category") == "constraint"
                || region
                    .child(1, "math")
                    .and_then(|math| math.math().next())
                    .and_then(|math| math.math().next())
                    .is_some_and(|op| op.name == "equal")
        })
        .min_by(|a, b| position(a, "top").total_cmp(&position(b, "top")))
    else {
        return;
    };
    let root = Node { ns: 2, name: "id".into(), text: "Given".into(), ..Node::default() };
    let math = Node { children: vec![root], ..node("math") };
    let attrs = HashMap::from([
        ("top".into(), (position(first, "top") - 0.01).to_string()),
        ("left".into(), position(first, "left").to_string()),
        ("region-id".into(), first.attr("region-id").to_owned()),
    ]);
    regions.children.push(Node { attrs, children: vec![math], ..node("region") });
}

fn contains_solver(node: &Node) -> bool {
    node.ns == 2 && node.name == "id" && matches!(node.text.trim(), "Find" | "find" | "Minerr" | "minerr")
        || node.children.iter().any(contains_solver)
}

fn normalize_math(node: &mut Node, display_unit: bool) {
    if node.ns == 2 && node.name == "id" && !node.children.is_empty() {
        let mut base = std::mem::take(&mut node.text);
        let mut subscript = String::new();
        identifier_parts(node, &mut base, &mut subscript);
        node.text = base;
        if !subscript.is_empty() {
            node.attrs.insert("subscript".into(), subscript);
        }
        node.children.clear();
    }
    if node.ns == 2
        && node.name == "id"
        && node.text.trim() == "lb"
        && node.attr("subscript").is_empty()
        && (display_unit || node.attr("labels") == "UNIT")
    {
        node.text = "lbm".into();
    }
    let display_unit = display_unit || node.ns == 2 && node.name == "unitOverride";
    if node.ns == 2 || node.ns == 1 && node.name == "math" {
        for child in &mut node.children {
            normalize_math(child, display_unit);
        }
    }
}

fn identifier_parts(node: &Node, base: &mut String, subscript: &mut String) {
    for child in &node.children {
        if child.ns == 4 && child.name == "Subscript" {
            collect_text(child, subscript);
        } else if child.ns == 3 || child.name.is_empty() {
            base.push_str(&child.text);
            identifier_parts(child, base, subscript);
        }
    }
}

fn collect_text(node: &Node, text: &mut String) {
    text.push_str(&node.text);
    for child in &node.children {
        collect_text(child, text);
    }
}

fn read_flow(archive: &mut Archive<'_>, path: &str, budget: &mut Budget) -> Result<Option<Node>, String> {
    let Some(bytes) = archive.read(path, MAX_PART).map_err(|e| e.to_string())? else { return Ok(None) };
    let limits = Limits { max_entries: 256, max_total_bytes: 2 * 1024 * 1024 };
    let mut package = Archive::open(&bytes, limits).map_err(|e| e.to_string())?;
    if package.has_duplicate_names() {
        return Err("Mathcad Prime text package has duplicate part names".into());
    }
    let Some(xml) = package.read("Xaml/Document.xaml", 256 * 1024).map_err(|e| e.to_string())? else { return Ok(None) };
    budget.charge(xml.len() as u64).map_err(|e| e.to_string())?;
    read_xml(&xml, true).map(Some)
}

#[derive(Clone, Default)]
struct FlowStyle {
    family: String,
    size: String,
    alignment: String,
    bold: bool,
    italic: bool,
}

impl FlowStyle {
    fn with(&self, source: &Node) -> Self {
        let mut style = self.clone();
        for (dest, key) in [(&mut style.family, "FontFamily"), (&mut style.size, "FontSize"), (&mut style.alignment, "TextAlignment")] {
            if !source.attr(key).is_empty() {
                *dest = source.attr(key).to_owned();
            }
        }
        if !source.attr("FontWeight").is_empty() {
            style.bold = matches!(source.attr("FontWeight"), "Bold" | "SemiBold" | "ExtraBold");
        }
        if !source.attr("FontStyle").is_empty() {
            style.italic = matches!(source.attr("FontStyle"), "Italic" | "Oblique");
        }
        style.bold |= source.name == "Bold";
        style.italic |= source.name == "Italic";
        style
    }
}

fn flow_paragraphs(source: &Node, inherited: &FlowStyle, out: &mut Vec<Node>) {
    if source.ns != 3 {
        return;
    }
    let style = inherited.with(source);
    if source.name == "Paragraph" {
        let mut paragraph = node("p");
        paragraph.attrs.insert("text-align".into(), style.alignment.to_ascii_lowercase());
        if let Some(indent) = points(source.attr("TextIndent")) {
            paragraph.attrs.insert("text-indent".into(), indent.to_string());
        }
        let margins: Vec<_> = source.attr("Margin").split(',').map(points).collect();
        if let [left, _, right, _] = margins.as_slice() {
            for (attr, value) in [("margin-left", left), ("margin-right", right)] {
                if let Some(value) = value {
                    paragraph.attrs.insert(attr.into(), value.to_string());
                }
            }
        }
        flow_runs(source, inherited, &mut paragraph.children);
        out.push(paragraph);
    } else {
        for child in &source.children {
            flow_paragraphs(child, &style, out);
        }
    }
}

fn flow_runs(source: &Node, inherited: &FlowStyle, out: &mut Vec<Node>) {
    if source.ns != 3 && !source.name.is_empty() {
        return;
    }
    if source.name == "LineBreak" {
        out.push(node("br"));
        return;
    }
    let style = inherited.with(source);
    let text = if source.text.is_empty() { source.attr("Text") } else { &source.text };
    if !text.is_empty() {
        let mut run = Node { text: text.to_owned(), ..node("f") };
        run.attrs.insert("family".into(), style.family.clone());
        if let Some(size) = points(&style.size) {
            run.attrs.insert("size".into(), size.to_string());
        }
        for (enabled, tag) in [(style.bold, "b"), (style.italic, "i")] {
            if enabled {
                run = Node { children: vec![run], ..node(tag) };
            }
        }
        out.push(run);
    }
    for child in &source.children {
        flow_runs(child, &style, out);
    }
}

fn reclassify_settings(node: &mut Node) {
    if node.ns == 5 {
        node.ns = 1;
        for child in &mut node.children {
            reclassify_settings(child);
        }
    }
}

fn points(value: &str) -> Option<f32> {
    let value = value.parse::<f32>().ok()? * POINTS_PER_DIP as f32;
    value.is_finite().then_some(value)
}

fn normalize_page(presentation: Option<&Node>, page: Option<&SectionProps>, settings: &mut Node) -> SectionProps {
    let mut section = page.cloned().unwrap_or_default();
    let Some(model) = presentation.and_then(|presentation| presentation.child(5, "pageModel")) else { return section };
    let size = match model.attr("paper-code") {
        "Letter" => Some((612.0, 792.0)),
        "Legal" => Some((612.0, 1008.0)),
        "A4" => Some((595.28, 841.89)),
        "A3" => Some((841.89, 1190.55)),
        _ => None,
    };
    if let Some((width, height)) = size {
        (section.page_w, section.page_h) = if model.attr("orientation") == "Landscape" { (height, width) } else { (width, height) };
    }
    let margins: Option<Vec<_>> = model.attr("page-margin").split(',').map(points).collect();
    if let Some([left, top, right, bottom]) = margins.as_deref()
        && [left, top, right, bottom].into_iter().all(|margin| (0.0..=250.0).contains(margin))
        && left + right + 18.0 < section.page_w
        && top + bottom + 18.0 < section.page_h
    {
        section.margin_left = *left;
        section.margin_top = *top;
        section.margin_right = *right;
        section.margin_bottom = *bottom;
    } else if model.attr("margin-type") == "Narrow" {
        section.margin_left = 36.0;
        section.margin_top = 36.0;
        section.margin_right = 36.0;
        section.margin_bottom = 36.0;
    }
    let margins = Node {
        attrs: HashMap::from([("top".into(), section.margin_top.to_string()), ("bottom".into(), section.margin_bottom.to_string())]),
        ..node("margins")
    };
    let model = Node { attrs: HashMap::from([("page-height".into(), section.page_h.to_string())]), children: vec![margins], ..node("pageModel") };
    let mut normalized = Node { children: vec![model], ..node("presentation") };
    if let Some(style) = presentation
        .and_then(|node| node.child(5, "mathRendering"))
        .and_then(|node| node.child(5, "mathStyles"))
        .and_then(|node| node.children.iter().find(|style| style.ns == 5 && style.attr("name") == "Variable"))
        && let Some(size) = points(style.attr("font-size"))
    {
        let mut style = style.clone();
        style.ns = 1;
        style.attrs.insert("font-size".into(), size.to_string());
        let styles = Node { children: vec![style], ..node("mathStyles") };
        normalized.children.push(Node { children: vec![styles], ..node("mathRendering") });
    }
    settings.children.push(normalized);
    section
}
