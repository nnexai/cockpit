import { describe, expect, it, vi } from "vitest";
import type { QuickAskLocation, QuickAskRequest } from "../protocol/generated/v1";
import { createBrowserClient } from "./browser";
import { createNativeClient } from "./native";
import { parseQuickAskAvailability, parseQuickAskLocation, parseQuickAskRequest } from "./quickAskProtocol";

const request: QuickAskRequest = {
  server_instance: "1234567890abcdef", space_id: "space-1", cwd: "/tmp",
  harness: "omp", prompt: "  literal @file\nnext line  ",
};
const location: QuickAskLocation = {
  space_id: "space-1", tab_id: "tab-1", pane_id: "pane-1", terminal_id: "terminal-1", tab_label: "Quick ask",
};

describe("quick ask protocol", () => {
  it("preserves prompt bytes and never includes them in validation errors", () => {
    expect(parseQuickAskRequest(request).prompt).toBe(request.prompt);
    expect(() => parseQuickAskRequest({ ...request, extra: true })).toThrow("Invalid quick ask data");
    expect(() => parseQuickAskRequest({ ...request, harness: "invalid" })).toThrow("Invalid quick ask data");
    try {
      parseQuickAskRequest({ ...request, harness: "invalid" });
    } catch (error) {
      expect(String(error)).not.toContain(request.prompt);
    }
  });
  it("rejects malformed locations and availability", () => {
    expect(parseQuickAskLocation(location)).toEqual(location);
    expect(() => parseQuickAskLocation({ ...location, terminal_id: null })).toThrow();
    expect(() => parseQuickAskAvailability({ agents: [] })).toThrow();
  });
  it("encodes one request for both HTTP and native without modifying the question", async () => {
    const fetcher = vi.fn(async () => new Response(JSON.stringify(location), { status: 200 }));
    const invoke = vi.fn(async () => location);
    await expect(createBrowserClient(fetcher).quickAsk("session-1", request)).resolves.toEqual(location);
    expect(fetcher).toHaveBeenCalledTimes(1);
    const [url, options] = fetcher.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toContain("/api/v1/sessions/session-1/quick-ask");
    expect(options.method).toBe("POST");
    expect(JSON.parse(String(options.body))).toEqual(request);
    await expect(createNativeClient(invoke).quickAsk("session-1", request)).resolves.toEqual(location);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("cockpit_quick_ask", { sessionId: "session-1", request });
  });
  it("rejects a malformed native location", async () => {
    const invoke = vi.fn(async () => ({ ...location, pane_id: null }));
    await expect(createNativeClient(invoke).quickAsk("session-1", request)).rejects.toMatchObject({ code: "malformed_response" });
  });
});
