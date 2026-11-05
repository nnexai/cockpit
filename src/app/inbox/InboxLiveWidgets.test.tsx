// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { InboxThread, InboxWidgetEvent, WidgetSummary } from "../../protocol/generated/v1";
import { getWidgetStore, widgetKey } from "../widgets/widgetStore";
import { InboxTimelineWidget } from "./InboxLiveWidgets";
import { InboxTimeline } from "./InboxTimeline";

describe("historical timeline widgets", () => {
  it("never loads a later publication or changed attachment under a reused widget key", () => {
    const client = {} as CockpitClient;
    const binding = { run_id: "run", attempt: 1, session_id: "session", attachment_id: "attachment", attachment_epoch: 1 };
    const event: InboxWidgetEvent = { key: { session_id: "session", tab_id: "tab", id: "view" }, revision: 1, title: "Original", binding, publication_id: "original", content_sha256: "hash" };
    const store = getWidgetStore(client);
    const widget = { key: event.key, revision: 1, kind: "html", title: "Replacement", content: { sha256: "hash" }, live: { binding, publication_id: "replacement" } } as WidgetSummary;
    store.byKey.set(widgetKey(event.key), widget);
    const host = document.createElement("div");
    const root = createRoot(host);
    const render = () => root.render(createElement(InboxTimelineWidget, { client, sessionId: "session", event, visible: true }));
    act(render);
    expect(host.textContent).toContain("Original");
    expect(host.textContent).toContain("no longer available");
    expect(host.textContent).not.toContain("Replacement");
    widget.live = { ...widget.live!, publication_id: "original", binding: { ...binding, attachment_epoch: 2 } };
    act(render);
    expect(host.textContent).toContain("no longer available");
    act(() => root.unmount());
  });

  it("renders an unavailable historical publication at its event, before later messages", () => {
    const client = {} as CockpitClient;
    const event: InboxWidgetEvent = {
      key: { session_id: "session", tab_id: "tab", id: "view" }, revision: 1, title: "Published view",
      binding: { run_id: "run", attempt: 1, session_id: "session", attachment_id: "attachment", attachment_epoch: 1 },
      publication_id: "publication", content_sha256: "hash",
    };
    const entry = (message_id: string, kind: "request" | "widget" | "follow_up", text: string) => ({
      message_id, kind, text, at: "2026-11-04T10:00:00Z", source: { kind: "user" as const },
      outcome: null, images: [], in_reply_to: null,
    });
    const thread = { thread_id: "thread", timeline_omitted: 0, state: "underway", questions: [], receipts: [],
      timeline: [entry("request", "request", "Before"), { ...entry("publication", "widget", ""), widget: event }, entry("follow-up", "follow_up", "After")] } as unknown as InboxThread;
    const host = document.createElement("div");
    const root = createRoot(host);
    act(() => root.render(createElement(InboxTimeline, { thread, view: null, client, sessionId: "session", ownerLabel: id => id, visible: true, questionCard: () => null, resultActions: null, acknowledged: null, recommendation: null })));
    const text = host.textContent!;
    expect(text.indexOf("Before")).toBeLessThan(text.indexOf("Published view"));
    expect(text.indexOf("Published view")).toBeLessThan(text.indexOf("After"));
    expect(text).toContain("no longer available");
    act(() => root.unmount());
  });

  it("labels retained older publications before the visible timeline", () => {
    const client = {} as CockpitClient;
    const event: InboxWidgetEvent = {
      key: { session_id: "session", tab_id: "tab", id: "old" }, revision: 1, title: "Older view",
      binding: { run_id: "run", attempt: 1, session_id: "session", attachment_id: "attachment", attachment_epoch: 1 },
      publication_id: "old-publication", content_sha256: "hash",
    };
    const old = { message_id: "old", kind: "widget", widget: event, at: "2026-11-04T10:00:00Z",
      text: "", source: { kind: "user" }, outcome: null, images: [], in_reply_to: null };
    const thread = { thread_id: "thread", timeline_omitted: 123, earlier_widgets: [old], earlier_widgets_omitted: 3,
      state: "underway", questions: [], receipts: [], timeline: [] } as unknown as InboxThread;
    const host = document.createElement("div");
    const root = createRoot(host);
    act(() => root.render(createElement(InboxTimeline, { thread, view: null, client, sessionId: "session", ownerLabel: id => id, visible: true, questionCard: () => null, resultActions: null, acknowledged: null, recommendation: null })));
    expect(host.textContent).toContain("Earlier live views");
    expect(host.textContent).toContain("3 older live views not shown");
    expect(host.textContent).toContain("Older view");
    act(() => root.unmount());
  });
});
