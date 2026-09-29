// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryOperation } from "../../protocol/generated/v1";
import { AddContextDialog } from "./AddContextDialog";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

/** Token states of a Jira provider with no token stored in Cockpit. */
const jiraNotStored = { providers: [{ provider_id: "jira", state: "not_stored", kind: null, supported_kinds: ["bearer", "basic"] }] };
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
    providerCredentials: vi.fn(async () => jiraNotStored),
    libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: "Rotate signing keys", canonical_id: "OPS-311", container_label: null, existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [] })),
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
    expect(client.libraryAdd).toHaveBeenCalledWith(expect.objectContaining({ input: "https://jira.test/jira/browse/OPS-311", provider_id: "jira", target: null, follow: false, follow_mode: null, download_attachments: false }));
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
    providerCredentials: vi.fn(async () => jiraNotStored),
    libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: "Rotate signing keys", canonical_id: "OPS-311", container_label: null, existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [] })),
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

it("adds to the Library and the target Space, keeps a phase-2 failure across reopening, and retries only the Space copy", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space-1" };
  const space = { target, label: "api-review", live: true };
  const libraryPhase = (state: "running" | "done") => ({ phase: "library" as const, state, done: state === "done" ? 1 : 0, total: 1, message: null, error: null });
  const running: LibraryOperation = {
    operation_id: "op-space", kind: "add", phases: [libraryPhase("running"), { phase: "space", state: "pending", done: 0, total: null, message: null, error: null }], item_ids: [],
    report: null, space: null, target, cancel_requested: false, finished: false, created_at: "", updated_at: "",
  };
  const failed: LibraryOperation = {
    ...running, item_ids: ["source:pr-7"], finished: true,
    phases: [libraryPhase("done"), { phase: "space", state: "failed", done: 0, total: 1, message: null, error: { code: "source_companion_unavailable", message: "Exactly one verified companion is required" } }],
  };
  const copied: LibraryOperation = {
    operation_id: "op-space-retry", kind: "space_add", phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:pr-7"],
    report: null, space: { space_id: "space-1", copy_mode: "reflink", written: ["sources/github/review/acme-api-7.md"], skipped_edited: [], companion_root_id: "companion:c1" },
    target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "github", base_url: "https://github.com", executable: "gh" }] })),
    libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "github", provider_instance: "https://github.com", title: "Fix token refresh race", canonical_id: "acme/api#7", container_label: "acme/api", existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [] })),
    librarySpaceList: vi.fn(async () => ({ target, companion: { status: "available", companion_root_id: "companion:c1", companion_label: "Context" }, attempts: [], rows: [], behind: 0, diagnostics: [] })),
    libraryAdd: vi.fn(async () => running),
    libraryOperation: vi.fn(async () => failed),
    librarySpaceAdd: vi.fn(async () => copied),
  } as unknown as CockpitClient;
  const open = vi.fn();
  const onClose = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const dialog = () => <AddContextDialog client={client} onClose={onClose} space={space} defaultDestination="space" openInSpace={{ companionRootId: "companion:c1", open }} />;
  const button = (label: string) => [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((candidate) => candidate.textContent === label);
  try {
    await act(async () => root.render(dialog()));
    await advance(0);
    const radios = [...document.body.querySelectorAll<HTMLInputElement>("input[type='radio']")];
    expect(radios.map((radio) => radio.closest("label")?.textContent?.trim())).toEqual(["Library only", "Library and api-review"]);
    expect(radios[1]?.checked).toBe(true);
    const input = document.body.querySelector<HTMLInputElement>("input[type='text']")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "https://github.com/acme/api/pull/7");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await advance(450);
    await act(async () => button("Add to Library and api-review")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith(expect.objectContaining({ input: "https://github.com/acme/api/pull/7", target }));
    expect(document.body.textContent).toContain("Saving to Library…");

    // Closing does not cancel; reopening follows the same accepted add.
    await act(async () => root.render(null));
    expect(document.body.querySelector("[role='dialog']")).toBeNull();
    await act(async () => root.render(dialog()));
    await advance(0);
    expect(document.body.textContent).toContain("Saving to Library…");
    await advance(750);
    expect(document.body.textContent).toContain("✓ Saved to Library");
    const failure = document.body.querySelector("[role='alert']");
    expect(failure?.textContent).toBe("✕ Not added to api-review. Its context folder couldn't be verified. The Library copy is saved; nothing was written to api-review.");
    const retry = button("Retry adding to api-review")!;
    expect(document.activeElement).toBe(retry);
    expect(button("Open in api-review")).toBeUndefined();

    // A failure stays for the next opening too.
    await act(async () => root.render(null));
    await act(async () => root.render(dialog()));
    await advance(0);
    expect(document.activeElement).toBe(button("Retry adding to api-review"));

    await act(async () => button("Retry adding to api-review")!.click());
    await advance(0);
    expect(client.librarySpaceAdd).toHaveBeenCalledWith({ target, item_ids: ["source:pr-7"], follow_ids: [] });
    expect(client.libraryAdd).toHaveBeenCalledTimes(1);
    expect(client.libraryResolve).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain("✓ Added to api-review · reflinked");
    const openInSpace = button("Open in api-review")!;
    expect(document.activeElement).toBe(openInSpace);
    await act(async () => openInSpace.click());
    expect(open).toHaveBeenCalledWith("sources/github/review/acme-api-7.md");
    expect(onClose).toHaveBeenCalled();

    // A success that was shown is not shown again.
    await act(async () => root.render(null));
    await act(async () => root.render(dialog()));
    await advance(0);
    expect(document.body.querySelector("input[type='text']")).not.toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("offers only the Library when Herdr isn't live, and copies an already saved item without a provider fetch", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space-1" };
  const resolution = { kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: "Rotate signing keys", canonical_id: "OPS-311", container_label: null, existing_item_id: "source:ops-311", existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [] };
  const copied: LibraryOperation = {
    operation_id: "op-existing", kind: "space_add", phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:ops-311"],
    report: null, space: { space_id: "space-1", copy_mode: "copy", written: ["sources/jira/issue/ops-311.md"], skipped_edited: [], companion_root_id: "companion:c1" },
    target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test/jira", executable: "jira" }] })),
    providerCredentials: vi.fn(async () => jiraNotStored),
    libraryResolve: vi.fn(async () => resolution),
    librarySpaceList: vi.fn(async () => ({ target, companion: { status: "available", companion_root_id: "companion:c1", companion_label: "Context" }, attempts: [], rows: [], behind: 0, diagnostics: [] })),
    libraryAdd: vi.fn(),
    librarySpaceAdd: vi.fn(async () => copied),
  } as unknown as CockpitClient;
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const type = async (value: string) => {
    const input = document.body.querySelector<HTMLInputElement>("input[type='text']")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await advance(450);
  };
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} space={{ target, label: "api-review", live: false }} defaultDestination="space" />));
    await advance(0);
    expect(document.body.querySelectorAll("input[type='radio']")).toHaveLength(0);
    expect(document.body.textContent).toContain("Herdr isn't live, so this adds to the Library only.");
    expect(client.librarySpaceList).not.toHaveBeenCalled();
    await type("OPS-311");
    const disabled = [...document.body.querySelectorAll<HTMLButtonElement>("button.setup-primary")].find((candidate) => candidate.textContent === "Already in Library");
    expect(disabled?.disabled).toBe(true);
    await act(async () => root.unmount());

    const live = createRoot(host);
    await act(async () => live.render(<AddContextDialog client={client} onClose={vi.fn()} space={{ target, label: "api-review", live: true }} defaultDestination="space" />));
    await advance(0);
    await type("OPS-311");
    const primary = document.body.querySelector<HTMLButtonElement>("button.setup-primary")!;
    expect(primary.textContent).toBe("Add to api-review");
    await act(async () => primary.click());
    await advance(0);
    expect(client.librarySpaceAdd).toHaveBeenCalledWith({ target, item_ids: ["source:ops-311"], follow_ids: [] });
    expect(client.libraryAdd).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("✓ Added to api-review · copied (reflink not supported here)");
    await act(async () => live.unmount());
  } finally {
    host.remove();
  }
});

const githubClient = (overrides: Partial<Record<keyof CockpitClient, unknown>>) => ({
  projectConfiguration: vi.fn(async () => ({ providers: [{ id: "github", base_url: "https://github.com", executable: "gh" }] })),
  libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "github", provider_instance: "https://github.com", title: "Fix token refresh race", canonical_id: "acme/api#7", container_label: "acme/api", existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [] })),
  librarySpaceList: vi.fn(async (request: { target: { session_id: string; space_id: string } }) => ({ target: request.target, companion: { status: "available", companion_root_id: "companion:c1", companion_label: "Context" }, attempts: [], rows: [], behind: 0, diagnostics: [] })),
  ...overrides,
}) as unknown as CockpitClient;

async function typeSource(value: string): Promise<void> {
  const input = document.body.querySelector<HTMLInputElement>("input[type='text']")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await advance(450);
}

function dialogButton(label: string): HTMLButtonElement | undefined {
  return [...document.body.querySelectorAll<HTMLButtonElement>("[role='dialog'] button")].find((candidate) => candidate.textContent === label);
}

it("follows an add closed before it was accepted, and keeps what a cancelled add saved and copied", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space-1" };
  let accept!: (operation: LibraryOperation) => void;
  const cancelledAfterSave: LibraryOperation = {
    operation_id: "op-cancelled-after-save", kind: "add", item_ids: ["source:pr-7"], report: null, target, cancel_requested: true, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "library", state: "cancelled", done: 1, total: 2, message: null, error: null }, { phase: "space", state: "done", done: 1, total: 1, message: null, error: null }],
    space: { space_id: "space-1", copy_mode: "reflink", written: ["sources/github/review/acme-api-7.md"], skipped_edited: [], companion_root_id: "companion:c1" },
  };
  const client = githubClient({ libraryAdd: vi.fn(() => new Promise<LibraryOperation>((resolve) => { accept = resolve; })) });
  const open = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const dialog = () => <AddContextDialog client={client} onClose={vi.fn()} space={{ target, label: "api-review", live: true }} defaultDestination="space" openInSpace={{ companionRootId: "companion:c1", open }} />;
  try {
    await act(async () => root.render(dialog()));
    await advance(0);
    await typeSource("https://github.com/acme/api/pull/7");
    await act(async () => dialogButton("Add to Library and api-review")!.click());
    await act(async () => root.render(null));
    await act(async () => root.render(dialog()));
    await advance(0);
    // Still starting: the reopened dialog shows progress, not an empty form.
    expect(document.body.querySelector("input[type='text']")).toBeNull();
    expect(document.body.textContent).toContain("Saving to Library…");
    expect(document.activeElement).toBe(dialogButton("Close"));

    await act(async () => { accept(cancelledAfterSave); });
    await advance(0);
    expect(document.body.textContent).toContain("◐ Saved to Library, then cancelled. Anything not yet saved wasn't added.");
    expect(document.body.textContent).toContain("✓ Added to api-review · reflinked");
    expect(document.body.textContent).not.toContain("Nothing was added");
    expect(dialogButton("Retry")).toBeDefined();
    expect(dialogButton("Add another")).toBeDefined();
    const openInSpace = dialogButton("Open in api-review")!;
    expect(document.activeElement).toBe(openInSpace);
    await act(async () => openInSpace.click());
    expect(open).toHaveBeenCalledWith("sources/github/review/acme-api-7.md");
    expect(client.libraryAdd).toHaveBeenCalledTimes(1);
  } finally {
    await act(async () => root.render(null));
    await act(async () => root.render(dialog()));
    await act(async () => dialogButton("Add another")?.click());
    await act(async () => root.unmount());
    host.remove();
  }
});

it("leaves focus on Cancel while progress polls arrive, and moves it only when the add finishes", async () => {
  vi.useFakeTimers();
  const running = (done: number): LibraryOperation => ({
    operation_id: "op-polled", kind: "add", item_ids: ["source:pr-7"], report: null, space: null, target: null, cancel_requested: false, finished: false, created_at: "", updated_at: "",
    phases: [{ phase: "library", state: "running", done, total: 3, message: null, error: null }],
  });
  const saved: LibraryOperation = { ...running(3), finished: true, phases: [{ phase: "library", state: "done", done: 3, total: 3, message: null, error: null }] };
  const client = githubClient({
    libraryAdd: vi.fn(async () => running(0)),
    libraryOperation: vi.fn().mockResolvedValueOnce(running(1)).mockResolvedValueOnce(running(2)).mockResolvedValueOnce(saved),
  });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} onOpenItem={vi.fn()} />));
    await advance(0);
    await typeSource("https://github.com/acme/api/pull/7");
    await act(async () => dialogButton("Add to Library")!.click());
    await advance(0);
    expect(document.activeElement).toBe(dialogButton("Close"));
    const cancel = dialogButton("Cancel")!;
    cancel.focus();
    await advance(750);
    await advance(750);
    expect(client.libraryOperation).toHaveBeenCalledTimes(2);
    expect(document.activeElement).toBe(cancel);
    await advance(750);
    expect(document.body.textContent).toContain("✓ Saved to Library");
    expect(document.activeElement).toBe(dialogButton("Open in Library"));
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("keeps a Library failure for retry across reopening, and Add another starts over with a new source", async () => {
  vi.useFakeTimers();
  const failedSave = (id: string): LibraryOperation => ({
    operation_id: id, kind: "add", item_ids: [], report: null, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "library", state: "failed", done: 0, total: 1, message: null, error: { code: "source_cli_failed", message: "gh exited with status 1" } }],
  });
  const client = githubClient({ libraryAdd: vi.fn().mockResolvedValueOnce(failedSave("op-save-failed-1")).mockResolvedValueOnce(failedSave("op-save-failed-2")) });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const dialog = () => <AddContextDialog client={client} onClose={vi.fn()} />;
  try {
    await act(async () => root.render(dialog()));
    await advance(0);
    await typeSource("https://github.com/acme/api/pull/7");
    await act(async () => dialogButton("Add to Library")!.click());
    await advance(0);
    expect(document.body.textContent).toContain("✕ Not saved. gh exited with status 1. Nothing was added.");
    expect(document.activeElement).toBe(dialogButton("Retry"));

    await act(async () => root.render(null));
    await act(async () => root.render(dialog()));
    await advance(0);
    expect(document.body.textContent).toContain("✕ Not saved. gh exited with status 1. Nothing was added.");
    await act(async () => dialogButton("Retry")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledTimes(2);
    expect(client.libraryAdd).toHaveBeenLastCalledWith(expect.objectContaining({ input: "https://github.com/acme/api/pull/7" }));

    await act(async () => dialogButton("Add another")!.click());
    await advance(16);
    const input = document.body.querySelector<HTMLInputElement>("input[type='text']")!;
    expect(input.value).toBe("");
    await act(async () => root.render(null));
    await act(async () => root.render(dialog()));
    await advance(0);
    // Starting over forgets the failure.
    expect(document.body.querySelector("input[type='text']")).not.toBeNull();
    expect(document.body.textContent).not.toContain("Not saved");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("tells a partial Space copy from one that wrote nothing, and still opens what it wrote", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space-1" };
  const partlyCopied: LibraryOperation = {
    operation_id: "op-partly-copied", kind: "add", item_ids: ["source:pr-7", "source:issue-3"], report: null, target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "library", state: "done", done: 2, total: 2, message: null, error: null }, { phase: "space", state: "failed", done: 2, total: 2, message: null, error: { code: "library_conflict", message: "Library source changed during copy" } }],
    space: { space_id: "space-1", copy_mode: "reflink", written: ["sources/github/review/acme-api-7.md"], skipped_edited: [], companion_root_id: "companion:c1" },
  };
  const retried: LibraryOperation = { ...partlyCopied, operation_id: "op-partly-retry", kind: "space_add", phases: [{ phase: "space", state: "done", done: 2, total: 2, message: null, error: null }], space: { ...partlyCopied.space!, written: ["sources/github/issue/acme-api-3.md"] } };
  const client = githubClient({ libraryAdd: vi.fn(async () => partlyCopied), librarySpaceAdd: vi.fn(async () => retried) });
  const open = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} space={{ target, label: "api-review", live: true }} defaultDestination="space" openInSpace={{ companionRootId: "companion:c1", open }} />));
    await advance(0);
    await typeSource("https://github.com/acme/api/pull/7");
    await act(async () => dialogButton("Add to Library and api-review")!.click());
    await advance(0);
    const failure = document.body.querySelector("[role='alert']");
    expect(failure?.textContent).toBe("✕ Only partly added to api-review: 1 file was copied before it stopped. Library source changed during copy. The Library copy is saved; retrying copies the rest.");
    expect([...document.body.querySelectorAll(".library-progress-files code")].map((code) => code.textContent)).toEqual(["sources/github/review/acme-api-7.md"]);
    expect(document.activeElement).toBe(dialogButton("Retry adding to api-review"));
    expect(dialogButton("Open in api-review")).toBeDefined();

    await act(async () => dialogButton("Retry adding to api-review")!.click());
    await advance(0);
    expect(client.librarySpaceAdd).toHaveBeenCalledWith({ target, item_ids: ["source:pr-7", "source:issue-3"], follow_ids: [] });
    expect(client.libraryAdd).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain("✓ Added to api-review · reflinked");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("retries an interrupted Space-only copy with the items it asked for, not the fewer it recorded", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space-1" };
  const interrupted: LibraryOperation = {
    operation_id: "op-space-interrupted", kind: "space_add", item_ids: [], report: null, space: null, target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "space", state: "failed", done: 0, total: 1, message: null, error: { code: "library_operation_interrupted", message: "Library worker stopped before recording completion" } }],
  };
  const copied: LibraryOperation = {
    ...interrupted, operation_id: "op-space-interrupted-retry", item_ids: ["source:ops-311"], phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }],
    space: { space_id: "space-1", copy_mode: "copy", written: ["sources/jira/issue/ops-311.md"], skipped_edited: [], companion_root_id: "companion:c1" },
  };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test/jira", executable: "jira" }] })),
    providerCredentials: vi.fn(async () => jiraNotStored),
    libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: "Rotate signing keys", canonical_id: "OPS-311", container_label: null, existing_item_id: "source:ops-311", existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [] })),
    librarySpaceList: vi.fn(async () => ({ target, companion: { status: "available", companion_root_id: "companion:c1", companion_label: "Context" }, attempts: [], rows: [], behind: 0, diagnostics: [] })),
    libraryAdd: vi.fn(),
    librarySpaceAdd: vi.fn().mockResolvedValueOnce(interrupted).mockResolvedValueOnce(copied),
  } as unknown as CockpitClient;
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} space={{ target, label: "api-review", live: true }} defaultDestination="space" />));
    await advance(0);
    await typeSource("OPS-311");
    await act(async () => dialogButton("Add to api-review")!.click());
    await advance(0);
    expect(document.body.querySelector("[role='alert']")?.textContent).toBe("✕ Not added to api-review. Library worker stopped before recording completion. The Library copy is saved.");
    await act(async () => dialogButton("Retry adding to api-review")!.click());
    await advance(0);
    expect(client.librarySpaceAdd).toHaveBeenNthCalledWith(2, { target, item_ids: ["source:ops-311"], follow_ids: [] });
    expect(client.libraryAdd).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("✓ Added to api-review · copied (reflink not supported here)");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("recognizes a typed folder path, defaults its label, and saves the renamed copy through Library progress", async () => {
  vi.useFakeTimers();
  const saved: LibraryOperation = {
    operation_id: "op-folder", kind: "add", phases: [{ phase: "library", state: "partial", done: 1, total: 1, message: null, error: null }],
    item_ids: ["folder:notes"], report: null, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const client = githubClient({
    libraryResolve: vi.fn(async () => ({ kind: "folder", provider_id: null, provider_instance: null, title: "notes", canonical_id: null, container_label: null,
      existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: true, file_count: 600, diagnostics: [] })),
    libraryAdd: vi.fn(async () => saved),
  });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onOpenItem = vi.fn();
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} onOpenItem={onOpenItem} />));
    await advance(0);
    await typeSource("~/notes");
    expect(client.libraryResolve).toHaveBeenCalledWith({ input: "~/notes", provider_id: null });
    expect(document.body.textContent).toContain("Folder · Git working tree");
    const labelElement = [...document.body.querySelectorAll("label")].find((element) => element.textContent === "Label")!;
    const label = document.getElementById(labelElement.htmlFor) as HTMLInputElement;
    expect(label.value).toBe("notes");
    expect(document.body.querySelector("input[type='checkbox']")).toBeNull();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(label, "Design notes");
      label.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      label.focus();
      label.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith({
      input: "~/notes", label: "Design notes", provider_id: null, target: null,
      reference_depth: 0, follow: false, follow_mode: null, download_attachments: false, refresh_existing: false,
    });
    expect(document.body.textContent).toContain("Saved to Library, partial");
    expect(document.activeElement).toBe(dialogButton("Open in Library"));
    await act(async () => dialogButton("Open in Library")!.click());
    expect(onOpenItem).toHaveBeenCalledWith("folder:notes");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

const CLOUD_PAGE = "https://nnexai.atlassian.net/wiki/spaces/SD/pages/98765/Release+checklist";
const DC_DISPLAY = "https://confluence.example.com/confluence/display/ENG/Release+Checklist";
const confluenceProviders = [
  { id: "cloud", base_url: "https://nnexai.atlassian.net/wiki", executable: "/opt/homebrew/bin/confluence", login: "default" },
  { id: "cloud-reader", base_url: "https://nnexai.atlassian.net/wiki/", executable: "confluence", login: "reader" },
  { id: "dc", base_url: "https://confluence.example.com/confluence", executable: "confluence", login: "dc" },
  { id: "github", base_url: "https://github.com", executable: "gh" },
];

function pageResolution(providerId: string | null) {
  const cloud = providerId !== "dc";
  return {
    kind: "confluence_page", provider_id: providerId, provider_instance: cloud ? "https://nnexai.atlassian.net/wiki" : "https://confluence.example.com/confluence",
    title: cloud ? "Release checklist" : "Release Checklist", canonical_id: cloud ? "98765" : "4242", container_label: cloud ? "SD · Software Development" : "ENG · Engineering",
    existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [],
  };
}

function providerSelect(): HTMLSelectElement | null {
  const label = [...document.body.querySelectorAll("label")].find((element) => element.textContent === "Provider");
  return label ? document.getElementById(label.htmlFor) as HTMLSelectElement : null;
}

it("recognizes a Cloud page link, asks for the provider only when several configured instances can read it, and adds through the chosen one", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space-1" };
  const saved: LibraryOperation = {
    operation_id: "op-page", kind: "add", item_ids: ["source:page-98765"], report: null, target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }, { phase: "space", state: "done", done: 1, total: 1, message: null, error: null }],
    space: { space_id: "space-1", copy_mode: "reflink", written: ["confluence/nnexai.atlassian.net/SD - Software Development/Release checklist/Release checklist.md"], skipped_edited: [], companion_root_id: "companion:c1" },
  };
  const client = githubClient({
    projectConfiguration: vi.fn(async () => ({ providers: confluenceProviders })),
    libraryResolve: vi.fn(async (request: { provider_id: string | null }) => pageResolution(request.provider_id)),
    libraryAdd: vi.fn(async () => saved),
  });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} space={{ target, label: "api-review", live: true }} defaultDestination="space" />));
    await advance(0);
    await typeSource(CLOUD_PAGE);
    // Both profiles of the Cloud site can read the link; the Data Center instance can't.
    expect(client.libraryResolve).toHaveBeenLastCalledWith({ input: CLOUD_PAGE, provider_id: "cloud" });
    expect([...providerSelect()!.options].map((option) => option.value)).toEqual(["cloud", "cloud-reader"]);
    expect(document.body.textContent).toContain("✓ Confluence page · Release checklist · SD");
    expect(document.body.textContent).toContain("SD · Software Development · nnexai.atlassian.net");
    expect(document.body.querySelector<HTMLInputElement>("input[type='checkbox']")?.checked).toBe(false);

    await act(async () => {
      const select = providerSelect()!;
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, "cloud-reader");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await advance(450);
    expect(client.libraryResolve).toHaveBeenLastCalledWith({ input: CLOUD_PAGE, provider_id: "cloud-reader" });
    expect(document.body.querySelector<HTMLInputElement>("input[type='text']")!.value).toBe(CLOUD_PAGE);

    // A bare id can be read by every configured Confluence instance; the pick and the typed id are kept.
    await typeSource("98765");
    expect([...providerSelect()!.options].map((option) => option.value)).toEqual(["cloud", "cloud-reader", "dc"]);
    expect(providerSelect()!.value).toBe("cloud-reader");
    expect(client.libraryResolve).toHaveBeenLastCalledWith({ input: "98765", provider_id: "cloud-reader" });
    // `Only this page` stays the default for a page; the destination keeps the Space.
    const radios = [...document.body.querySelectorAll<HTMLInputElement>("input[type='radio']")];
    expect(radios.map((radio) => radio.checked)).toEqual([true, false, false, true]);

    await act(async () => dialogButton("Add to Library and api-review")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith({
      input: "98765", provider_id: "cloud-reader", target, label: null,
      reference_depth: 0, follow: false, follow_mode: null, download_attachments: false, refresh_existing: false,
    });
    expect(document.body.textContent).toContain("✓ Saved to Library");
    expect(document.body.textContent).not.toContain("@");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("refuses a bare page id without a Confluence provider, and resolves ids and Data Center links through the only one", async () => {
  vi.useFakeTimers();
  const unconfigured = githubClient({});
  const host = document.createElement("div");
  document.body.append(host);
  let root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={unconfigured} onClose={vi.fn()} />));
    await advance(0);
    await typeSource("123456");
    expect(unconfigured.libraryResolve).not.toHaveBeenCalled();
    expect(document.body.querySelector("[role='alert']")?.textContent).toContain("✕ No Confluence provider configured");
    expect(document.body.querySelector<HTMLInputElement>("input[type='text']")!.getAttribute("aria-invalid")).toBe("true");
    expect(dialogButton("Add to Library")!.disabled).toBe(true);
    await act(async () => root.unmount());

    const saved: LibraryOperation = {
      operation_id: "op-dc", kind: "add", phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:page-4242"],
      report: null, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    };
    const client = githubClient({
      projectConfiguration: vi.fn(async () => ({ providers: [confluenceProviders[2]] })),
      libraryResolve: vi.fn(async (request: { provider_id: string | null }) => pageResolution(request.provider_id)),
      libraryAdd: vi.fn(async () => saved),
    });
    root = createRoot(host);
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} onOpenItem={vi.fn()} />));
    await advance(0);
    await typeSource("4242");
    expect(client.libraryResolve).toHaveBeenLastCalledWith({ input: "4242", provider_id: "dc" });
    expect(providerSelect()).toBeNull();
    expect(document.body.textContent).toContain("✓ Confluence page · Release Checklist · ENG");
    expect(document.body.textContent).toContain("ENG · Engineering · confluence.example.com/confluence");
    await typeSource(DC_DISPLAY);
    expect(client.libraryResolve).toHaveBeenLastCalledWith({ input: DC_DISPLAY, provider_id: "dc" });
    const input = document.body.querySelector<HTMLInputElement>("input[type='text']")!;
    await act(async () => {
      input.focus();
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith(expect.objectContaining({ input: DC_DISPLAY, provider_id: "dc", target: null, follow: false, follow_mode: null, download_attachments: false }));
    expect(document.activeElement).toBe(dialogButton("Open in Library"));
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("explains a Confluence sign-in failure and a missing confluence CLI without creating an item", async () => {
  vi.useFakeTimers();
  const client = githubClient({
    projectConfiguration: vi.fn(async () => ({ providers: confluenceProviders.slice(2) })),
    libraryResolve: vi.fn()
      .mockRejectedValueOnce(Object.assign(new Error("Confluence rejected the profile"), { code: "source_auth_failed" }))
      .mockRejectedValueOnce(Object.assign(new Error("confluence executable not found"), { code: "source_cli_unavailable" })),
    libraryAdd: vi.fn(),
  });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} />));
    await advance(0);
    await typeSource(DC_DISPLAY);
    const alert = () => document.body.querySelector("[role='alert']")!;
    expect(alert().querySelector("strong")?.textContent).toBe("✕ Confluence sign-in failed");
    expect(alert().textContent).toContain("confluence.example.com rejected the credentials for the confluence CLI. Store a token for this site in Cockpit, or sign in with the CLI's read-only profile, then retry.");
    await act(async () => dialogButton("Retry lookup")!.click());
    await advance(450);
    expect(alert().querySelector("strong")?.textContent).toBe("✕ confluence isn't installed");
    expect(alert().textContent).toContain("Install it with brew install pchuri/tap/confluence-cli, configure a read-only profile, then retry.");
    expect(dialogButton("Add to Library")!.disabled).toBe(true);
    expect(client.libraryAdd).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

function spaceResolution(overrides: Record<string, unknown>) {
  return {
    kind: "confluence_space", provider_id: "cloud", provider_instance: "https://nnexai.atlassian.net/wiki", title: "Software Development", canonical_id: "SD",
    container_label: "SD · Software Development", existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [],
    ...overrides,
  };
}

it("browses each Confluence provider's spaces, keeps a provider's sign-in failure to itself, and follows a chosen space into the Library and the Space", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space-1" };
  const followed: LibraryOperation = {
    operation_id: "op-follow", kind: "add", item_ids: ["source:h", "source:a", "source:t"], report: null, target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "library", state: "done", done: 3, total: 3, message: null, error: null }, { phase: "space", state: "done", done: 3, total: 3, message: null, error: null }],
    space: { space_id: "space-1", copy_mode: "reflink", written: ["confluence/nnexai.atlassian.net/SD - Software Development/Home/Home.md", "confluence/nnexai.atlassian.net/SD - Software Development/Home/Article/Article.md", "confluence/nnexai.atlassian.net/SD - Software Development/Topic/Topic.md"], skipped_edited: [], companion_root_id: "companion:c1" },
  };
  const client = githubClient({
    projectConfiguration: vi.fn(async () => ({ providers: [confluenceProviders[0], confluenceProviders[2]], limits: { library_space_pages: 200 } })),
    libraryConfluenceSpaces: vi.fn(async (request: { provider_id: string }) => {
      if (request.provider_id === "dc") throw Object.assign(new Error("Confluence rejected the profile"), { code: "source_auth_failed" });
      return [spaceResolution({}), spaceResolution({ title: "Operations", canonical_id: "OPS", container_label: "OPS · Operations", existing_follow_id: "follow:ops" })];
    }),
    libraryAdd: vi.fn(async () => followed),
  });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const button = (label: string) => [...document.body.querySelectorAll<HTMLButtonElement>("[role='dialog'] button")].find((candidate) => candidate.textContent?.trim() === label || candidate.getAttribute("aria-label") === label);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} onOpenItem={vi.fn()} space={{ target, label: "api-review", live: true }} defaultDestination="space" />));
    await advance(0);
    // Collapsed until asked: nothing is listed before the disclosure opens.
    const disclosure = button("Browse Confluence spaces")!;
    expect(disclosure.getAttribute("aria-expanded")).toBe("false");
    expect(client.libraryConfluenceSpaces).not.toHaveBeenCalled();
    await act(async () => disclosure.click());
    await advance(0);
    expect(disclosure.getAttribute("aria-expanded")).toBe("true");
    expect(client.libraryConfluenceSpaces).toHaveBeenCalledWith({ provider_id: "cloud" });
    expect(client.libraryConfluenceSpaces).toHaveBeenCalledWith({ provider_id: "dc" });
    const cloud = document.body.querySelector("[aria-label='Confluence · nnexai.atlassian.net']")!;
    expect([...cloud.querySelectorAll("li")].map((row) => row.textContent)).toEqual(["SD · Software DevelopmentFollow", "OPS · OperationsFollowingSelect"]);
    const dc = document.body.querySelector("[aria-label='Confluence · confluence.example.com/confluence']")!;
    expect(dc.querySelector("[role='alert'] strong")?.textContent).toBe("✕ Confluence sign-in failed");
    expect(dc.querySelector("li")).toBeNull();
    await act(async () => [...dc.querySelectorAll("button")].find((candidate) => candidate.textContent === "Retry")!.click());
    await advance(0);
    expect(client.libraryConfluenceSpaces).toHaveBeenCalledTimes(3);
    expect(client.libraryConfluenceSpaces).toHaveBeenLastCalledWith({ provider_id: "dc" });

    // Choosing a space fills Add with it without another lookup, and focus moves to the action it enables.
    await act(async () => button("Follow SD · Software Development")!.click());
    await advance(450);
    expect(client.libraryResolve).not.toHaveBeenCalled();
    expect(document.body.querySelector<HTMLInputElement>("input[type='text']")!.value).toBe("SD");
    expect(disclosure.getAttribute("aria-expanded")).toBe("false");
    expect(document.body.textContent).toContain("✓ Confluence space · SD · Software Development");
    expect(document.body.textContent).toContain("Follow the whole space (SD · Software Development)");
    expect(document.body.textContent).toContain("including every top-level page tree");
    const primary = button("Follow and add to api-review")!;
    expect(primary.disabled).toBe(false);
    expect(document.activeElement).toBe(primary);
    // Following is the only option for a space; attachment downloads require an explicit opt-in.
    expect([...document.body.querySelectorAll<HTMLInputElement>("input[type='radio']")].map((radio) => radio.checked)).toEqual([false, true]);
    const downloadAttachments = document.body.querySelector<HTMLInputElement>("input[type='checkbox']")!;
    expect(downloadAttachments.checked).toBe(false);
    await act(async () => downloadAttachments.click());

    await act(async () => primary.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith({
      input: "SD", provider_id: "cloud", target, label: null,
      reference_depth: 0, follow: true, follow_mode: null, download_attachments: true, refresh_existing: false,
    });
    expect(document.body.textContent).toContain("✓ Saved to Library · 3 pages");
    expect(document.body.textContent).toContain("✓ Added to api-review · reflinked");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("offers following a page's whole space, warns when the page limit makes it partial, and adds an already followed space to the Space without refetching", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space-1" };
  const saved: LibraryOperation = {
    operation_id: "op-follow-page", kind: "add", item_ids: ["source:page-98765"], report: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "library", state: "partial", done: 200, total: 312, message: null, error: null }], space: null,
  };
  const copied: LibraryOperation = {
    operation_id: "op-follow-space", kind: "space_add", item_ids: [], report: null, target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }],
    space: { space_id: "space-1", copy_mode: "copy", written: ["confluence/nnexai.atlassian.net/SD - Software Development/Home/Home.md"], skipped_edited: [], companion_root_id: "companion:c1" },
  };
  const client = githubClient({
    projectConfiguration: vi.fn(async () => ({ providers: [confluenceProviders[0]], limits: { library_space_pages: 200 } })),
    libraryResolve: vi.fn(async (request: { input: string }) => request.input === CLOUD_PAGE
      ? { ...pageResolution("cloud"), item_count: 312 }
      : spaceResolution({ existing_follow_id: "follow:sd", item_count: 38 })),
    libraryAdd: vi.fn(async () => saved),
    librarySpaceAdd: vi.fn(async () => copied),
  });
  const host = document.createElement("div");
  document.body.append(host);
  let root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} onOpenItem={vi.fn()} />));
    await advance(0);
    await typeSource(CLOUD_PAGE);
    const choice = [...document.body.querySelectorAll<HTMLInputElement>("input[type='radio']")];
    expect(choice.map((radio) => radio.parentElement?.textContent?.trim())).toEqual(["Only this page", "Follow the whole space (SD · Software Development · 312 pages)"]);
    expect(choice[0]!.checked).toBe(true);
    expect(dialogButton("Add to Library")!.disabled).toBe(false);
    expect(document.body.textContent).not.toContain("page limit");
    await act(async () => choice[1]!.click());
    expect(document.body.querySelector("[role='status'].library-note-partial")?.textContent).toContain("The page limit is 200, so this saves 200 of 312 pages. Refresh won't mark pages removed at source until the whole space fits.");
    await act(async () => dialogButton("Follow space")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith(expect.objectContaining({ input: CLOUD_PAGE, provider_id: "cloud", target: null, follow: true, follow_mode: null, download_attachments: false }));
    expect(document.body.textContent).toContain("◐ Saved to Library, partial");
    await act(async () => root.unmount());

    // A space that is already followed is copied into the Space as a follow, with no Library step.
    root = createRoot(host);
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} space={{ target, label: "api-review", live: true }} defaultDestination="space" />));
    await advance(0);
    await typeSource("https://nnexai.atlassian.net/wiki/spaces/SD");
    expect(client.libraryResolve).toHaveBeenLastCalledWith({ input: "https://nnexai.atlassian.net/wiki/spaces/SD", provider_id: "cloud" });
    expect(document.body.textContent).toContain("✓ Already following · SD · Software Development");
    expect(document.body.textContent).not.toContain("including every top-level page tree");
    await act(async () => dialogButton("Add to api-review")!.click());
    await advance(0);
    expect(client.librarySpaceAdd).toHaveBeenCalledWith({ target, item_ids: [], follow_ids: ["follow:sd"] });
    expect(client.libraryAdd).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain("✓ Added to api-review · copied (reflink not supported here)");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("downloads attachments for a page only after its checkbox is checked", async () => {
  vi.useFakeTimers();
  const saved: LibraryOperation = {
    operation_id: "op-page-attachments", kind: "add", item_ids: ["source:page-98765"], report: null, target: null, space: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }],
  };
  const client = githubClient({
    projectConfiguration: vi.fn(async () => ({ providers: confluenceProviders, limits: { library_attachment_bytes: 25 * 1024 * 1024 } })),
    libraryResolve: vi.fn(async () => pageResolution("cloud")),
    libraryAdd: vi.fn(async () => saved),
  });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} />));
    await advance(0); await typeSource(CLOUD_PAGE);
    const checkbox = document.body.querySelector<HTMLInputElement>("input[type='checkbox']")!;
    expect(checkbox.checked).toBe(false);
    expect(checkbox.parentElement?.textContent).toContain("Download attachments (up to 25 MB each)");
    await act(async () => checkbox.click());
    await act(async () => dialogButton("Add to Library")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith(expect.objectContaining({ follow: false, follow_mode: null, download_attachments: true }));
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("follows a Jira query: a relative-date query preselects accumulate, shows the count, hides the Space destination and sends follow and follow_mode", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space-1" };
  const jql = "project = OPS AND updated >= -7d";
  const saved: LibraryOperation = {
    operation_id: "op-query", kind: "add", phases: [{ phase: "library", state: "done", done: 2, total: 2, message: null, error: null }], item_ids: ["source:ops-1", "source:ops-2"],
    report: null, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const client = githubClient({
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test/jira", executable: "jira" }] })),
    providerCredentials: vi.fn(async () => jiraNotStored),
    libraryResolve: vi.fn(async () => ({
      kind: "jira_query", provider_id: "jira", provider_instance: "https://jira.test/jira", title: jql, canonical_id: jql, container_label: null, existing_item_id: null, existing_follow_id: null,
      item_count: 100, item_count_exact: false, follow_mode: "accumulate", git_working_tree: null, file_count: null, diagnostics: [],
    })),
    libraryAdd: vi.fn(async () => saved),
  });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} space={{ target, label: "api-review", live: true }} defaultDestination="space" />));
    await advance(0); await typeSource(jql);
    await advance(450);
    expect(client.libraryResolve).toHaveBeenCalledWith({ input: jql, provider_id: "jira" });
    expect(document.body.textContent).toContain(`✓ Jira query · ${jql} · 100+ issues`);
    expect(document.body.textContent).toContain("This query uses relative dates");
    expect([...document.body.querySelectorAll<HTMLInputElement>("input[type='radio']")].map((radio) => [radio.parentElement?.textContent?.trim(), radio.checked]))
      .toEqual([["Live — mirror the query", false], ["Accumulate — keep every issue that ever matched", true]]);
    // Jira follows can't go into a Space yet, even with a live target Space.
    expect(document.body.textContent).toContain("Jira follows can't be added to a Space yet");
    expect(document.body.textContent).not.toContain("Library and api-review");
    expect(depthSelect()!.value).toBe("0");
    await chooseDepth(depthSelect()!, "2");
    await act(async () => dialogButton("Follow query")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith(expect.objectContaining({ input: jql, provider_id: "jira", follow: true, follow_mode: "accumulate", reference_depth: 2, target: null, download_attachments: false }));

    // A preset fills the field; a bare key reads as a query on the Jira provider.
    await act(async () => dialogButton("Add another")!.click());
    await act(async () => [...document.body.querySelectorAll<HTMLButtonElement>(".library-query-presets button")].find((chip) => chip.textContent === "Assigned to me, unresolved")!.click());
    expect(document.body.querySelector<HTMLInputElement>("input[type='text']")!.value).toBe("assignee = currentUser() AND resolution = Unresolved");
  } finally { await act(async () => root.unmount()); host.remove(); }
});

function depthSelect(): HTMLSelectElement | null {
  const label = [...document.body.querySelectorAll("label")].find((element) => element.textContent === "Follow references");
  return label ? document.getElementById(label.htmlFor) as HTMLSelectElement : null;
}

async function chooseDepth(select: HTMLSelectElement, value: string): Promise<void> {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, value);
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

it("offers Follow references with per-source defaults, honors a stored depth, and forgets the choice when the source changes", async () => {
  vi.useFakeTimers();
  const saved: LibraryOperation = {
    operation_id: "op-depth", kind: "add", phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:ops-1"],
    report: null, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const jiraIssue = (key: string, extra: Record<string, unknown> = {}) => ({ kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: key, canonical_id: key, container_label: null, existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [], ...extra });
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test/jira", executable: "jira" }] })),
    providerCredentials: vi.fn(async () => jiraNotStored),
    libraryResolve: vi.fn(async ({ input }: { input: string }) => input.endsWith("OPS-2") ? jiraIssue("OPS-2", { reference_depth: 2 }) : input.includes("browse/OPS-3") ? jiraIssue("OPS-3", { existing_item_id: "source:ops-3", reference_depth: 3 }) : jiraIssue("OPS-1")),
    libraryAdd: vi.fn(async () => saved),
  } as unknown as CockpitClient;
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} />));
    await advance(0); await typeSource("OPS-1"); await advance(450);
    // A new Jira ticket follows one step by default.
    expect(depthSelect()!.value).toBe("1");
    await chooseDepth(depthSelect()!, "3");
    await act(async () => dialogButton("Add to Library")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenLastCalledWith(expect.objectContaining({ input: "https://jira.test/jira/browse/OPS-1", reference_depth: 3 }));

    // Add another forgets the pick; a stored depth wins over the default until the user changes it.
    await act(async () => dialogButton("Add another")!.click());
    await advance(0); await typeSource("OPS-1"); await advance(450);
    expect(depthSelect()!.value).toBe("1");
    await chooseDepth(depthSelect()!, "0");
    await typeSource("OPS-2"); await advance(450);
    expect(depthSelect()!.value).toBe("2");
    await act(async () => dialogButton("Add to Library")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenLastCalledWith(expect.objectContaining({ input: "https://jira.test/jira/browse/OPS-2", reference_depth: 2 }));

    // An item already in the Library applies its depth only when refreshed.
    await act(async () => dialogButton("Add another")!.click());
    await advance(0); await typeSource("OPS-3"); await advance(450);
    expect(depthSelect()).toBeNull();
    await act(async () => document.body.querySelector<HTMLInputElement>("input[type='checkbox']")!.click());
    expect(depthSelect()!.value).toBe("3");
    await act(async () => dialogButton("Refresh from source")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenLastCalledWith(expect.objectContaining({ input: "https://jira.test/jira/browse/OPS-3", refresh_existing: true, reference_depth: 3 }));
  } finally { await act(async () => root.unmount()); host.remove(); }
});

const jiraProviderConfig = { providers: [{ id: "jira", base_url: "https://jira.test/jira", executable: "jira" }] };
const jiraResolution = { kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: "Rotate signing keys", canonical_id: "OPS-311", container_label: null, existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [] };

it("offers the provider token dialog from a credential failure, with the entry form open for the failing provider", async () => {
  vi.useFakeTimers();
  const client = {
    projectConfiguration: vi.fn(async () => jiraProviderConfig),
    providerCredentials: vi.fn(async () => jiraNotStored),
    libraryResolve: vi.fn(async () => { throw Object.assign(new Error("Downloading needs a token stored in Cockpit."), { code: "source_credential_required" }); }),
  } as unknown as CockpitClient;
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} />));
    await advance(0); await typeSource("OPS-311");
    const alert = document.body.querySelector("[role='alert']")!;
    expect(alert.querySelector("strong")?.textContent).toBe("✕ A token is needed");
    expect(alert.textContent).toContain("Downloading needs a token stored in Cockpit.");
    await act(async () => dialogButton("Store a token…")!.click());
    await advance(0);
    expect([...document.body.querySelectorAll("h2")].map((heading) => heading.textContent)).toContain("Provider tokens");
    expect(document.body.querySelector("form[aria-label='Token for Jira · jira.test/jira'] input[type='password']")).not.toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it.each([
  { stored: false, offered: false },
  { stored: true, offered: true },
])("offers Download attachments for a new Jira issue only with a stored token ($stored)", async ({ stored, offered }) => {
  vi.useFakeTimers();
  const saved: LibraryOperation = {
    operation_id: "op-jira-attachments", kind: "add", phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:ops-311"],
    report: null, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const client = {
    projectConfiguration: vi.fn(async () => ({ ...jiraProviderConfig, limits: { library_attachment_bytes: 25 * 1024 * 1024 } })),
    providerCredentials: vi.fn(async () => ({ providers: [{ provider_id: "jira", state: stored ? "stored" : "not_stored", kind: stored ? "bearer" : null, supported_kinds: ["bearer", "basic"] }] })),
    libraryResolve: vi.fn(async () => jiraResolution),
    libraryAdd: vi.fn(async () => saved),
  } as unknown as CockpitClient;
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} />));
    await advance(0); await typeSource("OPS-311"); await advance(0);
    const checkbox = [...document.body.querySelectorAll<HTMLLabelElement>("label.task-setup-check")].find((label) => label.textContent?.includes("Download attachments"));
    expect(checkbox !== undefined).toBe(offered);
    expect(document.body.textContent?.includes("Downloading Jira attachments needs a token stored in Cockpit.")).toBe(!offered);
    if (checkbox) await act(async () => checkbox.querySelector("input")!.click());
    await act(async () => dialogButton("Add to Library")!.click());
    await advance(0);
    expect(client.libraryAdd).toHaveBeenCalledWith(expect.objectContaining({ download_attachments: offered }));
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("sends the opt-in when following a Jira query with a stored token and hides it without one", async () => {
  vi.useFakeTimers();
  const jql = "project = OPS";
  const saved: LibraryOperation = {
    operation_id: "op-query-attachments", kind: "add", phases: [{ phase: "library", state: "done", done: 2, total: 2, message: null, error: null }], item_ids: ["source:ops-1"],
    report: null, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  for (const stored of [false, true]) {
    const client = {
      projectConfiguration: vi.fn(async () => ({ ...jiraProviderConfig, limits: { library_attachment_bytes: 25 * 1024 * 1024 } })),
      providerCredentials: vi.fn(async () => ({ providers: [{ provider_id: "jira", state: stored ? "stored" : "not_stored", kind: stored ? "bearer" : null, supported_kinds: ["bearer", "basic"] }] })),
      libraryResolve: vi.fn(async () => ({
        kind: "jira_query", provider_id: "jira", provider_instance: "https://jira.test/jira", title: jql, canonical_id: jql, container_label: null, existing_item_id: null, existing_follow_id: null,
        item_count: 2, item_count_exact: true, follow_mode: "live", git_working_tree: null, file_count: null, diagnostics: [],
      })),
      libraryAdd: vi.fn(async () => saved),
    } as unknown as CockpitClient;
    const host = document.createElement("div"); document.body.append(host);
    const root = createRoot(host);
    try {
      await act(async () => root.render(<AddContextDialog client={client} onClose={vi.fn()} />));
      await advance(0); await typeSource(jql); await advance(450);
      const checkbox = [...document.body.querySelectorAll<HTMLLabelElement>("label.task-setup-check")].find((label) => label.textContent?.includes("Download attachments"));
      expect(checkbox !== undefined).toBe(stored);
      if (checkbox) await act(async () => checkbox.querySelector("input")!.click());
      await act(async () => dialogButton("Follow query")!.click());
      await advance(0);
      expect(client.libraryAdd).toHaveBeenCalledWith(expect.objectContaining({ input: jql, follow: true, download_attachments: stored }));
    } finally { await act(async () => root.unmount()); host.remove(); }
  }
});
