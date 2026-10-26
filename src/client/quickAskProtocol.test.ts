import { describe, expect, it, vi } from "vitest";
import type { QuickAskStartRequest, QuickAskStatus } from "../protocol/generated/v1";
import { createBrowserClient } from "./browser";
import { createNativeClient } from "./native";
import { parseQuickAskAvailability, parseQuickAskStartRequest, parseQuickAskStatus } from "./quickAskProtocol";

const request: QuickAskStartRequest = {
  request_id: "01234567-89ab-4cde-8fab-0123456789ab", server_instance: "1234567890abcdef",
  space_id: "space-1", cwd: "/tmp", harness: "omp", prompt: "  literal @file\nnext line  ",
};
const status: QuickAskStatus = {
  request_id: request.request_id, session_id: "session-1", server_instance: request.server_instance,
  harness: request.harness, phase: "creating_tab", delivery: "not_sent", cancellable: true,
  location: null, error: null, updated_at_ms: 1,
};

describe("quick ask protocol", () => {
  it("preserves prompt bytes and never includes them in validation errors", () => {
    expect(parseQuickAskStartRequest(request).prompt).toBe(request.prompt);
    expect(() => parseQuickAskStartRequest({ ...request, extra: true })).toThrow("Invalid quick ask data");
  });
  it("rejects inconsistent status and availability", () => {
    expect(() => parseQuickAskStatus({ ...status, delivery: "submitted" })).toThrow();
    expect(() => parseQuickAskStatus({ ...status, phase: "initial_question_supplied" })).toThrow();
    expect(parseQuickAskStatus({ ...status, phase: "initial_question_supplied", delivery: "initial_question_supplied", cancellable: false }).delivery).toBe("initial_question_supplied");
    expect(() => parseQuickAskAvailability({ agents: [] })).toThrow();
  });
  it("encodes start once for both HTTP and native without modifying the question", async () => {
    const fetcher = vi.fn(async () => new Response(JSON.stringify(status), { status: 200 }));
    const invoke = vi.fn(async () => status);
    await expect(createBrowserClient(fetcher).quickAskStart("session-1", request)).resolves.toEqual(status);
    expect(fetcher).toHaveBeenCalledTimes(1);
    const [url, options] = fetcher.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toContain("/api/v1/sessions/session-1/quick-ask");
    expect(JSON.parse(String(options.body))).toEqual(request);
    await expect(createNativeClient(invoke).quickAskStart("session-1", request)).resolves.toEqual(status);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("cockpit_quick_ask_start", { sessionId: "session-1", request });
  });
  it("rejects a status from another request", async () => {
    const invoke = vi.fn(async () => ({ ...status, request_id: "different" }));
    await expect(createNativeClient(invoke).quickAskStatus("session-1", request.request_id)).rejects.toMatchObject({ code: "malformed_response" });
  });
});
