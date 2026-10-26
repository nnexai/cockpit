import { useEffect, useLayoutEffect, useRef, useState, type ComponentProps, type RefObject } from "react";
import type { SessionSummary } from "../../protocol/generated/v1";
import type { SessionState } from "../session/sessionStore";
import { registerSidebarFocus } from "../input/shortcuts";
import { Agents } from "./Agents";
import { deriveAgentQueue } from "./agentAttentionModel";
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
  const sectionsRef = useRef<HTMLDivElement>(null);
  const fittedRows = useFittedSectionRows(sectionsRef);
  const spaceCount = spaces.spaces.length;
  const agentCount = agents.snapshot ? deriveAgentQueue(agents.snapshot, agents.attention.response, agents.attention.availability).visible.length : 0;
  const total = spaceCount + agentCount;
  const sectionRows = fittedRows ?? (total === 0
    ? "minmax(0, 1fr) minmax(0, 1fr)"
    : `minmax(0, ${total + spaceCount}fr) minmax(0, ${total + agentCount}fr)`);
  return <>
    <SidebarHeader session={session} sync={sync} hasSnapshot={hasSnapshot} narrow={narrow} onSession={onSession} onClose={onClose} closeRef={closeRef} />
    <div className="sidebar-sections" ref={sectionsRef} style={{ gridTemplateRows: sectionRows }}>
      <Spaces {...spaces} loading={loading} hasSession={hasSession} focusRef={spacesFocus} />
      <Agents {...agents} loading={loading} hasSession={hasSession} focusRef={agentsFocus} />
    </div>
  </>;
}

/**
 * Rows that keep Agents directly under the Spaces content while the lists fit, or while one side needs at most a
 * third of the height. Null when both overflow: then each section keeps a third and entry counts share the rest.
 */
function useFittedSectionRows(sectionsRef: RefObject<HTMLDivElement | null>): string | null {
  const [rows, setRows] = useState<string | null>(null);
  useLayoutEffect(() => {
    const container = sectionsRef.current;
    if (!container) return;
    const measure = () => {
      const [spacesSection, agentsSection] = Array.from(container.children) as HTMLElement[];
      if (!spacesSection || !agentsSection) return;
      const height = container.clientHeight;
      const spacesHeight = naturalSectionHeight(spacesSection);
      const agentsHeight = naturalSectionHeight(agentsSection);
      const next = spacesHeight + agentsHeight <= height || spacesHeight <= height / 3 ? `${spacesHeight}px minmax(0, 1fr)`
        : agentsHeight <= height / 3 ? `minmax(0, 1fr) ${agentsHeight}px`
        : null;
      setRows(current => current === next ? current : next);
    };
    // Row sizes change without a Sidebar render (expanded groups, Git notes), so watch the rows themselves.
    const resize = new ResizeObserver(measure);
    const observeRows = () => {
      resize.disconnect();
      resize.observe(container);
      for (const list of container.querySelectorAll(".space-list, .agent-list")) for (const row of list.children) resize.observe(row);
    };
    const mutations = new MutationObserver(() => { observeRows(); measure(); });
    observeRows();
    measure();
    mutations.observe(container, { childList: true, subtree: true });
    return () => { resize.disconnect(); mutations.disconnect(); };
  }, [sectionsRef]);
  return rows;
}

/** The section height that shows every list row without scrolling. */
function naturalSectionHeight(section: HTMLElement): number {
  const list = section.querySelector<HTMLElement>(".space-list, .agent-list");
  if (!list) return Math.ceil(section.scrollHeight);
  const last = list.lastElementChild as HTMLElement | null;
  const content = last ? last.getBoundingClientRect().bottom - list.getBoundingClientRect().top + list.scrollTop + parseFloat(getComputedStyle(last).marginBottom) : 0;
  return Math.ceil(section.getBoundingClientRect().height - list.clientHeight + content + parseFloat(getComputedStyle(list).paddingBottom));
}
