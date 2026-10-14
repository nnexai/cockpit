use std::collections::{BTreeMap, BTreeSet};

use syn::{
    Attribute, Data, DeriveInput, Expr, Fields, Item, Token, Variant,
    parse::{Parse, ParseStream, Parser},
    punctuated::Punctuated,
};

use super::attrs::{derives, wire_ty};

pub(super) fn inputs(file: &syn::File) -> Result<BTreeMap<String, DeriveInput>, String> {
    let mut inputs = BTreeMap::new();
    for item in &file.items {
        let input = match item {
            Item::Struct(item) => Some(DeriveInput {
                attrs: item.attrs.clone(),
                vis: item.vis.clone(),
                ident: item.ident.clone(),
                generics: item.generics.clone(),
                data: Data::Struct(syn::DataStruct {
                    struct_token: item.struct_token,
                    fields: item.fields.clone(),
                    semi_token: item.semi_token,
                }),
            }),
            Item::Enum(item) => Some(DeriveInput {
                attrs: item.attrs.clone(),
                vis: item.vis.clone(),
                ident: item.ident.clone(),
                generics: item.generics.clone(),
                data: Data::Enum(syn::DataEnum {
                    enum_token: item.enum_token,
                    brace_token: item.brace_token,
                    variants: item.variants.clone(),
                }),
            }),
            _ => None,
        };
        if let Some(input) = input {
            let name = input.ident.to_string();
            if inputs.insert(name.clone(), input).is_some() {
                return Err(format!("{name}: duplicate source type"));
            }
        }
    }
    Ok(inputs)
}

// This is the sole recognized manual Default: literal launch configuration.
// Resolve its expressions from the source rather than duplicating launch defaults.
pub(super) fn orchestration_defaults(
    file: &syn::File,
) -> Result<serde_json::Value, String> {
    use serde_json::{Map, Value};

    let fail = || "OrchestrationConfiguration: unsupported grounded Default expression".to_owned();
    let inputs = inputs(file)?;
    let configuration = inputs.get("OrchestrationConfiguration").ok_or_else(fail)?;
    let Data::Struct(data) = &configuration.data else {
        return Err(fail());
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(fail());
    };
    let expected = fields.named.iter().map(|field| {
        field.ident.as_ref().map(ToString::to_string).ok_or_else(fail)
    }).collect::<Result<BTreeSet<_>, _>>()?;
    let container = super::container(configuration, "OrchestrationConfiguration")?;
    let serde_derive_internals::ast::Data::Struct(_, source_fields) = &container.data else {
        return Err(fail());
    };
    let mut implementations = file.items.iter().filter_map(|item| match item {
        Item::Impl(item)
            if matches!(item.self_ty.as_ref(), syn::Type::Path(path)
                if path.path.is_ident("OrchestrationConfiguration"))
                && item.trait_.as_ref().is_some_and(|(_, path, _)| path.is_ident("Default")) =>
        {
            Some(item)
        }
        _ => None,
    });
    let implementation = implementations.next().ok_or_else(fail)?;
    if implementations.next().is_some() {
        return Err(fail());
    }
    let [syn::ImplItem::Fn(function)] = implementation.items.as_slice() else {
        return Err(fail());
    };
    if function.sig.ident != "default" || !function.sig.inputs.is_empty() {
        return Err(fail());
    }
    let [syn::Stmt::Expr(Expr::Struct(default), None)] = function.block.stmts.as_slice() else {
        return Err(fail());
    };
    if !default.path.is_ident("Self") || default.rest.is_some() {
        return Err(fail());
    }
    let mut values = Map::new();
    for field in &default.fields {
        let syn::Member::Named(name) = &field.member else {
            return Err(fail());
        };
        let source_field = source_fields.iter().find(|field| field.member == syn::Member::Named(name.clone()))
            .ok_or_else(fail)?;
        if let serde_derive_internals::attr::Default::Path(path) = source_field.attrs.default() {
            let Expr::Call(call) = &field.expr else { return Err(fail()); };
            let Expr::Path(function) = call.func.as_ref() else { return Err(fail()); };
            let same_function = function.qself.is_none() && path.qself.is_none()
                && function.path.get_ident().zip(path.path.get_ident())
                    .is_some_and(|(called, configured)| called == configured);
            if !same_function || !call.args.is_empty() {
                return Err(fail());
            }
        }
        let value = literal_default_expr(file, &inputs, &field.expr, &mut BTreeSet::new())?;
        if values.insert(name.to_string(), value).is_some() {
            return Err(fail());
        }
    }
    if values.len() != expected.len() || expected.iter().any(|name| !values.contains_key(name)) {
        return Err(fail());
    }
    Ok(Value::Object(values))
}

fn literal_default_function(
    file: &syn::File,
    inputs: &BTreeMap<String, DeriveInput>,
    name: &str,
    visiting: &mut BTreeSet<String>,
) -> Result<serde_json::Value, String> {
    let fail = || format!("OrchestrationConfiguration: unsupported literal default function {name}");
    if !matches!(name, "default_harness" | "default_max_parallel") || !visiting.insert(name.to_owned()) {
        return Err(fail());
    }
    let mut functions = file.items.iter().filter_map(|item| match item {
        Item::Fn(item) if item.sig.ident == name => Some(item),
        _ => None,
    });
    let function = functions.next().ok_or_else(fail)?;
    if functions.next().is_some() || !function.sig.inputs.is_empty()
        || !function.sig.generics.params.is_empty() || function.sig.asyncness.is_some()
        || function.sig.unsafety.is_some() || function.sig.variadic.is_some()
    {
        return Err(fail());
    }
    let [syn::Stmt::Expr(expr, None)] = function.block.stmts.as_slice() else {
        return Err(fail());
    };
    let value = literal_default_expr(file, inputs, expr, visiting)?;
    let syn::ReturnType::Type(_, output) = &function.sig.output else {
        return Err(fail());
    };
    let ty = wire_ty(output, name)?;
    let supported = match (&ty, &value) {
        (super::WireTy::Str, serde_json::Value::String(_)) => true,
        (super::WireTy::Int { min, max }, serde_json::Value::Number(number)) => {
            number.as_i64().is_some_and(|value| value >= *min && value <= *max)
        }
        _ => false,
    };
    if !supported {
        return Err(fail());
    }
    visiting.remove(name);
    Ok(value)
}

fn literal_default_expr(
    file: &syn::File,
    inputs: &BTreeMap<String, DeriveInput>,
    expr: &Expr,
    visiting: &mut BTreeSet<String>,
) -> Result<serde_json::Value, String> {
    use serde_derive_internals::{ast, attr};
    use serde_json::Value;
    let fail = || "OrchestrationConfiguration: unsupported grounded Default expression".to_owned();
    match expr {
        Expr::Call(call) if call.args.is_empty() && expr_path(&call.func, &["Vec", "new"]) => {
            Ok(Value::Array(Vec::new()))
        }
        Expr::Call(call) if call.args.is_empty() => {
            let Expr::Path(path) = call.func.as_ref() else { return Err(fail()); };
            if path.qself.is_some() || path.path.leading_colon.is_some() {
                return Err(fail());
            }
            let name = path.path.get_ident().ok_or_else(fail)?.to_string();
            literal_default_function(file, inputs, &name, visiting)
        }
        Expr::Path(path) if path.path.is_ident("None") => Ok(Value::Null),
        Expr::Path(path) if path.qself.is_none() && path.path.leading_colon.is_none()
            && path.path.segments.len() == 2
            && path.path.segments.iter().all(|segment| segment.arguments.is_empty()) =>
        {
            let mut segments = path.path.segments.iter();
            let name = segments.next().ok_or_else(fail)?.ident.to_string();
            let variant = &segments.next().ok_or_else(fail)?.ident;
            let input = inputs.get(&name).ok_or_else(fail)?;
            let container = super::container(input, &name)?;
            super::validate_container(&container, &name)?;
            let ast::Data::Enum(variants) = &container.data else { return Err(fail()); };
            if !matches!(container.attrs.tag(), attr::TagType::External) {
                return Err(fail());
            }
            let variant = variants.iter().find(|item| item.ident == *variant).ok_or_else(fail)?;
            if !matches!(variant.style, ast::Style::Unit) {
                return Err(fail());
            }
            super::validate_variant(variant, &name)?;
            Ok(Value::String(variant.attrs.name().deserialize_name().to_owned()))
        }
        Expr::Lit(literal) => match &literal.lit {
            syn::Lit::Bool(value) => Ok(Value::Bool(value.value)),
            syn::Lit::Int(value) => value.base10_parse::<u64>().map(Value::from).map_err(|_| fail()),
            _ => Err(fail()),
        },
        Expr::MethodCall(convert) if convert.method == "into" && convert.args.is_empty()
            && convert.turbofish.is_none() =>
        {
            let Expr::Lit(literal) = convert.receiver.as_ref() else { return Err(fail()); };
            let syn::Lit::Str(value) = &literal.lit else { return Err(fail()); };
            Ok(Value::String(value.value()))
        }
        _ => Err(fail()),
    }
}

// Registered expansion only; never evaluate arbitrary macros or maintain their variants here.
pub(super) fn retirement(file: &syn::File) -> Result<DeriveInput, String> {
    let fail = || "retirement_states: unsupported macro shape".to_owned();
    let mut definitions = file.items.iter().filter_map(|item| match item {
        Item::Macro(item)
            if item.mac.path.is_ident("macro_rules")
                && item
                    .ident
                    .as_ref()
                    .is_some_and(|ident| ident == "retirement_states") =>
        {
            Some(item)
        }
        _ => None,
    });
    let definition = definitions.next().ok_or_else(fail)?;
    if definitions.next().is_some() {
        return Err(fail());
    }
    let rule: RetirementRule = syn::parse2(definition.mac.tokens.clone()).map_err(|_| fail())?;
    let mut invocations = file.items.iter().filter_map(|item| match item {
        Item::Macro(item) if item.mac.path.is_ident("retirement_states") => Some(item),
        _ => None,
    });
    let invocation = invocations.next().ok_or_else(fail)?;
    if invocations.next().is_some() {
        return Err(fail());
    }
    let variants = Punctuated::<Variant, Token![,]>::parse_terminated
        .parse2(invocation.mac.tokens.clone())
        .map_err(|_| fail())?;
    if variants.is_empty()
        || variants.iter().any(|variant| {
            !matches!(variant.fields, Fields::Named(_))
                || variant.discriminant.is_some()
                || !variant.attrs.is_empty()
        })
    {
        return Err(fail());
    }
    let mut input: DeriveInput =
        syn::parse_str("pub enum RetirementState {}").map_err(|_| fail())?;
    input.attrs = rule.attrs;
    let Data::Enum(data) = &mut input.data else {
        return Err(fail());
    };
    data.variants = variants;
    if !derives(&input.attrs, "Deserialize")? {
        return Err(fail());
    }
    Ok(input)
}

struct RetirementRule {
    attrs: Vec<Attribute>,
}

impl Parse for RetirementRule {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let matcher;
        syn::parenthesized!(matcher in input);
        if compact(&take_tokens(&matcher)?)
            != "$($variant:ident{$($field:ident:$field_type:ty),*$(,)?}),*$(,)?"
        {
            return Err(input.error("unsupported matcher"));
        }
        input.parse::<Token![=>]>()?;
        let body;
        syn::braced!(body in input);
        let attrs = body.call(Attribute::parse_outer)?;
        body.parse::<Token![pub]>()?;
        body.parse::<Token![enum]>()?;
        let ident: syn::Ident = body.parse()?;
        if ident != "RetirementState" {
            return Err(body.error("unsupported enum"));
        }
        let variants;
        syn::braced!(variants in body);
        if compact(&take_tokens(&variants)?) != "$($variant{$($field:$field_type),*}),*" {
            return Err(variants.error("unsupported transcriber"));
        }
        // The remaining transcriber defines Rust-only kind()/RetirementStateKind.
        take_tokens(&body)?;
        if input.peek(Token![;]) {
            input.parse::<Token![;]>()?;
        }
        if !input.is_empty() {
            return Err(input.error("expected exactly one rule"));
        }
        Ok(Self { attrs })
    }
}

fn compact(tokens: &str) -> String {
    tokens.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn take_tokens(input: ParseStream<'_>) -> syn::Result<String> {
    input.step(|cursor| {
        let text = cursor.token_stream().to_string();
        let mut end = *cursor;
        while let Some((_, next)) = end.token_tree() {
            end = next;
        }
        Ok((text, end))
    })
}

pub(super) fn terminal_alias(
    file: &syn::File,
    inputs: &BTreeMap<String, DeriveInput>,
) -> Result<DeriveInput, String> {
    let fail = || "TerminalCommand: unsupported manual Deserialize delegate".to_owned();
    let public = inputs.get("TerminalCommand").ok_or_else(fail)?;
    let target = inputs.get("TerminalCommandWire").ok_or_else(fail)?;
    if derives(&public.attrs, "Deserialize")?
        || derives(&public.attrs, "Serialize")?
        || !derives(&target.attrs, "Deserialize")?
    {
        return Err(fail());
    }
    let mut delegates = file.items.iter().filter_map(|item| match item {
        Item::Impl(item)
            if matches!(item.self_ty.as_ref(), syn::Type::Path(path)
            if path.path.is_ident("TerminalCommand"))
                && item.trait_.as_ref().is_some_and(|(_, path, _)| {
                    path.segments
                        .last()
                        .is_some_and(|part| part.ident == "Deserialize")
                }) =>
        {
            Some(item)
        }
        _ => None,
    });
    let delegate = delegates.next().ok_or_else(fail)?;
    if delegates.next().is_some() || delegate.items.len() != 1 {
        return Err(fail());
    }
    let syn::ImplItem::Fn(method) = &delegate.items[0] else {
        return Err(fail());
    };
    if method.sig.ident != "deserialize" || !delegate_body(&method.block) {
        return Err(fail());
    }
    let (Data::Enum(public_enum), Data::Enum(target_enum)) = (&public.data, &target.data) else {
        return Err(fail());
    };
    if public_enum.variants.len() != target_enum.variants.len() {
        return Err(fail());
    }
    let mut wire = target.clone();
    add_ts(&mut wire.attrs, &public.attrs);
    let Data::Enum(wire_enum) = &mut wire.data else {
        return Err(fail());
    };
    for (public, target) in public_enum.variants.iter().zip(&mut wire_enum.variants) {
        if public.ident != target.ident || public.fields.len() != target.fields.len() {
            return Err(fail());
        }
        add_ts(&mut target.attrs, &public.attrs);
        for (public_field, target_field) in public.fields.iter().zip(target.fields.iter_mut()) {
            if public_field.ident != target_field.ident
                || wire_ty(&public_field.ty, "TerminalCommand")?
                    != wire_ty(&target_field.ty, "TerminalCommandWire")?
            {
                return Err(fail());
            }
            add_ts(&mut target_field.attrs, &public_field.attrs);
        }
    }
    Ok(wire)
}

fn add_ts(target: &mut Vec<Attribute>, public: &[Attribute]) {
    target.extend(
        public
            .iter()
            .filter(|attr| attr.path().is_ident("ts"))
            .cloned(),
    );
}

fn expr_path(expr: &Expr, expected: &[&str]) -> bool {
    let Expr::Path(path) = expr else {
        return false;
    };
    path.qself.is_none()
        && path.path.segments.len() == expected.len()
        && path
            .path
            .segments
            .iter()
            .zip(expected)
            .all(|(segment, expected)| {
                segment.ident == *expected && matches!(segment.arguments, syn::PathArguments::None)
            })
}

fn call(expr: &Expr, path: &[&str], arg: &[&str]) -> bool {
    let Expr::Call(call) = expr else {
        return false;
    };
    expr_path(&call.func, path)
        && call.args.len() == 1
        && call.args.first().is_some_and(|expr| expr_path(expr, arg))
}

fn delegate_body(block: &syn::Block) -> bool {
    let [
        syn::Stmt::Local(local),
        syn::Stmt::Expr(Expr::MethodCall(result), None),
    ] = block.stmts.as_slice()
    else {
        return false;
    };
    let syn::Pat::Ident(binding) = &local.pat else {
        return false;
    };
    if binding.ident != "wire"
        || binding.by_ref.is_some()
        || binding.mutability.is_some()
        || binding.subpat.is_some()
    {
        return false;
    }
    let Some(init) = &local.init else {
        return false;
    };
    let Expr::Try(value) = init.expr.as_ref() else {
        return false;
    };
    init.diverge.is_none()
        && call(
            &value.expr,
            &["TerminalCommandWire", "deserialize"],
            &["deserializer"],
        )
        && result.method == "map_err"
        && result.turbofish.is_none()
        && result.args.len() == 1
        && result
            .args
            .first()
            .is_some_and(|expr| expr_path(expr, &["serde", "de", "Error", "custom"]))
        && call(&result.receiver, &["Self", "try_from"], &["wire"])
}
