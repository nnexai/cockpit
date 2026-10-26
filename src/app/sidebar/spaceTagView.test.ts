// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Space } from "./spaceTree";
import { findTag, fitTagCount, normalizeTag, projectTagGroups, readSpacesView, rowTags, SPACES_VIEW_KEY, tagHue, tagKey, TAG_MAX_CHARS, UNTAGGED_KEY, writeSpacesView } from "./spaceTagView";

const space = (id: string): Space => ({ id, label: id, number: 1, tab_count: 0, pane_count: 0, focused: false, agent_status: "idle", git: null });
beforeEach(() => {
  const stored = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => { stored.set(key, value); },
  });
});
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe("tag normalization", () => {
  it("normalizes NFC, Rust whitespace and case-insensitive exact keys", () => {
    expect(normalizeTag(" \tCafe\u0301\u0085  review\n")).toBe("Café review");
    expect(tagKey("  NEEDS\u2003Testing ")).toBe("needs testing");
    expect(findTag(["Needs testing", "My check"], " needs  TESTING ")).toBe("Needs testing");
    expect(findTag(["Needs testing"], "need")).toBeUndefined();
    expect(findTag(["Needs testing"], "  ")).toBeUndefined();
    expect(normalizeTag("\ufefftag\ufeff")).toBe("\ufefftag\ufeff");
  });

  it("rejects empty, control and overlong names but counts Unicode scalar values", () => {
    expect(normalizeTag("\n \u0085")).toBe("");
    expect(normalizeTag("review\u0000later")).toBe("");
    expect(normalizeTag("review\u007flater")).toBe("");
    expect(normalizeTag("x".repeat(TAG_MAX_CHARS + 1))).toBe("");
    expect(normalizeTag("😀".repeat(TAG_MAX_CHARS))).toBe("😀".repeat(TAG_MAX_CHARS));
    expect(normalizeTag("😀".repeat(TAG_MAX_CHARS + 1))).toBe("");
  });
});

describe("Spaces view preference", () => {
  it("defaults to Repository and persists only the Tags choice", () => {
    expect(readSpacesView()).toBe("repository");
    writeSpacesView("tags");
    expect(localStorage.getItem(SPACES_VIEW_KEY)).toBe("tags");
    expect(readSpacesView()).toBe("tags");
    writeSpacesView("repository");
    expect(readSpacesView()).toBe("repository");
    localStorage.setItem(SPACES_VIEW_KEY, "unknown");
    expect(readSpacesView()).toBe("repository");
  });

  it("swallows unavailable storage for reads and writes", () => {
    vi.spyOn(localStorage, "getItem").mockImplementation(() => { throw new Error("denied"); });
    vi.spyOn(localStorage, "setItem").mockImplementation(() => { throw new Error("denied"); });
    expect(readSpacesView()).toBe("repository");
    expect(() => writeSpacesView("tags")).not.toThrow();
  });
});

describe("tag chips and groups", () => {
  const catalog = Object.freeze(["My check", "Needs testing", "Demo Friday", "Unused"]);
  const tags = Object.freeze(["My check", "Needs testing", "Demo Friday"]);


  it("fits whole leading tags and the exact overflow badge, retaining one overwide tag", () => {
    expect(fitTagCount([50, 50, 50], 4, [20, 20], 158)).toBe(3);
    expect(fitTagCount([50, 50, 50], 4, [20, 20], 128)).toBe(2);
    expect(fitTagCount([50, 50, 50], 4, [20, 20], 127)).toBe(1);
    expect(fitTagCount([200, 50], 4, [20], 100)).toBe(1);
    // All tags can fit even when an intermediate prefix plus its badge cannot.
    expect(fitTagCount([50, 10], 4, [20], 64)).toBe(2);
    expect(fitTagCount([20, 20, 20], 4, [10, 30], 58)).toBe(2);
    expect(fitTagCount([], 4, [], 100)).toBe(0);
    expect(fitTagCount([200], 4, [], 100)).toBe(1);
    expect(fitTagCount([50, 50], 4, [20], 0)).toBe(2);
  });
  it("shows every catalog-ordered tag except the group tag, without evicting or reordering tags", () => {
    expect(rowTags(tags)).toEqual(["My check", "Needs testing", "Demo Friday"]);
    expect(rowTags(tags, "Needs testing")).toEqual(["My check", "Demo Friday"]);
    expect(rowTags(["My check"], "My check")).toEqual([]);
    expect(tags).toEqual(["My check", "Needs testing", "Demo Friday"]);
  });

  it("keeps one colour per tag regardless of case or spacing", () => {
    expect(tagHue("Needs  Review")).toBe(tagHue("needs review"));
  });

  it("keeps catalog group order, Space order and Untagged last, omitting empty groups", () => {
    const spaces = Object.freeze([space("two"), space("one"), space("plain")]);
    const bySpace = new Map([["one", tags], ["two", ["Needs testing"]], ["removed", ["Unused"]]]);
    const rows = projectTagGroups(spaces, bySpace, catalog, new Set());
    expect(rows.filter(row => row.kind === "group").map(row => [row.label, row.count, row.expanded])).toEqual([
      ["My check", 1, true], ["Needs testing", 2, true], ["Demo Friday", 1, true], ["Untagged", 1, true],
    ]);
    expect(rows.filter(row => row.kind === "space").map(row => [row.groupKey, row.space.id, row.first, row.exclude])).toEqual([
      ["My check", "one", true, "My check"], ["Needs testing", "two", true, "Needs testing"],
      ["Needs testing", "one", false, "Needs testing"], ["Demo Friday", "one", false, "Demo Friday"],
      [UNTAGGED_KEY, "plain", true, undefined],
    ]);
    expect(new Set(rows.map(row => row.rowId)).size).toBe(rows.length);
    expect(bySpace.get("one")).toBe(tags);
  });

  it("marks the first visible occurrence after collapse and keeps full membership counts", () => {
    const spaces = [space("one"), space("plain")];
    const bySpace = new Map([["one", tags]]);
    const rows = projectTagGroups(spaces, bySpace, catalog, new Set(["My check", UNTAGGED_KEY]));
    expect(rows.filter(row => row.kind === "space").map(row => [row.groupKey, row.first])).toEqual([
      ["Needs testing", true], ["Demo Friday", false],
    ]);
    expect(rows[0]).toMatchObject({ kind: "group", count: 1, expanded: false });
    expect(rows.at(-1)).toMatchObject({ kind: "group", groupKey: UNTAGGED_KEY, count: 1, expanded: false });
    expect(projectTagGroups([], bySpace, catalog, new Set())).toEqual([]);
  });

  it("uses collision-free opaque row IDs, including a real tag named Untagged", () => {
    const spaces = [space('a:\"b'), space("plain")];
    const rows = projectTagGroups(spaces, new Map([[spaces[0].id, ["Untagged", 'a:\"b']]]), ["Untagged", 'a:\"b'], new Set());
    expect(rows[0].rowId).toBe(`group:${JSON.stringify("Untagged")}`);
    expect(rows[1].rowId).toBe(`tag:${JSON.stringify(["Untagged", spaces[0].id])}`);
    expect(new Set(rows.map(row => row.rowId)).size).toBe(rows.length);
    expect(rows.filter(row => row.kind === "group" && row.label === "Untagged")).toHaveLength(2);
  });
});
