import { rowTags, tagHue } from "./spaceTagView";

/** A row's tags as coloured dots with names (one tag editor trigger), or the "+ tag" trigger for an untagged row. */
export function SpaceTagChip({ spaceLabel, tags, exclude, showAdd, expanded, disabled, busy, onToggle }: {
  spaceLabel: string; tags: readonly string[]; exclude?: string;
  showAdd: boolean; expanded: boolean; disabled: string | null; busy: boolean;
  onToggle(trigger: HTMLButtonElement): void;
}) {
  const shown = rowTags(tags, exclude);
  if (!shown.length && !showAdd) return null;
  const label = shown.length ? `Edit tags for ${spaceLabel}: ${tags.join(", ")}` : `Add tag to ${spaceLabel}`;
  return <button type="button" tabIndex={-1} className={shown.length ? "space-tag-dots" : "space-tag-add"}
    aria-label={label} aria-haspopup="dialog" aria-expanded={expanded}
    aria-disabled={Boolean(disabled) || undefined} aria-busy={busy || undefined}
    title={`${tags.length ? tags.join(", ") : `Add tag to ${spaceLabel}`}${disabled ? `\n${disabled}` : ""}`}
    onClick={event => { event.stopPropagation(); if (!disabled) onToggle(event.currentTarget); }}>
    {shown.length ? shown.map(tag => <span key={tag} className="space-tag" data-hue={tagHue(tag)}>{tag}</span>) : "+ tag"}
  </button>;
}
