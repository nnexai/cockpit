import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { PaneSummary, SpaceGitStatus, SpaceSummary } from "../../protocol/generated/v1";

export const SPACE_GIT_POLL_MS = 15_000;

/**
 * A key that changes when the folder any Space shows a branch for changes, so a new Space is read at once.
 * That is the Space's checkout, else its first pane's folder, the same choice Cockpit makes when reading Git.
 */
export function spaceCheckoutKey(spaces: readonly SpaceSummary[], panes: readonly PaneSummary[] = []): string {
  return spaces.map((space) => `${space.id}\u0000${space.git?.checkout_path ?? panes.find((pane) => pane.space_id === space.id && pane.cwd)?.cwd ?? ""}`).join("\u0001");
}

/** Polls while visible; action completion explicitly refreshes even if the page is hidden. */
export function useSpaceGitStatus(client: CockpitClient, sessionId: string | null, checkoutKey: string, intervalMs = SPACE_GIT_POLL_MS) {
  const [status, setStatus] = useState<{ sessionId: string; checkoutKey: string; spaces: ReadonlyMap<string, SpaceGitStatus>; error?: string } | null>(null);
  const refreshRef = useRef<(() => void) | null>(null);
  const refresh = useCallback(() => refreshRef.current?.(), []);
  useEffect(() => {
    if (!sessionId || !checkoutKey) return;
    let controller: AbortController | null = null;
    const poll = (force = false) => {
      if (!force && (globalThis.document?.visibilityState === "hidden" || controller)) return;
      if (force) controller?.abort();
      const current = new AbortController();
      controller = current;
      client.spaceGitStatus(sessionId, current.signal).then((response) => {
        if (!current.signal.aborted) setStatus({ sessionId, checkoutKey, spaces: new Map(response.spaces.map((space) => [space.space_id, space])) });
      }, (error: unknown) => {
        if (!current.signal.aborted) setStatus({ sessionId, checkoutKey, spaces: EMPTY, error: error instanceof Error ? error.message : String(error) });
      }).finally(() => { if (controller === current) controller = null; });
    };
    const visiblePoll = () => poll();
    refreshRef.current = () => poll(true);
    poll();
    const timer = setInterval(visiblePoll, intervalMs);
    globalThis.document?.addEventListener("visibilitychange", visiblePoll);
    return () => {
      refreshRef.current = null;
      clearInterval(timer);
      globalThis.document?.removeEventListener("visibilitychange", visiblePoll);
      controller?.abort();
    };
  }, [client, sessionId, checkoutKey, intervalMs]);
  const current = status?.sessionId === sessionId && status.checkoutKey === checkoutKey ? status : null;
  return { spaces: current?.spaces ?? EMPTY, error: current?.error, refresh };
}

const EMPTY: ReadonlyMap<string, SpaceGitStatus> = new Map();

/** Herdr's compact branch position, e.g. "↑14" or "↑2 ↓1"; empty when level or without an upstream. */
export function aheadBehindLabel(status: SpaceGitStatus | undefined): string {
  const checkout = status?.checkout;
  if (checkout?.state !== "branch" || checkout.upstream.state !== "tracked") return "";
  const { ahead, behind } = checkout.upstream;
  return [ahead ? `↑${ahead}` : "", behind ? `↓${behind}` : ""].filter(Boolean).join(" ");
}
