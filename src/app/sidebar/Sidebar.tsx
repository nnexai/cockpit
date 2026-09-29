import { useEffect, useRef, type ComponentProps, type RefObject } from "react";
import type { SessionSummary } from "../../protocol/generated/v1";
import type { SessionState } from "../session/sessionStore";
import { registerSidebarFocus } from "../input/shortcuts";
import { Agents } from "./Agents";
import { SidebarHeader } from "./SidebarHeader";
import { Spaces } from "./Spaces";

type SidebarProps = {
  session: SessionSummary | undefined;
  sync: SessionState["sync"];
  narrow: boolean;
  onSession: () => void;
  onClose: () => void;
  closeRef?: RefObject<HTMLButtonElement | null>;
  spaces: Omit<ComponentProps<typeof Spaces>, "loading" | "hasSession" | "focusRef">;
  agents: Omit<ComponentProps<typeof Agents>, "loading" | "hasSession" | "focusRef">;
  hasSession: boolean;
  hasSnapshot: boolean;
};

/** The contents of the sidebar `aside`: session header, Spaces, Agents. The `aside` and its drawer behaviour stay with the workbench. */
export function Sidebar({ session, sync, narrow, onSession, onClose, closeRef, spaces, agents, hasSession, hasSnapshot }: SidebarProps) {
  const spacesFocus = useRef<(() => void) | null>(null);
  const agentsFocus = useRef<(() => void) | null>(null);
  useEffect(() => registerSidebarFocus({ spaces: () => spacesFocus.current?.(), agents: () => agentsFocus.current?.() }), []);
  const loading = hasSession && !hasSnapshot;
  return <>
    <SidebarHeader session={session} sync={sync} hasSnapshot={hasSnapshot} narrow={narrow} onSession={onSession} onClose={onClose} closeRef={closeRef} />
    <Spaces {...spaces} loading={loading} hasSession={hasSession} focusRef={spacesFocus} />
    <Agents {...agents} loading={loading} hasSession={hasSession} focusRef={agentsFocus} />
  </>;
}
