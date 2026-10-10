//! Type-only policy shards keep the generated index small without erasing names.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use super::super::wire_model::{WireField, WireShape, WireTy, WireType};
use super::{GENERATED_HEADER, quote};

const INTERFACES: [&str; 7] = [
    "WireTypes",
    "WireOwners",
    "WireFields",
    "WireFieldNames",
    "WireChildNames",
    "WireTagNames",
    "WireCheckpointFieldNames",
];
const UNIONS: [&str; 3] = ["WireOmittableField", "WireOptionalField", "WireListField"];

#[derive(Default)]
struct Metadata {
    interfaces: [String; 7],
    unions: [Vec<String>; 3],
}

impl Metadata {
    fn lines(&self) -> usize {
        self.interfaces
            .iter()
            .map(|body| body.lines().count())
            .sum::<usize>()
            + self.unions.iter().map(Vec::len).sum::<usize>()
    }

    fn append(&mut self, other: Self) {
        for (target, body) in self.interfaces.iter_mut().zip(other.interfaces) {
            target.push_str(&body);
        }
        for (target, values) in self.unions.iter_mut().zip(other.unions) {
            target.extend(values);
        }
    }

    fn render(&self) -> String {
        let mut out = String::from(GENERATED_HEADER);
        out.push_str("import type * as V1 from \"../v1\";\n\n");
        for (name, body) in INTERFACES.iter().zip(&self.interfaces) {
            writeln!(out, "export interface {name} {{").unwrap();
            out.push_str(body);
            out.push_str("}\n\n");
        }
        for (name, values) in UNIONS.iter().zip(&self.unions) {
            writeln!(out, "export type {name} =").unwrap();
            for value in values {
                writeln!(out, "  | {value}").unwrap();
            }
            out.push_str("  | never;\n\n");
        }
        out
    }
}

pub(super) fn render(
    model: &[WireType],
    descriptors: &[&str],
    locations: &BTreeMap<&str, &str>,
) -> Result<Vec<(PathBuf, String)>, String> {
    let object_names: BTreeSet<_> = model
        .iter()
        .filter(|ty| !matches!(ty.shape, WireShape::Literals(_)))
        .map(|ty| ty.name.as_str())
        .collect();
    let mut modules: BTreeMap<&str, Vec<&WireType>> = BTreeMap::new();
    for ty in model {
        modules.entry(ty.module).or_default().push(ty);
    }
    let mut files = Vec::new();
    let mut stems = Vec::new();
    for (module, types) in modules {
        let mut shard = Metadata::default();
        let mut number = 1;
        for ty in types {
            let entry = metadata(ty, &object_names);
            if entry.lines() > 1_400 {
                return Err(format!(
                    "policy metadata {} exceeds the generated chunk budget",
                    ty.name
                ));
            }
            if shard.lines() + entry.lines() > 1_400 {
                push_shard(&mut files, &mut stems, module, number, &shard);
                shard = Metadata::default();
                number += 1;
            }
            shard.append(entry);
        }
        push_shard(&mut files, &mut stems, module, number, &shard);
    }
    files.push((
        PathBuf::from("validate/index.ts"),
        render_index(model, descriptors, &stems, locations),
    ));
    Ok(files)
}

fn push_shard(
    files: &mut Vec<(PathBuf, String)>,
    stems: &mut Vec<String>,
    module: &str,
    number: usize,
    shard: &Metadata,
) {
    let stem = if number == 1 {
        format!("policy_{module}")
    } else {
        format!("policy_{module}_{number}")
    };
    files.push((PathBuf::from(format!("validate/{stem}.ts")), shard.render()));
    stems.push(stem);
}

fn metadata(ty: &WireType, objects: &BTreeSet<&str>) -> Metadata {
    let mut out = Metadata::default();
    writeln!(out.interfaces[0], "  {}: V1.{};", quote(&ty.name), ty.name).unwrap();
    match &ty.shape {
        WireShape::Struct(fields) => {
            owner(
                &mut out,
                &ty.name,
                &format!("V1.{}", ty.name),
                fields.iter().collect(),
                None,
                objects,
            );
        }
        WireShape::Literals(_) => {}
        WireShape::Tagged { tag, variants } => {
            let common = variants
                .first()
                .map(|(_, fields)| {
                    fields
                        .iter()
                        .filter(|field| {
                            variants.iter().all(|(_, fields)| {
                                fields.iter().any(|other| other.name == field.name)
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            owner(
                &mut out,
                &ty.name,
                &format!("V1.{}", ty.name),
                common,
                Some(tag),
                objects,
            );
            // The root owns only common fields, but dotted checkpoints may reach any variant's names.
            let all_names: BTreeSet<_> = variants
                .iter()
                .flat_map(|(_, fields)| fields.iter().map(|field| field.name.as_str()))
                .chain(std::iter::once(tag.as_str()))
                .collect();
            out.interfaces[6].clear();
            writeln!(
                out.interfaces[6],
                "  {}: {};",
                quote(&ty.name),
                union(all_names.into_iter())
            )
            .unwrap();
            let mut children: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
            for (_, fields) in variants {
                for field in fields {
                    if let Some(child) = object_child(&field.ty, objects) {
                        children.entry(&field.name).or_default().insert(child);
                    }
                }
            }
            out.interfaces[4].clear();
            for (field, children) in children {
                writeln!(
                    out.interfaces[4],
                    "  {}: {};",
                    quote(&format!("{}.{}", ty.name, field)),
                    union(children.into_iter())
                )
                .unwrap();
            }
            for (variant, fields) in variants {
                let id = format!("{}[{variant}]", ty.name);
                let value = format!(
                    "Extract<V1.{}, {{ {}: {} }}>",
                    ty.name,
                    quote(tag),
                    quote(variant)
                );
                owner(
                    &mut out,
                    &id,
                    &value,
                    fields.iter().collect(),
                    Some(tag),
                    objects,
                );
            }
        }
    }
    out
}

fn owner(
    out: &mut Metadata,
    id: &str,
    value: &str,
    fields: Vec<&WireField>,
    tag: Option<&str>,
    objects: &BTreeSet<&str>,
) {
    writeln!(out.interfaces[1], "  {}: {value};", quote(id)).unwrap();
    writeln!(
        out.interfaces[3],
        "  {}: {};",
        quote(id),
        union(fields.iter().map(|field| field.name.as_str()))
    )
    .unwrap();
    let names = fields
        .iter()
        .map(|field| field.name.as_str())
        .chain(tag.into_iter());
    writeln!(out.interfaces[6], "  {}: {};", quote(id), union(names)).unwrap();
    if let Some(tag) = tag {
        writeln!(out.interfaces[5], "  {}: {};", quote(id), quote(tag)).unwrap();
    }
    for field in fields {
        let field_id = format!("{id}.{}", field.name);
        let key = quote(&field_id);
        writeln!(
            out.interfaces[2],
            "  {key}: ({value})[{}];",
            quote(&field.name)
        )
        .unwrap();
        if field.ts_optional || field.fill.is_some() {
            out.unions[0].push(key.clone());
        }
        if field.ts_optional && field.fill.is_some() {
            out.unions[1].push(key.clone());
        }
        if list_field(&field.ty) {
            out.unions[2].push(key.clone());
        }
        if let Some(child) = object_child(&field.ty, objects) {
            writeln!(out.interfaces[4], "  {key}: {};", quote(child)).unwrap();
        }
    }
}

fn list_field(ty: &WireTy) -> bool {
    match ty {
        WireTy::Option(inner) => list_field(inner),
        WireTy::List(_) => true,
        _ => false,
    }
}

fn object_child<'a>(ty: &'a WireTy, objects: &BTreeSet<&str>) -> Option<&'a str> {
    match ty {
        WireTy::Option(inner) => object_child(inner, objects),
        WireTy::Named(name) if objects.contains(name.as_str()) => Some(name),
        // There is no names-only dotted syntax for a list index.
        _ => None,
    }
}

fn union<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let values: Vec<_> = names.map(quote).collect();
    if values.is_empty() {
        "never".to_owned()
    } else {
        values.join(" | ")
    }
}

fn render_index(
    model: &[WireType],
    descriptors: &[&str],
    shards: &[String],
    locations: &BTreeMap<&str, &str>,
) -> String {
    let mut out = String::from(GENERATED_HEADER);
    out.push_str("import type { WireDescriptor } from \"./runtime\";\n");
    for (index, stem) in descriptors.iter().enumerate() {
        writeln!(out, "import * as D{index} from \"./{stem}\";").unwrap();
        writeln!(out, "export * from \"./{stem}\";").unwrap();
    }
    for (index, stem) in shards.iter().enumerate() {
        writeln!(out, "import type * as P{index} from \"./{stem}\";").unwrap();
    }
    out.push('\n');
    for name in INTERFACES {
        if shards.is_empty() {
            writeln!(out, "export interface {name} {{}}").unwrap();
        } else {
            let bases = (0..shards.len())
                .map(|index| format!("P{index}.{name}"))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(out, "export interface {name} extends {bases} {{}}").unwrap();
        }
    }
    for name in UNIONS {
        let values = (0..shards.len())
            .map(|index| format!("P{index}.{name}"))
            .collect::<Vec<_>>();
        writeln!(
            out,
            "export type {name} = {};",
            if values.is_empty() {
                "never".to_owned()
            } else {
                values.join(" | ")
            }
        )
        .unwrap();
    }
    out.push_str(POLICY_TYPES);
    out.push_str("\nexport const wireRegistry = {\n");
    let descriptor_ids: BTreeMap<_, _> = descriptors
        .iter()
        .enumerate()
        .map(|(index, stem)| (*stem, index))
        .collect();
    for ty in model {
        let index = descriptor_ids[locations[ty.name.as_str()]];
        writeln!(out, "  {}: D{index}.wire{},", quote(&ty.name), ty.name).unwrap();
    }
    out.push_str(
        "} satisfies { readonly [K in keyof WireTypes]: WireDescriptor<WireTypes[K]> };\n",
    );
    out
}

const POLICY_TYPES: &str = r#"
/** Names come from the Rust model; cycle detection avoids unbounded recursive DTO paths. */
type ChildOwner<O extends keyof WireCheckpointFieldNames, F extends string> =
  `${O & string}.${F}` extends keyof WireChildNames ? WireChildNames[`${O & string}.${F}`] : never;
type WireCheckpoint<O extends keyof WireCheckpointFieldNames, Seen = never> = O extends Seen ? never :
  | WireCheckpointFieldNames[O]
  | { [F in WireCheckpointFieldNames[O] & string]: ChildOwner<O, F> extends infer C
      ? C extends keyof WireCheckpointFieldNames ? `${F}.${WireCheckpoint<C, Seen | O> & string}` : never
      : never }[WireCheckpointFieldNames[O] & string];
export type WireStep<O extends keyof WireFieldNames> =
  | WireCheckpoint<O> | `${WireCheckpoint<O> & string}:shallow` | `${WireCheckpoint<O> & string}:record` | `${WireCheckpoint<O> & string}:keys`
  | "keys" | "tag" | "tag:known" | `check:${string}`;

export interface TypedWirePolicy {
  /** Reject unknown keys only; `complete` additionally requires own required keys. */
  readonly exact?: ReadonlySet<keyof WireOwners | keyof WireTypes>;
  readonly complete?: ReadonlySet<keyof WireOwners | keyof WireTypes>;
  readonly absent?: ReadonlySet<WireOmittableField>;
  readonly nullish?: ReadonlySet<WireOmittableField>;
  readonly materialize?: ReadonlySet<WireOptionalField>;
  readonly raw?: ReadonlySet<keyof WireTypes | keyof WireFields>;
  readonly order?: { readonly [O in keyof WireFieldNames]?: readonly WireStep<O>[] };
  readonly emit?: { readonly [O in keyof WireFieldNames]?: readonly (WireFieldNames[O] | "tag")[] };
  readonly lengths?: { readonly [F in WireListField]?: { readonly min?: number; readonly max?: number } };
  readonly fields?: { readonly [F in keyof WireFields]?: (value: WireFields[F]) => boolean | string };
  readonly checks?: { readonly [O in keyof WireOwners]?: Readonly<Record<string, (staged: WireOwners[O], original: WireOwners[O]) => boolean>> };
  readonly overrides?: { readonly [F in keyof WireFields]?: TypedWirePolicy };
}
"#;
