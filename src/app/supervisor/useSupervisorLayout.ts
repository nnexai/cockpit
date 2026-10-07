import { useLayoutEffect, useState, type RefObject } from "react";

export type SupervisorLayout = {
  measured: boolean;
  width: number;
  height: number;
  narrow: boolean;
  short: boolean;
  compact: boolean;
  queueMode: "inline" | "overlay";
};
export type PanelKind = "details" | "activity" | "diagnostics" | "attention";
export type PanelPlacement = "side" | "sheet" | "overlay";
export type PanelBounds = { orientation: "vertical" | "horizontal"; min: number; max: number; value: number; defaultValue: number };

/** One measurement drives both responsive markup and its keyboard/focus behavior. */
export function useSupervisorLayout(ref: RefObject<HTMLElement | null>): SupervisorLayout {
  const [size, setSize] = useState(() => ({ width: 0, height: 0, windowHeight: window.innerHeight }));
  useLayoutEffect(() => {
    const element = ref.current;
    const measure = () => {
      const rect = ref.current?.getBoundingClientRect();
      const width = rect?.width ?? 0, height = rect?.height ?? 0, windowHeight = window.innerHeight;
      setSize(current => current.width === width && current.height === height && current.windowHeight === windowHeight
        ? current : { width, height, windowHeight });
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    if (element) observer?.observe(element);
    window.addEventListener("resize", measure);
    return () => { observer?.disconnect(); window.removeEventListener("resize", measure); };
  }, [ref]);
  const measured = size.width > 0;
  const narrow = measured && size.width < 720;
  const short = size.windowHeight <= 600;
  return { measured, width: size.width, height: size.height, narrow, short, compact: measured && size.width < 480, queueMode: narrow || short ? "overlay" : "inline" };
}

export function panelPlacement(layout: SupervisorLayout, view: "tasks" | "graph", panel: PanelKind): PanelPlacement {
  if (panel === "attention") return "overlay";
  if (!layout.narrow) return "side";
  return panel === "details" && view === "graph" && layout.height >= 560 ? "sheet" : "overlay";
}

export function panelBounds(layout: SupervisorLayout, placement: PanelPlacement, saved: { detailWidth: number | null; sheetHeight: number | null }, sheetMax?: number | null): PanelBounds | null {
  if (placement === "side") {
    // Before the first measurement, keep the wide default and valid separator bounds.
    const max = layout.measured ? Math.floor(layout.width * 0.5) : 340;
    return { orientation: "vertical", min: 280, max, defaultValue: 340, value: Math.round(Math.min(max, Math.max(280, saved.detailWidth ?? 340))) };
  }
  if (placement === "sheet") {
    // Keep enough measured graph space above the sheet to reveal a complete node.
    const max = Math.min(Math.floor(layout.height * 0.75), sheetMax == null ? Infinity : Math.floor(sheetMax));
    if (max < 160) return null;
    const defaultValue = Math.min(max, Math.round(layout.height * 0.5));
    return { orientation: "horizontal", min: 160, max, defaultValue, value: Math.round(Math.min(max, Math.max(160, saved.sheetHeight ?? defaultValue))) };
  }
  return null;
}

export const queueCap = (layout: SupervisorLayout, view: "tasks" | "graph") => Math.floor(layout.height * (view === "graph" ? 0.3 : 0.4));
