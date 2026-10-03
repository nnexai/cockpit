import { describe, expect, it } from "vitest";
import type { WidgetContent, WidgetSummary } from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";
import {
  matchWidgetContent, parseWidgetBody, parseWidgetChoicesSpec, parseWidgetContent,
  parseWidgetContentRequest, parseWidgetEvent, parseWidgetRemoveRequest, parseWidgetRemoveResponse,
  parseWidgetSelectRequest, parseWidgetSelectResponse, parseWidgetSelectionResponse,
  parseWidgetSummary, parseWidgetWindowReport,
} from "./widgetProtocol";

const key = { session_id: "session-1", tab_id: "tab-1", id: "latency" };
const sha256 = "a".repeat(64);
const summary: WidgetSummary = {
  key, space_id: "space-1", title: "Latency p95", revision: 2, created_seq: 1,
  kind: "choices", presentation: "choices", content: { sha256, bytes: 120, from: "file", name: "choices.json" },
  warnings: [], source: {
    pane_id: "pane-1", tab_id: "tab-1", space_id: "space-1", terminal_id: "terminal-1",
    agent_label: "omp", fingerprint_prefix: "012345abcdef", status: "present",
  },
  arrival: "own_tab", resolved_from: "current_pane", change: "updated",
  created_at_ms: 1000, updated_at_ms: 2000,
  selection: { revision: 1, at_ms: 1500, read_at_ms: null },
};
const content: WidgetContent = {
  key, revision: 2, sha256, selection: null,
  body: { type: "choices", spec: { prompt: "Which view?", choices: [
    { id: "latency", label: "Latency p95", detail: "per route" },
    { id: "errors", label: "Errors", detail: null },
  ] } },
};
const selected = {
  id: "latency", revision: 2, status: "selected", value_json: '{"id":"errors","label":"Errors"}',
  at_ms: 2000, removed_at_ms: null,
};

function rejects(parse: (value: unknown) => unknown, value: unknown): void {
  let failure: unknown;
  try { parse(value); } catch (error) { failure = error; }
  expect(failure).toBeInstanceOf(CockpitClientError);
  expect((failure as CockpitClientError).code).toBe("malformed_response");
}

describe("widget protocol events", () => {
  it("parses snapshot and upsert metadata without leaking unrecognized fields", () => {
    const event = parseWidgetEvent({ type: "snapshot", sequence: 4, widgets: [{ ...summary, private_html: "secret" }] });
    expect(event.type).toBe("snapshot");
    if (event.type !== "snapshot") throw new Error("Expected snapshot");
    expect(event.widgets[0].selection).toEqual({ revision: 1, at_ms: 1500, read_at_ms: null });
    expect(event.widgets[0]).not.toHaveProperty("private_html");
    const upsert = parseWidgetEvent({ type: "upserted", sequence: 5, widget: { ...summary, change: "replaced" } });
    expect(upsert.type).toBe("upserted");
    if (upsert.type !== "upserted") throw new Error("Expected upsert");
    expect(upsert.widget.change).toBe("replaced");
  });

  it("recognizes active HTML presentations without a static compatibility alias", () => {
    const html = { ...summary, kind: "html", presentation: "active" };
    expect(parseWidgetSummary(html).presentation).toBe("active");
    expect(parseWidgetEvent({ type: "upserted", sequence: 5, widget: html })).toMatchObject({
      type: "upserted", widget: { kind: "html", presentation: "active" },
    });
    rejects(parseWidgetSummary, { ...html, presentation: "static_preview" });
    rejects(parseWidgetSummary, { ...html, presentation: "choices" });
  });

  it.each(["user", "agent", "retired"])("parses %s removals without requiring a body", (reason) => {
    expect(parseWidgetEvent({ type: "removed", sequence: 6, key, reason })).toEqual({ type: "removed", sequence: 6, key, reason });
  });

  it.each([
    { type: "active", sequence: 1 },
    { type: "removed", sequence: 1, key, reason: "expired" },
    { type: "upserted", sequence: 1 },
    { type: "snapshot", sequence: 1, widgets: {} },
    { type: "snapshot", sequence: -1, widgets: [] },
    { type: "snapshot", sequence: Number.MAX_SAFE_INTEGER + 1, widgets: [] },
  ])("rejects malformed event %#", (value) => rejects(parseWidgetEvent, value));

  it.each([
    { kind: "active" }, { presentation: "active" }, { arrival: "elsewhere" },
    { resolved_from: "focused_pane" }, { change: "selected" },
    { source: { ...summary.source, status: "running" } },
    { content: { ...summary.content, from: "inline" } },
    { kind: "html", presentation: "choices" },
  ])("rejects unknown or inconsistent summary enum %#", (patch) => rejects(parseWidgetSummary, { ...summary, ...patch }));

  it("enforces text, slug, warning, hash and choices byte bounds", () => {
    expect(parseWidgetSummary({ ...summary, title: "😀".repeat(80) }).title).toBe("😀".repeat(80));
    rejects(parseWidgetSummary, { ...summary, title: "😀".repeat(81) });
    rejects(parseWidgetSummary, { ...summary, key: { ...key, id: "a".repeat(49) } });
    rejects(parseWidgetSummary, { ...summary, key: { ...key, id: "bad/id" } });
    rejects(parseWidgetSummary, { ...summary, warnings: ["a".repeat(513)] });
    rejects(parseWidgetSummary, { ...summary, warnings: Array(33).fill("warning") });
    rejects(parseWidgetSummary, { ...summary, content: { ...summary.content, sha256: "not-a-hash" } });
    rejects(parseWidgetSummary, { ...summary, content: { ...summary.content, bytes: 64 * 1024 + 1 } });
    rejects(parseWidgetSummary, { ...summary, source: { ...summary.source, fingerprint_prefix: "a".repeat(64) } });
  });

  it("rejects duplicate identities and per-tab overflow but permits independent tabs", () => {
    rejects(parseWidgetEvent, { type: "snapshot", sequence: 1, widgets: [summary, summary] });
    const widgets = Array.from({ length: 9 }, (_, index) => ({ ...summary, key: { ...key, id: `widget-${index}` } }));
    rejects(parseWidgetEvent, { type: "snapshot", sequence: 1, widgets });
    widgets[8].key.tab_id = "tab-2";
    const event = parseWidgetEvent({ type: "snapshot", sequence: 1, widgets });
    if (event.type !== "snapshot") throw new Error("Expected snapshot");
    expect(event.widgets[8].key.tab_id).toBe("tab-2");
  });
});

describe("widget bodies and content identity", () => {
  it("preserves active HTML scripts and normalizes omitted optional choice fields", () => {
    const document = '<script>window.cockpit.select({ answer: 42 })</script>';
    expect(parseWidgetBody({ type: "html", document })).toEqual({ type: "html", document });
    expect(parseWidgetBody({ type: "choices", spec: { choices: [{ id: "yes", label: "Yes" }] } })).toEqual({
      type: "choices", spec: { prompt: null, choices: [{ id: "yes", label: "Yes", detail: null }] },
    });
  });

  it("counts HTML bytes rather than UTF-16 code units", () => {
    const document = "é".repeat(512 * 1024);
    expect(parseWidgetBody({ type: "html", document })).toEqual({ type: "html", document });
    rejects(parseWidgetBody, { type: "html", document: `${document}é` });
    rejects(parseWidgetBody, { type: "active", document: "<p>Presentation is not body kind</p>" });
  });

  it.each([
    { choices: [] },
    { choices: [{ id: "yes", label: "Yes" }, { id: "yes", label: "Different" }] },
    { choices: [{ id: "bad id", label: "Yes" }] },
    { choices: [{ id: "yes", label: "" }] },
    { choices: [{ id: "yes", label: "a".repeat(81) }] },
    { choices: [{ id: "yes", label: "Yes", detail: "a".repeat(201) }] },
    { prompt: "a".repeat(201), choices: [{ id: "yes", label: "Yes" }] },
    { choices: [{ id: "yes", label: "Yes", script: "alert(1)" }] },
    { choices: [{ id: "yes", label: "Yes" }], multiple: true },
    { choices: Array.from({ length: 21 }, (_, index) => ({ id: `choice-${index}`, label: "Choice" })) },
  ])("rejects invalid declarative choices %#", (value) => rejects(parseWidgetChoicesSpec, value));

  it("matches every identity field and revision, not just the widget id", () => {
    expect(matchWidgetContent(content, { key, revision: 2 }).body).toEqual(content.body);
    for (const patch of [{ session_id: "other" }, { tab_id: "other" }, { id: "other" }]) {
      rejects((value) => matchWidgetContent(value, { key, revision: 2 }), { ...content, key: { ...key, ...patch } });
    }
    rejects((value) => matchWidgetContent(value, { key, revision: 2 }), { ...content, revision: 3 });
    rejects(parseWidgetContent, { ...content, body: { type: "page", value_json: "{}" } });
  });

  it("projects retained selection for remounts and replacements", () => {
    const selection = { ...selected, revision: 1, value_json: '{"filters":["errors"],"enabled":true}' };
    expect(parseWidgetContent({ ...content, selection: { ...selection, private: "ignored" } }).selection).toEqual(selection);
    rejects(parseWidgetContent, { ...content, selection: { ...selection, value_json: '{"nested":[1e999]}' } });
    rejects(parseWidgetContent, { ...content, selection: undefined });
    rejects(parseWidgetContent, { ...content, selection: { ...selection, id: "other" } });
    rejects(parseWidgetContent, { ...content, selection: { ...selection, revision: 3 } });
    rejects(parseWidgetContent, { ...content, selection: { ...selection, status: "none", value_json: null, at_ms: null } });
  });
});

describe("widget selection and window requests", () => {
  it("projects explicit declarative choices without forwarding extra fields", () => {
    const request = parseWidgetSelectRequest({ key, revision: 2, value: { type: "choice", choice_id: "errors", extra: "ignored" } });
    expect(request.value).toEqual({ type: "choice", choice_id: "errors" });
    rejects(parseWidgetSelectRequest, { key, revision: 2, value: { type: "choice", choice_id: "Bad Choice" } });
    rejects(parseWidgetSelectRequest, { key, revision: -1, value: request.value });
    rejects(parseWidgetContentRequest, { key, revision: 1.5 });
    rejects(parseWidgetRemoveRequest, { key: { ...key, tab_id: "" } });
  });

  it.each(["null", "true", "42", '"text"', "[1,null,false]", '{"answer":{"selected":[1,2]}}'])(
    "accepts bounded finite page JSON %s without forwarding target overrides",
    (value_json) => {
      expect(parseWidgetSelectRequest({
        key, revision: 2, value: { type: "page", value_json, key: { ...key, id: "other" }, revision: 999, extra: "ignored" },
      })).toEqual({ key, revision: 2, value: { type: "page", value_json } });
      expect(parseWidgetSelectionResponse({ ...selected, value_json }).value_json).toBe(value_json);
    },
  );

  it.each([
    { type: "unknown", value_json: "{}" },
    { type: "page", value_json: undefined },
    { type: "page", value_json: {} },
    { type: "page", value_json: "undefined" },
    { type: "page", value_json: '{"unterminated":' },
    { type: "page", value_json: "NaN" },
    { type: "page", value_json: "1e999" },
    { type: "page", value_json: '{"deep":[{"overflow":-1e999}]}' },
  ])("rejects invalid page selection input %#", (value) => {
    rejects(parseWidgetSelectRequest, { key, revision: 2, value });
    if (value.type === "page") rejects(parseWidgetSelectionResponse, { ...selected, value_json: value.value_json });
  });

  it("validates deeply nested finite JSON without recursive stack overflow", () => {
    const value_json = "[".repeat(4000) + "0" + "]".repeat(4000);
    expect(parseWidgetSelectRequest({ key, revision: 2, value: { type: "page", value_json } }).value).toEqual({ type: "page", value_json });
  });

  it("accepts selection and removal response states, rejecting unknown variants", () => {
    expect(parseWidgetSelectResponse({ at_ms: 0 }).at_ms).toBe(0);
    expect(parseWidgetRemoveResponse({ result: "already_removed" }).result).toBe("already_removed");
    rejects(parseWidgetSelectResponse, { at_ms: -1 });
    rejects(parseWidgetRemoveResponse, { result: "closed" });
    expect(JSON.parse(parseWidgetSelectionResponse(selected).value_json!)).toEqual({ id: "errors", label: "Errors" });
    expect(parseWidgetSelectionResponse({ ...selected, status: "dismissed", value_json: null, at_ms: null, removed_at_ms: 2500 }).removed_at_ms).toBe(2500);
    rejects(parseWidgetSelectionResponse, { ...selected, status: "active" });
    rejects(parseWidgetSelectionResponse, { ...selected, value_json: "not-json" });
    rejects(parseWidgetSelectionResponse, { ...selected, value_json: null });
    expect(parseWidgetSelectionResponse({ ...selected, value_json: '{"arbitrary":"untrusted page data"}' }).value_json).toBe('{"arbitrary":"untrusted page data"}');
    rejects(parseWidgetSelectionResponse, { ...selected, status: "none" });
  });

  it("enforces the selection JSON byte cap before parsing otherwise valid JSON", () => {
    const maximum = 16 * 1024;
    const value_json = selected.value_json.padEnd(maximum, " ");
    expect(parseWidgetSelectionResponse({ ...selected, value_json }).value_json).toBe(value_json);
    rejects(parseWidgetSelectionResponse, { ...selected, value_json: `${value_json} ` });
    expect(parseWidgetSelectRequest({ key, revision: 2, value: { type: "page", value_json } }).value).toEqual({ type: "page", value_json });
    rejects(parseWidgetSelectRequest, { key, revision: 2, value: { type: "page", value_json: `${value_json} ` } });
    const unicode = JSON.stringify("é".repeat((maximum - 2) / 2));
    expect(new TextEncoder().encode(unicode).byteLength).toBe(maximum);
    expect(parseWidgetSelectRequest({ key, revision: 2, value: { type: "page", value_json: unicode } }).value).toEqual({ type: "page", value_json: unicode });
    const over = JSON.stringify("é".repeat(maximum / 2));
    rejects(parseWidgetSelectRequest, { key, revision: 2, value: { type: "page", value_json: over } });
    rejects(parseWidgetSelectionResponse, { ...selected, value_json: over });
  });

  it("validates reports without accepting unknown blocker values or orphan displayed tabs", () => {
    expect(parseWidgetWindowReport({ session_id: "session-1", displayed_tab_id: "tab-1", blocker: "too_narrow" }).blocker).toBe("too_narrow");
    rejects(parseWidgetWindowReport, { session_id: "session-1", displayed_tab_id: "tab-1", blocker: "active" });
    rejects(parseWidgetWindowReport, { session_id: null, displayed_tab_id: "tab-1", blocker: null });
  });
});
