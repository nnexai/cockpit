use std::collections::BTreeMap;

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
