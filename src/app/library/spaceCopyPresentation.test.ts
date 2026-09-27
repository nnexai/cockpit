import { expect, it } from "vitest";
import type { SpaceCopyState } from "../../protocol/generated/v1";
import { headerSpaceAction, spaceCopyChip } from "./spaceCopyPresentation";

// `satisfies` fails to compile when the generated union gains or loses a state.
const STATES = Object.keys({
  up_to_date: true, library_newer: true, edited_in_space: true, removed_at_source: true, missing_in_space: true, not_in_library: true, not_linked: true,
} satisfies Record<SpaceCopyState, true>) as SpaceCopyState[];

function chipText(state: SpaceCopyState, libraryNewer: boolean): string {
  const chip = spaceCopyChip({ state, library_newer: libraryNewer });
  return [chip.glyph, chip.word, chip.notice ?? "", ...chip.actions.map((action) => action.label)].join(" ");
}

function headerText(state: SpaceCopyState | undefined, libraryNewer: boolean): string {
  const action = headerSpaceAction(state ? { state, library_newer: libraryNewer } : undefined, "api-review");
  return [action.text ?? "", ...action.actions.map((entry) => entry.label)].join(" ");
}

it("says Up to date only for an up-to-date copy, in Resources and in the item header", () => {
  for (const state of STATES) {
    for (const libraryNewer of [false, true]) {
      expect(chipText(state, libraryNewer).includes("Up to date"), `${state} chip`).toBe(state === "up_to_date");
      expect(headerText(state, libraryNewer).includes("Up to date"), `${state} header`).toBe(state === "up_to_date");
    }
  }
  expect(headerText(undefined, false)).not.toContain("Up to date");
});

it("names an edited copy that is also behind and offers replacing it", () => {
  const chip = spaceCopyChip({ state: "edited_in_space", library_newer: true });
  expect(`${chip.glyph} ${chip.word}`).toBe("✎ Edited in Space · Library newer");
  expect(chip.actions.map((action) => action.label)).toContain("Replace with Library version…");
  const header = headerSpaceAction({ state: "edited_in_space", library_newer: true }, "api-review");
  expect(header.text).toBe("✎ Edited in Space · Library newer");
  expect(header.actions.map((action) => action.label)).toEqual(["Replace with Library version…", "View Library version"]);
  expect(spaceCopyChip({ state: "edited_in_space", library_newer: false }).word).toBe("Edited in Space");
});

it("keeps a copy whose Library item was removed or whose source is gone", () => {
  const notInLibrary = spaceCopyChip({ state: "not_in_library", library_newer: false });
  expect(`${notInLibrary.glyph} ${notInLibrary.word}`).toBe("· Not in Library");
  expect(notInLibrary.actions.map((action) => action.label)).toEqual(["Remove from this Space…", "Add to Library again"]);
  const header = headerSpaceAction({ state: "not_in_library", library_newer: false }, "api-review");
  expect(header.text).toBe("· Not in Library");
  expect(header.actions.map((action) => action.label)).toEqual(["Remove from this Space…", "Add to Library again"]);
  const removed = spaceCopyChip({ state: "removed_at_source", library_newer: false });
  expect(`${removed.glyph} ${removed.word}`).toBe("⊘ Removed at source");
  expect(headerSpaceAction({ state: "removed_at_source", library_newer: false }, "api-review").text).toBe("⊘ Removed at source");
});

it("offers Add to <Space> in the header when the Space has no copy or has lost its file", () => {
  for (const row of [undefined, { state: "missing_in_space" as const, library_newer: false }, { state: "not_linked" as const, library_newer: false }]) {
    const header = headerSpaceAction(row, "api-review");
    expect(header.text).toBeNull();
    expect(header.actions).toEqual([{ kind: "add", label: "Add to api-review" }]);
  }
  const missing = spaceCopyChip({ state: "missing_in_space", library_newer: false });
  expect(`${missing.glyph} ${missing.word}`).toBe("○ Missing in Space");
  expect(headerSpaceAction({ state: "up_to_date", library_newer: false }, "api-review")).toEqual({ text: "In api-review · ✓ Up to date", tone: "idle", actions: [] });
});
