// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { VoiceScreen } from "./VoiceScreen";
import type { VoiceTransport, VoiceTransportEvent } from "./useVoiceSession";

vi.mock("./voicePeer", () => ({
  createVoicePeer: () => ({ start: async () => {}, stop: () => {}, setMuted: () => {} }),
}));

it("shows authoritative supervisor thread update text after a handoff", async () => {
  let listener: ((event: VoiceTransportEvent) => void) | undefined;
  const transport: VoiceTransport = {
    connect: async () => "answer",
    stop: () => {},
    setMuted: async () => {},
    subscribe: (callback) => { listener = callback; return () => { listener = undefined; }; },
  };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  try {
    await act(async () => {
      root.render(createElement(VoiceScreen, {
        transport, sessionId: "session", roots: [{ id: "root-1", label: "Supervisor" }], projects: [], workspaces: [], selectedRootId: null,
        selectedProjectId: null, selectedWorkspaceId: null, onRootChange: () => {}, onProjectChange: () => {}, onWorkspaceChange: () => {},
      }));
    });
    const start = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "Start voice call")!;
    await act(async () => { start.click(); });
    expect(listener).toBeDefined();
    await act(async () => {
      listener!({ type: "handoff", receipt: { status: "update", thread_id: "thread-1", root_id: "root-1", entries: [{ message_id: "message-1", kind: "question", text: "Which database should we use?", part: 0, more: false, outcome: null }] } });
    });
    const receipts = host.querySelector('[aria-label="Handoff receipts"]')!;
    expect(receipts.textContent).toContain("Supervisor thread update");
    expect(receipts.textContent).toContain("Which database should we use?");
    expect(receipts.textContent).not.toContain("Handoff status unavailable");
  } finally {
    await act(async () => { root.unmount(); });
    pause.mockRestore();
    host.remove();
  }
});

it("does not start or request a microphone when no supervisor is available", async () => {
  const subscribe = vi.fn(() => () => {});
  const transport: VoiceTransport = { connect: vi.fn(async () => "answer"), stop: () => {}, setMuted: async () => {}, subscribe };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  try {
    await act(async () => {
      root.render(createElement(VoiceScreen, {
        transport, sessionId: "session", roots: [], projects: [], workspaces: [], selectedRootId: null,
        selectedProjectId: null, selectedWorkspaceId: null, onRootChange: () => {}, onProjectChange: () => {}, onWorkspaceChange: () => {},
      }));
    });
    const start = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "Start voice call")!;
    expect(start.disabled).toBe(true);
    expect(host.textContent).toContain("Start a supervisor in Workbench first");
    expect(host.textContent).not.toContain("Ready to start");
    await act(async () => { start.click(); });
    expect(subscribe).not.toHaveBeenCalled();
  } finally {
    await act(async () => { root.unmount(); });
    pause.mockRestore();
    host.remove();
  }
});

it("keeps context choices explicit and privacy details available without duplicating status", async () => {
  const onRootChange = vi.fn();
  const transport: VoiceTransport = { connect: vi.fn(async () => "answer"), stop: () => {}, setMuted: async () => {}, subscribe: () => () => {} };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  try {
    await act(async () => {
      root.render(createElement(VoiceScreen, {
        transport, sessionId: "session", roots: [{ id: "root-1", label: "Root one" }],
        projects: [{ id: "project-1", label: "Project one" }], workspaces: [{ id: "space-1", label: "Space one" }],
        selectedRootId: null, selectedProjectId: "project-1", selectedWorkspaceId: "space-1",
        onRootChange, onProjectChange: () => {}, onWorkspaceChange: () => {},
      }));
    });
    const context = host.querySelector<HTMLDetailsElement>(".voice-context-details")!;
    expect(context.open).toBe(false);
    expect(context.querySelector("summary")?.textContent).toContain("Root: preferred");
    expect(context.querySelector("summary")?.textContent).toContain("Project: Project one");
    expect(context.querySelector("summary")?.textContent).toContain("Space: Space one");
    context.open = true;
    const selects = context.querySelectorAll("select");
    expect(selects).toHaveLength(3);
    await act(async () => {
      selects[0].value = "root-1";
      selects[0].dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(onRootChange).toHaveBeenCalledWith("root-1");
    expect(host.querySelector(".voice-privacy")?.textContent).toContain("Audio is sent to OpenAI");
    expect(host.querySelector(".voice-privacy details")?.textContent).toContain("OpenAI audio retention is not known");
    expect(host.querySelectorAll('[role="status"]')).toHaveLength(1);
    expect(host.querySelector('[role="status"]')?.textContent).toContain("Ready to start");
    await act(async () => { Array.from(host.querySelectorAll("button")).find(button => button.textContent === "Start voice call")!.click(); });
    expect(Array.from(selects).every(select => select.disabled)).toBe(true);
    expect(context.querySelector("summary")?.textContent).toContain("fixed for this call");
  } finally {
    await act(async () => { root.unmount(); });
    pause.mockRestore();
    host.remove();
  }
});

it("does not report ready when the session or selected context is unavailable", async () => {
  const transport: VoiceTransport = { connect: vi.fn(async () => "answer"), stop: () => {}, setMuted: async () => {}, subscribe: () => () => {} };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  const props = {
    transport, sessionId: null as string | null, roots: [{ id: "root-1", label: "Root" }], projects: [], workspaces: [],
    selectedRootId: null as string | null, selectedProjectId: null, selectedWorkspaceId: null,
    onRootChange: () => {}, onProjectChange: () => {}, onWorkspaceChange: () => {},
  };
  try {
    await act(async () => { root.render(createElement(VoiceScreen, props)); });
    expect(host.querySelector('[role="status"]')?.textContent).toContain("Session unavailable");
    await act(async () => { root.render(createElement(VoiceScreen, { ...props, sessionId: "session", selectedRootId: "missing" })); });
    expect(host.querySelector('[role="status"]')?.textContent).toContain("Context unavailable");
    expect(host.querySelector<HTMLButtonElement>(".voice-primary")?.disabled).toBe(true);
  } finally {
    await act(async () => { root.unmount(); });
    pause.mockRestore();
    host.remove();
  }
});
