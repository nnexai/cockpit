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
    libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "github", provider_instance: "https://github.com", title: "Fix token refresh race", canonical_id: "acme/api#7", container_label: "acme/api", existing_item_id: null, existing_follow_id: null, page_count: null, git_working_tree: null, file_count: null, diagnostics: [] })),
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
  const resolution = { kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: "Rotate signing keys", canonical_id: "OPS-311", container_label: null, existing_item_id: "source:ops-311", existing_follow_id: null, page_count: null, git_working_tree: null, file_count: null, diagnostics: [] };
  const copied: LibraryOperation = {
    operation_id: "op-existing", kind: "space_add", phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:ops-311"],
    report: null, space: { space_id: "space-1", copy_mode: "copy", written: ["sources/jira/issue/ops-311.md"], skipped_edited: [], companion_root_id: "companion:c1" },
    target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test/jira", executable: "jira" }] })),
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
  libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "github", provider_instance: "https://github.com", title: "Fix token refresh race", canonical_id: "acme/api#7", container_label: "acme/api", existing_item_id: null, existing_follow_id: null, page_count: null, git_working_tree: null, file_count: null, diagnostics: [] })),
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
    libraryResolve: vi.fn(async () => ({ kind: "artifact", provider_id: "jira", provider_instance: "https://jira.test/jira", title: "Rotate signing keys", canonical_id: "OPS-311", container_label: null, existing_item_id: "source:ops-311", existing_follow_id: null, page_count: null, git_working_tree: null, file_count: null, diagnostics: [] })),
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
