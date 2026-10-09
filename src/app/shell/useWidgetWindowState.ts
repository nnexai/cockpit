import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { solveLayout, type Rect } from "../layout/solveLayout";
import { useWidgets, widgetKey, type WidgetStore } from "../widgets/widgetStore";
import type { WorkbenchProps } from "./Workbench";
import type { WorkareaController } from "./useWorkarea";
import type { Tab } from "./model";
import type { Space } from "../sidebar/spaceTree";
export interface WidgetWindowState {
  canvasRef: RefObject<HTMLDivElement | null>; area: Rect; widgets: WidgetStore;
  widgetDots: Set<string>; widgetsPending: boolean; showWidgets(): void;
}
export function useWidgetWindowState({ client, ctx, state, tabLayout, selection, onWidgetAnnouncement }: WorkbenchProps, workarea: WorkareaController, selectedTab: Tab | undefined, sourcePaneId: string | null, spaces: Space[], allTabs: Tab[], tabs: Tab[]): WidgetWindowState {
  const terminalVisible = workarea.view.kind === "terminal";
  const canvasRef = useRef<HTMLDivElement>(null);
  const [area, setArea] = useState<Rect>({ x: 0, y: 0, width: 800, height: 600 });
  const widgets = useWidgets(client);
  const widgetRevision = widgets.getSnapshot();
  const [widgetWindow, setWidgetWindow] = useState(() => ({ visible: document.visibilityState === "visible", dragging: document.body.classList.contains("is-pane-dragging") }));
  useEffect(() => {
    const update = () => setWidgetWindow(current => {
      const visible = document.visibilityState === "visible", dragging = document.body.classList.contains("is-pane-dragging");
      return current.visible === visible && current.dragging === dragging ? current : { visible, dragging };
    });
    const observer = new MutationObserver(update);
    observer.observe(document.body, { attributes: true, attributeFilter: ["class"] });
    document.addEventListener("visibilitychange", update); window.addEventListener("pointerup", update);
    return () => { observer.disconnect(); document.removeEventListener("visibilitychange", update); window.removeEventListener("pointerup", update); };
  }, []);
  useEffect(() => widgets.retain(), [widgets]);
  useLayoutEffect(() => {
    const blockerFor = (paneId?: string) => {
      const tab = selectedTab ? ctx.getState().tabs[selectedTab.id] : null;
      const besideId = paneId && tab?.terminals[paneId] ? paneId : tab?.lastRealLeafId;
      const besideWidth = tab?.root && besideId ? solveLayout(tab.root, area).leaves.get(besideId)?.width ?? area.width : area.width;
      return !terminalVisible ? "library" as const : document.body.classList.contains("is-pane-dragging") ? "drag" as const : tab?.zoomLeafId && tab.zoomLeafId !== `${tab.tabId}:widget` ? "zoom" as const : !tab?.viewers.widget && besideWidth * (tab?.widgetShare ?? 0.4) < 320 ? "too_narrow" as const : null;
    };
    widgets.bind(ctx, (widget, docked) => {
      const space = spaces.find(candidate => candidate.id === widget.space_id)?.label ?? "Space";
      const tab = allTabs.find(candidate => candidate.id === widget.key.tab_id);
      const number = tab?.number ?? 1;
      onWidgetAnnouncement(docked
        ? `Widget “${widget.title}” opened beside the terminal.` : `Widget “${widget.title}” is in Space ${space}, tab ${number}.`);
    }, widget => blockerFor(widget.source?.pane_id));
    const pending = selectedTab ? widgets.widgets(ctx.sessionId, selectedTab.id).reverse().find(widget => widget.arrival === "own_tab" && !widgets.everDisplayed.has(widgetKey(widget.key))) : undefined;
    widgets.updateWindow({ session_id: state.sessionId, displayed_tab_id: widgetWindow.visible ? selectedTab?.id ?? null : null, blocker: blockerFor(pending?.source?.pane_id ?? sourcePaneId ?? undefined) });
  }, [widgets, widgetRevision, ctx, selectedTab, terminalVisible, tabLayout, area, widgetWindow, state.sessionId, sourcePaneId, spaces, allTabs, onWidgetAnnouncement]);
  const widgetDots = new Set(tabs.filter(tab => widgets.tabHasDot(ctx.sessionId, tab.id)).map(tab => tab.id));
  const widgetsPending = Boolean(selectedTab && widgets.needsClick(ctx.sessionId, selectedTab.id));
  const showWidgets = () => {
    if (!selectedTab) return;
    workarea.leave("keep");
    if (tabLayout?.zoomLeafId) ctx.dispatch({ type: "zoom-toggle", tabId: selectedTab.id, leafId: tabLayout.zoomLeafId });
    widgets.show(selectedTab.id);
  };
  useLayoutEffect(() => {
    const element = Array.from(canvasRef.current?.querySelectorAll<HTMLElement>(".tab-canvas") ?? []).find(node => node.closest<HTMLElement>("[data-tab-id]")?.dataset.tabId === selection.tabId);
    if (!element || !terminalVisible) return;
    const measure = () => {
      const rect = element.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) setArea(current => current.width === rect.width && current.height === rect.height ? current : { x: 0, y: 0, width: rect.width, height: rect.height });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(element);
    window.addEventListener("resize", measure);
    return () => { observer?.disconnect(); window.removeEventListener("resize", measure); };
  }, [terminalVisible, selection.tabId, tabLayout?.zoomLeafId]);
  return { canvasRef, area, widgets, widgetDots, widgetsPending, showWidgets };
}
