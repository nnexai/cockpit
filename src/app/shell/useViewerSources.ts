import { useEffect, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import { describeError } from "./model";
import type { ViewerSourcesState } from "./commands";
import type { ContextMenuState } from "./ContextMenu";

export interface ViewerSourcesInput {
  client: CockpitClient;
  menu: ContextMenuState | null;
  commandsOpen: boolean;
  sourcePaneId: string | null;
  sessionId: string | null;
  live: boolean;
}

export function useViewerSources({ client, menu, commandsOpen, sourcePaneId, sessionId, live }: ViewerSourcesInput): ViewerSourcesState {
  const [state, setState] = useState<ViewerSourcesState>({ status: "pending", source: Boolean(sourcePaneId) });
  useEffect(() => {
    if ((!menu && !commandsOpen) || !sourcePaneId || !sessionId || !live) {
      setState(current => current.status === "failed" ? current : { status: "pending", source: Boolean(sourcePaneId) });
      return;
    }
    let active = true;
    setState({ status: "pending", source: true });
    void client.viewerSources(sessionId, sourcePaneId).then(
      options => { if (active) setState({ status: "ready", options }); },
      error => { if (active) setState({ status: "failed", message: describeError(error, "Could not inspect viewer sources").message }); },
    );
    return () => { active = false; };
  }, [client, menu, commandsOpen, sourcePaneId, sessionId, live]);
  return state;
}
