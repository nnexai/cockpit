// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryOperation } from "../../protocol/generated/v1";
import { LibraryProblems } from "./LibraryProblems";
import { dismissLibraryProblem, useLibraryOperation, useLibraryProblems } from "./useLibraryOperation";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

it("keeps foreground failures local and retains background failures until keyboard-accessible dismissal", async () => {
  vi.useFakeTimers();
  let operationId = "problem-foreground";
  const running = (): LibraryOperation => ({
    operation_id: operationId, kind: "refresh", phases: [{ phase: "library", state: "running", done: 0, total: 1, message: null, error: null }],
    item_ids: ["source:problem"], report: null, space: null, target: null, cancel_requested: false, finished: false, created_at: "", updated_at: "",
  });
  const client = { libraryOperation: vi.fn(async (id: string) => ({
    ...running(), operation_id: id, finished: true,
    phases: [{ phase: "library", state: "failed", done: 0, total: 1, message: null, error: { code: "source_cli_failed", message: "Provider offline" } }],
  })) } as unknown as CockpitClient;
  let pending: Promise<LibraryOperation | null> | undefined;
  let begin: () => Promise<LibraryOperation> = async () => running();
  let outstanding: string[] = [];
  function View() {
    const state = useLibraryOperation(client);
    return <button type="button" onClick={() => { pending = state.start(begin); }}>Refresh</button>;
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  function Chrome() {
    outstanding = useLibraryProblems().map((problem) => problem.id);
    return <div><LibraryProblems /><button type="button" data-commands>Commands</button></div>;
  }
  const chrome = <Chrome key="chrome" />;
  try {
    await act(async () => root.render(<><View key="view" />{chrome}</>));
    await act(async () => { host.querySelector<HTMLButtonElement>("button")!.click(); await pending; });
    await act(async () => { await vi.advanceTimersByTimeAsync(750); });
    expect(host.querySelector(".library-problems-pill")).toBeNull();
    operationId = "problem-background";
    await act(async () => { host.querySelector<HTMLButtonElement>("button")!.click(); await pending; });
    await act(async () => root.render(<>{chrome}</>));
    const commands = host.querySelector<HTMLButtonElement>("[data-commands]")!;
    commands.focus();
    await act(async () => { await vi.advanceTimersByTimeAsync(750); });
    const pill = host.querySelector<HTMLButtonElement>(".library-problems-pill")!;
    expect(pill.textContent).toContain("1 problem");
    expect(document.activeElement).toBe(commands);
    await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
    expect(host.querySelector(".library-problems-pill")).toBe(pill);
    await act(async () => pill.click());
    const popover = host.querySelector<HTMLElement>("[role='dialog']")!;
    expect(popover.textContent).toContain("Provider offline");
    expect(document.activeElement).toBe(popover);
    await act(async () => popover.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(host.querySelector("[role='dialog']")).toBeNull();
    expect(document.activeElement).toBe(pill);
    await act(async () => pill.click());
    await act(async () => host.querySelector<HTMLButtonElement>(".library-problems-popover button")!.click());
    expect(host.querySelector(".library-problems-pill")).toBeNull();
    expect(document.activeElement).toBe(commands);
    // Startup can fail after the dialog closes, before an operation ID exists.
    let rejectStart!: (cause: Error) => void;
    const startup = new Promise<LibraryOperation>((_resolve, reject) => { rejectStart = reject; });
    begin = () => startup;
    await act(async () => root.render(<><View key="view" />{chrome}</>));
    await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
    await act(async () => root.render(<>{chrome}</>));
    await act(async () => { rejectStart(new Error("Provider refused startup")); await pending; });
    await act(async () => host.querySelector<HTMLButtonElement>(".library-problems-pill")!.click());
    expect(host.querySelector("[role='dialog']")?.textContent).toContain("Provider refused startup");
    await act(async () => host.querySelector<HTMLButtonElement>(".library-problems-popover button")!.click());
    expect(host.querySelector(".library-problems-pill")).toBeNull();
  } finally {
    await act(async () => { root.unmount(); for (const id of outstanding) dismissLibraryProblem(id); });
    host.remove();
    vi.useRealTimers();
  }
});
