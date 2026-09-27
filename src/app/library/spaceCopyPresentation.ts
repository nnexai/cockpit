import type { SpaceCopyRow, SpaceCopyState } from "../../protocol/generated/v1";
import type { StateTone } from "./libraryState";

/**
 * The only mapping from `SpaceCopyState` to words (D23, design §4.4, §4.6,
 * §4.8). No state other than `up_to_date` ever reads `Up to date`.
 *
 * Actions are named here so every surface offers the same verbs; a surface
 * renders only the kinds its slice implements (S2: `add`; S3: `update`,
 * `restore`, `replace`, `remove`, and `view_library` in the document notice).
 */
export type SpaceCopyActionKind = "add" | "update" | "restore" | "replace" | "view_library" | "remove" | "add_to_library_again" | "readd_to_library";
export type SpaceCopyAction = { kind: SpaceCopyActionKind; label: string };

export type SpaceCopyChip = {
  glyph: string;
  word: string;
  tone: StateTone;
  /** The Space-copy notice (design §4.6); null when the state needs none. */
  notice: string | null;
  actions: SpaceCopyAction[];
};

type StateInput = Pick<SpaceCopyRow, "state" | "library_newer">;

const REMOVE: SpaceCopyAction = { kind: "remove", label: "Remove from this Space…" };
const REPLACE: SpaceCopyAction = { kind: "replace", label: "Replace with Library version…" };
const VIEW_LIBRARY: SpaceCopyAction = { kind: "view_library", label: "View Library version" };
const ADD_TO_LIBRARY_AGAIN: SpaceCopyAction = { kind: "add_to_library_again", label: "Add to Library again" };

/** Resources rows, the companion tree meta and Space-copy notices. */
export function spaceCopyChip(row: StateInput): SpaceCopyChip {
  const state: SpaceCopyState = row.state;
  switch (state) {
    case "up_to_date":
      return { glyph: "✓", word: "Up to date", tone: "idle", notice: null, actions: [REMOVE] };
    case "library_newer":
      return { glyph: "↑", word: "Library newer", tone: "working", notice: "The Library has a newer version. This copy hasn't changed.", actions: [{ kind: "update", label: "Update" }] };
    case "edited_in_space":
      return { glyph: "✎", word: row.library_newer ? "Edited in Space · Library newer" : "Edited in Space", tone: "working", notice: "You edited this copy. Updates skip it until you replace it.", actions: [REPLACE, VIEW_LIBRARY] };
    case "removed_at_source":
      return { glyph: "⊘", word: "Removed at source", tone: "muted", notice: "Removed at source. This copy and the Library copy are kept.", actions: [REMOVE] };
    case "missing_in_space":
      return { glyph: "○", word: "Missing in Space", tone: "working", notice: "This copy's file is missing from the Space. The Library copy is kept.", actions: [{ kind: "restore", label: "Restore from Library" }] };
    case "not_in_library":
      return { glyph: "·", word: "Not in Library", tone: "muted", notice: "This copy's Library item was removed. It won't receive updates.", actions: [REMOVE, ADD_TO_LIBRARY_AGAIN] };
    case "not_linked":
      return { glyph: "·", word: "Not linked", tone: "muted", notice: "This existing copy has not been added to the Library.", actions: [{ kind: "readd_to_library", label: "Re-add to Library" }] };
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}

export type HeaderSpaceAction = {
  /** Status text beside the actions; null when the Space holds no healthy copy. */
  text: string | null;
  tone: StateTone;
  actions: SpaceCopyAction[];
};

/**
 * The Library item header's single Space action (design §4.4) for the Space
 * row whose `item_id` is the item, or `undefined` when the Space has no copy.
 * `Missing in Space` and `Not linked` are never shown as an existing copy here:
 * adding again restores the file (OQ7).
 */
export function headerSpaceAction(row: StateInput | undefined, space: string): HeaderSpaceAction {
  const add: HeaderSpaceAction = { text: null, tone: "muted", actions: [{ kind: "add", label: `Add to ${space}` }] };
  if (!row) return add;
  const state: SpaceCopyState = row.state;
  switch (state) {
    case "up_to_date":
      return { text: `In ${space} · ✓ Up to date`, tone: "idle", actions: [] };
    case "library_newer":
      return { text: `In ${space} · ↑ Library newer`, tone: "working", actions: [{ kind: "update", label: `Update in ${space}` }] };
    case "edited_in_space":
      return { text: row.library_newer ? "✎ Edited in Space · Library newer" : "✎ Edited in Space", tone: "working", actions: [REPLACE, VIEW_LIBRARY] };
    case "removed_at_source":
      return { text: "⊘ Removed at source", tone: "muted", actions: [REMOVE] };
    case "not_in_library":
      return { text: "· Not in Library", tone: "muted", actions: [REMOVE, ADD_TO_LIBRARY_AGAIN] };
    case "missing_in_space":
    case "not_linked":
      return add;
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}
