import { expect, it } from "vitest";
import { nextModalFocusIndex } from "./modal";

it("wraps modal tab focus in both directions", () => {
  expect(nextModalFocusIndex(2, 3, false)).toBe(0);
  expect(nextModalFocusIndex(0, 3, true)).toBe(2);
  expect(nextModalFocusIndex(-1, 3, false)).toBe(0);
});
