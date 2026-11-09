import { useEffect, useMemo, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { subscribeLocalChanges } from "../localChanges";

export type SpaceWatch = { counter: number; status: "pending" | "watching" | "unavailable" };
export type SpaceStateWatch = { git: SpaceWatch; tags: SpaceWatch };
const INITIAL: SpaceStateWatch = { git: { counter: 0, status: "pending" }, tags: { counter: 0, status: "pending" } };

/** Independent streams share the backend SpaceWatch and follow the live session incarnation. */
export function useSpaceStateStreams(client: CockpitClient, sessionId: string | null, serverInstance: string | null): SpaceStateWatch {
  const binding = useMemo(() => ({ value: INITIAL }), [client, sessionId, serverInstance]);
  const [, render] = useState(0);
  useEffect(() => {
    if (!sessionId || !serverInstance) return;
    let disposed = false;
    const publish = (topic: keyof SpaceStateWatch, status: SpaceWatch["status"], invalidate = false) => {
      if (disposed) return;
      const previous = binding.value[topic];
      if (!invalidate && previous.status === status) return;
      binding.value = { ...binding.value, [topic]: { status, counter: previous.counter + (invalidate ? 1 : 0) } };
      render(n => n + 1);
    };
    const subscribe = (topic: keyof SpaceStateWatch) => subscribeLocalChanges(client,
      { kind: topic === "git" ? "space_git" : "space_tags", session_id: sessionId, server_instance: serverInstance },
      async event => {
        if (event.kind === "unavailable") publish(topic, "unavailable");
        else if (event.kind === "ready" || event.kind === "changed") publish(topic, "watching", true);
      },
      () => publish(topic, "unavailable"),
    );
    const git = subscribe("git");
    const tags = subscribe("tags");
    return () => { disposed = true; git.close(); tags.close(); };
  }, [binding, client, sessionId, serverInstance]);
  return binding.value;
}
