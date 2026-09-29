import { useLayoutEffect, useState, type RefObject } from "react";

export function useFileOverview(ref: RefObject<HTMLElement | null>, controlledChoice?: boolean | null, onChoiceChange?: (choice: boolean | null) => void) {
  const [narrow, setNarrow] = useState(false);
  const [localChoice, setLocalChoice] = useState<boolean | null>(null);
  const choice = onChoiceChange ? controlledChoice ?? null : localChoice;
  const setChoice = onChoiceChange ?? setLocalChoice;
  useLayoutEffect(() => {
    const surface = ref.current;
    if (!surface) return;
    const resize = () => { const width = surface.getBoundingClientRect().width; if (width > 0) setNarrow(width <= 520); };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(surface);
    return () => observer.disconnect();
  }, [ref]);
  const open = choice ?? !narrow;
  return { open, narrow, reset: () => setChoice(null), toggle: () => setChoice(!open), show: () => setChoice(true), close: () => setChoice(false), select: () => { if (narrow) setChoice(false); } };
}
