import type { TaskView } from "../../protocol/generated/v1";

export type DependencyPosition = { view: TaskView; x: number; y: number; level: number; independent: boolean; cycle: boolean };
export type DependencyEdge = { from: string; to: string; provenance: boolean };
export function dependencyLayout(tasks: readonly TaskView[], showCompleted: boolean) {
  const counts = new Map<string, number>();
  for (const view of tasks) counts.set(view.task.task_id, (counts.get(view.task.task_id) ?? 0) + 1);
  const valid = tasks.filter(view => counts.get(view.task.task_id) === 1);
  const byId = new Map(valid.map(view => [view.task.task_id, view]));
  const incident = new Set<string>();
  const edges: DependencyEdge[] = [];
  for (const view of valid) {
    for (const id of view.task.depends_on) if (byId.has(id)) { edges.push({ from: id, to: view.task.task_id, provenance: false }); incident.add(id); incident.add(view.task.task_id); }
    if (view.task.follow_up_of && byId.has(view.task.follow_up_of)) { incident.add(view.task.follow_up_of); incident.add(view.task.task_id); if (!view.task.depends_on.includes(view.task.follow_up_of)) edges.push({ from: view.task.follow_up_of, to: view.task.task_id, provenance: true }); }
  }
  const shown = new Set(valid.filter(view => showCompleted || !view.task.checked || incident.has(view.task.task_id) || view.task.depends_on.length > 0 || !!view.task.follow_up_of).map(view => view.task.task_id));
  // Retain accepted context to a fixed point, including provenance chains.
  let added = true;
  while (added) { added = false; for (const edge of edges) if (shown.has(edge.from) || shown.has(edge.to)) for (const id of [edge.from, edge.to]) if (!shown.has(id)) { shown.add(id); added = true; } }
  const displayed = valid.filter(view => shown.has(view.task.task_id));
  const forward = new Map<string, string[]>(), reverse = new Map<string, string[]>();
  for (const view of displayed) { forward.set(view.task.task_id, []); reverse.set(view.task.task_id, []); }
  for (const edge of edges) if (!edge.provenance && shown.has(edge.from) && shown.has(edge.to)) { forward.get(edge.from)!.push(edge.to); reverse.get(edge.to)!.push(edge.from); }
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
  for (const edge of edges) if (!edge.provenance && component.has(edge.from) && component.has(edge.to)) {
    const from = component.get(edge.from)!, to = component.get(edge.to)!;
    if (from !== to && !outgoing[from].has(to)) { outgoing[from].add(to); indegree[to]++; }
  }
  const queue = indegree.flatMap((count, index) => count === 0 ? [index] : []);
  for (let cursor = 0; cursor < queue.length; cursor++) for (const next of outgoing[queue[cursor]]) { levels[next] = Math.max(levels[next], levels[queue[cursor]] + 1); if (--indegree[next] === 0) queue.push(next); }
  const rowByLevel = new Map<number, number>();
  const positions: DependencyPosition[] = [];
  const groupLine = members.map(ids => ids.reduce((line, id) => Math.min(line, byId.get(id)!.task.line), Number.POSITIVE_INFINITY));
  const ordered = [...displayed].sort((a, b) => groupLine[component.get(a.task.task_id)!] - groupLine[component.get(b.task.task_id)!] || component.get(a.task.task_id)! - component.get(b.task.task_id)! || a.task.line - b.task.line || a.task.task_id.localeCompare(b.task.task_id));
  for (const view of ordered) {
    const id = view.task.task_id, group = component.get(id)!, independent = !incident.has(id) && view.task.depends_on.length === 0;
    const level = levels[group], row = rowByLevel.get(level) ?? 0;
    if (!independent) rowByLevel.set(level, row + 1);
    positions.push({ view, x: 16 + level * 272, y: 44 + row * 72, level, independent, cycle: members[group].length > 1 || view.task.depends_on.includes(id) });
  }
  let bandTop = 64;
  for (const count of rowByLevel.values()) bandTop = Math.max(bandTop, 64 + count * 72);
  let independentRow = 0;
  for (const position of positions) if (position.independent) { position.x = 16; position.y = bandTop + (++independentRow) * 72; }
  positions.sort((a, b) => Number(a.independent) - Number(b.independent) || a.level - b.level || a.y - b.y || a.view.task.task_id.localeCompare(b.view.task.task_id));
  const cycleGroups: { x: number; y: number; height: number; count: number }[] = [];
  const positionById = new Map(positions.map(position => [position.view.task.task_id, position]));
  for (const ids of members) {
    if (ids.length < 2 && !byId.get(ids[0])!.task.depends_on.includes(ids[0])) continue;
    let first = Number.POSITIVE_INFINITY, last = 0;
    for (const id of ids) { const position = positionById.get(id)!; first = Math.min(first, position.y); last = Math.max(last, position.y); }
    cycleGroups.push({ x: positionById.get(ids[0])!.x - 6, y: first - 6, height: last - first + 60, count: ids.length });
  }
  let width = 288, height = 160;
  for (const position of positions) { width = Math.max(width, position.x + 256); height = Math.max(height, position.y + 72); }
  return { positions, cycleGroups, edges: edges.filter(edge => shown.has(edge.from) && shown.has(edge.to)), hiddenCompleted: valid.length - displayed.length, independentTop: bandTop, width, height };
}
