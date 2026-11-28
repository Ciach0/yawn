//! `#[derive(Bits)]` for structs and enums, written over the bare token stream: no syn, no quote,
//! so the derive adds nothing to the compile time of a project.
//!
//! The attributes: `#[bits(crate = "path")]` on the type names the crate the generated code
//! calls (default `::yawn::bits`), and `#[bits(opaque)]` on a field hashes the field's written
//! type instead of its `SCHEMA`, which a type that contains itself through a `Vec` needs.

use proc_macro::{Delimiter, Group, Spacing, TokenStream, TokenTree};

#[proc_macro_derive(Bits, attributes(bits))]
pub fn derive_bits(input: TokenStream) -> TokenStream {
    match expand(input) {
        Ok(out) => out.parse().expect("generated code parses"),
        Err(message) => format!("::core::compile_error!({message:?});").parse().unwrap(),
    }
}

struct Field {
    name: String,
    ty: String,
    opaque: bool,
}

enum Shape {
    Unit,
    Tuple(Vec<Field>),
    Named(Vec<Field>),
}

struct Variant {
    name: String,
    shape: Shape,
}

enum Body {
    Struct(Shape),
    Enum(Vec<Variant>),
}

struct Tokens {
    items: Vec<TokenTree>,
    at: usize,
}

impl Tokens {
    fn peek(&self) -> Option<&TokenTree> {
        self.items.get(self.at)
    }
    fn next(&mut self) -> Option<TokenTree> {
        let t = self.items.get(self.at).cloned();
        self.at += 1;
        t
    }
    fn punct(&self, c: char) -> bool {
        matches!(self.peek(), Some(TokenTree::Punct(p)) if p.as_char() == c)
    }
    fn ident(&self) -> Option<String> {
        match self.peek() {
            Some(TokenTree::Ident(i)) => Some(i.to_string()),
            _ => None,
        }
    }
}

fn tokens(stream: TokenStream) -> Tokens {
    Tokens { items: stream.into_iter().collect(), at: 0 }
}

struct Attrs {
    crate_path: Option<String>,
    opaque: bool,
}

fn attributes(t: &mut Tokens) -> Result<Attrs, String> {
    let mut out = Attrs { crate_path: None, opaque: false };
    while t.punct('#') {
        t.next();
        let Some(TokenTree::Group(g)) = t.next() else {
            return Err("a malformed attribute".into());
        };
        let mut inner = tokens(g.stream());
        match inner.ident().as_deref() {
            Some("bits") => {
                inner.next();
                let Some(TokenTree::Group(args)) = inner.next() else { continue };
                let mut a = tokens(args.stream());
                while let Some(name) = a.ident() {
                    a.next();
                    match name.as_str() {
                        "opaque" => out.opaque = true,
                        "crate" => {
                            a.next();
                            if let Some(TokenTree::Literal(l)) = a.next() {
                                out.crate_path = Some(l.to_string().trim_matches('"').to_string());
                            }
                        }
                        other => return Err(format!("unknown bits attribute `{other}`")),
                    }
                    if a.punct(',') {
                        a.next();
                    }
                }
            }
            Some("serde") => {
                inner.next();
                if let Some(TokenTree::Group(args)) = inner.next() {
                    check_serde(args.stream())?;
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// A serde attribute that changes what is written is refused: a positional format cannot do it
fn check_serde(args: TokenStream) -> Result<(), String> {
    const REFUSED: &[(&str, &str)] = &[
        ("flatten", "a flattened field has no place in a positional encoding: use a named field"),
        ("untagged", "an untagged enum cannot be told apart without its content: use a tagged one"),
        ("tag", "an internally tagged enum needs field names: Bits writes the variant as a number"),
        (
            "content",
            "an adjacently tagged enum needs field names: Bits writes the variant as a number",
        ),
        ("skip", "a skipped field changes the shape: remove the field or move it out of the type"),
        ("skip_serializing", "a skipped field changes the shape: remove it"),
        ("skip_deserializing", "a skipped field changes the shape: remove it"),
        (
            "skip_serializing_if",
            "an omitted field would shift the ones after it: write the field always",
        ),
        ("default", "a default needs a missing field, and a positional message never lacks one"),
        ("with", "a custom serializer is not used by Bits: implement Bits for the type"),
        ("serialize_with", "a custom serializer is not used by Bits: implement Bits for the type"),
        (
            "deserialize_with",
            "a custom deserializer is not used by Bits: implement Bits for the type",
        ),
        ("transparent", "write the inner type directly"),
        ("from", "a conversion is not used by Bits: implement Bits for the type"),
        ("into", "a conversion is not used by Bits: implement Bits for the type"),
        ("try_from", "a conversion is not used by Bits: implement Bits for the type"),
    ];
    for t in args {
        if let TokenTree::Ident(i) = t {
            let name = i.to_string();
            if let Some((_, why)) = REFUSED.iter().find(|(n, _)| *n == name) {
                return Err(format!("`serde({name})` cannot be used with Bits: {why}"));
            }
        }
    }
    Ok(())
}

fn skip_visibility(t: &mut Tokens) -> String {
    let mut vis = String::new();
    if t.ident().as_deref() == Some("pub") {
        t.next();
        vis.push_str("pub");
        if let Some(TokenTree::Group(g)) = t.peek()
            && g.delimiter() == Delimiter::Parenthesis
        {
            vis.push_str(&g.to_string());
            t.next();
        }
    }
    vis
}

fn render(items: &[TokenTree]) -> String {
    let mut out = String::new();
    for t in items {
        match t {
            TokenTree::Punct(p) => {
                out.push(p.as_char());
                if p.spacing() == Spacing::Alone && matches!(p.as_char(), ',') {
                    out.push(' ');
                }
            }
            other => {
                if !out.is_empty() && !out.ends_with(['<', ':', '&', '(', '[']) {
                    out.push(' ');
                }
                out.push_str(&other.to_string());
            }
        }
    }
    out
}

fn type_until_comma(t: &mut Tokens) -> String {
    let mut depth = 0i32;
    let mut taken = Vec::new();
    while let Some(tt) = t.peek() {
        if let TokenTree::Punct(p) = tt {
            match p.as_char() {
                '<' => depth += 1,
                '>' => depth -= 1,
                ',' if depth <= 0 => break,
                _ => {}
            }
        }
        taken.push(t.next().unwrap());
    }
    render(&taken)
}

fn fields(group: &Group, named: bool) -> Result<Vec<Field>, String> {
    let mut t = tokens(group.stream());
    let mut out = Vec::new();
    while t.peek().is_some() {
        let attrs = attributes(&mut t)?;
        skip_visibility(&mut t);
        let name = if named {
            let Some(n) = t.ident() else { return Err("expected a field name".into()) };
            t.next();
            if !t.punct(':') {
                return Err("expected `:` after a field name".into());
            }
            t.next();
            n
        } else {
            out.len().to_string()
        };
        let ty = type_until_comma(&mut t);
        if t.punct(',') {
            t.next();
        }
        out.push(Field { name, ty, opaque: attrs.opaque });
    }
    Ok(out)
}

fn shape(t: &mut Tokens) -> Result<Shape, String> {
    match t.peek() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
            let g = g.clone();
            t.next();
            Ok(Shape::Named(fields(&g, true)?))
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
            let g = g.clone();
            t.next();
            Ok(Shape::Tuple(fields(&g, false)?))
        }
        _ => Ok(Shape::Unit),
    }
}

fn expand(input: TokenStream) -> Result<String, String> {
    let mut t = tokens(input);
    let attrs = attributes(&mut t)?;
    let vis = skip_visibility(&mut t);
    let kind = t.ident().ok_or("expected a struct or an enum")?;
    t.next();
    let name = t.ident().ok_or("expected a name")?;
    t.next();
    if t.punct('<') {
        return Err("Bits does not support generic types yet: write the concrete type".into());
    }
    let body = match kind.as_str() {
        "struct" => {
            let s = shape(&mut t)?;
            Body::Struct(s)
        }
        "enum" => {
            let Some(TokenTree::Group(g)) = t.next() else {
                return Err("expected the variants".into());
            };
            let mut v = tokens(g.stream());
            let mut variants = Vec::new();
            while v.peek().is_some() {
                attributes(&mut v)?;
                let Some(vname) = v.ident() else { return Err("expected a variant name".into()) };
                v.next();
                let s = shape(&mut v)?;
                if v.punct('=') {
                    while v.peek().is_some() && !v.punct(',') {
                        v.next();
                    }
                }
                if v.punct(',') {
                    v.next();
                }
                variants.push(Variant { name: vname, shape: s });
            }
            if variants.is_empty() {
                return Err("an enum without variants has no values to write".into());
            }
            if variants.len() > 255 {
                return Err("Bits writes the variant in one byte: at most 255 variants".into());
            }
            Body::Enum(variants)
        }
        _ => return Err("Bits can be derived for structs and enums".into()),
    };
    let krate = attrs.crate_path.unwrap_or_else(|| "::yawn::bits".into());
    Ok(generate(&krate, &name, &vis, &body))
}

fn field_schema(krate: &str, own: &str, f: &Field) -> String {
    let mentions_self = f.ty.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == own);
    if f.opaque || mentions_self {
        format!("{krate}::schema::text({:?})", f.ty)
    } else {
        format!("<{} as {krate}::Bits>::SCHEMA", f.ty)
    }
}

fn parts(krate: &str, own: &str, fs: &[Field]) -> String {
    let mut out = String::from("&[");
    for f in fs {
        out += &format!("({:?}, {}), ", f.name, field_schema(krate, own, f));
    }
    out + "]"
}

fn shape_schema(krate: &str, own: &str, vname: &str, s: &Shape) -> String {
    match s {
        Shape::Unit => format!("{krate}::schema::shape({vname:?}, &[])"),
        Shape::Tuple(fs) | Shape::Named(fs) => {
            format!("{krate}::schema::shape({vname:?}, {})", parts(krate, own, fs))
        }
    }
}

fn binds(fs: &[Field], prefix: &str) -> Vec<String> {
    (0..fs.len()).map(|i| format!("{prefix}{i}")).collect()
}

fn generate(krate: &str, name: &str, vis: &str, body: &Body) -> String {
    let (schema, put, get) = match body {
        Body::Struct(s) => {
            let schema = shape_schema(krate, name, &format!("struct {name}"), s);
            let (put, get) = match s {
                Shape::Unit => (String::new(), name.to_string()),
                Shape::Tuple(fs) => (
                    (0..fs.len()).map(|i| format!("{krate}::Bits::put(&self.{i}, w);")).collect(),
                    format!(
                        "{name}({})",
                        vec![format!("{krate}::Bits::get(r)"); fs.len()].join(", ")
                    ),
                ),
                Shape::Named(fs) => (
                    fs.iter()
                        .map(|f| format!("{krate}::Bits::put(&self.{}, w);", f.name))
                        .collect(),
                    format!(
                        "{name} {{ {} }}",
                        fs.iter()
                            .map(|f| format!("{}: {krate}::Bits::get(r)", f.name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ),
            };
            (schema, put, get)
        }
        Body::Enum(vs) => {
            let mut sch = format!("{krate}::schema::shape(\"enum {name}\", &[");
            let mut put = String::from("match self {");
            let mut get = String::from("match r.byte() {");
            let construct = |v: &Variant| match &v.shape {
                Shape::Unit => format!("{name}::{}", v.name),
                Shape::Tuple(fs) => format!(
                    "{name}::{}({})",
                    v.name,
                    vec![format!("{krate}::Bits::get(r)"); fs.len()].join(", ")
                ),
                Shape::Named(fs) => format!(
                    "{name}::{} {{ {} }}",
                    v.name,
                    fs.iter()
                        .map(|f| format!("{}: {krate}::Bits::get(r)", f.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            for (i, v) in vs.iter().enumerate() {
                sch +=
                    &format!("({:?}, {}), ", v.name, shape_schema(krate, name, &v.name, &v.shape));
                match &v.shape {
                    Shape::Unit => put += &format!("{name}::{} => w.push({i}),", v.name),
                    Shape::Tuple(fs) => {
                        let b = binds(fs, "a");
                        put += &format!(
                            "{name}::{}({}) => {{ w.push({i}); {} }}",
                            v.name,
                            b.join(", "),
                            b.iter()
                                .map(|x| format!("{krate}::Bits::put({x}, w);"))
                                .collect::<String>()
                        );
                    }
                    Shape::Named(fs) => {
                        let b = binds(fs, "a");
                        put += &format!(
                            "{name}::{} {{ {} }} => {{ w.push({i}); {} }}",
                            v.name,
                            fs.iter()
                                .zip(&b)
                                .map(|(f, x)| format!("{}: {x}", f.name))
                                .collect::<Vec<_>>()
                                .join(", "),
                            b.iter()
                                .map(|x| format!("{krate}::Bits::put({x}, w);"))
                                .collect::<String>()
                        );
                    }
                }
                get += &format!("{i} => {},", construct(v));
            }
            sch += "])";
            put += "}";
            get += &format!("_ => {{ r.fail(\"not a variant\"); {} }} }}", construct(&vs[0]));
            (
                sch,
                put,
                format!(
                    "if !r.enter() {{ return {}; }} let v = {get}; r.leave(); v",
                    construct(&vs[0])
                ),
            )
        }
    };
    let (project, pipe) = match body {
        Body::Struct(Shape::Named(fs) | Shape::Tuple(fs)) if !fs.is_empty() => {
            let member = |f: &Field| {
                if f.name.chars().all(|c| c.is_ascii_digit()) {
                    format!("f{}", f.name)
                } else {
                    f.name.clone()
                }
            };
            let mut arms = String::new();
            let mut decl = String::new();
            let mut init = String::new();
            for (i, f) in fs.iter().enumerate() {
                arms += &format!(
                    "Some((&{i}, rest)) => {krate}::Bits::project(&self.{}, rest, w),",
                    f.name
                );
                decl += &format!("{vis} {}: {krate}::Field<{name}, {}>,", member(f), f.ty);
                init += &format!("{}: {krate}::Field::new(&[{i}]),", member(f));
            }
            let project = format!(
                "fn project(&self, path: &[u32], w: &mut ::std::vec::Vec<u8>) -> ::core::option::Option<u64> {{ match path.split_first() {{ None => {{ {krate}::Bits::put(self, w); Some(<Self as {krate}::Bits>::SCHEMA) }} {arms} _ => None }} }}"
            );
            let pipe = format!(
                "{vis} struct {name}Fields {{ {decl} }} impl {krate}::Pipe for {name} {{ type Fields = {name}Fields; fn fields() -> {name}Fields {{ {name}Fields {{ {init} }} }} }}"
            );
            (project, pipe)
        }
        _ => (String::new(), String::new()),
    };
    format!(
        "impl {krate}::Bits for {name} {{ const SCHEMA: u64 = {schema}; \
         fn put(&self, w: &mut ::std::vec::Vec<u8>) {{ {put} }} {project} \
         fn get(r: &mut {krate}::Reader) -> Self {{ {get} }} }} {pipe}"
    )
}
