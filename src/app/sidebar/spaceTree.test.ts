import { describe, expect, it } from "vitest";
import type { AgentSummary, SpaceGitSummary, SpaceSummary } from "../../protocol/generated/v1";
import { orderAgentsByHerdrPriority, projectSpaceTree, spaceDisplayName, spaceDropBeforeId, spaceRowStatus, spaceStatus } from "./spaceTree";

function space(id: string, label: string, git: SpaceGitSummary | null = null, agentStatus = "idle"): SpaceSummary {
  return { id, label, number: 1, tab_count: 0, pane_count: 0, focused: false, agent_status: agentStatus, git };
}

function git(repositoryKey: string, branch: string | null, isLinkedWorktree: boolean, repository = "repo", checkoutPath = "/checkout"): SpaceGitSummary {
  return { repository_key: repositoryKey, repository, branch, checkout_path: checkoutPath, is_linked_worktree: isLinkedWorktree };
}

describe("agent ordering", () => {
  it("matches Herdr's priority sort by status then latest state change", () => {
    const agent = (pane_id: string, status: string, state_change_seq: number): AgentSummary => ({
      pane_id,
      space_id: "space-1",
      tab_id: "tab-1",
      name: pane_id,
      status,
      title: null,
      focused: false,
      state_change_seq,
    });
    const agents = [
      agent("idle", "idle", 100),
      agent("working-old", "working", 10),
      agent("done", "done", 1),
      agent("blocked", "blocked", 1),
      agent("working-new", "working", 20),
    ];
    expect(orderAgentsByHerdrPriority(agents).map(({ pane_id }) => pane_id)).toEqual([
      "blocked",
      "done",
      "working-new",
      "working-old",
      "idle",
    ]);
    expect(orderAgentsByHerdrPriority([
      { ...agent("older-wire-shape", "idle", 0), focused: false },
      { ...agent("focused-wire-shape", "idle", 0), focused: true },
    ]).map(({ pane_id }) => pane_id)).toEqual(["focused-wire-shape", "older-wire-shape"]);
  });
});

describe("Space tree projection", () => {
  it("leaves non-Git, singleton Git, and all-linked Spaces ungrouped in source order", () => {
    const spaces = [
      space("plain", "plain"),
      space("single", "single", git("single-repo", "main", false)),
      space("linked-1", "linked one", git("linked-repo", "worktree/one", true)),
      space("linked-2", "linked two", git("linked-repo", "worktree/two", true)),
    ];
    const rows = projectSpaceTree(spaces);
    expect(rows.map((row) => [row.kind, row.space.id])).toEqual([
      ["top-level", "plain"],
      ["top-level", "single"],
      ["top-level", "linked-1"],
      ["top-level", "linked-2"],
    ]);
  });

  it("emits the first non-linked parent followed by children in authoritative source order", () => {
    const spaces = [
      space("w1C", "worktree one", git("lilygo", "worktree/brave-forest-7518", true)),
      space("w18", "lilygo-t3", git("lilygo", "main", false)),
      space("w1D", "worktree two", git("lilygo", "worktree/brave-stone-13f0", true)),
      space("cockpit", "cockpit", git("cockpit", "main", false)),
    ];
    const rows = projectSpaceTree(spaces);
    expect(rows.map((row) => [row.kind, row.space.id, row.label, row.branch])).toEqual([
      ["parent", "w18", "lilygo-t3", "main"],
      ["child", "w1C", "brave-forest-7518", "worktree/brave-forest-7518"],
      ["child", "w1D", "brave-stone-13f0", "worktree/brave-stone-13f0"],
      ["top-level", "cockpit", "cockpit", "main"],
    ]);
  });

  it("retains only the selected child beneath a collapsed parent", () => {
    const spaces = [
      space("parent", "repo", git("repo", "main", false)),
      space("child-1", "one", git("repo", "worktree/one", true)),
      space("child-2", "two", git("repo", "worktree/two", true)),
    ];
    const rows = projectSpaceTree(spaces, new Set(["repo"]), "child-2");
    expect(rows.map((row) => [row.kind, row.space.id, row.expanded, row.connector])).toEqual([
      ["parent", "parent", false, null],
      ["child", "child-2", false, "└─"],
    ]);
  });

  it("never duplicates a grouped Space row", () => {
    const spaces = [
      space("child-before", "before", git("repo", "worktree/before", true)),
      space("parent", "repo", git("repo", "main", false)),
      space("second-parent", "repo clone", git("repo", "release", false)),
      space("child-after", "after", git("repo", "worktree/after", true)),
    ];
    const ids = projectSpaceTree(spaces).map((row) => row.space.id);
    expect(ids).toEqual(["parent", "child-before", "second-parent", "child-after"]);
    expect(new Set(ids).size).toBe(spaces.length);
  });

  it("shows the most urgent worktree state on a collapsed repository row only", () => {
    const spaces = [
      space("parent", "repo", git("repo", "main", false), "idle"),
      space("child-1", "one", git("repo", "worktree/one", true), "blocked"),
      space("child-2", "two", git("repo", "worktree/two", true), "working"),
      space("other", "other", git("other", "main", false), "blocked"),
    ];
    const status = (collapsed: string[]) => projectSpaceTree(spaces, new Set(collapsed)).filter((row) => row.kind !== "child").map((row) => spaceRowStatus(row, spaces));
    expect(status([])).toEqual(["idle", "blocked"]);
    expect(status(["repo"])).toEqual(["blocked", "blocked"]);
  });

  it("maps agent status to a badge shape, colour class and state word", () => {
    expect(spaceStatus("blocked")).toEqual({ shape: "blocked", className: "blocked", word: "Blocked" });
    expect(spaceStatus("running")).toEqual({ shape: "working", className: "working", word: "Working" });
    expect(spaceStatus("complete")).toEqual({ shape: "done", className: "done", word: "Done" });
    expect(spaceStatus("idle")).toEqual({ shape: "idle", className: "idle", word: "Idle" });
    expect(spaceStatus("unexpected")).toEqual({ shape: "unknown", className: "unknown", word: "Unknown" });
  });
});

describe("worktree display names", () => {
  const child = (label: string, branch: string | null, repository: string, checkoutPath = "/checkout") =>
    space("child", label, git("repo-key", branch, true, repository, checkoutPath));

  it.each([
    ["branch with worktree/ prefix", "ignored", "worktree/brave-forest-7518", "lilygo", "brave-forest-7518"],
    ["label with worktree- prefix when the branch is absent", "worktree-brave-forest-7518", null, "lilygo", "brave-forest-7518"],
    ["repeated repository segments", "x", "cockpit/cockpit/SCRUM-142-inline-browser-pointer", "cockpit", "SCRUM-142-inline-browser-pointer"],
    ["repository segment match is case-insensitive", "x", "Cockpit/topic", "cockpit", "topic"],
    ["a branch that only looks like the repository path stays whole", "x", "feature/SCRUM-142-x", "cockpit", "feature/SCRUM-142-x"],
    ["only an exact worktree prefix is stripped", "x", "feature/worktree/topic", "repo", "feature/worktree/topic"],
    ["a name made only of repository segments keeps the last one", "x", "cockpit/cockpit", "cockpit", "cockpit"],
    ["the 4-hex Herdr suffix is identity and stays", "x", "worktree/brave-forest-7518", "repo", "brave-forest-7518"],
  ])("%s", (_case, label, branch, repository, expected) => {
    expect(spaceDisplayName(child(label, branch, repository))).toBe(expected);
  });

  it("falls back to the label, then to the checkout folder name", () => {
    expect(spaceDisplayName(child("worktree-topic", null, "repo"))).toBe("topic");
    expect(spaceDisplayName(child("", null, "repo", "/work/trees/topic-1234/"))).toBe("topic-1234");
  });

  it("names only child rows; top-level and parent rows keep the user's label", () => {
    const spaces = [
      space("parent", "worktree-parent", git("repo", "main", false)),
      space("child", "raw", git("repo", "worktree/topic", true)),
      space("alone", "worktree-alone", git("solo", "worktree/alone", false)),
    ];
    expect(projectSpaceTree(spaces).map((row) => row.label)).toEqual(["worktree-parent", "topic", "worktree-alone"]);
  });
});

describe("resource drop boundaries", () => {
  it("converts Space target halves to Herdr before anchors", () => {
    const spaces = ["a", "b", "c"].map((id) => space(id, id));
    expect(spaceDropBeforeId(spaces, "a", "b", false)).toBeUndefined();
    expect(spaceDropBeforeId(spaces, "a", "b", true)).toBe("c");
    expect(spaceDropBeforeId(spaces, "c", "a", false)).toBe("a");
    expect(spaceDropBeforeId(spaces, "c", "a", true)).toBe("b");
    expect(spaceDropBeforeId(spaces, "b", "b", true)).toBeUndefined();
  });
});
