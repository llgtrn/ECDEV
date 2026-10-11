//! `#[derive(ToJson)]` and `#[derive(FromJson)]` for named-field structs, unit-only enums and
//! internally tagged enums, written against the compiler's `proc_macro` interface alone.
//!
//! Attributes, all spelled `#[json(...)]`:
//! * container: `deny_unknown_fields`, `default` (missing fields come from `Default`),
//!   `rename_all = "SCREAMING_SNAKE_CASE" | "snake_case" | "lowercase" | "camelCase" | "kebab-case"`,
//!   `tag = "name"` (enums: the variant name is a member of the object)
//! * field: `default`, `default = "path::to::fn"`, `skip_if = "path::to::fn"` (omit when true),
//!   `rename = "wire"`
//! * variant: `rename = "wire"`
//!
//! `Option` fields may be absent; every other field must be present unless it has a default.

use proc_macro::{Delimiter, TokenStream, TokenTree};

#[proc_macro_derive(ToJson, attributes(json))]
pub fn derive_to_json(input: TokenStream) -> TokenStream {
    expand(input, true)
}

#[proc_macro_derive(FromJson, attributes(json))]
pub fn derive_from_json(input: TokenStream) -> TokenStream {
    expand(input, false)
}

fn expand(input: TokenStream, to_json: bool) -> TokenStream {
    let code = match parse(input).and_then(|item| {
        if to_json {
            gen_to(&item)
        } else {
            gen_from(&item)
        }
    }) {
        Ok(code) => code,
        Err(message) => format!("compile_error!({message:?});"),
    };
    code.parse().expect("the generated code is valid Rust")
}

// ---- model ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Options {
    deny_unknown_fields: bool,
    default_all: bool,
    default_field: Option<Default_>,
    rename_all: Option<String>,
    tag: Option<String>,
    skip_if: Option<String>,
    rename: Option<String>,
}

enum Default_ {
    Trait,
    With(String),
}

struct Field {
    name: String,
    options: Options,
}

struct Variant {
    name: String,
    options: Options,
    fields: Option<Vec<Field>>,
}

enum Body {
    Struct(Vec<Field>),
    Enum(Vec<Variant>),
}

struct Item {
    name: String,
    options: Options,
    body: Body,
}

// ---- parsing -------------------------------------------------------------------------------------

fn parse(input: TokenStream) -> Result<Item, String> {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let mut i = 0;
    let options = take_attributes(&tokens, &mut i)?;
    skip_visibility(&tokens, &mut i);
    let kind = match tokens.get(i) {
        Some(TokenTree::Ident(id)) => id.to_string(),
        _ => return Err("expected a struct or an enum".into()),
    };
    i += 1;
    let name = match tokens.get(i) {
        Some(TokenTree::Ident(id)) => id.to_string(),
        _ => return Err("expected a name".into()),
    };
    i += 1;
    if matches!(tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == '<') {
        return Err("generic types are not supported by the JSON derive".into());
    }
    let body = match tokens.get(i) {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => g.stream(),
        _ => {
            return Err(
                "only types with named fields (or enums) are supported by the JSON derive".into(),
            );
        }
    };
    let body = match kind.as_str() {
        "struct" => Body::Struct(parse_fields(body)?),
        "enum" => Body::Enum(parse_variants(body)?),
        _ => return Err("the JSON derive applies to structs and enums".into()),
    };
    Ok(Item {
        name,
        options,
        body,
    })
}

fn skip_visibility(tokens: &[TokenTree], i: &mut usize) {
    if matches!(tokens.get(*i), Some(TokenTree::Ident(id)) if id.to_string() == "pub") {
        *i += 1;
        if matches!(tokens.get(*i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis)
        {
            *i += 1;
        }
    }
}

/// Reads `#[...]` attributes at `*i`, returning the merged `json` options and skipping the others.
fn take_attributes(tokens: &[TokenTree], i: &mut usize) -> Result<Options, String> {
    let mut options = Options::default();
    while matches!(tokens.get(*i), Some(TokenTree::Punct(p)) if p.as_char() == '#') {
        let Some(TokenTree::Group(g)) = tokens.get(*i + 1) else {
            return Err("malformed attribute".into());
        };
        *i += 2;
        let inner: Vec<TokenTree> = g.stream().into_iter().collect();
        let is_json =
            matches!(inner.first(), Some(TokenTree::Ident(id)) if id.to_string() == "json");
        if !is_json {
            continue;
        }
        let Some(TokenTree::Group(args)) = inner.get(1) else {
            return Err("expected json(...)".into());
        };
        for part in split_commas(&args.stream().into_iter().collect::<Vec<_>>()) {
            apply_option(&mut options, &part)?;
        }
    }
    Ok(options)
}

fn apply_option(options: &mut Options, part: &[TokenTree]) -> Result<(), String> {
    let key = match part.first() {
        Some(TokenTree::Ident(id)) => id.to_string(),
        _ => return Err("expected an option name".into()),
    };
    let value = match (part.get(1), part.get(2)) {
        (Some(TokenTree::Punct(p)), Some(TokenTree::Literal(l))) if p.as_char() == '=' => {
            let text = l.to_string();
            let inner = text.strip_prefix('"').and_then(|t| t.strip_suffix('"'));
            Some(inner.ok_or("expected a string literal")?.to_string())
        }
        (None, None) => None,
        _ => return Err(format!("malformed option `{key}`")),
    };
    match (key.as_str(), value) {
        ("deny_unknown_fields", None) => options.deny_unknown_fields = true,
        ("default", None) => {
            options.default_all = true;
            options.default_field = Some(Default_::Trait);
        }
        ("default", Some(path)) => options.default_field = Some(Default_::With(path)),
        ("rename_all", Some(v)) => options.rename_all = Some(v),
        ("tag", Some(v)) => options.tag = Some(v),
        ("skip_if", Some(v)) => options.skip_if = Some(v),
        ("rename", Some(v)) => options.rename = Some(v),
        (other, _) => return Err(format!("unknown json option `{other}`")),
    }
    Ok(())
}

/// Splits at commas outside angle brackets (groups are single tokens already).
fn split_commas(tokens: &[TokenTree]) -> Vec<Vec<TokenTree>> {
    let mut parts = Vec::new();
    let mut current: Vec<TokenTree> = Vec::new();
    let mut angle = 0i32;
    let mut previous_dash = false;
    for t in tokens {
        let mut dash = false;
        if let TokenTree::Punct(p) = t {
            match p.as_char() {
                '<' => angle += 1,
                '>' if !previous_dash => angle -= 1,
                ',' if angle == 0 => {
                    parts.push(std::mem::take(&mut current));
                    previous_dash = false;
                    continue;
                }
                '-' => dash = true,
                _ => {}
            }
        }
        previous_dash = dash;
        current.push(t.clone());
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn parse_fields(stream: TokenStream) -> Result<Vec<Field>, String> {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut fields = Vec::new();
    for chunk in split_commas(&tokens) {
        let mut i = 0;
        let options = take_attributes(&chunk, &mut i)?;
        skip_visibility(&chunk, &mut i);
        let name = match chunk.get(i) {
            Some(TokenTree::Ident(id)) => id.to_string(),
            _ => return Err("expected a field name".into()),
        };
        if !matches!(chunk.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == ':') {
            return Err("only named fields are supported by the JSON derive".into());
        }
        fields.push(Field { name, options });
    }
    Ok(fields)
}

fn parse_variants(stream: TokenStream) -> Result<Vec<Variant>, String> {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut variants = Vec::new();
    for chunk in split_commas(&tokens) {
        let mut i = 0;
        let options = take_attributes(&chunk, &mut i)?;
        let name = match chunk.get(i) {
            Some(TokenTree::Ident(id)) => id.to_string(),
            _ => return Err("expected a variant name".into()),
        };
        let fields = match chunk.get(i + 1) {
            None => None,
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
                Some(parse_fields(g.stream())?)
            }
            Some(TokenTree::Punct(p)) if p.as_char() == '=' => None,
            Some(_) => return Err("tuple variants are not supported by the JSON derive".into()),
        };
        variants.push(Variant {
            name,
            options,
            fields,
        });
    }
    Ok(variants)
}

// ---- naming --------------------------------------------------------------------------------------

fn words_of_pascal(name: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for c in name.chars() {
        if c.is_uppercase() || words.is_empty() {
            words.push(c.to_string());
        } else {
            words.last_mut().unwrap().push(c);
        }
    }
    words
}

fn restyle(words: &[String], rule: &str) -> Result<String, String> {
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    Ok(match rule {
        "SCREAMING_SNAKE_CASE" => lower.join("_").to_uppercase(),
        "snake_case" => lower.join("_"),
        "lowercase" => lower.concat(),
        "kebab-case" => lower.join("-"),
        "camelCase" => {
            let mut out = lower[0].clone();
            for w in &lower[1..] {
                let mut cs = w.chars();
                if let Some(first) = cs.next() {
                    out.extend(first.to_uppercase());
                    out.push_str(cs.as_str());
                }
            }
            out
        }
        other => return Err(format!("unsupported rename_all rule `{other}`")),
    })
}

fn variant_wire(v: &Variant, item: &Item) -> Result<String, String> {
    if let Some(r) = &v.options.rename {
        return Ok(r.clone());
    }
    match &item.options.rename_all {
        Some(rule) => restyle(&words_of_pascal(&v.name), rule),
        None => Ok(v.name.clone()),
    }
}

fn field_wire(f: &Field, rename_all: Option<&str>) -> Result<String, String> {
    if let Some(r) = &f.options.rename {
        return Ok(r.clone());
    }
    match rename_all {
        Some(rule) => restyle(
            &f.name
                .split('_')
                .filter(|w| !w.is_empty())
                .map(String::from)
                .collect::<Vec<_>>(),
            rule,
        ),
        None => Ok(f.name.clone()),
    }
}

// ---- generation ----------------------------------------------------------------------------------

const J: &str = "::ecdev_core::json";

fn lit(s: &str) -> String {
    format!("{s:?}")
}

fn gen_to(item: &Item) -> Result<String, String> {
    let name = &item.name;
    let body = match &item.body {
        Body::Struct(fields) => {
            let mut code = format!("let mut __m = {J}::Map::new();\n");
            for f in fields {
                let wire = field_wire(f, item.options.rename_all.as_deref())?;
                let insert = format!(
                    "__m.insert({}.to_string(), {J}::ToJson::to_json(&self.{}));",
                    lit(&wire),
                    f.name
                );
                match &f.options.skip_if {
                    Some(path) => {
                        code.push_str(&format!("if !({path})(&self.{}) {{ {insert} }}\n", f.name))
                    }
                    None => code.push_str(&format!("{insert}\n")),
                }
            }
            code.push_str(&format!("{J}::Value::Object(__m)"));
            code
        }
        Body::Enum(variants) => {
            let mut arms = String::new();
            for v in variants {
                let wire = variant_wire(v, item)?;
                match (&item.options.tag, &v.fields) {
                    (None, None) => arms.push_str(&format!(
                        "Self::{} => {J}::Value::String({}.to_string()),\n",
                        v.name,
                        lit(&wire)
                    )),
                    (None, Some(_)) => {
                        return Err("a variant with fields needs `tag` on the enum".into());
                    }
                    (Some(tag), fields) => {
                        let fields = fields.as_deref().unwrap_or(&[]);
                        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
                        let pattern = if fields.is_empty() && v.fields.is_none() {
                            format!("Self::{}", v.name)
                        } else {
                            format!("Self::{} {{ {} }}", v.name, names.join(", "))
                        };
                        let mut arm = format!(
                            "{pattern} => {{ let mut __m = {J}::Map::new(); __m.insert({}.to_string(), {J}::Value::String({}.to_string()));\n",
                            lit(tag),
                            lit(&wire)
                        );
                        for f in fields {
                            let w = field_wire(f, None)?;
                            let insert = format!(
                                "__m.insert({}.to_string(), {J}::ToJson::to_json({}));",
                                lit(&w),
                                f.name
                            );
                            match &f.options.skip_if {
                                Some(path) => arm.push_str(&format!(
                                    "if !({path})({}) {{ {insert} }}\n",
                                    f.name
                                )),
                                None => arm.push_str(&format!("{insert}\n")),
                            }
                        }
                        arm.push_str(&format!("{J}::Value::Object(__m) }}\n"));
                        arms.push_str(&arm);
                    }
                }
            }
            format!("match self {{\n{arms}}}")
        }
    };
    Ok(format!(
        "impl {J}::ToJson for {name} {{ fn to_json(&self) -> {J}::Value {{\n{body}\n}} }}"
    ))
}

fn field_init(f: &Field, item_default: bool, rename_all: Option<&str>) -> Result<String, String> {
    let wire = field_wire(f, rename_all)?;
    let w = lit(&wire);
    let init = match &f.options.default_field {
        Some(Default_::With(path)) => {
            format!("{J}::derive_support::field_or_else(__obj, {w}, {path})?")
        }
        Some(Default_::Trait) => format!(
            "{J}::derive_support::field_or_else(__obj, {w}, ::std::default::Default::default)?"
        ),
        None if item_default => {
            format!(
                "match __obj.get({w}) {{ Some(__v) => {J}::FromJson::from_json(__v)?, None => __base.{} }}",
                f.name
            )
        }
        None => format!("{J}::derive_support::field(__obj, {w})?"),
    };
    Ok(format!("{}: {init},", f.name))
}

fn gen_from(item: &Item) -> Result<String, String> {
    let name = &item.name;
    let body = match &item.body {
        Body::Struct(fields) => {
            let rename_all = item.options.rename_all.as_deref();
            let mut code = format!(
                "let __obj = {J}::derive_support::object(__v, {})?;\n",
                lit(&format!("struct {name}"))
            );
            if item.options.deny_unknown_fields {
                let wires: Result<Vec<String>, String> = fields
                    .iter()
                    .map(|f| field_wire(f, rename_all).map(|w| lit(&w)))
                    .collect();
                code.push_str(&format!(
                    "{J}::derive_support::deny_unknown(__obj, &[{}])?;\n",
                    wires?.join(", ")
                ));
            }
            if item.options.default_all {
                code.push_str("let __base: Self = ::std::default::Default::default();\n");
            }
            code.push_str(&format!("::std::result::Result::Ok({name} {{\n"));
            for f in fields {
                code.push_str(&field_init(f, item.options.default_all, rename_all)?);
                code.push('\n');
            }
            code.push_str("})");
            code
        }
        Body::Enum(variants) => {
            let wires: Result<Vec<String>, String> =
                variants.iter().map(|v| variant_wire(v, item)).collect();
            let wires = wires?;
            let list = wires.iter().map(|w| lit(w)).collect::<Vec<_>>().join(", ");
            match &item.options.tag {
                None => {
                    let mut arms = String::new();
                    for (v, wire) in variants.iter().zip(&wires) {
                        if v.fields.is_some() {
                            return Err("a variant with fields needs `tag` on the enum".into());
                        }
                        arms.push_str(&format!(
                            "{} => ::std::result::Result::Ok(Self::{}),\n",
                            lit(wire),
                            v.name
                        ));
                    }
                    format!(
                        "match {J}::derive_support::string_variant(__v)? {{\n{arms}__other => ::std::result::Result::Err({J}::derive_support::unknown_variant(__other, &[{list}])),\n}}"
                    )
                }
                Some(tag) => {
                    let mut arms = String::new();
                    for (v, wire) in variants.iter().zip(&wires) {
                        let fields = v.fields.as_deref().unwrap_or(&[]);
                        let mut arm = format!("{} => {{\n", lit(wire));
                        if item.options.deny_unknown_fields {
                            let mut allowed = vec![lit(tag)];
                            for f in fields {
                                allowed.push(lit(&field_wire(f, None)?));
                            }
                            arm.push_str(&format!(
                                "{J}::derive_support::deny_unknown(__obj, &[{}])?;\n",
                                allowed.join(", ")
                            ));
                        }
                        if v.fields.is_none() {
                            arm.push_str(&format!("::std::result::Result::Ok(Self::{})\n", v.name));
                        } else {
                            arm.push_str(&format!(
                                "::std::result::Result::Ok(Self::{} {{\n",
                                v.name
                            ));
                            for f in fields {
                                arm.push_str(&field_init(f, false, None)?);
                                arm.push('\n');
                            }
                            arm.push_str("})\n");
                        }
                        arm.push_str("}\n");
                        arms.push_str(&arm);
                    }
                    format!(
                        "let __obj = {J}::derive_support::object(__v, {})?;\nmatch {J}::derive_support::tag(__obj, {})? {{\n{arms}__other => ::std::result::Result::Err({J}::derive_support::unknown_variant(__other, &[{list}])),\n}}",
                        lit(&format!("internally tagged enum {name}")),
                        lit(tag)
                    )
                }
            }
        }
    };
    Ok(format!(
        "impl {J}::FromJson for {name} {{ fn from_json(__v: &{J}::Value) -> {J}::Result<Self> {{\n{body}\n}} }}"
    ))
}
