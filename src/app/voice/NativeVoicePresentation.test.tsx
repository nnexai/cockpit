// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { NativeVoicePresentation } from "./NativeVoicePresentation";

const native = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: native.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: native.listen }));

type SnapshotFixture = {
  view_generation: number;
  revision: number;
  status: string;
  phase: null;
  muted: boolean;
  context: { session_id: string; selected_root_id: null; project_scope: null };
  transcript: { role: string; turn: number; text: string; final: boolean }[];
  handoffs: unknown[];
  error: null;
  end_reason: null;
  earlier_text_trimmed: boolean;
};

function snapshot(generation: number, revision: number, text: string, handoffs: unknown[] = []): SnapshotFixture {
  return {
    view_generation: generation, revision, status: "active", phase: null, muted: false,
    context: { session_id: "session", selected_root_id: null, project_scope: null },
    transcript: [{ role: "assistant", turn: 1, text, final: true }], handoffs,
    error: null, end_reason: null, earlier_text_trimmed: false,
  };
}

it("keeps the newest event when a delayed snapshot or stale event arrives", async () => {
  let listener: ((event: { payload: SnapshotFixture }) => void) | undefined;
  let resolveSnapshot!: (value: SnapshotFixture) => void;
  const pending = new Promise<SnapshotFixture>(resolve => { resolveSnapshot = resolve; });
  const unlisten = vi.fn();
  native.listen.mockImplementation(async (_name: string, callback: typeof listener) => { listener = callback; return unlisten; });
  native.invoke.mockReturnValue(pending);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => { root.render(createElement(NativeVoicePresentation)); });
    expect(native.listen).toHaveBeenCalledWith("cockpit-native-voice-presentation", expect.any(Function));
    expect(native.invoke).toHaveBeenCalledWith("cockpit_native_voice_snapshot");
    await act(async () => { listener!({ payload: snapshot(2, 4, "Current conversation") }); });
    await act(async () => { resolveSnapshot(snapshot(1, 99, "Old snapshot")); await pending; });
    await act(async () => {
      listener!({ payload: snapshot(1, 100, "Old generation") });
      listener!({ payload: snapshot(2, 3, "Old revision") });
    });
    expect(host.textContent).toContain("Current conversation");
    expect(host.textContent).not.toContain("Old snapshot");
    expect(host.textContent).not.toContain("Old generation");
    expect(host.textContent).not.toContain("Old revision");
    await act(async () => { listener!({ payload: snapshot(3, 1, "New conversation") }); });
    expect(host.textContent).toContain("New conversation");
    expect(host.textContent).not.toContain("Current conversation");
  } finally {
    await act(async () => { root.unmount(); });
    expect(unlisten).toHaveBeenCalledOnce();
    host.remove();
    native.invoke.mockReset();
    native.listen.mockReset();
  }
});

it("shows truthful handoff updates and exposes no browser call controls", async () => {
  const unlisten = vi.fn();
  native.listen.mockResolvedValue(unlisten);
  native.invoke.mockResolvedValue(snapshot(1, 1, "Hello", [
    { status: "update", thread_id: "thread-1", entries: [{ kind: "question", text: "Which database should we use?" }] },
    { status: "outcome_unknown", message: "The delivery result is uncertain." },
  ]));
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => { root.render(createElement(NativeVoicePresentation)); });
    const receipts = host.querySelector('[aria-label="Handoff receipts"]');
    expect(receipts?.textContent).toContain("Supervisor thread update");
    expect(receipts?.textContent).toContain("Thread: thread-1");
    expect(receipts?.textContent).toContain("Which database should we use?");
    expect(receipts?.textContent).toContain("Delivery not confirmed · check Inbox before trying again");
    expect(receipts?.textContent).toContain("The delivery result is uncertain.");
    expect(host.querySelector("audio, video, select")).toBeNull();
    expect(Array.from(host.querySelectorAll("button"), button => button.textContent)).not.toEqual(expect.arrayContaining([
      "Start voice call", "Start new call", "Mute", "Unmute", "End call", "Cancel",
    ]));
    expect(native.invoke.mock.calls.map(call => call[0])).toEqual(["cockpit_native_voice_snapshot"]);
  } finally {
    await act(async () => { root.unmount(); });
    host.remove();
    native.invoke.mockReset();
    native.listen.mockReset();
  }
});
