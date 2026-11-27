//! Lower a bounded subset of the legacy Mathcad program tree to calculation source.

use super::{Names, Node, expr, expressions, id_node};

pub(super) fn function(node: &Node, names: &Names, params: &[String]) -> Option<(String, Vec<String>)> {
    let mut scope = names.clone();
    scope.in_program = true;
    let mut pending = vec![node];
    while let Some(node) = pending.pop() {
        if matches!(node.name.as_str(), "localDefine" | "for") {
            let target = node.math().next()?;
            let target = if target.name == "apply" { target.math().nth(1)? } else { target };
            let key = id_node(target, names)?;
            let mut index = scope.program_locals.len();
            if !scope.program_locals.contains_key(&key) {
                loop {
                    let candidate = format!("WorksheetLocal_{index}");
                    if scope.used.insert(candidate.clone()) {
                        scope.program_locals.insert(key, candidate);
                        break;
                    }
                    index += 1;
                }
            }
        }
        pending.extend(node.math());
    }
    let params = params.iter().map(|param| scope.program_locals.get(param).unwrap_or(param).clone()).collect::<Vec<_>>();
    Some((source(node, &scope, &params)?, params))
}

pub(super) fn source(node: &Node, names: &Names, locals: &[String]) -> Option<String> {
    let result = if names.in_program {
        lower(node, names, locals, 0)?
    } else {
        return function(node, names, locals).map(|(source, _)| source);
    };
    (result.len() <= 16 * 1024).then_some(result)
}

fn lower(node: &Node, names: &Names, locals: &[String], depth: usize) -> Option<String> {
    if depth > 32 {
        return None;
    }
    let items = node.math().collect::<Vec<_>>();
    let lower = |node| lower(node, names, locals, depth + 1);
    let value = |node| expr(node, names, locals);
    let name = |node| {
        let name = id_node(node, names)?;
        Some(format!("\"{}\"", names.program_locals.get(&name).unwrap_or(&name)))
    };
    match (node.name.as_str(), items.as_slice()) {
        ("program", [first, otherwise]) if first.name == "ifThen" && otherwise.name == "otherwise" => {
            let [test, yes] = first.math().collect::<Vec<_>>().try_into().ok()?;
            let [no] = otherwise.math().collect::<Vec<_>>().try_into().ok()?;
            Some(format!("worksheetIf({}, {}, {})", value(test)?, lower(yes)?, lower(no)?))
        }
        ("program", [initial @ .., test, otherwise]) if test.name == "ifThen" && otherwise.name == "otherwise" => {
            let [condition, yes] = test.math().collect::<Vec<_>>().try_into().ok()?;
            let [no] = otherwise.math().collect::<Vec<_>>().try_into().ok()?;
            let mut steps = initial.iter().map(|item| lower(item)).collect::<Option<Vec<_>>>()?;
            steps.push(format!("worksheetIf({}, {}, {})", value(condition)?, lower(yes)?, lower(no)?));
            Some(format!("worksheetProgram({})", steps.join(", ")))
        }
        ("program", items) if !items.is_empty() => {
            let steps = items.iter().map(|item| lower(item)).collect::<Option<Vec<_>>>()?;
            Some(format!("worksheetProgram({})", steps.join(", ")))
        }
        ("localDefine", [target, rhs]) if target.name == "id" => Some(format!("worksheetSet({}, {})", name(target)?, value(rhs)?)),
        ("localDefine", [target, rhs]) if target.name == "apply" => {
            let [op, base, at] = target.math().collect::<Vec<_>>().try_into().ok()?;
            if op.name != "indexer" {
                return None;
            }
            let at = if at.name == "sequence" { at.math().collect() } else { vec![at] };
            let indexes = expressions(&at, names, locals)?.into_iter().map(|index| names.index(&index)).collect::<Vec<_>>();
            Some(format!("worksheetSetCell({}, {}, {})", name(base)?, value(rhs)?, indexes.join(", ")))
        }
        ("ifThen", [test, body]) => Some(format!("worksheetWhen({}, {})", value(test)?, lower(body)?)),
        ("for", [index, range, body]) if range.name == "range" => {
            let bounds = expressions(&range.math().collect::<Vec<_>>(), names, locals)?;
            Some(format!("worksheetFor({}, worksheetRange({}), {})", name(index)?, bounds.join(", "), lower(body)?))
        }
        ("return", [result]) => Some(format!("worksheetReturn({})", value(result)?)),
        ("break", []) => Some("worksheetBreak()".into()),
        ("otherwise", _) | ("program", []) => None,
        _ => value(node),
    }
}
