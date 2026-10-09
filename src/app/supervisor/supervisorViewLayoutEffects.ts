import { useLayoutEffect } from "react";
import { taskLanes } from "./boardNavigation";
import { writeOffset } from "./reveal";
import type { SupervisorViewInputs, SupervisorViewModel, SupervisorViewNavigation, SupervisorViewPanel } from "./supervisorViewTypes";

type LayoutContext = SupervisorViewInputs & SupervisorViewModel & SupervisorViewNavigation & SupervisorViewPanel;

export function useSupervisorViewRevealLayout(context: LayoutContext) {
  const { root, active, mode, graphRef, listRef, scope, layout, laneRefs, revealSelection, revealIntent, saveOffsets, workareaRef, setGraphViewportHeight, rootId, detailOpen, bounds, placement } = context;
  useLayoutEffect(() => {
    if (!root || !active) return;
    const scroller = mode !== "tasks" ? graphRef.current : listRef.current;
    if (scroller && scope.view.offsets[mode]) writeOffset(scroller, scope.view.offsets[mode]!);
    if (mode === "tasks" && !layout.narrow) for (const { lane } of taskLanes) { const list = laneRefs.current[lane]; if (list) list.scrollTop = scope.view.laneScroll[lane] ?? 0; }
    revealSelection(revealIntent.current?.focus ?? false, true);
    revealIntent.current = null;
    return saveOffsets;
  }, [scope, mode, active, !!root]);
  useLayoutEffect(() => {
    if (!revealIntent.current || !active) return;
    revealSelection(revealIntent.current.focus); revealIntent.current = null;
  });
  useLayoutEffect(() => {
    const graph = graphRef.current, workarea = workareaRef.current;
    if (mode === "tasks" || !active || !graph || !workarea) { setGraphViewportHeight(null); return; }
    const measure = () => {
      const available = Math.min(graph.clientHeight, workarea.getBoundingClientRect().bottom - graph.getBoundingClientRect().top);
      const next = available > 0 ? available : null;
      setGraphViewportHeight(current => current === next ? current : next);
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(graph); observer?.observe(workarea);
    return () => observer?.disconnect();
  }, [mode, active, rootId, !!root, layout.width, layout.height, layout.queueMode]);
  useLayoutEffect(() => { if (detailOpen && mode !== "tasks") revealSelection(false, true); }, [layout.width, layout.height, bounds?.value, placement]);
}

export function useSupervisorViewQueueLayout(context: LayoutContext & { rows: readonly unknown[] }) {
  const { workareaRef, rootRef, layout, setInlineQueueCap, mode, rootId, rows, banner, active } = context;
  useLayoutEffect(() => {
    const workarea = workareaRef.current, view = rootRef.current;
    if (!workarea || !view || layout.queueMode !== "inline") { setInlineQueueCap(null); return; }
    const chrome = [view.querySelector<HTMLElement>(".supervisor-summary"), view.querySelector<HTMLElement>(".supervisor-viewbar"), view.querySelector<HTMLElement>(".supervisor-strip")].filter((element): element is HTMLElement => !!element);
    const measure = () => {
      const height = workarea.getBoundingClientRect().height;
      if (!height) { setInlineQueueCap(null); return; }
      const chromeHeight = chrome.reduce((sum, element) => sum + element.getBoundingClientRect().height, 0);
      const wholeViewHeight = Math.max(height, view.getBoundingClientRect().height);
      // capPx bounds the scrollport, not the complete queue. Keep its footer and border inside the budget.
      const footer = Math.max(24, view.querySelector<HTMLElement>(".supervisor-queue-more")?.getBoundingClientRect().height ?? 0);
      const ceiling = height * (mode === "graph" ? 0.3 : 0.4);
      const boardBudget = mode === "tasks" ? height - chromeHeight - wholeViewHeight * 0.5 : ceiling;
      const next = Math.max(38, Math.floor(Math.min(ceiling, boardBudget) - footer - 1));
      setInlineQueueCap(current => current === next ? current : next);
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(workarea); observer?.observe(view);
    for (const element of chrome) observer?.observe(element);
    return () => observer?.disconnect();
  }, [layout.queueMode, layout.width, layout.height, mode, rootId, rows.length, banner, active]);
}
