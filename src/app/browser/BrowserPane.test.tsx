// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BrowserDraftRecoveryRequest, BrowserViewCommandOutcome, BrowserViewCommandRequest, BrowserViewCommandResponse, BrowserViewDraftState, BrowserViewEvent, BrowserViewFrameDescriptor, BrowserViewSnapshot, BrowserViewViewportState } from "../../protocol/generated/v1";
import type { BrowserViewFramePacket, CockpitClient } from "../../client/CockpitClient";
import { BrowserPane } from "./BrowserPane";

const jpeg = Uint8Array.from([0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x03, 0x00, 0x04, 0x01, 0x01, 0xff, 0xd9]);

function viewport(revision: number, scrollY: number): BrowserViewViewportState {
  return { viewport_revision: revision, css_width: 800, css_height: 600, visual_offset_x: 0, visual_offset_y: 0, scroll_x: 0, scroll_y: scrollY, visual_scale: 1, page_scale: 1, device_pixel_ratio: 1, geometry_fresh: true };
}

function descriptor(revision: number, sequence: number, scrollY: number): BrowserViewFrameDescriptor {
  return { target_id: "target", stream_epoch: 1, frame_sequence: sequence, document_generation: 1, viewport_revision: revision, image_width: 4, image_height: 3, viewport_css_width: 800, viewport_css_height: 600, viewport_offset_x: 0, viewport_offset_y: 0, scroll_x: 0, scroll_y: scrollY, capture_timestamp_micros: sequence, jpeg_length: jpeg.byteLength };
}

function snapshot(): BrowserViewSnapshot {
  return {
    identity: { association_key: "association", browser_incarnation: "incarnation", view_id: "view", stream_epoch: 1 }, metadata_sequence: 1,
    targets: [{ target_id: "target", kind: "page", title: "Fixture", url: "https://example.test", order: 0, opener_target_id: null, can_close: false }],
    displayed_target_id: "target", document: { target_id: "target", frame_id: "frame", document_generation: 1, frame_generation: 1 }, viewport: viewport(1, 0),
    navigation: { url: "https://example.test", title: "Fixture", loading: false, can_go_back: false, can_go_forward: false, requested_url: null }, cursor: null,
    focus: { page_focused: true, editable: false, selection_available: false, composition_active: false }, blocker: null,
    capabilities: { pointer_input: "supported", keyboard_input: "supported", text_input: "supported", composition_input: "supported", clipboard_read: "supported", clipboard_write: "supported", dialogs: "unsupported", file_chooser: "unsupported", downloads: "unsupported", permissions: "unsupported", inspection: "supported", capture: "supported", drafts: "supported", audio: "unsupported" },
    control: { status: "controlled", controller_view_id: "view", lease_generation: 4, next_input_sequence: 1, can_take_control: false }, frame_grant: null,
  };
}

function packet(frame: BrowserViewFrameDescriptor): BrowserViewFramePacket {
  return { descriptor: frame, jpeg: jpeg.slice().buffer, ack: vi.fn(), discard: vi.fn() };
}

const accepted = (request: BrowserViewCommandRequest, outcome: BrowserViewCommandOutcome = { type: "none" }): BrowserViewCommandResponse => ({ status: "accepted", view_id: "view", stream_epoch: 1, request_id: request.request_id, outcome });

describe("BrowserPane wheel recovery", () => {
  let root: Root | null = null;
  let host: HTMLDivElement | null = null;
  let originalDpr: PropertyDescriptor | undefined;

  afterEach(async () => {
    if (root) await act(async () => root?.unmount());
    root = null;
    host?.remove();
    host = null;
    vi.restoreAllMocks();
    if (originalDpr) Object.defineProperty(window, "devicePixelRatio", originalDpr);
    originalDpr = undefined;
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("keeps repeated wheel input flowing while scroll frames catch up", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    let emitEvent!: (event: BrowserViewEvent) => void;
    let emitFrame!: (frame: BrowserViewFramePacket) => void;
    const commands: BrowserViewCommandRequest[] = [];
    let openedDpr = 0;
    const client = {
      openBrowserView: vi.fn(async (request, onEvent, onFrame) => {
        openedDpr = request.viewport.device_pixel_ratio;
        emitEvent = onEvent;
        emitFrame = onFrame;
        onEvent({ type: "attached", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 1 }, snapshot: snapshot() });
        return { command: async (request: BrowserViewCommandRequest) => { commands.push(request); return accepted(request); }, close: vi.fn() };
      }),
    } as unknown as CockpitClient;
    originalDpr = Object.getOwnPropertyDescriptor(window, "devicePixelRatio");
    Object.defineProperty(window, "devicePixelRatio", { configurable: true, value: 1.25 });
    vi.stubGlobal("createImageBitmap", vi.fn(async () => ({ width: 4, height: 3, close: vi.fn() })));
    const drawImage = vi.fn();
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), drawImage } as unknown as CanvasRenderingContext2D);
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={{ session_id: "session", space_id: "space", pane_id: "pane", endpoint_path: null }} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(openedDpr).toBe(1.25);
    await act(async () => { await new Promise<void>((resolve) => setTimeout(resolve, 0)); });
    const surface = host.querySelector<HTMLDivElement>(".browser-surface")!;
    vi.spyOn(surface, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, toJSON: () => ({}) });
    const firstFrame = packet(descriptor(1, 1, 0));
    await act(async () => {
      emitFrame(firstFrame);
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
    });
    expect(firstFrame.ack).toHaveBeenCalledOnce();
    const canvas = host.querySelector<HTMLCanvasElement>("canvas.browser-frame")!;
    expect([canvas.width, canvas.height]).toEqual([4, 3]);

    await act(async () => {
      emitEvent({ type: "viewport_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 2 }, viewport: viewport(2, 120) });
    });
    const queuedWheels = [
      new WheelEvent("wheel", { deltaY: 120, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }),
      new WheelEvent("wheel", { deltaY: 80, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }),
      new WheelEvent("wheel", { deltaY: 20, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }),
    ];
    for (const wheel of queuedWheels) {
      const preventDefault = vi.spyOn(wheel, "preventDefault");
      await act(async () => { surface.dispatchEvent(wheel); });
      expect(preventDefault).toHaveBeenCalledOnce();
    }
    await act(async () => {
      surface.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true, cancelable: true }));
      surface.dispatchEvent(new KeyboardEvent("keyup", { key: "Enter", code: "Enter", bubbles: true, cancelable: true }));
    });
    await act(async () => { await new Promise<void>((resolve) => setTimeout(resolve, 0)); });
    const wheelsBeforeFreshFrame = commands.filter(({ command }) => command.type === "wheel");
    expect(wheelsBeforeFreshFrame.map(({ command }) => command.type === "wheel" ? [command.location.viewport_revision, command.input.delta_y_css, command.input.input_sequence] : null)).toEqual([[2, 120, 1], [2, 80, 2], [2, 20, 3]]);

    await act(async () => {
      emitEvent({ type: "viewport_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 3 }, viewport: viewport(3, 180) });
      const laggedFrame = packet(descriptor(2, 2, 120));
      emitFrame(laggedFrame);
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
      expect(laggedFrame.ack).toHaveBeenCalledOnce();
      expect(drawImage).toHaveBeenCalledTimes(2);
      const nextFrame = packet(descriptor(3, 3, 180));
      emitFrame(nextFrame);
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
      expect(nextFrame.ack).toHaveBeenCalledOnce();
    });
    const wheels = commands.filter(({ command }) => command.type === "wheel");
    expect(wheels.map(({ command }) => command.type === "wheel" ? [command.location.viewport_revision, command.input.delta_y_css, command.input.x, command.input.y, command.input.input_sequence] : null)).toEqual([[2, 120, 400, 300, 1], [2, 80, 400, 300, 2], [2, 20, 400, 300, 3]]);
    expect(commands.filter(({ command }) => command.type === "keyboard").map(({ command }) => command.type === "keyboard" ? [command.input.kind, command.input.key, command.input.input_sequence] : null)).toEqual([["down", "Enter", 4], ["up", "Enter", 5]]);
    expect(commands.filter(({ command }) => command.type === "wheel" || command.type === "keyboard").map(({ command }) => command.type)).toEqual(["wheel", "wheel", "wheel", "keyboard", "keyboard"]);
  });

  it("takes control before switching tabs from an observing browser view", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    const commands: BrowserViewCommandRequest[] = [];
    const observed = snapshot();
    observed.control = { status: "observing", controller_view_id: null, lease_generation: 4, next_input_sequence: 1, can_take_control: true };
    const client = {
      openBrowserView: vi.fn(async (_request, onEvent) => {
        onEvent({ type: "attached", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 1 }, snapshot: observed });
        return {
          command: async (request: BrowserViewCommandRequest) => {
            commands.push(request);
            if (request.command.type === "take_control") {
              observed.control = { status: "controlled", controller_view_id: "view", lease_generation: 5, next_input_sequence: 1, can_take_control: false };
              return accepted(request, { type: "control", control: observed.control });
            }
            if (request.command.type === "tab") {
              if (observed.control.status !== "controlled") return {
                status: "rejected" as const, view_id: "view", stream_epoch: 1, request_id: request.request_id,
                code: "browser_control_required", message: "Another browser view holds the input lease",
              };
              const second = {
                ...observed, metadata_sequence: 2, displayed_target_id: "second",
                targets: [...observed.targets, { target_id: "second", kind: "page" as const, title: "Second fixture", url: "https://example.test/second", order: 1, opener_target_id: null, can_close: true }],
                navigation: { ...observed.navigation!, url: "https://example.test/second", title: "Second fixture" },
              };
              return accepted(request, { type: "snapshot", snapshot: second });
            }
            return accepted(request);
          },
          close: vi.fn(),
        };
      }),
    } as unknown as CockpitClient;
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn() } as unknown as CanvasRenderingContext2D);
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={{ session_id: "session", space_id: "space", pane_id: "pane", endpoint_path: null }} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    await act(async () => {
      host!.querySelector<HTMLButtonElement>(".browser-new-tab")!.click();
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(commands.filter(({ command }) => command.type === "take_control" || command.type === "tab")
      .map(({ command }) => command.type)).toEqual(["take_control", "tab"]);
    expect(host.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')?.textContent).toBe("Second fixture");
    expect(host.querySelector<HTMLInputElement>('input[aria-label="Page URL"]')?.value).toBe("https://example.test/second");
    expect(host.textContent).not.toContain("Input rejected: Another browser view holds the input lease");
  });

  it("does not let a delayed scroll repaint block later input and navigation", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    let emitEvent!: (event: BrowserViewEvent) => void;
    let emitFrame!: (frame: BrowserViewFramePacket) => void;
    const commands: BrowserViewCommandRequest[] = [];
    const client = {
      openBrowserView: vi.fn(async (_request, onEvent, onFrame) => {
        emitEvent = onEvent;
        emitFrame = onFrame;
        onEvent({ type: "attached", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 1 }, snapshot: snapshot() });
        return { command: async (request: BrowserViewCommandRequest) => { commands.push(request); return accepted(request); }, close: vi.fn() };
      }),
    } as unknown as CockpitClient;
    vi.stubGlobal("createImageBitmap", vi.fn(async () => ({ width: 4, height: 3, close: vi.fn() })));
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D);
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={{ session_id: "session", space_id: "space", pane_id: "pane", endpoint_path: null }} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    const surface = host.querySelector<HTMLDivElement>(".browser-surface")!;
    vi.spyOn(surface, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, toJSON: () => ({}) });
    await act(async () => {
      emitFrame(packet(descriptor(1, 1, 0)));
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
    });
    await act(async () => {
      emitEvent({ type: "viewport_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 2 }, viewport: viewport(2, 120) });
      surface.dispatchEvent(new WheelEvent("wheel", { deltaY: 120, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
      await Promise.resolve();
      surface.dispatchEvent(new WheelEvent("wheel", { deltaY: 80, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
      surface.dispatchEvent(new WheelEvent("wheel", { deltaY: 20, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
    });
    await act(async () => {
      host!.querySelector<HTMLButtonElement>(".browser-new-tab")!.click();
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    const inputAndNavigation = commands.filter(({ command }) => command.type === "wheel" || command.type === "tab");
    expect(inputAndNavigation.some(({ command }) => command.type === "wheel")).toBe(true);
    expect(inputAndNavigation.at(-1)?.command).toMatchObject({ type: "tab", command: { type: "create" } });
  });
  it("waits for the stream command handle before exposing a painted page as live", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    const commands: BrowserViewCommandRequest[] = [];
    let resolveOpen!: (stream: { command: (request: BrowserViewCommandRequest) => Promise<BrowserViewCommandResponse>; close: () => void }) => void;
    let emitFrame!: (frame: BrowserViewFramePacket) => void;
    const client = {
      openBrowserView: vi.fn((_request, onEvent, onFrame) => {
        emitFrame = onFrame;
        onEvent({ type: "attached", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 1 }, snapshot: snapshot() });
        return new Promise<{ command: (request: BrowserViewCommandRequest) => Promise<BrowserViewCommandResponse>; close: () => void }>((resolve) => { resolveOpen = resolve; });
      }),
    } as unknown as CockpitClient;
    vi.stubGlobal("createImageBitmap", vi.fn(async () => ({ width: 4, height: 3, close: vi.fn() })));
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D);
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={{ session_id: "session", space_id: "space", pane_id: "pane", endpoint_path: null }} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    await act(async () => {
      emitFrame(packet(descriptor(1, 1, 0)));
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
    });
    expect(host.querySelector<HTMLCanvasElement>("canvas.browser-frame")?.width).toBe(4);
    expect(host.querySelector(".browser-toolbar-status")?.textContent).toBe("Loading browser view…");
    await act(async () => {
      resolveOpen({ command: async (request) => { commands.push(request); return accepted(request); }, close: vi.fn() });
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(host.querySelector(".browser-toolbar-status")?.textContent).toBe("Live browser view");
    await act(async () => {
      const input = host!.querySelector<HTMLInputElement>('input[aria-label="Page URL"]')!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "https://example.test/next");
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.closest("form")!.requestSubmit();
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(commands.some(({ command }) => command.type === "navigation" && command.command.type === "navigate"
      && command.command.url === "https://example.test/next")).toBe(true);
  });

  it("opens one stream for the initial association across hydration rerenders and resizes", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    const close = vi.fn();
    const client = {
      openBrowserView: vi.fn(async () => ({ command: vi.fn(async () => accepted({} as BrowserViewCommandRequest)), close })),
    } as unknown as CockpitClient;
    const target = { session_id: "session", space_id: "space", pane_id: "pane", endpoint_path: null };
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={target} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(client.openBrowserView).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();

    await act(async () => {
      root?.render(<BrowserPane client={client} target={target} viewport={{ css_width: 900, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(client.openBrowserView).toHaveBeenCalledTimes(1);
    expect(close).not.toHaveBeenCalled();
  });
  it("restores a retained annotation when control changes during draft inventory", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    let emitEvent!: (event: BrowserViewEvent) => void;
    let emitFrame!: (frame: BrowserViewFramePacket) => void;
    let releaseInventory!: () => void;
    const inventoryGate = new Promise<void>((resolve) => { releaseInventory = resolve; });
    const commands: BrowserViewCommandRequest[] = [];
    const retained: BrowserViewDraftState = {
      draft_id: "retained", target_id: "target", document_generation: 1, revision: 3,
      annotations: [{ id: "persisted-region", kind: "region", color: "#2a9d55", points: [{ x: 10, y: 10 }, { x: 100, y: 90 }], bounds: { x: 10, y: 10, width: 90, height: 80 }, evidence: null, comment: null }],
      freshness: "fresh", stale: false, editor: { selected_annotation_id: null, notes_open: false, note_annotation_id: null, note_text: "" },
    };
    const client = {
      openBrowserView: vi.fn(async (_request, onEvent, onFrame) => {
        emitEvent = onEvent;
        emitFrame = onFrame;
        onEvent({ type: "attached", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 1 }, snapshot: snapshot() });
        return { command: async (request: BrowserViewCommandRequest): Promise<BrowserViewCommandResponse> => {
          commands.push(request);
          if (request.command.type !== "draft") return accepted(request);
          if (request.command.command.type === "list") {
            await inventoryGate;
            return accepted(request, { type: "draft_inventory", inventory: { drafts: [retained], active_draft_limit: 8, pending_capture: null } });
          }
          if (request.command.command.type === "open") return accepted(request, { type: "draft", draft: retained });
          return accepted(request);
        }, close: vi.fn() };
      }),
    } as unknown as CockpitClient;
    vi.stubGlobal("createImageBitmap", vi.fn(async () => ({ width: 4, height: 3, close: vi.fn() })));
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D);
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={{ session_id: "session", space_id: "space", pane_id: "pane", endpoint_path: null }} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(commands.some(({ command }) => command.type === "draft" && command.command.type === "list")).toBe(true);
    await act(async () => {
      emitFrame(packet(descriptor(1, 1, 0)));
      emitEvent({ type: "control_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 2 }, control: { ...snapshot().control, lease_generation: 5 } });
      releaseInventory();
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
    });
    expect(commands.filter(({ command }) => command.type === "draft" && command.command.type === "open").map(({ command }) => command.type === "draft" && command.command.type === "open" ? [command.command.draft_id, command.context.lease_generation] : null)).toEqual([["retained", 5]]);
    expect(host.querySelector("rect.browser-annotation-region")?.getAttribute("stroke")).toBe("#2a9d55");
  });
  it("retires unsubmitted marks on document navigation without a stale notes list", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    let emitEvent!: (event: BrowserViewEvent) => void;
    const oldDraft: BrowserViewDraftState = {
      draft_id: "older-page", target_id: "target", document_generation: 1, revision: 4,
      annotations: [{ id: "unsent-region", kind: "region", color: "#2a9d55", points: [{ x: 10, y: 10 }, { x: 100, y: 90 }], bounds: { x: 10, y: 10, width: 90, height: 80 }, evidence: null, comment: "Unsent navigation note" }],
      freshness: "stale", stale: true, editor: { selected_annotation_id: "unsent-region", notes_open: false, note_annotation_id: "unsent-region", note_text: "Unsent navigation note" },
    };
    const newDraft: BrowserViewDraftState = {
      ...oldDraft, draft_id: "new-page", document_generation: 2, revision: 1, annotations: [],
      freshness: "fresh", stale: false, editor: { selected_annotation_id: null, notes_open: false, note_annotation_id: null, note_text: "" },
    };
    let storedDrafts = [oldDraft, newDraft];
    const recovery = vi.fn(async (request: BrowserDraftRecoveryRequest): Promise<BrowserViewCommandOutcome> => {
      if (request.action.type === "list") return { type: "draft_inventory", inventory: { drafts: storedDrafts, active_draft_limit: 8, pending_capture: null } };
      if (request.action.type === "discard_draft" && request.action.draft_id === oldDraft.draft_id && request.action.expected_revision === oldDraft.revision) {
        storedDrafts = [newDraft];
        return { type: "none" };
      }
      throw new Error(`Unexpected recovery mutation: ${request.action.type}`);
    });
    const client = {
      browserDraftRecovery: recovery,
      openBrowserView: vi.fn(async (_request, onEvent) => {
        emitEvent = onEvent;
        onEvent({ type: "attached", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 1 }, snapshot: snapshot() });
        return { command: async (request: BrowserViewCommandRequest) => {
          if (request.command.type !== "draft") return accepted(request);
          if (request.command.command.type === "list") return accepted(request, { type: "draft_inventory", inventory: { drafts: [oldDraft, newDraft], active_draft_limit: 8, pending_capture: null } });
          return accepted(request, { type: "draft", draft: request.command.context.document_generation === 1 ? oldDraft : newDraft });
        }, close: vi.fn() };
      }),
    } as unknown as CockpitClient;
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D);
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={{ session_id: "session", space_id: "space", pane_id: null, endpoint_path: null }} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(host.querySelector(".browser-annotation-notes")?.getAttribute("aria-label")).toBe("Notes 1");
    await act(async () => {
      emitEvent({ type: "document_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 2 }, document: { target_id: "target", frame_id: "new-frame", document_generation: 2, frame_generation: 2 } });
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
    });
    expect(host.querySelector(".browser-annotation-notes")?.getAttribute("aria-label")).toBe("Notes 0");
    expect(recovery.mock.calls.filter(([request]) => request.action.type === "discard_draft").map(([request]) => request.action.type === "discard_draft" ? [request.action.draft_id, request.action.expected_revision] : null)).toEqual([["older-page", 4]]);
    expect(storedDrafts).toEqual([newDraft]);
    expect(host.querySelector(".browser-annotation")).toBeNull();
    expect(host.querySelector(".browser-retained-drafts")).toBeNull();
  });
  it("keeps unsent notes on another browser tab when switching away and back", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    let emitEvent!: (event: BrowserViewEvent) => void;
    const first: BrowserViewDraftState = {
      draft_id: "first-tab", target_id: "target", document_generation: 1, revision: 3,
      annotations: [{ id: "unsent", kind: "region", color: "#2a9d55", points: [{ x: 10, y: 10 }, { x: 100, y: 90 }], bounds: { x: 10, y: 10, width: 90, height: 80 }, evidence: null, comment: "Keep this note" }],
      freshness: "fresh", stale: false,
      editor: { selected_annotation_id: null, notes_open: false, note_annotation_id: null, note_text: "" },
    };
    const second: BrowserViewDraftState = {
      ...first, draft_id: "second-tab", target_id: "another-target", document_generation: 2, revision: 1,
      annotations: [],
    };
    const targets = [
      ...snapshot().targets,
      { target_id: "another-target", kind: "page" as const, title: "Other tab", url: "https://example.test/other", order: 1, opener_target_id: null, can_close: true },
    ];
    const recovery = vi.fn(async (request: BrowserDraftRecoveryRequest): Promise<BrowserViewCommandOutcome> => {
      if (request.action.type === "list") {
        return { type: "draft_inventory", inventory: { drafts: [first, second], active_draft_limit: 8, pending_capture: null } };
      }
      throw new Error(`An ordinary tab switch cannot ${request.action.type} another tab's draft`);
    });
    const client = {
      browserDraftRecovery: recovery,
      openBrowserView: vi.fn(async (_request, onEvent) => {
        emitEvent = onEvent;
        onEvent({ type: "attached", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 1 }, snapshot: { ...snapshot(), targets } });
        return { command: async (request: BrowserViewCommandRequest) => {
          if (request.command.type !== "draft") return accepted(request);
          if (request.command.command.type === "list") {
            return accepted(request, { type: "draft_inventory", inventory: { drafts: [first, second], active_draft_limit: 8, pending_capture: null } });
          }
          const draft = request.command.context.target_id === "target" ? first : second;
          return accepted(request, { type: "draft", draft });
        }, close: vi.fn() };
      }),
    } as unknown as CockpitClient;
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D);
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={{ session_id: "session", space_id: "space", pane_id: null, endpoint_path: null }} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(host.querySelector(".browser-annotation-notes")?.getAttribute("aria-label")).toBe("Notes 1");
    for (const [targetId, generation, sequence] of [["another-target", 2, 2], ["target", 1, 4]] as const) {
      await act(async () => {
        emitEvent({ type: "document_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: sequence }, document: { target_id: targetId, frame_id: "frame", document_generation: generation, frame_generation: sequence } });
        await new Promise<void>((resolve) => setTimeout(resolve, 0));
        emitEvent({ type: "targets_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: sequence + 1 }, targets, displayed_target_id: targetId });
        await new Promise<void>((resolve) => setTimeout(resolve, 30));
      });
    }
    expect(host.querySelector(".browser-annotation-notes")?.getAttribute("aria-label")).toBe("Notes 1");
    expect(recovery.mock.calls.some(([request]) => request.action.type === "discard_draft")).toBe(false);
  });
  it("clears the acknowledged latest draft revision after a pending editor save", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    let releaseEditor!: () => void;
    const editorGate = new Promise<void>((resolve) => { releaseEditor = resolve; });
    const marked: BrowserViewDraftState = {
      draft_id: "marked", target_id: "target", document_generation: 1, revision: 2,
      annotations: [{ id: "region", kind: "region", color: "#d62828", points: [{ x: 10, y: 10 }, { x: 100, y: 90 }], bounds: { x: 10, y: 10, width: 90, height: 80 }, evidence: null, comment: "Durable note" }],
      freshness: "fresh", stale: false, editor: { selected_annotation_id: "region", notes_open: false, note_annotation_id: "region", note_text: "Durable note" },
    };
    const clean = { ...marked, draft_id: "clean", revision: 1, annotations: [],
      editor: { selected_annotation_id: null, notes_open: false, note_annotation_id: null, note_text: "" } };
    let stored: BrowserViewDraftState[] = [marked];
    const recovery = vi.fn(async (request: BrowserDraftRecoveryRequest): Promise<BrowserViewCommandOutcome> => {
      if (request.action.type === "list") return { type: "draft_inventory", inventory: { drafts: stored, active_draft_limit: 8, pending_capture: null } };
      if (request.action.type === "set_editor") {
        await editorGate;
        stored = [{ ...marked, revision: 3, editor: request.action.editor }];
        return { type: "draft", draft: stored[0] };
      }
      if (request.action.type === "discard_draft") {
        if (request.action.draft_id !== "marked" || request.action.expected_revision !== stored[0]?.revision) throw new Error("draft revision conflict");
        stored = [];
        return { type: "none" };
      }
      throw new Error(`Unexpected recovery mutation: ${request.action.type}`);
    });
    const client = {
      browserDraftRecovery: recovery,
      openBrowserView: vi.fn(async (_request, onEvent) => {
        onEvent({ type: "attached", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 1 }, snapshot: snapshot() });
        return { command: async (request: BrowserViewCommandRequest) => {
          if (request.command.type !== "draft") return accepted(request);
          if (request.command.command.type === "list") return accepted(request, { type: "draft_inventory", inventory: { drafts: stored, active_draft_limit: 8, pending_capture: null } });
          return accepted(request, { type: "draft", draft: stored[0] ?? clean });
        }, close: vi.fn() };
      }),
    } as unknown as CockpitClient;
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({ clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D);
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={{ session_id: "session", space_id: "space", pane_id: null, endpoint_path: null }} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    expect(host.querySelector(".browser-annotation-notes")?.getAttribute("aria-label")).toBe("Notes 1");
    await act(async () => {
      const editor = host!.querySelector<HTMLTextAreaElement>('textarea[aria-label="Annotation note"]')!;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(editor, "New editor text");
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await vi.waitFor(() => expect(recovery.mock.calls.some(([request]) => request.action.type === "set_editor")).toBe(true));
    await act(async () => {
      host!.querySelector<HTMLButtonElement>('button[aria-label="Remove selected annotation or Control-click to discard draft"]')!
        .dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true }));
      releaseEditor();
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
    });
    await vi.waitFor(() => expect(host!.querySelector(".browser-annotation-notes")?.getAttribute("aria-label")).toBe("Notes 0"));
    expect(recovery.mock.calls.filter(([request]) => request.action.type === "discard_draft").map(([request]) => request.action.type === "discard_draft" ? request.action.expected_revision : null)).toEqual([3]);
    expect(stored).toEqual([]);
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });
  it("reopens the authoritative editable draft after a rejected saved-feedback delivery", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    let emitFrame!: (frame: BrowserViewFramePacket) => void;
    let authoritativeDraft: BrowserViewDraftState = {
      draft_id: "draft", target_id: "target", document_generation: 1, revision: 4,
      annotations: [{ id: "annotation", kind: "region", color: "#d62828", points: [{ x: 10, y: 10 }, { x: 100, y: 90 }], bounds: { x: 10, y: 10, width: 90, height: 80 }, evidence: null, comment: "Frozen note" }],
      freshness: "fresh", stale: false, editor: { selected_annotation_id: null, notes_open: false, note_annotation_id: null, note_text: "" },
    };
    let captureSaved = false;
    const emptyDraft = { ...authoritativeDraft, revision: 5, annotations: [] };
    const client = {
      openBrowserView: vi.fn(async (_request, onEvent, onFrame) => {
        emitFrame = onFrame;
        onEvent({ type: "attached", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 1 }, snapshot: snapshot() });
        return { command: async (request: BrowserViewCommandRequest): Promise<BrowserViewCommandResponse> => {
          if (request.command.type === "capture") return accepted(request, { type: "capture_prepared", capture_id: "capture", descriptor: descriptor(1, 1, 0) });
          if (request.command.type !== "draft") return accepted(request);
          if (request.command.command.type === "list") return accepted(request, { type: "draft_inventory", inventory: { drafts: [authoritativeDraft], active_draft_limit: 8, pending_capture: null } });
          if (request.command.command.type === "open") return accepted(request, { type: "draft", draft: authoritativeDraft });
          if (request.command.command.type === "save_capture") {
            captureSaved = true;
            return accepted(request, { type: "capture", capture: { state: "saved", saved: { capture_id: "capture", annotation_ids: ["annotation"], image_path: "capture.png", pending_count: 1 } } });
          }
          return accepted(request);
        }, close: vi.fn() };
      }),
      browserFeedback: vi.fn(async () => ({
        feedback: { captures: captureSaved ? [{ id: "capture", pending_ids: ["annotation"] }] : [], pending_count: captureSaved ? 1 : 0 },
        deliveries: captureSaved ? [{ capture_id: "capture", operation_id: "browser-feedback-capture", selected_ids: ["annotation"], state: "rejected", message: "No eligible agent." }] : [],
      })),
    } as unknown as CockpitClient;
    vi.stubGlobal("createImageBitmap", vi.fn(async () => ({ width: 4, height: 3, close: vi.fn() })));
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
      clearRect: vi.fn(), drawImage: vi.fn(), fillRect: vi.fn(), strokeRect: vi.fn(),
      strokeText: vi.fn(), fillText: vi.fn(),
    } as unknown as CanvasRenderingContext2D);
    vi.spyOn(HTMLCanvasElement.prototype, "toBlob").mockImplementation((callback) => callback(new Blob(["image"], { type: "image/png" })));
    class TestFileReader {
      result: string | null = null;
      onload: ((event: ProgressEvent<FileReader>) => void) | null = null;
      readAsDataURL(): void { this.result = "data:image/png;base64,aW1hZ2U="; this.onload?.(new ProgressEvent("load") as ProgressEvent<FileReader>); }
    }
    vi.stubGlobal("FileReader", TestFileReader);
    host = document.createElement("div");
    document.body.append(host);
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={{ session_id: "session", space_id: "space", pane_id: "pane", endpoint_path: null }} viewport={{ css_width: 800, css_height: 600, device_pixel_ratio: 1 }} onFeedback={async (_ids, operation_id) => {
        authoritativeDraft = emptyDraft;
        return { operation_id, state: "rejected", target: null, acknowledged_ids: [], pending_count: 1, message: "No eligible agent." };
      }} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    await act(async () => { emitFrame(packet(descriptor(1, 1, 0))); await new Promise<void>((resolve) => setTimeout(resolve, 30)); });
    const send = host.querySelector<HTMLButtonElement>('button[aria-label="Send annotations"]')!;
    expect(send.disabled).toBe(false);
    await act(async () => { send.click(); await new Promise<void>((resolve) => setTimeout(resolve, 30)); });
    expect(host.querySelector(".browser-annotation-region")).toBeNull();
    expect(host.querySelector(".browser-annotation-notes")?.getAttribute("aria-label")).toBe("Notes 0");
    expect(host.querySelector('[aria-label="Saved feedback recovery"]')?.textContent).toContain("No eligible agent.");
    expect(client.browserFeedback).toHaveBeenCalled();
  });
});
