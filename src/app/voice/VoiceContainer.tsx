import { useMemo, useState } from "react";
import { createVoiceTransport, type VoiceTransportApi } from "../../client/voiceTransport";
import type { HandoffBinding } from "../../protocol/generated/v1";
import { VoiceScreen, type VoiceChoice } from "./VoiceScreen";

export type VoiceContainerProps = {
  sessionId: string | null;
  roots: VoiceChoice[];
  projects: VoiceChoice[];
  workspaces: VoiceChoice[];
  api: VoiceTransportApi;
  onStopReady?: (stop: (() => Promise<void>) | null) => void;
};

/** Choices are supplied by the host; neither focus nor display labels establish identity. */
export function VoiceContainer({ sessionId, roots, projects, workspaces, api, onStopReady }: VoiceContainerProps) {
  const [selectedRootId, setSelectedRootId] = useState<string | null>(null);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [selectedWorkspaceId, setSelectedWorkspaceId] = useState<string | null>(null);

  const transport = useMemo(() => {
    if (!sessionId) return null;
    const projectScope: HandoffBinding | null = selectedProjectId === null && selectedWorkspaceId === null ? null : {
      repository_id: selectedProjectId,
      workspace_id: selectedWorkspaceId,
      branch: null,
      base_ref: null,
    };
    return createVoiceTransport(sessionId, api, { selectedRootId, projectScope });
  }, [sessionId, api, selectedRootId, selectedProjectId, selectedWorkspaceId]);

  if (!transport) {
    return <section className="voice-root" aria-label="Voice"><p>Select a session before starting voice.</p></section>;
  }

  return <VoiceScreen
    transport={transport}
    sessionId={sessionId}
    roots={roots}
    projects={projects}
    workspaces={workspaces}
    selectedRootId={selectedRootId}
    selectedProjectId={selectedProjectId}
    selectedWorkspaceId={selectedWorkspaceId}
    onRootChange={setSelectedRootId}
    onProjectChange={setSelectedProjectId}
    onWorkspaceChange={setSelectedWorkspaceId}
    onStopReady={onStopReady}
    supervisorName={roots.find((root) => root.id === selectedRootId)?.label}
    projectName={projects.find((project) => project.id === selectedProjectId)?.label}
  />;
}
