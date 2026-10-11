import { describe, expect, it, vi } from "vitest";
import { createBrowserClient } from "./browser";
import { createNativeClient } from "./native";

describe("project-root lookup admission", () => {
  it.each(["browser", "native"] as const)("refuses invalid routing or root selection before dispatch on %s", async transport => {
    const dispatch = vi.fn();
    const client = transport === "browser" ? createBrowserClient(dispatch) : createNativeClient(dispatch);
    const request = {
      session_id: "session-1", workspace_id: "space-1",
      selected_root_id: "11111111-1111-4111-8111-111111111111",
    };
    await expect(client.orchestrationProjectRootLookup({ ...request, session_id: "../session" }))
      .rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.orchestrationProjectRootLookup({ ...request, selected_root_id: "not-a-uuid" }))
      .rejects.toMatchObject({ code: "malformed_response" });
    expect(dispatch).not.toHaveBeenCalled();
  });
});
