export const FILE_NAVIGATION_EVENT = "cockpit:file-navigation";

export type FileNavigationAction = "open-picker" | "focus-tree" | "focus-content";

export type FileNavigationEventDetail = { action: FileNavigationAction };

export type FileNavigationCandidate = { id: string; path: string; detail?: string };

export type FileNavigationMatch = FileNavigationCandidate & { score: number; matchedIndices: number[] };

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

function subsequenceMatch(query: string, path: string): { score: number; matchedIndices: number[] } | null {
  // Keep original code-point positions when case folding expands a character.
  const candidate = Array.from(path).flatMap((character, sourceIndex) =>
    Array.from(character.toLocaleLowerCase(), (folded) => ({ folded, sourceIndex })));
  let cursor = 0;
  let score = 0;
  let previous = -2;
  const matchedIndices = new Set<number>();
  for (const character of query.toLocaleLowerCase()) {
    let index = cursor;
    while (index < candidate.length && candidate[index].folded !== character) index += 1;
    if (index === candidate.length) return null;
    score += index - cursor;
    if (index !== previous + 1) score += 3;
    if (index === 0 || ["/", "-", "_"].includes(candidate[index - 1].folded)) score -= 4;
    matchedIndices.add(candidate[index].sourceIndex);
    previous = index;
    cursor = index + 1;
  }
  const basenameStart = candidate.map((part) => part.folded).lastIndexOf("/") + 1;
  if (previous >= basenameStart) score -= 8;
  return { score: score + Math.max(0, candidate.length - Array.from(query).length) / 1000, matchedIndices: [...matchedIndices] };
}

export function rankFileMatches(query: string, candidates: readonly FileNavigationCandidate[]): FileNavigationMatch[] {
  const normalized = query.trim();
  return candidates.flatMap((candidate, index) => {
    const match = normalized ? subsequenceMatch(normalized, candidate.path) : { score: index, matchedIndices: [] };
    return match === null ? [] : [{ ...candidate, ...match }];
  }).sort((left, right) => left.score - right.score || left.path.localeCompare(right.path));
}
