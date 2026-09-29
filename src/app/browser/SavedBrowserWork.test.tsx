// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { BrowserFeedbackLookup, BrowserFeedbackSendRequest, BrowserViewPendingCapture, BrowserWorkScope, CommentPasteTarget } from "../../protocol/generated/v1";
import { SavedBrowserWork } from "./SavedBrowserWork";

const archiveScope: BrowserWorkScope = { kind: "legacy_archive", association_key: "aaaaaaaaaaaaaaaaaaaaaaaa" };
const tabScope: BrowserWorkScope = { kind: "tab", target: { session_id: "session", tab_id: "tab", pane_id: null, endpoint_path: null } };
const recipient: CommentPasteTarget = { endpoint_identity: "endpoint", session_id: "session", workspace_id: "workspace", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", agent_label: "Fixture agent", agent_fingerprint: "fingerprint" };
const pendingCapture: BrowserViewPendingCapture = { association_key: "aaaaaaaaaaaaaaaaaaaaaaaa", browser_incarnation: "old-browser", capture_id: "capture", draft_id: "draft", draft_revision: 4, annotation_ids: ["annotation"], last_error: "Disk unavailable" };

function lookup(state?: "accepted" | "rejected" | "pending" | "outcome_unknown"): BrowserFeedbackLookup {
  return {
    browser: { association: null, connection: "absent", message: "Archived", cleanup: "none", cleanup_reason: null },
    feedback: { pending_count: 1, retention_seconds: 86400, captures: [{
      id: "capture", pending_ids: ["annotation"], annotations: [{ id: "annotation", kind: "region", comment: "Saved issue", color: "#d62828", points: [], bounds: null, element: null }], image_path: "capture.png",
      context: { association_key: "aaaaaaaaaaaaaaaaaaaaaaaa", session_id: "session", space_id: "old-space", space_label: "Original Space", playwright_session: "old-browser", working_directory: "workspace", invocation: "browser", browser_instance: "old-browser", inline_provenance: null },
      page: { url: "https://example.test", title: "Original page", document_id: "document", captured_at: "2026-09-29T00:00:00Z", viewport: { width: 800, height: 600, scroll_x: 0, scroll_y: 0, device_pixel_ratio: 1, visual_scale: 1 }, image_width: 800, image_height: 600 },
    }] },
    deliveries: state ? [{ capture_id: "capture", operation_id: "original-operation", selected_ids: ["annotation"], state, message: "Original receipt" }] : [],
    drafts: { drafts: [], active_draft_limit: 8, pending_capture: null },
  };
}

describe("SavedBrowserWork recovery", () => {
  let host: HTMLDivElement;
  let root: Root | null = null;
  afterEach(async () => {
    if (root) await act(async () => root?.unmount());
    root = null; host?.remove(); vi.restoreAllMocks();
  });
  async function render(client: CockpitClient, scope: BrowserWorkScope, onRecovered?: () => void): Promise<void> {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    host = document.createElement("div"); document.body.append(host);
    await act(async () => {
      root = createRoot(host);
      root.render(<SavedBrowserWork client={client} scope={scope} sessionId="session" onRecovered={onRecovered} />);
    });
  }
  const click = async (label: string): Promise<void> => {
    const button = [...host.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === label);
    expect(button).toBeDefined();
    await act(async () => button!.click());
  };

  it("requires an explicit legacy recipient and preserves the archived scope when sending", async () => {
    let saved = lookup();
    const send = vi.fn(async (request: BrowserFeedbackSendRequest) => {
      saved = lookup("accepted");
      saved.deliveries[0].operation_id = request.operation_id;
      return { operation_id: request.operation_id, state: "accepted" as const, target: request.recipient, acknowledged_ids: [], pending_count: 1, message: "Pasted" };
    });
    const client = {
      browserFeedback: vi.fn(async () => saved), browserLegacyRecipients: vi.fn(async () => [recipient]), sendBrowserFeedback: send,
      acknowledgeBrowserFeedback: vi.fn(async () => {
        saved = { ...saved, feedback: { ...saved.feedback, captures: [], pending_count: 0 }, deliveries: [] };
        return { acknowledged_ids: ["annotation"], remaining: 0 };
      }),
    } as unknown as CockpitClient;
    await render(client, archiveScope);
    const retry = host.querySelector<HTMLButtonElement>('[aria-label="Retry saved capture 1"]')!;
    expect(retry.disabled).toBe(true);
    expect(send).not.toHaveBeenCalled();
    const picker = host.querySelector<HTMLSelectElement>('[aria-label="Saved feedback recipient"]')!;
    await act(async () => { picker.value = picker.options[1].value; picker.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(retry.disabled).toBe(false);
    await click("Retry saved feedback");
    expect(send).toHaveBeenCalledExactlyOnceWith({ scope: archiveScope, ids: ["annotation"], operation_id: "browser-feedback-capture", acknowledge_duplicate_risk: false, recipient });
    expect(host.textContent).toContain("Pasted to Fixture agent · Enter not sent");
    expect(host.querySelector('[aria-label="Saved feedback recovery"]')).toBeNull();
  });

  it("invalidates a chosen legacy recipient when the focused tab's agents change", async () => {
    let targets = [recipient];
    const send = vi.fn();
    const client = { browserFeedback: vi.fn(async () => lookup()), browserLegacyRecipients: vi.fn(async () => targets), sendBrowserFeedback: send } as unknown as CockpitClient;
    await render(client, archiveScope);
    const picker = host.querySelector<HTMLSelectElement>('[aria-label="Saved feedback recipient"]')!;
    await act(async () => { picker.value = picker.options[1].value; picker.dispatchEvent(new Event("change", { bubbles: true })); });
    targets = [{ ...recipient, tab_id: "other-tab", agent_fingerprint: "other-agent" }];
    await click("Retry saved feedback");
    expect(send).not.toHaveBeenCalled();
    expect(picker.value).toBe("");
    expect(host.querySelector<HTMLButtonElement>('[aria-label="Retry saved capture 1"]')!.disabled).toBe(true);
  });

  it("never replays an unknown receipt without explicit duplicate-risk acknowledgement", async () => {
    let saved = lookup("pending");
    const send = vi.fn(async (request: BrowserFeedbackSendRequest) => ({ operation_id: request.operation_id, state: "rejected" as const, target: null, acknowledged_ids: [], pending_count: 1, message: "Rejected retry" }));
    const client = { browserFeedback: vi.fn(async () => saved), sendBrowserFeedback: send } as unknown as CockpitClient;
    await render(client, tabScope);
    saved = lookup("outcome_unknown");
    await click("Retry saved feedback");
    expect(send).not.toHaveBeenCalled();
    const resolve = [...host.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === "Resolve and retry")!;
    expect(resolve.disabled).toBe(true);
    await act(async () => host.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
    await click("Resolve and retry");
    expect(send).toHaveBeenCalledOnce();
    const request = send.mock.calls[0][0];
    expect(request.scope).toEqual(tabScope);
    expect(request.ids).toEqual(["annotation"]);
    expect(request.operation_id).not.toBe("original-operation");
    expect(request.acknowledge_duplicate_risk).toBe(true);
    expect(request.recipient).toBeNull();
  });

  it("blocks a retry when the durable receipt's selected identity changes", async () => {
    let saved = lookup("rejected");
    const send = vi.fn();
    const client = { browserFeedback: vi.fn(async () => saved), sendBrowserFeedback: send } as unknown as CockpitClient;
    await render(client, tabScope);
    saved = lookup("rejected"); saved.deliveries[0].selected_ids = ["different-annotation"];
    await click("Retry saved feedback");
    expect(send).not.toHaveBeenCalled();
    expect(host.textContent).toContain("no retry was sent");
    expect(host.querySelector<HTMLButtonElement>('[aria-label="Retry saved capture 1"]')!.disabled).toBe(true);
  });

  it("recovers a legacy pending capture without assigning it to a tab or sending it", async () => {
    let saved = lookup(); saved.feedback.captures = []; saved.feedback.pending_count = 0;
    saved.drafts!.pending_capture = pendingCapture;
    const recovered = vi.fn();
    const recovery = vi.fn(async () => {
      saved = lookup();
      return { type: "capture" as const, capture: { state: "saved" as const, saved: { capture_id: "capture", annotation_ids: ["annotation"], image_path: "capture.png", pending_count: 1 } } };
    });
    const send = vi.fn();
    const client = { browserFeedback: vi.fn(async () => saved), browserLegacyRecipients: vi.fn(async () => [recipient]), browserDraftRecovery: recovery, sendBrowserFeedback: send } as unknown as CockpitClient;
    await render(client, archiveScope, recovered);
    await click("Retry pending capture");
    expect(recovery).toHaveBeenCalledExactlyOnceWith({ scope: archiveScope, action: { type: "retry_pending" } });
    expect(send).not.toHaveBeenCalled();
    expect(recovered).toHaveBeenCalledWith(expect.objectContaining({ type: "capture" }), pendingCapture);
    expect(host.querySelector<HTMLButtonElement>('[aria-label="Retry saved capture 1"]')!.disabled).toBe(true);
  });
  it("preserves a newer saved draft instead of discarding the revision the user has not reviewed", async () => {
    const saved = lookup();
    saved.feedback.captures = []; saved.feedback.pending_count = 0;
    saved.drafts!.drafts = [{
      draft_id: "draft", target_id: "original-document", document_generation: 1, revision: 4,
      annotations: [], freshness: "stale", stale: true,
      editor: { selected_annotation_id: null, note_annotation_id: null, note_text: "Original saved note", notes_open: true },
    }];
    const recovery = vi.fn(async () => {
      saved.drafts!.drafts[0] = { ...saved.drafts!.drafts[0], revision: 5, editor: { ...saved.drafts!.drafts[0].editor, note_text: "Newer saved note" } };
      return { type: "draft_inventory" as const, inventory: saved.drafts! };
    });
    const client = { browserFeedback: vi.fn(async () => saved), browserLegacyRecipients: vi.fn(async () => [recipient]), browserDraftRecovery: recovery } as unknown as CockpitClient;
    await render(client, archiveScope);
    expect(host.textContent).toContain("Original saved note");
    await click("Discard saved draft");
    expect(recovery).toHaveBeenCalledExactlyOnceWith({ scope: archiveScope, action: { type: "list" } });
    expect(host.textContent).toContain("Newer saved note");
    expect(host.textContent).toContain("review it before discarding");
  });
  it("keeps retired-tab work reachable and sends only to an explicitly chosen current recipient", async () => {
    const scope: BrowserWorkScope = { kind: "saved_tab", association_key: "aaaaaaaaaaaaaaaaaaaaaaaa" };
    const saved = lookup();
    const originalSource = JSON.stringify(saved.feedback.captures[0].context);
    const send = vi.fn(async (request: BrowserFeedbackSendRequest) => ({
      operation_id: request.operation_id, state: "rejected" as const, target: null,
      acknowledged_ids: [], pending_count: 1, message: "Retained for later delivery",
    }));
    const client = { browserFeedback: vi.fn(async () => saved), browserLegacyRecipients: vi.fn(async () => [recipient]), sendBrowserFeedback: send } as unknown as CockpitClient;
    await render(client, scope);
    expect(host.textContent).toContain("Original page");
    expect(host.querySelector<HTMLButtonElement>('[aria-label="Retry saved capture 1"]')!.disabled).toBe(true);
    const picker = host.querySelector<HTMLSelectElement>('[aria-label="Saved feedback recipient"]')!;
    await act(async () => { picker.value = picker.options[1].value; picker.dispatchEvent(new Event("change", { bubbles: true })); });
    await click("Retry saved feedback");
    expect(send).toHaveBeenCalledOnce();
    expect(send.mock.calls[0][0]).toMatchObject({ scope, recipient, ids: ["annotation"] });
    expect(JSON.stringify(saved.feedback.captures[0].context)).toBe(originalSource);
  });
});
