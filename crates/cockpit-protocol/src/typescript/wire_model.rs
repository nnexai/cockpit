//! Wire metadata comes from the same module sources and type lists as ts-rs declarations.
use std::collections::{BTreeMap, BTreeSet};

use serde_derive_internals::{Ctxt, Derive, ast, attr};
use serde_json::{Map, Value};
use syn::DeriveInput;

use super::Section;

#[path = "wire_model_attrs.rs"]
mod attrs;
#[path = "wire_model_source.rs"]
mod source;
#[cfg(test)]
#[path = "wire_model_tests.rs"]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WireTy {
    Str,
    Bool,
    F64,
    Int { min: i64, max: i64 },
    Option(Box<WireTy>),
    List(Box<WireTy>),
    Named(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WireField {
    pub name: String,
    pub ty: WireTy,
    pub ts_optional: bool,
    /// ts-rs removes Option's null for explicit optional, unless a type override preserves it.
    pub ts_nonnull: bool,
    pub rust_omits: bool,
    pub fill: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WireShape {
    Struct(Vec<WireField>),
    Literals(Vec<String>),
    Tagged {
        tag: String,
        variants: Vec<(String, Vec<WireField>)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WireType {
    pub name: String,
    pub module: &'static str,
    pub shape: WireShape,
    pub deny_unknown_fields: bool,
}

struct Module {
    source: &'static str,
    file: syn::File,
    inputs: BTreeMap<String, DeriveInput>,
}

type Inputs = BTreeMap<String, (&'static str, DeriveInput)>;

pub(crate) fn wire_model(sections: &[&Section]) -> Result<Vec<WireType>, String> {
    let mut modules: BTreeMap<&'static str, Module> = BTreeMap::new();
    for section in sections {
        if let Some(module) = modules.get(section.module) {
            if module.source != section.source {
                return Err(format!(
                    "{}: sections have different module sources",
                    section.module
                ));
            }
            continue;
        }
        let file = syn::parse_file(section.source)
            .map_err(|error| format!("{}: cannot parse DTO source: {error}", section.module))?;
        let inputs = source::inputs(&file)?;
        modules.insert(
            section.module,
            Module {
                source: section.source,
                file,
                inputs,
            },
        );
    }
    let mut inputs = Inputs::new();
    let mut public_ts = BTreeMap::new();
    let mut order = Vec::new();
    for section in sections {
        let module = modules
            .get_mut(section.module)
            .ok_or_else(|| format!("{}: missing parsed module", section.module))?;
        for &name in section.names {
            if inputs.contains_key(name) {
                return Err(format!("{name}: duplicate listed type"));
            }
            let input = if name == "TerminalCommand" {
                let wire = source::terminal_alias(&module.file, &module.inputs)?;
                let public = module
                    .inputs
                    .remove(name)
                    .ok_or_else(|| format!("{name}: missing public TS source"))?;
                public_ts.insert(name, public);
                wire
            } else {
                if name == "RetirementState" && !module.inputs.contains_key(name) {
                    module
                        .inputs
                        .insert(name.to_owned(), source::retirement(&module.file)?);
                }
                module.inputs.remove(name).ok_or_else(|| {
                    format!("{name}: listed type has no source item or registered macro recognizer")
                })?
            };
            if !attrs::derives(&input.attrs, "Deserialize")? {
                return Err(format!(
                    "{name}: unsupported source without derived Deserialize"
                ));
            }
            order.push(name);
            inputs.insert(name.to_owned(), (section.module, input));
        }
    }
    let mut result = Vec::with_capacity(order.len());
    for name in order {
        let (module, input) = &inputs[name];
        let container = container(input, name)?;
        validate_container(&container, name)?;
        let shape = shape(&container, name, &inputs, public_ts.get(name))?;
        result.push(WireType {
            name: name.to_owned(),
            module,
            shape,
            deny_unknown_fields: container.attrs.deny_unknown_fields(),
        });
    }
    Ok(result)
}

fn container<'a>(input: &'a DeriveInput, owner: &str) -> Result<ast::Container<'a>, String> {
    let context = Ctxt::new();
    let parsed = ast::Container::from_ast(&context, input, Derive::Deserialize);
    context
        .check()
        .map_err(|error| format!("{owner}: invalid serde model: {error}"))?;
    parsed.ok_or_else(|| format!("{owner}: unsupported serde container"))
}

fn validate_container(container: &ast::Container<'_>, owner: &str) -> Result<(), String> {
    attrs::serde_keys(
        &container.original.attrs,
        &[
            "rename",
            "rename_all",
            "rename_all_fields",
            "tag",
            "default",
            "deny_unknown_fields",
        ],
        owner,
    )?;
    if !container.generics.params.is_empty() || container.generics.where_clause.is_some() {
        return Err(format!("{owner}: unsupported generic DTO"));
    }
    // The alias carries its public TS attributes but keeps the private wire container's serde name.
    attrs::check_name(
        &container.original.attrs,
        container.attrs.name().serialize_name(),
        container.attrs.name().deserialize_name(),
        owner,
    )?;
    reject_default_path(container.attrs.default(), owner)?;
    let ts = attrs::ts_attrs(&container.original.attrs, owner)?;
    if let Some(tag) = ts.tag {
        if !matches!(container.attrs.tag(), attr::TagType::Internal { tag: serde_tag } if tag == *serde_tag)
        {
            return Err(format!("{owner}: ts tag differs from serde tag"));
        }
    }
    Ok(())
}

fn shape(
    container: &ast::Container<'_>,
    owner: &str,
    inputs: &Inputs,
    ts_source: Option<&DeriveInput>,
) -> Result<WireShape, String> {
    match &container.data {
        ast::Data::Struct(ast::Style::Struct, fields) => {
            let default = if matches!(container.attrs.default(), attr::Default::Default) {
                named_default(owner, inputs, &mut BTreeSet::new())?
            } else {
                None
            };
            Ok(WireShape::Struct(fields_model(
                fields,
                owner,
                inputs,
                default.as_ref(),
                !container.attrs.default().is_none(),
                None,
            )?))
        }
        ast::Data::Enum(variants) => {
            if variants.is_empty() {
                return Err(format!("{owner}: unsupported empty enum"));
            }
            let mut names = BTreeSet::new();
            for variant in variants {
                validate_variant(variant, owner)?;
                if !names.insert(variant.attrs.name().deserialize_name()) {
                    return Err(format!("{owner}: duplicate serde variant name"));
                }
            }
            match container.attrs.tag() {
                attr::TagType::External
                    if variants
                        .iter()
                        .all(|variant| matches!(variant.style, ast::Style::Unit)) =>
                {
                    Ok(WireShape::Literals(
                        variants
                            .iter()
                            .map(|variant| variant.attrs.name().deserialize_name().to_owned())
                            .collect(),
                    ))
                }
                attr::TagType::Internal { tag } => {
                    let variants = variants
                        .iter()
                        .map(|variant| {
                            let label = variant.attrs.name().deserialize_name();
                            let variant_owner = format!("{owner}[{label}]");
                            if !matches!(variant.style, ast::Style::Struct | ast::Style::Unit) {
                                return Err(format!(
                                    "{variant_owner}: unsupported enum variant shape"
                                ));
                            }
                            let ts_fields = match ts_source.map(|input| &input.data) {
                                Some(syn::Data::Enum(data)) => Some(
                                    &data
                                        .variants
                                        .iter()
                                        .find(|public| public.ident == variant.ident)
                                        .ok_or_else(|| {
                                            format!("{variant_owner}: missing public TS variant")
                                        })?
                                        .fields,
                                ),
                                Some(_) => {
                                    return Err(format!(
                                        "{variant_owner}: unsupported public TS shape"
                                    ));
                                }
                                None => None,
                            };
                            let fields = fields_model(
                                &variant.fields,
                                &variant_owner,
                                inputs,
                                None,
                                false,
                                ts_fields,
                            )?;
                            if fields.iter().any(|field| field.name == *tag) {
                                return Err(format!(
                                    "{variant_owner}: field collides with serde tag"
                                ));
                            }
                            Ok((label.to_owned(), fields))
                        })
                        .collect::<Result<_, String>>()?;
                    Ok(WireShape::Tagged {
                        tag: tag.clone(),
                        variants,
                    })
                }
                _ => Err(format!("{owner}: unsupported enum representation")),
            }
        }
        _ => Err(format!("{owner}: unsupported DTO shape")),
    }
}

fn validate_variant(variant: &ast::Variant<'_>, owner: &str) -> Result<(), String> {
    let owner = format!("{owner}::{}", variant.ident);
    attrs::serde_keys(
        &variant.original.attrs,
        &["rename", "rename_all", "deserialize_with"],
        &owner,
    )?;
    attrs::check_name(
        &variant.original.attrs,
        variant.attrs.name().serialize_name(),
        variant.attrs.name().deserialize_name(),
        &owner,
    )?;
    if variant.attrs.deserialize_with().is_some() && !matches!(variant.style, ast::Style::Unit) {
        return Err(format!(
            "{owner}: unsupported payload variant deserialize_with"
        ));
    }
    Ok(())
}

fn fields_model(
    fields: &[ast::Field<'_>],
    owner: &str,
    inputs: &Inputs,
    container_default: Option<&Value>,
    has_container_default: bool,
    ts_fields: Option<&syn::Fields>,
) -> Result<Vec<WireField>, String> {
    let mut result = Vec::with_capacity(fields.len());
    let mut names = BTreeSet::new();
    for (index, field) in fields.iter().enumerate() {
        let name = field.attrs.name().deserialize_name();
        let field_owner = format!("{owner}.{name}");
        if !names.insert(name) {
            return Err(format!("{field_owner}: duplicate serde field name"));
        }
        attrs::serde_keys(
            &field.original.attrs,
            &["rename", "default", "skip_serializing_if"],
            &field_owner,
        )?;
        attrs::check_name(
            &field.original.attrs,
            field.attrs.name().serialize_name(),
            name,
            &field_owner,
        )?;
        reject_default_path(field.attrs.default(), &field_owner)?;
        let ty = attrs::wire_ty(field.ty, &field_owner)?;
        resolve_ty(&ty, inputs, &field_owner)?;
        let ts_attrs = match ts_fields {
            Some(fields) => {
                &fields
                    .iter()
                    .nth(index)
                    .ok_or_else(|| format!("{field_owner}: missing public TS field"))?
                    .attrs
            }
            None => &field.original.attrs,
        };
        let ts = attrs::ts_attrs(ts_attrs, &field_owner)?;
        if let Some(type_as) = &ts.type_as {
            // The one supported representation change makes a defaulted non-Option optional.
            let as_ty = attrs::wire_ty(type_as, &field_owner)?;
            if as_ty != ty && !matches!(&as_ty, WireTy::Option(inner) if **inner == ty) {
                return Err(format!("{field_owner}: unsupported ts(as) wire divergence"));
            }
        }
        let rust_omits = field.attrs.skip_serializing_if().is_some();
        let inferred_optional = if ts_fields.is_some() {
            attrs::serde_optional(ts_attrs, &field_owner)?
        } else {
            rust_omits && !field.attrs.default().is_none()
        };
        let ts_optional = !ts.optional_disabled && (ts.optional.is_some() || inferred_optional);
        let ts_nonnull = ts.optional == Some(false)
            && matches!(ty, WireTy::Option(_))
            && ts.type_override.is_none();
        validate_override(ts.type_override.as_deref(), &ty, inputs, &field_owner)?;
        let fill = match field.attrs.default() {
            attr::Default::Default => default_value(&ty, inputs, &mut BTreeSet::new())?,
            attr::Default::None if has_container_default => {
                container_default.and_then(|value| value.get(name)).cloned()
            }
            attr::Default::None if matches!(ty, WireTy::Option(_)) => Some(Value::Null),
            attr::Default::None => None,
            attr::Default::Path(_) => unreachable!("default path rejected above"),
        };
        result.push(WireField {
            name: name.to_owned(),
            ty,
            ts_optional,
            ts_nonnull,
            rust_omits,
            fill,
        });
    }
    Ok(result)
}

fn validate_override(
    value: Option<&str>,
    ty: &WireTy,
    inputs: &Inputs,
    owner: &str,
) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    let expected = match ty {
        WireTy::Int { .. } | WireTy::F64 => "number".to_owned(),
        WireTy::Option(inner) => match inner.as_ref() {
            WireTy::Int { .. } | WireTy::F64 => "number | null".to_owned(),
            WireTy::Named(name) if inputs.contains_key(name) => format!("{name} | null"),
            _ => return Err(format!("{owner}: unsupported ts(type) override")),
        },
        _ => return Err(format!("{owner}: unsupported ts(type) override")),
    };
    if value != expected {
        return Err(format!("{owner}: unsupported ts(type) override {value:?}"));
    }
    Ok(())
}

fn resolve_ty(ty: &WireTy, inputs: &Inputs, owner: &str) -> Result<(), String> {
    match ty {
        WireTy::Option(inner) | WireTy::List(inner) => resolve_ty(inner, inputs, owner),
        WireTy::Named(name) if !inputs.contains_key(name) => {
            Err(format!("{owner}: unsupported named source type {name}"))
        }
        _ => Ok(()),
    }
}

fn reject_default_path(default: &attr::Default, owner: &str) -> Result<(), String> {
    if let attr::Default::Path(path) = default {
        let path = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        return Err(format!(
            "{owner}: unsupported custom default function {path}"
        ));
    }
    Ok(())
}

fn default_value(
    ty: &WireTy,
    inputs: &Inputs,
    visiting: &mut BTreeSet<String>,
) -> Result<Option<Value>, String> {
    Ok(match ty {
        WireTy::Str => Some(Value::String(String::new())),
        WireTy::Bool => Some(Value::Bool(false)),
        WireTy::Int { .. } => Some(Value::from(0)),
        WireTy::F64 => Some(Value::from(0.0)),
        WireTy::Option(_) => Some(Value::Null),
        WireTy::List(_) => Some(Value::Array(Vec::new())),
        WireTy::Named(name) => named_default(name, inputs, visiting)?,
    })
}

fn named_default(
    name: &str,
    inputs: &Inputs,
    visiting: &mut BTreeSet<String>,
) -> Result<Option<Value>, String> {
    let (_, input) = inputs
        .get(name)
        .ok_or_else(|| format!("{name}: unsupported default source type"))?;
    // Never infer values from a manual Default implementation.
    if !attrs::derives(&input.attrs, "Default")? {
        return Ok(None);
    }
    if !visiting.insert(name.to_owned()) {
        return Err(format!("{name}: recursive derived Default"));
    }
    let container = container(input, name)?;
    validate_container(&container, name)?;
    let value = match &container.data {
        ast::Data::Struct(ast::Style::Struct, fields) => {
            let mut value = Map::new();
            for field in fields {
                let field_name = field.attrs.name().deserialize_name();
                let owner = format!("{name}.{field_name}");
                attrs::serde_keys(
                    &field.original.attrs,
                    &["rename", "default", "skip_serializing_if"],
                    &owner,
                )?;
                reject_default_path(field.attrs.default(), &owner)?;
                let ty = attrs::wire_ty(field.ty, &owner)?;
                let Some(fill) = default_value(&ty, inputs, visiting)? else {
                    visiting.remove(name);
                    return Ok(None);
                };
                if let Some(predicate) = field.attrs.skip_serializing_if() {
                    let parts = predicate
                        .path
                        .segments
                        .iter()
                        .map(|part| part.ident.to_string())
                        .collect::<Vec<_>>();
                    if parts != ["Option", "is_none"] || !matches!(ty, WireTy::Option(_)) {
                        return Err(format!(
                            "{owner}: unsupported default serialization predicate"
                        ));
                    }
                    if matches!(ty, WireTy::Option(_)) && fill.is_null() {
                        continue;
                    }
                }
                value.insert(field_name.to_owned(), fill);
            }
            Some(Value::Object(value))
        }
        ast::Data::Enum(variants) => {
            let defaults = variants
                .iter()
                .filter(|variant| {
                    variant
                        .original
                        .attrs
                        .iter()
                        .any(|attr| attr.path().is_ident("default"))
                })
                .collect::<Vec<_>>();
            if defaults.len() != 1 || !matches!(defaults[0].style, ast::Style::Unit) {
                return Err(format!("{name}: unsupported derived enum Default"));
            }
            validate_variant(defaults[0], name)?;
            let variant = defaults[0].attrs.name().deserialize_name().to_owned();
            match container.attrs.tag() {
                attr::TagType::External => Some(Value::String(variant)),
                attr::TagType::Internal { tag } => {
                    let mut value = Map::new();
                    value.insert(tag.clone(), Value::String(variant));
                    Some(Value::Object(value))
                }
                _ => return Err(format!("{name}: unsupported default enum representation")),
            }
        }
        _ => return Err(format!("{name}: unsupported derived Default shape")),
    };
    visiting.remove(name);
    Ok(value)
}
