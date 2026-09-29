import { describe, expect, it } from "vitest";
import { CockpitClientError } from "./CockpitClient";
import { parseCommentBatch, parseCommentBatchList, parseCommentPreview, matchCommentBatch } from "./commentProtocol";

const anchor = {
  kind: "lines",
  start_line: 1,
  end_line: 20_000,
  selected_lines: Array.from({ length: 20_000 }, () => "line\n"),
};

const batch = {
  batch_id: "batch",
  generation: 1,
  owner: { kind: "viewer", session_id: "session", server_instance: "0123456789abcdef", tab_id: "tab", source_kind: "context", source_id: "source" },
  last_known_location: { workspace_id: "workspace", tab_id: "tab" },
  live_attachment: null,
  drafts: [{
    draft_id: "draft",
    file_ref: { root_id: "root", path: "file.md", absolute_path: "/companion/file.md", revision: "revision", content_hash: null },
    anchor,
    comment_text: "note",
    source_state: "current",
    updated_at: "now",
  }],
  updated_at: "now",
};

describe("comment protocol bounds", () => {
  it("accepts an anchor at the configured Context line maximum", () => {
    expect(parseCommentBatch(batch).drafts[0]?.anchor).toEqual(anchor);
  });

  it("requires the fixed framed preview ceiling even for a non-exportable preview", () => {
    const payload = "é";
    const preview = {
      batch_id: "batch",
      generation: 1,
      payload,
      payload_bytes: 2,
      framed_bytes: 14,
      limit_bytes: 64 * 1024,
      sanitized_controls: 0,
      stale_draft_ids: [],
      exportable: false,
      reason: "preview exceeds the framed limit",
    };
    expect(parseCommentPreview(preview)).toEqual(preview);
    expect(() => parseCommentPreview({ ...preview, limit_bytes: 4 * 1024 * 1024 })).toThrow(CockpitClientError);
  });
});

describe("viewer comment ownership boundaries", () => {
  const scope = { binding_id: "binding", client_id: "client" };
  const attachment = { owner: batch.owner, location: batch.last_known_location, ...scope };

  it("keeps legacy batches detached and rejects untagged or legacy live owners", () => {
    const legacyOwner = { kind: "legacy_pane", session_id: "session", pane_id: "pane", terminal_id: "terminal", source_kind: "context", source_id: "source" };
    expect(parseCommentBatch({ ...batch, owner: legacyOwner }).owner).toEqual(legacyOwner);
    expect(() => parseCommentBatch({ ...batch, owner: { ...legacyOwner, kind: undefined } })).toThrow(CockpitClientError);
    expect(() => parseCommentBatch({ ...batch, live_attachment: { ...attachment, owner: legacyOwner } })).toThrow(CockpitClientError);
    expect(() => parseCommentBatchList({ attachment: { ...attachment, location: { ...attachment.location, tab_id: "other" } }, batches: [], truncated: false })).toThrow(CockpitClientError);
  });

  it("rejects stale bindings and owner incarnation mismatches", () => {
    const live = parseCommentBatch({ ...batch, live_attachment: attachment });
    expect(matchCommentBatch(live, "session", scope, "batch")).toEqual(live);
    expect(() => matchCommentBatch(live, "session", { ...scope, binding_id: "other" }, "batch")).toThrow(CockpitClientError);
    const mismatched = parseCommentBatch({ ...batch, owner: { ...batch.owner, server_instance: "fedcba9876543210" }, live_attachment: attachment });
    expect(() => matchCommentBatch(mismatched, "session", scope, "batch")).toThrow(CockpitClientError);
  });
});
