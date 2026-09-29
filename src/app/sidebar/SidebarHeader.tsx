import type { RefObject } from "react";
import type { SessionSummary } from "../../protocol/generated/v1";
import type { SessionState } from "../session/sessionStore";
import { withShortcut } from "../input/shortcuts";
import { UiIcon } from "../UiIcon";
import { StateGlyph, type GlyphShape } from "./StateGlyph";

type HeaderStatus = { shape: GlyphShape; tone?: "warning" | "blocked" | "muted"; chip: string | null; word: string };

/** The connection mark and state chip: a shape and a word, so the state is not colour only. */
export function headerStatus(hasSession: boolean, sync: SessionState["sync"], hasSnapshot: boolean): HeaderStatus {
  if (!hasSession) return { shape: "idle", tone: "muted", chip: null, word: "no session" };
  switch (sync) {
    case "live": return { shape: "live", chip: null, word: "connected" };
    case "loading": return hasSnapshot
      ? { shape: "working", tone: "warning", chip: "Resyncing", word: "resyncing" }
      : { shape: "working", tone: "warning", chip: "Connecting", word: "connecting" };
    case "stale": return { shape: "idle", tone: "warning", chip: "Stale", word: "stale" };
    case "disconnected": return { shape: "blocked", tone: "blocked", chip: "Offline", word: "disconnected" };
    default: return { shape: "working", tone: "warning", chip: "Connecting", word: "connecting" };
  }
}

export function SidebarHeader({ session, sync, hasSnapshot, narrow, onSession, onClose, closeRef }: {
  session: SessionSummary | undefined;
  sync: SessionState["sync"];
  hasSnapshot: boolean;
  narrow: boolean;
  onSession: () => void;
  onClose: () => void;
  closeRef?: RefObject<HTMLButtonElement | null>;
}) {
  const status = headerStatus(Boolean(session), sync, hasSnapshot);
  const name = session?.label ?? "No session";
  const switchTitle = withShortcut("Switch session", "switch-session");
  return <header className="sidebar-header">
    <button type="button" className="session-selector" onClick={onSession}
      aria-label={session ? `Switch session, current ${session.label}, ${status.word}` : "Switch session"}
      title={session ? `${session.label} · ${switchTitle}` : switchTitle}>
      <StateGlyph shape={status.shape} tone={status.tone} />
      <span className={`session-name${status.tone === "muted" ? " is-muted" : ""}`}>{name}</span>
      {status.chip ? <span className={`session-chip tone-${status.tone}`} role="status">{status.chip}</span> : null}
      <span className="session-caret" aria-hidden="true"><UiIcon name="down" /></span>
    </button>
    {narrow ? <button ref={closeRef} type="button" className="sidebar-close" onClick={onClose} aria-label="Close sidebar"><UiIcon name="close" /></button> : null}
  </header>;
}
