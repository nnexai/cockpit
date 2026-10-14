import type { TaskView } from "../../protocol/generated/v1";

export type DependencyPosition = { view: TaskView; x: number; y: number; level: number; independent: boolean; cycle: boolean };
export type DependencyEdge = { from: string; to: string };
export function dependencyLayout(tasks: readonly TaskView[]) {
  const counts = new Map<string, number>();
  for (const view of tasks) counts.set(view.task.task_id, (counts.get(view.task.task_id) ?? 0) + 1);
  const valid = tasks.filter(view => counts.get(view.task.task_id) === 1);
  const byId = new Map(valid.map(view => [view.task.task_id, view]));
  const incident = new Set<string>();
  const edges: DependencyEdge[] = [];
  for (const view of valid) {
    for (const id of view.task.depends_on) if (byId.has(id)) { edges.push({ from: id, to: view.task.task_id }); incident.add(id); incident.add(view.task.task_id); }
  }
  const displayed = valid;
  const forward = new Map<string, string[]>(), reverse = new Map<string, string[]>();
  for (const view of displayed) { forward.set(view.task.task_id, []); reverse.set(view.task.task_id, []); }
  for (const edge of edges) { forward.get(edge.from)!.push(edge.to); reverse.get(edge.to)!.push(edge.from); }
  const visited = new Set<string>(), finish: string[] = [];
  for (const view of displayed) {
    const id = view.task.task_id;
    if (visited.has(id)) continue;
    visited.add(id);
    const stack = [{ id, next: 0 }];
    while (stack.length) {
      const top = stack[stack.length - 1], neighbors = forward.get(top.id)!;
      if (top.next < neighbors.length) { const next = neighbors[top.next++]; if (!visited.has(next)) { visited.add(next); stack.push({ id: next, next: 0 }); } }
      else { finish.push(top.id); stack.pop(); }
    }
  }
  const component = new Map<string, number>(), members: string[][] = [];
  for (const id of finish.reverse()) {
    if (component.has(id)) continue;
    const group = members.length, collected: string[] = [], stack = [id]; component.set(id, group);
    while (stack.length) { const current = stack.pop()!; collected.push(current); for (const next of reverse.get(current)!) if (!component.has(next)) { component.set(next, group); stack.push(next); } }
    members.push(collected);
  }
  const outgoing = members.map(() => new Set<number>()), indegree = members.map(() => 0), levels = members.map(() => 0);
  for (const edge of edges) if (component.has(edge.from) && component.has(edge.to)) {
    const from = component.get(edge.from)!, to = component.get(edge.to)!;
    if (from !== to && !outgoing[from].has(to)) { outgoing[from].add(to); indegree[to]++; }
  }
  const queue = indegree.flatMap((count, index) => count === 0 ? [index] : []);
  for (let cursor = 0; cursor < queue.length; cursor++) for (const next of outgoing[queue[cursor]]) { levels[next] = Math.max(levels[next], levels[queue[cursor]] + 1); if (--indegree[next] === 0) queue.push(next); }
  const rowsByLevel = new Map<number, Set<number>>();
  const positions: DependencyPosition[] = [];
  const ordered = [...displayed].sort((a, b) => levels[component.get(a.task.task_id)!] - levels[component.get(b.task.task_id)!] || a.task.line - b.task.line || a.task.task_id.localeCompare(b.task.task_id));
  const positionById = new Map<string, DependencyPosition>();
  for (const view of ordered) {
    const id = view.task.task_id, group = component.get(id)!, independent = !incident.has(id) && view.task.depends_on.length === 0;
    const level = levels[group], used = rowsByLevel.get(level) ?? new Set<number>();
    const firstPrerequisite = view.task.depends_on.map(prerequisite => positionById.get(prerequisite)).find(position => position !== undefined);
    let row = firstPrerequisite ? (firstPrerequisite.y - 16) / 56 : 0;
    if (!independent) {
      while (used.has(row)) row++;
      used.add(row); rowsByLevel.set(level, used);
    }
    const position = { view, x: 16 + level * 272, y: 16 + row * 56, level, independent, cycle: members[group].length > 1 || view.task.depends_on.includes(id) };
    positions.push(position); positionById.set(id, position);
  }
  let bandTop = 16;
  for (const used of rowsByLevel.values()) for (const row of used) bandTop = Math.max(bandTop, 16 + (row + 1) * 56);
  let independentRow = 0;
  for (const position of positions) if (position.independent) { position.x = 16; position.y = bandTop + independentRow++ * 56; }
  positions.sort((a, b) => a.y - b.y || a.level - b.level || a.view.task.line - b.view.task.line);
  let width = 288, height = 80;
  for (const position of positions) { width = Math.max(width, position.x + 256); height = Math.max(height, position.y + 64); }
  return { positions, edges, independentTop: bandTop, width, height };
}
