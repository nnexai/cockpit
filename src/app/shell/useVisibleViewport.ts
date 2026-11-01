import { useEffect } from "react";

export function useVisibleViewport(): void {
  useEffect(() => {
    const viewport = window.visualViewport;
    const style = document.documentElement.style;
    let frame: number | null = null;
    let height: number | undefined;
    let top: number | undefined;

    const update = () => {
      frame = null;
      if (viewport && viewport.scale !== 1) return;
      if (window.scrollX !== 0 || window.scrollY !== 0) window.scrollTo(0, 0);
      const nextHeight = viewport?.height ?? window.innerHeight;
      const nextTop = viewport?.offsetTop ?? 0;
      if (height === undefined || Math.abs(nextHeight - height) >= 1) {
        height = nextHeight;
        style.setProperty("--app-height", `${nextHeight}px`);
      }
      if (top === undefined || Math.abs(nextTop - top) >= 1) {
        top = nextTop;
        style.setProperty("--app-top", `${nextTop}px`);
      }
    };
    const schedule = () => {
      if (frame === null) frame = window.requestAnimationFrame(update);
    };

    viewport?.addEventListener("resize", schedule);
    viewport?.addEventListener("scroll", schedule);
    window.addEventListener("resize", schedule);
    document.addEventListener("focusin", schedule);
    schedule();
    return () => {
      viewport?.removeEventListener("resize", schedule);
      viewport?.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
      document.removeEventListener("focusin", schedule);
      if (frame !== null) window.cancelAnimationFrame(frame);
      style.removeProperty("--app-height");
      style.removeProperty("--app-top");
    };
  }, []);
}
