import { useEffect, useRef, useState } from "react";
import { trapModalTab } from "../input/modal";
import { findTag, normalizeTag, TAG_MAX_CHARS, tagKey } from "./spaceTagView";

const CHANGE_FAILURE = "Could not confirm the tag change. Check the tags before trying again.";

export function SpaceTagPopover({ spaceLabel, catalog, tags, busy, blocked, onChange, onClose, triggerRef }: {
  spaceLabel: string; catalog: readonly string[]; tags: readonly string[];
  busy: boolean; blocked: string | null;
  onChange(tag: string, assigned: boolean): Promise<boolean>;
  onClose(reason: "escape" | "outside" | "toggle", outsideTarget?: Element): void;
  triggerRef: { current: HTMLElement | null };
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const createRef = useRef<HTMLButtonElement>(null);
  const submitting = useRef(false);
  const composing = useRef(false);
  const [query, setQuery] = useState("");
  const [localBusy, setLocalBusy] = useState(false);
  const [failure, setFailure] = useState(false);
  const [announcement, setAnnouncement] = useState<string | null>(null);
  const [scrollTag, setScrollTag] = useState<string | null>(null);
  const normalized = normalizeTag(query);
  const exact = findTag(catalog, normalized);
  const filter = query.normalize("NFC").replace(/\p{White_Space}+/gu, " ").replace(/^ | $/g, "").toLowerCase();
  const filtered = catalog.filter(tag => tag.toLowerCase().includes(filter));
  const pending = busy || localBusy;
  const unavailable = pending || Boolean(blocked);
  const assigned = (tag: string) => tags.some(value => tagKey(value) === tagKey(tag));

  useEffect(() => { inputRef.current?.focus({ preventScroll: true }); }, []);
  useEffect(() => {
    const outside = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node) || rootRef.current?.contains(target) || triggerRef.current?.contains(target)) return;
      onClose("outside", target instanceof Element ? target : target.parentElement ?? undefined);
    };
    window.addEventListener("pointerdown", outside);
    return () => window.removeEventListener("pointerdown", outside);
  }, [onClose, triggerRef]);
  useEffect(() => {
    if (!scrollTag || !tags.includes(scrollTag)) return;
    const option = [...(rootRef.current?.querySelectorAll<HTMLElement>("[data-tag]") ?? [])].find(element => element.dataset.tag === scrollTag);
    if (option) { option.scrollIntoView?.({ block: "nearest" }); setScrollTag(null); }
  }, [scrollTag, catalog, tags]);

  const apply = async (tag: string, checked: boolean, fromInput = false) => {
    if (busy || blocked || submitting.current) return;
    const submittedQuery = query;
    const exists = findTag(catalog, tag);
    setFailure(false);
    if (fromInput && exists && assigned(exists)) {
      setQuery("");
      setAnnouncement(`Added ${exists}`);
      return;
    }
    submitting.current = true;
    setLocalBusy(true);
    let acknowledged = false;
    try { acknowledged = await onChange(tag, checked); } catch { /* A rejected write is not acknowledged. */ }
    finally { submitting.current = false; setLocalBusy(false); }
    if (!acknowledged) { setFailure(true); return; }
    setAnnouncement(checked ? `${exists ? "Added" : "Created"} ${exists ?? tag}` : `Removed ${tag}`);
    if (fromInput) {
      setQuery(current => current === submittedQuery ? "" : current);
      if (!exists) setScrollTag(tag);
      if (document.activeElement === createRef.current) inputRef.current?.focus({ preventScroll: true });
    }
  };
  const submit = () => { if (normalized) void apply(exact ?? normalized, true, true); };

  return <div ref={rootRef} className="space-tag-popover" role="dialog" aria-label={`Tags for ${spaceLabel}`}
    aria-busy={pending || undefined} onKeyDown={event => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose("escape"); return; }
      trapModalTab(event, rootRef.current);
    }}>
    <input ref={inputRef} className="space-tag-input" aria-label="Find or create tag" placeholder="Find or create tag…"
      maxLength={TAG_MAX_CHARS * 2} autoComplete="off" value={query}
      onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; }}
      onChange={event => { setQuery(event.target.value); setFailure(false); setAnnouncement(null); }}
      onKeyDown={event => {
        if (event.key === "Enter") {
          if (event.nativeEvent.isComposing || composing.current || event.keyCode === 229) return;
          event.preventDefault(); submit();
        } else if (event.key === "ArrowDown") {
          const first = rootRef.current?.querySelector<HTMLInputElement>('input[type="checkbox"]');
          if (first) { event.preventDefault(); first.focus(); }
        }
      }} />
    <div className="space-tag-list" role="group" aria-label="Tags">
      {filtered.map((tag, index) => {
        const checked = assigned(tag);
        return <label key={tag} className="space-tag-option" data-tag={tag}>
          <input type="checkbox" checked={checked} aria-disabled={unavailable || undefined} title={blocked ?? undefined}
            onChange={event => {
              // Native activation flips checked before React's handler. Restore the acknowledged value immediately.
              event.currentTarget.checked = checked;
              void apply(tag, !checked);
            }} onKeyDown={event => {
              if (event.key === "Enter") event.preventDefault();
              else if (event.key === "ArrowUp" && index === 0) { event.preventDefault(); inputRef.current?.focus(); }
            }} />
          <span className="space-tag-option-name">{tag}</span>
        </label>;
      })}
      {normalized && !exact ? <button ref={createRef} type="button" className="space-tag-create"
        aria-disabled={unavailable || undefined} title={blocked ?? undefined} onClick={submit}>Create "{normalized}"</button> : null}
    </div>
    <div className="space-tag-announcement sr-only" role="status" aria-live="polite">{announcement ?? `${filtered.length} tag${filtered.length === 1 ? "" : "s"}`}</div>
    {failure ? <div className="space-tag-status" role="status" aria-live="polite">{CHANGE_FAILURE}</div> : null}
  </div>;
}
