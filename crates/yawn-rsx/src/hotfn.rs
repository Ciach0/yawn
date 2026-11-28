//! Lets a dev build replace the body of a plain Rust function without rebuilding the app.
//!
//! A component renders through `yawn::dynamic`, so its body can come from a module loaded later.
//! Plain functions are called directly, so [`transform`] gives the eligible ones the same
//! indirection: the function keeps its signature and asks the registry for a newer body first, and
//! its old body moves into `__yawn_body_<name>`. Line numbers of the file stay the same.

use proc_macro2::TokenTree;
use quote::ToTokens;
use std::path::Path;

/// The last line of a transformed file: a hash of the file without the bodies it replaced, so a
/// dev build can tell an edit of a body from any other edit
pub const SKELETON: &str = "// yawn:skeleton ";
const ITEMS: &str = "// yawn:items ";

/// The line before it: the signature of each public function as `name#hash`, so a dev build can tell
/// which of them an edit changed
pub const SIGS: &str = "// yawn:sigs ";

/// Whether a public free function takes part in hot swapping like a private one: its symbol and
/// registry key carry a hash of the signature, so a changed signature is a new function and the
/// files that call it can be rebuilt as modules too
pub static HOT_PUB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The hashes of the top-level items of a transformed file (what is left of each once the bodies
/// that are swapped are taken out). A new item adds a hash and leaves the others as they were
pub fn items(text: &str) -> Vec<String> {
    for line in text.lines().rev().take(5) {
        if let Some(rest) = line.strip_prefix(ITEMS) {
            return rest.split_whitespace().map(str::to_string).collect();
        }
    }
    Vec::new()
}

/// The names and hashes of the public functions in a transformed file
pub fn sigs(text: &str) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for line in text.lines().rev().take(4) {
        if let Some(rest) = line.strip_prefix(SIGS) {
            for item in rest.split_whitespace() {
                if let Some((name, hash)) = item.split_once('#') {
                    out.insert(name.to_string(), hash.to_string());
                }
            }
            break;
        }
    }
    out
}

struct Edit {
    at: usize,
    end: usize,
    text: String,
}

struct Registered {
    key: String,
    target: String,
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn offset(text: &str, starts: &[usize], line: usize, column: usize) -> Option<usize> {
    let start = *starts.get(line.checked_sub(1)?)?;
    let rest = text.get(start..)?;
    let mut chars = rest.char_indices();
    let mut n = 0;
    while n < column {
        chars.next()?;
        n += 1;
    }
    Some(start + chars.next().map(|(i, _)| i).unwrap_or(rest.len()))
}

fn span_start(text: &str, starts: &[usize], span: proc_macro2::Span) -> Option<usize> {
    let s = span.start();
    offset(text, starts, s.line, s.column)
}

fn span_end(text: &str, starts: &[usize], span: proc_macro2::Span) -> Option<usize> {
    let s = span.end();
    offset(text, starts, s.line, s.column)
}

fn mentions(ts: proc_macro2::TokenStream, word: &str) -> bool {
    ts.into_iter().any(|t| match t {
        TokenTree::Ident(i) => i == word,
        TokenTree::Group(g) => mentions(g.stream(), word),
        _ => false,
    })
}

fn plain_attrs(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().all(|a| {
        a.path().get_ident().is_some_and(|i| {
            matches!(i.to_string().as_str(), "doc" | "inline" | "allow" | "must_use" | "deprecated")
        })
    })
}

struct Shape {
    lifetime_params: Vec<String>,
    lifetime_types: Vec<String>,
    borrowed: bool,
    params: Vec<String>,
    types: Vec<String>,
    names: Vec<String>,
    receiver: Option<String>,
    ret: String,
}

fn borrows(ts: proc_macro2::TokenStream) -> bool {
    ts.into_iter().any(|t| match t {
        proc_macro2::TokenTree::Punct(p) => p.as_char() == '&' || p.as_char() == '\'',
        proc_macro2::TokenTree::Group(g) => borrows(g.stream()),
        _ => false,
    })
}

/// The type with every elided reference given one lifetime, `None` when it names a lifetime
fn one_lifetime(ts: proc_macro2::TokenStream) -> Option<String> {
    let mut out = String::new();
    for t in ts {
        match t {
            proc_macro2::TokenTree::Punct(p) if p.as_char() == '\'' => return None,
            proc_macro2::TokenTree::Punct(p) if p.as_char() == '&' => out.push_str("&'__y "),
            proc_macro2::TokenTree::Punct(p) => {
                out.push(p.as_char());
                if p.spacing() == proc_macro2::Spacing::Alone {
                    out.push(' ');
                }
            }
            proc_macro2::TokenTree::Group(g) => {
                let (open, close) = match g.delimiter() {
                    proc_macro2::Delimiter::Parenthesis => ("(", ")"),
                    proc_macro2::Delimiter::Brace => ("{", "}"),
                    proc_macro2::Delimiter::Bracket => ("[", "]"),
                    proc_macro2::Delimiter::None => ("", ""),
                };
                out.push_str(open);
                out.push_str(&one_lifetime(g.stream())?);
                out.push_str(close);
                out.push(' ');
            }
            other => {
                out.push_str(&other.to_string());
                out.push(' ');
            }
        }
    }
    Some(out)
}

fn shape(sig: &syn::Signature, self_ty: Option<&str>, allow_async: bool) -> Option<Shape> {
    if sig.constness.is_some()
        || (sig.asyncness.is_some() && (!allow_async || self_ty.is_some()))
        || !matches!(sig.safety, syn::Safety::Default)
        || sig.abi.is_some()
        || sig.variadic.is_some()
        || !sig.generics.params.is_empty()
        || sig.generics.where_clause.is_some()
    {
        return None;
    }
    let mut out = Shape {
        lifetime_params: Vec::new(),
        lifetime_types: Vec::new(),
        borrowed: false,
        params: Vec::new(),
        types: Vec::new(),
        names: Vec::new(),
        receiver: None,
        ret: "()".into(),
    };
    for (i, arg) in sig.inputs.iter().enumerate() {
        match arg {
            syn::FnArg::Receiver(r) => {
                let owner = self_ty?;
                let (decl, ty) = match (&r.kind, &r.mutability) {
                    (syn::ReceiverKind::Reference(_, None, Some(_)), _) => {
                        ("&mut self".to_string(), format!("&mut {owner}"))
                    }
                    (syn::ReceiverKind::Reference(_, None, None), _) => {
                        ("&self".to_string(), format!("&{owner}"))
                    }
                    (syn::ReceiverKind::Value, Some(_)) => {
                        ("mut self".to_string(), owner.to_string())
                    }
                    (syn::ReceiverKind::Value, None) => ("self".to_string(), owner.to_string()),
                    _ => return None,
                };
                out.receiver = Some(decl.clone());
                out.lifetime_params.push(decl.clone());
                out.lifetime_types.push(ty.clone());
                out.params.push(decl);
                out.types.push(ty);
                out.names.push("self".into());
            }
            syn::FnArg::Typed(t) => {
                let ty = t.ty.to_token_stream();
                if mentions(ty.clone(), "impl") || mentions(ty.clone(), "_") {
                    return None;
                }
                let name = format!("__a{i}");
                if sig.asyncness.is_some() {
                    out.borrowed |= borrows(ty.clone());
                    let lt = one_lifetime(ty.clone())?;
                    out.lifetime_params.push(format!("{name}: {lt}"));
                    out.lifetime_types.push(lt);
                }
                out.params.push(format!("{name}: {ty}"));
                out.types.push(ty.to_string());
                out.names.push(name);
            }
        }
    }
    match &sig.output {
        syn::ReturnType::Default => {}
        syn::ReturnType::Type(_, ty) => {
            let ts = ty.to_token_stream();
            if mentions(ts.clone(), "impl") || mentions(ts.clone(), "_") {
                return None;
            }
            if sig.asyncness.is_some() && borrows(ts.clone()) {
                return None;
            }
            out.ret = ts.to_string();
        }
    }
    Some(out)
}

fn wrapper(vis: &str, name: &str, shape: &Shape, tail: &str, call: &str) -> String {
    let args = shape
        .names
        .iter()
        .map(|n| if n == "self" { "self".to_string() } else { n.clone() })
        .collect::<Vec<_>>()
        .join(", ");
    let sig_ty = format!("fn({}) -> {}", shape.types.join(", "), shape.ret);
    let body = format!("{call}({args})");
    format!(
        "{vis}fn {name}({params}) -> {ret} {{ if let Some(__p) = ::yawn::dynamic::entry_raw(concat!(module_path!(), {tail:?})) {{ let __f: {sig_ty} = unsafe {{ ::core::mem::transmute::<usize, {sig_ty}>(__p) }}; return __f({args}); }} {body} }} #[doc(hidden)] #[allow(dead_code, clippy::all)] ",
        params = shape.params.join(", "),
        ret = shape.ret,
    )
}

fn wrapper_async(vis: &str, name: &str, shape: &Shape, tail: &str, call: &str) -> String {
    let args = shape.names.join(", ");
    let (hr, generics, bound) =
        if shape.borrowed { ("for<'__y> ", "<'__y>", " + '__y") } else { ("", "", "") };
    let boxed = format!(
        "::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = {}> + ::core::marker::Send{bound}>>",
        shape.ret
    );
    let sig_ty = format!("{hr}fn({}) -> {boxed}", shape.lifetime_types.join(", "));
    let params = shape.params.join(", ");
    let boxed_params = shape.lifetime_params.join(", ");
    format!(
        "{vis}async fn {name}({params}) -> {ret} {{ if let Some(__p) = ::yawn::dynamic::entry_raw(concat!(module_path!(), {tail:?})) {{ let __f: {sig_ty} = unsafe {{ ::core::mem::transmute::<usize, {sig_ty}>(__p) }}; return __f({args}).await; }} {call}({args}).await }} #[doc(hidden)] #[allow(dead_code, clippy::all)] fn __yawn_boxed_{name}{generics}({boxed_params}) -> {boxed} {{ ::std::boxed::Box::pin(__yawn_body_{name}({args})) }} #[doc(hidden)] #[allow(dead_code, clippy::all)] ",
        ret = shape.ret,
    )
}

fn vis_start(
    text: &str,
    starts: &[usize],
    vis: &syn::Visibility,
    fn_span: proc_macro2::Span,
) -> Option<usize> {
    match vis {
        syn::Visibility::Inherited => span_start(text, starts, fn_span),
        syn::Visibility::Public(t) => span_start(text, starts, t.span),
        syn::Visibility::Restricted(r) => span_start(text, starts, r.pub_token.span),
    }
}

fn vis_text(vis: &syn::Visibility) -> String {
    match vis {
        syn::Visibility::Inherited => String::new(),
        v => format!("{} ", v.to_token_stream()),
    }
}

struct Statics<'a> {
    text: &'a str,
    starts: &'a [usize],
    unit: String,
    seen: std::collections::BTreeMap<String, usize>,
    edits: Vec<Edit>,
}

struct LocalDecl {
    attrs: Vec<syn::Attribute>,
    vis: syn::Visibility,
    name: syn::Ident,
    ty: syn::Type,
    init: proc_macro2::TokenStream,
}

struct LocalDecls(Vec<LocalDecl>);

impl syn::parse::Parse for LocalDecls {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut out = Vec::new();
        while !input.is_empty() {
            let attrs = input.call(syn::Attribute::parse_outer)?;
            let vis: syn::Visibility = input.parse()?;
            input.parse::<syn::Token![static]>()?;
            let name: syn::Ident = input.parse()?;
            input.parse::<syn::Token![:]>()?;
            let ty: syn::Type = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            let init = if input.peek(syn::Token![const]) && input.peek2(syn::token::Brace) {
                input.parse::<syn::Token![const]>()?;
                let content;
                syn::braced!(content in input);
                content.parse::<proc_macro2::TokenStream>()?
            } else {
                input.parse::<syn::Expr>()?.to_token_stream()
            };
            if input.peek(syn::Token![;]) {
                input.parse::<syn::Token![;]>()?;
            }
            out.push(LocalDecl { attrs, vis, name, ty, init });
        }
        Ok(LocalDecls(out))
    }
}

impl Statics<'_> {
    fn symbol(&mut self, name: &str) -> String {
        let k = self.seen.entry(name.to_string()).or_insert(0);
        let symbol = format!("__yawn_s_{}_{}_{}", self.unit, name, k);
        *k += 1;
        symbol
    }

    fn thread_locals(&mut self, m: &syn::Macro) -> Option<()> {
        if m.path.segments.last()?.ident != "thread_local" {
            return None;
        }
        let decls: LocalDecls = syn::parse2(m.tokens.clone()).ok()?;
        let at = span_start(self.text, self.starts, m.path.segments.first()?.ident.span())?;
        let close = match &m.delimiter {
            syn::MacroDelimiter::Paren(d) => d.span.close(),
            syn::MacroDelimiter::Brace(d) => d.span.close(),
            syn::MacroDelimiter::Bracket(d) => d.span.close(),
        };
        let mut end = span_end(self.text, self.starts, close)?;
        if self.text[end..].trim_start().starts_with(';') {
            end += self.text[end..].find(';')? + 1;
        }
        let original = self.text.get(at..end)?.trim_end_matches(';').to_string();
        let mut out = format!("#[cfg(target_arch = \"wasm32\")] {original} ");
        for d in &decls.0 {
            let symbol = self.symbol(&d.name.to_string());
            let attrs: String =
                d.attrs.iter().map(|a| format!("{} ", a.to_token_stream())).collect();
            out.push_str(&format!(
                "#[cfg(not(target_arch = \"wasm32\"))] {attrs}#[unsafe(export_name = {symbol:?})] {}static {}: ::yawn::dynamic::DevLocal<{}> = ::yawn::dynamic::DevLocal::new(|| {{ {} }}); ",
                vis_text(&d.vis),
                d.name,
                d.ty.to_token_stream(),
                d.init
            ));
        }
        self.edits.push(Edit { at, end, text: out });
        Some(())
    }
}

impl<'ast> syn::visit::Visit<'ast> for Statics<'_> {
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if self.thread_locals(m).is_none() {
            syn::visit::visit_macro(self, m);
        }
    }

    fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
        let shared = i.attrs.iter().any(|a| {
            let p = a.path();
            p.is_ident("no_mangle")
                || p.is_ident("export_name")
                || p.is_ident("link_section")
                || p.is_ident("unsafe")
                || p.is_ident("thread_local")
        });
        if !shared && let Some(at) = vis_start(self.text, self.starts, &i.vis, i.static_token.span)
        {
            let name = i.ident.to_string();
            let symbol = self.symbol(&name);
            self.edits.push(Edit {
                at,
                end: at,
                text: format!("#[unsafe(export_name = {symbol:?})] "),
            });
        }
        syn::visit::visit_item_static(self, i);
    }
}

/// Whether every async function of the server's own modules may be made replaceable (their
/// futures must be `Send`, as the handlers' are), not just the handlers
/// How many files of the project's `src/` mention each word, to tell a `pub fn` only its own file knows
/// (so a change of its signature has no caller elsewhere to break) from one other files may call
static WORDS: std::sync::Mutex<Option<std::collections::HashMap<String, usize>>> =
    std::sync::Mutex::new(None);

pub fn index_words(src: &Path) {
    fn walk(dir: &Path, counts: &mut std::collections::HashMap<String, usize>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, counts);
            } else if p.extension().is_some_and(|x| x == "rs" || x == "rsx") {
                let Ok(text) = std::fs::read_to_string(&p) else { continue };
                let words: std::collections::HashSet<&str> = text
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .filter(|w| !w.is_empty())
                    .collect();
                for w in words {
                    *counts.entry(w.to_string()).or_insert(0) += 1;
                }
            }
        }
    }
    let mut counts = std::collections::HashMap::new();
    walk(src, &mut counts);
    if let Ok(mut g) = WORDS.lock() {
        *g = Some(counts);
    }
}

fn known_only_here(name: &str) -> bool {
    WORDS
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|m| m.get(name).copied() == Some(1)))
        .unwrap_or(false)
}

pub static ASYNC_ALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

struct Work<'a> {
    text: &'a str,
    unit: String,
    sigs: Vec<String>,
    hot_pub: bool,
    starts: Vec<usize>,
    edits: Vec<Edit>,
    skeleton: Vec<(usize, usize)>,
    registered: Vec<Registered>,
    handlers: bool,
    async_all: bool,
    item_ranges: Vec<(usize, usize)>,
    items_known: bool,
}

fn item_hashes(text: &str, ranges: &[(usize, usize)], cuts: &[(usize, usize)]) -> Vec<String> {
    ranges
        .iter()
        .filter_map(|&(a, b)| {
            let mut one = String::new();
            let mut at = a;
            for &(ca, cb) in cuts.iter().filter(|&&(ca, cb)| ca < b && cb > a) {
                one.push_str(text.get(at..ca.max(at))?);
                at = cb.max(at);
            }
            one.push_str(text.get(at..b)?);
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&without_bodies(&one), &mut h);
            Some(format!("{:016x}", std::hash::Hasher::finish(&h)))
        })
        .collect()
}

fn item_range(text: &str, starts: &[usize], item: &syn::Item) -> Option<(usize, usize)> {
    let tokens: Vec<proc_macro2::TokenTree> = item.to_token_stream().into_iter().collect();
    let first = span_start(text, starts, tokens.first()?.span())?;
    let last = span_end(text, starts, tokens.last()?.span())?;
    Some((first, last))
}

impl Work<'_> {
    fn function(
        &mut self,
        vis: &syn::Visibility,
        sig: &syn::Signature,
        block: &syn::Block,
        attrs: &[syn::Attribute],
        self_ty: Option<&str>,
    ) -> Option<()> {
        let name = sig.ident.to_string();
        if !plain_attrs(attrs) || name.starts_with("__yawn") || name == "main" {
            return None;
        }
        let is_async = sig.asyncness.is_some();
        let handler = matches!(
            name.as_str(),
            "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
        ) || self.async_all;
        let shape = shape(sig, self_ty, self.handlers && handler && is_async)?;
        let starts = &self.starts;
        let text = self.text;
        let ident_at = span_start(text, starts, sig.ident.span())?;
        let ident_end = span_end(text, starts, sig.ident.span())?;
        let open = span_start(text, starts, block.brace_token.span.open())?;
        let close = span_end(text, starts, block.brace_token.span.close())?;
        let begin = match (vis, &sig.asyncness) {
            (syn::Visibility::Inherited, Some(a)) => span_start(text, starts, a.span)?,
            _ => vis_start(text, starts, vis, sig.fn_token.span)?,
        };
        let inherited = matches!(vis, syn::Visibility::Inherited);
        let hot_pub = self_ty.is_none() && !inherited && !self.handlers && self.hot_pub;
        let private = self_ty.is_none()
            && (inherited
                || hot_pub
                || (matches!(vis, syn::Visibility::Public(_))
                    && !self.handlers
                    && known_only_here(&name)));
        let sig_hash = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&(&shape.types, &shape.ret, is_async), &mut h);
            format!("{:08x}", std::hash::Hasher::finish(&h) as u32)
        };
        let key_tail = match self_ty {
            Some(ty) => format!("::{ty}::{name}"),
            None if private => format!("::{name}#{sig_hash}"),
            None => format!("::{name}"),
        };
        let vis_prefix = if hot_pub {
            self.sigs.push(format!("{name}#{sig_hash}"));
            format!(
                "#[unsafe(export_name = \"__yawn_pub_{}_{name}_{sig_hash}\")] {}",
                self.unit,
                vis_text(vis)
            )
        } else {
            vis_text(vis)
        };
        let body_name = format!("__yawn_body_{name}");
        let call = match (self_ty, shape.receiver.is_some()) {
            (Some(_), _) => format!("Self::{body_name}"),
            (None, _) => body_name.clone(),
        };
        let target = match (self_ty, is_async) {
            (Some(ty), _) => format!("{ty}::{body_name}"),
            (None, true) => format!("__yawn_boxed_{name}"),
            (None, false) => body_name.clone(),
        };
        let w = if is_async {
            wrapper_async(&vis_prefix, &name, &shape, &key_tail, &call)
        } else {
            wrapper(&vis_prefix, &name, &shape, &key_tail, &call)
        };
        self.edits.push(Edit { at: begin, end: begin, text: w });
        self.edits.push(Edit { at: ident_at, end: ident_end, text: body_name });
        if private {
            self.skeleton.push((begin, close));
        } else {
            self.skeleton.push((open + 1, close.saturating_sub(1)));
        }
        self.registered.push(Registered { key: key_tail, target });
        Some(())
    }
}

fn includes_rewritten(text: &str, dir: &Path) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: loop {
        let mut next: Option<(usize, usize)> = None;
        for name in ["include_str!", "include_bytes!", "include!"] {
            if let Some(i) = rest.find(name)
                && next.is_none_or(|(j, _)| i < j)
            {
                next = Some((i, name.len()));
            }
        }
        let Some((i, len)) = next else { break 'scan };
        out.push_str(&rest[..i + len]);
        rest = &rest[i + len..];
        let trimmed = rest.trim_start();
        let Some(inner) = trimmed.strip_prefix('(') else { return None };
        out.push_str(&rest[..rest.len() - trimmed.len() + 1]);
        let inner_trim = inner.trim_start();
        let mut lead = inner[..inner.len() - inner_trim.len()].to_string();
        let mut inner_trim = inner_trim;
        let mut joined = false;
        if let Some(r) = inner_trim.strip_prefix("concat!") {
            let r = r.trim_start();
            let Some(r) = r.strip_prefix('(') else { return None };
            let r_trim = r.trim_start();
            lead.push_str("concat!(");
            lead.push_str(&r[..r.len() - r_trim.len()]);
            inner_trim = r_trim;
            joined = true;
        }
        let Some(lit) = inner_trim.strip_prefix('"') else { return None };
        let Some(end) = lit.find('"') else { return None };
        let path = &lit[..end];
        if path.contains('\\') {
            return None;
        }
        let after = &lit[end + 1..];
        if !joined && !after.trim_start().starts_with(')') {
            return None;
        }
        out.push_str(&lead);
        let full = if Path::new(path).is_absolute() {
            path.to_string()
        } else {
            dir.join(path).to_string_lossy().replace('\\', "/")
        };
        out.push_str(&format!("{full:?}"));
        rest = after;
    }
    out.push_str(rest);
    Some(out)
}

fn has_path_attr(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("#[path") || t.starts_with("#![path")
    })
}

/// The text of the mirror of a source file: its relative `include_*!` paths made absolute, and its
/// eligible functions made replaceable. `None` when the file uses something the mirror cannot
/// keep working (a computed include, a `#[path]`). `unit` names the file, so that the function that
/// registers its replacements has a name of its own.
pub fn transform(text: &str, source_dir: &Path, unit: &str) -> Option<String> {
    if has_path_attr(text) {
        return None;
    }
    let text = includes_rewritten(text, source_dir)?;
    let Ok(file) = syn::parse_file(&text) else { return Some(text) };
    let mut work = Work {
        text: &text,
        unit: unit.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect(),
        sigs: Vec::new(),
        hot_pub: HOT_PUB.load(std::sync::atomic::Ordering::Relaxed),
        starts: line_starts(&text),
        edits: Vec::new(),
        skeleton: Vec::new(),
        registered: Vec::new(),
        handlers: Path::new(unit).starts_with("api")
            || Path::new(unit).starts_with("server")
            || Path::new(unit) == Path::new("server.rs"),
        async_all: ASYNC_ALL.load(std::sync::atomic::Ordering::Relaxed),
        item_ranges: Vec::new(),
        items_known: true,
    };
    for item in &file.items {
        match item_range(&text, &work.starts, item) {
            Some(r) => work.item_ranges.push(r),
            None => work.items_known = false,
        }
        match item {
            syn::Item::Fn(f) => {
                work.function(&f.vis, &f.sig, &f.block, &f.attrs, None);
            }
            syn::Item::Impl(i)
                if i.trait_.is_none()
                    && i.generics.params.is_empty()
                    && i.generics.where_clause.is_none()
                    && i.modifiers.defaultness.is_none()
                    && i.unsafety.is_none()
                    && plain_attrs(&i.attrs) =>
            {
                let ty = i.self_ty.to_token_stream();
                if mentions(ty.clone(), "impl") || mentions(ty.clone(), "_") {
                    continue;
                }
                let ty = ty.to_string();
                for member in &i.items {
                    if let syn::ImplItem::Fn(f) = member {
                        work.function(&f.vis, &f.sig, &f.block, &f.attrs, Some(&ty));
                    }
                }
            }
            _ => {}
        }
    }
    let mut statics = Statics {
        text: &text,
        starts: &work.starts,
        unit: unit.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect(),
        seen: Default::default(),
        edits: Vec::new(),
    };
    syn::visit::Visit::visit_file(&mut statics, &file);
    work.edits.extend(statics.edits);
    if work.edits.is_empty() {
        if work.items_known && !unit.is_empty() {
            let mut items = item_hashes(&text, &work.item_ranges, &[]);
            items.sort();
            let mut out = text.clone();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("{ITEMS}{}\n", items.join(" ")));
            return Some(out);
        }
        return Some(text);
    }
    finish(work, unit)
}

pub fn statics_only(text: &str, unit: &str) -> Option<String> {
    let text = text.to_string();
    let Ok(file) = syn::parse_file(&text) else { return Some(text) };
    let starts = line_starts(&text);
    let mut statics = Statics {
        text: &text,
        starts: &starts,
        unit: unit.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect(),
        seen: Default::default(),
        edits: Vec::new(),
    };
    syn::visit::Visit::visit_file(&mut statics, &file);
    let mut edits = statics.edits;
    edits.sort_by_key(|e| e.at);
    let mut out = String::with_capacity(text.len() + edits.len() * 60);
    let mut at = 0;
    for e in &edits {
        out.push_str(text.get(at..e.at)?);
        out.push_str(&e.text);
        at = e.end;
    }
    out.push_str(text.get(at..)?);
    Some(out)
}

/// `text` without what the generated code of a component keeps between its body markers: those
/// bodies are swapped by their own mechanism, so editing one leaves the skeleton as it was
fn without_bodies(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if t == crate::codegen::BODY_BEGIN {
            inside = true;
        } else if t == crate::codegen::BODY_END {
            inside = false;
        } else if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn finish(work: Work<'_>, unit: &str) -> Option<String> {
    let text = work.text;
    let mut cuts = work.skeleton.clone();
    cuts.sort();
    let mut skeleton = String::new();
    let mut last = 0;
    for (a, b) in &cuts {
        skeleton.push_str(text.get(last..*a)?);
        last = *b;
    }
    skeleton.push_str(text.get(last..)?);
    let skeleton = without_bodies(&skeleton);
    let items: Option<Vec<String>> =
        work.items_known.then(|| item_hashes(text, &work.item_ranges, &cuts));
    let mut edits = work.edits;
    edits.sort_by_key(|e| (e.at, e.end));
    let mut out = String::with_capacity(text.len() + edits.len() * 400);
    let mut at = 0;
    for e in &edits {
        out.push_str(text.get(at..e.at)?);
        out.push_str(&e.text);
        at = e.end;
    }
    out.push_str(text.get(at..)?);
    let ident: String =
        unit.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let registrar = format!("__yawn_register_fns_{ident}");
    let calls = work
        .registered
        .iter()
        .map(|r| {
            format!(
                "::yawn::dynamic::register_raw(concat!(module_path!(), {:?}), {} as usize);",
                r.key, r.target
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!(
        "#[doc(hidden)] #[allow(dead_code, clippy::all)] #[unsafe(no_mangle)] pub extern \"C\" fn {registrar}() {{ {calls} }}\n"
    ));
    out.push_str(&format!(
        "#[cfg(target_os = \"linux\")] #[allow(non_upper_case_globals)] #[used] #[unsafe(link_section = \".init_array\")] static __YAWN_FNS_{ident}: extern \"C\" fn() = {registrar};\n"
    ));
    out.push_str(&format!(
        "#[cfg(target_os = \"macos\")] #[allow(non_upper_case_globals)] #[used] #[unsafe(link_section = \"__DATA,__mod_init_func\")] static __YAWN_FNS_MAC_{ident}: extern \"C\" fn() = {registrar};\n"
    ));
    let mut sigs = work.sigs.clone();
    sigs.sort();
    if !sigs.is_empty() {
        out.push_str(&format!("{SIGS}{}\n", sigs.join(" ")));
    }
    if let Some(items) = items {
        let mut items = items;
        items.sort();
        out.push_str(&format!("{ITEMS}{}\n", items.join(" ")));
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&skeleton, &mut h);
    out.push_str(&format!("{SKELETON}{:016x}\n", std::hash::Hasher::finish(&h)));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"use std::fmt;

/// Adds.
#[inline]
pub fn add(a: u32, (b, c): (u32, u32)) -> u32 {
    a + b + c
}

pub(crate) fn name(s: &str) -> &str
{
    s
}

fn generic<T>(t: T) -> T { t }
async fn later() {}
pub fn lit() -> &'static str { include_str!("x.json") }

pub struct Entry(u32);

impl Entry {
    pub fn new(n: u32) -> Self { Entry(n) }
    pub fn get(&self) -> u32 { self.0 }
    fn bump(&mut self, by: u32) { self.0 += by }
    fn take(self) -> u32 { self.0 }
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "{}", self.0) }
}
"#;

    #[test]
    fn keeps_the_lines_and_parses() {
        let out = transform(SAMPLE, Path::new("/p/src"), "m").unwrap();
        println!("{out}");
        syn::parse_file(&out).unwrap();
        let before: Vec<&str> = SAMPLE.lines().collect();
        let after: Vec<&str> = out.lines().collect();
        assert!(after.len() > before.len());
        assert!(out.contains("__yawn_body_add"));
        assert!(out.contains("__yawn_body_new"));
        assert!(!out.contains("__yawn_body_generic"));
        assert!(!out.contains("__yawn_body_later"));
        assert!(!out.contains("__yawn_body_fmt"));
        assert!(out.contains("\"/p/src/x.json\""));
        for (i, l) in before.iter().enumerate() {
            if l.contains("a + b + c") || l.trim() == "s" {
                assert_eq!(after[i], *l);
            }
        }
    }

    #[test]
    fn a_public_function_can_change_its_signature_without_changing_the_skeleton() {
        let text = "pub fn shown() -> u32 { helper(1) }\nfn helper(x: u32) -> u32 { x + 1 }\npub struct S(u32);\n";
        let last = |t: &str| t.lines().last().unwrap().to_string();
        let a = transform(text, Path::new("/p/src"), "m").unwrap();
        let b = transform(
            &text.replace("x: u32) -> u32 { x + 1 }", "x: u64, y: u64) -> u32 { (x + y) as u32 }"),
            Path::new("/p/src"),
            "m",
        )
        .unwrap();
        let c = transform(
            &text.replace("pub fn shown() -> u32", "pub fn shown() -> u64"),
            Path::new("/p/src"),
            "m",
        )
        .unwrap();
        let d = transform(
            &text.replace("pub struct S(u32)", "pub struct S(u64)"),
            Path::new("/p/src"),
            "m",
        )
        .unwrap();
        assert_eq!(last(&a), last(&b));
        assert_eq!(last(&a), last(&c));
        assert_ne!(last(&a), last(&d));
        assert_ne!(sigs(&a), sigs(&c));
        assert_eq!(sigs(&a), sigs(&b));
        assert!(sigs(&a).contains_key("shown"));
        assert!(a.contains("export_name = \"__yawn_pub_m_shown_"));
    }

    #[test]
    fn a_body_edit_keeps_the_skeleton_and_a_struct_edit_changes_it() {
        let a = transform(SAMPLE, Path::new("/p/src"), "m").unwrap();
        let b =
            transform(&SAMPLE.replace("a + b + c", "a * b * c"), Path::new("/p/src"), "m").unwrap();
        let c = transform(
            &SAMPLE.replace("pub struct Entry(u32);", "pub struct Entry(u64);"),
            Path::new("/p/src"),
            "m",
        )
        .unwrap();
        let tail = |t: &str| t.lines().last().unwrap().to_string();
        assert_ne!(a, b);
        assert_eq!(tail(&a), tail(&b));
        assert_ne!(tail(&a), tail(&c));
    }
}
