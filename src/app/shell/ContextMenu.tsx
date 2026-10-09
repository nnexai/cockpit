import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

export type ContextTarget =
  | { kind: "space"; id: string }
  | { kind: "tab"; id: string }
  | { kind: "pane"; id: string };
export type ContextMenuState = { target: ContextTarget; x: number; y: number };

export function contextMenuPosition(
  x: number,
  y: number,
  viewportWidth: number,
  viewportHeight: number,
  menuWidth = 286,
  menuHeight = 320,
  gutter = 8,
): { x: number; y: number } {
  return {
    x: Math.max(gutter, Math.min(x, viewportWidth - menuWidth - gutter)),
    y: Math.max(gutter, Math.min(y, viewportHeight - menuHeight - gutter)),
  };
}

export function ContextMenu({ menu, children, onDismiss }: { menu: ContextMenuState; children: ReactNode; onDismiss: () => void }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const dismissAndRestore = useCallback(() => {
    onDismiss();
    window.setTimeout(() => opener.current?.focus({ preventScroll: true }), 0);
  }, [onDismiss]);
  useEffect(() => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dismiss = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) dismissAndRestore(); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") dismissAndRestore(); };
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("keydown", escape);
    ref.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => { window.removeEventListener("pointerdown", dismiss); window.removeEventListener("keydown", escape); };
  }, [dismissAndRestore, onDismiss, menu]);
  const [menuSize, setMenuSize] = useState({ width: 286, height: 320 });
  useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;
    const measure = () => {
      const { width, height } = root.getBoundingClientRect();
      setMenuSize((current) => current.width === width && current.height === height ? current : { width, height });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(root);
    return () => observer?.disconnect();
  }, [menu]);
  const position = contextMenuPosition(menu.x, menu.y, window.innerWidth, window.innerHeight, menuSize.width, menuSize.height);
  return <div ref={ref} className="context-menu" role="menu" aria-label={`${menu.target.kind} actions`} style={{ left: position.x, top: position.y }} onContextMenu={(event) => event.preventDefault()} onKeyDown={(event) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const items = [...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    if (items.length === 0) return;
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : event.key === "ArrowDown" ? (current + 1) % items.length : (current - 1 + items.length) % items.length;
    items[next].focus();
  }}>{children}</div>;
}
