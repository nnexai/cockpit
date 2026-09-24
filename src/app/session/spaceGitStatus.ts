import { useEffect, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { SpaceGitStatus, SpaceSummary } from "../../protocol/generated/v1";

export const SPACE_GIT_POLL_MS = 15_000;

/** A key that changes when the set of Space checkouts changes, so a new Space is read at once. */
export function spaceCheckoutKey(spaces: readonly SpaceSummary[]): string {
  return spaces.map((space) => `${space.id}\u0000${space.git?.checkout_path ?? ""}`).join("\u0001");
}

/**
 * Polls Cockpit for each Space's branch and ahead/behind counts while the page is visible.
 * Herdr's snapshot has no such counts and commits do not produce Herdr events, so this refreshes on a timer
 * the way Herdr's TUI does. Failures keep the last known values: the counts are informational.
 */
export function useSpaceGitStatus(client: CockpitClient, sessionId: string | null, checkoutKey: string, intervalMs = SPACE_GIT_POLL_MS): ReadonlyMap<string, SpaceGitStatus> {
  const [status, setStatus] = useState<{ sessionId: string | null; spaces: ReadonlyMap<string, SpaceGitStatus> }>({ sessionId: null, spaces: new Map() });
  useEffect(() => {
    if (!sessionId || !checkoutKey) return;
    let controller: AbortController | null = null;
    const refresh = () => {
      if (globalThis.document?.visibilityState === "hidden" || controller) return;
      const current = new AbortController();
      controller = current;
      client.spaceGitStatus(sessionId, current.signal).then((response) => {
        if (!current.signal.aborted) setStatus({ sessionId, spaces: new Map(response.spaces.map((space) => [space.space_id, space])) });
      }, () => undefined).finally(() => { if (controller === current) controller = null; });
    };
    refresh();
    const timer = setInterval(refresh, intervalMs);
    globalThis.document?.addEventListener("visibilitychange", refresh);
    return () => {
      clearInterval(timer);
      globalThis.document?.removeEventListener("visibilitychange", refresh);
      controller?.abort();
    };
  }, [client, sessionId, checkoutKey, intervalMs]);
  return status.sessionId === sessionId ? status.spaces : EMPTY;
}

const EMPTY: ReadonlyMap<string, SpaceGitStatus> = new Map();

/** Herdr's compact branch position, e.g. "↑14" or "↑2 ↓1"; empty when level or without an upstream. */
export function aheadBehindLabel(status: SpaceGitStatus | undefined): string {
  if (!status) return "";
  return [status.ahead ? `↑${status.ahead}` : "", status.behind ? `↓${status.behind}` : ""].filter(Boolean).join(" ");
}
