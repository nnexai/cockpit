import type {
  WidgetBody, WidgetChoicesSpec, WidgetContent, WidgetContentRequest, WidgetEvent, WidgetKey,
  WidgetRemoveRequest, WidgetRemoveResponse, WidgetSelectRequest, WidgetSelectResponse,
  WidgetSelectionResponse, WidgetSummary, WidgetWindowReport,
} from "../protocol/generated/v1";
import {
  wireWidgetBody, wireWidgetChoicesSpec, wireWidgetContent, wireWidgetContentRequest,
  wireWidgetEvent, wireWidgetKey, wireWidgetRemoveRequest, wireWidgetRemoveResponse,
  wireWidgetSelectRequest, wireWidgetSelectResponse, wireWidgetSelectionResponse,
  wireWidgetSummary, wireWidgetWindowReport, type TypedWirePolicy,
} from "../protocol/generated/validate";
import { CockpitClientError } from "./CockpitClient";
import { constantMessage, definePolicy, parseWire } from "./wire";

const MAX_HTML_BYTES = 1024 * 1024;
const MAX_CHOICES_BYTES = 64 * 1024;
const MAX_SELECTION_BYTES = 16 * 1024;
const encoder = new TextEncoder();
const slug = (value: string): boolean => /^[a-z0-9][a-z0-9_-]{0,47}$/.test(value);
const identity = (value: string): boolean => value.length > 0 && value.length <= 512;
const hash = (value: string): boolean => /^[a-f0-9]{64}$/.test(value);

// Error text never includes untrusted widget content or identities.
function malformed(): never {
  throw new CockpitClientError("malformed_response", "Invalid widget protocol response");
}
function text(value: string, maximum: number, minimum = 0): boolean {
  let length = 0;
  for (const _character of value) {
    if (++length > maximum) return false;
  }
  return length >= minimum;
}
function boundedBytes(value: string, maximum: number): boolean {
  return value.length <= maximum && encoder.encode(value).byteLength <= maximum;
}
function parseSelectionJson(value: unknown): string {
  if (typeof value !== "string" || !boundedBytes(value, MAX_SELECTION_BYTES)) malformed();
  let parsed: unknown;
  try { parsed = JSON.parse(value); } catch { malformed(); }
  const pending: unknown[] = [parsed];
  while (pending.length > 0) {
    const item = pending.pop();
    if (typeof item === "number" && !Number.isFinite(item)) malformed();
    if (Array.isArray(item)) pending.push(...item);
    else if (item !== null && typeof item === "object") pending.push(...Object.values(item));
  }
  return value;
}
function distinctWidgets(widgets: WidgetSummary[]): boolean {
  const keys = new Set<string>();
  const counts = new Map<string, number>();
  for (const widget of widgets) {
    const key = JSON.stringify(widget.key);
    const tab = JSON.stringify([widget.key.session_id, widget.key.tab_id]);
    const count = (counts.get(tab) ?? 0) + 1;
    if (keys.has(key) || count > 8) return false;
    keys.add(key);
    counts.set(tab, count);
  }
  return true;
}
const WIDGET = definePolicy({
  message: constantMessage("Invalid widget protocol response"),
  wire: {
    exact: new Set(["WidgetChoicesSpec", "WidgetChoice"]),
    nullish: new Set(["WidgetChoicesSpec.prompt", "WidgetChoice.detail"]),
    raw: new Set(["WidgetSummary.warnings"]),
    lengths: { "WidgetSummary.warnings": { max: 32 }, "WidgetChoicesSpec.choices": { min: 1, max: 20 } },
    fields: {
      "WidgetKey.session_id": identity, "WidgetKey.tab_id": identity, "WidgetKey.id": slug,
      "WidgetContentFacts.sha256": hash, "WidgetContentFacts.bytes": (v) => v <= MAX_HTML_BYTES,
      "WidgetContentFacts.name": (v) => v === null || text(v, 512),
      "WidgetSourceSummary.pane_id": identity, "WidgetSourceSummary.tab_id": identity,
      "WidgetSourceSummary.space_id": identity, "WidgetSourceSummary.terminal_id": identity,
      "WidgetSourceSummary.agent_label": (v) => v === null || text(v, 512),
      "WidgetSourceSummary.fingerprint_prefix": (v) => v === null || /^[a-f0-9]{12}$/.test(v),
      "WidgetSummary.space_id": identity, "WidgetSummary.title": (v) => text(v, 80),
      "WidgetSummary.warnings": (v) => v.every((warning) => text(warning, 512)),
      "WidgetChoice.id": slug, "WidgetChoice.label": (v) => text(v, 80, 1),
      "WidgetChoice.detail": (v) => v === null || text(v, 200),
      "WidgetChoicesSpec.prompt": (v) => v === null || text(v, 200),
      "WidgetBody[html].document": (v) => boundedBytes(v, MAX_HTML_BYTES),
      "WidgetContent.sha256": hash, "WidgetSelectValue[choice].choice_id": slug,
      "WidgetSelectValue[page].value_json": (v) => parseSelectionJson(v) === v,
      "WidgetWindowReport.session_id": (v) => v === null || identity(v),
      "WidgetWindowReport.displayed_tab_id": (v) => v === null || identity(v),
      "WidgetSelectionResponse.id": slug,
      "WidgetSelectionResponse.value_json": (v) => v === null || parseSelectionJson(v) === v,
    },
    checks: {
      WidgetSummary: { presentation: (v) => (v.kind === "choices") === (v.presentation === "choices")
        && (v.kind !== "choices" || v.content.bytes <= MAX_CHOICES_BYTES) },
      "WidgetEvent[snapshot]": { identities: (v) => distinctWidgets(v.widgets) },
      WidgetChoicesSpec: { choices: (v) => new Set(v.choices.map((choice) => choice.id)).size === v.choices.length
        && boundedBytes(JSON.stringify(v), MAX_CHOICES_BYTES) },
      WidgetContent: { selection: (v) => v.selection === null || (v.selection.id === v.key.id
        && v.selection.status === "selected" && v.selection.revision !== null && v.selection.revision <= v.revision) },
      WidgetWindowReport: { displayed: (v) => v.session_id !== null || v.displayed_tab_id === null },
      WidgetSelectionResponse: { selection: (v) => (v.status === "selected") === (v.value_json !== null)
        && (v.status !== "selected" || (v.revision !== null && v.at_ms !== null))
        && (v.status === "selected" || v.at_ms === null)
        && (v.status === "dismissed") === (v.removed_at_ms !== null) },
    },
  } satisfies TypedWirePolicy,
});

export function parseWidgetKey(value: unknown): WidgetKey { return parseWire(value, wireWidgetKey, WIDGET); }
export function parseWidgetSummary(value: unknown): WidgetSummary { return parseWire(value, wireWidgetSummary, WIDGET); }
export function parseWidgetEvent(value: unknown): WidgetEvent { return parseWire(value, wireWidgetEvent, WIDGET); }
export function parseWidgetChoicesSpec(value: unknown): WidgetChoicesSpec { return parseWire(value, wireWidgetChoicesSpec, WIDGET); }
export function parseWidgetBody(value: unknown): WidgetBody { return parseWire(value, wireWidgetBody, WIDGET); }
export function parseWidgetContent(value: unknown): WidgetContent { return parseWire(value, wireWidgetContent, WIDGET); }
export function parseWidgetContentRequest(value: unknown): WidgetContentRequest { return parseWire(value, wireWidgetContentRequest, WIDGET); }
export function parseWidgetRemoveRequest(value: unknown): WidgetRemoveRequest { return parseWire(value, wireWidgetRemoveRequest, WIDGET); }
export function parseWidgetRemoveResponse(value: unknown): WidgetRemoveResponse { return parseWire(value, wireWidgetRemoveResponse, WIDGET); }
export function parseWidgetSelectRequest(value: unknown): WidgetSelectRequest { return parseWire(value, wireWidgetSelectRequest, WIDGET); }
export function parseWidgetSelectResponse(value: unknown): WidgetSelectResponse { return parseWire(value, wireWidgetSelectResponse, WIDGET); }
export function parseWidgetWindowReport(value: unknown): WidgetWindowReport { return parseWire(value, wireWidgetWindowReport, WIDGET); }
export function parseWidgetSelectionResponse(value: unknown): WidgetSelectionResponse { return parseWire(value, wireWidgetSelectionResponse, WIDGET); }

export function matchWidgetContent(value: unknown, request: WidgetContentRequest): WidgetContent {
  const content = parseWidgetContent(value);
  const expected = parseWidgetContentRequest(request);
  if (content.revision !== expected.revision || content.key.session_id !== expected.key.session_id
    || content.key.tab_id !== expected.key.tab_id || content.key.id !== expected.key.id) malformed();
  return content;
}
