import type {
  WidgetBody, WidgetChoice, WidgetChoicesSpec, WidgetContent, WidgetContentFacts,
  WidgetContentRequest, WidgetEvent, WidgetKey, WidgetRemoveRequest, WidgetRemoveResponse,
  WidgetSelectRequest, WidgetSelectResponse, WidgetSelectionFacts, WidgetSelectionResponse,
  WidgetSourceSummary, WidgetSummary, WidgetWindowReport,
} from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";

const MAX_HTML_BYTES = 1024 * 1024;
const MAX_CHOICES_BYTES = 64 * 1024;
const MAX_SELECTION_BYTES = 16 * 1024;
const encoder = new TextEncoder();
const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const integer = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const slug = (value: unknown): value is string => typeof value === "string" && /^[a-z0-9][a-z0-9_-]{0,47}$/.test(value);
const identity = (value: unknown): value is string => typeof value === "string" && value.length > 0 && value.length <= 512;
const hash = (value: unknown): value is string => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
const member = <T extends string>(value: unknown, choices: readonly T[]): value is T => typeof value === "string" && choices.includes(value as T);

// Error text never includes untrusted widget content or identities.
function malformed(): never {
  throw new CockpitClientError("malformed_response", "Invalid widget protocol response");
}

function text(value: unknown, maximum: number, minimum = 0): value is string {
  if (typeof value !== "string") return false;
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
    else if (record(item)) pending.push(...Object.values(item));
  }
  return value;
}

function nullable<T>(value: unknown, parser: (value: unknown) => T): T | null {
  return value === null ? null : parser(value);
}

function parseInteger(value: unknown): number {
  if (!integer(value)) malformed();
  return value;
}

export function parseWidgetKey(value: unknown): WidgetKey {
  if (!record(value) || !identity(value.session_id) || !identity(value.tab_id) || !slug(value.id)) malformed();
  return { session_id: value.session_id, tab_id: value.tab_id, id: value.id };
}

function parseContentFacts(value: unknown): WidgetContentFacts {
  if (!record(value) || !hash(value.sha256) || !integer(value.bytes) || value.bytes > MAX_HTML_BYTES
    || !member(value.from, ["file", "stdin"] as const) || !(value.name === null || text(value.name, 512))) malformed();
  return { sha256: value.sha256, bytes: value.bytes, from: value.from, name: value.name };
}

function parseSource(value: unknown): WidgetSourceSummary {
  if (!record(value) || !identity(value.pane_id) || !identity(value.tab_id) || !identity(value.space_id)
    || !identity(value.terminal_id) || !(value.agent_label === null || text(value.agent_label, 512))
    || !(value.fingerprint_prefix === null || (typeof value.fingerprint_prefix === "string" && /^[a-f0-9]{12}$/.test(value.fingerprint_prefix)))
    || !member(value.status, ["present", "closed", "restarted", "unknown"] as const)) malformed();
  return {
    pane_id: value.pane_id, tab_id: value.tab_id, space_id: value.space_id, terminal_id: value.terminal_id,
    agent_label: value.agent_label, fingerprint_prefix: value.fingerprint_prefix, status: value.status,
  };
}

function parseSelectionFacts(value: unknown): WidgetSelectionFacts {
  if (!record(value) || !integer(value.revision) || !integer(value.at_ms)
    || !(value.read_at_ms === null || integer(value.read_at_ms))) malformed();
  return { revision: value.revision, at_ms: value.at_ms, read_at_ms: value.read_at_ms };
}

export function parseWidgetSummary(value: unknown): WidgetSummary {
  if (!record(value) || !identity(value.space_id) || !text(value.title, 80) || !integer(value.revision)
    || !integer(value.created_seq) || !member(value.kind, ["html", "choices"] as const)
    || !member(value.presentation, ["active", "choices"] as const)
    || !Array.isArray(value.warnings) || value.warnings.length > 32 || !value.warnings.every((warning) => text(warning, 512))
    || !member(value.arrival, ["own_tab", "cross_source"] as const)
    || !member(value.resolved_from, ["current_pane", "pane", "tab", "space_focused_tab", "stored"] as const)
    || !member(value.change, ["opened", "replaced", "reopened", "updated"] as const)
    || !integer(value.created_at_ms) || !integer(value.updated_at_ms)) malformed();
  const content = parseContentFacts(value.content);
  if ((value.kind === "choices") !== (value.presentation === "choices")
    || (value.kind === "choices" && content.bytes > MAX_CHOICES_BYTES)) malformed();
  return {
    key: parseWidgetKey(value.key), space_id: value.space_id, title: value.title,
    revision: value.revision, created_seq: value.created_seq, kind: value.kind, presentation: value.presentation,
    content, warnings: value.warnings as string[], source: nullable(value.source, parseSource),
    arrival: value.arrival, resolved_from: value.resolved_from, change: value.change,
    created_at_ms: value.created_at_ms, updated_at_ms: value.updated_at_ms,
    selection: nullable(value.selection, parseSelectionFacts),
  };
}

export function parseWidgetEvent(value: unknown): WidgetEvent {
  if (!record(value) || !integer(value.sequence)) malformed();
  if (value.type === "snapshot") {
    if (!Array.isArray(value.widgets)) malformed();
    const widgets = value.widgets.map(parseWidgetSummary);
    const keys = new Set<string>();
    const counts = new Map<string, number>();
    for (const widget of widgets) {
      const key = JSON.stringify(widget.key);
      const tab = JSON.stringify([widget.key.session_id, widget.key.tab_id]);
      const count = (counts.get(tab) ?? 0) + 1;
      if (keys.has(key) || count > 8) malformed();
      keys.add(key);
      counts.set(tab, count);
    }
    return { type: "snapshot", sequence: value.sequence, widgets };
  }
  if (value.type === "upserted") return { type: "upserted", sequence: value.sequence, widget: parseWidgetSummary(value.widget) };
  if (value.type === "removed" && member(value.reason, ["user", "agent", "retired"] as const)) {
    return { type: "removed", sequence: value.sequence, key: parseWidgetKey(value.key), reason: value.reason };
  }
  return malformed();
}

function parseChoice(value: unknown): WidgetChoice {
  if (!record(value) || Object.keys(value).some((key) => !["id", "label", "detail"].includes(key))
    || !slug(value.id) || !text(value.label, 80, 1)
    || !(value.detail == null || text(value.detail, 200))) malformed();
  return { id: value.id, label: value.label, detail: value.detail == null ? null : value.detail };
}

export function parseWidgetChoicesSpec(value: unknown): WidgetChoicesSpec {
  if (!record(value) || Object.keys(value).some((key) => !["prompt", "choices"].includes(key))
    || !(value.prompt == null || text(value.prompt, 200)) || !Array.isArray(value.choices)
    || value.choices.length < 1 || value.choices.length > 20) malformed();
  const choices = value.choices.map(parseChoice);
  if (new Set(choices.map((choice) => choice.id)).size !== choices.length) malformed();
  const spec = { prompt: value.prompt == null ? null : value.prompt, choices };
  if (!boundedBytes(JSON.stringify(spec), MAX_CHOICES_BYTES)) malformed();
  return spec;
}

export function parseWidgetBody(value: unknown): WidgetBody {
  if (!record(value)) malformed();
  if (value.type === "html" && typeof value.document === "string" && boundedBytes(value.document, MAX_HTML_BYTES)) {
    return { type: "html", document: value.document };
  }
  if (value.type === "choices") return { type: "choices", spec: parseWidgetChoicesSpec(value.spec) };
  return malformed();
}

export function parseWidgetContent(value: unknown): WidgetContent {
  if (!record(value) || !integer(value.revision) || !hash(value.sha256)) malformed();
  const key = parseWidgetKey(value.key);
  const selection = nullable(value.selection, parseWidgetSelectionResponse);
  if (selection !== null && (selection.id !== key.id || selection.status !== "selected"
    || selection.revision === null || selection.revision > value.revision)) malformed();
  return {
    key, revision: value.revision, sha256: value.sha256, body: parseWidgetBody(value.body), selection,
  };
}

export function parseWidgetContentRequest(value: unknown): WidgetContentRequest {
  if (!record(value) || !integer(value.revision)) malformed();
  return { key: parseWidgetKey(value.key), revision: value.revision };
}

export function matchWidgetContent(value: unknown, request: WidgetContentRequest): WidgetContent {
  const content = parseWidgetContent(value);
  const expected = parseWidgetContentRequest(request);
  if (content.revision !== expected.revision || content.key.session_id !== expected.key.session_id
    || content.key.tab_id !== expected.key.tab_id || content.key.id !== expected.key.id) malformed();
  return content;
}

export function parseWidgetRemoveRequest(value: unknown): WidgetRemoveRequest {
  if (!record(value)) malformed();
  return { key: parseWidgetKey(value.key) };
}

export function parseWidgetRemoveResponse(value: unknown): WidgetRemoveResponse {
  if (!record(value) || !member(value.result, ["removed", "already_removed"] as const)) malformed();
  return { result: value.result };
}

export function parseWidgetSelectRequest(value: unknown): WidgetSelectRequest {
  if (!record(value) || !integer(value.revision) || !record(value.value)) malformed();
  const key = parseWidgetKey(value.key);
  if (value.value.type === "choice" && slug(value.value.choice_id)) {
    return { key, revision: value.revision, value: { type: "choice", choice_id: value.value.choice_id } };
  }
  if (value.value.type === "page") {
    return { key, revision: value.revision, value: { type: "page", value_json: parseSelectionJson(value.value.value_json) } };
  }
  return malformed();
}

export function parseWidgetSelectResponse(value: unknown): WidgetSelectResponse {
  if (!record(value) || !integer(value.at_ms)) malformed();
  return { at_ms: value.at_ms };
}

export function parseWidgetWindowReport(value: unknown): WidgetWindowReport {
  if (!record(value) || !(value.session_id === null || identity(value.session_id))
    || !(value.displayed_tab_id === null || identity(value.displayed_tab_id))
    || !(value.blocker === null || member(value.blocker, ["library", "zoom", "drag", "too_narrow"] as const))
    || (value.session_id === null && value.displayed_tab_id !== null)) malformed();
  return { session_id: value.session_id, displayed_tab_id: value.displayed_tab_id, blocker: value.blocker };
}

export function parseWidgetSelectionResponse(value: unknown): WidgetSelectionResponse {
  if (!record(value) || !slug(value.id) || !member(value.status, ["none", "selected", "timeout", "dismissed", "retired"] as const)) malformed();
  const revision = nullable(value.revision, parseInteger);
  const at_ms = nullable(value.at_ms, parseInteger);
  const removed_at_ms = nullable(value.removed_at_ms, parseInteger);
  let value_json: string | null = null;
  if (value.value_json !== null) {
    value_json = parseSelectionJson(value.value_json);
  }
  if ((value.status === "selected") !== (value_json !== null)
    || (value.status === "selected" && (revision === null || at_ms === null))
    || (value.status !== "selected" && at_ms !== null)
    || (value.status === "dismissed") !== (removed_at_ms !== null)) malformed();
  return { id: value.id, revision, status: value.status, value_json, at_ms, removed_at_ms };
}
