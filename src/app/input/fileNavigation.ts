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
  const folded = Array.from(query.toLocaleLowerCase());
  const basenameStart = candidate.map((part) => part.folded).lastIndexOf("/") + 1;
  let best: { score: number; matchedIndices: number[] } | null = null;
  // Greedy from every occurrence of the first character, so "sd" prefers a
  // contiguous "SD" over the first "s" in the path.
  for (let start = 0; start < candidate.length; start += 1) {
    if (candidate[start].folded !== folded[0]) continue;
    let cursor = start;
    let score = 0;
    let previous = -2;
    const matchedIndices = new Set<number>();
    let complete = true;
    for (const character of folded) {
      let index = cursor;
      while (index < candidate.length && candidate[index].folded !== character) index += 1;
      if (index === candidate.length) { complete = false; break; }
      score += index - cursor;
      if (index !== previous + 1) score += 3;
      if (index === 0 || ["/", "-", "_", " "].includes(candidate[index - 1].folded)) score -= 4;
      matchedIndices.add(candidate[index].sourceIndex);
      previous = index;
      cursor = index + 1;
    }
    if (!complete) break;
    if (previous >= basenameStart) score -= 8;
    if (!best || score < best.score) best = { score, matchedIndices: [...matchedIndices] };
  }
  return best && { score: best.score + Math.max(0, candidate.length - folded.length) / 1000, matchedIndices: best.matchedIndices };
}

export function rankFuzzyMatches<T>(
  query: string,
  candidates: readonly T[],
  text: (candidate: T) => string,
): Array<T & { score: number; matchedIndices: number[] }> {
  const normalized = query.trim();
  return candidates.flatMap((candidate, index) => {
    const match = normalized ? subsequenceMatch(normalized, text(candidate)) : { score: index, matchedIndices: [] };
    return match === null ? [] : [{ ...candidate, ...match }];
  }).sort((left, right) => left.score - right.score || text(left).localeCompare(text(right)));
}

/** File matching favors the file name. Each whitespace-separated query token
 * matches the basename first, then the directory, then the whole path, so
 * `sd deploy` finds a page named "Deploy…" below a folder containing "sd". */
export function rankFileMatches(query: string, candidates: readonly FileNavigationCandidate[]): FileNavigationMatch[] {
  const tokens = query.trim().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return candidates.map((candidate, index) => ({ ...candidate, score: index, matchedIndices: [] }));
  return candidates.flatMap((candidate) => {
    const characters = Array.from(candidate.path);
    const nameStart = characters.lastIndexOf("/") + 1;
    const name = characters.slice(nameStart).join("");
    const directory = characters.slice(0, nameStart).join("");
    let score = 0;
    const matchedIndices = new Set<number>();
    for (const token of tokens) {
      const inName = subsequenceMatch(token, name);
      const inDirectory = inName ? null : subsequenceMatch(token, directory);
      const inPath = inName || inDirectory ? null : subsequenceMatch(token, candidate.path);
      const match = inName ?? inDirectory ?? inPath;
      if (!match) return [];
      const offset = inName ? nameStart : 0;
      score += match.score + (inName ? -10 : inDirectory ? 0 : 2);
      for (const index of match.matchedIndices) matchedIndices.add(index + offset);
    }
    return [{ ...candidate, score: score + characters.length / 1000, matchedIndices: [...matchedIndices].sort((a, b) => a - b) }];
  }).sort((left, right) => left.score - right.score || left.path.localeCompare(right.path));
}

export type FilePathParts = {
  /** Basename without its extension. */
  stem: string;
  /** Extension including the dot, or "". */
  extension: string;
  /** Directory segments; a last segment equal to the stem (`Title/Title.md`) is omitted. */
  directories: Array<{ text: string; start: number }>;
  nameStart: number;
};

/** Splits a path for display, keeping code-point offsets for match highlighting. */
export function filePathParts(path: string): FilePathParts {
  const characters = Array.from(path);
  const nameStart = characters.lastIndexOf("/") + 1;
  const name = characters.slice(nameStart);
  const dot = name.lastIndexOf(".");
  const stemLength = dot > 0 ? dot : name.length;
  const stem = name.slice(0, stemLength).join("");
  const directories: Array<{ text: string; start: number }> = [];
  let start = 0;
  for (let index = 0; index < nameStart; index += 1) {
    if (characters[index] !== "/") continue;
    if (index > start) directories.push({ text: characters.slice(start, index).join(""), start });
    start = index + 1;
  }
  if (directories.at(-1)?.text === stem) directories.pop();
  return { stem, extension: name.slice(stemLength).join(""), directories, nameStart };
}
