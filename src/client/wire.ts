import { validateWire, validateWireList, WireFault, type WireDescriptor, type WireFailure, type WireFrame } from "../protocol/generated/validate/runtime";
import { wireRegistry, type TypedWirePolicy } from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";

export interface ProtocolPolicy { readonly wire: TypedWirePolicy; readonly message: (failure: WireFailure) => string }
type Entry = string | ((failure: WireFailure) => string);
type Descriptor = WireDescriptor<unknown>;
type Fields = NonNullable<Descriptor["fields"]>;
function has(object: object, key: string): boolean { return Object.hasOwn(object, key); }

function malformed(error: unknown, policy: ProtocolPolicy): never {
  if (error instanceof WireFault) throw new CockpitClientError("malformed_response", policy.message(error.failure));
  throw error;
}
export function parseWire<T>(value: unknown, descriptor: WireDescriptor<T>, policy: ProtocolPolicy): T {
  try { return validateWire(value, descriptor, policy.wire); } catch (error) { return malformed(error, policy); }
}
export function parseWireList<T>(value: unknown, descriptor: WireDescriptor<T>, policy: ProtocolPolicy, lengths?: { min?: number; max?: number }): T[] {
  try { return validateWireList(value, descriptor, policy.wire, lengths); } catch (error) { return malformed(error, policy); }
}
export function tryParseWire<T>(value: unknown, descriptor: WireDescriptor<T>, policy: ProtocolPolicy): T | undefined {
  try { return validateWire(value, descriptor, policy.wire); } catch (error) {
    if (error instanceof WireFault) return undefined;
    throw error;
  }
}
export const constantMessage = (text: string): (() => string) => () => text;
function ownerName(frame: WireFrame): string { return frame.variant === undefined ? frame.type : `${frame.type}[${frame.variant}]`; }
export function messageTable(table: Readonly<Partial<Record<string, Entry>>>, fallback: string, absorb?: ReadonlySet<string>): (failure: WireFailure) => string {
  const entry = (frame: WireFrame): Entry | undefined => table[ownerName(frame)] ?? table[frame.type];
  return failure => {
    for (const frame of failure.path) {
      if (frame.field === undefined || !(absorb?.has(`${ownerName(frame)}.${frame.field}`) || absorb?.has(`${frame.type}.${frame.field}`))) continue;
      const absorbed = entry(frame);
      if (absorbed !== undefined) return typeof absorbed === "string" ? absorbed : absorbed(failure);
    }
    for (let index = failure.path.length - 1; index >= 0; index--) {
      const inner = entry(failure.path[index]);
      if (inner !== undefined) return typeof inner === "string" ? inner : inner(failure);
    }
    return fallback;
  };
}
/** Supply labels for every parser root; unlabelled children retain the innermost labelled ancestor. */
export function rootMessage(prefix: string, rootLabels: Readonly<Partial<Record<string, string>>>): (failure: WireFailure) => string {
  const fallback = Object.values(rootLabels).find(label => label !== undefined);
  if (fallback === undefined) throw new Error("rootMessage requires a root label");
  return failure => {
    for (let index = failure.path.length - 1; index >= 0; index--) {
      const frame = failure.path[index], label = rootLabels[ownerName(frame)] ?? rootLabels[frame.type];
      if (label !== undefined) return `${prefix} ${label}`;
    }
    return `${prefix} ${rootLabels[failure.path[0]?.type] ?? fallback}`;
  };
}
function invalid(owner: string, detail: string): never { throw new Error(`Invalid wire policy ${owner}: ${detail}`); }
function ownerFields(owner: string): { descriptor: Descriptor; fields: Fields; variant: boolean } {
  const match = /^([^\[\]]+)(?:\[([^\]]+)\])?$/.exec(owner);
  const registry: Readonly<Record<string, Descriptor>> = wireRegistry;
  const descriptor = match && has(registry, match[1]) ? registry[match[1]] : undefined;
  if (!descriptor) return invalid(owner, "unknown owner");
  const variant = match![2];
  const fields = variant === undefined ? descriptor.fields : descriptor.variants && has(descriptor.variants, variant) ? descriptor.variants[variant] : undefined;
  if (!fields) return invalid(owner, "owner has no fields");
  return { descriptor, fields, variant: variant !== undefined };
}
function nested(check: Fields[string]["check"]): Descriptor | undefined {
  while (check.kind === "nullable" || check.kind === "list") check = check.inner!;
  return check.kind === "named" ? check.descriptor!() : undefined;
}
function fieldPath(owner: string, path: string, suffix?: string): void {
  let { descriptor, fields } = ownerFields(owner);
  const parts = path.split(".");
  for (let index = 0; index < parts.length; index++) {
    const name = parts[index], last = index === parts.length - 1;
    if (last && index > 0 && descriptor.tag === name && !suffix) return;
    if (!has(fields, name)) return invalid(owner, `unknown field path ${path}`);
    const child = nested(fields[name].check);
    if (last) {
      if ((suffix === "keys" || suffix === "record") && (!child || child.kind === "literal")) invalid(owner, `record checkpoint is not a record: ${path}`);
      return;
    }
    if (!child?.fields) return invalid(owner, `non-record field path ${path}`);
    descriptor = child;
    fields = child.fields;
  }
}
function validatePolicy(wire: TypedWirePolicy): void {
  for (const owner of [...wire.exact ?? [], ...wire.complete ?? []]) ownerFields(owner);
  for (const [owner, steps] of Object.entries(wire.order ?? {})) {
    if (!steps) continue;
    const { descriptor, fields, variant } = ownerFields(owner);
    const root = descriptor.kind === "tagged" && !variant;
    const inherited = variant ? (wire.order as Readonly<Record<string, readonly string[] | undefined>> | undefined)?.[descriptor.type] ?? [] : [];
    const deep = new Set<string>(), seen = new Set<string>();
    for (const step of steps) {
      if (seen.has(step)) invalid(owner, `duplicate step ${step}`);
      seen.add(step);
      if (step === "keys") continue;
      if (step === "tag" || step === "tag:known") {
        if (!root) invalid(owner, "tag checkpoints belong to the tagged root");
        continue;
      }
      if (step.startsWith("check:")) {
        const checks = (wire.checks as Readonly<Record<string, object | undefined>> | undefined)?.[owner];
        if (!checks || !has(checks, step.slice(6))) invalid(owner, `unknown check ${step}`);
        continue;
      }
      const [path, suffix, extra] = step.split(":");
      if (extra !== undefined || (suffix !== undefined && suffix !== "shallow" && suffix !== "record" && suffix !== "keys")) invalid(owner, `unknown checkpoint ${step}`);
      fieldPath(owner, path, suffix);
      if ((suffix === "shallow" || suffix === "record") && deep.has(path.split(".")[0])) invalid(owner, `shape checkpoint follows field ${path}`);
      if (!suffix && !path.includes(".")) {
        if (inherited.includes(path)) invalid(owner, `field already checked by tagged root: ${path}`);
        deep.add(path);
      }
    }
    if (root) {
      if (!seen.has("tag") || !seen.has("tag:known")) invalid(owner, "tag checkpoints are incomplete");
    } else if (Object.keys(fields).some(name => !deep.has(name) && !inherited.includes(name))) invalid(owner, "field order is incomplete");
  }
  for (const [owner, names] of Object.entries(wire.emit ?? {})) {
    if (!names) continue;
    const { descriptor, fields, variant } = ownerFields(owner);
    if (descriptor.kind === "tagged" && !variant) invalid(owner, "tagged emit requires a variant owner");
    const expected = [...Object.keys(fields), ...variant ? ["tag"] : []];
    if (new Set(names).size !== names.length || names.length !== expected.length || names.some(name => !expected.includes(name))) invalid(owner, "emit is not a complete field permutation");
  }
  for (const override of Object.values(wire.overrides ?? {})) if (override) validatePolicy(override);
}
/** Checks policy names and complete order/emit permutations against the generated source descriptors. */
export function definePolicy(policy: ProtocolPolicy): ProtocolPolicy {
  validatePolicy(policy.wire);
  return policy;
}
