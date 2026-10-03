export type ArrivalInput = {
  change: "opened" | "reopened" | "replaced" | "updated";
  arrival: "own_tab" | "cross_source";
  tabDisplayed: boolean;
  blocker: "library" | "zoom" | "drag" | "too_narrow" | null;
  dockPresent: boolean;
  isCurrent: boolean;
  focusInsideFrame: boolean;
};

export type ArrivalDecision =
  | { kind: "dock_now"; makeCurrent: boolean; announce: boolean }
  | { kind: "defer_until_clear"; indicator: "none" | "widgets_button" }
  | { kind: "wait_tab"; announce: boolean }
  | { kind: "wait_click"; announce: boolean }
  | { kind: "replace_in_place"; markUnseen: boolean }
  | { kind: "none" };

export function decideArrival(input: ArrivalInput): ArrivalDecision {
  // Selection/provenance updates are not arrivals. Replacements never reopen a dock.
  if (input.change === "updated") return { kind: "none" };
  if (input.change === "replaced") return { kind: "replace_in_place", markUnseen: input.dockPresent && !input.isCurrent };
  if (input.arrival === "cross_source") return { kind: "wait_click", announce: true };
  if (!input.tabDisplayed) return { kind: "wait_tab", announce: true };
  if (input.blocker) return { kind: "defer_until_clear",
    indicator: input.blocker === "zoom" || input.blocker === "too_narrow" ? "widgets_button" : "none" };
  return { kind: "dock_now", makeCurrent: !input.dockPresent || !input.focusInsideFrame, announce: true };
}
