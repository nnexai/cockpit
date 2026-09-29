import { UiIcon } from "../UiIcon";
import type { StateShape, StateTone } from "./libraryState";

type PillSize = "tree" | "header";

/**
 * The one Library state mark (design §4.1a): a tone-tinted pill with a bare
 * SVG shape and the state word, so state is never colour or shape alone.
 * `tree` is the 18 px row pill, `header` the 22 px item-header pill.
 */
export function StatePill({ shape, word, tone, size = "tree", title, className }: { shape: StateShape; word: string; tone: StateTone; size?: PillSize; title?: string; className?: string }) {
  return <span className={`library-pill library-state is-${tone} is-${size}${className ? ` ${className}` : ""}`} title={title}><UiIcon name={shape} /><span>{word}</span></span>;
}

/** The pill of a running operation: a spinner in place of a state shape. */
export function PendingPill({ word, size = "tree", className }: { word: string; size?: PillSize; className?: string }) {
  return <span className={`library-pill library-state is-muted is-${size}${className ? ` ${className}` : ""}`}><span className="library-spinner" aria-hidden="true" /><span>{word}</span></span>;
}
