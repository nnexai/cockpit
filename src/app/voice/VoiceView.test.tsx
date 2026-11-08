// @vitest-environment jsdom
import { act, createElement, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { VoiceView } from "./VoiceView";
import type { VoiceContainerProps } from "./VoiceContainer";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), selectClient: vi.fn(), stop: vi.fn(), container: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../client/select", () => ({ selectClient: mocks.selectClient }));
vi.mock("./VoiceContainer", () => ({ VoiceContainer: (props: VoiceContainerProps) => {
  mocks.container(props);
  useEffect(() => { props.onStopReady?.(mocks.stop); return () => props.onStopReady?.(null); }, [props.onStopReady]);
  return createElement("div", null, "Shared Voice");
} }));

it("loads the selected session and stops the shared call before leaving", async () => {
  mocks.invoke.mockReset().mockImplementation(async (command: string) => command === "cockpit_voice_view_context" ? { session_id: "session-1" } : undefined);
  mocks.stop.mockReset();
  // The project's ES2022 library does not include Promise.withResolvers.
  let resolveStop!: () => void;
  const stopping = new Promise<void>(resolve => { resolveStop = resolve; });
  mocks.stop.mockReturnValue(stopping);
  const client = {
    orchestrationLiaisonLookup: vi.fn(async () => ({ status: "resolved", roots: [{ root_id: "root-1" }] })),
    repositories: vi.fn(async () => ({ repositories: [{ repository_id: "repo-1", name: "Project" }] })),
    sessionSnapshot: vi.fn(async () => ({ spaces: [{ id: "space-1", label: "Space" }] })),
    voiceConnect: vi.fn(), voiceEvents: vi.fn(), voiceControl: vi.fn(),
  };
  mocks.selectClient.mockResolvedValue(client);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => { root.render(createElement(VoiceView)); });
    expect(mocks.container).toHaveBeenLastCalledWith(expect.objectContaining({
      sessionId: "session-1", roots: [{ id: "root-1", label: "root-1" }],
      projects: [{ id: "repo-1", label: "Project" }], workspaces: [{ id: "space-1", label: "Space" }],
    }));
    expect(client.sessionSnapshot).toHaveBeenCalledWith("session-1");
    await act(async () => { Array.from(host.querySelectorAll("button")).find(button => button.textContent?.includes("Inbox"))!.click(); });
    expect(mocks.stop).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalledWith("cockpit_voice_view_close", expect.anything());
    await act(async () => { resolveStop(); });
    expect(mocks.invoke).toHaveBeenCalledWith("cockpit_voice_view_close", { view: "inbox" });
  } finally {
    await act(async () => { root.unmount(); });
    host.remove();
  }
});

it("reports context failures without inventing a session", async () => {
  mocks.invoke.mockReset().mockResolvedValue({ session_id: null });
  mocks.selectClient.mockResolvedValue({});
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => { root.render(createElement(VoiceView)); });
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Select a session");
  } finally {
    await act(async () => { root.unmount(); });
    host.remove();
  }
});
