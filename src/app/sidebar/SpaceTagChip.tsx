import { chipFor } from "./spaceTagView";

export function SpaceTagChip({ spaceLabel, tags, exclude, showAdd, expanded, disabled, busy, onToggle }: {
  spaceLabel: string; tags: readonly string[]; exclude?: string;
  showAdd: boolean; expanded: boolean; disabled: string | null; busy: boolean;
  onToggle(trigger: HTMLButtonElement): void;
}) {
  const chip = chipFor(tags, exclude);
  if (!chip && !showAdd) return null;
  const label = chip ? `Edit tags for ${spaceLabel}: ${tags.join(", ")}` : `Add tag to ${spaceLabel}`;
  return <button type="button" tabIndex={-1} className={chip ? "space-tag-chip" : "space-tag-add"}
    aria-label={label} aria-haspopup="dialog" aria-expanded={expanded}
    aria-disabled={Boolean(disabled) || undefined} aria-busy={busy || undefined}
    title={`${tags.length ? tags.join(", ") : `Add tag to ${spaceLabel}`}${disabled ? `\n${disabled}` : ""}`}
    onClick={event => { event.stopPropagation(); if (!disabled) onToggle(event.currentTarget); }}>
    {chip ? <><span className="space-tag-name">{chip.primary}</span>{chip.more ? <span className="space-tag-more">+{chip.more}</span> : null}</> : "+ tag"}
  </button>;
}
