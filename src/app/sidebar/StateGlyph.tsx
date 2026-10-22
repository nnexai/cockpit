import type { SpaceStatus, StatusClass } from "./spaceTree";

/** `none` is a Space without an agent; unknown agents retain their ring. */
export type GlyphShape = StatusClass | "live" | "none";

export function spaceGlyphShape(status: SpaceStatus): GlyphShape {
  return status.className === "unknown" ? "none" : status.shape;
}

/**
 * A hollow unknown ring or state-tinted disc with one distinct shape, drawn in `currentColor` (the state token
 * comes from the `.sb-glyph.is-<shape>` rule or a tone class). 18x18 by default; the
 * child rows scale it to `--icon-size`. Decorative: the row's name carries the state word.
 */
export function StateGlyph({ shape, tone }: { shape: GlyphShape; tone?: "warning" | "blocked" | "muted" }) {
  return <svg className={`sb-glyph is-${shape}${tone ? ` tone-${tone}` : ""}`} viewBox="0 0 18 18" aria-hidden="true" focusable="false">
    {shape === "none"
      ? <circle cx="9" cy="9" r="2.75" fill="currentColor" />
      : shape === "unknown"
        ? <circle cx="9" cy="9" r="8.25" fill="none" stroke="currentColor" strokeWidth="1.5" />
        : <circle cx="9" cy="9" r="9" fill="currentColor" fillOpacity={shape === "idle" ? ".10" : ".16"} />}
    {shape === "blocked" ? <><path d="M9 5v4.75" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" /><circle cx="9" cy="12.6" r="1.05" fill="currentColor" /></> : null}
    {shape === "done" ? <path d="M5.5 9.5l2.6 2.6 4.6-5.7" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /> : null}
    {shape === "working" ? <><circle cx="9" cy="9" r="3.75" fill="none" stroke="currentColor" strokeWidth="1.5" /><path d="M9 5.25a3.75 3.75 0 0 0 0 7.5z" fill="currentColor" /></> : null}
    {shape === "idle" ? <circle cx="9" cy="9" r="3.25" fill="none" stroke="currentColor" strokeWidth="1.5" /> : null}
    {shape === "live" ? <circle cx="9" cy="9" r="3.5" fill="currentColor" /> : null}
  </svg>;
}
