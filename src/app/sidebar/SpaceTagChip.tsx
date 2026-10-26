import { useLayoutEffect, useRef, useState } from "react";
import { fitTagCount, rowTags, tagHue } from "./spaceTagView";

/** A fitted tag line (all tags wrap on selection), or the untagged row's add trigger. */
export function SpaceTagChip({ spaceLabel, tags, exclude, showAdd, showAll, expanded, disabled, busy, onToggle }: {
  spaceLabel: string; tags: readonly string[]; exclude?: string;
  showAdd: boolean; showAll: boolean; expanded: boolean; disabled: string | null; busy: boolean;
  onToggle(trigger: HTMLButtonElement): void;
}) {
  const shown = rowTags(tags, exclude);
  const tagKey = shown.join("\u0000");
  const buttonRef = useRef<HTMLButtonElement>(null);
  const measureRef = useRef<HTMLSpanElement>(null);
  const [fit, setFit] = useState(shown.length);
  useLayoutEffect(() => {
    const button = buttonRef.current;
    const measure = measureRef.current;
    const line = button?.parentElement;
    if (!line || !measure || showAll) {
      setFit(shown.length);
      return;
    }
    const chips = measure.firstElementChild!;
    const badges = measure.lastElementChild!;
    const update = () => {
      const widths = Array.from(chips.children, chip => chip.getBoundingClientRect().width);
      const moreWidths = Array.from(badges.children, badge => badge.getBoundingClientRect().width);
      const gap = parseFloat(getComputedStyle(chips).columnGap) || 0;
      setFit(fitTagCount(widths, gap, moreWidths, line.getBoundingClientRect().width));
    };
    update();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    observer?.observe(line);
    observer?.observe(chips);
    observer?.observe(badges);
    document.fonts?.addEventListener("loadingdone", update);
    return () => {
      observer?.disconnect();
      document.fonts?.removeEventListener("loadingdone", update);
    };
  }, [showAll, tagKey]);
  if (!shown.length && !showAdd) return null;
  const visible = showAll ? shown.length : Math.max(1, Math.min(fit, shown.length));
  const hidden = shown.length - visible;
  const chip = (tag: string) => <span key={tag} className="space-tag" data-hue={tagHue(tag)}><span className="space-tag-name">{tag}</span></span>;
  const label = shown.length ? `Edit tags for ${spaceLabel}: ${tags.join(", ")}` : `Add tag to ${spaceLabel}`;
  return <button ref={buttonRef} type="button" tabIndex={-1} className={shown.length ? `space-tag-dots${showAll ? " is-all" : ""}` : "space-tag-add"}
    aria-label={label} aria-haspopup="dialog" aria-expanded={expanded}
    aria-disabled={Boolean(disabled) || undefined} aria-busy={busy || undefined}
    title={`${tags.length ? tags.join(", ") : `Add tag to ${spaceLabel}`}${disabled ? `\n${disabled}` : ""}`}
    onClick={event => { event.stopPropagation(); if (!disabled) onToggle(event.currentTarget); }}>
    {shown.length ? <>{shown.slice(0, visible).map(chip)}{hidden ? <span className="space-tag-more">+{hidden}</span> : null}
      {!showAll && shown.length > 1 ? <span ref={measureRef} className="space-tag-measure" aria-hidden="true">
        <span className="space-tag-measure-chips">{shown.map(chip)}</span>
        <span className="space-tag-measure-badges">{shown.slice(1).map((_, index) => <span key={index} className="space-tag-more">+{index + 1}</span>)}</span>
      </span> : null}</> : "+ tag"}
  </button>;
}
