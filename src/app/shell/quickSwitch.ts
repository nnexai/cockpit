import { orderAgentsByHerdrPriority, spaceStatus, type Agent, type Space, type SpaceStatus } from "../sidebar/spaceTree";
import type { Tab } from "./model";

export type SwitchTarget = { kind: "space" | "agent"; id: string };
export type SwitchRow = SwitchTarget & {
  key: string;
  primary: string;
  meta: string;
  title: string;
  status: SpaceStatus;
  current: boolean;
  disabledReason: string | null;
};

export function switchRows({ spaces, agents, tabs, currentSpaceId, currentPaneId, live, busy }: {
  spaces: readonly Space[];
  agents: readonly Agent[];
  tabs: readonly Tab[];
  currentSpaceId: string | null;
  currentPaneId: string | null;
  live: boolean;
  busy: boolean;
}): SwitchRow[] {
  const spaceLabels = new Map(spaces.map(space => [space.id, space.label]));
  const tabLabels = new Map(tabs.map(tab => [tab.id, tab.label]));
  const rows: (SwitchRow & { discriminator: string })[] = spaces.map(space => {
    const status = spaceStatus(space.agent_status);
    return {
      kind: "space", id: space.id, key: `space:${space.id}`, primary: space.label,
      meta: [space.git?.branch, space.git?.repository !== space.label ? space.git?.repository : null, space.git?.is_linked_worktree ? "worktree" : null, status.word].filter(Boolean).join(" · "),
      title: space.git?.checkout_path || space.label, status, current: space.id === currentSpaceId,
      disabledReason: !live ? "Herdr is not live" : busy ? "Another Herdr action is pending" : null,
      discriminator: `#${space.number}`,
    };
  });
  for (const agent of orderAgentsByHerdrPriority([...agents])) {
    const status = spaceStatus(agent.status || "unknown");
    const location = [spaceLabels.get(agent.space_id), tabLabels.get(agent.tab_id)].filter(Boolean);
    rows.push({
      kind: "agent", id: agent.pane_id, key: `agent:${agent.pane_id}:${agent.name}`, primary: agent.name || agent.title || "Agent",
      meta: [...location, status.word].join(" · "), title: [...location, agent.pane_id].join(" · "),
      status, current: agent.pane_id === currentPaneId, disabledReason: live ? null : "Herdr is not live",
      discriminator: `#${agent.pane_id.slice(-4)}`,
    });
  }
  const duplicates = new Map<string, number>();
  const identity = (row: SwitchRow) => JSON.stringify([row.kind, row.primary, row.meta]);
  for (const row of rows) {
    const key = identity(row);
    duplicates.set(key, (duplicates.get(key) ?? 0) + 1);
  }
  return rows.map(({ discriminator, ...row }) => duplicates.get(identity(row))! > 1
    ? { ...row, meta: [row.meta.slice(0, -row.status.word.length), `${discriminator} · ${row.status.word}`].join("") }
    : row);
}

export const switchSearchText = (row: SwitchRow): string => `${row.primary} ${row.meta}`;
