import { useEffect, useMemo, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { SessionSnapshotResponse, SpaceTagsResponse } from "../../protocol/generated/v1";

export const SPACE_TAGS_POLL_MS = 15_000;

export type SpaceTagsState = {
  ready: boolean;
  failed: boolean;
  editBlocked: string | null;
  catalog: readonly string[];
  bySpace: ReadonlyMap<string, readonly string[]>;
  pending: ReadonlySet<string>;
  /** One acknowledged change per Space at a time; failures and unknown outcomes resolve false. */
  change(spaceId: string, tag: string, assigned: boolean): Promise<boolean>;
};

export function useSpaceTags(client: CockpitClient, sessionId: string | null, snapshot: SessionSnapshotResponse | null, live: boolean): SpaceTagsState {
  const serverInstance = snapshot?.session_id === sessionId ? snapshot.server_instance : null;
  const [, render] = useState(0);
  const binding = useMemo(() => ({
    ready: false, failed: false, catalog: [] as string[], bySpace: new Map<string, readonly string[]>(),
    pending: new Set<string>(), sequence: 0, lastApplied: 0, generation: 0,
    refresh: null as ((force?: boolean) => void) | null,
  }), [client, sessionId, serverInstance]);
  const current = useRef({ binding, live, snapshot });
  current.current = { binding, live, snapshot };

  useEffect(() => {
    if (!live || !sessionId || !serverInstance) return;
    ++binding.generation;
    let disposed = false;
    let controller: AbortController | null = null;
    let timer: number | undefined;
    const poll = (force = false) => {
      clearTimeout(timer);
      if (disposed || document.visibilityState === "hidden" || binding.pending.size > 0) return;
      if (controller) {
        if (!force) return;
        controller.abort();
      }
      const request = new AbortController();
      controller = request;
      const sequence = ++binding.sequence;
      void client.spaceTags(sessionId, request.signal).then(response => {
        if (disposed || request.signal.aborted || sequence <= binding.lastApplied) return;
        if (response.session_id !== sessionId || response.server_instance !== serverInstance) {
          if (!binding.ready) { binding.failed = true; render(value => value + 1); }
          return;
        }
        binding.lastApplied = sequence;
        binding.ready = true;
        binding.failed = false;
        binding.catalog = response.catalog;
        binding.bySpace = new Map(response.spaces.map(space => [space.space_id, space.tags]));
        render(value => value + 1);
      }, () => {
        if (!disposed && !request.signal.aborted && sequence > binding.lastApplied && !binding.ready) {
          binding.failed = true;
          render(value => value + 1);
        }
      }).finally(() => {
        if (controller !== request) return;
        controller = null;
        if (!disposed && document.visibilityState !== "hidden") timer = window.setTimeout(poll, SPACE_TAGS_POLL_MS);
      });
    };
    const visibilityChanged = () => {
      clearTimeout(timer);
      if (document.visibilityState === "hidden") {
        controller?.abort();
        controller = null;
      } else poll(true);
    };
    const focused = () => poll(true);
    binding.refresh = poll;
    document.addEventListener("visibilitychange", visibilityChanged);
    window.addEventListener("focus", focused);
    poll();
    return () => {
      disposed = true;
      ++binding.generation;
      binding.refresh = null;
      binding.pending.clear();
      clearTimeout(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", visibilityChanged);
      window.removeEventListener("focus", focused);
    };
  }, [binding, client, sessionId, serverInstance, live]);

  const editBlocked = !live ? "Herdr is not live" : !binding.ready ? binding.failed ? "Tags unavailable" : "Tags are loading" : null;
  const bySpace = new Map<string, readonly string[]>();
  const spaces = new Set(snapshot?.session_id === sessionId ? snapshot.spaces.map(space => space.id) : []);
  for (const [spaceId, tags] of binding.bySpace) if (spaces.has(spaceId)) bySpace.set(spaceId, tags);

  const change = async (spaceId: string, tag: string, assigned: boolean): Promise<boolean> => {
    const active = current.current;
    if (active.binding !== binding || !active.live || !binding.ready || !sessionId || !serverInstance
      || active.snapshot?.session_id !== sessionId || !active.snapshot.spaces.some(space => space.id === spaceId)
      || binding.pending.has(spaceId) || !binding.refresh) return false;
    const generation = binding.generation;
    binding.pending.add(spaceId);
    // Any read begun before this write is no longer authoritative.
    binding.lastApplied = ++binding.sequence;
    render(value => value + 1);
    const isCurrent = () => current.current.binding === binding && current.current.live && binding.generation === generation;
    try {
      const response: SpaceTagsResponse = await client.changeSpaceTag(sessionId, {
        space_id: spaceId, server_instance: serverInstance, tag: tag.normalize("NFC"), assigned,
      });
      if (!isCurrent() || response.session_id !== sessionId || response.server_instance !== serverInstance) return false;
      binding.lastApplied = ++binding.sequence;
      // Full write responses have no revision. Only this Space is known to be current:
      // another concurrently acknowledged Space must not be reverted by response order.
      const tags = response.spaces.find(space => space.space_id === spaceId)?.tags;
      if (tags?.length) binding.bySpace.set(spaceId, tags);
      else binding.bySpace.delete(spaceId);
      const known = new Set(binding.catalog.map(value => value.toLowerCase()));
      for (const value of response.catalog) {
        if (!known.has(value.toLowerCase())) { known.add(value.toLowerCase()); binding.catalog = [...binding.catalog, value]; }
      }
      binding.failed = false;
      return true;
    } catch {
      return false;
    } finally {
      if (isCurrent()) {
        binding.pending.delete(spaceId);
        render(value => value + 1);
        // Reconcile full state/catalog order after all concurrent writes, including unknown outcomes.
        if (binding.pending.size === 0) binding.refresh?.(true);
      }
    }
  };

  return { ready: binding.ready, failed: binding.failed, editBlocked, catalog: binding.catalog, bySpace, pending: live ? new Set(binding.pending) : new Set(), change };
}
