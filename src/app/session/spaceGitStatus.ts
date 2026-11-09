import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { PaneSummary, SpaceGitStatus, SpaceSummary } from "../../protocol/generated/v1";
import type { SpaceWatch } from "./spaceStateStreams";

export const SPACE_GIT_POLL_MS = 15_000;

/**
 * A key that changes when the folder any Space shows a branch for changes, so a new Space is read at once.
 * That is the Space's checkout, else its first pane's folder, the same choice Cockpit makes when reading Git.
 */
export function spaceCheckoutKey(spaces: readonly SpaceSummary[], panes: readonly PaneSummary[] = []): string {
  return spaces.map((space) => `${space.id}\u0000${space.git?.checkout_path ?? panes.find((pane) => pane.space_id === space.id && pane.cwd)?.cwd ?? ""}`).join("\u0001");
}

/** Reads initially and on watch changes; polling is a fallback for unavailable watches. */
export function useSpaceGitStatus(client: CockpitClient, sessionId: string | null, checkoutKey: string, intervalMs = SPACE_GIT_POLL_MS, watch: SpaceWatch = FALLBACK, serverInstance: string | null = null) {
  const [status, setStatus] = useState<{ client: CockpitClient; sessionId: string; serverInstance: string | null; checkoutKey: string; spaces: ReadonlyMap<string, SpaceGitStatus>; error?: string } | null>(null);
  // Compare before calling setState: updater bailouts can still invoke the consumer.
  const statusRef = useRef(status);
  const refreshRef = useRef<(() => void) | null>(null);
  const pollRef = useRef<(() => void) | null>(null);
  const counterRef = useRef(watch.counter);
  const refresh = useCallback(() => refreshRef.current?.(), []);
  useEffect(() => {
    if (!sessionId || !serverInstance || !checkoutKey) return;
    let controller: AbortController | null = null;
    let retry: number | undefined;
    const poll = (force = false) => {
      if (!force && (globalThis.document?.visibilityState === "hidden" || controller)) return;
      clearTimeout(retry);
      if (force) controller?.abort();
      const current = new AbortController();
      controller = current;
      client.spaceGitStatus(sessionId, current.signal).then((response) => {
        if (current.signal.aborted) return;
        if (response.session_id !== sessionId || response.server_instance !== serverInstance) {
          throw new Error("Git status response does not match the active session");
        }
        clearTimeout(retry);
        const previous = statusRef.current;
        const sameBinding = previous?.client === client && previous.sessionId === sessionId && previous.serverInstance === serverInstance && previous.checkoutKey === checkoutKey;
        const spaces = sameBinding && sameGitSpaces(previous.spaces, response.spaces)
          ? previous.spaces : new Map(response.spaces.map((space) => [space.space_id, space]));
        if (sameBinding && spaces === previous.spaces && previous.error === undefined) return;
        const next = { client, sessionId, serverInstance, checkoutKey, spaces };
        statusRef.current = next;
        setStatus(next);
      }).catch((error: unknown) => {
        if (current.signal.aborted) return;
        if (document.visibilityState !== "hidden") retry = window.setTimeout(() => poll(), intervalMs);
        const previous = statusRef.current;
        const sameBinding = previous?.client === client && previous.sessionId === sessionId && previous.serverInstance === serverInstance && previous.checkoutKey === checkoutKey;
        const message = error instanceof Error ? error.message : String(error);
        if (sameBinding && previous.error === message) return;
        const next = { client, sessionId, serverInstance, checkoutKey, spaces: sameBinding ? previous.spaces : EMPTY, error: message };
        statusRef.current = next;
        setStatus(next);
      }).finally(() => { if (controller === current) controller = null; });
    };
    const visiblePoll = () => {
      if (document.visibilityState === "hidden") { clearTimeout(retry); controller?.abort(); controller = null; }
      else poll();
    };
    refreshRef.current = () => poll(true);
    pollRef.current = () => poll();
    poll();
    globalThis.document?.addEventListener("visibilitychange", visiblePoll);
    return () => {
      refreshRef.current = null;
      pollRef.current = null;
      globalThis.document?.removeEventListener("visibilitychange", visiblePoll);
      clearTimeout(retry);
      controller?.abort();
    };
  }, [client, sessionId, serverInstance, checkoutKey, intervalMs]);
  useEffect(() => {
    if (watch.status !== "unavailable" || !sessionId || !serverInstance || !checkoutKey) return;
    let timer: ReturnType<typeof setInterval> | undefined;
    const updateTimer = () => {
      clearInterval(timer);
      timer = document.visibilityState === "hidden" ? undefined : setInterval(() => pollRef.current?.(), intervalMs);
    };
    document.addEventListener("visibilitychange", updateTimer);
    updateTimer();
    return () => { clearInterval(timer); document.removeEventListener("visibilitychange", updateTimer); };
  }, [watch.status, intervalMs, sessionId, serverInstance, checkoutKey]);
  useEffect(() => {
    if (counterRef.current !== watch.counter) {
      counterRef.current = watch.counter;
      if (document.visibilityState !== "hidden") refreshRef.current?.();
    }
  }, [watch.counter]);
  const current = status?.client === client && status.sessionId === sessionId && status.serverInstance === serverInstance && status.checkoutKey === checkoutKey ? status : null;
  return { spaces: current?.spaces ?? EMPTY, error: current?.error, refresh };
}

const FALLBACK: SpaceWatch = { counter: 0, status: "unavailable" };
const EMPTY: ReadonlyMap<string, SpaceGitStatus> = new Map();

function sameGitSpaces(previous: ReadonlyMap<string, SpaceGitStatus>, spaces: readonly SpaceGitStatus[]): boolean {
  if (previous.size !== spaces.length) return false;
  const entries = previous.values();
  return spaces.every(space => {
    const before = entries.next().value;
    return before !== undefined && before.space_id === space.space_id && before.source === space.source
      && sameCheckout(before.checkout, space.checkout);
  });
}

function sameCheckout(previous: SpaceGitStatus["checkout"], checkout: SpaceGitStatus["checkout"]): boolean {
  if (previous.state !== checkout.state || previous.root !== checkout.root) return false;
  switch (checkout.state) {
    case "detached": return true;
    case "unavailable": return previous.state === "unavailable" && previous.code === checkout.code && previous.message === checkout.message;
    case "branch": return previous.state === "branch" && previous.branch === checkout.branch && sameUpstream(previous.upstream, checkout.upstream);
  }
}

type Upstream = Extract<SpaceGitStatus["checkout"], { state: "branch" }>["upstream"];

function sameUpstream(previous: Upstream, upstream: Upstream): boolean {
  if (previous.state !== upstream.state) return false;
  switch (upstream.state) {
    case "none": return true;
    case "tracked": return previous.state === "tracked" && previous.name === upstream.name && previous.ahead === upstream.ahead && previous.behind === upstream.behind;
    case "gone":
    case "local": return (previous.state === "gone" || previous.state === "local") && previous.name === upstream.name;
    case "unavailable": return previous.state === "unavailable" && previous.name === upstream.name && previous.code === upstream.code && previous.message === upstream.message;
  }
}

/** Herdr's compact branch position, e.g. "↑14" or "↑2 ↓1"; empty when level or without an upstream. */
export function aheadBehindLabel(status: SpaceGitStatus | undefined): string {
  const checkout = status?.checkout;
  if (checkout?.state !== "branch" || checkout.upstream.state !== "tracked") return "";
  const { ahead, behind } = checkout.upstream;
  return [ahead ? `↑${ahead}` : "", behind ? `↓${behind}` : ""].filter(Boolean).join(" ");
}
