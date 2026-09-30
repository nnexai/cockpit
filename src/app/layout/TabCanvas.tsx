import { useLayoutEffect, useMemo, useRef, type CSSProperties, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { applyDrop, leaves, setPairWeights, type Leaf } from "./splitTree";
import { computeDrop, DRAG_PX, solveLayout, type DividerHandle, type DropTarget, type Rect } from "./solveLayout";
import type { LayoutAction, TabLayoutState } from "./tabLayoutStore";
import { PANE_BADGE } from "./PaneChrome";
import "./tabCanvas.css";

export interface TabCanvasProps {
  tab: TabLayoutState;
  area: Rect;
  renderLeaf: (leaf: Leaf, rect: Rect) => ReactNode;
  dispatch: (action: LayoutAction) => void;
  registerTransient: (cancel: () => void) => () => void;
  announce: (text: string) => void;
  inputBlocked?: boolean;
}

const dividerKey = (divider: DividerHandle) => `${divider.splitId}:${divider.index}`;
const rectStyle = (rect: Rect): CSSProperties => ({ left: rect.x, top: rect.y, width: rect.width, height: rect.height });
function writeRect(element: HTMLElement, rect: Rect) {
  element.style.left = `${rect.x}px`;
  element.style.top = `${rect.y}px`;
  element.style.width = `${rect.width}px`;
  element.style.height = `${rect.height}px`;
}
function contains(outer: Rect, inner: Rect) {
  return inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.width <= outer.x + outer.width && inner.y + inner.height <= outer.y + outer.height;
}
function focusLeaf(canvas: HTMLElement, id: string | null) {
  const host = Array.from(canvas.querySelectorAll<HTMLElement>("[data-leaf-id]")).find(node => node.dataset.leafId === id);
  const target = host?.querySelector<HTMLElement>('textarea:not([disabled]), input:not([disabled]), [contenteditable="true"], [tabindex="0"]') ?? host;
  target?.focus({ preventScroll: true });
}

/** Used by Ctrl+B r; prefer the selected leaf's following divider, right then below. */
export function focusSelectedDivider(canvas: HTMLElement, selectedLeafId: string | null): boolean {
  if (!selectedLeafId) return false;
  const dividers = Array.from(canvas.querySelectorAll<HTMLElement>('[data-layout-divider]'));
  for (const side of ["a", "b"] as const) {
    for (const dir of ["row", "col"]) {
      const divider = dividers.find(node => node.dataset.dir === dir && (JSON.parse(node.dataset[side === "a" ? "aLeaves" : "bLeaves"] ?? "[]") as string[]).includes(selectedLeafId));
      if (divider) { divider.focus({ preventScroll: true }); return true; }
    }
  }
  return false;
}

export function TabCanvas(props: TabCanvasProps) {
  const { tab, area, renderLeaf, dispatch, registerTransient, announce } = props;
  const canvas = useRef<HTMLDivElement>(null);
  const hosts = useRef(new Map<string, HTMLElement>());
  const dividerNodes = useRef(new Map<string, HTMLElement>());
  const cancelTransient = useRef<(() => void) | null>(null);
  const latest = useRef(props);
  latest.current = props;
  const members = useMemo(() => leaves(tab.root), [tab.root]);
  // A stable creation-order list prevents DOM moves (and focus loss) when tree order changes.
  const order = useRef<string[]>([]);
  const memberMap = new Map(members.map(leaf => [leaf.id, leaf]));
  order.current = order.current.filter(id => memberMap.has(id));
  for (const leaf of members) if (!order.current.includes(leaf.id)) order.current.push(leaf.id);
  const solved = useMemo(() => tab.root ? solveLayout(tab.root, { ...area, x: 0, y: 0 }) : { leaves: new Map<string, Rect>(), dividers: [], degraded: false }, [tab.root, area.x, area.y, area.width, area.height]);
  const layout = useRef(solved);
  layout.current = solved;
  const titleFor = (id: string | null) => id ? hosts.current.get(id)?.querySelector("[data-pane-title]")?.textContent || (memberMap.get(id)?.kind === "terminal" ? id : memberMap.get(id)?.kind) || id : "pane";

  useLayoutEffect(() => {
    const unregister = registerTransient(() => cancelTransient.current?.());
    return () => { cancelTransient.current?.(); unregister(); };
  }, [registerTransient, tab.tabId]);
  useLayoutEffect(() => {
    cancelTransient.current?.();
  }, [tab.tabId, tab.revision, tab.zoomLeafId, area.width, area.height, props.inputBlocked]);
  useLayoutEffect(() => {
    for (const leaf of members) {
      const host = hosts.current.get(leaf.id);
      const title = host?.querySelector("[data-pane-title]")?.textContent;
      if (title) host!.setAttribute("aria-label", `${leaf.kind} pane: ${title}`);
    }
  });

  const restoreStyles = () => {
    for (const [id, rect] of layout.current.leaves) {
      const host = hosts.current.get(id);
      if (host) writeRect(host, latest.current.tab.zoomLeafId === id
        ? { x: 0, y: 0, width: latest.current.area.width, height: latest.current.area.height } : rect);
    }
    for (const divider of layout.current.dividers) {
      const node = dividerNodes.current.get(dividerKey(divider));
      if (node) writeRect(node, divider.rect);
    }
  };

  const beginDrag = (event: ReactPointerEvent<HTMLElement>, leaf: Leaf) => {
    if (event.button !== 0) return;
    dispatch({ type: "select-leaf", tabId: tab.tabId, leafId: leaf.id });
    const target = event.target as HTMLElement;
    if (!target.closest("[data-pane-header]") || target.closest("button,input,textarea,select,a,[contenteditable=true]") || tab.zoomLeafId || members.length < 2) return;
    // Header drag must not blur an editor or terminal already focused in this leaf.
    event.preventDefault();
    cancelTransient.current?.();
    const revision = tab.revision;
    const startX = event.clientX, startY = event.clientY, pointer = event.pointerId;
    let active = false;
    let drop: DropTarget | null = null;
    let ghost: HTMLDivElement | null = null;
    let preview: HTMLDivElement | null = null;
    const host = event.currentTarget;
    const dropAt = (x: number, y: number) => {
      const bounds = canvas.current!.getBoundingClientRect();
      const boundsArea = { x: 0, y: 0, width: area.width, height: area.height };
      const candidate = computeDrop({ x: x - bounds.left, y: y - bounds.top }, leaf.id, layout.current.leaves, boundsArea, members.length);
      if (candidate && candidate.kind !== "swap" && tab.root && solveLayout(applyDrop(tab.root, leaf.id, candidate), boundsArea).degraded) return null;
      return candidate;
    };
    const finish = (commit: boolean) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key, true);
      cancelTransient.current = null;
      host.classList.remove("is-dragging");
      document.body.classList.remove("is-pane-dragging");
      ghost?.remove(); preview?.remove();
      if (commit && active && drop && latest.current.tab.tabId === tab.tabId && latest.current.tab.revision === revision) {
        dispatch({ type: "drop", tabId: tab.tabId, src: leaf.id, target: drop, revision });
        announce(drop.kind === "swap" ? `${titleFor(leaf.id)} swapped with ${titleFor(drop.target)}.` : `${titleFor(leaf.id)} moved to ${drop.kind === "root" ? "outer " : ""}${drop.side} edge.`);
      }
    };
    const move = (ev: PointerEvent) => {
      if (ev.pointerId !== pointer) return;
      if (!active && Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_PX) return;
      if (!active) {
        active = true;
        host.classList.add("is-dragging");
        document.body.classList.add("is-pane-dragging");
        ghost = document.createElement("div"); ghost.className = "pane-drag-ghost";
        const badge = document.createElement("span"); badge.className = "pane-kind-badge"; badge.dataset.kind = leaf.kind; badge.textContent = PANE_BADGE[leaf.kind];
        const title = document.createElement("span"); title.textContent = titleFor(leaf.id);
        ghost.append(badge, title); document.body.append(ghost);
        preview = document.createElement("div"); preview.className = "pane-drop-preview"; preview.append(document.createElement("span")); canvas.current!.append(preview);
      }
      ev.preventDefault();
      ghost!.style.left = `${ev.clientX + 12}px`; ghost!.style.top = `${ev.clientY + 12}px`;
      drop = dropAt(ev.clientX, ev.clientY);
      preview!.hidden = !drop;
      if (drop) {
        writeRect(preview!, drop.rect);
        preview!.dataset.dropKind = drop.kind;
        preview!.dataset.dropZone = drop.kind === "swap" ? "center" : drop.side;
        preview!.dataset.dropTarget = drop.kind === "root" ? "workspace" : drop.target;
        preview!.firstChild!.textContent = drop.kind === "swap" ? drop.label : drop.kind === "root" ? `Outer ${drop.side} edge (${drop.sharePct}%)` : { left: "Left", right: "Right", top: "Above", bottom: "Below" }[drop.side];
      }
    };
    const up = (ev: PointerEvent) => { if (ev.pointerId === pointer) { if (active) drop = dropAt(ev.clientX, ev.clientY); finish(true); } };
    const cancel = () => finish(false);
    const key = (ev: KeyboardEvent) => { if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); cancel(); } };
    cancelTransient.current = cancel;
    window.addEventListener("pointermove", move, { passive: false }); window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel); window.addEventListener("keydown", key, true);
  };

  const pairAt = (divider: DividerHandle, target: number) => {
    const a = divider.dir === "row" ? divider.a.width : divider.a.height;
    const b = divider.dir === "row" ? divider.b.width : divider.b.height;
    const total = a + b;
    const size = total < divider.minA + divider.minB ? total / 2 : Math.max(divider.minA, Math.min(total - divider.minB, target));
    const weight = divider.wa + divider.wb;
    const wa = total > 0 ? weight * size / total : weight / 2;
    return { wa, wb: weight - wa, a: size, b: total - size };
  };
  const commitPair = (divider: DividerHandle, pair: { wa: number; wb: number }) => {
    dispatch({ type: "resize-commit", tabId: tab.tabId, splitId: divider.splitId, index: divider.index, wa: pair.wa, wb: pair.wb });
    const total = divider.dir === "row" ? divider.a.width + divider.b.width : divider.a.height + divider.b.height;
    const a = total * pair.wa / (pair.wa + pair.wb);
    announce(`Pane sizes ${Math.round(a)} and ${Math.round(total - a)} pixels.`);
  };
  const beginResize = (event: ReactPointerEvent<HTMLDivElement>, divider: DividerHandle) => {
    if (event.button !== 0 || !tab.root) return;
    event.preventDefault(); cancelTransient.current?.();
    const node = event.currentTarget, pointer = event.pointerId, revision = tab.revision;
    const start = divider.dir === "row" ? event.clientX : event.clientY;
    const base = divider.dir === "row" ? divider.a.width : divider.a.height;
    let pair = pairAt(divider, base), frame = 0;
    const readout = document.createElement("div"); readout.className = "pane-divider-readout"; document.body.append(readout);
    node.dataset.active = "true"; document.body.classList.add("is-pane-dragging");
    node.setPointerCapture?.(pointer);
    const showReadout = (x: number, y: number) => {
      readout.textContent = `${Math.round(pair.a)} | ${Math.round(pair.b)}`;
      readout.style.left = `${x + 14}px`; readout.style.top = `${y + 14}px`;
    };
    showReadout(event.clientX, event.clientY);
    const flush = () => {
      frame = 0;
      const live = solveLayout(setPairWeights(tab.root!, divider.splitId, divider.index, pair.wa, pair.wb), { ...area, x: 0, y: 0 });
      for (const [id, rect] of live.leaves) { const element = hosts.current.get(id); if (element) writeRect(element, rect); }
      for (const handle of live.dividers) {
        const element = dividerNodes.current.get(dividerKey(handle));
        if (element) {
          writeRect(element, handle.rect);
          element.setAttribute("aria-valuenow", String(Math.round(100 * handle.wa / (handle.wa + handle.wb))));
        }
      }
    };
    const finish = () => {
      window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel); window.removeEventListener("keydown", key, true);
      node.removeEventListener("lostpointercapture", lost);
      if (frame) cancelAnimationFrame(frame);
      cancelTransient.current = null; delete node.dataset.active;
      document.body.classList.remove("is-pane-dragging"); readout.remove();
      if (node.hasPointerCapture?.(pointer)) node.releasePointerCapture(pointer);
      if (latest.current.tab.tabId === tab.tabId && latest.current.tab.revision === revision && !latest.current.tab.zoomLeafId) {
        if (latest.current.area.width === area.width && latest.current.area.height === area.height) flush();
        else restoreStyles();
        commitPair(divider, pair);
      } else restoreStyles();
    };
    const move = (ev: PointerEvent) => {
      if (ev.pointerId !== pointer) return;
      ev.preventDefault(); pair = pairAt(divider, base + (divider.dir === "row" ? ev.clientX : ev.clientY) - start);
      showReadout(ev.clientX, ev.clientY);
      if (!frame) frame = requestAnimationFrame(flush);
    };
    const up = (ev: PointerEvent) => {
      if (ev.pointerId !== pointer) return;
      pair = pairAt(divider, base + (divider.dir === "row" ? ev.clientX : ev.clientY) - start); finish();
    };
    const cancel = () => finish();
    const lost = () => finish();
    const key = (ev: KeyboardEvent) => { if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); finish(); focusLeaf(canvas.current!, latest.current.tab.selectedLeafId); } };
    cancelTransient.current = cancel;
    node.addEventListener("lostpointercapture", lost);
    window.addEventListener("pointermove", move, { passive: false }); window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel); window.addEventListener("keydown", key, true);
  };

  const zoomed = memberMap.get(tab.zoomLeafId ?? "");
  return <div className="tab-canvas-layout" data-tab-id={tab.tabId} onKeyDown={event => {
    const target = event.target as HTMLElement;
    if (event.key === "Escape" && tab.zoomLeafId && target.closest("[data-pane-header]")) {
      event.preventDefault(); event.stopPropagation(); dispatch({ type: "zoom-toggle", tabId: tab.tabId });
    }
  }}>
    {solved.degraded && !zoomed && <div className="pane-layout-strip pane-degraded-strip" role="status">Panes are smaller than their usual minimum in this window. Zoom a pane or enlarge the window.</div>}
    <div className="pane-canvas tab-canvas" ref={canvas}>
      {order.current.map(id => {
        const leaf = memberMap.get(id)!;
        const hidden = !!zoomed && id !== zoomed.id;
        const rect = zoomed?.id === id ? { x: 0, y: 0, width: area.width, height: area.height } : solved.leaves.get(id)!;
        return <section key={id} className={`pane pane-view${tab.selectedLeafId === id ? " is-selected" : ""}`} data-leaf-id={id} data-kind={leaf.kind} data-selected={tab.selectedLeafId === id} data-zoomed={zoomed?.id === id || undefined} role="group" aria-label={`${leaf.kind} pane: ${titleFor(id)}`} aria-current={tab.selectedLeafId === id ? "true" : undefined} tabIndex={-1} hidden={hidden} style={rectStyle(rect)} ref={element => { if (element) hosts.current.set(id, element); else hosts.current.delete(id); }} onPointerDown={event => beginDrag(event, leaf)} onDoubleClick={event => {
          const target = event.target as HTMLElement;
          if (target.closest("[data-pane-header]") && !target.closest("button,input,textarea,select,a")) dispatch({ type: "zoom-toggle", tabId: tab.tabId, leafId: id });
        }}>
          {!hidden && renderLeaf(leaf, rect)}
        </section>;
      })}
      {!zoomed && solved.dividers.map(divider => <div key={dividerKey(divider)} className="pane-divider" data-layout-divider={dividerKey(divider)} data-dir={divider.dir} data-a-leaves={JSON.stringify(members.filter(leaf => contains(divider.a, solved.leaves.get(leaf.id)!)).map(leaf => leaf.id))} data-b-leaves={JSON.stringify(members.filter(leaf => contains(divider.b, solved.leaves.get(leaf.id)!)).map(leaf => leaf.id))} ref={element => { if (element) dividerNodes.current.set(dividerKey(divider), element); else dividerNodes.current.delete(dividerKey(divider)); }} style={rectStyle(divider.rect)} role="separator" tabIndex={0} aria-orientation={divider.dir === "row" ? "vertical" : "horizontal"} aria-label="Resize panes (arrow keys, double-click resets to 50:50)" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(100 * divider.wa / (divider.wa + divider.wb))} onPointerDown={event => beginResize(event, divider)} onDoubleClick={() => commitPair(divider, { wa: (divider.wa + divider.wb) / 2, wb: (divider.wa + divider.wb) / 2 })} onKeyDown={event => {
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); focusLeaf(canvas.current!, tab.selectedLeafId); return; }
        const steps: Record<string, number> = divider.dir === "row" ? { ArrowLeft: -24, ArrowRight: 24 } : { ArrowUp: -24, ArrowDown: 24 };
        const step = steps[event.key]; if (step === undefined) return;
        event.preventDefault(); event.stopPropagation();
        const base = divider.dir === "row" ? divider.a.width : divider.a.height;
        const pair = pairAt(divider, base + step * (event.shiftKey ? 4 : 1)); commitPair(divider, pair);
      }} />)}
    </div>
  </div>;
}
