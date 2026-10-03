// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { RepositoryCandidate, SpaceContextListing } from "../../protocol/generated/v1";
import { useSpaceContextListing } from "../library/useLibraryOperation";
import { ContextResources } from "./ContextResources";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

it("reads context directly from the Library without a context root and keeps the resource dialog keyboard contract", async () => {
  const host = document.createElement("div");
  const opener = document.createElement("button");
  document.body.append(opener, host);
  opener.focus();
  const root = createRoot(host);
  const onAdd = vi.fn();
  const onClose = vi.fn();
  const target = { session_id: "session", space_id: "space" };
  try {
    await act(async () => root.render(<ContextResources client={{ projectConfiguration: vi.fn(async () => ({ providers: [] })) } as unknown as CockpitClient}
      space={{ target, label: "Review", live: true }}
      spaceListing={{ status: "ready", error: null, received: 1, reload: vi.fn(), listing: { target, space_label: "Review", library_root: "/data/library", checkout_path: "/repo", items: [], repository_paths: [], diagnostics: [] } }}
      onAdd={onAdd} onClose={onClose} />));
    const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(document.activeElement).toBe(dialog.querySelector('button[aria-label="Close Context resources"]'));
    expect(dialog.querySelector("select")).toBeNull();
    expect(dialog.textContent).not.toMatch(/snapshot/i);
    expect(dialog.textContent).toContain("Context is read directly from the Library, so Library refreshes show up here immediately.");
    expect(dialog.textContent).toContain("The Library is managed by Cockpit and read-only by convention. Notes live in your checkout.");
    expect(dialog.textContent).toContain("Library context for Review");
    expect(dialog.textContent).toContain("Nothing selected for Review yet.");
    expect(dialog.textContent).toContain("No extra repositories. The Space's own checkout is always included.");
    const add = [...dialog.querySelectorAll("button")].find((button) => button.textContent === "Add…")!;
    await act(async () => add.click());
    expect(onAdd).toHaveBeenCalledOnce();
    const buttons = [...dialog.querySelectorAll<HTMLButtonElement>("button:not([disabled])")];
    await act(async () => {
      buttons[buttons.length - 1]!.focus();
      buttons[buttons.length - 1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    });
    expect(document.activeElement).toBe(buttons[0]);
    await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(onClose).toHaveBeenCalledOnce();
    await act(async () => root.unmount());
    expect(document.activeElement).toBe(opener);
  } finally {
    host.remove();
    opener.remove();
  }
});

it("explains Library-only context and disables repository additions when Herdr is not live", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const target = { session_id: "session", space_id: "space" };
  try {
    await act(async () => root.render(<ContextResources
      client={{ projectConfiguration: vi.fn(async () => ({ providers: [] })) } as unknown as CockpitClient}
      space={{ target, label: "Review", live: false }}
      spaceListing={{ status: "ready", error: null, received: 1, reload: vi.fn(), listing: { target, space_label: "Review", library_root: "/data/library", checkout_path: null, items: [], repository_paths: [], diagnostics: [] } }}
      onAdd={vi.fn()} onClose={vi.fn()} />));
    const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(dialog.textContent).toContain("Herdr isn't live, so Review's context is Library-only.");
    const addRepositories = [...dialog.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Add repositories…");
    expect(addRepositories === undefined || addRepositories.disabled).toBe(true);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("selects catalog repository paths, restores picker focus on Escape, and retains a path when removal fails", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const target = { session_id: "session", space_id: "space" };
  const catalog: RepositoryCandidate[] = ["/repo", "/existing", "/first", "/second"].map((path) => ({
    repository_id: path, name: path.slice(1), root: path, checkout_path: path, common_dir: `${path}/.git`, branch: "main",
    is_linked_worktree: false, is_detached: false, provenance: "catalog",
  }));
  let listing: SpaceContextListing = { target, space_label: "Review", library_root: "/data/library", checkout_path: "/repo", items: [], repository_paths: ["/existing"], diagnostics: [] };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [] })),
    librarySpaceList: vi.fn(async () => listing),
    repositories: vi.fn(async () => ({ repositories: catalog, diagnostics: [] })),
    librarySpaceRepositories: vi.fn(async (request: { repository_paths: string[] }) => {
      listing = { ...listing, repository_paths: request.repository_paths };
      return listing;
    }),
  } as unknown as CockpitClient;
  const onClose = vi.fn();
  function Harness() {
    const spaceListing = useSpaceContextListing(client, target, true);
    return <ContextResources client={client} space={{ target, label: "Review", live: true }} spaceListing={spaceListing} onAdd={vi.fn()} onClose={onClose} />;
  }
  try {
    await act(async () => root.render(<Harness />));
    const addRepositories = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Add repositories…")!;
    addRepositories.focus();
    await act(async () => addRepositories.click());
    expect(client.repositories).toHaveBeenCalledOnce();
    const picker = document.body.querySelector<HTMLElement>(".context-repository-dialog")!;
    expect([...picker.querySelectorAll("code")].map((path) => path.textContent)).toEqual(["/first", "/second"]);
    expect(picker.querySelector<HTMLButtonElement>(".setup-primary")?.disabled).toBe(true);
    await act(async () => picker.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(document.body.querySelector(".context-repository-dialog")).toBeNull();
    expect(document.activeElement).toBe(addRepositories);
    expect(onClose).not.toHaveBeenCalled();
    expect(client.librarySpaceRepositories).not.toHaveBeenCalled();

    await act(async () => addRepositories.click());
    const reopened = document.body.querySelector<HTMLElement>(".context-repository-dialog")!;
    for (const checkbox of reopened.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')) {
      await act(async () => checkbox.click());
    }
    expect(reopened.querySelector<HTMLButtonElement>(".setup-primary")?.textContent).toBe("Add 2 repositories");
    await act(async () => reopened.querySelector<HTMLButtonElement>(".setup-primary")!.click());
    expect(client.librarySpaceRepositories).toHaveBeenCalledWith({ target, repository_paths: ["/existing", "/first", "/second"] });
    expect(document.body.querySelector(".context-repository-dialog")).toBeNull();
    expect([...host.querySelectorAll(".context-repo-path")].map((path) => path.textContent)).toEqual(["/existing", "/first", "/second"]);

    vi.mocked(client.librarySpaceRepositories).mockRejectedValueOnce(new Error("Disk error"));
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="Remove /first from Review"]')!.click());
    expect(client.librarySpaceRepositories).toHaveBeenLastCalledWith({ target, repository_paths: ["/existing", "/second"] });
    expect(host.querySelector(".context-repositories [role='alert']")?.textContent).toContain("Couldn't remove /first from Review. Disk error");
    expect([...host.querySelectorAll(".context-repo-path")].map((path) => path.textContent)).toEqual(["/existing", "/first", "/second"]);
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="Remove /first from Review"]')!.click());
    expect([...host.querySelectorAll(".context-repo-path")].map((path) => path.textContent)).toEqual(["/existing", "/second"]);
    expect(host.querySelector(".context-repositories [role='alert']")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
