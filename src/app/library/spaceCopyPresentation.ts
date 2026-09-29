import type { SpaceCopyRow, SpaceCopyState, SpaceFollowSummary } from "../../protocol/generated/v1";
import { pageCount, type StateChip, type StateShape, type StateTone } from "./libraryState";

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
  shape: StateShape;
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
      return { shape: "check", word: "Up to date", tone: "idle", notice: null, actions: [REMOVE] };
    case "library_newer":
      return { shape: "up", word: "Library newer", tone: "working", notice: "The Library has a newer version. This copy hasn't changed.", actions: [{ kind: "update", label: "Update" }] };
    case "edited_in_space":
      return { shape: "edit", word: row.library_newer ? "Edited in Space · Library newer" : "Edited in Space", tone: "working", notice: "You edited this copy. Updates skip it until you replace it.", actions: [REPLACE, VIEW_LIBRARY] };
    case "removed_at_source":
      return { shape: "slash-ring", word: "Removed at source", tone: "muted", notice: "Removed at source. This copy and the Library copy are kept.", actions: [REMOVE] };
    case "missing_in_space":
      return { shape: "ring", word: "Missing in Space", tone: "working", notice: "This copy's file is missing from the Space. The Library copy is kept.", actions: [{ kind: "restore", label: "Restore from Library" }] };
    case "not_in_library":
      return { shape: "ring", word: "Not in Library", tone: "muted", notice: "This copy's Library item was removed. It won't receive updates.", actions: [REMOVE, ADD_TO_LIBRARY_AGAIN] };
    case "not_linked":
      return { shape: "ring", word: "Not linked", tone: "muted", notice: "This existing copy has not been added to the Library.", actions: [{ kind: "readd_to_library", label: "Re-add to Library" }] };
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}

/** The Space status beside the header's actions: `In api-review` (when the copy is healthy) then a pill of `shape` and `word`. */
export type HeaderSpaceStatus = { context: string | null; shape: StateShape; word: string; tone: StateTone };

export type HeaderSpaceAction = {
  /** Status beside the actions; null when the Space holds no healthy copy. */
  status: HeaderSpaceStatus | null;
  actions: SpaceCopyAction[];
};

/**
 * The Library item header's single Space action (design §4.4) for the Space
 * row whose `item_id` is the item, or `undefined` when the Space has no copy.
 * `Missing in Space` and `Not linked` are never shown as an existing copy here:
 * adding again restores the file (OQ7).
 */
export function headerSpaceAction(row: StateInput | undefined, space: string): HeaderSpaceAction {
  const add: HeaderSpaceAction = { status: null, actions: [{ kind: "add", label: `Add to ${space}` }] };
  if (!row) return add;
  const state: SpaceCopyState = row.state;
  switch (state) {
    case "up_to_date":
      return { status: { context: `In ${space}`, shape: "check", word: "Up to date", tone: "idle" }, actions: [] };
    case "library_newer":
      return { status: { context: `In ${space}`, shape: "up", word: "Library newer", tone: "working" }, actions: [{ kind: "update", label: `Update in ${space}` }] };
    case "edited_in_space":
      return { status: { context: null, shape: "edit", word: row.library_newer ? "Edited in Space · Library newer" : "Edited in Space", tone: "working" }, actions: [REPLACE, VIEW_LIBRARY] };
    case "removed_at_source":
      return { status: { context: null, shape: "slash-ring", word: "Removed at source", tone: "muted" }, actions: [REMOVE] };
    case "not_in_library":
      return { status: { context: null, shape: "ring", word: "Not in Library", tone: "muted" }, actions: [REMOVE, ADD_TO_LIBRARY_AGAIN] };
    case "missing_in_space":
    case "not_linked":
      return add;
    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}

export type SpaceFollowPresentation = { chips: StateChip[]; notice: string | null; actions: SpaceCopyAction[] };

/**
 * A followed space's one aggregate row in a Space (design §4.6): `Library
 * newer: 1 new, 2 changed pages`, plus `1 page edited in Space` and
 * removed-at-source counts when they apply. `Update` is offered only while the
 * Library has new or changed pages; it writes this follow alone.
 */
export function spaceFollowPresentation(row: StateInput, follow: SpaceFollowSummary): SpaceFollowPresentation {
  if (row.state === "not_in_library") return { chips: [spaceCopyChip(row)], notice: "This followed space was removed from the Library. Its pages here are kept and won't receive updates.", actions: [] };
  const behind = follow.new_pages + follow.changed_pages;
  const chips: StateChip[] = [];
  if (behind > 0) {
    const counts = [follow.new_pages > 0 ? `${follow.new_pages} new` : null, follow.changed_pages > 0 ? `${follow.changed_pages} changed` : null].filter(Boolean).join(", ");
    chips.push({ shape: "up", word: `Library newer: ${counts} ${behind === 1 ? "page" : "pages"}`, tone: "working" });
  }
  if (follow.edited_pages > 0) chips.push({ shape: "edit", word: `${pageCount(follow.edited_pages)} edited in Space`, tone: "working" });
  if (follow.removed_at_source_pages > 0) chips.push({ shape: "slash-ring", word: `${pageCount(follow.removed_at_source_pages)} removed at source`, tone: "muted" });
  if (chips.length === 0) chips.push({ shape: "check", word: "Up to date", tone: "idle" });
  const notice = behind > 0 ? "The Library has new or changed pages. Update copies them here; pages you edited or removed in this Space are skipped."
    : follow.edited_pages > 0 ? "Updates skip pages you edited in this Space." : null;
  return { chips, notice, actions: behind > 0 ? [{ kind: "update", label: "Update" }] : [] };
}
