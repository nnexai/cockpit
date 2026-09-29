import type { MouseEvent } from "react";
import type { Leaf } from "./splitTree";

export const PANE_BADGE = { terminal: "T", files: "F", review: "R", browser: "B" } as const;

export interface PaneChromeProps {
  leaf: Leaf;
  title: string;
  subtitle?: string | null;
  selected?: boolean;
  zoomed?: boolean;
  lastTerminal?: boolean;
  focusStatus?: "pending" | "error" | null;
  focusError?: string | null;
  onRetryFocus?: () => void;
  onSplit: (direction: "right" | "down") => void;
  onZoom: () => void;
  onClose: () => void;
  onMenu?: (event: MouseEvent<HTMLButtonElement>) => void;
  splitDisabled?: boolean;
  closeDisabled?: boolean;
}

function PaneIcon({ name }: { name: "right" | "down" | "zoom" | "restore" | "close" }) {
  return <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
    {name === "right" || name === "down"
      ? <><rect x="2" y="3" width="12" height="10" rx="1" /><path d={name === "right" ? "M8 3v10" : "M2 8h12"} /></>
      : <path d={{ zoom: "M6 2H2v4M10 2h4v4M6 14H2v-4M10 14h4v-4", restore: "M2 6h4V2M14 6h-4V2M2 10h4v4M14 10h-4v4", close: "M4 4l8 8M12 4l-8 8" }[name]} />}
  </svg>;
}

export function PaneChrome({ leaf, title, subtitle, selected, zoomed, lastTerminal, focusStatus, focusError, onRetryFocus, onSplit, onZoom, onClose, onMenu, splitDisabled, closeDisabled }: PaneChromeProps) {
  const closeLabel = leaf.kind === "terminal" && lastTerminal
    ? "Close last terminal (closes all viewers in this tab)"
    : leaf.kind === "browser" ? "Close Browser (stops it and deletes its profile: cookies, logins, site data)" : `Close ${title}`;
  return <header className="pane-header pane-chrome" data-pane-header={leaf.id} data-selected={selected || undefined} tabIndex={-1} title="Drag to move or swap this pane">
    <span className="pane-kind-badge" data-kind={leaf.kind} aria-label={leaf.kind}>{PANE_BADGE[leaf.kind]}</span>
    <span className="pane-title" data-pane-title title={title}>{title}</span>
    {subtitle && <span className="pane-subtitle" title={subtitle}>{subtitle}</span>}
    <span className="pane-scope">{leaf.kind === "terminal" ? "Herdr terminal" : "local to this tab"}</span>
    {focusStatus === "pending" && <span className="focus-label" role="status" aria-label="Focus pending">⟳</span>}
    {focusStatus === "error" && <>
      <span className="focus-label is-error" role="status" title={focusError ?? "Focus failed"} aria-label={focusError ?? "Focus failed"}>!</span>
      {onRetryFocus && <button type="button" className="pane-control" aria-label="Retry pane focus" title="Retry pane focus" onClick={onRetryFocus}>↻</button>}
    </>}
    <button type="button" className="pane-control" aria-label="Split right: new terminal" title="Split right: new terminal" disabled={splitDisabled} onClick={() => onSplit("right")}><PaneIcon name="right" /></button>
    <button type="button" className="pane-control" aria-label="Split down: new terminal" title="Split down: new terminal" disabled={splitDisabled} onClick={() => onSplit("down")}><PaneIcon name="down" /></button>
    <button type="button" className="pane-control" aria-label={zoomed ? "Restore layout" : "Zoom this pane"} title={`${zoomed ? "Restore layout" : "Zoom this pane"} (Ctrl+B z)`} onClick={onZoom}><PaneIcon name={zoomed ? "restore" : "zoom"} /></button>
    <button type="button" className="pane-control" aria-label={closeLabel} title={closeLabel} disabled={closeDisabled} onClick={onClose}><PaneIcon name="close" /></button>
    {onMenu && <button type="button" className="pane-control pane-menu-control" aria-label={`Actions for ${title}`} title={`Actions for ${title}`} onClick={onMenu}>⋯</button>}
  </header>;
}
