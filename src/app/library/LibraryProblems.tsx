import { useEffect, useId, useRef, useState } from "react";
import { UiIcon } from "../UiIcon";
import { dismissLibraryProblem, useLibraryProblems } from "./useLibraryOperation";
import "../errorSlot.css";

/** Failures that settled after their Library surface closed; no timers or focus theft. */
export function LibraryProblems() {
  const problems = useLibraryProblems();
  const [open, setOpen] = useState(false);
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    popoverRef.current?.focus({ preventScroll: true });
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !rootRef.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  useEffect(() => {
    if (problems.length === 0 && open) {
      setOpen(false);
    }
  }, [problems.length, open]);
  if (problems.length === 0) return null;
  return <div className="library-problems" ref={rootRef} onKeyDown={(event) => {
    if (event.key !== "Escape" || !open) return;
    event.preventDefault();
    event.stopPropagation();
    setOpen(false);
    buttonRef.current?.focus({ preventScroll: true });
  }}>
    <button ref={buttonRef} type="button" className="library-problems-pill" aria-expanded={open} aria-controls={open ? id : undefined} aria-haspopup="dialog" onClick={() => setOpen((value) => !value)}>
      <UiIcon name="info" /><span role="alert" aria-atomic="true">{problems.length} {problems.length === 1 ? "problem" : "problems"}</span>
    </button>
    {open ? <div id={id} ref={popoverRef} tabIndex={-1} role="dialog" aria-label="Background Library problems" className="library-problems-popover">
      {problems.map((problem) => <article key={problem.id}>
        <strong>{problem.label}</strong>
        <p>{problem.message}</p>
        <button type="button" onClick={() => {
          if (problems.length === 1) {
            const next = rootRef.current?.nextElementSibling;
            if (next instanceof HTMLElement) next.focus({ preventScroll: true });
          } else buttonRef.current?.focus({ preventScroll: true });
          dismissLibraryProblem(problem.id);
        }}>Dismiss</button>
      </article>)}
    </div> : null}
  </div>;
}
