use heck::ToKebabCase;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use std::collections::HashMap;
use syn::visit_mut::{self, VisitMut};
use syn::{
    Attribute, Data, DeriveInput, Expr, Fields, GenericArgument, Lifetime, Meta, PathArguments,
    Token, Type, TypePath, TypeTuple, parse_macro_input, punctuated::Punctuated,
};

// ── Type analysis ─────────────────────────────────────────────────────────

fn is_ident_type(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(TypePath { qself: None, path }) if path.is_ident(name))
}

fn is_str_slice(ty: &Type) -> bool {
    matches!(ty, Type::Reference(r) if is_ident_type(&r.elem, "str"))
}

fn is_bool(ty: &Type) -> bool {
    is_ident_type(ty, "bool")
}

fn unwrap_option(ty: &Type) -> Option<&Type> {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return None;
    };
    let last = path.segments.last()?;
    if last.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    if args.args.len() != 1 {
        return None;
    }
    let GenericArgument::Type(inner) = args.args.first()? else {
        return None;
    };
    Some(inner)
}

fn unwrap_tuple(ty: &Type) -> Option<&Punctuated<Type, Token![,]>> {
    let Type::Tuple(TypeTuple { elems, .. }) = ty else {
        return None;
    };
    (!elems.is_empty()).then_some(elems)
}

// ── Lifetime rewriter ─────────────────────────────────────────────────────

struct LifetimeRewriter {
    target: Lifetime,
}

impl VisitMut for LifetimeRewriter {
    fn visit_lifetime_mut(&mut self, lt: &mut Lifetime) {
        if lt.ident != "static" {
            *lt = self.target.clone();
        }
        visit_mut::visit_lifetime_mut(self, lt);
    }
}

fn rewrite_lifetimes(ty: &Type, target: &str) -> Type {
    let mut ty = ty.clone();
    LifetimeRewriter {
        target: Lifetime::new(target, Span::call_site()),
    }
    .visit_type_mut(&mut ty);
    ty
}

// ── Expression group unwrapping ───────────────────────────────────────────

fn unwrap_groups(mut e: &Expr) -> &Expr {
    loop {
        e = match e {
            Expr::Group(g) => &g.expr,
            Expr::Paren(p) => &p.expr,
            _ => return e,
        };
    }
}

fn string_lit(expr: &Expr) -> Option<String> {
    if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(s),
        ..
    }) = unwrap_groups(expr)
    {
        Some(s.value())
    } else {
        None
    }
}

// ── #[set(...)] attribute parsing ─────────────────────────────────────────

#[derive(Default)]
struct SetAttrs {
    short: Option<String>,
    default: Option<Expr>,
    min: Option<Expr>,
    max: Option<Expr>,
    one_of: Option<(Expr, Vec<String>)>,
}

impl SetAttrs {
    /// Resolve the short flag, expanding the empty-string sentinel to "-{first char}".
    fn resolved_short(&self, field_name: &str) -> Option<String> {
        match self.short.as_deref() {
            Some("") => field_name.chars().next().map(|c| format!("-{c}")),
            Some(s) => Some(s.to_string()),
            None => None,
        }
    }
}

fn parse_set_attrs(attrs: &[Attribute]) -> Result<SetAttrs, syn::Error> {
    let mut result = SetAttrs::default();

    for attr in attrs {
        if !attr.path().is_ident("set") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("short") {
                result.short = Some(if meta.input.peek(Token![=]) {
                    let lit: syn::LitChar = meta.value()?.parse()?;
                    format!("-{}", lit.value())
                } else {
                    String::new()
                });
                return Ok(());
            }
            if meta.path.is_ident("default") {
                result.default = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("min") {
                result.min = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("max") {
                result.max = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("one_of") {
                let expr: Expr = meta.value()?.parse()?;
                let elem_strs = match unwrap_groups(&expr) {
                    Expr::Array(arr) => arr
                        .elems
                        .iter()
                        .map(|e| match unwrap_groups(e) {
                            Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Str(s),
                                ..
                            }) => s.value(),
                            Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Char(c),
                                ..
                            }) => c.value().to_string(),
                            other => quote!(#other).to_string(),
                        })
                        .collect::<Vec<_>>(),
                    _ => {
                        return Err(meta.error(
                            "expected an array expression like `one_of = [a, b, c]`",
                        ));
                    }
                };
                result.one_of = Some((expr, elem_strs));
                return Ok(());
            }

            let key = meta
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            Err(meta.error(format!(
                "unknown attribute key `{}`; expected `short`, `default`, `min`, `max`, or `one_of`",
                key
            )))
        })?;
    }

    Ok(result)
}

fn array_is_all_str_literals(expr: &Expr) -> bool {
    if let Expr::Array(arr) = unwrap_groups(expr) {
        arr.elems.iter().all(|e| {
            matches!(
                unwrap_groups(e),
                Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(_),
                    ..
                })
            )
        })
    } else {
        false
    }
}

/// Emit a `match FromStr::from_str(__default_str) { ... }` block, optionally
/// wrapping the success value in `Some(...)`.
fn from_str_default_block(
    parse_ty: &Type,
    s_value: &str,
    type_str: &str,
    wrap_some: bool,
) -> proc_macro2::TokenStream {
    let parse_ty_par = rewrite_lifetimes(parse_ty, "'par");
    let ok_arm = if wrap_some {
        quote! { ::core::option::Option::Some(parsed) }
    } else {
        quote! { parsed }
    };
    quote! {
        {
            let __default_str: &str = #s_value;
            match <#parse_ty_par as ::core::str::FromStr>::from_str(__default_str) {
                Ok(parsed) => #ok_arm,
                Err(_) => return Err(
                    ::bareminal_cli::process::ProcessError::InvalidValue((
                        "default",
                        #type_str,
                        __default_str,
                    ))
                ),
            }
        }
    }
}

fn default_value_expr(expr: &Expr, target_ty: &Type) -> proc_macro2::TokenStream {
    let target_ty_par = rewrite_lifetimes(target_ty, "'par");

    let Some(s_value) = string_lit(expr) else {
        // Non-string-literal default: splice the typed expression directly.
        return quote! {
            {
                let __v: #target_ty_par = #expr;
                __v
            }
        };
    };

    let inner = match (is_str_slice(target_ty), unwrap_option(target_ty)) {
        // Case 1: target is &str — splice the literal directly.
        (true, _) => quote! { #s_value },

        // Case 2a: Option<&str> — wrap the literal in Some.
        (_, Some(inner)) if is_str_slice(inner) => {
            quote! { ::core::option::Option::Some(#s_value) }
        }

        // Case 2b: Option<T> for other T — parse via FromStr, wrap in Some.
        (_, Some(inner)) => {
            let type_str = quote!(#inner).to_string();
            from_str_default_block(inner, &s_value, &type_str, true)
        }

        // Case 3: any other type — parse via FromStr.
        _ => {
            let type_str = quote!(#target_ty).to_string();
            from_str_default_block(target_ty, &s_value, &type_str, false)
        }
    };

    quote! {
        {
            let __v: #target_ty_par = #inner;
            __v
        }
    }
}

fn range_check(
    var: &syn::Ident,
    min: Option<&Expr>,
    max: Option<&Expr>,
    target_ty: &Type,
    context_str: &str,
) -> proc_macro2::TokenStream {
    if min.is_none() && max.is_none() {
        return quote! {};
    }

    let target_ty_par = rewrite_lifetimes(target_ty, "'par");
    let type_str = quote!(#target_ty).to_string();

    let min_str = min.map(pretty_expr).unwrap_or_else(|| "none".to_string());
    let max_str = max.map(pretty_expr).unwrap_or_else(|| "none".to_string());

    let min_check = min.map(|min_expr| {
        quote! {
            {
                let __min: #target_ty_par = #min_expr;
                if #var < __min {
                    return Err(::bareminal_cli::process::ProcessError::OutOfRange((
                        #context_str,
                        #type_str,
                        #min_str,
                        #max_str,
                    )));
                }
            }
        }
    });

    let max_check = max.map(|max_expr| {
        quote! {
            {
                let __max: #target_ty_par = #max_expr;
                if #var > __max {
                    return Err(::bareminal_cli::process::ProcessError::OutOfRange((
                        #context_str,
                        #type_str,
                        #min_str,
                        #max_str,
                    )));
                }
            }
        }
    });

    quote! {
        #min_check
        #max_check
    }
}

fn one_of_checks(
    parsed_var: &syn::Ident,
    one_of: Option<&(Expr, Vec<String>)>,
    target_ty: &Type,
    context_str: &str,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let Some((array_expr, elem_strs)) = one_of else {
        return (quote! {}, quote! {});
    };

    let array_expr = unwrap_groups(array_expr);
    let type_str = quote!(#target_ty).to_string();

    if array_is_all_str_literals(array_expr) {
        let pre = quote! {
            {
                let __allowed: &[&str] = &#array_expr;
                if !__allowed.iter().any(|__a| *__a == value) {
                    static __ONE_OF_STRS: &[&str] = &[#(#elem_strs),*];
                    return Err(::bareminal_cli::process::ProcessError::NotInSet((
                        #context_str,
                        #type_str,
                        __ONE_OF_STRS,
                    )));
                }
            }
        };
        (pre, quote! {})
    } else {
        let target_ty_par = rewrite_lifetimes(target_ty, "'par");
        let post = quote! {
            {
                let __allowed: &[#target_ty_par] = &#array_expr;
                if !__allowed.iter().any(|__a| __a == &#parsed_var) {
                    static __ONE_OF_STRS: &[&str] = &[#(#elem_strs),*];
                    return Err(::bareminal_cli::process::ProcessError::NotInSet((
                        #context_str,
                        #type_str,
                        __ONE_OF_STRS,
                    )));
                }
            }
        };
        (quote! {}, post)
    }
}

// ── Doc comment extraction ────────────────────────────────────────────────

fn unwrap_some_for_display(expr: &Expr) -> &Expr {
    let inner = unwrap_groups(expr);
    if let Expr::Call(call) = inner
        && let Expr::Path(path) = &*call.func
        && path.path.is_ident("Some")
        && call.args.len() == 1
    {
        return &call.args[0];
    }
    inner
}

fn extract_doc(attrs: &[Attribute]) -> Vec<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(nv) = &attr.meta
            && let Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
        {
            let raw = s.value();
            let trimmed = raw.strip_prefix(' ').unwrap_or(&raw).to_string();
            lines.push(trimmed);
        }
    }
    lines
}

fn clean_ts(s: String) -> String {
    s.replace(" :: ", "::")
        .replace(" < ", "<")
        .replace(" > ", ">")
}

fn pretty_expr(expr: &Expr) -> String {
    // Special-case unary negation of a literal: render as "-N" with no space.
    if let Expr::Unary(syn::ExprUnary {
        op: syn::UnOp::Neg(_),
        expr: inner,
        ..
    }) = expr
        && let Expr::Lit(_) = &**inner
    {
        return format!("-{}", quote!(#inner));
    }

    clean_ts(quote!(#expr).to_string())
}

fn pretty_type(ty: &Type) -> String {
    clean_ts(quote!(#ty).to_string())
}

fn pretty_type_bare(ty: &Type) -> String {
    let full = pretty_type(ty);
    match full.find('<') {
        Some(i) => full[..i].trim_end().to_string(),
        None => full,
    }
}

fn render_attr_summary(set: &SetAttrs, indent: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(d) = &set.default {
        let inner = unwrap_some_for_display(d);
        let display = match string_lit(inner) {
            Some(s) => s,
            None => pretty_expr(unwrap_groups(inner)),
        };
        lines.push(format!("{}[default: {}]", indent, display));
    }
    if let Some(mn) = &set.min {
        lines.push(format!("{}[min: {}]", indent, pretty_expr(mn)));
    }
    if let Some(mx) = &set.max {
        lines.push(format!("{}[max: {}]", indent, pretty_expr(mx)));
    }
    if let Some((_, elem_strs)) = &set.one_of {
        lines.push(format!("{}[one of: {}]", indent, elem_strs.join(", ")));
    }
    lines
}

// ── Help-line builders ────────────────────────────────────────────────────

const FLAG_COLUMN: usize = 22;
const FLAG_INDENT: &str = "                      ";

fn build_field_help_lines(field: &syn::Field) -> Vec<String> {
    let mut lines = Vec::new();

    let field_ident = field.ident.as_ref().unwrap();
    let field_name = field_ident.to_string();
    let long_flag = format!("--{}", field_name.to_kebab_case());

    let set = parse_set_attrs(&field.attrs).ok().unwrap_or_default();

    let optional = unwrap_option(&field.ty).is_some();

    let long_part = if optional {
        format!("[{}]", long_flag)
    } else {
        long_flag.clone()
    };

    let short_part = match set.resolved_short(&field_name) {
        Some(s) if optional => format!(", [{}]", s),
        Some(s) => format!(", {}", s),
        None => String::new(),
    };

    let flag_label = format!("{}{}", long_part, short_part);
    let label_padding = if flag_label.len() < FLAG_COLUMN - 2 {
        FLAG_COLUMN - 2 - flag_label.len()
    } else {
        2
    };

    let doc = extract_doc(&field.attrs);
    let first_doc = doc.first().cloned().unwrap_or_default();

    lines.push(format!(
        "  {}{}{}",
        flag_label,
        " ".repeat(label_padding),
        first_doc
    ));

    for line in doc.iter().skip(1) {
        lines.push(format!("{}{}", FLAG_INDENT, line));
    }

    if optional && set.default.is_none() {
        lines.push(format!("{}[optional]", FLAG_INDENT));
    }

    for line in render_attr_summary(&set, FLAG_INDENT) {
        lines.push(line);
    }

    lines
}

fn usage_placeholder(ty: &Type) -> String {
    if let Some(inner) = unwrap_option(ty) {
        return format!("[{}]", usage_placeholder(inner));
    }
    if let Some(elems) = unwrap_tuple(ty) {
        return elems
            .iter()
            .map(usage_placeholder)
            .collect::<Vec<_>>()
            .join(" ");
    }
    if is_str_slice(ty) {
        return "<string>".to_string();
    }
    format!("<{}>", pretty_type_bare(ty))
}

fn build_command_help_lines(v: &syn::Variant, variant_str: &str) -> Vec<String> {
    let mut lines = Vec::new();

    // Section header.
    lines.push(format!("== {} ==", variant_str));

    // Doc comment lines.
    let doc = extract_doc(&v.attrs);
    for line in &doc {
        lines.push(line.clone());
    }

    let set = parse_set_attrs(&v.attrs).ok().unwrap_or_default();

    // Usage line.
    match &v.fields {
        Fields::Unit => {
            lines.push(variant_str.to_string());
        }
        Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
            let ty = &unnamed.unnamed[0].ty;
            let usage = usage_placeholder(ty);
            lines.push(format!("{} {}", variant_str, usage));

            let optional = unwrap_option(ty).is_some();
            if optional && set.default.is_none() {
                lines.push("  [optional]".to_string());
            }
            for line in render_attr_summary(&set, "  ") {
                lines.push(line);
            }
        }
        Fields::Named(named) => {
            let mut usage_parts = Vec::new();
            for field in &named.named {
                let field_ident = field.ident.as_ref().unwrap();
                let long = format!("--{}", field_ident.to_string().to_kebab_case());
                let optional = unwrap_option(&field.ty).is_some();
                let inner_ty = unwrap_option(&field.ty).unwrap_or(&field.ty);
                let placeholder = if is_bool(inner_ty) {
                    String::new()
                } else {
                    format!(" {}", usage_placeholder(inner_ty))
                };
                let part = if optional {
                    format!("[{}{}]", long, placeholder)
                } else {
                    format!("{}{}", long, placeholder)
                };
                usage_parts.push(part);
            }
            if usage_parts.is_empty() {
                lines.push(variant_str.to_string());
            } else {
                lines.push(format!("{} {}", variant_str, usage_parts.join(" ")));
            }

            for line in render_attr_summary(&set, "  ") {
                lines.push(line);
            }
            if !named.named.is_empty() {
                lines.push(String::new());
                lines.push("Flags:".to_string());
                for field in &named.named {
                    for line in build_field_help_lines(field) {
                        lines.push(line);
                    }
                }
            }
        }
        _ => {}
    }

    lines
}

fn build_top_level_help_lines(
    general_doc: &[String],
    variants: &[(String, Vec<String>)],
) -> Vec<String> {
    let mut lines: Vec<String> = general_doc.to_vec();
    if !general_doc.is_empty() {
        lines.push(String::new());
    }
    lines.push("Commands:".to_string());

    let max_name = variants.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    // Invariant: each name.len() <= max_name, so col - name.len() >= 4.
    let col = max_name + 4;

    for (name, doc) in variants {
        let summary = doc.first().cloned().unwrap_or_default();
        lines.push(format!(
            "  {}{}{}",
            name,
            " ".repeat(col - name.len()),
            summary
        ));
    }

    lines
}

// ── Parse-expression generators ───────────────────────────────────────────

fn parse_element_value_expr(ty: &Type, context_str: &str) -> proc_macro2::TokenStream {
    if is_str_slice(ty) {
        quote! { Ok::<_, ::bareminal_cli::process::ProcessError<'_>>(value) }
    } else {
        let type_str = quote!(#ty).to_string();
        quote! {
            match <#ty as ::core::str::FromStr>::from_str(value) {
                Ok(parsed) => Ok(parsed),
                Err(_) => Err(::bareminal_cli::process::ProcessError::InvalidValue((
                    #context_str,
                    #type_str,
                    value,
                ))),
            }
        }
    }
}

fn parse_tuple_expr(
    elems: &Punctuated<Type, Token![,]>,
    context_str: &str,
) -> proc_macro2::TokenStream {
    let element_blocks: Vec<_> = elems
        .iter()
        .enumerate()
        .map(|(i, elem_ty)| {
            let elem_context = format!("{}.{}", context_str, i);
            let var = quote::format_ident!("__elem_{}", i);
            if let Some(inner) = unwrap_option(elem_ty) {
                let inner_parse = parse_element_value_expr(inner, &elem_context);
                let binding_ty = rewrite_lifetimes(elem_ty, "'par");
                quote! {
                    let #var: #binding_ty = match tokens.next() {
                        None => None,
                        Some(value) => match #inner_parse {
                            Ok(parsed) => Some(parsed),
                            Err(e) => return Err(e),
                        },
                    };
                }
            } else {
                let parse = parse_element_value_expr(elem_ty, &elem_context);
                quote! {
                    let value = tokens.next()
                        .ok_or(::bareminal_cli::process::ProcessError::Empty)?;
                    let #var = match #parse {
                        Ok(parsed) => parsed,
                        Err(e) => return Err(e),
                    };
                }
            }
        })
        .collect();

    let vars: Vec<_> = (0..elems.len())
        .map(|i| quote::format_ident!("__elem_{}", i))
        .collect();

    quote! {
        (|| -> Result<_, ::bareminal_cli::process::ProcessError<'par>> {
            #(#element_blocks)*
            Ok((#(#vars),*))
        })()
    }
}

fn parse_expr_with_check(
    ty: &Type,
    context_str: &str,
    pre_check: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if let Some(elems) = unwrap_tuple(ty) {
        parse_tuple_expr(elems, context_str)
    } else if is_str_slice(ty) {
        quote! {
            (|| -> Result<_, ::bareminal_cli::process::ProcessError<'par>> {
                let value = tokens.next()
                    .ok_or(::bareminal_cli::process::ProcessError::Empty)?;
                #pre_check
                Ok(value)
            })()
        }
    } else {
        let type_str = quote!(#ty).to_string();
        quote! {
            (|| -> Result<_, ::bareminal_cli::process::ProcessError<'par>> {
                let value = tokens.next()
                    .ok_or(::bareminal_cli::process::ProcessError::Empty)?;
                #pre_check
                match <#ty as ::core::str::FromStr>::from_str(value) {
                    Ok(parsed) => Ok(parsed),
                    Err(_) => Err(::bareminal_cli::process::ProcessError::InvalidValue((
                        #context_str,
                        #type_str,
                        value,
                    ))),
                }
            })()
        }
    }
}

// Emit the `let no_payload = ...;` block used to detect whether a
// single-field tuple variant has been given a payload token.
fn no_payload_check() -> proc_macro2::TokenStream {
    quote! {
        let no_payload = match tokens.peek() {
            None => true,
            Some(t) if t == "--" => { tokens.next(); true }
            Some(t) => {
                let b = t.as_bytes();
                // It's a flag if it starts with `-` followed by a non-digit, non-`.`.
                b.len() >= 2 && b[0] == b'-' && !matches!(b[1], b'0'..=b'9' | b'.')
            }
        };
    }
}

// ── Struct-variant arm ────────────────────────────────────────────────────

fn struct_variant_arm(
    variant_ident: &syn::Ident,
    variant_str: &str,
    fields: &syn::FieldsNamed,
) -> proc_macro2::TokenStream {
    let mut bindings = Vec::new();
    let mut flag_arms = Vec::new();
    let mut field_inits = Vec::new();
    let mut errors = Vec::new();
    let mut seen_short: HashMap<String, syn::Ident> = HashMap::new();

    for field in &fields.named {
        let field_ident = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        let long_flag = format!("--{}", field_ident.to_string().to_kebab_case());
        let context_str = format!("{}.{}", variant_str, field_ident);

        let set_attrs = match parse_set_attrs(&field.attrs) {
            Ok(s) => s,
            Err(e) => {
                errors.push(e);
                SetAttrs::default()
            }
        };

        let short_flag = set_attrs.resolved_short(&field_ident.to_string());

        let mut flag_patterns: Vec<proc_macro2::TokenStream> = vec![quote! { #long_flag }];
        if let Some(short) = &short_flag {
            if let Some(prev) = seen_short.get(short) {
                errors.push(syn::Error::new_spanned(
                    field,
                    format!("short flag `{}` is already used by field `{}`", short, prev),
                ));
            } else {
                seen_short.insert(short.clone(), field_ident.clone());
                flag_patterns.push(quote! { #short });
            }
        }
        let flag_pattern = quote! { #(#flag_patterns)|* };

        let default = set_attrs.default;
        let min = set_attrs.min;
        let max = set_attrs.max;
        let one_of = set_attrs.one_of;
        let binding_ty = rewrite_lifetimes(field_ty, "'par");

        if let Some(inner) = unwrap_option(field_ty) {
            bindings.push(quote! {
                let mut #field_ident: #binding_ty = None;
            });

            if is_bool(inner) {
                let parse = parse_element_value_expr(inner, &context_str);
                let range = range_check(
                    &quote::format_ident!("parsed"),
                    min.as_ref(),
                    max.as_ref(),
                    inner,
                    &context_str,
                );
                let (_pre, post_check) = one_of_checks(
                    &quote::format_ident!("parsed"),
                    one_of.as_ref(),
                    inner,
                    &context_str,
                );
                flag_arms.push(quote! {
                    #flag_pattern => {
                        match tokens.peek() {
                            None => { #field_ident = Some(true); }
                            Some(next) if next.starts_with('-') => {
                                #field_ident = Some(true);
                            }
                            Some(_) => {
                                let value = tokens.next().unwrap();
                                match #parse {
                                    Ok(parsed) => {
                                        #range
                                        #post_check
                                        #field_ident = Some(parsed);
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                    }
                });
            } else {
                let (pre_check, post_check) = one_of_checks(
                    &quote::format_ident!("parsed"),
                    one_of.as_ref(),
                    inner,
                    &context_str,
                );
                let parse = parse_expr_with_check(inner, &context_str, &pre_check);
                let range = range_check(
                    &quote::format_ident!("parsed"),
                    min.as_ref(),
                    max.as_ref(),
                    inner,
                    &context_str,
                );
                flag_arms.push(quote! {
                    #flag_pattern => {
                        match #parse {
                            Ok(parsed) => {
                                #range
                                #post_check
                                #field_ident = Some(parsed);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                });
            }

            let final_expr = if let Some(default_expr) = &default {
                let default_value = default_value_expr(default_expr, field_ty);
                quote! {
                    match #field_ident {
                        Some(_) => #field_ident,
                        None => #default_value,
                    }
                }
            } else {
                quote! { #field_ident }
            };
            field_inits.push(final_expr);
        } else {
            bindings.push(quote! {
                let mut #field_ident: ::core::option::Option<#binding_ty> = None;
            });

            let (pre_check, post_check) = one_of_checks(
                &quote::format_ident!("parsed"),
                one_of.as_ref(),
                field_ty,
                &context_str,
            );
            let parse = parse_expr_with_check(field_ty, &context_str, &pre_check);
            let range = range_check(
                &quote::format_ident!("parsed"),
                min.as_ref(),
                max.as_ref(),
                field_ty,
                &context_str,
            );
            flag_arms.push(quote! {
                #flag_pattern => {
                    match #parse {
                        Ok(parsed) => {
                            #range
                            #post_check
                            #field_ident = Some(parsed);
                        }
                        Err(e) => return Err(e),
                    }
                }
            });

            let final_expr = if let Some(default_expr) = &default {
                let default_value = default_value_expr(default_expr, field_ty);
                quote! {
                    match #field_ident {
                        Some(v) => v,
                        None => #default_value,
                    }
                }
            } else {
                quote! {
                    #field_ident.ok_or(
                        ::bareminal_cli::process::ProcessError::MissingFlag(#long_flag)
                    )?
                }
            };
            field_inits.push(final_expr);
        }
    }

    if !errors.is_empty() {
        let combined = errors
            .into_iter()
            .reduce(|mut a, b| {
                a.combine(b);
                a
            })
            .unwrap();
        return combined.to_compile_error();
    }

    let field_names: Vec<_> = fields
        .named
        .iter()
        .map(|f| f.ident.as_ref().unwrap())
        .collect();

    quote! {
        #variant_str => {
            #(#bindings)*
            while let Some(flag) = tokens.peek() {
                if !flag.starts_with('-') {
                    break;
                }
                if flag == "--" {
                    tokens.next();
                    break;
                }
                let flag = tokens.next().unwrap();
                match flag {
                    #(#flag_arms)*
                    _ => return Err(::bareminal_cli::process::ProcessError::UnknownFlag(flag)),
                }
            }
            Ok(Self::Match::#variant_ident {
                #(#field_names: #field_inits),*
            })
        },
    }
}

// ── Derive entry point ────────────────────────────────────────────────────

#[proc_macro_derive(Command, attributes(set))]
pub fn derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    let data_enum = match &ast.data {
        Data::Enum(data) => data,
        _ => {
            return syn::Error::new_spanned(&ast, "Command can only be derived for enums")
                .to_compile_error()
                .into();
        }
    };

    let arms = data_enum.variants.iter().map(|v| {
        let variant_ident = &v.ident;
        let variant_str = variant_ident.to_string().to_kebab_case();

        match &v.fields {
            Fields::Unit => quote! {
                #variant_str => Ok(Self::Match::#variant_ident),
            },
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let ty = &unnamed.unnamed[0].ty;
                let variant_set = match parse_set_attrs(&v.attrs) {
                    Ok(s) => s,
                    Err(e) => return e.to_compile_error(),
                };
                let variant_default = variant_set.default;
                let min = variant_set.min;
                let max = variant_set.max;
                let one_of = variant_set.one_of;
                let no_payload = no_payload_check();

                if let Some(inner) = unwrap_option(ty) {
                    let (pre_check, post_check) = one_of_checks(
                        &quote::format_ident!("parsed"),
                        one_of.as_ref(),
                        inner,
                        &variant_str,
                    );
                    let parse = parse_expr_with_check(inner, &variant_str, &pre_check);
                    let range = range_check(
                        &quote::format_ident!("parsed"),
                        min.as_ref(),
                        max.as_ref(),
                        inner,
                        &variant_str,
                    );
                    let empty_branch = if let Some(default_expr) = &variant_default {
                        let default_value = default_value_expr(default_expr, ty);
                        quote! { Ok(Self::Match::#variant_ident(#default_value)) }
                    } else {
                        quote! { Ok(Self::Match::#variant_ident(None)) }
                    };
                    quote! {
                        #variant_str => {
                            #no_payload
                            if no_payload {
                                #empty_branch
                            } else {
                                match #parse {
                                    Ok(parsed) => {
                                        #range
                                        #post_check
                                        Ok(Self::Match::#variant_ident(Some(parsed)))
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                        },
                    }
                } else {
                    let (pre_check, post_check) = one_of_checks(
                        &quote::format_ident!("parsed"),
                        one_of.as_ref(),
                        ty,
                        &variant_str,
                    );
                    let parse = parse_expr_with_check(ty, &variant_str, &pre_check);
                    let range = range_check(
                        &quote::format_ident!("parsed"),
                        min.as_ref(),
                        max.as_ref(),
                        ty,
                        &variant_str,
                    );
                    if let Some(default_expr) = &variant_default {
                        let default_value = default_value_expr(default_expr, ty);
                        quote! {
                            #variant_str => {
                                #no_payload
                                if no_payload {
                                    Ok(Self::Match::#variant_ident(#default_value))
                                } else {
                                    match #parse {
                                        Ok(parsed) => {
                                            #range
                                            #post_check
                                            Ok(Self::Match::#variant_ident(parsed))
                                        }
                                        Err(e) => Err(e),
                                    }
                                }
                            },
                        }
                    } else {
                        quote! {
                            #variant_str => {
                                match #parse {
                                    Ok(parsed) => {
                                        #range
                                        #post_check
                                        Ok(Self::Match::#variant_ident(parsed))
                                    }
                                    Err(e) => Err(e),
                                }
                            },
                        }
                    }
                }
            }
            Fields::Named(named) => struct_variant_arm(variant_ident, &variant_str, named),
            _ => syn::Error::new_spanned(
                v,
                "Command supports unit, single-field tuple, or struct variants",
            )
            .to_compile_error(),
        }
    });

    let name = &ast.ident;
    let has_lifetimes = ast.generics.lifetimes().next().is_some();

    let match_ty = if has_lifetimes {
        quote! { #name<'mch> }
    } else {
        quote! { #name }
    };

    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    // Build help data
    let general_doc = extract_doc(&ast.attrs);
    let variant_summaries: Vec<(String, Vec<String>)> = data_enum
        .variants
        .iter()
        .map(|v| (v.ident.to_string().to_kebab_case(), extract_doc(&v.attrs)))
        .collect();
    let top_level_lines = build_top_level_help_lines(&general_doc, &variant_summaries);

    let help_consts: Vec<proc_macro2::TokenStream> = data_enum
        .variants
        .iter()
        .map(|v| {
            let variant_str = v.ident.to_string().to_kebab_case();
            let lines = build_command_help_lines(v, &variant_str);
            let const_name = quote::format_ident!("__HELP_{}", v.ident.to_string().to_uppercase());
            quote! {
                const #const_name: &[&str] = &[#(#lines),*];
            }
        })
        .collect();

    let help_for_arms: Vec<proc_macro2::TokenStream> = data_enum
        .variants
        .iter()
        .map(|v| {
            let variant_str = v.ident.to_string().to_kebab_case();
            let const_name = quote::format_ident!("__HELP_{}", v.ident.to_string().to_uppercase());
            quote! { #variant_str => #const_name, }
        })
        .collect();

    let variant_names: Vec<String> = variant_summaries
        .iter()
        .map(|(name, _)| name.clone())
        .collect();

    let output = quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            pub const HELP_LINES: &'static [&'static str] = &[#(#top_level_lines),*];
        }

        impl #impl_generics ::bareminal_cli::process::CommandsParser for #name #ty_generics #where_clause {
            type Match<'mch> = #match_ty;

            fn parse<'par>(
                tokens: &mut ::bareminal_cli::tokens::TokensIter<'par>,
            ) -> Result<Self::Match<'par>, ::bareminal_cli::process::ProcessError<'par>> {
                while tokens.peek() == Some("--") {
                    tokens.next();
                }
                let command_name = tokens.next()
                    .ok_or(::bareminal_cli::process::ProcessError::Empty)?;
                match command_name {
                    #(#arms)*
                    _ => Err(::bareminal_cli::process::ProcessError::Unknown),
                }
            }

            fn help() -> &'static [&'static str] {
                Self::HELP_LINES
            }

            fn help_for(name: &str) -> &'static [&'static str] {
                #(#help_consts)*

                match name {
                    #(#help_for_arms)*
                    _ => &[],
                }
            }

            fn autocomplete(name: &str) -> ::core::option::Option<&'static str> {
                static __NAMES: &[&str] = &["help", #(#variant_names),*];

                // Two-pass: first find an exact match's position among prefix matches,
                // then return the candidate after it (or the first match if no exact match).

                let mut first_match: ::core::option::Option<&'static str> = None;
                let mut return_next = false;
                let mut i = 0;
                while i < __NAMES.len() {
                    let candidate = __NAMES[i];
                    if candidate.starts_with(name) {
                        if return_next {
                            return Some(candidate);
                        }
                        if first_match.is_none() {
                            first_match = Some(candidate);
                        }
                        if candidate == name {
                            return_next = true;
                        }
                    }
                    i += 1;
                }

                first_match
            }
        }

    };

    output.into()
}

// ── CommandGroup derive ───────────────────────────────────────────────────

#[proc_macro_derive(CommandGroup)]
pub fn derive_command_group(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    let data_enum = match &ast.data {
        Data::Enum(data) => data,
        _ => {
            return syn::Error::new_spanned(&ast, "CommandGroup can only be derived for enums")
                .to_compile_error()
                .into();
        }
    };

    let name = &ast.ident;

    let try_blocks = data_enum.variants.iter().map(|v| {
        let variant_ident = &v.ident;
        match &v.fields {
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let inner_ty = &unnamed.unnamed[0].ty;
                quote! {
                    {
                        let mut __attempt = tokens.clone();
                        match <#inner_ty as ::bareminal_cli::process::CommandsParser>::parse(&mut __attempt) {
                            Ok(parsed) => {
                                *tokens = __attempt;
                                return Ok(Self::Match::#variant_ident(parsed));
                            }
                            Err(::bareminal_cli::process::ProcessError::Unknown) => {}
                            Err(other) => {
                                if first_real_error.is_none() {
                                    first_real_error = Some(other);
                                }
                            }
                        }
                    }
                }
            }
            _ => syn::Error::new_spanned(
                v,
                "CommandGroup variants must be single-field tuple variants like `Variant(Inner)`",
            )
            .to_compile_error(),
        }
    });

    let help_for_delegates: Vec<proc_macro2::TokenStream> = data_enum
        .variants
        .iter()
        .filter_map(|v| match &v.fields {
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let inner_ty = &unnamed.unnamed[0].ty;
                Some(quote! {
                    let r = <#inner_ty as ::bareminal_cli::process::CommandsParser>::help_for(name);
                    if !r.is_empty() { return r; }
                })
            }
            _ => None,
        })
        .collect();

    // Build group sections: (header, member_HELP_LINES) pairs.
    let section_entries: Vec<proc_macro2::TokenStream> = data_enum
        .variants
        .iter()
        .filter_map(|v| match &v.fields {
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let inner_ty = &unnamed.unnamed[0].ty;
                let inner_ty_static = rewrite_lifetimes(inner_ty, "'static");
                let header = format!("== {} ==", pretty_type_bare(inner_ty));
                Some(quote! {
                    (#header, <#inner_ty_static>::HELP_LINES),
                })
            }
            _ => None,
        })
        .collect();

    let general_doc = extract_doc(&ast.attrs);

    let mut top_lines: Vec<String> = general_doc.to_vec();
    if !general_doc.is_empty() {
        top_lines.push(String::new());
    }
    top_lines.push("Command groups:".to_string());
    for v in &data_enum.variants {
        if let Fields::Unnamed(unnamed) = &v.fields
            && unnamed.unnamed.len() == 1
        {
            let variant_str = v.ident.to_string().to_kebab_case();
            let inner_ty = &unnamed.unnamed[0].ty;
            let ty_str = pretty_type_bare(inner_ty);
            top_lines.push(format!("  {} (group: {})", variant_str, ty_str));
        }
    }

    let has_lifetimes = ast.generics.lifetimes().next().is_some();

    let match_ty = if has_lifetimes {
        quote! { #name<'mch> }
    } else {
        quote! { #name }
    };

    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let autocomplete_delegates: Vec<proc_macro2::TokenStream> = data_enum
        .variants
        .iter()
        .filter_map(|v| match &v.fields {
            Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
                let inner_ty = &unnamed.unnamed[0].ty;
                Some(quote! {
                    if let Some(candidate) =
                        <#inner_ty as ::bareminal_cli::process::CommandsParser>::autocomplete(name)
                    {
                        return Some(candidate);
                    }
                })
            }
            _ => None,
        })
        .collect();

    let output = quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            pub const HELP_LINES: &'static [&'static str] = &[#(#top_lines),*];
        }

        impl #impl_generics ::bareminal_cli::process::CommandsParser for #name #ty_generics #where_clause {
            type Match<'mch> = #match_ty;

            fn parse<'par>(
                tokens: &mut ::bareminal_cli::tokens::TokensIter<'par>,
            ) -> Result<Self::Match<'par>, ::bareminal_cli::process::ProcessError<'par>> {
                let mut first_real_error: ::core::option::Option<::bareminal_cli::process::ProcessError<'par>> = None;

                #(#try_blocks)*

                Err(first_real_error.unwrap_or(::bareminal_cli::process::ProcessError::Unknown))
            }

            fn help() -> &'static [&'static str] {
                Self::HELP_LINES
            }

            fn help_for(name: &str) -> &'static [&'static str] {
                #(#help_for_delegates)*
                &[]
            }

            fn help_lines() -> ::bareminal_cli::process::HelpIter {
                static __SECTIONS: &[(&str, &[&str])] = &[
                    #(#section_entries)*
                ];
                ::bareminal_cli::process::HelpIter::multi(__SECTIONS)
            }

            fn autocomplete(name: &str) -> ::core::option::Option<&'static str> {
                #(#autocomplete_delegates)*
                None
            }
        }
    };

    output.into()
}
