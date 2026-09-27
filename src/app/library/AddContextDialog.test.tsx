// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryOperation } from "../../protocol/generated/v1";
import { AddContextDialog } from "./AddContextDialog";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

afterEach(() => {
  vi.useRealTimers();
});

async function advance(milliseconds: number): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(milliseconds);
  });
}

it("resolves a Jira key on the configured site, adds to the Library only, and offers Open in Library", async () => {
  vi.useFakeTimers();
  const saved: LibraryOperation = {
    operation_id: "op-1", kind: "add", phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:ops-311"],
    report: null, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test/jira", executable: "jira" }] })),
    libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: "Rotate signing keys", canonical_id: "OPS-311", container_label: null, existing_item_id: null, existing_follow_id: null, page_count: null, git_working_tree: null, file_count: null, diagnostics: [] })),
    libraryAdd: vi.fn(async () => saved),
  } as unknown as CockpitClient;
  const onOpenItem = vi.fn();
  const onClose = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    // The dialog renders on the document body, outside its React host.
    await act(async () => root.render(<AddContextDialog client={client} onClose={onClose} onOpenItem={onOpenItem} />));
    await advance(0);
    const input = document.body.querySelector<HTMLInputElement>("input[type='text']")!;
    expect(document.activeElement).toBe(input);
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, "OPS-311");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await advance(450);
    expect(client.libraryResolve).toHaveBeenCalledWith({ input: "https://jira.test/jira/browse/OPS-311", provider_id: "jira" });
    expect(document.body.textContent).toContain("✓ Jira issue OPS-311 · Rotate signing keys · jira.test/jira");
    expect(document.body.querySelector(".library-destination")?.textContent).toBe("Library");
    expect(document.body.querySelectorAll("input[type='radio']")).toHaveLength(0);

    const primary = [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Add to Library")!;
    expect(primary.disabled).toBe(false);
    await act(async () => primary.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith(expect.objectContaining({ input: "https://jira.test/jira/browse/OPS-311", provider_id: "jira", target: null, follow_space: false, download_attachments: false }));
    expect(document.body.textContent).toContain("✓ Saved to Library");
    const open = [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Open in Library")!;
    expect(document.activeElement).toBe(open);
    await act(async () => open.click());
    expect(onOpenItem).toHaveBeenCalledWith("source:ops-311");
    expect(onClose).toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("keeps focus trapped during add startup and exposes a retry for the same rejected request", async () => {
  vi.useFakeTimers();
  const saved: LibraryOperation = {
    operation_id: "op-retry", kind: "add", phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:ops-311"],
    report: null, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  let rejectStart!: (cause: Error) => void;
  const firstStart = new Promise<LibraryOperation>((_resolve, reject) => { rejectStart = reject; });
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test/jira", executable: "jira" }] })),
    libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: "Rotate signing keys", canonical_id: "OPS-311", container_label: null, existing_item_id: null, existing_follow_id: null, page_count: null, git_working_tree: null, file_count: null, diagnostics: [] })),
    libraryAdd: vi.fn().mockReturnValueOnce(firstStart).mockResolvedValueOnce(saved),
  } as unknown as CockpitClient;
  const onClose = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={onClose} />));
    await advance(0);
    const input = document.body.querySelector<HTMLInputElement>("input[type='text']")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, "OPS-311");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await advance(450);
    await act(async () => document.body.querySelector<HTMLButtonElement>("button.setup-primary")!.click());
    const dialog = document.body.querySelector<HTMLElement>("[role='dialog']")!;
    expect(document.activeElement?.closest("[role='dialog']")).toBe(dialog);
    const close = [...dialog.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Close")!;
    expect(document.activeElement).toBe(close);
    await act(async () => {
      close.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    });
    expect(document.activeElement?.closest("[role='dialog']")).toBe(dialog);
    await act(async () => { rejectStart(new Error("provider is offline")); await firstStart.catch(() => undefined); });
    expect(dialog.querySelector('[role="alert"]')?.textContent).toContain("provider is offline");
    expect(document.activeElement).toBe(dialog.querySelector("button.setup-primary"));
    const retry = [...dialog.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Retry")!;
    await act(async () => retry.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledTimes(2);
    expect(client.libraryAdd).toHaveBeenNthCalledWith(1, expect.objectContaining({ input: "https://jira.test/jira/browse/OPS-311", provider_id: "jira" }));
    expect(client.libraryAdd).toHaveBeenNthCalledWith(2, expect.objectContaining({ input: "https://jira.test/jira/browse/OPS-311", provider_id: "jira" }));
    expect(document.activeElement?.closest("[role='dialog']")).toBe(dialog);
    await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(onClose).toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
