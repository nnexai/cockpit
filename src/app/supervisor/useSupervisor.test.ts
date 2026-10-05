import { describe, expect, it } from "vitest";
import type { OrchestrationSnapshot } from "../../protocol/generated/v1";
import { acceptsSupervisorSnapshot } from "./useSupervisor";

const snapshot: OrchestrationSnapshot = {
  session_id: "session-a", revision: 12, tasks_token: "external-edit-a",
  roots: [], board: { root_id: "root-a", path: "/state/tasks/root-a.md", doc_revision: "doc-a", unidentified_items: 0, diagnostics: [], tasks: [] },
  runs: [], messages: [], subagents: [], intents: [], attention: [], unmanaged_agents: [],
  runtime: { status: "unavailable", error: { code: "herdr_unavailable", message: "offline" } },
};

describe("supervisor snapshot fences", () => {
  it("rejects an old machine revision after a successful mutation", () => {
    expect(acceptsSupervisorSnapshot(snapshot, "session-a", "root-a", 13)).toBe(false);
    expect(acceptsSupervisorSnapshot({ ...snapshot, revision: 13 }, "session-a", "root-a", 13)).toBe(true);
  });
  it("rejects late responses from another session or selected root", () => {
    expect(acceptsSupervisorSnapshot(snapshot, "session-b", "root-a", 0)).toBe(false);
    expect(acceptsSupervisorSnapshot(snapshot, "session-a", "root-b", 0)).toBe(false);
  });
  it("accepts external Markdown changes without requiring a machine revision increment", () => {
    expect(acceptsSupervisorSnapshot({ ...snapshot, tasks_token: "external-edit-b", board: { ...snapshot.board!, doc_revision: "doc-b" } }, "session-a", "root-a", 12)).toBe(true);
  });
  it("accepts a missing board as absence of canonical tasks, not missing runtime", () => {
    expect(acceptsSupervisorSnapshot({ ...snapshot, board: null }, "session-a", "root-a", 12)).toBe(true);
  });
});
