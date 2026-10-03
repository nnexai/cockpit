// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import type { Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { WidgetChoicesSpec, WidgetSummary } from "../../protocol/generated/v1";
import { WidgetChip } from "./WidgetChip";
import { WidgetChoices } from "./WidgetChoices";
import { WidgetFrame } from "./WidgetFrame";
import { widgetDocument } from "./widgetDocument";
import { WidgetTabs } from "./WidgetTabs";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const widget: WidgetSummary = {
  key: { session_id: "session", tab_id: "tab", id: "stats" }, space_id: "api", title: "Requests",
  revision: 3, created_seq: 1, kind: "html", presentation: "active",
  content: { sha256: "a".repeat(64), bytes: 2048, from: "file", name: "stats.html" }, warnings: [],
  source: { pane_id: "pane", tab_id: "tab", space_id: "api", terminal_id: "terminal", agent_label: "omp", fingerprint_prefix: "123456abcdef", status: "present" },
  arrival: "own_tab", resolved_from: "current_pane", change: "opened", created_at_ms: 1000, updated_at_ms: 2000, selection: null,
};
const choices: WidgetChoicesSpec = { prompt: "Which view?", choices: [
  { id: "latency", label: "Latency", detail: "per route" }, { id: "errors", label: "Errors", detail: null },
] };
const choiceWidget: WidgetSummary = { ...widget, kind: "choices", presentation: "choices" };
let host: HTMLDivElement;
let root: Root;
let frameLoad: Event;
beforeEach(() => {
  host = document.createElement("div");
  // Only explicit test loads may complete a preview; jsdom's automatic about:blank load is not srcDoc readiness.
  frameLoad = new Event("load");
  host.addEventListener("load", (event) => {
    if (event.target instanceof HTMLIFrameElement && event !== frameLoad) event.stopImmediatePropagation();
  }, true);
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); document.body.classList.remove("is-pane-dragging"); });

function frameBridge(frame: HTMLIFrameElement): { nonce: string; revision: number; selection: unknown; hasSelection: boolean } {
  const state = frame.srcdoc.match(/const state = JSON.parse\(("(?:[^"\\]|\\.)*")\);/)!;
  return JSON.parse(JSON.parse(state[1]));
}

function frameMessage(frame: HTMLIFrameElement, payload: Record<string, unknown>) {
  const { nonce, revision } = frameBridge(frame);
  window.dispatchEvent(new MessageEvent("message", { source: frame.contentWindow, data: { nonce, revision, ...payload } }));
}

it("keeps the old opaque scripted frame until the replacement loads, then retires its focus", async () => {
  const retire = vi.fn(() => host.querySelector<HTMLElement>("[data-wrapper]")!.focus());
  const render = async (revision: number, html: string) => {
    await act(async () => root.render(<div data-wrapper tabIndex={0}><WidgetFrame document={html} revision={revision}
      currentRevision={revision} widgetKey="stats" onSelect={() => {}} agent="omp" inputBlocked={false} onFocusRetired={retire} /></div>));
  };
  await render(1, '<script>parent.compromised=true</script><p onclick="alert(1)">first</p>');
  const old = host.querySelector<HTMLIFrameElement>("iframe")!;
  expect(old.getAttribute("sandbox")).toBe("allow-scripts");
  expect(old.getAttribute("referrerpolicy")).toBe("no-referrer");
  expect(old.srcdoc).toContain("<script>parent.compromised=true</script>");
  expect(old.srcdoc).toContain('onclick="alert(1)"');
  expect(old.srcdoc).toContain("first");
  frameMessage(old, { type: "cockpit.widget.intent" });
  old.focus();
  await render(2, "<p>second</p>");
  expect(host.querySelector("iframe")).toBe(old);
  expect(host.querySelectorAll("iframe")).toHaveLength(2);
  const pending = host.querySelector<HTMLIFrameElement>("iframe[data-pending]")!;
  expect(pending.getAttribute("aria-hidden")).toBe("true");
  expect(pending.tabIndex).toBe(-1);
  expect(retire).not.toHaveBeenCalled();
  await act(async () => pending.dispatchEvent(frameLoad));
  expect(host.querySelectorAll("iframe")).toHaveLength(1);
  expect(host.querySelector("iframe")).toBe(pending);
  expect(pending.srcdoc).toContain("second");
  expect(retire).toHaveBeenCalledOnce();
  expect(document.activeElement).toBe(host.querySelector("[data-wrapper]"));
});

it("abandons an intermediate replacement and never moves focus from an outside control", async () => {
  const retire = vi.fn();
  const render = async (revision: number) => {
    await act(async () => root.render(<><button>Outside</button><WidgetFrame document={`<p>r${revision}</p>`}
      revision={revision} currentRevision={revision} widgetKey="stats" onSelect={() => {}} agent="omp" inputBlocked={false} onFocusRetired={retire} /></>));
  };
  await render(1);
  const original = host.querySelector<HTMLIFrameElement>("iframe")!;
  const outside = host.querySelector("button")!;
  outside.focus();
  await render(2);
  const abandoned = host.querySelector<HTMLIFrameElement>("iframe[data-pending]")!;
  expect(host.querySelector("iframe")).toBe(original);
  expect(abandoned.dataset.revision).toBe("2");
  await render(3);
  const latest = host.querySelector<HTMLIFrameElement>("iframe[data-pending]")!;
  expect(latest.dataset.revision).toBe("3");
  expect(host.querySelectorAll("iframe")).toHaveLength(2);
  expect(host.querySelector("iframe")).toBe(original);
  expect(abandoned.isConnected).toBe(false);
  await act(async () => {
    original.dispatchEvent(frameLoad);
    abandoned.dispatchEvent(frameLoad);
  });
  expect(host.querySelector("iframe[data-pending]")).toBe(latest);
  expect(host.querySelector("iframe")).toBe(original);
  expect(document.activeElement).toBe(outside);
  expect(retire).not.toHaveBeenCalled();
  await act(async () => latest.dispatchEvent(frameLoad));
  expect(host.querySelectorAll("iframe")).toHaveLength(1);
  expect(host.querySelector("iframe")).toBe(latest);
  expect(host.querySelector("iframe")!.srcdoc).toContain("r3");
  expect(document.activeElement).toBe(outside);
  expect(retire).not.toHaveBeenCalled();
  await act(async () => abandoned.dispatchEvent(frameLoad));
  expect(host.querySelector("iframe")).toBe(latest);
  expect(document.activeElement).toBe(outside);
  expect(retire).not.toHaveBeenCalled();
});

it("submits choices with the exact widget key and revision, disables pending input, and retries inline", async () => {
  // The project's ES2022 library has no Promise.withResolvers.
  let finish!: (value: { at_ms: number }) => void;
  const pending = new Promise<{ at_ms: number }>((resolve) => { finish = resolve; });
  const select = vi.fn().mockRejectedValueOnce(new Error("Owner unavailable")).mockReturnValueOnce(pending);
  const client = { widgetSelect: select } as unknown as CockpitClient;
  await act(async () => root.render(<WidgetChoices client={client} widget={choiceWidget} spec={choices} inputBlocked={false} />));
  expect(host.querySelector("iframe")).toBeNull();
  const latency = host.querySelector<HTMLInputElement>('input[value="latency"]')!;
  await act(async () => latency.click());
  const request = { key: widget.key, revision: 3, value: { type: "choice", choice_id: "latency" } };
  expect(select).toHaveBeenCalledWith(request);
  expect(host.querySelector('[role="alert"]')!.textContent).toContain("Owner unavailable");
  expect(latency.checked).toBe(false);
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(select).toHaveBeenLastCalledWith(request);
  expect(host.querySelector('[role="radiogroup"]')!.getAttribute("aria-busy")).toBe("true");
  expect(host.querySelector("fieldset")!.disabled).toBe(true);
  await act(async () => finish({ at_ms: 3000 }));
  expect(latency.checked).toBe(true);
  expect(host.querySelector("fieldset")!.disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("does not let an older pending choice response select a replaced widget", async () => {
  // The project's ES2022 library has no Promise.withResolvers.
  let finish!: (value: { at_ms: number }) => void;
  const pending = new Promise<{ at_ms: number }>((resolve) => { finish = resolve; });
  const select = vi.fn().mockReturnValueOnce(pending).mockResolvedValue({ at_ms: 4000 });
  const client = { widgetSelect: select } as unknown as CockpitClient;
  await act(async () => root.render(<WidgetChoices client={client} widget={choiceWidget} spec={choices} inputBlocked={false} />));
  await act(async () => host.querySelector<HTMLInputElement>('input[value="latency"]')!.click());
  await act(async () => root.render(<WidgetChoices client={client} widget={{ ...choiceWidget, revision: 4 }} spec={choices} inputBlocked={false} />));
  await act(async () => finish({ at_ms: 3000 }));
  expect(host.querySelector<HTMLInputElement>('input[value="latency"]')!.checked).toBe(false);
  await act(async () => host.querySelector<HTMLInputElement>('input[value="errors"]')!.click());
  expect(select).toHaveBeenLastCalledWith({ key: widget.key, revision: 4, value: { type: "choice", choice_id: "errors" } });
});

it("blocks choices during input suspension or a pane drag", async () => {
  const select = vi.fn();
  const client = { widgetSelect: select } as unknown as CockpitClient;
  await act(async () => root.render(<WidgetChoices client={client} widget={choiceWidget} spec={choices} inputBlocked />));
  await act(async () => host.querySelector<HTMLInputElement>("input")!.click());
  expect(select).not.toHaveBeenCalled();
  await act(async () => root.render(<WidgetChoices client={client} widget={choiceWidget} spec={choices} inputBlocked={false} />));
  document.body.classList.add("is-pane-dragging");
  await act(async () => host.querySelector<HTMLInputElement>("input")!.click());
  expect(select).not.toHaveBeenCalled();
});

it("opens nonmodally and restores chip focus on Escape", async () => {
  const go = vi.fn();
  await act(async () => root.render(<WidgetChip widget={widget} width={500} live onGoToAgent={go} />));
  const trigger = host.querySelector<HTMLButtonElement>("button")!;
  await act(async () => trigger.click());
  const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
  expect(dialog.getAttribute("aria-modal")).toBe("false");
  expect(document.activeElement).toBe(dialog);
  await act(async () => dialog.querySelector<HTMLButtonElement>("button")!.click());
  expect(go).toHaveBeenCalledOnce();
  await act(async () => dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(host.querySelector('[role="dialog"]')).toBeNull();
  expect(document.activeElement).toBe(trigger);
});

it.each(["closed", "restarted", "unknown", null] as const)("does not expose agent focus for a %s source", async (status) => {
  const source = status === null ? null : { ...widget.source!, status };
  const go = vi.fn();
  await act(async () => root.render(<WidgetChip widget={{ ...widget, source }} width={500} live onGoToAgent={go} />));
  const trigger = host.querySelector<HTMLButtonElement>("button")!;
  if (status === "closed") expect(trigger.textContent).toContain("pane closed");
  if (status === "restarted") expect(trigger.textContent).toContain("omp restarted");
  if (status === null) expect(trigger.textContent).toContain("CLI");
  await act(async () => trigger.click());
  expect(host.querySelector('[role="dialog"] button')).toBeNull();
  expect(go).not.toHaveBeenCalled();
});

it("explains disconnected agent focus and exposes selection provenance without claiming isolation", async () => {
  const selected: WidgetSummary = { ...widget, arrival: "cross_source", source: { ...widget.source!, space_id: "docs", tab_id: "other" },
    selection: { revision: 2, at_ms: 3000, read_at_ms: 4000 } };
  const go = vi.fn();
  await act(async () => root.render(<WidgetChip widget={selected} width={300} live={false} onGoToAgent={go} />));
  const trigger = host.querySelector<HTMLButtonElement>("button")!;
  expect(trigger.getAttribute("aria-label")).toContain("omp");
  await act(async () => trigger.click());
  const dialog = host.querySelector<HTMLElement>('[role="dialog"]')!;
  const button = dialog.querySelector<HTMLButtonElement>("button")!;
  expect(button.disabled).toBe(true);
  expect(document.getElementById(button.getAttribute("aria-describedby")!)!.textContent).toContain("Herdr isn't live");
  const selection = [...dialog.querySelectorAll("dt")].find((element) => element.textContent === "Selection")!.nextElementSibling!;
  expect(selection.textContent).toContain("revision 2");
  expect(selection.textContent).toContain("read via CLI");
  expect(dialog.textContent).toContain("Published from Space docs · Tab other");
  expect(dialog.textContent).toContain(widget.content.sha256);
  expect(dialog.textContent).not.toMatch(/network.less|network isolation|blocks network/i);
});

it("renders agent choice text as text rather than an agent page", async () => {
  const client = { widgetSelect: vi.fn() } as unknown as CockpitClient;
  await act(async () => root.render(<><WidgetChoices client={client} widget={choiceWidget} inputBlocked={false}
    spec={{ prompt: "<script>run()</script>", choices: [{ id: "safe", label: "<img src=x onerror=run()>", detail: null }] }} />
    <WidgetChip widget={choiceWidget} width={500} live onGoToAgent={() => {}} /></>));
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(host.querySelectorAll("iframe,script,img")).toHaveLength(0);
  expect(host.querySelector("legend")!.textContent).toBe("<script>run()</script>");
});

it("uses roving tab navigation and retains unseen markers until the owner clears them", async () => {
  const second = { ...widget, key: { ...widget.key, id: "errors" }, title: "Errors" };
  const third = { ...widget, key: { ...widget.key, id: "latency" }, title: "Latency" };
  const onCurrent = vi.fn();
  function View() {
    const [current, setCurrent] = useState(widget.key.id);
    return <WidgetTabs widgets={[widget, second, third]} currentId={current} width={500}
      unseen={new Set([JSON.stringify([second.key.session_id, second.key.tab_id, second.key.id])])}
      onCurrent={(value) => { onCurrent(value); setCurrent(value); }} />;
  }
  await act(async () => root.render(<View />));
  const tabs = host.querySelectorAll<HTMLButtonElement>('[role="tab"]');
  expect([...tabs].map((tab) => tab.tabIndex)).toEqual([0, -1, -1]);
  tabs[0].focus();
  await act(async () => tabs[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true, cancelable: true })));
  expect(onCurrent).toHaveBeenLastCalledWith("latency");
  expect(document.activeElement).toBe(tabs[2]);
  await act(async () => tabs[2].dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true, cancelable: true })));
  expect(document.activeElement).toBe(tabs[0]);
  expect(tabs[1].textContent).toContain("updated");
});

it.each([{ count: 2, width: 420 }, { count: 5, width: 700 }])("uses a keyboard menu for $count widgets at width $width", async ({ count, width }) => {
  const widgets = Array.from({ length: count }, (_, index) => ({ ...widget, key: { ...widget.key, id: `widget-${index}` }, title: `Widget ${index}` }));
  const onCurrent = vi.fn();
  await act(async () => root.render(<WidgetTabs widgets={widgets} currentId="widget-0" unseen={new Set()} width={width} onCurrent={onCurrent} />));
  expect(host.querySelector('[role="tablist"]')).toBeNull();
  const trigger = host.querySelector<HTMLButtonElement>("button")!;
  await act(async () => trigger.click());
  const items = host.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]');
  expect(document.activeElement).toBe(items[0]);
  await act(async () => items[0].dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true, cancelable: true })));
  expect(document.activeElement).toBe(items[count - 1]);
  await act(async () => items[count - 1].click());
  expect(onCurrent).toHaveBeenCalledWith(`widget-${count - 1}`);
  expect(host.querySelector('[role="menu"]')).toBeNull();
  expect(document.activeElement).toBe(trigger);
});

it("keeps menu keyboard focus and roving tabindex when another widget arrives", async () => {
  const widgets = Array.from({ length: 2 }, (_, index) => ({ ...widget, key: { ...widget.key, id: `widget-${index}` }, title: `Widget ${index}` }));
  const render = (values: WidgetSummary[], currentId: string) => root.render(<WidgetTabs widgets={values} currentId={currentId} unseen={new Set()} width={420} onCurrent={() => {}} />);
  await act(async () => render(widgets, "widget-0"));
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  const second = host.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')[1];
  await act(async () => host.querySelector<HTMLButtonElement>('[role="menuitemradio"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true, cancelable: true })));
  expect(document.activeElement).toBe(second);
  expect(second.tabIndex).toBe(0);
  await act(async () => render([...widgets, { ...widget, key: { ...widget.key, id: "new" }, title: "New" }], "new"));
  expect(document.activeElement).toBe(second);
  expect(second.tabIndex).toBe(0);
  expect(host.querySelectorAll('[role="menuitemradio"][tabindex="0"]')).toHaveLength(1);
});

it("accepts only the live source, nonce and revision, with a 16 KiB UTF-8 bound and four selections per second", async () => {
  const select = vi.fn();
  const clock = vi.spyOn(performance, "now").mockReturnValue(1000);
  try {
    await act(async () => root.render(<WidgetFrame document="<p>pick</p>" revision={1} currentRevision={1}
      widgetKey="stats" agent="omp" inputBlocked={false} onSelect={select} onFocusRetired={() => {}} />));
    const frame = host.querySelector<HTMLIFrameElement>("iframe")!;
    const identity = frameBridge(frame);
    await act(async () => {
      window.dispatchEvent(new MessageEvent("message", { source: window, data: {
        ...identity, type: "cockpit.widget.select", value_json: '"wrong source"',
      } }));
      frameMessage(frame, { type: "cockpit.widget.select", nonce: "wrong", value_json: "1" });
      frameMessage(frame, { type: "cockpit.widget.select", revision: 2, value_json: "1" });
      frameMessage(frame, { type: "cockpit.widget.select", value_json: JSON.stringify("é".repeat(8192)) });
      frameMessage(frame, { type: "cockpit.widget.select", value_json: "x".repeat(16385) });
    });
    expect(select).not.toHaveBeenCalled();
    await act(async () => {
      for (let value = 0; value < 5; value++) frameMessage(frame, { type: "cockpit.widget.select", value_json: JSON.stringify({ value }) });
    });
    expect(select.mock.calls).toEqual([[JSON.stringify({ value: 0 })], [JSON.stringify({ value: 1 })],
      [JSON.stringify({ value: 2 })], [JSON.stringify({ value: 3 })]]);
    clock.mockReturnValue(2001);
    await act(async () => frameMessage(frame, { type: "cockpit.widget.select", value_json: "null" }));
    expect(select).toHaveBeenLastCalledWith("null");
  } finally { clock.mockRestore(); }
});

it("queues only the newest pending selection and discards superseded frames before promotion", async () => {
  const select = vi.fn();
  const render = async (revision: number, currentRevision = revision) => {
    await act(async () => root.render(<WidgetFrame document={`<p>r${revision}</p>`} revision={revision}
      currentRevision={currentRevision} widgetKey="stats" agent="omp" inputBlocked={false}
      onSelect={select} onFocusRetired={() => {}} />));
  };
  await render(1);
  const original = host.querySelector<HTMLIFrameElement>("iframe")!;
  await render(1, 2);
  await act(async () => frameMessage(original, { type: "cockpit.widget.select", value_json: '"old during fetch"' }));
  await render(2);
  const abandoned = host.querySelector<HTMLIFrameElement>("iframe[data-pending]")!;
  await act(async () => frameMessage(abandoned, { type: "cockpit.widget.select", value_json: '"abandoned"' }));
  expect(select).not.toHaveBeenCalled();
  await render(3);
  const pending = host.querySelector<HTMLIFrameElement>("iframe[data-pending]")!;
  await act(async () => {
    frameMessage(original, { type: "cockpit.widget.select", value_json: '"old"' });
    frameMessage(abandoned, { type: "cockpit.widget.select", value_json: '"retired pending"' });
    frameMessage(pending, { type: "cockpit.widget.select", value_json: '"first pending"' });
    frameMessage(pending, { type: "cockpit.widget.select", value_json: '"latest pending"' });
  });
  expect(select).not.toHaveBeenCalled();
  await act(async () => pending.dispatchEvent(frameLoad));
  expect(select).toHaveBeenCalledExactlyOnceWith('"latest pending"');
  await act(async () => {
    abandoned.dispatchEvent(frameLoad);
    frameMessage(abandoned, { type: "cockpit.widget.select", value_json: '"late retired"' });
  });
  expect(select).toHaveBeenCalledTimes(1);
});

it("keeps valid selected null distinct from no selection without reloading a running page", async () => {
  const render = async (revision: number, selection: unknown, hasSelection: boolean) => {
    await act(async () => root.render(<WidgetFrame document={`<p>r${revision}</p>`} revision={revision}
      currentRevision={revision} widgetKey="stats" selection={selection} hasSelection={hasSelection}
      agent="omp" inputBlocked={false} onSelect={() => {}} onFocusRetired={() => {}} />));
  };
  await render(1, null, true);
  const initial = host.querySelector<HTMLIFrameElement>("iframe")!;
  expect(frameBridge(initial)).toMatchObject({ selection: null, hasSelection: true });
  const originalSrc = initial.srcdoc;
  await render(1, { answer: 42 }, true);
  expect(host.querySelector("iframe")).toBe(initial);
  expect(initial.srcdoc).toBe(originalSrc);
  await render(2, { answer: 42 }, true);
  const pending = host.querySelector<HTMLIFrameElement>("iframe[data-pending]")!;
  expect(frameBridge(pending)).toMatchObject({ selection: { answer: 42 }, hasSelection: true });
});

it("blocks suspended selections and routes only focused vetted shortcuts without injecting keydown", async () => {
  const select = vi.fn();
  const keyboard = vi.fn();
  const shortcuts = vi.fn();
  window.addEventListener("keydown", keyboard);
  window.addEventListener("cockpit-widget-shortcut", shortcuts);
  const render = async (inputBlocked: boolean) => {
    await act(async () => root.render(<WidgetFrame document="<p>keys</p>" revision={1} currentRevision={1}
      widgetKey="stats" agent="omp" inputBlocked={inputBlocked} onSelect={select} onFocusRetired={() => {}} />));
  };
  try {
    await render(true);
    const frame = host.querySelector<HTMLIFrameElement>("iframe")!;
    const chord = { type: "cockpit.widget.shortcut", key: "b", code: "KeyB", ctrlKey: true,
      altKey: false, shiftKey: false, metaKey: false, repeat: false };
    await act(async () => {
      frameMessage(frame, { type: "cockpit.widget.select", value_json: "1" });
      frameMessage(frame, chord);
    });
    expect(select).not.toHaveBeenCalled();
    expect(shortcuts).not.toHaveBeenCalled();
    await render(false);
    await act(async () => frameMessage(frame, chord));
    expect(shortcuts).not.toHaveBeenCalled();
    await act(async () => {
      frameMessage(frame, { type: "cockpit.widget.intent" });
      frame.focus();
      frameMessage(frame, { ...chord, key: "arbitrary string" });
      frameMessage(frame, { ...chord, code: "NotAKey" });
      frameMessage(frame, chord);
      frameMessage(frame, { ...chord, key: "Tab", code: "Tab", ctrlKey: false });
      frameMessage(frame, { ...chord, key: "v", code: "KeyV", ctrlKey: false });
      frameMessage(frame, { ...chord, key: "å", code: "KeyA", ctrlKey: false, altKey: true });
    });
    expect(shortcuts).toHaveBeenCalledTimes(3);
    expect(keyboard).not.toHaveBeenCalled();
    document.body.classList.add("is-pane-dragging");
    await act(async () => frameMessage(frame, { type: "cockpit.widget.select", value_json: "2" }));
    expect(select).not.toHaveBeenCalled();
  } finally {
    window.removeEventListener("keydown", keyboard);
    window.removeEventListener("cockpit-widget-shortcut", shortcuts);
  }
});

it("restores outside focus after script-like iframe focus but preserves recent user intent", async () => {
  await act(async () => root.render(<><button>Dialog control</button><WidgetFrame document="<input autofocus>" revision={1}
    currentRevision={1} widgetKey="stats" agent="omp" inputBlocked={false} onSelect={() => {}} onFocusRetired={() => {}} /></>));
  const outside = host.querySelector("button")!;
  const frame = host.querySelector<HTMLIFrameElement>("iframe")!;
  vi.useFakeTimers();
  try {
    outside.focus();
    await act(async () => { frame.focus(); await vi.advanceTimersByTimeAsync(50); });
    expect(document.activeElement).toBe(outside);
    await act(async () => {
      frameMessage(frame, { type: "cockpit.widget.intent" });
      frame.focus();
      await vi.advanceTimersByTimeAsync(50);
    });
    expect(document.activeElement).toBe(frame);
  } finally { vi.useRealTimers(); }
});

it("acknowledges configured host prefixes only to the focused live frame and accepts its plain follower", async () => {
  const shortcuts = vi.fn();
  window.addEventListener("cockpit-widget-shortcut", shortcuts);
  await act(async () => root.render(<WidgetFrame document="<p>prefix</p>" revision={1} currentRevision={1}
    widgetKey="stats" agent="omp" inputBlocked={false} onSelect={() => {}} onFocusRetired={() => {}} />));
  const frame = host.querySelector<HTMLIFrameElement>("iframe")!;
  const post = vi.spyOn(frame.contentWindow!, "postMessage").mockImplementation(() => {});
  const chord = { type: "cockpit.widget.shortcut", key: "f", code: "KeyF",
    ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, repeat: false };
  try {
    await act(async () => {
      window.dispatchEvent(new CustomEvent("cockpit-widget-prefix", { detail: { target: frame, active: true } }));
      frameMessage(frame, { type: "cockpit.widget.intent" });
      frame.focus();
      frameMessage(frame, chord);
    });
    expect(post).not.toHaveBeenCalled();
    expect(shortcuts).not.toHaveBeenCalled();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("cockpit-widget-prefix", { detail: { target: window, active: true } }));
      window.dispatchEvent(new CustomEvent("cockpit-widget-prefix", { detail: { target: frame, active: "yes" } }));
      window.dispatchEvent(new CustomEvent("cockpit-widget-prefix", { detail: { target: frame, active: true } }));
      frameMessage(frame, chord);
    });
    expect(post).toHaveBeenCalledExactlyOnceWith({ type: "cockpit.widget.prefix", nonce: frameBridge(frame).nonce, revision: 1, active: true }, "*");
    expect(shortcuts).toHaveBeenCalledOnce();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("cockpit-widget-prefix", { detail: { target: frame, active: false } }));
      frameMessage(frame, chord);
    });
    expect(shortcuts).toHaveBeenCalledOnce();
  } finally { post.mockRestore(); window.removeEventListener("cockpit-widget-shortcut", shortcuts); }
});

it("lets only nonce-bound parent prefix acknowledgements arm the author document's plain follower", () => {
  const listeners = new Map<string, (event: Record<string, unknown>) => void>();
  const page = { addEventListener: (type: string, listener: (event: Record<string, unknown>) => void) => listeners.set(type, listener) };
  const parentFrame = { postMessage: vi.fn() };
  const html = widgetDocument("<p>author</p>", { nonce: "frame-nonce", revision: 5, selection: null, hasSelection: true });
  const script = new DOMParser().parseFromString(html, "text/html").querySelector("script")!.textContent!;
  new Function("window", "parent", "TextEncoder", script)(page, parentFrame, TextEncoder);
  const key = { isTrusted: true, isComposing: false, key: "f", code: "KeyF", ctrlKey: false,
    altKey: false, shiftKey: false, metaKey: false, repeat: false, preventDefault: vi.fn(), stopImmediatePropagation: vi.fn() };
  const acknowledgement = { type: "cockpit.widget.prefix", nonce: "frame-nonce", revision: 5, active: true };
  const message = listeners.get("message")!;
  const keydown = listeners.get("keydown")!;
  message({ source: {}, data: acknowledgement });
  message({ source: parentFrame, data: { ...acknowledgement, nonce: "stale" } });
  message({ source: parentFrame, data: { ...acknowledgement, revision: 4 } });
  message({ source: parentFrame, data: { ...acknowledgement, active: "yes" } });
  keydown(key);
  expect(parentFrame.postMessage.mock.calls.filter(([value]) => value.type === "cockpit.widget.shortcut")).toHaveLength(0);
  message({ source: parentFrame, data: acknowledgement });
  keydown(key);
  expect(parentFrame.postMessage.mock.calls.filter(([value]) => value.type === "cockpit.widget.shortcut")).toHaveLength(1);
  expect(key.preventDefault).toHaveBeenCalledOnce();
  message({ source: parentFrame, data: { ...acknowledgement, active: false } });
  keydown(key);
  expect(parentFrame.postMessage.mock.calls.filter(([value]) => value.type === "cockpit.widget.shortcut")).toHaveLength(1);
});

it("notifies the host when a literal second Ctrl+B disarms the prefix without consuming author input", () => {
  const listeners = new Map<string, (event: Record<string, unknown>) => void>();
  const page = { addEventListener: (type: string, listener: (event: Record<string, unknown>) => void) => listeners.set(type, listener) };
  const parentFrame = { postMessage: vi.fn() };
  const html = widgetDocument("<p>author</p>", { nonce: "literal-prefix", revision: 1, selection: null });
  const script = new DOMParser().parseFromString(html, "text/html").querySelector("script")!.textContent!;
  new Function("window", "parent", "TextEncoder", script)(page, parentFrame, TextEncoder);
  listeners.get("message")!({ source: parentFrame, data: {
    type: "cockpit.widget.prefix", nonce: "literal-prefix", revision: 1, active: true,
  } });
  const literal = { isTrusted: true, isComposing: false, key: "b", code: "KeyT", ctrlKey: true,
    altKey: false, shiftKey: false, metaKey: false, repeat: false, preventDefault: vi.fn(), stopImmediatePropagation: vi.fn() };
  listeners.get("keydown")!(literal);
  expect(parentFrame.postMessage.mock.calls.filter(([value]) => value.type === "cockpit.widget.shortcut")).toHaveLength(1);
  expect(literal.preventDefault).not.toHaveBeenCalled();
  expect(literal.stopImmediatePropagation).not.toHaveBeenCalled();
});

it("admits a parked-pointer click before delayed child intent and keeps that focus session after 500 ms", async () => {
  const userFocus = vi.fn();
  await act(async () => root.render(<><button>Outside</button><WidgetFrame document="<input>" revision={1}
    currentRevision={1} widgetKey="stats" agent="omp" inputBlocked={false} onSelect={() => {}}
    onFocusRetired={() => {}} onUserFocus={userFocus} /></>));
  const outside = host.querySelector("button")!;
  const frame = host.querySelector<HTMLIFrameElement>("iframe")!;
  const hover = vi.spyOn(frame, "matches").mockImplementation(selector => selector === ":hover");
  vi.useFakeTimers();
  try {
    outside.focus();
    await act(async () => vi.advanceTimersByTimeAsync(600));
    await act(async () => { frame.focus(); await vi.advanceTimersByTimeAsync(40); });
    expect(document.activeElement).toBe(frame);
    expect(userFocus).toHaveBeenCalledOnce();
    await act(async () => frameMessage(frame, { type: "cockpit.widget.intent" }));
    hover.mockReturnValue(false);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
      window.dispatchEvent(new Event("blur"));
      await vi.advanceTimersByTimeAsync(40);
    });
    expect(document.activeElement).toBe(frame);
    expect(userFocus).toHaveBeenCalledOnce();
  } finally { hover.mockRestore(); vi.useRealTimers(); }
});

it("allows a separate child trusted-input task to arrive after focus without accepting a forged intent", async () => {
  const userFocus = vi.fn();
  await act(async () => root.render(<><button>Outside</button><WidgetFrame document="<input>" revision={1}
    currentRevision={1} widgetKey="stats" agent="omp" inputBlocked={false} onSelect={() => {}}
    onFocusRetired={() => {}} onUserFocus={userFocus} /></>));
  const outside = host.querySelector("button")!;
  const frame = host.querySelector<HTMLIFrameElement>("iframe")!;
  const hover = vi.spyOn(frame, "matches").mockReturnValue(false);
  vi.useFakeTimers();
  try {
    outside.focus();
    await act(async () => { frame.focus(); await vi.advanceTimersByTimeAsync(10); });
    await act(async () => {
      frameMessage(frame, { type: "cockpit.widget.intent" });
      await vi.advanceTimersByTimeAsync(40);
    });
    expect(document.activeElement).toBe(frame);
    expect(userFocus).toHaveBeenCalledOnce();
    outside.focus();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600);
      frame.focus();
      frameMessage(frame, { type: "cockpit.widget.intent", nonce: "forged" });
      await vi.advanceTimersByTimeAsync(40);
    });
    expect(document.activeElement).toBe(outside);
    expect(userFocus).toHaveBeenCalledOnce();
  } finally { hover.mockRestore(); vi.useRealTimers(); }
});
