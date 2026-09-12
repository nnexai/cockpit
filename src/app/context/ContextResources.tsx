import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextRoot, ContextSnapshotResponse } from "../../protocol/generated/v1";
import { SnapshotImport } from "./SnapshotImport";
import { SourceImport } from "./SourceImport";

export function ContextResources({
  client,
  sessionId,
  paneId,
  bindingId,
  root,
  onChanged,
  onImported,
  onClose,
}: {
  client: CockpitClient;
  sessionId: string;
  paneId: string;
  bindingId: string;
  root: ContextRoot;
  onChanged: (paths: string[]) => void;
  onImported: (result: ContextSnapshotResponse) => void;
  onClose: () => void;
}) {
  if (root.kind !== "companion") return null;
  return <div className="context-resources" role="dialog" aria-modal="true" aria-label="Context resources">
    <header><strong>Context resources</strong><button type="button" onClick={onClose}>Close</button></header>
    <SourceImport client={client} sessionId={sessionId} paneId={paneId} bindingId={bindingId} rootId={root.root_id} onChanged={onChanged} />
    <SnapshotImport client={client} sessionId={sessionId} paneId={paneId} bindingId={bindingId} rootId={root.root_id} onImported={onImported} />
  </div>;
}
