use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use wordcraft_doc::para::{WorksheetNote, WorksheetNoteKind};

use base64::Engine as _;
use quick_xml::{XmlVersion, events::Event, name::ResolveResult, reader::NsReader};
use wordcraft_calc::{Expr, Stmt};
use wordcraft_doc::{
    Block, Document, Paragraph, SectionProps, Table,
    calc::{CalcRegion, CalcSettings, CalcState, NumberStyle, ResultFormat, UnitSystem},
    para::InlineObject,
    props::{Align, CharProps},
};

mod layout;
mod links;
pub(super) mod prime;
mod program;

const WORKSHEET: &[u8] = b"http://schemas.mathsoft.com/worksheet30";
const MATH: &[u8] = b"http://schemas.mathsoft.com/math30";

#[derive(Clone, Default)]
struct Node {
    ns: u8,
    name: String,
    attrs: HashMap<String, String>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn attr(&self, key: &str) -> &str {
        self.attrs.get(key).map_or("", String::as_str)
    }

    fn child(&self, ns: u8, name: &str) -> Option<&Node> {
        self.children.iter().find(|node| node.ns == ns && node.name == name)
    }

    fn math(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(|node| node.ns == 2)
    }
}

fn append_text(
    node: &Node,
    paragraph: &mut Paragraph,
    inherited: &CharProps,
    doc: &mut Document,
    binary: &HashMap<&str, &Node>,
    names: &mut Names,
    next_inline_id: &mut u64,
) -> Result<(), String> {
    let mut props = inherited.clone();
    if node.ns == 1 {
        match node.name.as_str() {
            "region" if let Some(math) = node.child(1, "math") => {
                let disabled = math.attr("disable-calc") == "true";
                if !disabled && let Some(root) = math.math().next() {
                    let mut scoped = names.clone();
                    if let Some(source) = calculation(root, &mut scoped, true)
                        && matches!(wordcraft_calc::parse(&source), Ok(Stmt::Define { .. } | Stmt::DefineMany { .. }))
                    {
                        let id = *next_inline_id;
                        *next_inline_id += 1;
                        let calc = CalcRegion {
                            result_format: math.child(1, "resultFormat").and_then(result_format),
                            worksheet_order: None,
                            id,
                            source,
                            unit: None,
                            precision: None,
                            numbered: false,
                            state: CalcState::Fresh,
                        };
                        paragraph
                            .insert_object(
                                paragraph.len(),
                                InlineObject::Equation {
                                    linear: String::new(),
                                    display: false,
                                    math: Default::default(),
                                    calc: Some(Box::new(calc)),
                                },
                                &props,
                            )
                            .map_err(|error| error.to_string())?;
                        *names = scoped;
                        return Ok(());
                    }
                }
                let preview = if disabled { None } else { saved_preview(doc, node, binary, "Mathcad equation".into()) };
                if let Some(object) = preview.or_else(|| static_equation(math, false)).or_else(|| blank_comparison(math)) {
                    paragraph.insert_object(paragraph.len(), object, &props).map_err(|error| error.to_string())?;
                } else {
                    let marker = note(node, "embedded equation");
                    let off = paragraph.len();
                    paragraph.insert_text(off, &marker.text, &props).map_err(|error| error.to_string())?;
                    paragraph.worksheet_notes.push(WorksheetNote { kind: WorksheetNoteKind::Unsupported, off, message: marker.text });
                }
                return Ok(());
            }
            "b" => props.bold = Some(true),
            "i" => props.italic = Some(true),
            "inlineAttr" => layout::character_attrs(&mut props, |key| node.attr(key)),
            "f" => {
                if let Some(size) = node.attr("size").parse::<f32>().ok().filter(|size| (1.0..=400.0).contains(size)) {
                    props.size = Some(size);
                }
                if !node.attr("family").is_empty() {
                    props.font = Some(node.attr("family").to_owned());
                }
            }
            "sp" | "tab" => {
                let count = node.attr("count").parse::<usize>().unwrap_or(1).clamp(1, 256);
                paragraph.insert_text(paragraph.len(), &" ".repeat(count), &props).map_err(|error| error.to_string())?;
            }
            "br" => {
                paragraph.insert_text(paragraph.len(), "\n", &props).map_err(|error| error.to_string())?;
            }
            _ => {}
        }
    }
    if !node.text.trim().is_empty() || (!node.text.is_empty() && !node.text.contains(['\r', '\n'])) {
        paragraph.insert_text(paragraph.len(), &node.text, &props).map_err(|error| error.to_string())?;
    }
    for child in &node.children {
        append_text(child, paragraph, &props, doc, binary, names, next_inline_id)?;
    }
    Ok(())
}

fn read_xml(bytes: &[u8], prime: bool) -> Result<Node, String> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("Mathcad worksheet exceeds 64 MiB".into());
    }
    let decoded = super::txt::decode(bytes);
    let mut reader = NsReader::from_str(&decoded);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = vec![Node::default()];
    let mut count = 0;
    loop {
        match reader.read_event().map_err(|error| error.to_string())? {
            Event::Start(element) => {
                count += 1;
                if stack.len() > 64 || count > 500_000 {
                    return Err("Mathcad worksheet is too complex".into());
                }
                let (namespace, local) = reader.resolver().resolve_element(element.name());
                let ns = match namespace {
                    ResolveResult::Bound(uri) if !prime && uri.as_ref() == WORKSHEET => 1,
                    ResolveResult::Bound(uri) if !prime && uri.as_ref() == MATH => 2,
                    ResolveResult::Bound(uri) if prime => prime::namespace(uri.as_ref()),
                    _ => 0,
                };
                let mut node = Node { ns, name: String::from_utf8_lossy(local.as_ref()).into_owned(), ..Node::default() };
                for attr in element.attributes() {
                    let attr = attr.map_err(|error| error.to_string())?;
                    let key = String::from_utf8_lossy(attr.key.as_ref());
                    if matches!(
                        key.as_ref(),
                        "region-id"
                            | "align-y"
                            | "top"
                            | "bottom"
                            | "left"
                            | "right"
                            | "width"
                            | "height"
                            | "page-width"
                            | "page-height"
                            | "print-single-page-width"
                            | "is-collapsed"
                            | "font-family"
                            | "font-size"
                            | "font-weight"
                            | "font-style"
                            | "family"
                            | "size"
                            | "item-id"
                            | "item-idref"
                            | "clsid-buddy"
                            | "content-encoding"
                            | "style"
                            | "base-style"
                            | "text-align"
                            | "text-indent"
                            | "margin-left"
                            | "margin-right"
                            | "subscript"
                            | "rows"
                            | "cols"
                            | "count"
                            | "array-origin"
                            | "disable-calc"
                            | "name"
                            | "fixity"
                            | "precision"
                            | "show-trailing-zeros"
                            | "exponential-threshold"
                            | "use-e-notation"
                            | "use-mixed-numbers"
                            | "use-engineering-format"
                            | "engineering-format"
                            | "engineering-exponents"
                    ) || prime && prime::attribute(&key)
                    {
                        let value =
                            attr.decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder()).map_err(|error| error.to_string())?;
                        node.attrs.insert(key.into_owned(), value.into_owned());
                    }
                }
                stack.push(node);
            }
            Event::End(_) => {
                let node = stack.pop().ok_or("Invalid Mathcad XML")?;
                stack.last_mut().ok_or("Invalid Mathcad XML")?.children.push(node);
            }
            Event::Text(value) => {
                let value = value.decode().map_err(|error| error.to_string())?;
                push_text(&mut stack, &value)?;
            }
            Event::CData(value) => {
                let value = value.decode().map_err(|error| error.to_string())?;
                push_text(&mut stack, &value)?;
            }
            Event::GeneralRef(reference) => {
                if let Some(value) = reference.resolve_char_ref().map_err(|error| error.to_string())? {
                    push_text(&mut stack, value.encode_utf8(&mut [0; 4]))?;
                } else {
                    let name = reference.decode().map_err(|error| error.to_string())?;
                    let value = quick_xml::escape::resolve_predefined_entity(&name).ok_or("Unknown Mathcad XML entity")?;
                    push_text(&mut stack, value)?;
                }
            }
            Event::DocType(_) => return Err("Mathcad XML document types are unsupported".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    let [outer] = stack.as_mut_slice() else { return Err("Unclosed Mathcad XML element".into()) };
    if !outer.text.trim().is_empty() || outer.children.len() != 1 {
        return Err("Expected one Mathcad worksheet".into());
    }
    outer.children.pop().ok_or_else(|| "Expected one Mathcad XML element".into())
}

fn push_text(stack: &mut [Node], value: &str) -> Result<(), String> {
    let node = stack.last_mut().ok_or("Invalid Mathcad XML")?;
    if let Some(tail) = node.children.last_mut().filter(|tail| tail.name.is_empty()) {
        tail.text.push_str(value);
    } else if node.children.is_empty() {
        node.text.push_str(value);
    } else if !value.trim().is_empty()
        || matches!(node.ns, 3 | 4)
        || node.ns == 1 && matches!(node.name.as_str(), "p" | "b" | "i" | "u" | "f" | "c" | "sub" | "sup" | "inlineAttr")
    {
        node.children.push(Node { text: value.to_owned(), ..Node::default() });
    }
    Ok(())
}

fn identifier(node: &Node) -> Option<String> {
    let base = node.text.trim();
    let subscript = node.attr("subscript");
    let name = if subscript.is_empty() { base.to_owned() } else { format!("{base}_{subscript}") };
    let mut chars = name.chars();
    if !chars.next().is_some_and(|ch| ch.is_alphabetic() || ch == '°')
        || !chars.all(|ch| ch.is_alphabetic() || ch.is_ascii_digit() || matches!(ch, '_' | '.' | '°' | '/'))
    {
        return None;
    }
    Some(name)
}

#[derive(Clone, Default)]
struct Names {
    used: HashSet<String>,
    current: HashMap<String, String>,
    next: HashMap<String, usize>,
    normalized: HashMap<String, String>,
    origin: u8,
    source_origin: u8,
    contiguous_ranges: HashSet<String>,
    zero_definitions: HashSet<String>,
    program_locals: HashMap<String, String>,
    in_program: bool,
}

impl Names {
    fn identifier(&self, node: &Node) -> Option<String> {
        let raw = node.text.trim();
        if node.attr("subscript").is_empty()
            && let Some(alias) = self.normalized.get(raw)
        {
            return Some(alias.clone());
        }
        let name = identifier(node)?;
        Some(self.normalized.get(&name).cloned().unwrap_or(name))
    }

    fn definition(&mut self, name: &str) -> String {
        let renamed = if self.current.contains_key(name) || wordcraft_calc::check_name(name).is_err() {
            let next = self.next.entry(name.to_owned()).or_insert(2);
            loop {
                let candidate = if *next <= 26 { format!("{name}{}", char::from(b'a' + (*next as u8 - 1))) } else { format!("{name}_{next}") };
                *next += 1;
                if wordcraft_calc::check_name(&candidate).is_ok() && self.used.insert(candidate.clone()) {
                    break candidate;
                }
            }
        } else {
            name.to_owned()
        };
        self.current.insert(name.to_owned(), renamed.clone());
        renamed
    }

    fn reserve(&mut self, node: &Node, needs_alias: &mut Vec<String>) {
        if node.ns == 2
            && node.name == "id"
            && let Some(name) = identifier(node)
        {
            if name.contains(['.', '/']) || name.split('_').any(str::is_empty) {
                needs_alias.push(name);
            } else {
                self.used.insert(name);
            }
        }
        for child in &node.children {
            self.reserve(child, needs_alias);
        }
    }

    fn index(&self, value: &str) -> String {
        match i16::from(self.origin) - i16::from(self.source_origin) {
            0 => value.to_owned(),
            offset => format!("(({value}) + ({offset}))"),
        }
    }

    fn reserve_component_output(&mut self, node: &Node) {
        if identifier(node).is_some() || !node.attr("subscript").is_empty() {
            return;
        }
        let raw = node.text.trim();
        if raw.is_empty() || self.normalized.contains_key(raw) {
            return;
        }
        let label: String = raw.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let base = format!("Worksheet_{}", label.trim_matches('_'));
        let mut alias = base.clone();
        let mut suffix = 2;
        while wordcraft_calc::check_name(&alias).is_err() || !self.used.insert(alias.clone()) {
            alias = format!("{base}{suffix}");
            suffix += 1;
        }
        self.normalized.insert(raw.to_owned(), alias);
    }

    fn normalize_aliases(&mut self, needs_alias: Vec<String>) {
        let mut suffixes = HashMap::new();
        for name in needs_alias {
            if self.normalized.contains_key(&name) {
                continue;
            }
            let base = name.split(['_', '.', '/']).filter(|part| !part.is_empty()).collect::<Vec<_>>().join("_");
            let mut candidate = base.clone();
            let suffix = suffixes.entry(base.clone()).or_insert(2);
            while wordcraft_calc::check_name(&candidate).is_err() || !self.used.insert(candidate.clone()) {
                candidate = format!("{base}_{suffix}");
                *suffix += 1;
            }
            self.normalized.insert(name, candidate);
        }
    }
}

fn pointwise(node: &Node) -> bool {
    match node.name.as_str() {
        "id" | "real" => node.math().next().is_none(),
        "matrix" => node.math().all(|item| item.name == "real" || item.name == "id"),
        "parens" | "provenance" => {
            let mut children = node.math();
            children.next().is_some_and(pointwise) && children.next().is_none()
        }
        "apply" => {
            let mut children = node.math();
            match children.next().map(|operator| operator.name.as_str()) {
                Some("nthRoot") => {
                    children.next().is_some_and(|degree| degree.name == "placeholder" || degree.name == "real") && children.all(pointwise)
                }
                Some("plus" | "minus" | "mult" | "scale" | "div" | "pow" | "neg" | "sqrt" | "absval") => children.all(pointwise),
                _ => false,
            }
        }
        _ => false,
    }
}

fn expr(node: &Node, names: &Names, locals: &[String]) -> Option<String> {
    let children: Vec<_> = node.math().collect();
    let result = match (node.name.as_str(), children.as_slice()) {
        ("id", []) if node.text.trim() == "%" => "0.01".into(),
        ("id", []) => {
            let name = names.identifier(node)?;
            if let Some(local) = names.program_locals.get(&name) {
                local.clone()
            } else if locals.contains(&name) {
                name
            } else if let Some(current) = names.current.get(&name) {
                current.clone()
            } else if name == "R" {
                "°R".into()
            } else if name == "lb" {
                "lbm".into()
            } else {
                name
            }
        }
        ("real", []) => {
            let value = node.text.trim();
            value.parse::<f64>().ok().filter(|number| number.is_finite())?;
            if value.starts_with('-') { format!("({value})") } else { value.trim_start_matches('+').to_owned() }
        }
        ("mixed", [whole, numerator, denominator]) if whole.name == "whole" && numerator.name == "numerator" && denominator.name == "denominator" => {
            let whole = fraction_part(whole)?.parse::<i64>().ok()?;
            let numerator = fraction_part(numerator)?.parse::<u64>().ok()?;
            let denominator = fraction_part(denominator)?.parse::<u64>().ok().filter(|v| *v > 0)?;
            let sign = if whole < 0 { "-" } else { "+" };
            format!("({whole} {sign} ({numerator} / {denominator}))")
        }
        ("str", []) if !node.text.contains(['"', '\\', '\n', '\r']) => format!("\"{}\"", node.text),
        ("str", []) => return worksheet_data(1, 1, std::iter::once(node)),
        ("provenance", [inner]) => return expr(inner, names, locals),
        ("parens", [inner]) => format!("({})", expr(inner, names, locals)?),
        ("apply", [operator, value]) if node.attr("fixity") == "postfix" && operator.name == "id" => {
            let unit = expr(operator, names, locals)?;
            if !matches!(unit.as_str(), "°F" | "°C" | "°R") {
                return None;
            }
            temperature(value, &unit, names, locals)?
        }
        ("apply", [operator, value, unit]) if operator.name == "scale" && unit.name == "id" => {
            let unit = expr(unit, names, locals)?;
            if matches!(unit.as_str(), "°F" | "°C" | "°R") {
                temperature(value, &unit, names, locals)?
            } else {
                format!("({} * {unit})", expr(value, names, locals)?)
            }
        }
        ("apply", [operator, operands @ ..]) => apply(operator, operands, names, locals)?,
        ("matrix", items) if names.in_program => {
            let rows = node.attr("rows").parse::<usize>().ok()?;
            let cols = node.attr("cols").parse::<usize>().ok()?;
            if rows.checked_mul(cols)? != items.len() || items.is_empty() {
                return None;
            }
            format!("worksheetCells({rows}, {cols}, {})", expressions(items, names, locals)?.join(", "))
        }
        ("matrix", items) => {
            let rows = node.attr("rows").parse::<usize>().ok()?;
            let cols = node.attr("cols").parse::<usize>().ok()?;
            if cols == 0 || rows == 0 || rows.checked_mul(cols)? != items.len() || items.len() > 10_000 {
                return None;
            }
            if items.len() > 4096 || (items.iter().any(|cell| cell.name == "str") && items.iter().any(|cell| cell.name == "real")) {
                return worksheet_data(rows, cols, items.iter().copied());
            }
            let items = expressions(items, names, locals)?;
            if rows == 1 && cols > 1 {
                return Some(format!("transpose([{}])", items.join("; ")));
            }
            let rows: Vec<_> = items.chunks(cols).map(|row| row.join(", ")).collect();
            format!("[{}]", rows.join("; "))
        }
        ("range", [sequence, last]) if sequence.name == "sequence" => {
            let [first, second] = sequence.math().collect::<Vec<_>>().try_into().ok()?;
            format!("{}, {}..{}", expr(first, names, locals)?, expr(second, names, locals)?, expr(last, names, locals)?)
        }
        ("range", [first, last]) => format!("{}..{}", expr(first, names, locals)?, expr(last, names, locals)?),
        ("range", [first, second, last]) => {
            format!("{}, {}..{}", expr(first, names, locals)?, expr(second, names, locals)?, expr(last, names, locals)?)
        }
        ("program" | "test" | "then" | "else", [expression]) => return expr(expression, names, locals),
        ("program", [branch, fallback]) if branch.name == "ifThen" && fallback.name == "otherwise" => {
            let [test, yes] = branch.math().collect::<Vec<_>>().try_into().ok()?;
            let [no] = fallback.math().collect::<Vec<_>>().try_into().ok()?;
            let (yes, no) = if yes.name == "localDefine" && no.name == "localDefine" {
                let [yes_name, yes] = yes.math().collect::<Vec<_>>().try_into().ok()?;
                let [no_name, no] = no.math().collect::<Vec<_>>().try_into().ok()?;
                if yes_name.name != "id" || no_name.name != "id" || identifier(yes_name)? != identifier(no_name)? {
                    return None;
                }
                (yes, no)
            } else {
                (yes, no)
            };
            format!("if({}, {}, {})", expr(test, names, locals)?, expr(yes, names, locals)?, expr(no, names, locals)?)
        }
        ("program", _) => return program::source(node, names, locals),
        ("if", [test, yes, no]) if test.name == "test" && yes.name == "then" && no.name == "else" => {
            format!("if({}, {}, {})", expr(test, names, locals)?, expr(yes, names, locals)?, expr(no, names, locals)?)
        }
        _ => return None,
    };
    (result.len() <= 16 * 1024).then_some(result)
}

fn is_legacy_zero(node: &Node, names: &Names, locals: &[String]) -> bool {
    match node.name.as_str() {
        "real" => node.text.trim().parse::<f64>().ok() == Some(0.0),
        "id" => names
            .identifier(node)
            .is_some_and(|name| !locals.contains(&name) && !names.program_locals.contains_key(&name) && names.zero_definitions.contains(&name)),
        "parens" | "provenance" => node.math().next().is_some_and(|inner| is_legacy_zero(inner, names, locals)),
        _ => false,
    }
}

fn fraction_part(node: &Node) -> Option<&str> {
    let mut children = node.math();
    match (children.next(), children.next()) {
        (Some(real), None) if real.name == "real" => Some(real.text.trim()),
        (None, None) => Some(node.text.trim()),
        _ => None,
    }
}

fn temperature(value: &Node, unit: &str, names: &Names, locals: &[String]) -> Option<String> {
    let source = expr(value, names, locals)?;
    if matches!(unit, "°F" | "°C") && value.name != "real" {
        Some(format!(r#"worksheetTemperature({source}, "{unit}")"#))
    } else {
        Some(format!("({source} {unit})"))
    }
}

fn has_offset_scale(node: &Node) -> bool {
    let parts = node.math().collect::<Vec<_>>();
    match (node.name.as_str(), parts.as_slice()) {
        ("parens" | "provenance", [inner]) => has_offset_scale(inner),
        ("apply", [unit, _]) if node.attr("fixity") == "postfix" => unit.name == "id" && matches!(unit.text.trim(), "°F" | "°C"),
        ("apply", [op, _, unit]) if op.name == "scale" => unit.name == "id" && matches!(unit.text.trim(), "°F" | "°C"),
        _ => false,
    }
}

fn worksheet_data<'a>(rows: usize, cols: usize, cells: impl Iterator<Item = &'a Node>) -> Option<String> {
    use wordcraft_calc::worksheet_data::{WorksheetCell, source};
    let cells: Vec<_> = cells
        .map(|cell| match cell.name.as_str() {
            "real" => cell.text.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(WorksheetCell::Number),
            "str" | "empty" => Some(WorksheetCell::Text(cell.text.clone())),
            _ => None,
        })
        .collect::<Option<_>>()?;
    source(rows, cols, &cells)
}

fn expressions(nodes: &[&Node], names: &Names, locals: &[String]) -> Option<Vec<String>> {
    let mut length = 0;
    nodes
        .iter()
        .map(|node| {
            let source = expr(node, names, locals)?;
            length += source.len() + 2;
            (length <= 16 * 1024).then_some(source)
        })
        .collect()
}

fn apply(operator: &Node, operands: &[&Node], names: &Names, locals: &[String]) -> Option<String> {
    let operands: Vec<_> = match operands {
        [sequence] if sequence.name == "sequence" => sequence.math().collect(),
        [base, sequence] if operator.name == "indexer" && sequence.name == "sequence" => std::iter::once(*base).chain(sequence.math()).collect(),
        _ => operands.to_vec(),
    };
    if let ("nthRoot", [degree, value]) = (operator.name.as_str(), operands.as_slice()) {
        let value = expr(value, names, locals)?;
        return if degree.name == "placeholder" || degree.name == "real" && degree.text.trim() == "2" {
            Some(format!("sqrt({value})"))
        } else {
            Some(format!("({value} ^ (1 / {}))", expr(degree, names, locals)?))
        };
    }
    let args = expressions(&operands, names, locals)?;
    match (operator.name.as_str(), args.as_slice()) {
        ("vectorize", [value]) if operands.first().is_some_and(|operand| pointwise(operand)) => Some(value.clone()),
        ("Find" | "Minerr", _) => Some(format!("{}({})", operator.name, args.join(", "))),
        ("id", [length]) if operator.text.trim() == "/FIF" && operator.attr("subscript").is_empty() => Some(format!("feetInches({length})")),
        ("id", _) => {
            let name = expr(operator, names, locals)?;
            let has_zero = names.in_program || operands.iter().any(|n| is_legacy_zero(n, names, locals));
            let name = match name.as_str() {
                "find" => "Find",
                "minerr" => "Minerr",
                "augment" => "worksheetAugment",
                "stack" if names.in_program => "worksheetStack",
                "concat" => "worksheetConcat",
                "num2str" => "worksheetNumberText",
                "min" if has_zero => "worksheetMin",
                "max" if has_zero => "worksheetMax",
                other => other,
            };
            let mut args = args.to_vec();
            if name == "vlookup" && args.len() == 3 {
                args[2] = names.index(&args[2]);
            }
            if name == "submatrix" && args.len() == 5 {
                for arg in &mut args[1..] {
                    *arg = names.index(arg);
                }
            }
            Some(format!("{name}({})", args.join(", ")))
        }
        ("matcol", [base, column]) => Some(format!("matcol({base}, {})", names.index(column))),
        ("indexer", [base, indexes @ ..]) if matches!(indexes.len(), 1 | 2) => {
            let indexes = indexes.iter().map(|value| names.index(value)).collect::<Vec<_>>().join(", ");
            if operands.first().is_some_and(|operand| operand.name == "id") {
                Some(format!("{base}[{indexes}]"))
            } else {
                Some(format!("worksheetAt({base}, {indexes})"))
            }
        }
        ("equal" | "notEqual", [left, right]) if names.in_program => {
            let function = if operator.name == "equal" { "worksheetEqual" } else { "worksheetNotEqual" };
            Some(format!("{function}({left}, {right})"))
        }
        ("neg" | "minus", [value]) => Some(format!("(-({value}))")),
        ("sqrt" | "absval" | "not" | "transpose" | "vectorSum", [value]) => {
            let function = match operator.name.as_str() {
                "absval" => "abs",
                "vectorSum" => "sum",
                name => name,
            };
            Some(format!("{function}({value})"))
        }
        ("plus" | "minus", [left, right]) if operands.iter().any(|node| is_legacy_zero(node, names, locals)) => {
            let function = if operator.name == "plus" { "worksheetAdd" } else { "worksheetSubtract" };
            Some(format!("{function}({left}, {right})"))
        }
        ("minus", [left, right]) if operands.iter().all(|node| has_offset_scale(node)) => {
            Some(format!("worksheetTemperatureDifference({left}, {right})"))
        }
        (name, [left, right]) => {
            let symbol = match name {
                "plus" => "+",
                "minus" => "-",
                "mult" | "scale" => "*",
                "div" => "/",
                "pow" => "^",
                "equal" => "==",
                "notEqual" => "!=",
                "lessThan" => "<",
                "greaterThan" => ">",
                "lessOrEqual" => "<=",
                "greaterOrEqual" => ">=",
                "and" => "and",
                "or" => "or",
                _ => return None,
            };
            Some(format!("({left} {symbol} {right})"))
        }
        _ => None,
    }
}

fn closes_solve(stmt: &Stmt) -> bool {
    let expr = match stmt {
        Stmt::Define { body, .. } | Stmt::DefineMany { body, .. } => body,
        Stmt::Eval { expr, .. } => expr,
        Stmt::Given | Stmt::Constraint { .. } => return false,
    };
    matches!(expr, Expr::Call(name, _) if matches!(name.as_str(), "Find" | "Minerr"))
}

fn saved_scalar(math: &Node) -> Option<f64> {
    let mut evaluation = math.math().next()?;
    while evaluation.name == "provenance" {
        evaluation = evaluation.math().next()?;
    }
    if evaluation.name != "eval" {
        return None;
    }
    let result = evaluation.child(2, "result")?;
    let mut values = result.math();
    let value = values.next()?;
    if value.name != "real" || values.next().is_some() {
        return None;
    }
    value.text.trim().parse::<f64>().ok().filter(|value| value.is_finite())
}

fn evaluation(node: &Node, names: &Names, locals: &[String]) -> Option<String> {
    let mut children = node.math().filter(|child| child.name != "result");
    let expression = expr(children.next()?, names, locals)?;
    match (children.next(), children.next()) {
        (None, None) => Some(format!("{expression} =")),
        (Some(unit), None) if unit.name == "unitOverride" => {
            let units: Vec<_> = unit.math().collect();
            match units.as_slice() {
                [] => Some(format!("{expression} =")),
                [unit] if unit.name == "placeholder" => Some(format!("{expression} =")),
                [unit] => {
                    let alias = (unit.name == "id")
                        .then(|| names.identifier(unit))
                        .flatten()
                        .filter(|name| name != "R")
                        .and_then(|name| names.current.get(&name));
                    let unit = match alias {
                        Some(alias) => alias.clone(),
                        None if unit.name == "id" && unit.text.trim() == "g" => "g_0".into(),
                        None => display_unit(unit, 0)?,
                    };
                    Some(format!("{expression} = ? {unit}"))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn display_unit(node: &Node, minimum: u8) -> Option<String> {
    let children: Vec<_> = node.math().collect();
    let (text, precedence) = match (node.name.as_str(), children.as_slice()) {
        ("parens" | "provenance", [inner]) => return display_unit(inner, minimum),
        ("apply", [operator, left, right]) if matches!(operator.name.as_str(), "mult" | "div") => {
            let symbol = if operator.name == "mult" { "·" } else { "/" };
            (format!("{}{symbol}{}", display_unit(left, 1)?, display_unit(right, 2)?), 1)
        }
        ("apply", [operator, base, power]) if operator.name == "pow" && power.name == "real" => {
            let exponent = power.text.trim().parse::<i32>().ok()?;
            let digits = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
            let suffix = digits.get(exponent.unsigned_abs() as usize).map_or_else(
                || format!("^{exponent}"),
                |digit| {
                    let sign = if exponent < 0 { "⁻" } else { "" };
                    format!("{sign}{digit}")
                },
            );
            (format!("{}{suffix}", display_unit(base, 3)?), 2)
        }
        _ => (expr(node, &Names::default(), &[])?, 3),
    };
    if precedence < minimum { Some(format!("({text})")) } else { Some(text) }
}

fn calculation(mut node: &Node, names: &mut Names, live: bool) -> Option<String> {
    while node.name == "provenance" {
        node = node.math().next()?;
    }
    match node.name.as_str() {
        "eval" => evaluation(node, names, &[]),
        "define" | "globalDefine" => {
            let items: Vec<_> = node.math().collect();
            let [lhs, rhs] = items.as_slice() else { return None };
            let mut index = None;
            let flatten =
                lhs.name == "matrix" && rhs.name == "matrix" && lhs.attr("rows") == rhs.attr("rows") && lhs.attr("cols") == rhs.attr("cols");
            let (targets, locals) = match lhs.name.as_str() {
                "id" => (vec![names.identifier(lhs)?], None),
                "apply" => {
                    let parts: Vec<_> = lhs.math().collect();
                    let [op, name, at] = parts.as_slice() else { return None };
                    if op.name != "indexer" {
                        return None;
                    }
                    let coordinates = if at.name == "sequence" { at.math().collect::<Vec<_>>() } else { vec![*at] };
                    if (1..=2).contains(&coordinates.len()) && coordinates.iter().all(|at| at.name == "real") {
                        let target = id_node(name, names)?;
                        let previous = names.current.get(&target).cloned().unwrap_or_else(|| "0".into());
                        let indexes = expressions(&coordinates, names, &[])?.iter().map(|at| names.index(at)).collect::<Vec<_>>();
                        let rhs = expr(rhs, names, &[])?;
                        let target = if live { names.definition(&target) } else { target };
                        return Some(format!("{target} := worksheetSetAt({previous}, {rhs}, {})", indexes.join(", ")));
                    }
                    let variable = if at.name == "sequence" {
                        let coordinates: Vec<_> = at.math().collect();
                        let [variable, column] = coordinates.as_slice() else { return None };
                        if column.name != "real" || column.text.trim().parse::<u8>().ok()? != names.source_origin {
                            return None;
                        }
                        *variable
                    } else {
                        *at
                    };
                    let variable = id_node(variable, names)?;
                    index = Some(variable);
                    (vec![id_node(name, names)?], None)
                }
                "matrix" | "sequence" if flatten || lhs.name == "sequence" || lhs.attr("cols") == "1" => {
                    let targets: Vec<_> = lhs.math().map(|node| id_node(node, names)).collect::<Option<_>>()?;
                    if targets.len() < 2 || targets.iter().collect::<HashSet<_>>().len() != targets.len() {
                        return None;
                    }
                    (targets, None)
                }
                "function" => {
                    let parts: Vec<_> = lhs.math().collect();
                    let [name, args] = parts.as_slice() else { return None };
                    if name.name != "id" || args.name != "boundVars" {
                        return None;
                    }
                    let params: Vec<_> = args.math().map(|node| id_node(node, names)).collect::<Option<_>>()?;
                    (vec![names.identifier(name)?], Some(params))
                }
                _ => return None,
            };
            let program_node =
                rhs.name == "program" && rhs.math().count() > 1 && rhs.math().any(|step| matches!(step.name.as_str(), "for" | "localDefine"));
            if program_node && let Some(params) = &locals {
                let (body, params) = program::function(rhs, names, params)?;
                let [target] = targets.as_slice() else { return None };
                let target = if live { names.definition(target) } else { target.clone() };
                return Some(format!("{target}({}) := {body}", params.join(", ")));
            }
            let params: &[String] = locals.as_deref().unwrap_or_default();
            let rhs = if flatten {
                expressions(&rhs.math().collect::<Vec<_>>(), names, params).map(|items| format!("transpose(worksheetAugment({}))", items.join(", ")))
            } else if rhs.name == "eval" {
                evaluation(rhs, names, params)
            } else {
                expr(rhs, names, params)
            };
            let identity =
                index.as_ref().is_some_and(|index| id_node(items[1], names).as_ref() == Some(index) && names.contiguous_ranges.contains(index));
            let bound = index.as_ref().map(|index| names.current.get(index).unwrap_or(index).to_owned());
            if bound.is_some() && !identity && names.origin != names.source_origin {
                return None;
            }
            if let [target] = targets.as_slice()
                && live
                && index.is_none()
                && locals.is_none()
            {
                if is_legacy_zero(items[1], names, &[]) {
                    names.zero_definitions.insert(target.clone());
                } else {
                    names.zero_definitions.remove(target);
                }
                let endpoints: Vec<_> = items[1].math().collect();
                if items[1].name == "range"
                    && endpoints.len() == 2
                    && endpoints.iter().all(|node| node.name == "real")
                    && endpoints[0].text.trim().parse::<u8>().ok() == Some(names.source_origin)
                {
                    names.contiguous_ranges.insert(target.clone());
                } else {
                    names.contiguous_ranges.remove(target);
                }
            }
            let targets = if live { targets.iter().map(|name| names.definition(name)).collect() } else { targets };
            let rhs = rhs?;
            let [target] = targets.as_slice() else { return Some(format!("[{}] := {rhs}", targets.join("; "))) };
            if let Some(bound) = bound {
                return Some(if identity { format!("{target} := {rhs}") } else { format!("{target}[{bound}] := {rhs}") });
            }
            match locals {
                Some(params) => Some(format!("{target}({}) := {rhs}", params.join(", "))),
                None => Some(format!("{target} := {rhs}")),
            }
        }
        "id" if node.text.trim() == "Given" => Some("Given".into()),
        _ => expr(node, names, &[]).map(|source| {
            if node.name == "apply" && node.math().next().is_some_and(|operator| operator.name == "equal") {
                source.get(1..source.len() - 1).unwrap_or(&source).to_owned()
            } else {
                format!("{source} =")
            }
        }),
    }
}

fn id_node(node: &Node, names: &Names) -> Option<String> {
    (node.name == "id").then(|| names.identifier(node)).flatten()
}

fn collect_regions<'a>(container: &'a Node, hidden: bool, right_margin: Option<f64>, out: &mut Vec<(&'a Node, &'a Node, bool)>) {
    let mut regions: Vec<_> = container.children.iter().filter(|child| child.ns == 1 && child.name == "region").collect();
    regions.sort_by(|left, right| {
        let y = |node: &Node| position(node, if node.attr("align-y").is_empty() { "top" } else { "align-y" });
        y(left).total_cmp(&y(right)).then_with(|| position(left, "left").total_cmp(&position(right, "left")))
    });
    for region in regions {
        let hidden = hidden || right_margin.is_some_and(|margin| position(region, "left") >= margin);
        if let Some(area) = region.child(1, "area") {
            collect_regions(area.child(1, "regions").unwrap_or(area), hidden || matches!(area.attr("is-collapsed"), "true" | "1"), right_margin, out);
        } else {
            out.push((container, region, hidden));
        }
    }
}

fn position(node: &Node, attr: &str) -> f64 {
    node.attr(attr).parse::<f64>().ok().filter(|value| value.is_finite()).unwrap_or(0.0)
}

fn note(region: &Node, feature: &str) -> Paragraph {
    let id = region.attr("region-id");
    worksheet_note(format!("[Mathcad region {id}: {feature} was not imported]"), WorksheetNoteKind::Unsupported)
}

fn unsupported_calculation(region: &Node, math: &Node) -> Paragraph {
    let mut pending = math.math().collect::<Vec<_>>();
    while let Some(node) = pending.pop() {
        let feature = match node.name.as_str() {
            "while" => Some("calculation with a while loop"),
            "tryCatch" => Some("calculation with an exception handler"),
            "complex" => Some("calculation with a complex number"),
            "id" if node.text.trim().is_empty() => Some("calculation with an empty identifier"),
            "str" if node.text.contains(['"', '\\', '\n', '\r']) => Some("calculation with unsupported string punctuation"),
            _ => None,
        };
        if let Some(feature) = feature {
            return note(region, feature);
        }
        if !matches!(node.name.as_str(), "result" | "unitOverride") {
            pending.extend(node.math());
        }
    }
    note(region, "unsupported calculation")
}

fn worksheet_note(message: String, kind: WorksheetNoteKind) -> Paragraph {
    let mut paragraph = Paragraph::with_text(&message, Default::default());
    paragraph.worksheet_notes.push(WorksheetNote { kind, off: 0, message });
    paragraph
}

fn component_snapshots<'a>(ordered: &[(&Node, &'a Node, bool)]) -> HashMap<usize, &'a Node> {
    let mut pending = HashMap::new();
    let mut snapshots = HashMap::new();
    for (index, (_, region, _)) in ordered.iter().enumerate() {
        if let Some(outputs) = region.child(1, "component").filter(|c| c.attr("disable-calc") != "true").and_then(|c| c.child(1, "outputs")) {
            let mut names = outputs.math().filter(|n| n.name == "id").filter_map(identifier);
            if let Some(first) = names.next() {
                if let Some(second) = names.next() {
                    pending.remove(&first);
                    pending.remove(&second);
                    for name in names {
                        pending.remove(&name);
                    }
                } else {
                    pending.insert(first, index);
                }
            }
            continue;
        }
        let Some(mut math) = region.child(1, "math").filter(|n| n.attr("disable-calc") != "true").and_then(|n| n.math().next()) else { continue };
        while math.name == "provenance" {
            let Some(inner) = math.math().next() else { break };
            math = inner;
        }
        if matches!(math.name.as_str(), "define" | "globalDefine") {
            if let Some(lhs) = math.math().next() {
                if let Some(name) = identifier(lhs).filter(|_| lhs.name == "id") {
                    pending.remove(&name);
                } else {
                    for child in lhs.math().filter(|n| n.name == "id").filter_map(identifier) {
                        pending.remove(&child);
                    }
                }
            }
            continue;
        }
        if math.name != "eval" {
            continue;
        }
        let Ok([id, result]): Result<[&Node; 2], _> = math.math().collect::<Vec<_>>().try_into() else { continue };
        if id.name != "id" || result.name != "result" {
            continue;
        }
        let Some(component) = identifier(id).and_then(|name| pending.get(&name)).copied() else { continue };
        let Some(matrix) = result.child(2, "matrix").filter(|node| node.math().all(|cell| matches!(cell.name.as_str(), "real" | "str"))) else {
            continue;
        };
        if let Some(source) = expr(matrix, &Names::default(), &[]) {
            snapshots
                .entry(component)
                .and_modify(|old: &mut Option<(String, &Node)>| {
                    if !old.as_ref().is_some_and(|(text, _)| *text == source) {
                        *old = None;
                    }
                })
                .or_insert(Some((source, matrix)));
        }
    }
    snapshots.into_iter().filter_map(|(index, value)| value.map(|(_, matrix)| (index, matrix))).collect()
}

fn snapshot_rows(matrix: &Node) -> Option<Vec<Vec<&Node>>> {
    let rows = matrix.attr("rows").parse::<usize>().ok()?;
    let cols = matrix.attr("cols").parse::<usize>().ok()?;
    let values: Vec<_> = matrix.math().collect();
    if rows == 0 || cols == 0 || rows.checked_mul(cols)? != values.len() || values.len() > 4096 {
        return None;
    }
    Some((0..rows).map(|row| (0..cols).map(|col| values[col * rows + row]).collect()).collect())
}

enum ComponentOrigin<'a> {
    Linked(&'a str),
    Missing(&'a str),
    Saved,
}

fn import_component(
    doc: &mut Document,
    region: &Node,
    component: &Node,
    data: (Option<&Node>, ComponentOrigin<'_>),
    names: &mut Names,
    id: u64,
) -> Result<(), String> {
    let outputs = component.child(1, "outputs");
    let output = outputs.and_then(|outputs| outputs.math().next()).filter(|node| node.name == "id");
    let (snapshot, origin) = data;
    if let Some((rows, output)) = snapshot.and_then(snapshot_rows).zip(output) {
        let name = output.text.trim();
        let mixed = rows.iter().flatten().any(|cell| cell.name == "empty" || cell.name != rows[0][0].name);
        let description = match origin {
            ComponentOrigin::Linked(path) => format!("imported from {path}"),
            ComponentOrigin::Missing(_) | ComponentOrigin::Saved => "imported from saved worksheet values".into(),
        };
        if mixed && rows[0].len() <= wordcraft_doc::table::MAX_COLS {
            let binding = names.identifier(output).and_then(|alias| {
                let source = worksheet_data(rows.len(), rows[0].len(), rows.iter().flatten().copied())?;
                let mut updated = names.clone();
                let name = updated.definition(&alias);
                let source = format!("{name} := {source}");
                wordcraft_calc::parse(&source).ok()?;
                *names = updated;
                Some(source)
            });
            let provenance = format!(
                "[Mathcad region {}: {name} {description} as a table{}]",
                region.attr("region-id"),
                if binding.is_some() { "" } else { "; calculation binding unavailable" }
            );
            let paragraph = match (&origin, &binding) {
                (ComponentOrigin::Linked(_), Some(_)) => Paragraph::with_text(&provenance, Default::default()),
                (ComponentOrigin::Linked(_), None) => worksheet_note(provenance, WorksheetNoteKind::Unsupported),
                _ => worksheet_note(provenance, WorksheetNoteKind::SavedSnapshot),
            };
            doc.body.push(Block::Para(paragraph).into());
            let mut table = Table::new(rows.len(), rows[0].len(), doc.last_section.text_width());
            for (row, values) in table.rows.iter_mut().zip(rows) {
                for (cell, value) in row.cells.iter_mut().zip(values) {
                    cell.blocks = vec![Block::Para(Paragraph::with_text(&value.text, Default::default())).into()];
                }
            }
            doc.body.push(Block::Table(table).into());
            if let Some(source) = binding {
                let calc = CalcRegion {
                    result_format: None,
                    worksheet_order: None,
                    id,
                    source,
                    unit: None,
                    precision: None,
                    numbered: false,
                    state: CalcState::Fresh,
                };
                let mut paragraph = Paragraph::new();
                paragraph
                    .insert_object(
                        0,
                        InlineObject::Equation { linear: String::new(), display: true, math: Default::default(), calc: Some(Box::new(calc)) },
                        &Default::default(),
                    )
                    .map_err(|error| error.to_string())?;
                doc.body.push(Block::Para(paragraph).into());
            }
            return Ok(());
        }
        if !mixed
            && let Some(name) = names.identifier(output)
            && let Some(values) =
                rows.iter().map(|row| expressions(row, &Names::default(), &[]).map(|cells| cells.join(", "))).collect::<Option<Vec<_>>>()
        {
            let name = names.definition(&name);
            let source = format!("{name} := [{}]", values.join("; "));
            if wordcraft_calc::parse(&source).is_ok() {
                let provenance = format!("[Mathcad region {}: {name} {description}]", region.attr("region-id"));
                let paragraph = if matches!(origin, ComponentOrigin::Linked(_)) {
                    Paragraph::with_text(&provenance, Default::default())
                } else {
                    worksheet_note(provenance, WorksheetNoteKind::SavedSnapshot)
                };
                doc.body.push(Block::Para(paragraph).into());
                let calc = CalcRegion {
                    result_format: None,
                    worksheet_order: None,
                    id,
                    source,
                    unit: None,
                    precision: None,
                    numbered: false,
                    state: CalcState::Fresh,
                };
                let math = wordcraft_doc::math::Math { jc: wordcraft_doc::math::MathJc::Left, ..Default::default() };
                let mut paragraph = Paragraph::new();
                paragraph
                    .insert_object(
                        0,
                        InlineObject::Equation { linear: String::new(), display: true, math, calc: Some(Box::new(calc)) },
                        &Default::default(),
                    )
                    .map_err(|error| error.to_string())?;
                doc.body.push(Block::Para(paragraph).into());
                return Ok(());
            }
        }
    }
    let names: Vec<_> = outputs.into_iter().flat_map(Node::math).filter(|n| n.name == "id").map(|n| n.text.trim()).collect();
    let feature = if names.is_empty() { "external component".into() } else { format!("external data for {}", names.join(", ")) };
    let feature = match origin {
        ComponentOrigin::Missing(path) | ComponentOrigin::Linked(path) => format!("{feature} (workbook: {path})"),
        ComponentOrigin::Saved => feature,
    };
    doc.body.push(Block::Para(note(region, &feature)).into());
    Ok(())
}

fn blank_comparison(math: &Node) -> Option<InlineObject> {
    let apply = math.child(2, "apply")?;
    let [operator, lhs, rhs] = apply.math().collect::<Vec<_>>().try_into().ok()?;
    if ![lhs, rhs].into_iter().all(|n| n.name == "id" && n.text.trim().is_empty() && n.attr("subscript").is_empty()) {
        return None;
    }
    let symbol = match operator.name.as_str() {
        "lessThan" => "<",
        "lessOrEqual" => "≤",
        "greaterThan" => ">",
        "greaterOrEqual" => "≥",
        _ => return None,
    };
    let linear = format!("□{symbol}□");
    let math = wordcraft_doc::math::Math { nodes: wordcraft_doc::math::parse_linear(&linear), ..Default::default() };
    Some(InlineObject::Equation { linear, display: false, math, calc: None })
}

fn relation_chain(math: &Node) -> bool {
    fn unwrap_provenance(mut node: &Node) -> Option<&Node> {
        while node.name == "provenance" {
            node = node.math().next()?;
        }
        Some(node)
    }

    fn equality(node: &Node) -> bool {
        node.name == "apply" && node.math().next().is_some_and(|operator| operator.name == "equal")
    }

    let Some(root) = math.math().next().and_then(unwrap_provenance) else { return false };
    equality(root) && root.math().skip(1).filter_map(unwrap_provenance).any(equality)
}

fn picture(doc: &mut Document, region: &Node, bytes: Vec<u8>, extension: &str, alt: String) -> Option<InlineObject> {
    let width = position(region, "width") as f32;
    let height = position(region, "height") as f32;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let scale = (doc.last_section.text_width() / width).min(1.0);
    Some(InlineObject::Image {
        media: doc.add_media(bytes, extension),
        w: width * scale,
        h: height * scale,
        alt,
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    })
}

fn metafile_picture(doc: &mut Document, region: &Node, binary: &HashMap<&str, &Node>) -> Option<InlineObject> {
    use std::io::Read as _;
    let id = region.child(1, "picture")?.child(1, "metafile")?.attr("item-idref");
    let item = binary.get(id)?;
    let encoded: Vec<_> = item.text.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let bytes = match item.attr("content-encoding") {
        "" => bytes,
        "gzip" => {
            const MAX_METAFILE_BYTES: u64 = 16 * 1024 * 1024;
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(bytes.as_slice()).take(MAX_METAFILE_BYTES + 1).read_to_end(&mut decoded).ok()?;
            if decoded.len() as u64 > MAX_METAFILE_BYTES {
                return None;
            }
            decoded
        }
        _ => return None,
    };
    let extension = match craft_metafile::sniff(&bytes)? {
        craft_metafile::Kind::Wmf => "wmf",
        craft_metafile::Kind::Emf => "emf",
    };
    craft_metafile::size(&bytes).ok()?;
    picture(doc, region, bytes, extension, "Worksheet picture".into())
}

fn saved_preview(doc: &mut Document, region: &Node, binary: &HashMap<&str, &Node>, alt: String) -> Option<InlineObject> {
    let id = region.child(1, "rendering")?.attr("item-idref");
    let item = binary.get(id)?;
    if !item.attr("content-encoding").is_empty() {
        return None;
    }
    let encoded: Vec<_> = item.text.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    picture(doc, region, bytes, "png", alt)
}

fn saved_region(doc: &mut Document, region: &Node, binary: &HashMap<&str, &Node>, description: &str) -> Result<Option<Paragraph>, String> {
    let Some(image) = saved_preview(doc, region, binary, description.into()) else { return Ok(None) };
    let message = format!("[Mathcad region {}: {description} preserved as a saved image]", region.attr("region-id"));
    let mut paragraph = Paragraph::new();
    paragraph.insert_object(0, image, &Default::default()).map_err(|error| error.to_string())?;
    paragraph.worksheet_notes.push(WorksheetNote { kind: WorksheetNoteKind::SavedSnapshot, off: 0, message });
    Ok(Some(paragraph))
}

fn static_equation(math: &Node, display: bool) -> Option<InlineObject> {
    let mut names = Names::default();
    let mut aliases = Vec::new();
    names.reserve(math, &mut aliases);
    names.normalize_aliases(aliases);
    let source = calculation(math.math().next()?, &mut names, false)?;
    let stmt = wordcraft_calc::parse(&source).ok()?;
    let (linear, mut math) = wordcraft_calc::render::unevaluated(&stmt, math.attr("disable-calc") == "true");
    math.jc = wordcraft_doc::math::MathJc::Left;
    Some(InlineObject::Equation { linear, display, math, calc: None })
}

fn has_cover(worksheet: &Node, regions: &Node, placed: &[layout::Placed<'_>], doc: &Document) -> bool {
    let Some(model) = page_model(worksheet) else { return false };
    let Some(margins) = model.child(1, "margins") else { return false };
    let page_height = position(model, "page-height") - position(margins, "top") - position(margins, "bottom");
    let [cover, next, ..] = placed else { return false };
    page_height > 0.0
        && std::ptr::eq(cover.container, regions)
        && std::ptr::eq(next.container, regions)
        && cover.region.child(1, "text").is_some()
        && cover.blocks.len() == 1
        && position(cover.region, "top") + position(cover.region, "height") + 6.0 < page_height
        && position(next.region, "top") >= page_height
        && doc.body[cover.blocks.start].as_para().is_some_and(|p| p.props.align == Some(Align::Center) && !p.text.trim().is_empty())
}

pub(super) fn import(bytes: &[u8], page: Option<&SectionProps>) -> Result<Document, String> {
    import_with_links(bytes, page, &|_| None)
}

pub(super) fn import_with_links(bytes: &[u8], page: Option<&SectionProps>, load: &LinkLoader<'_>) -> Result<Document, String> {
    let worksheet = read_xml(bytes, false)?;
    if worksheet.ns != 1 || worksheet.name != "worksheet" {
        return Err("Expected a Mathcad 14/15 worksheet".into());
    }
    import_worksheet(worksheet, page, load)
}

type LinkLoader<'a> = dyn Fn(&str) -> Option<Arc<Vec<u8>>> + 'a;

fn result_format(node: &Node) -> Option<ResultFormat> {
    node.children.iter().filter(|child| child.ns == 1).find_map(|child| {
        let style = match child.name.as_str() {
            "general" => NumberStyle::General,
            "decimal" => NumberStyle::Decimal,
            "scientific" => NumberStyle::Scientific,
            "engineering" => NumberStyle::Engineering,
            "fraction" => NumberStyle::Fraction,
            _ => return None,
        };
        let default = ResultFormat::default();
        let number = |key, fallback| child.attr(key).parse::<u8>().unwrap_or(fallback).min(15);
        let toggle = |key| matches!(child.attr(key), "true" | "1");
        Some(ResultFormat {
            style,
            decimals: number("precision", default.decimals),
            trailing_zeros: toggle("show-trailing-zeros"),
            engineering_exponents: toggle("engineering-format") || toggle("use-engineering-format") || toggle("engineering-exponents"),
            exponential_threshold: number("exponential-threshold", default.exponential_threshold),
            e_notation: toggle("use-e-notation"),
            accuracy: number("precision", default.accuracy),
            mixed_numbers: toggle("use-mixed-numbers"),
        })
    })
}

fn page_model(worksheet: &Node) -> Option<&Node> {
    worksheet.child(1, "settings")?.child(1, "presentation")?.child(1, "pageModel")
}

fn apply_page_model(model: &Node, page: &mut SectionProps) {
    let width = position(model, "page-width") as f32;
    let height = position(model, "page-height") as f32;
    if !(72.0..=2880.0).contains(&width) || !(72.0..=2880.0).contains(&height) {
        return;
    }
    page.page_w = width;
    page.page_h = height;
    if let Some(margins) = model.child(1, "margins") {
        let pick = |key| margins.attr(key).parse::<f32>().ok().filter(|value| (0.0..=720.0).contains(value));
        if let (Some(left), Some(right), Some(top), Some(bottom)) = (pick("left"), pick("right"), pick("top"), pick("bottom"))
            && left + right + 18.0 <= width
            && top + bottom + 18.0 <= height
        {
            (page.margin_left, page.margin_right, page.margin_top, page.margin_bottom) = (left, right, top, bottom);
        }
    }
}

fn apply_presentation(worksheet: &Node, doc: &mut Document) -> Option<&'static str> {
    let presentation = worksheet.child(1, "settings").and_then(|settings| settings.child(1, "presentation"));
    if let Some(model) = presentation.and_then(|node| node.child(1, "pageModel")) {
        apply_page_model(model, &mut doc.last_section);
    }
    doc.settings.math = Some(wordcraft_doc::math::MathProps { wrap_indent: 0.0, ..Default::default() });
    let style = presentation?
        .child(1, "mathRendering")?
        .child(1, "mathStyles")?
        .children
        .iter()
        .find(|style| matches!(style.attr("name"), "Variable" | "Variables"))?;
    let mut chr = CharProps::default();
    layout::character_attrs(&mut chr, |key| style.attr(key));
    if chr.size.is_none() && chr.font.is_none() {
        return None;
    }
    chr.italic = None;
    chr.bold = None;
    let id = "ImportedEquation";
    Arc::make_mut(&mut doc.styles).styles.push(wordcraft_doc::styles::Style {
        id: id.into(),
        name: "Imported Equation".into(),
        based_on: Some("Normal".into()),
        next: Some("Normal".into()),
        chr,
        ..Default::default()
    });
    Some(id)
}

fn import_worksheet(worksheet: Node, page: Option<&SectionProps>, load: &LinkLoader<'_>) -> Result<Document, String> {
    let regions = worksheet.child(1, "regions").ok_or("Mathcad worksheet has no regions")?;
    let mut doc = Document::default();
    doc.body.clear();
    if let Some(page) = page {
        doc.last_section = page.clone();
    }
    let equation_style = apply_presentation(&worksheet, &mut doc);
    let right_margin = page_model(&worksheet)
        .filter(|model| matches!(model.attr("print-single-page-width"), "true" | "1"))
        .map(|_| f64::from(doc.last_section.text_width()));
    let mut ordered = Vec::new();
    collect_regions(regions, false, right_margin, &mut ordered);
    let has_appendix = ordered.iter().any(|(_, region, hidden)| *hidden && region.child(1, "pageBreak").is_none());
    let snapshots = component_snapshots(&ordered);
    let mut names = Names::default();
    let mut needs_alias = Vec::new();
    names.reserve(regions, &mut needs_alias);
    names.normalize_aliases(needs_alias);
    for (_, region, _) in &ordered {
        if let Some(outputs) = region.child(1, "component").and_then(|component| component.child(1, "outputs")) {
            for output in outputs.math().filter(|node| node.name == "id") {
                names.reserve_component_output(output);
            }
        }
    }
    let mut settings = CalcSettings::default();
    if let Some(config) = worksheet.child(1, "settings").and_then(|node| node.child(1, "calculation")) {
        if let Some(variables) = config.child(1, "builtInVariables") {
            settings.origin = match variables.attr("array-origin") {
                "1" => 1,
                "" | "0" => 0,
                _ => return Err("WordCraft supports Mathcad array origins of 0 or 1".into()),
            };
        }
        if config.child(1, "units").is_some_and(|units| {
            units.child(1, "currentUnitSystem").is_some_and(|node| node.attr("name") == "si")
                || units.child(1, "customUnitSystem").is_some_and(|node| node.attr("base-unit-system") == "si")
        }) {
            settings.system = UnitSystem::Si;
        }
    }
    settings.result_format = worksheet
        .child(1, "settings")
        .and_then(|settings| settings.child(1, "presentation"))
        .and_then(|presentation| presentation.child(1, "mathRendering"))
        .and_then(|rendering| rendering.child(1, "resultFormat"))
        .and_then(result_format);
    names.origin = settings.origin;
    names.source_origin = settings.origin;
    doc.calc_settings = Some(settings);
    doc.worksheet_imported = true;
    let mut given = None;
    let styles = layout::TextStyles::new(&worksheet);
    let binary: HashMap<_, _> = worksheet
        .child(1, "binaryContent")
        .into_iter()
        .flat_map(|content| &content.children)
        .filter(|item| item.ns == 1 && item.name == "item")
        .map(|item| (item.attr("item-id"), item))
        .collect();
    let mut placed = Vec::new();
    let mut appendix = Vec::new();
    let mut worksheet_order = 0;
    let mut breaks = Vec::new();
    let mut pending_break = false;
    let mut next_inline_id = ordered.len() as u64 + 1;
    for (index, (container, region, hidden)) in ordered.into_iter().enumerate() {
        if region.child(1, "pageBreak").is_some() {
            if !hidden {
                pending_break = !placed.is_empty();
            }
            continue;
        }
        if let Some((block, start, solve_container)) = given
            && !std::ptr::eq(container, solve_container)
        {
            if let Some(slot) = doc.body.get_mut(block) {
                *slot = Block::Para(note(start, "incomplete solve block")).into();
            }
            given = None;
        }
        let start = doc.body.len();
        if let Some(math) = region.child(1, "math") {
            if math.attr("disable-calc") != "true"
                && let Some(definition) = math.child(2, "define").or_else(|| math.child(2, "globalDefine"))
                && let [name, value] = definition.math().collect::<Vec<_>>().as_slice()
                && name.name == "id"
                && name.text.trim() == "ORIGIN"
                && value.name == "real"
                && let Some(origin) = value.text.trim().parse::<u8>().ok().filter(|origin| *origin <= 1)
            {
                names.source_origin = origin;
            }
            let paragraph = if math.attr("disable-calc") == "true" {
                if let Some(object) = static_equation(math, true) {
                    let mut paragraph = Paragraph::new();
                    paragraph.props.style = equation_style.map(str::to_owned);
                    paragraph.insert_object(0, object, &Default::default()).map_err(|error| error.to_string())?;
                    paragraph
                } else {
                    note(region, "disabled calculation")
                }
            } else if let Some((source, stmt)) = math.math().next().and_then(|root| calculation(root, &mut names, true)).and_then(|source| {
                let stmt = wordcraft_calc::parse(&source).ok()?;
                Some((source, stmt))
            }) {
                let object = if given.is_none()
                    && let Stmt::Constraint { lhs, rhs } = stmt
                {
                    let (linear, mut math) = wordcraft_calc::render::standalone_equality(lhs, rhs);
                    math.jc = wordcraft_doc::math::MathJc::Left;
                    InlineObject::Equation { linear, display: true, math, calc: None }
                } else {
                    if matches!(stmt, Stmt::Given) {
                        given = Some((doc.body.len(), region, container));
                    } else if closes_solve(&stmt) {
                        given = None;
                    }
                    let result_format = math.child(1, "resultFormat").and_then(result_format);
                    let id = index as u64 + 1;
                    if matches!(&stmt, Stmt::Eval { .. })
                        && let Some(value) = saved_scalar(math)
                    {
                        doc.worksheet_saved_results.insert(id, value);
                    }
                    let calc = CalcRegion {
                        result_format,
                        worksheet_order: None,
                        id,
                        source,
                        unit: None,
                        precision: None,
                        numbered: false,
                        state: CalcState::Fresh,
                    };
                    let math = wordcraft_doc::math::Math { jc: wordcraft_doc::math::MathJc::Left, ..Default::default() };
                    InlineObject::Equation { linear: String::new(), display: true, math, calc: Some(Box::new(calc)) }
                };
                let mut paragraph = Paragraph::new();
                paragraph.props.style = equation_style.map(str::to_owned);
                paragraph.insert_object(0, object, &Default::default()).map_err(|error| error.to_string())?;
                paragraph
            } else {
                if let Some((block, start, _)) = given.take()
                    && let Some(slot) = doc.body.get_mut(block)
                {
                    *slot = Block::Para(note(start, "solve block with an unsupported expression")).into();
                }
                if relation_chain(math) {
                    saved_region(&mut doc, region, &binary, "symbolic derivation")?.unwrap_or_else(|| unsupported_calculation(region, math))
                } else {
                    unsupported_calculation(region, math)
                }
            };
            doc.body.push(Block::Para(paragraph).into());
        } else if let Some(component) = region.child(1, "component") {
            let linked = links::relative_link(component, &binary);
            let workbook = linked.as_deref().and_then(load).and_then(|bytes| links::matrix(&bytes));
            let data = workbook.as_ref().or_else(|| snapshots.get(&index).copied());
            let origin = match (&linked, &workbook) {
                (Some(path), Some(_)) => ComponentOrigin::Linked(path),
                (Some(path), None) => ComponentOrigin::Missing(path),
                _ => ComponentOrigin::Saved,
            };
            if component.child(1, "outputs").is_none_or(|outputs| outputs.math().next().is_none())
                && let Some(preview) = saved_region(&mut doc, region, &binary, "component output")?
            {
                doc.body.push(Block::Para(preview).into());
            } else {
                import_component(&mut doc, region, component, (data, origin), &mut names, index as u64 + 1)?;
            }
        } else if let Some(text) = region.child(1, "text") {
            for paragraph in text.children.iter().filter(|node| node.ns == 1 && node.name == "p") {
                let mut imported = Paragraph::new();
                append_text(paragraph, &mut imported, &styles.character_props(paragraph), &mut doc, &binary, &mut names, &mut next_inline_id)?;
                styles.apply(paragraph, &mut imported);
                doc.body.push(Block::Para(imported).into());
            }
        } else if (region.child(1, "ole").is_some() || region.child(1, "picture").is_some())
            && let Some(preview @ InlineObject::Image { w, .. }) =
                saved_preview(&mut doc, region, &binary, String::new()).or_else(|| metafile_picture(&mut doc, region, &binary))
        {
            let mut paragraph = Paragraph::new();
            let left = if hidden { 0.0 } else { position(region, "left") as f32 };
            paragraph.props.indent_left = Some(left.clamp(0.0, (doc.last_section.text_width() - w).max(0.0)));
            paragraph.insert_object(0, preview, &Default::default()).map_err(|error| error.to_string())?;
            doc.body.push(Block::Para(paragraph).into());
        } else {
            let feature = region.children.iter().find(|node| node.ns == 1).map_or("unsupported content", |node| node.name.as_str());
            doc.body.push(Block::Para(note(region, feature)).into());
        }
        if start < doc.body.len() {
            let blocks = start..doc.body.len();
            if has_appendix {
                for block in &mut doc.body[blocks.clone()] {
                    if let Block::Para(paragraph) = Arc::make_mut(block) {
                        for object in &mut paragraph.objects {
                            if let InlineObject::Equation { calc: Some(calc), .. } = object {
                                worksheet_order += 1;
                                calc.worksheet_order = Some(worksheet_order);
                            }
                        }
                    }
                }
            }
            if hidden {
                appendix.extend(blocks);
            } else {
                if std::mem::take(&mut pending_break) {
                    breaks.push(placed.len());
                }
                placed.push(layout::Placed { container, region, blocks });
            }
        }
    }
    let cover = has_cover(&worksheet, regions, &placed, &doc);
    if cover && breaks.first() != Some(&1) {
        breaks.insert(0, 1);
    }
    let mut body = Vec::new();
    let mut start = 0;
    for end in breaks.into_iter().chain(std::iter::once(placed.len())) {
        let mut chunk = layout::arrange(&doc.body, &placed[start..end], doc.last_section.text_width());
        if start != 0
            && let Some(next) = chunk.first_mut()
        {
            let opening = match Arc::make_mut(next) {
                Block::Table(table) => {
                    table.rows.first_mut().and_then(|row| row.cells.first_mut()).and_then(|cell| cell.blocks.first_mut()).map(Arc::make_mut)
                }
                block => Some(block),
            };
            if let Some(Block::Para(paragraph)) = opening {
                paragraph.props.page_break_before = Some(true);
            }
        }
        body.extend(chunk);
        start = end;
    }
    if !appendix.is_empty() {
        let mut heading = Paragraph::with_text("Worksheet Appendix", CharProps::default()).styled("Heading1");
        heading.props.page_break_before = Some(!body.is_empty());
        body.push(Block::Para(heading).into());
        body.extend(appendix.into_iter().map(|i| doc.body[i].clone()));
    }
    doc.body = body;
    Ok(doc)
}
