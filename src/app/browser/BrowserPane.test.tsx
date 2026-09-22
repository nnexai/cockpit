// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BrowserViewCommandRequest, BrowserViewCommandResponse, BrowserViewEvent, BrowserViewFrameDescriptor, BrowserViewSnapshot, BrowserViewViewportState } from "../../protocol/generated/v1";
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

const accepted = (request: BrowserViewCommandRequest): BrowserViewCommandResponse => ({ status: "accepted", view_id: "view", stream_epoch: 1, request_id: request.request_id, outcome: { type: "none" } });

describe("BrowserPane wheel recovery", () => {
  let root: Root | null = null;
  let host: HTMLDivElement | null = null;

  afterEach(async () => {
    if (root) await act(async () => root?.unmount());
    root = null;
    host?.remove();
    host = null;
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("keeps wheel input received between viewport metadata and its matching frame", async () => {
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
    await act(async () => { await new Promise<void>((resolve) => setTimeout(resolve, 0)); });
    const surface = host.querySelector<HTMLDivElement>(".browser-surface")!;
    vi.spyOn(surface, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, toJSON: () => ({}) });

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

    await act(async () => {
      emitEvent({ type: "viewport_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 3 }, viewport: viewport(3, 180) });
      const nextFrame = packet(descriptor(3, 3, 180));
      emitFrame(nextFrame);
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
      expect(nextFrame.ack).toHaveBeenCalledOnce();
    });
    const wheels = commands.filter(({ command }) => command.type === "wheel");
    expect(wheels.map(({ command }) => command.type === "wheel" ? [command.location.viewport_revision, command.input.delta_y_css, command.input.x, command.input.y, command.input.input_sequence] : null)).toEqual([[3, 120, 400, 300, 1], [3, 100, 400, 300, 2]]);
    expect(commands.filter(({ command }) => command.type === "keyboard").map(({ command }) => command.type === "keyboard" ? [command.input.kind, command.input.key, command.input.input_sequence] : null)).toEqual([["down", "Enter", 3], ["up", "Enter", 4]]);
    expect(commands.filter(({ command }) => command.type === "wheel" || command.type === "keyboard").map(({ command }) => command.type)).toEqual(["wheel", "wheel", "keyboard", "keyboard"]);
  });

  it("releases later browser commands when the matching frame never arrives", async () => {
    Reflect.set(globalThis, "IS_REACT_ACT_ENVIRONMENT", true);
    let emitEvent!: (event: BrowserViewEvent) => void;
    const commands: BrowserViewCommandRequest[] = [];
    const client = {
      openBrowserView: vi.fn(async (_request, onEvent, _onFrame) => {
        emitEvent = onEvent;
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
    vi.useFakeTimers();
    await act(async () => {
      emitEvent({ type: "viewport_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 2 }, viewport: viewport(2, 120) });
      surface.dispatchEvent(new WheelEvent("wheel", { deltaY: 120, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
      await Promise.resolve();
      surface.dispatchEvent(new WheelEvent("wheel", { deltaY: 80, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
      surface.dispatchEvent(new WheelEvent("wheel", { deltaY: 20, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
    });
    await act(async () => {
      host!.querySelector<HTMLButtonElement>(".browser-new-tab")!.click();
      await vi.advanceTimersByTimeAsync(2_100);
    });
    expect(commands.filter(({ command }) => command.type === "wheel")).toHaveLength(0);
    expect(commands.filter(({ command }) => command.type === "tab").map(({ command }) => command.type === "tab" ? command.command.type : null)).toEqual(["create"]);
  });

  it("does not revive a waiting wheel after live input is disabled and re-enabled", async () => {
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
    const target = { session_id: "session", space_id: "space", pane_id: "pane", endpoint_path: null } as const;
    const viewportRequest = { css_width: 800, css_height: 600, device_pixel_ratio: 1 } as const;
    await act(async () => {
      root = createRoot(host!);
      root.render(<BrowserPane client={client} target={target} viewport={viewportRequest} />);
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    });
    const surface = host.querySelector<HTMLDivElement>(".browser-surface")!;
    vi.spyOn(surface, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, toJSON: () => ({}) });
    await act(async () => {
      emitEvent({ type: "viewport_changed", metadata: { view_id: "view", stream_epoch: 1, metadata_sequence: 2 }, viewport: viewport(2, 120) });
      surface.dispatchEvent(new WheelEvent("wheel", { deltaY: 120, bubbles: true, cancelable: true, clientX: 400, clientY: 300 }));
      await Promise.resolve();
    });
    await act(async () => root!.render(<BrowserPane client={client} target={target} viewport={viewportRequest} liveInputEnabled={false} />));
    await act(async () => root!.render(<BrowserPane client={client} target={target} viewport={viewportRequest} liveInputEnabled />));
    await act(async () => {
      emitFrame(packet(descriptor(2, 2, 120)));
      await new Promise<void>((resolve) => setTimeout(resolve, 30));
    });
    expect(commands.filter(({ command }) => command.type === "wheel")).toHaveLength(0);
  });
});
