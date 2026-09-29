import type { SessionSnapshotResponse } from "../../protocol/generated/v1";

export type Space = SessionSnapshotResponse["spaces"][number];
export type Agent = SessionSnapshotResponse["agents"][number];

export type StatusClass = "blocked" | "working" | "done" | "idle" | "unknown";
export type SpaceStatus = { shape: StatusClass; className: StatusClass; word: string };

export function stateClass(status: string): StatusClass {
  switch (status.toLowerCase()) {
    case "blocked": case "error": return "blocked";
    case "working": case "running": return "working";
    case "done": case "complete": return "done";
    case "idle": return "idle";
    default: return "unknown";
  }
}

const STATE_WORDS: Record<StatusClass, string> = { blocked: "Blocked", working: "Working", done: "Done", idle: "Idle", unknown: "Unknown" };

/** The badge shape (one distinct shape per state, so colour is never the only cue), colour class and state word. */
export function spaceStatus(status: string): SpaceStatus {
  const className = stateClass(status);
  return { shape: className, className, word: STATE_WORDS[className] };
}

function agentStatusPriority(status: string): number {
  switch (stateClass(status)) {
    case "blocked": return 4;
    case "done": return 3;
    case "working": return 2;
    case "idle": return 1;
    default: return 0;
  }
}

export function orderAgentsByHerdrPriority(agents: Agent[]): Agent[] {
  return agents
    .map((agent, index) => ({ agent, index }))
    .sort((left, right) => agentStatusPriority(right.agent.status) - agentStatusPriority(left.agent.status)
      || right.agent.state_change_seq - left.agent.state_change_seq
      || Number(right.agent.focused) - Number(left.agent.focused)
      || left.index - right.index)
    .map(({ agent }) => agent);
}

export function spaceDropBeforeId(spaces: Space[], sourceId: string, targetId: string, afterTarget: boolean): string | null | undefined {
  const sourceIndex = spaces.findIndex((space) => space.id === sourceId);
  if (sourceIndex < 0 || sourceId === targetId) return undefined;
  const remaining = spaces.filter((space) => space.id !== sourceId);
  const targetIndex = remaining.findIndex((space) => space.id === targetId);
  if (targetIndex < 0) return undefined;
  const insertionIndex = targetIndex + (afterTarget ? 1 : 0);
  if (insertionIndex === sourceIndex) return undefined;
  return remaining[insertionIndex]?.id ?? null;
}

export type SpaceTreeRow = {
  kind: "top-level" | "parent" | "child";
  space: Space;
  label: string;
  branch: string | null;
  repositoryKey: string | null;
  expanded: boolean;
  connector: "├─" | "└─" | null;
};

/** A collapsed repository row stands for its hidden worktrees, so it shows the most urgent of their states. */
export function spaceRowStatus(row: SpaceTreeRow, spaces: Space[]): string {
  if (row.kind !== "parent" || row.expanded || !row.repositoryKey) return row.space.agent_status;
  return spaces
    .filter((space) => space.git?.repository_key === row.repositoryKey)
    .reduce((urgent, space) => agentStatusPriority(space.agent_status) > agentStatusPriority(urgent) ? space.agent_status : urgent, row.space.agent_status);
}

/** The name a worktree child shows: the part that tells it apart from its siblings. The tooltip carries the full raw name. */
export function spaceDisplayName(space: Space): string {
  const git = space.git;
  const checkoutName = git?.checkout_path.replace(/\/+$/, "").split("/").pop();
  const raw = git?.branch || space.label || checkoutName || "";
  const stripped = raw.replace(/^worktree[/-]/, "") || raw;
  const repository = git?.repository.toLowerCase();
  if (!repository) return stripped;
  const segments = stripped.split("/");
  let leading = 0;
  while (leading < segments.length - 1 && segments[leading].toLowerCase() === repository) leading += 1;
  return segments.slice(leading).join("/");
}

/** The unshortened name of a Space: its branch when Herdr reports one, else its label. */
export function spaceRawName(space: Space): string {
  return space.git?.branch || space.label;
}

export function projectSpaceTree(
  spaces: Space[],
  collapsedRepositoryKeys: ReadonlySet<string> = new Set(),
  selectedSpaceId: string | null = null,
): SpaceTreeRow[] {
  const membersByRepository = new Map<string, Space[]>();
  for (const space of spaces) {
    const repositoryKey = space.git?.repository_key;
    if (!repositoryKey) continue;
    const members = membersByRepository.get(repositoryKey);
    if (members) members.push(space);
    else membersByRepository.set(repositoryKey, [space]);
  }

  const groups = new Map<string, { parent: Space; members: Space[] }>();
  for (const [repositoryKey, members] of membersByRepository) {
    const parent = members.find((space) => !space.git?.is_linked_worktree);
    if (members.length >= 2 && parent) groups.set(repositoryKey, { parent, members });
  }

  const rows: SpaceTreeRow[] = [];
  const emittedRepositoryKeys = new Set<string>();
  for (const space of spaces) {
    const repositoryKey = space.git?.repository_key ?? null;
    const group = repositoryKey === null ? undefined : groups.get(repositoryKey);
    if (repositoryKey === null || !group) {
      rows.push({ kind: "top-level", space, label: space.label, branch: space.git?.branch ?? null, repositoryKey: null, expanded: true, connector: null });
      continue;
    }
    if (emittedRepositoryKeys.has(repositoryKey)) continue;
    emittedRepositoryKeys.add(repositoryKey);

    const expanded = !collapsedRepositoryKeys.has(repositoryKey);
    rows.push({ kind: "parent", space: group.parent, label: group.parent.label, branch: group.parent.git?.branch ?? null, repositoryKey, expanded, connector: null });
    const children = group.members.filter((member) => member.id !== group.parent.id);
    const visibleChildren = expanded ? children : children.filter((child) => child.id === selectedSpaceId);
    visibleChildren.forEach((child, index) => rows.push({
      kind: "child",
      label: spaceDisplayName(child),
      space: child,
      branch: child.git?.branch ?? null,
      repositoryKey,
      expanded,
      connector: index === visibleChildren.length - 1 ? "└─" : "├─",
    }));
  }
  return rows;
}
