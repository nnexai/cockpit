import { useLayoutEffect, useState, type RefObject } from "react";

export function useFileOverview(ref: RefObject<HTMLElement | null>) {
  const [narrow, setNarrow] = useState(false);
  const [choice, setChoice] = useState<boolean | null>(null);
  useLayoutEffect(() => {
    const pane = ref.current;
    if (!pane) return;
    const resize = () => { const width = pane.getBoundingClientRect().width; if (width > 0) setNarrow(width <= 520); };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(pane);
    return () => observer.disconnect();
  }, [ref]);
  const open = choice ?? !narrow;
  return { open, narrow, reset: () => setChoice(null), toggle: () => setChoice(!open), show: () => setChoice(true), close: () => setChoice(false), select: () => { if (narrow) setChoice(false); } };
}
