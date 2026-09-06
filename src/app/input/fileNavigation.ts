export const FILE_NAVIGATION_EVENT = "cockpit:file-navigation";

export type FileNavigationAction = "open-picker" | "focus-tree" | "focus-content";

export type FileNavigationEventDetail = { action: FileNavigationAction };

export type FileNavigationCandidate = { id: string; path: string; detail?: string };

export type FileNavigationMatch = FileNavigationCandidate & { score: number };

/**
 * The workbench prefix router dispatches this event after it has established
 * that the focused pane is a graphical file surface. Context and Review own
 * the local meaning of the action and ignore it unless they contain DOM focus.
 */
export function dispatchFileNavigation(action: FileNavigationAction, target: EventTarget = window): void {
  target.dispatchEvent(new CustomEvent<FileNavigationEventDetail>(FILE_NAVIGATION_EVENT, { detail: { action } }));
}

export function fileNavigationAction(event: Event): FileNavigationAction | null {
  if (event.type !== FILE_NAVIGATION_EVENT || !(event instanceof CustomEvent)) return null;
  const action = event.detail?.action;
  return action === "open-picker" || action === "focus-tree" || action === "focus-content" ? action : null;
}

function subsequenceScore(query: string, path: string): number | null {
  const candidate = path.toLocaleLowerCase();
  let cursor = 0;
  let score = 0;
  let previous = -2;
  for (const character of query.toLocaleLowerCase()) {
    const index = candidate.indexOf(character, cursor);
    if (index < 0) return null;
    score += index - cursor;
    if (index !== previous + 1) score += 3;
    if (index === 0 || candidate[index - 1] === "/" || candidate[index - 1] === "-" || candidate[index - 1] === "_") score -= 4;
    previous = index;
    cursor = index + 1;
  }
  const basenameStart = candidate.lastIndexOf("/") + 1;
  if (previous >= basenameStart) score -= 8;
  return score + Math.max(0, candidate.length - query.length) / 1000;
}

export function rankFileMatches(query: string, candidates: readonly FileNavigationCandidate[]): FileNavigationMatch[] {
  const normalized = query.trim();
  return candidates.flatMap((candidate, index) => {
    const score = normalized ? subsequenceScore(normalized, candidate.path) : index;
    return score === null ? [] : [{ ...candidate, score }];
  }).sort((left, right) => left.score - right.score || left.path.localeCompare(right.path));
}
