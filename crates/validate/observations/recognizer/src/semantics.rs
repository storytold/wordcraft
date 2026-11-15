//! Bounded name resolution and absence polarity for the closed observation grammar.
//! Unsupported tainted operations fail closed; this is deliberately not a Rust type checker.
use crate::{ident, tokens};
use serde_json::{Value, json};
use std::collections::HashMap;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{
    visit::{self, Visit},
    *,
};

#[derive(Clone, Default)]
pub(crate) struct Names {
    aliases: HashMap<String, String>,
}
impl Names {
    pub(crate) fn from_file(file: &File) -> Self {
        fn imports(tree: &UseTree, prefix: String, out: &mut HashMap<String, String>) {
            match tree {
                UseTree::Path(p) => imports(&p.tree, format!("{prefix}{}::", ident(&p.ident)), out),
                UseTree::Name(n) => {
                    out.insert(ident(&n.ident), format!("{prefix}{}", ident(&n.ident)));
                }
                UseTree::Rename(n) => {
                    out.insert(ident(&n.rename), format!("{prefix}{}", ident(&n.ident)));
                }
                UseTree::Group(g) => {
                    for t in &g.items {
                        imports(t, prefix.clone(), out);
                    }
                }
                UseTree::Glob(_) => {} // The syntax recognizer rejects these.
            }
        }
        let mut result = Self::default();
        for item in &file.items {
            match item {
                Item::Use(u) => imports(&u.tree, String::new(), &mut result.aliases),
                Item::Type(t) => {
                    result.aliases.insert(ident(&t.ident), tokens(&t.ty));
                }
                _ => {}
            }
        }
        result
    }
    pub(crate) fn resolve(&self, text: &str) -> String {
        fn expand(names: &Names, text: &str, active: &mut Vec<String>) -> String {
            let mut result = String::new();
            let mut word = String::new();
            let flush = |word: &mut String, result: &mut String, active: &mut Vec<String>| {
                if !result.trim_end().ends_with(':')
                    && !active.contains(word)
                    && active.len() < 32
                    && let Some(target) = names.aliases.get(word)
                {
                    active.push(word.clone());
                    result.push_str(&expand(names, target, active));
                    active.pop();
                } else {
                    result.push_str(word);
                }
                word.clear();
            };
            for ch in text.chars() {
                if ch.is_alphanumeric() || ch == '_' || (!ch.is_ascii() && !ch.is_whitespace()) {
                    word.push(ch);
                } else {
                    flush(&mut word, &mut result, active);
                    result.push(ch);
                }
            }
            flush(&mut word, &mut result, active);
            result
        }
        expand(self, text, &mut vec![])
    }
}

fn last(text: &str) -> &str {
    text.split("::").last().unwrap_or(text).trim()
}
fn lookup(name: &str) -> bool {
    matches!(name, "canonical_entry" | "entry_by_name" | "content_unavailable")
}
fn accessor(name: &str) -> bool {
    matches!(
        name,
        "prefix" | "is_complete" | "project" | "into_complete_set" | "missing" | "append" | "retain_budget" | "push_bounded" | "invalidate"
    )
}
fn raw_type(text: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|w| matches!(w, "Package" | "Entry" | "ContentTypes" | "HashMap" | "BTreeMap" | "HashSet" | "BTreeSet"))
}
#[derive(Clone, Default)]
struct Origin {
    callable: Option<String>,
    outcome: bool,
    raw: bool,
    limits: bool,
    prefix: bool,
    membership: bool,
}

fn resolution_type(text: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric() && c != '_').any(|word| word == "Resolution")
}
fn absence_return(output: &ReturnType, names: &Names, self_type: &str) -> bool {
    let ReturnType::Type(_, ty) = output else {
        return false;
    };
    let resolved = names.resolve(&tokens(ty).replace("Self", self_type));
    let resolved = resolved.trim().trim_start_matches('(').trim_end_matches(')').trim();
    let head = resolved.split('<').next().unwrap_or_default();
    matches!(head.rsplit("::").next().unwrap_or_default().trim(), "bool" | "Option" | "Resolution")
}

pub(crate) fn analyze(sig: &Signature, block: &Block, names: &Names, file: &str, self_type: &str) -> Value {
    let mut audit = Flow {
        names: names.clone(),
        bindings: HashMap::new(),
        scopes: vec![],
        issues: vec![],
        calls: vec![],
        file: file.into(),
        returns_absence: absence_return(&sig.output, names, self_type),
        function: ident(&sig.ident),
        self_type: self_type.into(),
        negative: false,
        accessors: vec![],
        scrub_paths: vec![],
        positive_membership: vec![],
        scalar_reads: vec![],
        callable_context: 0,
        state_depth: 0,
        state_predicates: vec![],
    };
    for input in &sig.inputs {
        match input {
            FnArg::Typed(p) => audit.bind(
                &p.pat,
                Origin {
                    raw: raw_type(&names.resolve(&tokens(&p.ty))),
                    outcome: resolution_type(&names.resolve(&tokens(&p.ty))),
                    limits: names.resolve(&tokens(&p.ty)).split_whitespace().last() == Some("Limits"),
                    ..Origin::default()
                },
            ),
            FnArg::Receiver(_) => {
                audit.bindings.insert(
                    "self".into(),
                    Origin { raw: raw_type(self_type), outcome: resolution_type(&names.resolve(self_type)), ..Origin::default() },
                );
            }
        }
    }
    audit.visit_block(block);
    json!({"issues":audit.issues,"lookups":audit.calls,"state_predicates":audit.state_predicates,"accessors":audit.accessors,"scrub_paths":audit.scrub_paths,"positive_membership":audit.positive_membership,"scalar_reads":audit.scalar_reads})
}
#[derive(Clone)]
struct Flow {
    names: Names,
    bindings: HashMap<String, Origin>,
    scopes: Vec<HashMap<String, Option<Origin>>>,
    issues: Vec<String>,
    calls: Vec<String>,
    file: String,
    function: String,
    returns_absence: bool,
    self_type: String,
    negative: bool,
    accessors: Vec<String>,
    scrub_paths: Vec<String>,
    positive_membership: Vec<String>,
    scalar_reads: Vec<String>,
    callable_context: usize,
    state_depth: usize,
    state_predicates: Vec<String>,
}
impl Flow {
    fn issue(&mut self, message: &str) {
        self.issues.push(message.into());
    }
    fn path(&self, p: &ExprPath) -> String {
        self.names.resolve(&tokens(p).replace("Self", &self.self_type))
    }
    fn merge_origins(&self, values: impl IntoIterator<Item = Origin>) -> Origin {
        let mut result = Origin::default();
        for v in values {
            result.prefix |= v.prefix;
            result.raw |= v.raw;
            result.outcome |= v.outcome;
            result.membership |= v.membership;
        }
        result
    }
    fn bind(&mut self, pat: &Pat, origin: Origin) {
        match pat {
            Pat::Ident(p) => {
                let name = ident(&p.ident);
                if let Some(scope) = self.scopes.last_mut() {
                    scope.entry(name.clone()).or_insert_with(|| self.bindings.get(&name).cloned());
                }
                self.bindings.insert(name, origin);
            }
            Pat::Type(p) => self.bind(&p.pat, origin),
            Pat::Paren(p) => self.bind(&p.pat, origin),
            Pat::Reference(p) => self.bind(&p.pat, origin),
            Pat::Tuple(p) => {
                for p in &p.elems {
                    self.bind(p, origin.clone());
                }
            }
            Pat::Slice(p) => {
                for p in &p.elems {
                    self.bind(p, origin.clone());
                }
            }
            Pat::TupleStruct(p) => {
                let mut payload = origin;
                if matches!(last(&self.names.resolve(&tokens(&p.path))), "One" | "Some" | "Unknown" | "Ambiguous") {
                    // Extracting a proven payload removes the Option/Resolution query tag.
                    payload.outcome = false;
                    payload.membership = false;
                }
                for p in &p.elems {
                    self.bind(p, payload.clone());
                }
            }
            _ => {} // Unsupported named patterns are rejected by syntax recognition.
        }
    }
    fn origin(&self, expr: &Expr) -> Origin {
        match expr {
            Expr::Path(p) => {
                let path = self.path(p);
                if let Some(value) = self.bindings.get(&tokens(p)) {
                    return value.clone();
                }
                let name = last(&path);
                Origin {
                    callable: (lookup(name) || (accessor(name) && (path.contains("::") || p.qself.is_some())) || name == "canonical")
                        .then(|| name.into()),
                    ..Origin::default()
                }
            }
            Expr::Call(c) => {
                let name = self.origin(&c.func).callable;
                match name.as_deref() {
                    Some("prefix") => Origin { prefix: true, ..Origin::default() },
                    Some(n) if lookup(n) => Origin { outcome: true, ..Origin::default() },
                    _ => {
                        let mut value = self.merge_origins(c.args.iter().map(|arg| self.origin(arg)));
                        if let Expr::Path(p) = c.func.as_ref()
                            && matches!(last(&self.path(p)), "contains" | "contains_key" | "get" | "any")
                        {
                            value.membership |= value.prefix || value.raw;
                        }
                        value
                    }
                }
            }
            Expr::Tuple(t) => self.merge_origins(t.elems.iter().map(|e| self.origin(e))),
            Expr::Array(t) => self.merge_origins(t.elems.iter().map(|e| self.origin(e))),
            Expr::Index(i) => self.origin(&i.expr),
            Expr::Block(b) => self.block_origin(&b.block),
            Expr::If(i) => {
                self.merge_origins([self.block_origin(&i.then_branch), i.else_branch.as_ref().map(|(_, e)| self.origin(e)).unwrap_or_default()])
            }
            Expr::Match(m) => self.merge_origins(m.arms.iter().map(|a| self.origin(&a.body))),
            Expr::Reference(r) => self.origin(&r.expr),
            Expr::Unary(u) if matches!(u.op, UnOp::Deref(_)) => self.origin(&u.expr),
            Expr::Cast(c) => self.origin(&c.expr),
            Expr::Try(t) => self.origin(&t.expr),
            Expr::Paren(p) => self.origin(&p.expr),
            Expr::Group(g) => self.origin(&g.expr),
            Expr::Field(f) => self.origin(&f.base),
            Expr::MethodCall(m) => {
                let origin = self.origin(&m.receiver);
                match ident(&m.method).as_str() {
                    "prefix" => Origin { prefix: true, ..Origin::default() },
                    "into_complete_set" | "is_complete" | "missing" => Origin::default(),
                    "contains" | "contains_key" | "any" | "all" | "get" | "find" | "is_none" | "is_some" | "is_some_and" => {
                        Origin { membership: origin.prefix || origin.raw, ..origin }
                    }
                    _ => origin,
                }
            }
            _ => Origin::default(),
        }
    }
    fn block_origin(&self, block: &Block) -> Origin {
        let mut nested = self.clone();
        for stmt in &block.stmts {
            if let Stmt::Local(local) = stmt
                && let Some(init) = &local.init
            {
                nested.bind(&local.pat, nested.origin(&init.expr));
            }
        }
        match block.stmts.last() {
            Some(Stmt::Expr(e, None)) => nested.origin(e),
            _ => Origin::default(),
        }
    }
    fn conditional_outcome(&self, expr: &Expr) -> bool {
        if self.origin(expr).outcome {
            return true;
        }
        match expr {
            Expr::Let(l) => self.origin(&l.expr).outcome,
            Expr::Binary(b) => self.conditional_outcome(&b.left) || self.conditional_outcome(&b.right),
            Expr::Unary(u) => self.conditional_outcome(&u.expr),
            Expr::Paren(p) => self.conditional_outcome(&p.expr),
            Expr::Macro(m) if tokens(&m.mac.path).ends_with("matches") => {
                fn subject(input: syn::parse::ParseStream<'_>) -> syn::Result<Expr> {
                    let expr = input.parse()?;
                    input.parse::<Token![,]>()?;
                    let _: proc_macro2::TokenStream = input.parse()?;
                    Ok(expr)
                }
                subject.parse2(m.mac.tokens.clone()).is_ok_and(|value| self.origin(&value).outcome)
            }
            _ => false,
        }
    }
    fn pattern(&mut self, pat: &Pat, guard: bool, presence_only: bool) {
        fn variants(p: &Pat, names: &Names, out: &mut Vec<String>) {
            match p {
                Pat::Path(p) => out.push(last(&names.resolve(&tokens(&p.path))).into()),
                Pat::TupleStruct(p) => out.push(last(&names.resolve(&tokens(&p.path))).into()),
                Pat::Or(p) => {
                    for p in &p.cases {
                        variants(p, names, out);
                    }
                }
                Pat::Paren(p) => variants(&p.pat, names, out),
                Pat::Reference(p) => variants(&p.pat, names, out),
                _ => out.push("catch-all".into()),
            }
        }
        let mut vs = vec![];
        variants(pat, &self.names, &mut vs);
        if presence_only {
            if vs != ["One"] {
                self.issue("absence polarity: only positive One extraction is supported in package if-let/let-else; absence decisions require an exhaustive match");
            }
            return;
        }
        if vs.iter().any(|v| v == "catch-all")
            || (guard && vs.iter().any(|v| v == "Absent" || v == "Unknown"))
            || (vs.iter().any(|v| v == "Absent") && vs.iter().any(|v| v == "Unknown"))
        {
            self.issue(
                "absence polarity: package lookup Unknown must have an explicit separate unguarded arm; catch-all/Absent collapse is forbidden",
            );
        }
    }
    fn negative_expr(&mut self, expr: &Expr, negative: bool) {
        let old = self.negative;
        self.negative = negative;
        self.visit_expr(expr);
        self.negative = old;
    }
}
impl<'ast> Visit<'ast> for Flow {
    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        let file = File { shebang: None, attrs: vec![], items: vec![Item::Use(item.clone())] };
        self.names.aliases.extend(Names::from_file(&file).aliases);
    }
    fn visit_item_type(&mut self, item: &'ast ItemType) {
        self.names.aliases.insert(ident(&item.ident), tokens(&item.ty));
    }

    fn visit_block(&mut self, block: &'ast Block) {
        let names = self.names.clone();
        self.scopes.push(HashMap::new());
        visit::visit_block(self, block);
        if let Some(declared) = self.scopes.pop() {
            for (name, previous) in declared {
                if let Some(origin) = previous {
                    self.bindings.insert(name, origin);
                } else {
                    self.bindings.remove(&name);
                }
            }
        }
        self.names = names;
    }
    fn visit_local(&mut self, local: &'ast Local) {
        if let Some(init) = &local.init {
            if matches!(&local.pat, Pat::Ident(p) if matches!(ident(&p.ident).as_str(), "entries_truncated" | "unknown_unreadable")) {
                self.state_predicates.push(format!("producer {} = {}", tokens(&local.pat), tokens(&init.expr)));
            }
            let callable = self.origin(&init.expr).callable.is_some();
            if callable {
                self.callable_context += 1;
            }
            self.visit_expr(&init.expr);
            if callable {
                self.callable_context -= 1;
            }
            self.bind(&local.pat, self.origin(&init.expr));
            if let Some((_, diverge)) = &init.diverge {
                if self.origin(&init.expr).outcome {
                    self.pattern(&local.pat, false, true);
                }
                self.visit_expr(diverge);
            }
        } else {
            self.bind(&local.pat, Origin::default());
        }
    }
    fn visit_expr(&mut self, expr: &'ast Expr) {
        let state_root = self.state_depth == 0
            && matches!(expr, Expr::Binary(_) | Expr::Unary(_) | Expr::Field(_) | Expr::Path(_) | Expr::Assign(_))
            && tokens(expr)
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|w| matches!(w, "entries_truncated" | "identity_incomplete" | "unknown_unreadable"));
        if state_root {
            self.state_predicates.push(tokens(expr));
            self.state_depth += 1;
        }
        let origin_here = self.origin(expr);
        if origin_here.callable.is_some() && self.callable_context == 0 {
            self.issue("tracked callable may only be invoked directly or bound to a local function value; escaping callable flow is unsupported");
        }
        if self.negative && matches!(expr, Expr::Path(_)) && origin_here.membership && origin_here.raw && self.returns_absence {
            self.issue("raw absence through a local boolean still requires completeness-carrying evidence");
        }
        if self.negative && origin_here.membership && origin_here.prefix {
            self.issue("negative retained-prefix membership requires CompleteSet, regardless of contract or local binding");
        }
        match expr {
            Expr::Field(f) if self.origin(&f.base).limits => {
                self.scalar_reads.push(tokens(f));
                visit::visit_expr(self, expr);
            }
            Expr::Path(p) if accessor(last(&self.path(p))) => {
                if p.qself.is_some() || p.path.segments.len() > 1 {
                    self.scrub_paths.push(tokens(p));
                }
            }
            Expr::Path(p) if last(&self.path(p)) == "Absent" && self.file != "resolution.rs" => {
                self.issue("unauthorized literal Resolution::Absent construction; use discovered with package loss evidence");
            }
            Expr::Call(c) => {
                if self.origin(&c.func).outcome || c.args.iter().any(|arg| self.origin(arg).outcome) {
                    self.issue(
                        "absence polarity: package Resolution must be matched or locally bound; unmodeled callable/argument flow is forbidden",
                    );
                }
                if self.negative && c.args.iter().any(|arg| self.origin(arg).prefix) {
                    self.issue("unmodeled negative retained-prefix call requires CompleteSet");
                }
                if let Some(name) = self.origin(&c.func).callable {
                    if lookup(&name) {
                        self.calls.push(name);
                    } else if accessor(&name) {
                        self.accessors.push(name);
                    } else if name == "canonical"
                        && !(self.file == "package.rs" && matches!(self.function.as_str(), "load" | "glossary_part" | "canonical_entry"))
                    {
                        self.issue("canonical is an internal presence-only helper; calls outside the contracted package lookup/assembly implementations are forbidden");
                    }
                }
                if let Expr::Path(p) = c.func.as_ref()
                    && last(&self.path(p)) == "discovered"
                {
                    if (self.file != "package.rs" || self.function != "load")
                        && matches!(c.args.iter().nth(1), Some(Expr::Lit(ExprLit {lit: Lit::Bool(b), ..})) if b.value)
                    {
                        self.issue("literal true is not package completeness evidence");
                    }
                    if self.file == "package.rs" && lookup(&self.function) {
                        let complete = c.args.iter().nth(1).map(tokens).unwrap_or_default();
                        if complete != "! pkg . entries_truncated && ! pkg . identity_incomplete" {
                            self.issue("lookup discovered completeness must consult both package entry-loss states");
                        }
                    }
                }
                self.callable_context += 1;
                self.visit_expr(&c.func);
                self.callable_context -= 1;
                for arg in &c.args {
                    let origin = self.origin(arg);
                    if origin.prefix && origin.membership {
                        self.issue("unmodeled retained-membership callable flow requires CompleteSet");
                    }
                    self.visit_expr(arg);
                }
            }
            Expr::MethodCall(m) => {
                let origin = self.origin(&m.receiver);
                let method = ident(&m.method);
                if origin.prefix && origin.membership && !matches!(method.as_str(), "is_none" | "is_some" | "is_some_and") {
                    self.issue("unmodeled retained-membership method transform requires CompleteSet");
                }
                if accessor(&method) {
                    self.accessors.push(method.clone());
                }
                if origin.prefix
                    && ((self.negative && matches!(method.as_str(), "contains" | "contains_key" | "any" | "is_some" | "is_some_and"))
                        || method == "is_none")
                {
                    self.issue("negative retained-prefix membership requires CompleteSet, regardless of contract or local binding");
                }
                if !self.negative && method != "is_none" && (origin.prefix || origin.raw) && matches!(method.as_str(), "contains" | "contains_key") {
                    self.positive_membership.push(tokens(&m.receiver).replace(' ', ""));
                }
                if origin.outcome {
                    self.issue("unsupported package Resolution operation; match One/Absent/Unknown/Ambiguous explicitly");
                }
                if self.returns_absence
                    && origin.raw
                    && ((self.negative && matches!(method.as_str(), "any" | "contains" | "contains_key"))
                        || matches!(method.as_str(), "is_none" | "all"))
                    && !(self.file == "package.rs" && self.function == "canonical")
                    && !(self.file == "lib.rs"
                        && self.function == "note_sources_complete"
                        && !self.negative
                        && method == "all"
                        && tokens(m).starts_with("pkg . entries . iter () . filter (| e | ! e . is_dir) . all ("))
                    && !matches!(
                        tokens(m).as_str(),
                        "pkg . rels . iter () . any (| rp | rp . incomplete)"
                            | "pkg . rels . iter () . any (| rp | rp . name . eq_ignore_ascii_case (\"_rels/.rels\") && rp . incomplete)"
                            | "entries . iter () . any (| e | e . name . eq_ignore_ascii_case (\"[Content_Types].xml\") && e . content_unavailable)"
                            | "entries . iter () . any (| e | e . name . eq_ignore_ascii_case (& want) && e . content_unavailable)"
                    )
                {
                    self.issue("raw absence requires completeness-carrying input/result; an observation contract cannot establish raw membership completeness");
                }
                visit::visit_expr(self, expr);
            }
            Expr::Match(m) => {
                if self.origin(&m.expr).outcome {
                    let patterns = m.arms.iter().map(|a| self.names.resolve(&tokens(&a.pat))).collect::<Vec<_>>().join(" ");
                    if ["One", "Absent", "Unknown", "Ambiguous"].iter().any(|v| !patterns.split(|c: char| !c.is_alphanumeric()).any(|w| w == *v)) {
                        self.issue("package Resolution match must explicitly cover One, Absent, Unknown and Ambiguous");
                    }
                    for arm in &m.arms {
                        self.pattern(&arm.pat, arm.guard.is_some(), false);
                    }
                }
                // A match on a retained membership bool is an absence-polarity spelling.
                // Require CompleteSet instead of guessing how arbitrary arm bodies use false.
                if self.origin(&m.expr).prefix && self.origin(&m.expr).membership {
                    self.issue("boolean membership match on retained prefix requires CompleteSet");
                }
                visit::visit_expr(self, expr);
            }
            Expr::If(i) => {
                let cond = self.origin(&i.cond);
                if i.else_branch.is_some() && cond.prefix && cond.membership {
                    self.issue("unmodeled retained-membership if/else polarity requires CompleteSet");
                }
                if i.else_branch.is_some() && self.conditional_outcome(&i.cond) {
                    self.issue("absence polarity: package lookup if-let/matches else collapse is unsupported; use explicit Resolution arms");
                }
                visit::visit_expr(self, expr);
            }
            Expr::Let(l) => {
                if self.origin(&l.expr).outcome {
                    self.pattern(&l.pat, false, true);
                }
                visit::visit_expr(self, expr);
            }
            Expr::Unary(u) if matches!(u.op, UnOp::Not(_)) => self.negative_expr(&u.expr, true),
            Expr::Binary(b) => {
                let membership = |e: &Expr| {
                    let value = self.origin(e);
                    value.membership && (value.prefix || (self.returns_absence && value.raw))
                };
                let left_member = membership(&b.left);
                let right_member = membership(&b.right);
                let boolean = |e: &Expr| match e {
                    Expr::Lit(ExprLit { lit: Lit::Bool(v), .. }) => Some(v.value),
                    _ => None,
                };
                let value = if left_member {
                    boolean(&b.right)
                } else if right_member {
                    boolean(&b.left)
                } else {
                    None
                };
                if let Some(value) = value {
                    let negative = (matches!(b.op, BinOp::Eq(_)) && !value) || (matches!(b.op, BinOp::Ne(_)) && value);
                    if matches!(b.op, BinOp::Eq(_) | BinOp::Ne(_)) {
                        self.negative_expr(&b.left, self.negative || negative);
                        self.negative_expr(&b.right, self.negative || negative);
                    } else {
                        self.issue("unmodeled membership boolean transform requires CompleteSet");
                    }
                } else if (left_member || right_member) && !matches!(b.op, BinOp::And(_) | BinOp::Or(_)) {
                    self.issue("unmodeled membership boolean comparison requires CompleteSet");
                } else {
                    visit::visit_expr(self, expr);
                }
            }
            Expr::Macro(m) => {
                let name = tokens(&m.mac.path);
                match last(&name) {
                    "matches" => {
                        struct MatchInput {
                            expr: Expr,
                            pat: Pat,
                            guard: Option<Expr>,
                        }
                        impl syn::parse::Parse for MatchInput {
                            fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
                                let expr = input.parse()?;
                                input.parse::<Token![,]>()?;
                                let pat = Pat::parse_multi_with_leading_vert(input)?;
                                let guard = if input.peek(Token![if]) {
                                    input.parse::<Token![if]>()?;
                                    Some(input.parse()?)
                                } else {
                                    None
                                };
                                if input.peek(Token![,]) {
                                    input.parse::<Token![,]>()?;
                                }
                                Ok(Self { expr, pat, guard })
                            }
                        }
                        if let Ok(m) = syn::parse2::<MatchInput>(m.mac.tokens.clone()) {
                            if self.origin(&m.expr).outcome {
                                self.pattern(&m.pat, m.guard.is_some(), false);
                            }
                            let negative = tokens(&m.pat) != "true" && self.origin(&m.expr).membership;
                            self.negative_expr(&m.expr, self.negative || negative);
                            if let Some(g) = m.guard {
                                self.visit_expr(&g);
                            }
                        }
                    }
                    "format" | "debug_assert" => {
                        if let Ok(args) = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(m.mac.tokens.clone()) {
                            for arg in args {
                                self.visit_expr(&arg);
                            }
                        }
                    }
                    "vec" => {
                        let ts = &m.mac.tokens;
                        if let Ok(repeat) = syn::parse2::<ExprRepeat>(quote::quote!([#ts])) {
                            self.visit_expr(&repeat.expr);
                            self.visit_expr(&repeat.len);
                        } else if let Ok(args) = Punctuated::<Expr, Token![,]>::parse_terminated.parse2(ts.clone()) {
                            for arg in args {
                                self.visit_expr(&arg);
                            }
                        }
                    }
                    _ => {} // Syntax recognition rejects all other expression macros.
                }
            }
            Expr::Assign(a) => {
                if let Expr::Path(p) = a.left.as_ref() {
                    let old = self.origin(&a.left);
                    if old.prefix || old.outcome || old.callable.is_some() {
                        self.issue("reassignment of tracked observation/callable provenance is unsupported");
                    }
                    let next = self.origin(&a.right);
                    self.bindings.insert(tokens(p), Origin { raw: old.raw || next.raw, ..next });
                }
                visit::visit_expr(self, expr);
            }
            _ => visit::visit_expr(self, expr),
        }
        if state_root {
            self.state_depth -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze_fn(code: &str) -> Vec<String> {
        let item: ItemFn = syn::parse_str(code).unwrap();
        let names = Names::default();
        let value = analyze(&item.sig, &item.block, &names, "package.rs", "");
        value["issues"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn nested_assignment_retains_outer_raw_provenance() {
        let issues = analyze_fn(
            "fn f(entries: &[Entry], name: &str) -> bool { let mut items: &[Entry] = &[]; { items = entries; } !items.iter().any(|e| e.name == name) }",
        );
        assert!(issues.iter().any(|i| i.contains("raw absence")), "expected raw absence issue: {issues:?}");
    }

    #[test]
    fn nested_shadowing_restores_outer_binding() {
        let issues = analyze_fn(
            "fn f(entries: &[Entry], name: &str) -> bool { let items = entries; { let items: &[Entry] = &[]; let _ = items; } !items.iter().any(|e| e.name == name) }",
        );
        assert!(issues.iter().any(|i| i.contains("raw absence")), "expected raw absence issue: {issues:?}");
    }

    #[test]
    fn nested_assignment_before_shadowing_retains_outer_raw_provenance() {
        let issues = analyze_fn(
            "fn f(entries: &[Entry], name: &str) -> bool { let mut items: &[Entry] = &[]; { items = entries; let items: &[Entry] = &[]; let _ = items; } !items.iter().any(|e| e.name == name) }",
        );
        assert!(issues.iter().any(|i| i.contains("raw absence")), "expected raw absence issue: {issues:?}");
    }

    #[test]
    fn raw_iterator_all_absence_is_rejected() {
        let issues = analyze_fn("fn f(entries: &[Entry], name: &str) -> bool { entries.iter().all(|e| e.name != name) }");
        assert!(issues.iter().any(|i| i.contains("raw absence")), "expected raw absence issue for Iterator::all: {issues:?}");
    }
}
