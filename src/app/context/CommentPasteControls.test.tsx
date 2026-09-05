// @vitest-environment jsdom
import { webcrypto } from "node:crypto";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, CommentPastePrepareResponse, CommentPreview } from "../../protocol/generated/v1";
import { CommentPasteControls } from "./CommentPasteControls";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it("requires the displayed bytes and explicit duplicate acknowledgment before sending once", async () => {
  vi.stubGlobal("crypto", webcrypto);
  const payload = "Exact reviewed prose α";
  const hash = `sha256:${Buffer.from(await webcrypto.subtle.digest("SHA-256", new TextEncoder().encode(payload))).toString("hex")}`;
  const target = { endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "agent", terminal_id: "terminal", agent_fingerprint: "fingerprint", agent_label: "Agent" };
  const prepared = { batch_id: "batch", generation: 1, payload_hash: hash, targets: [target], paste_available: true, reason: null, receipts: [{ operation_id: "prior", state: "outcome_unknown", message: null }] } as CommentPastePrepareResponse;
  const send = vi.fn(async () => ({ operation_id: "sent", state: "accepted", message: null }));
  const client = { commentPastePrepare: vi.fn(async () => prepared), commentPasteSend: send } as unknown as CockpitClient;
  const batch = { batch_id: "batch", generation: 1 } as CommentBatch;
  const scope = { binding_id: "binding", client_id: "client" };
  const accepted = vi.fn();
  const host = document.createElement("div"); document.body.append(host); const mounted = createRoot(host);
  const render = async (text: string) => {
    await act(async () => { mounted.render(<CommentPasteControls client={client} sessionId="session" paneId="source" scope={scope} batch={batch} retainStale={false} preview={{ batch_id: "batch", generation: 1, exportable: true, payload: text } as CommentPreview} onAccepted={accepted} />); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
  };
  const button = () => [...host.querySelectorAll("button")].find(item => item.textContent === "Paste to agent")!;
  try {
    await render("Different unreviewed bytes");
    await act(async () => { const select = host.querySelector("select")!; select.value = "agent"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(button().disabled).toBe(true);
    await render(payload);
    expect(button().disabled).toBe(true);
    await act(async () => { (host.querySelector('input[type="checkbox"]') as HTMLInputElement).click(); });
    expect(button().disabled).toBe(false);
    await act(async () => button().click());
    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith("session", "source", expect.objectContaining({ expected_payload_hash: hash, target, acknowledge_duplicate_risk: true }));
    expect(accepted).toHaveBeenCalledTimes(1);
    expect(button().disabled).toBe(true);
  } finally { await act(async () => mounted.unmount()); host.remove(); vi.unstubAllGlobals(); }
});

it("resolves an uncertain receipt only after input inspection without sending again", async () => {
  vi.stubGlobal("crypto", webcrypto);
  const prior = { operation_id: "uncertain-operation", state: "outcome_unknown", user_confirmed: false, message: "Delivery unconfirmed" };
  const resolve = vi.fn(async () => ({ ...prior, state: "accepted", user_confirmed: true }));
  const send = vi.fn();
  const client = { commentPastePrepare: vi.fn(async () => ({ batch_id: "batch", generation: 1, payload_hash: "different", targets: [], paste_available: false, reason: null, receipts: [prior] })), commentPasteMarkPasted: resolve, commentPasteSend: send } as unknown as CockpitClient;
  const batch = { batch_id: "batch", generation: 1 } as CommentBatch;
  const scope = { binding_id: "binding", client_id: "client" };
  const host = document.createElement("div"); document.body.append(host); const mounted = createRoot(host);
  const accepted = vi.fn();
  try {
    await act(async () => mounted.render(<CommentPasteControls client={client} sessionId="session" paneId="source" scope={scope} batch={batch} retainStale={false} preview={null} onAccepted={accepted} />));
    const button = [...host.querySelectorAll("button")].find(item => item.textContent?.startsWith("Mark pasted"))!;
    expect(button.disabled).toBe(true);
    await act(async () => (host.querySelector('input[type="checkbox"]') as HTMLInputElement).click());
    await act(async () => button.click());
    expect(resolve).toHaveBeenCalledWith("session", "source", { batch: { scope, batch_id: "batch", expected_generation: 1 }, operation_id: prior.operation_id });
    expect(send).not.toHaveBeenCalled(); expect(accepted).toHaveBeenCalledOnce();
  } finally { await act(async () => mounted.unmount()); host.remove(); vi.unstubAllGlobals(); }
});
