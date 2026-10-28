import { describe, expect, it, vi } from "vitest";
import { createBrowserClient } from "./browser";
import { createNativeClient } from "./native";

describe("liaison lookup admission", () => {
  it.each(["browser", "native"] as const)("refuses invalid routing or root selection before dispatch on %s", async transport => {
    const dispatch = vi.fn();
    const client = transport === "browser" ? createBrowserClient(dispatch) : createNativeClient(dispatch);
    const request = {
      session_id: "session-1",
      selected_root_id: "11111111-1111-4111-8111-111111111111",
    };
    await expect(client.orchestrationLiaisonLookup({ ...request, session_id: "../session" }))
      .rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.orchestrationLiaisonLookup({ ...request, selected_root_id: "not-a-uuid" }))
      .rejects.toMatchObject({ code: "malformed_response" });
    expect(dispatch).not.toHaveBeenCalled();
  });
  it("routes the session lookup to the browser and native operations", async () => {
    const request = { session_id: "session-1", selected_root_id: null };
    const response = { status: "resolved", lookup_revision: 1, preferred_root_id: null, roots: [] };
    const fetch = vi.fn(async () => new Response(JSON.stringify(response), { status: 200 }));
    const invoke = vi.fn(async () => response);
    await expect(createBrowserClient(fetch).orchestrationLiaisonLookup(request)).resolves.toEqual(response);
    expect(fetch).toHaveBeenCalledWith("/api/v1/sessions/session-1/orchestration/liaison", expect.objectContaining({
      method: "POST", body: JSON.stringify(request),
    }));
    await expect(createNativeClient(invoke).orchestrationLiaisonLookup(request)).resolves.toEqual(response);
    expect(invoke).toHaveBeenCalledWith("orchestration_liaison_lookup", { request });
  });
});
