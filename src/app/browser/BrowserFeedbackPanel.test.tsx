// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { BrowserFeedbackLookup, BrowserFeedbackSendRequest, BrowserViewPendingCapture, BrowserWorkScope } from "../../protocol/generated/v1";
import { BrowserFeedbackPanel, type BrowserFeedbackPanelProps } from "./BrowserFeedbackPanel";

const tabScope: BrowserWorkScope = { kind: "tab", target: { session_id: "session", tab_id: "tab", pane_id: null, endpoint_path: null } };
const pendingCapture: BrowserViewPendingCapture = { association_key: "aaaaaaaaaaaaaaaaaaaaaaaa", browser_incarnation: "current-browser", capture_id: "capture", draft_id: "draft", draft_revision: 4, annotation_ids: ["annotation"], last_error: "Disk unavailable" };

function lookup(state?: "accepted" | "rejected" | "pending" | "outcome_unknown"): BrowserFeedbackLookup {
  return {
    browser: { association: null, connection: "open", message: "Current run", cleanup: "none", cleanup_reason: null },
    feedback: { pending_count: 1, retention_seconds: 86400, captures: [{
      id: "capture", pending_ids: ["annotation"], annotations: [{ id: "annotation", kind: "region", comment: "Current issue", color: "#d62828", points: [], bounds: null, element: null }], image_path: "capture.png",
      context: { association_key: "aaaaaaaaaaaaaaaaaaaaaaaa", session_id: "session", space_id: "space", space_label: "Current Space", playwright_session: "current-browser", working_directory: "workspace", invocation: "browser", browser_instance: "current-browser", inline_provenance: null },
      page: { url: "https://example.test", title: "Original page", document_id: "document", captured_at: "2026-09-29T00:00:00Z", viewport: { width: 800, height: 600, scroll_x: 0, scroll_y: 0, device_pixel_ratio: 1, visual_scale: 1 }, image_width: 800, image_height: 600 },
    }] },
    deliveries: state ? [{ capture_id: "capture", operation_id: "original-operation", selected_ids: ["annotation"], state, message: "Original receipt" }] : [],
    drafts: { drafts: [], active_draft_limit: 8, pending_capture: null },
  };
}

describe("BrowserFeedbackPanel current-run recovery", () => {
  let host: HTMLDivElement;
  let root: Root | null = null;
  afterEach(async () => {
    if (root) await act(async () => root?.unmount());
    root = null; host?.remove(); vi.restoreAllMocks();
  });
  async function render(client: CockpitClient, scope: BrowserWorkScope = tabScope, props: Partial<BrowserFeedbackPanelProps> = {}): Promise<void> {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    host = document.createElement("div"); document.body.append(host);
    await act(async () => {
      root = createRoot(host);
      root.render(<BrowserFeedbackPanel {...props} client={client} scope={scope} />);
    });
  }
  const click = async (label: string): Promise<void> => {
    const button = [...host.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === label);
    expect(button).toBeDefined();
    await act(async () => button!.click());
  };

  it("delivers current-run feedback and removes accepted pending feedback after acknowledgement", async () => {
    let saved = lookup();
    const send = vi.fn(async (request: BrowserFeedbackSendRequest) => {
      saved = lookup("accepted");
      saved.deliveries[0].operation_id = request.operation_id;
      return { operation_id: request.operation_id, state: "accepted" as const, target: null, acknowledged_ids: [], pending_count: 1, message: "Pasted" };
    });
    const client = {
      browserFeedback: vi.fn(async () => saved), sendBrowserFeedback: send,
      acknowledgeBrowserFeedback: vi.fn(async () => {
        saved = { ...saved, feedback: { ...saved.feedback, captures: [], pending_count: 0 }, deliveries: [] };
        return { acknowledged_ids: ["annotation"], remaining: 0 };
      }),
    } as unknown as CockpitClient;
    await render(client);
    expect(host.querySelector('[aria-label="Browser feedback"]')).not.toBeNull();
    expect(host.textContent).toContain("Current issue");
    await click("Retry saved feedback");
    expect(send).toHaveBeenCalledExactlyOnceWith({ scope: tabScope, ids: ["annotation"], operation_id: "browser-feedback-capture", acknowledge_duplicate_risk: false });
    expect(host.textContent).toContain("Pasted to selected agent · Enter not sent");
    expect(host.querySelector('[aria-label="Saved feedback recovery"]')).toBeNull();
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
  });

  it("blocks a retry when the receipt's selected identity changes", async () => {
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

  it("retries a current-run pending capture and attempts delivery after preserving the recovered capture", async () => {
    let saved = lookup(); saved.feedback.captures = []; saved.feedback.pending_count = 0;
    saved.drafts!.pending_capture = pendingCapture;
    const recovered = vi.fn();
    const recovery = vi.fn(async () => {
      saved = lookup();
      return { type: "capture" as const, capture: { state: "saved" as const, saved: { capture_id: "capture", annotation_ids: ["annotation"], image_path: "capture.png", pending_count: 1 } } };
    });
    const send = vi.fn(async (request: BrowserFeedbackSendRequest) => ({ operation_id: request.operation_id, state: "rejected" as const, target: null, acknowledged_ids: [], pending_count: 1, message: "Agent unavailable" }));
    const client = { browserFeedback: vi.fn(async () => saved), browserDraftRecovery: recovery, sendBrowserFeedback: send } as unknown as CockpitClient;
    await render(client, tabScope, { onRecovered: recovered });
    await click("Retry pending capture");
    expect(recovery).toHaveBeenCalledExactlyOnceWith({ scope: tabScope, action: { type: "retry_pending" } });
    expect(send).toHaveBeenCalledExactlyOnceWith({ scope: tabScope, ids: ["annotation"], operation_id: "browser-feedback-capture", acknowledge_duplicate_risk: false });
    expect(recovered).toHaveBeenCalledWith(expect.objectContaining({ type: "capture" }), pendingCapture);
    expect(host.textContent).toContain("Agent unavailable");
    expect(host.textContent).toContain("Current issue");
  });
  it("preserves a newer draft instead of discarding the revision the user has not reviewed", async () => {
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
    const client = { browserFeedback: vi.fn(async () => saved), browserDraftRecovery: recovery } as unknown as CockpitClient;
    await render(client);
    expect(host.textContent).toContain("Original saved note");
    await click("Discard current-run draft");
    expect(recovery).toHaveBeenCalledExactlyOnceWith({ scope: tabScope, action: { type: "list" } });
    expect(host.textContent).toContain("Newer saved note");
    expect(host.textContent).toContain("review it before discarding");
  });

  it("acknowledges an accepted receipt without replaying its paste", async () => {
    let current = lookup("accepted");
    const send = vi.fn();
    const acknowledge = vi.fn(async () => {
      current = { ...current, feedback: { ...current.feedback, captures: [], pending_count: 0 }, deliveries: [] };
      return { acknowledged_ids: ["annotation"], remaining: 0 };
    });
    const client = { browserFeedback: vi.fn(async () => current), sendBrowserFeedback: send, acknowledgeBrowserFeedback: acknowledge } as unknown as CockpitClient;
    await render(client);
    await click("Acknowledge saved receipt");
    expect(send).not.toHaveBeenCalled();
    expect(acknowledge).toHaveBeenCalledExactlyOnceWith({ scope: tabScope, ids: ["annotation"] });
    expect(host.querySelector('[aria-label="Saved feedback recovery"]')).toBeNull();
  });

  it("retains an unconfirmed operation and blocks replay when its receipt disappears", async () => {
    const current = lookup();
    const send = vi.fn(async () => { throw new Error("Transport closed"); });
    const client = { browserFeedback: vi.fn(async () => current), sendBrowserFeedback: send } as unknown as CockpitClient;
    await render(client);
    await click("Retry saved feedback");
    expect(host.textContent).toContain("the original operation was retained");
    await act(async () => root!.render(<BrowserFeedbackPanel client={client} scope={tabScope} refreshKey={1} />));
    expect(send).toHaveBeenCalledOnce();
    expect(host.textContent).toContain("The operation receipt is unavailable; no paste can be replayed.");
    expect([...host.querySelectorAll<HTMLButtonElement>("button")].some((button) => button.textContent === "Resolve and retry")).toBe(false);
  });

  it("does not acknowledge a response with a different delivery operation identity", async () => {
    const acknowledge = vi.fn();
    const send = vi.fn(async () => ({ operation_id: "different-operation", state: "accepted" as const, target: null, acknowledged_ids: [], pending_count: 1, message: "Pasted" }));
    const client = { browserFeedback: vi.fn(async () => lookup()), sendBrowserFeedback: send, acknowledgeBrowserFeedback: acknowledge } as unknown as CockpitClient;
    await render(client);
    await click("Retry saved feedback");
    expect(acknowledge).not.toHaveBeenCalled();
    expect(host.textContent).toContain("different operation identity");
    expect(host.textContent).toContain("Current issue");
  });

  it("retains a pending capture when saving it still fails", async () => {
    const current = lookup();
    current.feedback.captures = []; current.feedback.pending_count = 0;
    current.drafts!.pending_capture = pendingCapture;
    const nextPending = { ...pendingCapture, last_error: "Still unavailable" };
    const recovery = vi.fn(async () => {
      current.drafts!.pending_capture = nextPending;
      return { type: "capture" as const, capture: { state: "pending" as const, pending: nextPending } };
    });
    const send = vi.fn();
    const client = { browserFeedback: vi.fn(async () => current), browserDraftRecovery: recovery, sendBrowserFeedback: send } as unknown as CockpitClient;
    await render(client);
    await click("Retry pending capture");
    expect(send).not.toHaveBeenCalled();
    expect(host.textContent).toContain("Still unavailable");
    expect([...host.querySelectorAll("button")].map((button) => button.textContent)).toContain("Retry pending capture");
    expect([...host.querySelectorAll("button")].map((button) => button.textContent)).toContain("Discard pending capture");
  });

  it("ignores a previous tab's feedback lookup after the scope changes", async () => {
    let finishLookup!: (value: BrowserFeedbackLookup) => void;
    const delayed = new Promise<BrowserFeedbackLookup>((resolve) => { finishLookup = resolve; });
    const nextScope: BrowserWorkScope = { kind: "tab", target: { ...tabScope.target, tab_id: "next-tab" } };
    const next = lookup(); next.feedback.captures[0].annotations[0].comment = "Next tab issue";
    const client = {
      browserFeedback: vi.fn(({ scope }: { scope: BrowserWorkScope }) => JSON.stringify(scope) === JSON.stringify(tabScope) ? delayed : Promise.resolve(next)),
    } as unknown as CockpitClient;
    await render(client);
    await act(async () => root!.render(<BrowserFeedbackPanel client={client} scope={nextScope} />));
    expect(host.textContent).toContain("Next tab issue");
    await act(async () => finishLookup(lookup()));
    expect(host.textContent).toContain("Next tab issue");
    expect(host.textContent).not.toContain("Current issue");
  });

  it("ignores a previous tab's completed delivery after the scope changes", async () => {
    let finishSend!: (value: { operation_id: string; state: "accepted"; target: null; acknowledged_ids: string[]; pending_count: number; message: string }) => void;
    const delayed = new Promise<Parameters<typeof finishSend>[0]>((resolve) => { finishSend = resolve; });
    const acknowledge = vi.fn();
    const send = vi.fn(() => delayed);
    const nextScope: BrowserWorkScope = { kind: "tab", target: { ...tabScope.target, tab_id: "next-tab" } };
    const next = lookup(); next.feedback.captures = []; next.feedback.pending_count = 0;
    const client = {
      browserFeedback: vi.fn(async ({ scope }: { scope: BrowserWorkScope }) => JSON.stringify(scope) === JSON.stringify(tabScope) ? lookup() : next),
      sendBrowserFeedback: send, acknowledgeBrowserFeedback: acknowledge,
    } as unknown as CockpitClient;
    await render(client);
    await click("Retry saved feedback");
    expect(send).toHaveBeenCalledOnce();
    await act(async () => root!.render(<BrowserFeedbackPanel client={client} scope={nextScope} />));
    await act(async () => finishSend({ operation_id: "browser-feedback-capture", state: "accepted", target: null, acknowledged_ids: [], pending_count: 1, message: "Pasted" }));
    expect(acknowledge).not.toHaveBeenCalled();
    expect(host.textContent).not.toContain("Pasted");
    expect(host.textContent).not.toContain("Current issue");
  });
});
