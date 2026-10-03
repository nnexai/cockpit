import type { MouseEvent, ReactNode } from "react";
import type { Leaf } from "./splitTree";

export const PANE_BADGE = { terminal: "T", files: "F", review: "R", browser: "B", widget: "W" } as const;

export interface PaneChromeProps {
  leaf: Leaf;
  title: string;
  subtitle?: string | null;
  titleContent?: ReactNode;
  controls?: ReactNode;
  selected?: boolean;
  zoomed?: boolean;
  lastTerminal?: boolean;
  focusStatus?: "pending" | "error" | null;
  focusError?: string | null;
  onRetryFocus?: () => void;
  onZoom: () => void;
  onClose: () => void;
  onMenu?: (event: MouseEvent<HTMLButtonElement>) => void;
  closeDisabled?: boolean;
}

function PaneIcon({ name }: { name: "zoom" | "restore" | "close" }) {
  return <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
    <path d={{ zoom: "M6 2H2v4M10 2h4v4M6 14H2v-4M10 14h4v-4", restore: "M2 6h4V2M14 6h-4V2M2 10h4v4M14 10h-4v4", close: "M4 4l8 8M12 4l-8 8" }[name]} />
  </svg>;
}

export function PaneChrome({ leaf, title, subtitle, titleContent, controls, selected, zoomed, lastTerminal, focusStatus, focusError, onRetryFocus, onZoom, onClose, onMenu, closeDisabled }: PaneChromeProps) {
  const closeLabel = leaf.kind === "terminal" && lastTerminal
    ? "Close last terminal (closes all viewers in this tab)"
    : leaf.kind === "widget" ? `Remove widget: ${title}` : leaf.kind === "browser" ? "Close Browser (stops it and deletes its profile: cookies, logins, site data)" : `Close ${title}`;
  return <header className="pane-header pane-chrome" data-pane-header={leaf.id} data-selected={selected || undefined} tabIndex={-1} title="Drag to move or swap this pane">
    <span className="pane-kind-badge" data-kind={leaf.kind} aria-label={leaf.kind}>{PANE_BADGE[leaf.kind]}</span>
    {titleContent ?? <span className="pane-title" data-pane-title title={title}>{title}</span>}
    {subtitle && <span className="pane-subtitle" title={subtitle}>{subtitle}</span>}
    {leaf.kind !== "widget" && <span className="pane-scope">{leaf.kind === "terminal" ? "Herdr terminal" : "local to this tab"}</span>}
    {controls}
    {focusStatus === "pending" && <span className="focus-label" role="status" aria-label="Focus pending">⟳</span>}
    {focusStatus === "error" && <>
      <span className="focus-label is-error" role="status" title={focusError ?? "Focus failed"} aria-label={focusError ?? "Focus failed"}>!</span>
      {onRetryFocus && <button type="button" className="pane-control" aria-label="Retry pane focus" title="Retry pane focus" onClick={onRetryFocus}>↻</button>}
    </>}
    <button type="button" className="pane-control" aria-label={zoomed ? "Restore layout" : "Zoom this pane"} title={`${zoomed ? "Restore layout" : "Zoom this pane"} (Ctrl+B z)`} onClick={onZoom}><PaneIcon name={zoomed ? "restore" : "zoom"} /></button>
    <button type="button" className="pane-control" aria-label={closeLabel} title={closeLabel} disabled={closeDisabled} onClick={onClose}><PaneIcon name="close" /></button>
    {onMenu && <button type="button" className="pane-control pane-menu-control" aria-label={`Actions for ${title}`} title={`Actions for ${title}`} onClick={onMenu}>⋯</button>}
  </header>;
}
