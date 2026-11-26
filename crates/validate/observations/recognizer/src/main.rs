//! Closed syntax recognition for the observation gate. Unknown syntax is an error.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod semantics;

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::{ToTokens, quote};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, BufRead, Write},
};
use syn::{
    parse::Parser,
    punctuated::Punctuated,
    visit::{self, Visit},
    *,
};

type Check<T> = std::result::Result<T, String>;

fn tokens(node: &impl ToTokens) -> String {
    fn normalize(stream: TokenStream) -> TokenStream {
        stream
            .into_iter()
            .map(|t| match t {
                TokenTree::Ident(i) => TokenTree::Ident(syn::ext::IdentExt::unraw(&i)),
                TokenTree::Group(g) => TokenTree::Group(Group::new(g.delimiter(), normalize(g.stream()))),
                t => t,
            })
            .collect()
    }
    normalize(node.to_token_stream()).to_string()
}
fn ident(node: &proc_macro2::Ident) -> String {
    node.to_string().trim_start_matches("r#").to_owned()
}
fn bad(kind: &str, node: &impl ToTokens) -> String {
    format!("unrecognized {kind}: {}", tokens(node).chars().take(180).collect::<String>())
}

#[derive(Clone)]
struct Template {
    // Preserve token spelling, including raw identifiers and literal forms, in cache seeds.
    text: String,
    matcher: Vec<TokenTree>,
    body: TokenStream,
}

// No repetition, alternative arms, nested matchers, or non-ident/literal fragments.
// Every accepted template is instantiated and parsed, including unused definitions.
fn template(mac: &Macro) -> Check<Template> {
    let ts: Vec<_> = mac.tokens.clone().into_iter().collect();
    match ts.as_slice() {
        [TokenTree::Group(matcher), TokenTree::Punct(eq), TokenTree::Punct(gt), TokenTree::Group(body), rest @ ..]
            if matcher.delimiter() == Delimiter::Parenthesis
                && eq.as_char() == '='
                && gt.as_char() == '>'
                && body.delimiter() == Delimiter::Brace
                && (rest.is_empty() || matches!(rest, [TokenTree::Punct(p)] if p.as_char() == ';')) =>
        {
            let result = Template { text: mac.tokens.to_string(), matcher: matcher.stream().into_iter().collect(), body: body.stream() };
            let _ = instantiate(&result, None)?;
            Ok(result)
        }
        _ => Err(bad("macro definition", mac)),
    }
}

fn instantiate(template: &Template, args: Option<TokenStream>) -> Check<TokenStream> {
    let mut input = args.map(|a| a.into_iter());
    let mut captures = HashMap::new();
    let mut matcher = template.matcher.iter();
    while let Some(token) = matcher.next() {
        if matches!(token, TokenTree::Punct(p) if p.as_char() == '$') {
            let (Some(TokenTree::Ident(name)), Some(TokenTree::Punct(colon)), Some(TokenTree::Ident(kind))) =
                (matcher.next(), matcher.next(), matcher.next())
            else {
                return Err("unsupported macro matcher".into());
            };
            if colon.as_char() != ':' {
                return Err("unsupported macro fragment".into());
            }
            let dummy: TokenStream = match ident(kind).as_str() {
                "ident" => "AuditPlaceholder".parse(),
                "literal" => "10".parse(),
                _ => return Err("unsupported macro fragment kind".into()),
            }
            .map_err(|e| format!("macro placeholder: {e}"))?;
            let value = if let Some(ref mut args) = input {
                args.next().ok_or("missing macro argument")?
            } else {
                dummy.into_iter().next().ok_or("missing placeholder")?
            };
            let valid = match ident(kind).as_str() {
                "ident" => matches!(value, TokenTree::Ident(_)),
                "literal" => matches!(value, TokenTree::Literal(_)),
                _ => false,
            };
            if !valid || captures.insert(ident(name), value).is_some() {
                return Err("unsupported macro argument".into());
            }
        } else {
            if !matches!(token, TokenTree::Ident(_) | TokenTree::Punct(_)) {
                return Err("unsupported macro matcher token".into());
            }
            if let Some(ref mut args) = input
                && args.next().as_ref().map(tokens) != Some(tokens(token))
            {
                return Err("macro argument mismatch".into());
            }
        }
    }
    if input.is_some_and(|mut a| a.next().is_some()) {
        return Err("extra macro arguments".into());
    }
    substitute(template.body.clone(), &captures)
}

fn substitute(stream: TokenStream, captures: &HashMap<String, TokenTree>) -> Check<TokenStream> {
    let mut out = TokenStream::new();
    let mut input = stream.into_iter();
    while let Some(token) = input.next() {
        match token {
            TokenTree::Punct(p) if p.as_char() == '$' => {
                let Some(TokenTree::Ident(name)) = input.next() else {
                    return Err("unsupported macro substitution".into());
                };
                out.extend([captures.get(&ident(&name)).ok_or("unknown macro capture")?.clone()]);
            }
            TokenTree::Group(g) => out.extend([TokenTree::Group(Group::new(g.delimiter(), substitute(g.stream(), captures)?))]),
            token => out.extend([token]),
        }
    }
    Ok(out)
}

fn derive_identity(path: &Path, names: &semantics::Names) -> Option<String> {
    let resolved = names.resolve(&tokens(path)).replace(' ', "");
    match resolved.as_str() {
        "serde::Serialize" | "serde::Deserialize" => Some(resolved),
        "Debug" | "std::fmt::Debug" | "core::fmt::Debug" => Some("Debug".into()),
        "Clone" | "Copy" | "Default" | "PartialEq" | "Eq" | "PartialOrd" | "Ord" | "Hash" => Some(resolved),
        _ => None,
    }
}
fn field_shape(fields: &Fields, names: &semantics::Names) -> Value {
    json!(
        fields
            .iter()
            .map(|f| json!({
                "name":f.ident.as_ref().map(ident), "type":names.resolve(&tokens(&f.ty)),
                "attributes":f.attrs.iter().filter(|a| !a.path().is_ident("doc")).map(tokens).collect::<Vec<_>>()
            }))
            .collect::<Vec<_>>()
    )
}

struct Audit {
    file: String,
    names: semantics::Names,
    self_type: String,
    error: Option<String>,
    units: Vec<Value>,
    surface: Vec<String>,
    enum_variants: Vec<String>,
    collection_fields: Vec<String>,
    readers: Vec<Value>,
    derive_context: bool,
    scope: Vec<String>,
    modules: Vec<String>,
    templates: HashMap<String, Template>,
    depth: usize,
    in_unit: usize,
    syntax_depth: usize,
}
impl Audit {
    fn reader_derives(&mut self, name: &proc_macro2::Ident, attrs: &[Attribute], generics: &Generics, shape: Value) {
        for attr in attrs.iter().filter(|a| a.path().is_ident("derive")) {
            if let Meta::List(list) = &attr.meta
                && let Ok(paths) = Punctuated::<Path, Token![,]>::parse_terminated.parse2(list.tokens.clone())
            {
                for path in paths {
                    if let Some(derive) = derive_identity(&path, &self.names)
                        && matches!(derive.as_str(), "Debug" | "serde::Serialize" | "serde::Deserialize")
                    {
                        self.readers
                            .push(json!({"type":format!("{}::{}{}",self.file,if self.scope.is_empty() { String::new() } else { self.scope.join("::")+"::" },ident(name)),"derive":derive,"generics":tokens(generics),"shape":shape}));
                    }
                }
            }
        }
    }
    fn fail(&mut self, message: String) {
        if self.error.is_none() {
            self.error = Some(message);
        }
    }
    fn unit(&mut self, name: String, header: String, body: String) {
        self.units.push(json!({"name":name,"header":header,"body":body}));
    }
    fn function(&mut self, sig: &Signature, block: &Block) {
        let semantic = semantics::analyze(sig, block, &self.names, &self.file, &self.self_type);
        self.units.push(json!({"name":ident(&sig.ident),"header":tokens(sig),"body":tokens(block),"semantic":semantic}));
        self.in_unit += 1;
        self.scope.push(format!("<function {}>", ident(&sig.ident)));
        self.visit_signature(sig);
        self.visit_block(block);
        self.scope.pop();
        self.in_unit -= 1;
    }
    fn initializer(&mut self, name: &proc_macro2::Ident, ty: &Type, expr: &Expr) {
        let sig: Signature = syn::parse_quote!(fn initializer());
        let block: Block = syn::parse_quote!({ #expr });
        let semantic = semantics::analyze(&sig, &block, &self.names, &self.file, &self.self_type);
        self.units.push(json!({"name":format!("<initializer {}>", ident(name)),"header":tokens(ty),"body":tokens(expr),"semantic":semantic}));
        self.in_unit += 1;
        self.visit_type(ty);
        self.visit_expr(expr);
        self.in_unit -= 1;
    }
    fn expanded_items(&mut self, stream: TokenStream) {
        if self.depth >= 32 {
            self.fail("macro expansion depth exceeded".into());
            return;
        }
        match syn::parse2::<File>(stream) {
            Ok(file) => {
                self.depth += 1;
                self.visit_file(&file);
                self.depth -= 1;
            }
            Err(e) => self.fail(format!("unsupported item macro expansion: {e}")),
        }
    }
    fn expression_list(&mut self, stream: TokenStream) {
        match Punctuated::<Expr, Token![,]>::parse_terminated.parse2(stream) {
            Ok(exprs) => {
                for expr in exprs {
                    self.visit_expr(&expr);
                }
            }
            Err(e) => self.fail(format!("unsupported macro expression arguments: {e}")),
        }
    }
}
impl<'ast> Visit<'ast> for Audit {
    fn visit_type_param_bound(&mut self, bound: &'ast TypeParamBound) {
        match bound {
            TypeParamBound::Trait(_) | TypeParamBound::Lifetime(_) => visit::visit_type_param_bound(self, bound),
            _ => self.fail(bad("type parameter bound", bound)),
        }
    }
    fn visit_lit(&mut self, lit: &'ast Lit) {
        match lit {
            Lit::Str(_) | Lit::ByteStr(_) | Lit::CStr(_) | Lit::Byte(_) | Lit::Char(_) | Lit::Int(_) | Lit::Float(_) | Lit::Bool(_) => {
                visit::visit_lit(self, lit)
            }
            _ => self.fail(bad("literal", lit)),
        }
    }
    fn visit_token_stream(&mut self, stream: &'ast TokenStream) {
        self.fail(bad("unparsed token stream", stream));
    }
    fn visit_use_tree(&mut self, tree: &'ast UseTree) {
        // Imports must not replace a standard macro whose argument grammar we audit.
        // Glob imports conceal binding names and therefore cannot be recognized here.
        let binding = match tree {
            UseTree::Name(n) => Some(ident(&n.ident)),
            UseTree::Rename(r) => Some(ident(&r.rename)),
            UseTree::Glob(_) => {
                self.fail(bad("glob import", tree));
                return;
            }
            UseTree::Path(_) | UseTree::Group(_) => None,
        };
        if binding.is_some_and(|n| matches!(n.as_str(), "std" | "format" | "vec" | "matches" | "debug_assert" | "observation_contract")) {
            self.fail(bad("macro-shadowing import", tree));
            return;
        }
        visit::visit_use_tree(self, tree);
    }
    fn visit_field(&mut self, field: &'ast Field) {
        let ty = tokens(&field.ty);
        if ty
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|n| matches!(n, "Observed" | "Vec" | "HashSet" | "HashMap" | "BTreeSet" | "BTreeMap"))
            && let Some(name) = &field.ident
        {
            self.collection_fields.push(ident(name));
        }
        let old = std::mem::replace(&mut self.derive_context, false);
        visit::visit_field(self, field);
        self.derive_context = old;
    }
    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        // Attribute macros/cfg_attr may replace bodies and are outside this grammar.
        let name = tokens(attr.path());
        if !matches!(name.as_str(), "doc" | "derive" | "default" | "allow" | "deny" | "forbid" | "cfg" | "inline" | "cold") {
            self.fail(bad("attribute", attr));
        }
        if name == "inline" {
            let valid = match &attr.meta {
                Meta::Path(_) => true,
                Meta::List(list) => matches!(list.tokens.to_string().as_str(), "always" | "never"),
                _ => false,
            };
            if !valid {
                self.fail(bad("inline attribute", attr));
            }
        }
        if name == "cold" && !matches!(attr.meta, Meta::Path(_)) {
            self.fail(bad("cold attribute", attr));
        }
        // Canonical derive identities are subsequently checked against exact type/field contracts.
        if name == "derive" {
            if !self.derive_context {
                self.fail("derive is permitted only on a recorded struct/enum, never a function, field or other attribute context".into());
            }
            let Meta::List(list) = &attr.meta else {
                self.fail(bad("derive", attr));
                return;
            };
            match Punctuated::<Path, Token![,]>::parse_terminated.parse2(list.tokens.clone()) {
                Ok(paths) if paths.iter().all(|p| derive_identity(p, &self.names).is_some()) => {}
                _ => self.fail(bad("derive", attr)),
            }
        }
    }
    fn visit_item(&mut self, item: &'ast Item) {
        if let Item::Macro(m) = item {
            for attr in &m.attrs {
                self.visit_attribute(attr);
            }
        }
        match item {
            Item::Fn(f) => {
                for a in &f.attrs {
                    self.visit_attribute(a);
                }
                self.function(&f.sig, &f.block);
            }
            Item::Const(c) => {
                for a in &c.attrs {
                    self.visit_attribute(a);
                }
                self.visit_generics(&c.generics);
                self.initializer(&c.ident, &c.ty, &c.expr);
            }
            Item::Static(s) => {
                for a in &s.attrs {
                    self.visit_attribute(a);
                }
                self.initializer(&s.ident, &s.ty, &s.expr);
            }
            Item::Impl(i) => {
                let self_type = tokens(&i.self_ty);
                if self_type != "PackageRegistryContext" || i.trait_.is_some() {
                    self.surface.push(self_type);
                }
                let old = std::mem::replace(&mut self.self_type, tokens(&i.self_ty));
                visit::visit_item_impl(self, i);
                self.self_type = old;
            }
            Item::Struct(s) => {
                self.reader_derives(&s.ident, &s.attrs, &s.generics, json!({"struct":field_shape(&s.fields,&self.names)}));
                self.surface.push(tokens(&s.fields));
                let old = std::mem::replace(&mut self.derive_context, true);
                visit::visit_item_struct(self, s);
                self.derive_context = old;
            }
            Item::Enum(e) => {
                let shape = json!({"enum":e.variants.iter().map(|v| json!({"variant":ident(&v.ident),"fields":field_shape(&v.fields,&self.names)})).collect::<Vec<_>>()});
                self.reader_derives(&e.ident, &e.attrs, &e.generics, shape);
                for v in &e.variants {
                    self.enum_variants.push(format!("{}::{}", ident(&e.ident), ident(&v.ident)));
                }
                self.surface.push(tokens(&e.variants));
                let old = std::mem::replace(&mut self.derive_context, true);
                visit::visit_item_enum(self, e);
                self.derive_context = old;
            }
            Item::Type(t) => {
                self.surface.push(tokens(t));
                visit::visit_item_type(self, t);
            }
            Item::Use(u) => {
                self.surface.push(tokens(u));
                visit::visit_item_use(self, u);
            }
            Item::Mod(m)
                if m.content.is_some()
                    && m.attrs.iter().any(|a| matches!(&a.meta, Meta::List(l) if l.path.is_ident("cfg") && l.tokens.to_string() == "test")) => {} // Inline unit tests are explicitly outside the production audit.
            Item::Mod(m) if m.content.is_none() => {
                self.modules.push(ident(&m.ident));
                visit::visit_item_mod(self, m);
            }
            Item::Macro(m) if tokens(&m.mac.path) == "macro_rules" => {
                let Some(name) = &m.ident else {
                    self.fail(bad("macro definition", m));
                    return;
                };
                if matches!(ident(name).as_str(), "format" | "vec" | "matches" | "debug_assert") {
                    self.fail("standard expression macro shadowing is outside the closed grammar".into());
                    return;
                }
                match template(&m.mac) {
                    Ok(t) => {
                        match instantiate(&t, None) {
                            Ok(stream) => {
                                self.scope.push(format!("<template {}>", ident(name)));
                                self.expanded_items(stream);
                                self.scope.pop();
                            }
                            Err(e) => self.fail(e),
                        }
                        if self.templates.insert(ident(name), t).is_some() {
                            self.fail("macro shadowing is unsupported".into());
                        }
                    }
                    Err(e) => self.fail(e),
                }
            }
            Item::Macro(m) => {
                let name = tokens(&m.mac.path);
                if let Some(t) = self.templates.get(&name).cloned() {
                    match instantiate(&t, Some(m.mac.tokens.clone())) {
                        Ok(stream) => self.expanded_items(stream),
                        Err(e) => self.fail(e),
                    }
                } else {
                    self.fail(bad("item macro", m));
                }
            }
            // Traits, unions, inline production modules, foreign items, and verbatim items
            // have no recognized production in this grammar. Never skip their bodies.
            _ => self.fail(bad("item", item)),
        }
    }
    fn visit_impl_item(&mut self, item: &'ast ImplItem) {
        match item {
            ImplItem::Fn(f) => {
                for a in &f.attrs {
                    self.visit_attribute(a);
                }
                self.function(&f.sig, &f.block);
            }
            ImplItem::Const(c) => {
                for a in &c.attrs {
                    self.visit_attribute(a);
                }
                self.visit_generics(&c.generics);
                self.initializer(&c.ident, &c.ty, &c.expr);
            }
            ImplItem::Type(t) => {
                self.surface.push(tokens(t));
                visit::visit_impl_item_type(self, t);
            }
            _ => self.fail(bad("impl item", item)),
        }
    }
    fn visit_variant(&mut self, variant: &'ast Variant) {
        let old = std::mem::replace(&mut self.derive_context, false);
        visit::visit_variant(self, variant);
        self.derive_context = old;
    }
    fn visit_pat(&mut self, pat: &'ast Pat) {
        match pat {
            Pat::Ident(_)
            | Pat::Lit(_)
            | Pat::Or(_)
            | Pat::Paren(_)
            | Pat::Path(_)
            | Pat::Range(_)
            | Pat::Reference(_)
            | Pat::Rest(_)
            | Pat::Slice(_)
            | Pat::Tuple(_)
            | Pat::TupleStruct(_)
            | Pat::Type(_)
            | Pat::Wild(_) => visit::visit_pat(self, pat),
            _ => self.fail(bad("pattern", pat)),
        }
    }
    fn visit_type(&mut self, ty: &'ast Type) {
        match ty {
            Type::Array(_)
            | Type::BareFn(_)
            | Type::Group(_)
            | Type::ImplTrait(_)
            | Type::Infer(_)
            | Type::Never(_)
            | Type::Paren(_)
            | Type::Path(_)
            | Type::Reference(_)
            | Type::Slice(_)
            | Type::TraitObject(_)
            | Type::Tuple(_) => visit::visit_type(self, ty),
            _ => self.fail(bad("type", ty)),
        }
    }
    fn visit_expr(&mut self, expr: &'ast Expr) {
        // Expressions in discriminants, type-level constants and other structural
        // positions are audited too, even when there is no enclosing named body.
        if self.error.is_some() {
            return;
        }
        if self.syntax_depth >= 256 {
            self.fail("syntax depth exceeded".into());
            return;
        }
        if self.in_unit == 0 {
            self.unit("<structural expression>".into(), String::new(), tokens(expr));
        }
        self.in_unit += 1;
        self.syntax_depth += 1;
        match expr {
            Expr::Assign(a) => {
                fn place(e: &Expr) -> bool {
                    match e {
                        Expr::Path(_) | Expr::Field(_) | Expr::Index(_) => true,
                        Expr::Unary(u) => matches!(u.op, UnOp::Deref(_)),
                        Expr::Paren(p) => place(&p.expr),
                        Expr::Group(g) => place(&g.expr),
                        _ => false,
                    }
                }
                if !place(&a.left) {
                    self.fail(bad("assignment target", &a.left));
                }
                visit::visit_expr(self, expr);
            }
            Expr::Array(_)
            | Expr::Binary(_)
            | Expr::Block(_)
            | Expr::Break(_)
            | Expr::Call(_)
            | Expr::Cast(_)
            | Expr::Closure(_)
            | Expr::Continue(_)
            | Expr::Field(_)
            | Expr::ForLoop(_)
            | Expr::Group(_)
            | Expr::If(_)
            | Expr::Index(_)
            | Expr::Let(_)
            | Expr::Lit(_)
            | Expr::Loop(_)
            | Expr::Macro(_)
            | Expr::Match(_)
            | Expr::MethodCall(_)
            | Expr::Paren(_)
            | Expr::Path(_)
            | Expr::Range(_)
            | Expr::Reference(_)
            | Expr::Repeat(_)
            | Expr::Return(_)
            | Expr::Struct(_)
            | Expr::Try(_)
            | Expr::Tuple(_)
            | Expr::Unary(_)
            | Expr::While(_) => visit::visit_expr(self, expr),
            _ => self.fail(bad("expression", expr)),
        }
        self.syntax_depth -= 1;
        self.in_unit -= 1;
    }
    fn visit_macro(&mut self, mac: &'ast Macro) {
        let path = tokens(&mac.path);
        let name = match path.as_str() {
            "std :: format" => "format",
            "std :: vec" => "vec",
            "std :: matches" => "matches",
            "std :: debug_assert" => "debug_assert",
            other => other,
        };
        match name {
            "observation_contract" => {
                // Validate the actual no-code template, not just the invocation name.
                match self
                    .templates
                    .get(name)
                    .ok_or("undeclared observation contract macro".into())
                    .and_then(|t| instantiate(t, Some(mac.tokens.clone())))
                {
                    Ok(stream) if stream.is_empty() => {}
                    Ok(_) => self.fail("observation contract macro emits code".into()),
                    Err(e) => self.fail(e),
                }
            }
            "format" | "debug_assert" => self.expression_list(mac.tokens.clone()),
            "vec" => {
                // Vec's two public forms contain expressions only.
                let ts = mac.tokens.clone();
                if let Ok(repeat) = syn::parse2::<ExprRepeat>(quote!([#ts])) {
                    self.visit_expr(&repeat.expr);
                    self.visit_expr(&repeat.len);
                } else {
                    self.expression_list(ts);
                }
            }
            "matches" => {
                struct Matches {
                    expr: Expr,
                    pat: Pat,
                    guard: Option<Expr>,
                }
                impl syn::parse::Parse for Matches {
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
                match syn::parse2::<Matches>(mac.tokens.clone()) {
                    Ok(m) => {
                        self.visit_expr(&m.expr);
                        self.visit_pat(&m.pat);
                        if let Some(g) = m.guard {
                            self.visit_expr(&g);
                        }
                    }
                    Err(e) => self.fail(format!("unsupported matches arguments: {e}")),
                }
            }
            _ => self.fail(bad("expression/pattern macro", mac)),
        }
    }
}

fn audit(name: &str, source: &str, templates: HashMap<String, Template>) -> Check<(Value, HashMap<String, Template>)> {
    let file = syn::parse_file(source).map_err(|e| format!("Rust parse failed: {e}"))?;
    let mut audit = Audit {
        file: name.into(),
        names: semantics::Names::from_file(&file),
        self_type: String::new(),
        error: None,
        units: vec![],
        surface: vec![],
        enum_variants: vec![],
        collection_fields: vec![],
        readers: vec![],
        derive_context: false,
        scope: vec![],
        modules: vec![],
        templates,
        depth: 0,
        in_unit: 0,
        syntax_depth: 0,
    };
    audit.visit_file(&file);
    if let Some(e) = audit.error {
        return Err(e);
    }
    Ok((
        json!({"units":audit.units,"surface":audit.surface,"enum_variants":audit.enum_variants,"collection_fields":audit.collection_fields,"modules":audit.modules,"readers":audit.readers}),
        audit.templates,
    ))
}
// SHA-256 (FIPS 180-4). The streaming blocks keep padding allocation constant.
// Cache identities need collision resistance; std's process-local hasher is insufficient.
type SourceHash = [u8; 32];

fn sha256(bytes: &[u8]) -> SourceHash {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
        0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
        0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
        0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    fn compress(state: &mut [u32; 8], block: &[u8]) {
        let mut words = [0u32; 64];
        for (word, bytes) in words.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*bytes);
        }
        for i in 16..64 {
            let x = words[i - 15];
            let y = words[i - 2];
            let s0 = x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3);
            let s1 = y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10);
            words[i] = words[i - 16].wrapping_add(s0).wrapping_add(words[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for (word, constant) in words.into_iter().zip(K) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(choose).wrapping_add(constant).wrapping_add(word);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(add);
        }
    }
    let mut state = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let (blocks, remainder) = bytes.as_chunks::<64>();
    for block in blocks {
        compress(&mut state, block);
    }
    let mut tail = [0u8; 128];
    for (byte, input) in tail.iter_mut().zip(remainder) {
        *byte = *input;
    }
    if let Some(byte) = tail.get_mut(remainder.len()) {
        *byte = 0x80;
    }
    let bit_len = (bytes.len() as u64).wrapping_mul(8).to_be_bytes();
    if remainder.len() < 56 {
        tail[56..64].copy_from_slice(&bit_len);
        compress(&mut state, &tail[..64]);
    } else {
        tail[120..128].copy_from_slice(&bit_len);
        compress(&mut state, &tail[..64]);
        compress(&mut state, &tail[64..]);
    }
    let mut result = [0u8; 32];
    for (bytes, word) in result.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    result
}

fn template_fingerprint(templates: &HashMap<String, Template>) -> SourceHash {
    let mut ordered: Vec<_> = templates.iter().collect();
    ordered.sort_by_key(|(name, _)| *name);
    let mut bytes = Vec::new();
    for (name, template) in ordered {
        for text in [name.as_str(), template.text.as_str()] {
            bytes.extend_from_slice(&(text.len() as u64).to_be_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
    }
    sha256(&bytes)
}

#[derive(Default)]
struct Cache {
    files: HashMap<(String, SourceHash), Value>,
    lib_templates: HashMap<SourceHash, HashMap<String, Template>>,
    templates_fp: Option<SourceHash>,
}

impl Cache {
    fn request(&mut self, value: Value) -> Check<Value> {
        let sources = value.as_object().ok_or("expected source map")?;
        // A long-lived debugging client must not retain an unbounded mutation history.
        if self.files.len() >= 256 {
            *self = Self::default();
        }
        let mut result = serde_json::Map::new();
        let mut templates = HashMap::new();
        if let Some(source) = sources.get("lib.rs") {
            let source = source.as_str().ok_or("expected source text")?;
            let hash = sha256(source.as_bytes());
            let key = ("lib.rs".to_owned(), hash);
            if let (Some(data), Some(seed)) = (self.files.get(&key), self.lib_templates.get(&hash)) {
                result.insert("lib.rs".into(), data.clone());
                templates = seed.clone();
            } else {
                // Never seed lib.rs from a previous request's macro definitions.
                let (data, seed) = audit("lib.rs", source, HashMap::new()).map_err(|e| format!("lib.rs: {e}"))?;
                self.files.insert(key, data.clone());
                self.lib_templates.insert(hash, seed.clone());
                result.insert("lib.rs".into(), data);
                templates = seed;
            }
        }
        let fingerprint = template_fingerprint(&templates);
        if self.templates_fp != Some(fingerprint) {
            // Unchanged child text can expand differently under a changed lib template.
            self.files.retain(|(name, _), _| name == "lib.rs");
            self.templates_fp = Some(fingerprint);
        }
        for (name, source) in sources.iter().filter(|(name, _)| name.as_str() != "lib.rs") {
            let source = source.as_str().ok_or("expected source text")?;
            let key = (name.clone(), sha256(source.as_bytes()));
            let data = if let Some(data) = self.files.get(&key) {
                data.clone()
            } else {
                let (data, _) = audit(name, source, templates.clone()).map_err(|e| format!("{name}: {e}"))?;
                if self.files.len() < 256 {
                    self.files.insert(key, data.clone());
                }
                data
            };
            result.insert(name.clone(), data);
        }
        Ok(Value::Object(result))
    }
}

fn run() -> Check<()> {
    let mut cache = Cache::default();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let response = match line
            .map_err(|e| e.to_string())
            .and_then(|line| serde_json::from_str(&line).map_err(|e| e.to_string()))
            .and_then(|value| cache.request(value))
        {
            Ok(value) => json!({"ok":value}),
            Err(e) => json!({"error":e}),
        };
        writeln!(stdout, "{response}").map_err(|e| e.to_string())?;
        stdout.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn hex(bytes: SourceHash) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha256_known_answers() {
        for (input, expected) in [
            ("", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            ("abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            ("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq", "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"),
        ] {
            assert_eq!(hex(sha256(input.as_bytes())), expected);
        }
        assert_eq!(hex(sha256(&vec![b'a'; 1_000_000])), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    fn source(value: &str) -> String {
        format!("macro_rules! make {{ ($name:ident) => {{ fn $name() -> u8 {{ {value} }} }}; }}")
    }

    #[test]
    fn template_change_revert_and_reapply_match_fresh_audits() {
        let mut cache = Cache::default();
        let child = "make!(chosen);";
        let original = cache.request(json!({"lib.rs": source("1"), "child.rs": child})).unwrap();
        let original_fp = cache.templates_fp;
        let child_key = ("child.rs".to_owned(), sha256(child.as_bytes()));
        for text in ["0x01", "1", "0x01", "1"] {
            let actual = cache.request(json!({"lib.rs": source(text), "child.rs": child})).unwrap();
            let fresh = Cache::default().request(json!({"lib.rs": source(text), "child.rs": child})).unwrap();
            assert_eq!(actual, fresh);
            assert!(cache.files.contains_key(&child_key));
            if text == "1" {
                assert_eq!(actual, original);
                assert_eq!(cache.templates_fp, original_fp);
            } else {
                assert_ne!(cache.templates_fp, original_fp);
            }
        }
        // A child without lib.rs cannot inherit the preceding request's templates.
        assert!(cache.request(json!({"child.rs": child})).is_err());
    }

    #[test]
    fn child_content_and_failed_requests_do_not_poison_cached_audits() {
        let mut cache = Cache::default();
        let lib = source("1");
        let original = json!({"lib.rs": lib, "child.rs": "make!(chosen);"});
        let expected = cache.request(original.clone()).unwrap();
        let child = "make!(other);";
        let changed = json!({"lib.rs": lib, "child.rs": child});
        assert_eq!(cache.request(changed.clone()).unwrap(), Cache::default().request(changed).unwrap());
        assert!(cache.files.contains_key(&("lib.rs".to_owned(), sha256(lib.as_bytes()))));
        assert!(cache.files.contains_key(&("child.rs".to_owned(), sha256(child.as_bytes()))));
        assert!(cache.request(json!({"lib.rs": "unclosed {", "child.rs": child})).is_err());
        assert!(cache.request(json!({"lib.rs": lib, "child.rs": "hidden!();"})).is_err());
        assert_eq!(cache.request(original).unwrap(), expected);
    }

    #[test]
    fn template_fingerprint_is_order_independent_and_covers_matchers() {
        let parse = |s: &str| {
            let file = syn::parse_file(s).unwrap();
            let Item::Macro(m) = file.items.first().unwrap() else { panic!("expected macro") };
            template(&m.mac).unwrap()
        };
        let a = parse("macro_rules! a { ($name:ident) => { fn $name() {} }; }");
        let b = parse("macro_rules! b { ($name:literal) => { const N: u8 = $name; }; }");
        let mut left = HashMap::from([("a".into(), a.clone()), ("b".into(), b.clone())]);
        let right = HashMap::from([("b".into(), b), ("a".into(), a)]);
        assert_eq!(template_fingerprint(&left), template_fingerprint(&right));
        left.insert("a".into(), parse("macro_rules! a { ($name:literal) => { const N: u8 = $name; }; }"));
        assert_ne!(template_fingerprint(&left), template_fingerprint(&right));
    }
}
