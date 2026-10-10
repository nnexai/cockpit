use syn::{
    Attribute, Expr, GenericArgument, Meta, Path, PathArguments, Token, Type,
    punctuated::Punctuated,
};
use syn::{ext::IdentExt, parse::ParseStream};

use super::WireTy;

#[derive(Default)]
pub(super) struct TsAttrs {
    pub rename: Option<String>,
    pub tag: Option<String>,
    pub rename_all: Option<String>,
    pub rename_all_fields: Option<String>,
    pub optional: Option<bool>, // Some(true) preserves null, Some(false) strips it.
    pub optional_disabled: bool,
    pub type_override: Option<String>,
    pub type_as: Option<Type>,
}

pub(super) fn ts_attrs(attrs: &[Attribute], owner: &str) -> Result<TsAttrs, String> {
    let mut result = TsAttrs::default();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("ts")) {
        let entries = attr
            .parse_args_with(|input: ParseStream<'_>| {
                Punctuated::<Meta, Token![,]>::parse_terminated_with(input, parse_ts_meta)
            })
            .map_err(|error| format!("{owner}: invalid ts attribute: {error}"))?;
        for entry in entries {
            let key = entry
                .path()
                .get_ident()
                .map(ToString::to_string)
                .ok_or_else(|| format!("{owner}: unsupported ts attribute path"))?;
            match key.as_str() {
                "rename" => result.rename = Some(string_value(&entry, owner)?),
                "tag" => result.tag = Some(string_value(&entry, owner)?),
                "type" => result.type_override = Some(string_value(&entry, owner)?),
                "as" => {
                    result.type_as = Some(
                        syn::parse_str(&string_value(&entry, owner)?)
                            .map_err(|error| format!("{owner}: invalid ts(as): {error}"))?,
                    );
                }
                "optional" => match &entry {
                    Meta::Path(_) => result.optional = Some(false),
                    Meta::NameValue(value) => match &value.value {
                        Expr::Path(path) if path.path.is_ident("nullable") => {
                            result.optional = Some(true)
                        }
                        Expr::Lit(lit) if matches!(&lit.lit, syn::Lit::Bool(value) if !value.value) =>
                        {
                            result.optional_disabled = true;
                        }
                        _ => return Err(format!("{owner}: unsupported ts(optional)")),
                    },
                    _ => return Err(format!("{owner}: unsupported ts(optional)")),
                },
                "rename_all" => result.rename_all = Some(string_value(&entry, owner)?),
                "rename_all_fields" => {
                    result.rename_all_fields = Some(string_value(&entry, owner)?)
                }
                _ => return Err(format!("{owner}: unsupported ts attribute {key}")),
            }
        }
    }
    Ok(result)
}

fn parse_ts_meta(input: ParseStream<'_>) -> syn::Result<Meta> {
    let path = syn::Path::from(input.call(syn::Ident::parse_any)?);
    if input.peek(Token![=]) {
        Ok(Meta::NameValue(syn::MetaNameValue {
            path,
            eq_token: input.parse()?,
            value: input.parse()?,
        }))
    } else {
        Ok(Meta::Path(path))
    }
}

fn string_value(meta: &Meta, owner: &str) -> Result<String, String> {
    if let Meta::NameValue(value) = meta {
        if let Expr::Lit(lit) = &value.value {
            if let syn::Lit::Str(value) = &lit.lit {
                return Ok(value.value());
            }
        }
    }
    Err(format!("{owner}: expected a string attribute value"))
}

pub(super) fn check_name(
    attrs: &[Attribute],
    ser: &str,
    de: &str,
    owner: &str,
) -> Result<(), String> {
    if ser != de {
        return Err(format!(
            "{owner}: divergent serde serialize/deserialize names"
        ));
    }
    let ts = ts_attrs(attrs, owner)?;
    if let Some(name) = ts.rename {
        if name != de {
            return Err(format!(
                "{owner}: ts rename {name:?} differs from serde name {de:?}"
            ));
        }
    }
    for (key, ts_rule) in [
        ("rename_all", ts.rename_all),
        ("rename_all_fields", ts.rename_all_fields),
    ] {
        if let Some(ts_rule) = ts_rule {
            let mut serde_rule = None;
            for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
                let entries = attr
                    .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                    .map_err(|error| format!("{owner}: invalid serde attribute: {error}"))?;
                for entry in entries {
                    if entry.path().is_ident(key) {
                        serde_rule = Some(string_value(&entry, owner)?);
                    }
                }
            }
            if serde_rule.as_deref() != Some(ts_rule.as_str()) {
                return Err(format!("{owner}: ts {key} differs from serde naming rule"));
            }
        }
    }
    Ok(())
}

pub(super) fn derives(attrs: &[Attribute], name: &str) -> Result<bool, String> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("derive")) {
        let paths = attr
            .parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated)
            .map_err(|error| format!("invalid derive attribute: {error}"))?;
        if paths
            .iter()
            .any(|path| path.segments.last().is_some_and(|part| part.ident == name))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn wire_ty(ty: &Type, owner: &str) -> Result<WireTy, String> {
    let Type::Path(path) = ty else {
        return Err(format!("{owner}: unsupported field type"));
    };
    if path.qself.is_some() {
        return Err(format!("{owner}: unsupported qualified field type"));
    }
    let segment = path
        .path
        .segments
        .last()
        .ok_or_else(|| format!("{owner}: empty type path"))?;
    if path
        .path
        .segments
        .iter()
        .take(path.path.segments.len() - 1)
        .any(|segment| !matches!(segment.arguments, PathArguments::None))
    {
        return Err(format!("{owner}: unsupported generic type path"));
    }
    let name = segment.ident.to_string();
    if name == "Option" || name == "Vec" {
        let PathArguments::AngleBracketed(args) = &segment.arguments else {
            return Err(format!("{owner}: {name} needs one type argument"));
        };
        if args.args.len() != 1 {
            return Err(format!("{owner}: {name} needs one type argument"));
        }
        let Some(GenericArgument::Type(inner)) = args.args.first() else {
            return Err(format!("{owner}: unsupported {name} argument"));
        };
        let inner = Box::new(wire_ty(inner, owner)?);
        return Ok(if name == "Option" {
            WireTy::Option(inner)
        } else {
            WireTy::List(inner)
        });
    }
    if !matches!(segment.arguments, PathArguments::None) {
        return Err(format!("{owner}: unsupported generic field type {name}"));
    }
    const SAFE: i64 = 9_007_199_254_740_991;
    Ok(match name.as_str() {
        "String" => WireTy::Str,
        "bool" => WireTy::Bool,
        "f64" => WireTy::F64,
        "u8" => WireTy::Int {
            min: 0,
            max: u8::MAX.into(),
        },
        "u16" => WireTy::Int {
            min: 0,
            max: u16::MAX.into(),
        },
        "u32" => WireTy::Int {
            min: 0,
            max: u32::MAX.into(),
        },
        "u64" | "usize" => WireTy::Int { min: 0, max: SAFE },
        "i64" => WireTy::Int {
            min: -SAFE,
            max: SAFE,
        },
        _ => WireTy::Named(name),
    })
}

pub(super) fn serde_keys(attrs: &[Attribute], allowed: &[&str], owner: &str) -> Result<(), String> {
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        let entries = attr
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .map_err(|error| format!("{owner}: invalid serde attribute: {error}"))?;
        for entry in entries {
            let key = entry
                .path()
                .get_ident()
                .map(ToString::to_string)
                .ok_or_else(|| format!("{owner}: unsupported serde attribute path"))?;
            if !allowed.contains(&key.as_str()) {
                return Err(format!("{owner}: unsupported serde attribute {key}"));
            }
        }
    }
    Ok(())
}

pub(super) fn serde_optional(attrs: &[Attribute], owner: &str) -> Result<bool, String> {
    let mut omitted = false;
    let mut default = false;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        let entries = attr
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .map_err(|error| format!("{owner}: invalid public serde attribute: {error}"))?;
        for entry in entries {
            omitted |= entry.path().is_ident("skip_serializing_if")
                || entry.path().is_ident("skip_serializing");
            default |= entry.path().is_ident("default");
        }
    }
    Ok(omitted && default)
}
