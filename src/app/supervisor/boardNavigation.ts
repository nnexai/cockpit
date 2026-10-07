import type { TaskLane, TaskView } from "../../protocol/generated/v1";

export const taskLanes: readonly { lane: TaskLane; label: string }[] = [
  { lane: "queued", label: "Queued" }, { lane: "setup", label: "Preparing" },
  { lane: "ready", label: "Ready" }, { lane: "working", label: "Working" },
  { lane: "review", label: "Review" }, { lane: "accepted", label: "Done" },
];

export type BoardArrangement = "lanes" | "stacked";
export type BoardNavOptions = { arrangement: BoardArrangement; completedOpen: boolean; collapsedLanes: readonly TaskLane[] };

export function visibleTaskIds(tasks: readonly TaskView[], options: BoardNavOptions): string[] {
  return taskLanes.flatMap(({ lane }) =>
    lane === "accepted" && !options.completedOpen || options.arrangement === "stacked" && options.collapsedLanes.includes(lane)
      ? [] : tasks.filter(task => task.lane === lane).map(task => task.task.task_id));
}

/** Focus navigation only: never changes the backend-derived lane or selection. */
export function taskNeighbor(tasks: readonly TaskView[], id: string, key: string, options: BoardNavOptions): string | undefined {
  if (options.arrangement === "stacked") {
    const ids = visibleTaskIds(tasks, options);
    const index = ids.indexOf(id);
    if (index < 0) return undefined;
    if (key === "Home") return ids[0];
    if (key === "End") return ids[ids.length - 1];
    if (key === "ArrowUp" || key === "ArrowDown") return ids[Math.max(0, Math.min(ids.length - 1, index + (key === "ArrowDown" ? 1 : -1)))];
    return undefined;
  }
  const lanes = taskLanes.filter(item => item.lane !== "accepted" || options.completedOpen)
    .map(item => tasks.filter(task => task.lane === item.lane));
  const laneIndex = lanes.findIndex(lane => lane.some(task => task.task.task_id === id));
  if (laneIndex < 0) return undefined;
  const lane = lanes[laneIndex];
  const index = lane.findIndex(task => task.task.task_id === id);
  if (key === "ArrowUp" || key === "ArrowDown") return lane[Math.max(0, Math.min(lane.length - 1, index + (key === "ArrowDown" ? 1 : -1)))]?.task.task_id;
  if (key === "Home" || key === "End") return lane[key === "Home" ? 0 : lane.length - 1]?.task.task_id;
  if (key !== "ArrowLeft" && key !== "ArrowRight") return undefined;
  const direction = key === "ArrowRight" ? 1 : -1;
  for (let next = laneIndex + direction; next >= 0 && next < lanes.length; next += direction) {
    if (lanes[next].length) return lanes[next][Math.min(index, lanes[next].length - 1)].task.task_id;
  }
  return id;
}
