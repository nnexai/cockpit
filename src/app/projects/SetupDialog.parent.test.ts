import { describe, expect, it } from "vitest";
import type { PaneSummary, RepositoryCandidate, SpaceSummary } from "../../protocol/generated/v1";
import { resolveParentRepository, setupParentFor } from "./SetupDialog";

function repository(id: string, checkout: string): RepositoryCandidate {
  return { repository_id: id, name: id, root: checkout, checkout_path: checkout, common_dir: `${checkout}/.git`, branch: "main", is_linked_worktree: false, is_detached: false, provenance: "discovered" };
}

function space(git: SpaceSummary["git"] = null): SpaceSummary {
  return { id: "w1", label: "Work", number: 1, tab_count: 1, pane_count: 2, focused: true, agent_status: "idle", git };
}

function pane(id: string, cwd: string | undefined, focused = false): PaneSummary {
  return { id, terminal_id: `t-${id}`, space_id: "w1", tab_id: "w1:t1", title: null, focused, agent: null, agent_status: "idle", revision: 0, cwd };
}

describe("setup parent", () => {
  const repositories = [repository("outer", "/src/app"), repository("inner", "/src/app/vendor/lib"), repository("other", "/src/application")];

  it("prefers the Space's own checkout", () => {
    const parent = setupParentFor(space({ repository_key: "/src/app/.git", repository: "app", branch: "main", checkout_path: "/src/app", is_linked_worktree: false }), [pane("w1:p1", "/tmp")], null);
    expect(parent?.checkoutPath).toBe("/src/app");
  });

  it("uses the focused pane's folder for a plain Space", () => {
    const panes = [pane("w1:p1", "/tmp"), pane("w1:p2", "/src/app/crates/core")];
    expect(setupParentFor(space(), panes, "w1:p2")?.checkoutPath).toBe("/src/app/crates/core");
    expect(setupParentFor(space(), [pane("w1:p1", undefined)], "w1:p1")).toBeNull();
  });

  it("matches the innermost checkout containing the folder, not a name prefix", () => {
    const resolve = (path: string) => resolveParentRepository(repositories, { label: "Work", repositoryKey: "", checkoutPath: path })?.repository_id ?? null;
    expect(resolve("/src/app/crates/core")).toBe("outer");
    expect(resolve("/src/app/vendor/lib/src")).toBe("inner");
    expect(resolve("/src/application")).toBe("other");
    expect(resolve("/src/appx")).toBeNull();
    expect(resolve("/elsewhere")).toBeNull();
  });
});
