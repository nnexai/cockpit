import { useEffect, useId, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import type { WidgetSummary } from "../../protocol/generated/v1";

export type WidgetTabsProps = {
  widgets: WidgetSummary[];
  currentId: string;
  unseen: ReadonlySet<string>;
  width: number;
  onCurrent(id: string): void;
};

function Updated({ widget, unseen }: { widget: WidgetSummary; unseen: ReadonlySet<string> }) {
  return unseen.has(JSON.stringify([widget.key.session_id, widget.key.tab_id, widget.key.id]))
    ? <><span className="widget-updated-dot" aria-hidden="true" /><span className="sr-only"> updated</span></> : null;
}

function title(widget: WidgetSummary) {
  return `${widget.title} — Space ${widget.space_id} · Tab ${widget.key.tab_id} · id ${widget.key.id}`;
}

function destination(key: string, index: number, count: number): number | null {
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  if (key === "ArrowRight" || key === "ArrowDown") return (index + 1) % count;
  if (key === "ArrowLeft" || key === "ArrowUp") return (index + count - 1) % count;
  return null;
}

export function WidgetTabs({ widgets, currentId, unseen, width, onCurrent }: WidgetTabsProps) {
  const [open, setOpen] = useState(false);
  const [menuIndex, setMenuIndex] = useState(0);
  const id = useId();
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const items = useRef<Array<HTMLButtonElement | null>>([]);
  const currentIndex = Math.max(0, widgets.findIndex((widget) => widget.key.id === currentId));
  const current = widgets[currentIndex];
  const dropdown = widgets.length > 4 || width <= 420;
  const initialMenuFocus = useRef(currentIndex);
  if (!open) initialMenuFocus.current = currentIndex;

  useEffect(() => {
    if (open && (!dropdown || widgets.length < 2)) setOpen(false);
  }, [open, dropdown, widgets.length]);
  useEffect(() => {
    if (!open) return;
    items.current[initialMenuFocus.current]?.focus();
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !wrap.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  if (!current) return null;
  if (widgets.length === 1) return <span className="widget-single-title" title={title(current)}>{current.title}</span>;

  const navigate = (event: KeyboardEvent<HTMLButtonElement>, index: number, select: boolean) => {
    const next = destination(event.key, index, widgets.length);
    if (next === null) return;
    event.preventDefault();
    event.stopPropagation();
    if (select) onCurrent(widgets[next].key.id);
    else setMenuIndex(next);
    items.current[next]?.focus();
  };

  if (!dropdown) return <div className="widget-tabs" role="tablist" aria-label="Widgets">
    {widgets.map((widget, index) => <button key={widget.key.id} type="button" role="tab" title={title(widget)}
      ref={(node) => { items.current[index] = node; }} aria-selected={index === currentIndex} tabIndex={index === currentIndex ? 0 : -1}
      onKeyDown={(event) => navigate(event, index, true)} onClick={() => onCurrent(widget.key.id)}>
      <span className="widget-tab-title">{widget.title.length > 20 ? `${widget.title.slice(0, 19)}…` : widget.title}</span>
      <Updated widget={widget} unseen={unseen} />
    </button>)}
  </div>;

  return <div className="widget-picker" ref={wrap} onBlur={(event) => {
    if (event.relatedTarget instanceof Node && !event.currentTarget.contains(event.relatedTarget)) setOpen(false);
  }} onKeyDown={(event) => {
    if (open && event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
      trigger.current?.focus();
    }
  }}>
    <button ref={trigger} type="button" className="widget-picker-trigger" title={title(current)}
      aria-haspopup="menu" aria-expanded={open} aria-controls={open ? id : undefined}
      onClick={() => { setMenuIndex(currentIndex); setOpen(!open); }} onKeyDown={(event) => {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); event.stopPropagation(); setMenuIndex(currentIndex); setOpen(true); }
      }}>
      <span className="widget-tab-title">{current.title}</span><Updated widget={current} unseen={unseen} /><span aria-hidden="true">▾</span>
    </button>
    {open ? <div id={id} className="widget-picker-menu" role="menu" aria-label="Widgets" onPointerDown={(event) => event.stopPropagation()}>
      {widgets.map((widget, index) => <button key={widget.key.id} ref={(node) => { items.current[index] = node; }}
        type="button" role="menuitemradio" aria-checked={index === currentIndex} tabIndex={index === menuIndex ? 0 : -1}
        title={title(widget)} onFocus={() => setMenuIndex(index)} onKeyDown={(event) => navigate(event, index, false)} onClick={() => {
          onCurrent(widget.key.id);
          setOpen(false);
          trigger.current?.focus();
        }}>
        <span className="widget-tab-title">{widget.title}</span><Updated widget={widget} unseen={unseen} />
      </button>)}
    </div> : null}
  </div>;
}
