import { describe, expect, it, vi } from "vitest";
import { createBrowserClient, type BrowserWebSocket } from "./browser";
import { createNativeClient, type NativeChannel } from "./native";
import {
  CockpitClientError,
  parseSessionListResponse,
  parseResourceMutationRequest,
  parseResourceMutationResponse,
  parseSessionSnapshotResponse,
  parseSpaceGitStatusResponse,
  parseStatusResponse,
  parseTerminalCommand,
  parseTerminalOpenRequest,
  parseTerminalStreamMessage,
  parseSessionStreamMessage,
  type CockpitClient,
  type CockpitSessionSnapshot,
  type ResourceMutationRequest,
  type TerminalOpenRequest,
} from "./CockpitClient";
import {
  STREAM_SEQUENCE_MAX,
  transitionSessionStream,
  type StreamOrderCursor,
} from "./streamOrder";

const snapshot: CockpitSessionSnapshot = {
  session_id: "session-1",
  version: "0.8.2",
  protocol: 20,
  focused_space_id: "space-1",
  focused_tab_id: "tab-1",
  focused_pane_id: "pane-1",
  spaces: [{ id: "space-1", label: "Main", number: 1, tab_count: 1, pane_count: 1, focused: true, agent_status: "working", git: { repository_key: "repo-opaque", repository: "cockpit", branch: "main", checkout_path: "/work/cockpit", is_linked_worktree: false } }],
  tabs: [{ id: "tab-1", space_id: "space-1", label: "Shell", number: 1, pane_count: 1, focused: true }],
  panes: [{ id: "pane-1", terminal_id: "terminal-1", space_id: "space-1", tab_id: "tab-1", title: "Terminal", focused: true, agent: null, agent_status: "idle", revision: 1 }],
  layouts: [{ space_id: "space-1", tab_id: "tab-1", area: { x: 0, y: 0, width: 80, height: 24 }, focused_pane_id: "pane-1", panes: [{ pane_id: "pane-1", focused: true, rect: { x: 0, y: 0, width: 80, height: 24 } }], zoomed: false }],
  agents: [],
};
const status = { protocol_version: "v1", cockpit_version: "0.1.0", mode: "normal" as const, capabilities: { terminal_mouse_input: true }, herdr: { status: "unavailable" as const, code: "test", message: "test" } };
const sessions = { sessions: [{ id: "session-1", label: "Main", is_default: true, running: true }] };
const terminalRequest: TerminalOpenRequest = {
  session_id: "session-1",
  pane_id: "pane-1",
  mode: "observe" as const,
  takeover: false,
  cols: 80,
  rows: 24,
  cell_width_px: 8,
  cell_height_px: 16,
};
function terminalOpen(overrides: Partial<typeof terminalRequest> = {}) {
  return { ...terminalRequest, ...overrides };
}

class FakeSocket implements BrowserWebSocket {
  readyState = 0;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  readonly sent: string[] = [];
  send(data: string): void { this.sent.push(data); }
  open(): void { this.readyState = 1; this.onopen?.(new Event("open")); }
  message(data: unknown): void { this.onmessage?.({ data } as MessageEvent); }
  close(): void { this.readyState = 3; this.onclose?.({ code: 1000, reason: "closed" } as CloseEvent); }
}

function jsonResponse(value: unknown, statusCode = 200): Response {
  return new Response(JSON.stringify(value), { status: statusCode });
}
function streamSnapshot(sequence: number, generation = 1) {
  return { type: "snapshot", session_id: snapshot.session_id, generation, sequence, snapshot } as const;
}
function streamStale(sequence: number, generation = 1) {
  return { type: "stale", session_id: snapshot.session_id, generation, sequence, code: "stale", message: "resync" } as const;
}

function completeClient(overrides: Partial<CockpitClient> = {}): CockpitClient {
  const base: CockpitClient = {
    status: vi.fn(async () => status),
    browserAction: vi.fn(async () => ({ association: null, connection: "absent" as const, message: "No browser is associated with this Space" })),
    browserFeedback: vi.fn(async () => { throw new Error("Unexpected browser feedback in terminal fixture"); }),
    browserDraftRecovery: vi.fn(async () => ({ type: "none" as const })),
    acknowledgeBrowserFeedback: vi.fn(async () => { throw new Error("Unexpected browser feedback acknowledgement in terminal fixture"); }),
    browserFeedbackImage: vi.fn(async () => { throw new Error("Unexpected browser feedback image in terminal fixture"); }),
    sendBrowserFeedback: vi.fn(async () => { throw new Error("Unexpected browser feedback send in terminal fixture"); }),
    resolveWorkspaceDefaults: vi.fn(async () => { throw new Error("Unexpected workspace defaults in terminal fixture"); }),
    projectConfiguration: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    providerCredentials: vi.fn(async () => { throw new Error("Unexpected provider credentials in terminal fixture"); }),
    setProviderCredential: vi.fn(async () => { throw new Error("Unexpected provider credentials in terminal fixture"); }),
    clearProviderCredential: vi.fn(async () => { throw new Error("Unexpected provider credentials in terminal fixture"); }),
    repositories: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    planWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    startWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    workspaceOperation: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    resumeWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    cancelWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    reconcileWorkspace: vi.fn(async () => { throw new Error("Unexpected project operation in terminal fixture"); }),
    workspaceTeardownPreview: vi.fn(async () => { throw new Error("Unexpected workspace teardown in terminal fixture"); }),
    workspaceTeardownExecute: vi.fn(async () => { throw new Error("Unexpected workspace teardown in terminal fixture"); }),
    workspaceTeardownRecoveries: vi.fn(async () => { throw new Error("Unexpected workspace teardown in terminal fixture"); }),
    inspectPane: vi.fn(async () => { throw new Error("Context inspection is unavailable in this terminal fixture"); }),
    contextDirectory: vi.fn(async () => { throw new Error("Unexpected Context read in terminal fixture"); }),
    contextFileIndex: vi.fn(async () => { throw new Error("Unexpected Context file index in terminal fixture"); }),
    contextDocument: vi.fn(async () => { throw new Error("Unexpected Context read in terminal fixture"); }),
    contextSearch: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    reviewSnapshot: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    reviewFile: vi.fn(async () => { throw new Error("Unexpected Context search in terminal fixture"); }),
    contextInvalidate: vi.fn(async () => { throw new Error("Unexpected Context invalidation in terminal fixture"); }),
    contextMedia: vi.fn(), librarySpaceList: vi.fn(), librarySpaceAdd: vi.fn(), librarySpaceAttemptsDismiss: vi.fn(), librarySpaceUpdate: vi.fn(), librarySpaceRemove: vi.fn(), openReview: vi.fn(), openContext: vi.fn(async () => { throw new Error("Unexpected Context launch in terminal fixture"); }),
    commentBatches: vi.fn(async () => { throw new Error("Unexpected comments list in terminal fixture"); }),
    commentBatch: vi.fn(async () => { throw new Error("Unexpected comment batch in terminal fixture"); }),
    commentUpsert: vi.fn(async () => { throw new Error("Unexpected comment upsert in terminal fixture"); }),
    commentDiscard: vi.fn(async () => { throw new Error("Unexpected comment discard in terminal fixture"); }),
    commentRemove: vi.fn(async () => { throw new Error("Unexpected comment remove in terminal fixture"); }),
    commentAttach: vi.fn(async () => { throw new Error("Unexpected comment attach in terminal fixture"); }),
    commentPastePrepare: async () => { throw new Error("unused"); },
    commentPasteSend: async () => { throw new Error("unused"); },
    commentPasteMarkPasted: async () => { throw new Error("unused"); },
    commentPreview: vi.fn(async () => { throw new Error("Unexpected comment preview in terminal fixture"); }),
    libraryListing: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryResolve: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryConfluenceSpaces: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryAdd: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryAttachments: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryRefresh: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryOperation: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryOperationCancel: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryReplace: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryRemove: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryDirectory: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryFileIndex: vi.fn(async () => { throw new Error("Unexpected Library file index in terminal fixture"); }),
    libraryDocument: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    libraryMedia: vi.fn(async () => { throw new Error("Unexpected Library operation in terminal fixture"); }),
    sessions: vi.fn(async () => sessions),
    sessionSnapshot: vi.fn(async () => snapshot),
    spaceGitStatus: vi.fn(async (sessionId: string) => ({ session_id: sessionId, spaces: [] })),
    focus: vi.fn(async () => ({ session_id: snapshot.session_id, kind: "pane" as const, target_id: "pane-1", accepted: true })),
    mutate: vi.fn(async () => ({ session_id: snapshot.session_id, snapshot })),
    subscribeSession: vi.fn(async () => ({ close: vi.fn() })),
    openTerminal: vi.fn(async () => ({ send: vi.fn(), close: vi.fn() })),
    openBrowserView: vi.fn(async () => ({ command: vi.fn(), close: vi.fn() })),
  };
  return Object.assign(base, overrides);
}

describe("client DTO parsers", () => {
  it("normalizes legacy and validates status capabilities", () => {
    const legacy = { protocol_version: "v1", cockpit_version: "0.1.0", mode: "normal", herdr: status.herdr };
    expect(parseStatusResponse(legacy).capabilities).toEqual({ terminal_mouse_input: false });
    expect(parseStatusResponse(status).capabilities).toEqual({ terminal_mouse_input: true });
    expect(() => parseStatusResponse({ ...legacy, capabilities: { terminal_mouse_input: "yes" } })).toThrow(CockpitClientError);
    expect(() => parseStatusResponse({ ...legacy, capabilities: undefined })).toThrow(CockpitClientError);
  });
  it("strictly validates session and terminal contracts", () => {
    expect(parseSessionListResponse(sessions)).toEqual(sessions);
    const legacyAgent = { pane_id: "pane-1", space_id: "space-1", tab_id: "tab-1", name: "omp", status: "working", title: null, focused: true };
    expect(parseSessionSnapshotResponse({ ...snapshot, agents: [legacyAgent] }).agents[0]?.state_change_seq).toBe(0);
    expect(() => parseSessionSnapshotResponse({ ...snapshot, session_id: undefined })).toThrow(CockpitClientError);
    expect(parseSpaceGitStatusResponse({ session_id: "s", spaces: [{ space_id: "w1", branch: "main", upstream: null, ahead: null, behind: null, extra: 1 }] }))
      .toEqual({ session_id: "s", spaces: [{ space_id: "w1", branch: "main", upstream: null, ahead: null, behind: null }] });
    expect(() => parseSpaceGitStatusResponse({ session_id: "s", spaces: [{ space_id: "w1", branch: null, upstream: null, ahead: -1, behind: null }] })).toThrow(CockpitClientError);
    expect(() => parseSessionSnapshotResponse({
      ...snapshot,
      spaces: [{ ...snapshot.spaces[0], git: { repository: "cockpit", branch: "main", checkout_path: "/work/cockpit" } }],
    })).toThrow(CockpitClientError);
    expect(() => parseSessionSnapshotResponse({
      ...snapshot,
      spaces: [{ ...snapshot.spaces[0], agent_status: undefined }],
    })).toThrow(CockpitClientError);
    expect(() => parseSessionStreamMessage({ type: "snapshot", session_id: "session-1", generation: 1, sequence: 1, snapshot: { ...snapshot, session_id: "other" } })).toThrow(CockpitClientError);
    expect(() => parseTerminalCommand({ type: "terminal.input", text: null, bytes: null })).toThrow(/exactly one/);
    expect(() => parseTerminalOpenRequest({ ...terminalRequest, session_id: "../session" })).toThrow(/invalid target/);
    expect(() => parseTerminalOpenRequest({ ...terminalRequest, pane_id: "pane/child" })).toThrow(/invalid target/);
    expect(parseTerminalOpenRequest(terminalRequest)).toEqual(terminalRequest);
    expect(() => parseTerminalStreamMessage({ type: "graphics", session_id: "session-1", pane_id: "pane-1", stream_id: "stream-1", revision: "7", bytes: "S0lUVA==" })).toThrow(/unknown type/);
    expect(() => parseTerminalStreamMessage({ type: "graphics", session_id: "session-1", pane_id: "pane-1", stream_id: "stream-1", revision: "7", bytes: "bad" })).toThrow(/unknown type/);
    expect(() => parseTerminalCommand({ type: "terminal.mouse", kind: "moved", button: "left", column: 12, row: 7, modifiers: 0 })).toThrow(CockpitClientError);
  });
  it("validates every resource mutation discriminant and required nullable field", () => {
    const mutations: ResourceMutationRequest[] = [
      { type: "space_create", cwd: null, label: null },
      { type: "space_rename", space_id: "space-1", label: "Work" },
      { type: "space_move_block", space_ids: ["space-1"], before_space_id: null },
      { type: "space_close", space_id: "space-1" },
      { type: "tab_create", space_id: "space-1", label: null },
      { type: "tab_rename", tab_id: "tab-1", label: "Shell" },
      { type: "tab_move", tab_id: "tab-1", insert_index: 2 },
      { type: "tab_close", tab_id: "tab-1" },
      { type: "pane_split", pane_id: "pane-1", direction: "right", ratio: null },
      { type: "pane_resize", pane_id: "pane-1", direction: "left", amount: 0.1 },
      { type: "pane_rename", pane_id: "pane-1", label: null },
      { type: "pane_swap", source_pane_id: "pane-1", target_pane_id: "pane-2" },
      {
        type: "pane_move",
        pane_id: "pane-1",
        destination: { type: "existing_tab", tab_id: "tab-2", direction: "down", target_pane_id: null, ratio: null },
      },
      { type: "pane_zoom", pane_id: "pane-1", mode: "toggle" },
      { type: "pane_close", pane_id: "pane-1" },
    ];
    for (const mutation of mutations) expect(parseResourceMutationRequest(mutation)).toEqual(mutation);
    expect(() => parseResourceMutationRequest({ type: "space_create", label: null })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationRequest({ type: "pane_resize", pane_id: "pane-1", direction: "left", amount: Number.NaN })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationRequest({ type: "pane_resize", pane_id: "pane-1", direction: "left", amount: 0 })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationRequest({ type: "pane_split", pane_id: "pane-1", direction: "right", ratio: 1 })).toThrow(CockpitClientError);

    expect(() => parseResourceMutationRequest({ type: "pane_move", pane_id: "pane-1", destination: { type: "raw", method: "pane.move" } })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationRequest({ type: "raw", method: "layout.apply", params: {} })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationResponse({ session_id: "session-1", snapshot: { ...snapshot, session_id: "other" } })).toThrow(/another session/);
  });
});
describe("session stream transition policy", () => {
  it("classifies the complete ordering corpus", () => {
    const initial = transitionSessionStream("session-1", null, streamSnapshot(1));
    expect(initial).toMatchObject({ kind: "accept", classification: "initial", cursor: { generation: 1, sequence: 1 } });
    const cursor = (initial.kind === "accept" ? initial.cursor : null) as StreamOrderCursor;
    expect(transitionSessionStream("session-1", null, streamSnapshot(2))).toMatchObject({ kind: "error", classification: "missing_first", code: "stream_sequence" });
    expect(transitionSessionStream("session-1", cursor, streamSnapshot(2))).toMatchObject({ kind: "accept", classification: "same_generation" });
    expect(transitionSessionStream("session-1", cursor, streamSnapshot(1))).toMatchObject({ kind: "ignore", classification: "duplicate" });
    expect(transitionSessionStream("session-1", cursor, streamSnapshot(4))).toMatchObject({ kind: "error", classification: "sequence_gap", code: "stream_sequence" });
    expect(transitionSessionStream("session-1", { ...cursor, generation: 2 }, streamSnapshot(1, 1))).toMatchObject({ kind: "ignore", classification: "stale", code: "stream_generation" });
    expect(transitionSessionStream("session-1", cursor, streamSnapshot(1, 2))).toMatchObject({ kind: "accept", classification: "generation_transition" });
    expect(transitionSessionStream("session-1", cursor, streamSnapshot(5, 2))).toMatchObject({ kind: "error", classification: "missing_first", code: "stream_sequence" });
    expect(transitionSessionStream("session-1", cursor, streamSnapshot(1, 3))).toMatchObject({ kind: "error", classification: "generation_gap", code: "stream_generation" });
    expect(transitionSessionStream("session-1", { ...cursor, sequence: STREAM_SEQUENCE_MAX }, streamSnapshot(STREAM_SEQUENCE_MAX + 1))).toMatchObject({ kind: "error", classification: "overflow", code: "stream_overflow" });
    expect(transitionSessionStream("other", cursor, streamSnapshot(2))).toMatchObject({ kind: "error", classification: "identity", code: "stream_identity" });
    expect(transitionSessionStream("session-1", cursor, streamStale(2))).toMatchObject({ kind: "accept", classification: "same_generation" });
  });
});

const libraryListing = {
  root: { root_id: "library:test", kind: "library", label: "Library", path: "/library", repository_id: "library", checkout_path: "/library", companion_id: null },
  generation: "1", items: [], follows: [], next_offset: null, diagnostics: [],
};
const libraryOperation = {
  operation_id: "op/1", kind: "refresh", phases: [], item_ids: [], report: null, space: null,
  target: null, cancel_requested: false, finished: true, created_at: "now", updated_at: "now",
};
const libraryRequests = {
  resolve: { input: "https://example.test/item", provider_id: null },
  add: { input: "https://example.test/item", provider_id: null, reference_depth: 0, follow: false, follow_mode: null, download_attachments: false, refresh_existing: false, label: null, target: null },
  refresh: { scope: "all" as const },
  replace: { item_id: "item-1", confirmed: [] },
  remove: { mode: "follow" as const, follow_id: "follow-1" },
  directory: { path: "", offset: null, revision: null },
  document: { path: "item/document.md", expected_revision: "rev-1", offset: 0 },
  media: { path: "item/image.png", expected_revision: "rev-1" },
  attachments: { item_id: "item-1", attachment_ids: ["att-1"], action: "download" as const },
};
const libraryDirectory = { binding_id: "library", root_id: "library:test", path: "", entries: [], truncated: false, diagnostics: [] };
const libraryDocument = { binding_id: "library", root_id: "library:test", path: "item/document.md", revision: "rev-1", content_hash: "hash", bytes: 1, media_type: "text/markdown", text: "x", truncated: false, offset: 0, diagnostics: [] };
const libraryMedia = { binding_id: "library", root_id: "library:test", path: "item/image.png", revision: "rev-1", content_hash: "hash", bytes: 1, mime_type: "image/png", width: 1, height: 1, data_base64: "AA==" };
const libraryAttachmentsOperation = { ...libraryOperation, kind: "attachments", item_ids: ["item-1"] };
const unsafeLibraryItem = {
  item_id: "item", logical_id: "logical", kind: "provider_snapshot", provider_id: null, provider_instance: null, resource_type: null, canonical_id: null,
  container: null, parent_item_id: null, ancestors: [], order: null, title: "Item", document_path: "item/document.md", item_path: "../escape",
  source_url: null, original_url: null, source_revision: null, revision: "rev-1", state: "fresh", partial: null, conflict: [],
  fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, attachments: [], folder: null, diagnostics: [],
};

describe("library client validation", () => {
  it("posts explicit attachment requests and rejects operations for another item", async () => {
    const fetch = vi.fn(async () => jsonResponse(libraryAttachmentsOperation));
    const browser = createBrowserClient(fetch);
    await expect(browser.libraryAttachments(libraryRequests.attachments)).resolves.toMatchObject({ kind: "attachments", item_ids: ["item-1"] });
    const started = { ...libraryAttachmentsOperation, item_ids: [], finished: false };
    await expect(createBrowserClient(vi.fn(async () => jsonResponse(started))).libraryAttachments(libraryRequests.attachments))
      .resolves.toMatchObject({ kind: "attachments", item_ids: [], finished: false });
    await expect(createNativeClient(vi.fn(async () => started)).libraryAttachments(libraryRequests.attachments))
      .resolves.toMatchObject({ kind: "attachments", item_ids: [], finished: false });
    expect(fetch).toHaveBeenCalledWith(expect.stringContaining("/api/v1/library/attachments"), expect.objectContaining({
      method: "POST",
      body: JSON.stringify(libraryRequests.attachments),
    }));

    const invoke = vi.fn(async () => libraryAttachmentsOperation);
    const native = createNativeClient(invoke);
    await expect(native.libraryAttachments(libraryRequests.attachments)).resolves.toMatchObject({ kind: "attachments", item_ids: ["item-1"] });
    expect(invoke).toHaveBeenCalledWith("cockpit_library_attachments", { request: libraryRequests.attachments });

    for (const response of [
      { ...libraryAttachmentsOperation, item_ids: ["another-item"] },
      { ...libraryAttachmentsOperation, kind: "refresh" },
      { ...libraryAttachmentsOperation, item_ids: ["item-1", "another-item"] },
    ]) {
      const wrongBrowser = createBrowserClient(vi.fn(async () => jsonResponse(response)));
      const wrongNative = createNativeClient(vi.fn(async () => response));
      await expect(wrongBrowser.libraryAttachments(libraryRequests.attachments)).rejects.toMatchObject({ code: "malformed_response" });
      await expect(wrongNative.libraryAttachments(libraryRequests.attachments)).rejects.toMatchObject({ code: "malformed_response" });
    }
    for (const client of [browser, native]) {
      await expect(client.libraryAttachments({ ...libraryRequests.attachments, attachment_ids: [] })).rejects.toMatchObject({ code: "malformed_response" });
    }
  });

  it("refuses unsafe item paths, malformed DTOs, and unmatched operation or context responses", async () => {
    const client = createBrowserClient(vi.fn(async () => jsonResponse({ ...libraryOperation, operation_id: "other" })));
    await expect(client.libraryDirectory({ path: "/absolute", offset: null, revision: null })).rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.libraryDirectory({ path: "../escape", offset: null, revision: null })).rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.libraryOperation("op/1")).rejects.toMatchObject({ code: "malformed_response" });
    const wrongContext = createBrowserClient(vi.fn(async () => jsonResponse({ ...libraryDirectory, path: "elsewhere" })));
    await expect(wrongContext.libraryDirectory(libraryRequests.directory)).rejects.toMatchObject({ code: "malformed_response" });
    const malformedItemPath = createBrowserClient(vi.fn(async () => jsonResponse({ ...libraryListing, items: [unsafeLibraryItem] })));
    await expect(malformedItemPath.libraryListing()).rejects.toMatchObject({ code: "malformed_response" });
    const native = createNativeClient(vi.fn(async () => ({ ...libraryOperation, operation_id: "other" })));
    await expect(native.libraryDocument({ ...libraryRequests.document, path: "folder/../escape" })).rejects.toMatchObject({ code: "malformed_response" });
    await expect(native.libraryOperationCancel("op/1")).rejects.toMatchObject({ code: "malformed_response" });
  });
  it("keeps reference depth and inclusions optional and bounded", async () => {
    const item = { ...unsafeLibraryItem, item_path: "item" };
    const inclusion = { holder: { kind: "follow", follow_id: "follow-1" }, from_item_id: null, from_label: "OPS-7", relation: "comment", depth: 1 };
    const plain = { ...item, item_id: "item-plain", logical_id: "logical-plain" };
    const parsed = await createBrowserClient(vi.fn(async () => jsonResponse({ ...libraryListing, items: [{ ...item, reference_depth: 2, included_by: [inclusion] }, plain] }))).libraryListing();
    expect(parsed.items[0]).toMatchObject({ reference_depth: 2, included_by: [inclusion] });
    expect("reference_depth" in parsed.items[1]! || "included_by" in parsed.items[1]!).toBe(false);
    for (const bad of [{ reference_depth: 6 }, { included_by: [{ ...inclusion, depth: 6 }] }, { included_by: [{ ...inclusion, holder: { kind: "space" } }] }]) {
      await expect(createBrowserClient(vi.fn(async () => jsonResponse({ ...libraryListing, items: [{ ...item, ...bad }] }))).libraryListing()).rejects.toMatchObject({ code: "malformed_response" });
    }
    await expect(createNativeClient(vi.fn(async () => libraryOperation)).libraryAdd({ ...libraryRequests.add, reference_depth: 6 })).rejects.toMatchObject({ code: "malformed_response" });
  });
  it("accepts nullable optional strings and configured maximum operation and directory arrays", async () => {
    const operation = {
      ...libraryOperation,
      phases: [{ phase: "library", state: "done", done: 0, total: null, message: null, error: null }],
      item_ids: Array.from({ length: 5_001 }, (_, index) => `item-${index}`),
    };
    const directory = {
      ...libraryDirectory,
      entries: Array.from({ length: 10_000 }, (_, index) => ({
        entry_id: `entry-${index}`, name: "x", path: `item-${index}`, kind: "file", bytes: null, revision: "r", refusal: null,
      })),
    };
    const request = vi.fn(async (path: string) => {
      if (path.endsWith("/resolve")) return jsonResponse({
        kind: "artifact", provider_id: null, provider_instance: null, title: "Item", canonical_id: null,
        container_label: null, existing_item_id: null, existing_follow_id: null, item_count: null, item_count_exact: true, follow_mode: null,
        git_working_tree: null, file_count: null, diagnostics: [],
      });
      return jsonResponse(path.endsWith("/directory") ? directory : operation);
    });
    const client = createBrowserClient(request);
    await expect(client.libraryResolve(libraryRequests.resolve)).resolves.toMatchObject({ provider_id: null, canonical_id: null });
    await expect(client.libraryOperation("op/1")).resolves.toMatchObject({ phases: [{ message: null }], item_ids: operation.item_ids });
    await expect(client.libraryDirectory(libraryRequests.directory)).resolves.toMatchObject({ entries: directory.entries });
  });
  it("rejects cross-Space replies, unsafe Space paths and oversized selections in both transports", async () => {
    const target = { session_id: "session", space_id: "space" };
    const other = { ...target, space_id: "other-space" };
    const listing = { target, companion: { status: "unavailable", error: { code: "source_companion_unavailable", message: "Disconnected" } }, attempts: [], rows: [], behind: 0, diagnostics: [] };
    let payload: unknown = listing;
    for (const client of [
      createBrowserClient(vi.fn(async () => jsonResponse(payload))),
      createNativeClient(vi.fn(async () => payload)),
    ]) {
      payload = { ...listing, target: other };
      await expect(client.librarySpaceList({ target })).rejects.toMatchObject({ code: "malformed_response" });
      payload = { ...listing, attempts: [{ target: other, space_label: null, item_id: "source:1", follow_id: null, title: "Saved", state: "failed", error: null, operation_id: "operation", updated_at: "now" }] };
      await expect(client.librarySpaceList({ target })).rejects.toMatchObject({ code: "malformed_response" });
      payload = { ...listing, rows: [{ item_id: "source:1", logical_id: "logical", title: "Saved", provider_id: null, resource_type: null, kind: "provider_snapshot", state: "up_to_date", library_newer: false, paths: ["../escape"], edited: [], copy_mode: null, library_revision_copied: null, current_library_revision: null, follow: null }] };
      await expect(client.librarySpaceList({ target })).rejects.toMatchObject({ code: "malformed_response" });
      payload = { ...libraryOperation, kind: "space_add", target: other };
      await expect(client.librarySpaceAdd({ target, item_ids: ["source:1"], follow_ids: [] })).rejects.toMatchObject({ code: "malformed_response" });
      await expect(client.librarySpaceAttemptsDismiss({ target, item_ids: Array(5001).fill("item"), follow_ids: [] })).rejects.toMatchObject({ code: "malformed_response" });
    }
  });
  it("sends per-Space update and removal only for this Space and refuses unsafe confirmations in both transports", async () => {
    const target = { session_id: "session", space_id: "space" };
    const other = { ...target, space_id: "other-space" };
    const listing = { target, companion: { status: "available", companion_root_id: "companion:c1", companion_label: "Context" }, attempts: [], rows: [], behind: 0, diagnostics: [] };
    const updated = { ...libraryOperation, kind: "space_update", target };
    const confirmed = [{ path: "sources/github/issue/acme-api-3.md", current_hash: "sha256:edited" }];
    const request = vi.fn(async (path: string, _init?: RequestInit) => jsonResponse(path.endsWith("/space/update") ? updated : listing));
    const invoke = vi.fn(async (command: string, _args?: Record<string, unknown>) => command === "cockpit_library_space_update" ? updated : listing);
    const browser = createBrowserClient(request);
    const native = createNativeClient(invoke);
    const update = { target, scope: { scope: "selection" as const, item_ids: ["source:a"], follow_ids: [] }, replace_edited: confirmed };
    const remove = { target, logical_id: "source:github:acme/api#3", confirmed };
    await expect(browser.librarySpaceUpdate(update)).resolves.toMatchObject({ kind: "space_update", target });
    await expect(browser.librarySpaceRemove(remove)).resolves.toMatchObject({ target });
    expect(request.mock.calls.map(([path, init]) => [path, JSON.parse(String(init?.body))])).toEqual([
      ["/api/v1/library/space/update", update],
      ["/api/v1/library/space/remove", remove],
    ]);
    await expect(native.librarySpaceUpdate({ target, scope: { scope: "all" }, replace_edited: [] })).resolves.toMatchObject({ kind: "space_update" });
    await expect(native.librarySpaceRemove(remove)).resolves.toMatchObject({ target });
    expect(invoke.mock.calls).toEqual([
      ["cockpit_library_space_update", { request: { target, scope: { scope: "all" }, replace_edited: [] } }],
      ["cockpit_library_space_remove", { request: remove }],
    ]);
    for (const client of [browser, native]) {
      await expect(client.librarySpaceRemove({ ...remove, confirmed: [{ path: "../escape", current_hash: "sha256:edited" }] })).rejects.toMatchObject({ code: "malformed_response" });
      await expect(client.librarySpaceUpdate({ ...update, scope: { scope: "every" } } as never)).rejects.toMatchObject({ code: "malformed_response" });
    }
    expect(request).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenCalledTimes(2);
    updated.target = other;
    listing.target = other;
    for (const client of [browser, native]) {
      await expect(client.librarySpaceUpdate(update)).rejects.toMatchObject({ code: "malformed_response" });
      await expect(client.librarySpaceRemove(remove)).rejects.toMatchObject({ code: "malformed_response" });
    }
  });
  it("browses one Confluence provider's spaces and refuses spaces of another provider in both transports", async () => {
    const space = { kind: "confluence_space", provider_id: "confluence", provider_instance: "https://acme.atlassian.net/wiki", title: "Software Development", canonical_id: "SD", container_label: "SD · Software Development", existing_item_id: null, existing_follow_id: "follow:1", item_count: 38, item_count_exact: true, follow_mode: null, git_working_tree: null, file_count: null, diagnostics: [] };
    let payload: unknown = [space];
    const request = vi.fn(async (_path: string, _init?: RequestInit) => jsonResponse(payload));
    const invoke = vi.fn(async (_command: string, _args?: Record<string, unknown>) => payload);
    const browser = createBrowserClient(request);
    const native = createNativeClient(invoke);
    await expect(browser.libraryConfluenceSpaces({ provider_id: "confluence" })).resolves.toEqual([space]);
    await expect(native.libraryConfluenceSpaces({ provider_id: "confluence" })).resolves.toEqual([space]);
    expect(request.mock.calls.map(([path, init]) => [path, init?.method, JSON.parse(String(init?.body))])).toEqual([["/api/v1/library/confluence/spaces", "POST", { provider_id: "confluence" }]]);
    expect(invoke.mock.calls).toEqual([["cockpit_library_confluence_spaces", { request: { provider_id: "confluence" } }]]);
    for (const client of [browser, native]) {
      await expect(client.libraryConfluenceSpaces({ provider_id: "" })).rejects.toMatchObject({ code: "malformed_response" });
      payload = [{ ...space, provider_id: "other" }];
      await expect(client.libraryConfluenceSpaces({ provider_id: "confluence" })).rejects.toMatchObject({ code: "malformed_response" });
      payload = [{ ...space, kind: "confluence_page" }];
      await expect(client.libraryConfluenceSpaces({ provider_id: "confluence" })).rejects.toMatchObject({ code: "malformed_response" });
      payload = [space];
    }
  });
  it("parses Jira query follows, item refs and dropped outcomes strictly", async () => {
    const jiraFollow = { follow_id: "follow:j", provider_id: "jira", provider_instance: "https://jira.test", source: { kind: "jira_query", jql: "project = OPS", mode: "live" }, include_attachments: false, item_count: 2, partial: null, excluded_ids: [], last_refreshed_at: null, state: "fresh" };
    const issue = { ...unsafeLibraryItem, item_path: "jira/x/OPS/OPS-1", refs: [{ kind: "follow", follow_id: "follow:j" }, { kind: "space", companion_root_id: "companion:c" }], purge_after: "0", issue: { updated: "2026-01-01 10:00:00", fetched_updated: null, status: "Open", issue_type: "Task", assignee: null } };
    let payload: unknown = { root: { root_id: "library:fs", kind: "library", label: "Library", path: "/l", repository_id: "repo", checkout_path: "", companion_id: null }, generation: "1", items: [issue], follows: [jiraFollow], next_offset: null, diagnostics: [] };
    const browser = createBrowserClient(vi.fn(async () => jsonResponse(payload)));
    await expect(browser.libraryListing()).resolves.toMatchObject({ follows: [{ source: { kind: "jira_query", mode: "live" }, item_count: 2 }], items: [{ refs: [{ kind: "follow" }, { kind: "space" }] }] });
    payload = { ...(payload as object), follows: [{ ...jiraFollow, source: { kind: "jira_query", jql: "x", mode: "sometimes" } }] };
    await expect(browser.libraryListing()).rejects.toMatchObject({ code: "malformed_response" });
    payload = { ...(payload as object), follows: [], items: [{ ...issue, refs: [{ kind: "pin" }] }] };
    await expect(browser.libraryListing()).rejects.toMatchObject({ code: "malformed_response" });
  });
});


describe("browser CockpitClient", () => {
  it("maps named sessions, encoded snapshots, and focus", async () => {
    const request = vi.fn(async (input: string, init?: RequestInit) => {
      if (input === "/api/v1/sessions") return jsonResponse(sessions);
      if (input === "/api/v1/sessions/session_1/snapshot") return jsonResponse({ ...snapshot, session_id: "session_1" });
      if (input === "/api/v1/sessions/session_1/focus") {
        expect(init?.method).toBe("POST");
        expect(init?.body).toBe(JSON.stringify({ kind: "pane", target_id: "pane-1" }));
        return jsonResponse({ session_id: "session_1", kind: "pane", target_id: "pane-1", accepted: true });
      }
      expect(input).toBe("/api/v1/sessions/session_1/mutations");
      expect(init?.method).toBe("POST");
      expect(init?.body).toBe(JSON.stringify({ type: "pane_close", pane_id: "pane-1" }));
      return jsonResponse({ session_id: "session_1", snapshot: { ...snapshot, session_id: "session_1" } });
    });
    const client = createBrowserClient(request);
    await expect(client.sessions()).resolves.toEqual(sessions);
    await expect(client.sessionSnapshot("session_1")).resolves.toMatchObject({ session_id: "session_1" });
    await expect(client.focus("session_1", { kind: "pane", target_id: "pane-1" })).resolves.toMatchObject({ accepted: true });
    expect(request).toHaveBeenCalledTimes(3);
    await expect(client.mutate("session_1", { type: "pane_close", pane_id: "pane-1" })).resolves.toMatchObject({ session_id: "session_1" });
    expect(request).toHaveBeenCalledTimes(4);
  });

  it("validates stream order, reports gaps and malformed messages, and closes idempotently", async () => {
    const socket = new FakeSocket();
    const factory = vi.fn(() => socket);
    const errors: CockpitClientError[] = [];
    const messages: unknown[] = [];
    const client = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), factory);
    const streamPromise = client.subscribeSession("session-1", (message) => messages.push(message), (error) => errors.push(error));
    expect(factory).toHaveBeenCalledWith("/api/v1/sessions/session-1/events");
    socket.open();
    const stream = await streamPromise;
    socket.message(JSON.stringify(streamSnapshot(1)));
    socket.message(JSON.stringify(streamStale(2)));
    socket.message(JSON.stringify(streamSnapshot(5, 2)));
    expect(messages).toHaveLength(2);
    expect(errors.at(-1)).toMatchObject({ code: "stream_error", operationCode: "stream_sequence" });
    const errorCount = errors.length;
    socket.message("not json");
    expect(errors).toHaveLength(errorCount);
    stream.close();
    stream.close();
  });
  it("requires a session stream to start at sequence one", async () => {
    const socket = new FakeSocket();
    const errors: CockpitClientError[] = [];
    const open = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), () => socket).subscribeSession("session-1", vi.fn(), (error) => errors.push(error));
    socket.open();
    await open;
    socket.message(JSON.stringify(streamSnapshot(2)));
    expect(errors.at(-1)?.message).toMatch(/begin at sequence 1/);
    expect(socket.readyState).toBe(3);
  });
  it("cancels a browser session handshake without reporting a stale error", async () => {
    const socket = new FakeSocket();
    const controller = new AbortController();
    const errors = vi.fn();
    const open = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), () => socket)
      .subscribeSession("session-1", vi.fn(), errors, controller.signal);
    controller.abort();
    await expect(open).rejects.toMatchObject({ code: "stream_error", message: "Session subscription was cancelled" });
    expect(socket.readyState).toBe(3);
    socket.open();
    socket.message(JSON.stringify(streamSnapshot(1)));
    expect(errors).not.toHaveBeenCalled();
  });


  it("opens a per-pane terminal with fitted dimensions and forwards text and bytes", async () => {
    const socket = new FakeSocket();
    const factory = vi.fn(() => socket);
    const client = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), factory);
    const messages = vi.fn();
    const open = client.openTerminal(terminalOpen({ session_id: "s_1", pane_id: "w1A:p1", mode: "control" }), messages, vi.fn());
    expect(factory).toHaveBeenCalledWith("/api/v1/sessions/s_1/panes/w1A%3Ap1/terminal?mode=control&takeover=false&cols=80&rows=24&cell_width_px=8&cell_height_px=16");
    socket.open();
    const stream = await open;
    socket.message(JSON.stringify({ type: "ownership", session_id: "s_1", pane_id: "w1A:p1", stream_id: "stream-1", state: "owned", message: null }));
    socket.message(JSON.stringify({ type: "frame", session_id: "s_1", pane_id: "w1A:p1", stream_id: "stream-1", seq: "1", encoding: "ansi", width: 80, height: 24, full: true, bytes: "aGVsbG8=" }));
    expect(messages).toHaveBeenCalledWith(expect.objectContaining({ type: "frame", seq: "1", full: true }));
    stream.send({ type: "terminal.input", text: "hello", bytes: null });
    stream.send({ type: "terminal.input", text: null, bytes: "AP8=" });
    stream.send({ type: "terminal.mouse", kind: "down", button: "left", column: 12, row: 7, modifiers: 0 });
    expect(socket.sent).toEqual([
      JSON.stringify({ type: "terminal.input", text: "hello", bytes: null }),
      JSON.stringify({ type: "terminal.input", text: null, bytes: "AP8=" }),
      JSON.stringify({ type: "terminal.mouse", kind: "down", button: "left", column: 12, row: 7, modifiers: 0 }),
    ]);
    stream.close();
    stream.close();
    expect(socket.readyState).toBe(3);
  });

  it("aborts a browser terminal before the handshake and ignores later events", async () => {
    const socket = new FakeSocket();
    const controller = new AbortController();
    const messages = vi.fn();
    const errors = vi.fn();
    const open = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), () => socket).openTerminal(terminalOpen(), messages, errors, controller.signal);
    controller.abort();
    await expect(open).rejects.toMatchObject({ code: "stream_error", message: "Terminal attach was cancelled" });
    socket.open();
    socket.message(JSON.stringify({ type: "ownership", session_id: "session-1", pane_id: "pane-1", stream_id: "late", state: "owned", message: null }));
    expect(messages).not.toHaveBeenCalled();
    expect(errors).not.toHaveBeenCalled();
  });

  it("rejects duplicate and skipped terminal full frames", async () => {
    for (const invalidSequence of ["1", "3"]) {
      const socket = new FakeSocket();
      const errors: CockpitClientError[] = [];
      const messages: unknown[] = [];
      const client = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), () => socket);
      const open = client.openTerminal(terminalOpen(), (message) => messages.push(message), (error) => errors.push(error));
      socket.open();
      await open;
      const frame = (seq: string) => ({
        type: "frame",
        session_id: "session-1",
        pane_id: "pane-1",
        stream_id: "stream-1",
        seq,
        encoding: "ansi",
        width: 80,
        height: 24,
        full: true,
        bytes: "",
      });
      socket.message(JSON.stringify(frame("1")));
      socket.message(JSON.stringify(frame(invalidSequence)));
      expect(messages).toHaveLength(1);
      expect(errors.at(-1)?.message).toMatch(/not consecutive/);
      expect(socket.readyState).toBe(3);
    }
  });

  it("rejects terminal target mismatches and initial incremental frames", async () => {
    const first = new FakeSocket();
    const second = new FakeSocket();
    const sockets = [first, second];
    const factory = vi.fn(() => sockets.shift()!);
    const errors: CockpitClientError[] = [];
    const client = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), factory);
    const firstOpen = client.openTerminal(terminalOpen(), vi.fn(), (error) => errors.push(error));
    first.open();
    await firstOpen;
    first.message(JSON.stringify({ type: "ownership", session_id: "other", pane_id: "pane-1", stream_id: "stream-1", state: "observing", message: null }));
    expect(first.readyState).toBe(3);
    const secondOpen = client.openTerminal(terminalOpen(), vi.fn(), (error) => errors.push(error));
    second.open();
    await secondOpen;
    second.message(JSON.stringify({ type: "frame", session_id: "session-1", pane_id: "pane-1", stream_id: "stream-2", seq: "1", encoding: "ansi", width: 80, height: 24, full: false, bytes: "" }));
    expect(errors).toHaveLength(2);
    expect(second.readyState).toBe(3);
  });
  it("preserves backend HTTP error envelopes", async () => {
    const client = createBrowserClient(vi.fn(async () => jsonResponse({ code: "live_inspection_disabled", message: "disabled" }, 503)));
    await expect(client.sessions()).rejects.toMatchObject({ code: "http_error", status: 503, operationCode: "live_inspection_disabled", message: "disabled" });
  });

  it("rejects mutation response and snapshot session mismatches", async () => {
    const wrongEnvelope = createBrowserClient(vi.fn(async () => jsonResponse({ session_id: "other", snapshot: { ...snapshot, session_id: "other" } })));
    await expect(wrongEnvelope.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).rejects.toMatchObject({ code: "malformed_response" });
    const wrongSnapshot = createBrowserClient(vi.fn(async () => jsonResponse({ session_id: "session-1", snapshot: { ...snapshot, session_id: "other" } })));
    await expect(wrongSnapshot.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).rejects.toMatchObject({ code: "malformed_response" });
  });
});

describe("native CockpitClient", () => {
  it("matches browser stream messages and cancellation semantics", async () => {
    let sessionChannel: NativeChannel<unknown> | undefined;
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const invoke = vi.fn(async (command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === "cockpit_sessions") return sessions;
      if (command === "cockpit_session_snapshot") return snapshot;
      if (command === "cockpit_focus") return { session_id: "session-1", kind: "pane", target_id: "pane-1", accepted: true };
      if (command === "cockpit_session_subscribe") return "sub-1";
      if (command === "cockpit_terminal_open") return "term-1";
      if (command === "cockpit_mutate") return { session_id: "session-1", snapshot };
      return undefined;
    });
    const channels = <T,>(onMessage: (message: T) => void): NativeChannel<T> => {
      const channel = { onmessage: onMessage };
      if (sessionChannel === undefined) sessionChannel = channel as NativeChannel<unknown>;
      return channel;
    };
    const errors: CockpitClientError[] = [];
    const client = createNativeClient(invoke, channels);
    await expect(client.sessionSnapshot("session-1")).resolves.toEqual(snapshot);
    await expect(client.focus("session-1", { kind: "pane", target_id: "pane-1" })).resolves.toMatchObject({ accepted: true });
    const subscription = await client.subscribeSession("session-1", vi.fn(), (error) => errors.push(error));
    await expect(client.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).resolves.toEqual({ session_id: "session-1", snapshot });
    sessionChannel!.onmessage(streamSnapshot(1));
    subscription.close();
    subscription.close();
    const terminal = await client.openTerminal(terminalOpen({ mode: "control" }), vi.fn(), (error) => errors.push(error));
    expect(calls.find(([command]) => command === "cockpit_terminal_open")?.[1]).toMatchObject({ request: terminalOpen({ mode: "control" }) });
    terminal.send({ type: "terminal.release" });
    terminal.close();
    terminal.close();
    expect(calls.filter(([command]) => command === "cockpit_stream_cancel")).toHaveLength(2);
    expect(calls).toContainEqual(["cockpit_terminal_command", { streamId: "term-1", command: { type: "terminal.release" } }]);
    expect(calls).toContainEqual(["cockpit_mutate", { sessionId: "session-1", request: { type: "pane_close", pane_id: "pane-1" } }]);
    expect(errors).toEqual([]);
  });

  it("rejects native terminal first-frame violations", async () => {
    let channel: NativeChannel<unknown> | undefined;
    const invoke = vi.fn(async (command: string) => command === "cockpit_terminal_open" ? "term-1" : undefined);
    const channelFactory = <T,>(onMessage: (message: T) => void): NativeChannel<T> => {
      const created = { onmessage: onMessage };
      channel = created as NativeChannel<unknown>;
      return created;
    };
    const errors: CockpitClientError[] = [];
    const stream = await createNativeClient(invoke, channelFactory).openTerminal(terminalOpen(), vi.fn(), (error) => errors.push(error));
    channel!.onmessage({ type: "frame", session_id: "session-1", pane_id: "pane-1", stream_id: "term-1", seq: "1", encoding: "ansi", width: 80, height: 24, full: false, bytes: "" });

    expect(errors).toHaveLength(1);
    stream.close();
    expect(invoke).toHaveBeenCalledWith("cockpit_stream_cancel", { streamId: "term-1" });
  });
  it("cancels a native terminal that resolves after its opening caller aborts", async () => {
    let resolveOpen: ((value: unknown) => void) | undefined;
    const controller = new AbortController();
    const invoke = vi.fn((command: string) => command === "cockpit_terminal_open"
      ? new Promise<unknown>((resolve) => { resolveOpen = resolve; })
      : Promise.resolve(undefined));
    const client = createNativeClient(invoke, <T,>(onmessage: (message: T) => void) => ({ onmessage }));
    const open = client.openTerminal(terminalOpen(), vi.fn(), vi.fn(), controller.signal);
    controller.abort();
    await expect(open).rejects.toMatchObject({ code: "stream_error", message: "Terminal attach was cancelled" });
    resolveOpen!("late-stream");
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("cockpit_stream_cancel", { streamId: "late-stream" }));
  });
  it("cancels a native session handshake and retires its late stream id once", async () => {
    let resolveSubscribe: ((value: unknown) => void) | undefined;
    const controller = new AbortController();
    const errors = vi.fn();
    const invoke = vi.fn((command: string) => command === "cockpit_session_subscribe"
      ? new Promise<unknown>((resolve) => { resolveSubscribe = resolve; })
      : Promise.resolve(undefined));
    const open = createNativeClient(invoke, <T,>(onmessage: (message: T) => void) => ({ onmessage }))
      .subscribeSession("session-1", vi.fn(), errors, controller.signal);
    controller.abort();
    await expect(open).rejects.toMatchObject({ code: "stream_error" });
    resolveSubscribe!("late-session-stream");
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("cockpit_stream_cancel", { streamId: "late-session-stream" }));
    expect(invoke.mock.calls.filter(([command]) => command === "cockpit_stream_cancel")).toHaveLength(1);
    expect(errors).not.toHaveBeenCalled();
  });
  it("settles native snapshot cancellation before the host response", async () => {
    let rejectInvoke!: (reason: unknown) => void;
    const hostResponse = new Promise<unknown>((_resolve, reject) => { rejectInvoke = reject; });
    const client = createNativeClient(async () => hostResponse);
    const controller = new AbortController();
    const read = client.sessionSnapshot("session-1", controller.signal);
    controller.abort();
    await expect(read).rejects.toMatchObject({ name: "AbortError" });
    rejectInvoke(new Error("late native failure"));
    await Promise.resolve();
    await Promise.resolve();
  });

  it("aborts a live native session and ignores subsequent channel events", async () => {
    let channel: NativeChannel<unknown> | undefined;
    const invoke = vi.fn(async (command: string) => command === "cockpit_session_subscribe" ? "live-session" : undefined);
    const channelFactory = <T,>(onmessage: (message: T) => void): NativeChannel<T> => {
      channel = { onmessage } as NativeChannel<unknown>;
      return channel as NativeChannel<T>;
    };
    const messages = vi.fn();
    const errors = vi.fn();
    const controller = new AbortController();
    const stream = await createNativeClient(invoke, channelFactory).subscribeSession("session-1", messages, errors, controller.signal);
    channel!.onmessage(streamSnapshot(1));
    expect(messages).toHaveBeenCalledOnce();
    controller.abort();
    channel!.onmessage(streamSnapshot(2));
    expect(messages).toHaveBeenCalledOnce();
    expect(errors).not.toHaveBeenCalled();
    expect(invoke).toHaveBeenCalledWith("cockpit_stream_cancel", { streamId: "live-session" });
    stream.close();
    expect(invoke.mock.calls.filter(([command]) => command === "cockpit_stream_cancel")).toHaveLength(1);
  });
  it("closes native session streams on invalid generation transitions", async () => {
    let channel: NativeChannel<unknown> | undefined;
    const calls: string[] = [];
    const invoke = vi.fn(async (command: string) => {
      calls.push(command);
      return command === "cockpit_session_subscribe" ? "sub-1" : undefined;
    });
    const channelFactory = <T,>(onMessage: (message: T) => void): NativeChannel<T> => {
      channel = { onmessage: onMessage } as NativeChannel<unknown>;
      return channel as NativeChannel<T>;
    };
    const errors: CockpitClientError[] = [];
    const messages: unknown[] = [];
    const stream = await createNativeClient(invoke, channelFactory).subscribeSession(
      "session-1",
      (message) => messages.push(message),
      (error) => errors.push(error),
    );
    channel!.onmessage(streamSnapshot(1));
    channel!.onmessage(streamSnapshot(5, 2));
    expect(messages).toHaveLength(1);
    expect(errors.at(-1)).toMatchObject({ code: "stream_error", operationCode: "stream_sequence" });
    expect(calls).toContain("cockpit_stream_cancel");
    stream.close();
  });

  it("rejects duplicate and skipped native terminal full frames", async () => {
    for (const invalidSequence of ["1", "3"]) {
      let channel: NativeChannel<unknown> | undefined;
      const invoke = vi.fn(async (command: string) => command === "cockpit_terminal_open" ? "term-1" : undefined);
      const channelFactory = <T,>(onMessage: (message: T) => void): NativeChannel<T> => {
        const created = { onmessage: onMessage };
        channel = created as NativeChannel<unknown>;
        return created;
      };
      const errors: CockpitClientError[] = [];
      const messages: unknown[] = [];
      const stream = await createNativeClient(invoke, channelFactory).openTerminal(
        terminalOpen(),
        (message) => messages.push(message),
        (error) => errors.push(error),
      );
      const frame = (seq: string) => ({
        type: "frame",
        session_id: "session-1",
        pane_id: "pane-1",
        stream_id: "term-1",
        seq,
        encoding: "ansi",
        width: 80,
        height: 24,
        full: true,
        bytes: "",
      });
      channel!.onmessage(frame("1"));
      channel!.onmessage(frame(invalidSequence));
      expect(messages).toHaveLength(1);
      expect(errors.at(-1)?.message).toMatch(/not consecutive/);
      expect(invoke).toHaveBeenCalledWith("cockpit_stream_cancel", { streamId: "term-1" });
      stream.close();
    }
  });

  it("preserves native backend error envelopes", async () => {
    const invoke = vi.fn(async () => { throw { code: "live_inspection_disabled", message: "disabled" }; });
    await expect(createNativeClient(invoke).sessions()).rejects.toMatchObject({ code: "native_error", operationCode: "live_inspection_disabled", message: "disabled" });
  });

  it("rejects native mutation response and snapshot session mismatches", async () => {
    const wrongEnvelope = createNativeClient(vi.fn(async () => ({ session_id: "other", snapshot: { ...snapshot, session_id: "other" } })));
    await expect(wrongEnvelope.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).rejects.toMatchObject({ code: "malformed_response" });
    const wrongSnapshot = createNativeClient(vi.fn(async () => ({ session_id: "session-1", snapshot: { ...snapshot, session_id: "other" } })));
    await expect(wrongSnapshot.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).rejects.toMatchObject({ code: "malformed_response" });
  });
});

describe("client selector mocks", () => {
  it("accepts complete transport-neutral clients", async () => {
    const client = completeClient();
    expect(await client.sessions()).toEqual(sessions);
  });
});

it("discards saved batches through both transports with generation and attachment checks", async () => {
  const scope = { binding_id: "binding", client_id: "client" };
  const mutation = { scope, batch_id: "batch", expected_generation: 3 };
  const result = { attachment: { owner: { session_id: "session-1", pane_id: "pane-1", terminal_id: "terminal-1", source_kind: "review", source_id: "source" }, location: { workspace_id: "space-1", tab_id: "tab-1" }, ...scope }, batches: [], truncated: false };
  const request = vi.fn(async () => jsonResponse(result));
  expect(await createBrowserClient(request).commentDiscard("session-1", "pane-1", mutation)).toEqual(result);
  expect(request.mock.calls[0]).toEqual([expect.stringContaining("/comments/discard"), expect.objectContaining({ method: "POST", body: JSON.stringify(mutation) })]);
  const invoke = vi.fn(async () => result);
  const native = createNativeClient(invoke, <T,>(onmessage: (message: T) => void) => ({ onmessage }));
  expect(await native.commentDiscard("session-1", "pane-1", mutation)).toEqual(result);
  expect(invoke).toHaveBeenCalledWith("cockpit_comments_discard", { sessionId: "session-1", paneId: "pane-1", request: mutation });
  const wrong = createBrowserClient(vi.fn(async () => jsonResponse({ ...result, attachment: { ...result.attachment, binding_id: "other" } })));
  await expect(wrong.commentDiscard("session-1", "pane-1", mutation)).rejects.toThrow();
});
