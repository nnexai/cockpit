import type { SessionSnapshotResponse } from "../../protocol/generated/v1";

type Agent = SessionSnapshotResponse["agents"][number];
export type AgentContext = { title: string | null; titleLabel: "Session title" | "Agent title"; activity: string | null; activityLabel: "Current activity" | "Session recap" };

/** Context is attached to the live snapshot agent, never inferred from another pane. */
export function agentContext(agent: Agent, labels: (string | null | undefined)[] = []) {
  const excluded = [agent.name, ...labels].map(value => value?.trim().toLocaleLowerCase()).filter(Boolean);
  const clean = (value: string | null | undefined): string | null => {
    const text = value?.trim();
    return text && !excluded.includes(text.toLocaleLowerCase()) ? text : null;
  };
  const sessionTitle = clean(agent.session_title);
  const title = sessionTitle || clean(agent.title);
  const working = agent.status === "working";
  const activity = working ? clean(agent.working_description) : (agent.status === "idle" || agent.status === "done") ? clean(agent.idle_summary) : null;
  return { title, titleLabel: sessionTitle ? "Session title" as const : "Agent title" as const, activity: activity && activity.toLocaleLowerCase() !== title?.toLocaleLowerCase() ? activity : null, activityLabel: working ? "Current activity" as const : "Session recap" as const };
}
