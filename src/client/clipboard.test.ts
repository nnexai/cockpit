// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { createClipboardAccess, readTerminalClipboardContent, stageTerminalFiles } from "./clipboard";

const native = vi.hoisted(() => ({ enabled: false, invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => native.enabled, invoke: native.invoke }));

afterEach(() => {
  native.enabled = false;
  native.invoke.mockReset();
  vi.unstubAllGlobals();
});

describe("clipboard contracts", () => {
  it("keeps native readText text-only while terminal paste uses the rich command", async () => {
    native.enabled = true;
    vi.stubGlobal("navigator", { userAgent: "Mac OS X", clipboard: { readText: vi.fn() } });
    native.invoke.mockResolvedValueOnce("ordinary text").mockResolvedValueOnce("'/tmp/image.png'");
    expect(await createClipboardAccess().readText()).toBe("ordinary text");
    expect(await readTerminalClipboardContent()).toBe("'/tmp/image.png'");
    expect(native.invoke.mock.calls).toEqual([["cockpit_clipboard_read"], ["cockpit_clipboard_read_terminal"]]);
  });

  it("does not mask native read or write failures with stale browser clipboard content", async () => {
    native.enabled = true;
    const readText = vi.fn(async () => "stale text");
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal("navigator", { userAgent: "Linux", clipboard: { readText, writeText } });
    native.invoke.mockRejectedValue(new Error("native clipboard denied"));
    await expect(createClipboardAccess().readText()).rejects.toThrow("native clipboard denied");
    await expect(readTerminalClipboardContent()).rejects.toThrow("native clipboard denied");
    await expect(createClipboardAccess().writeText("selected")).rejects.toThrow("native clipboard denied");
    expect(readText).not.toHaveBeenCalled();
    expect(writeText).not.toHaveBeenCalled();
  });

  it("stages exact bytes and original basenames, returning only host-provided quoted paths", async () => {
    native.enabled = true;
    const bytes = new Uint8Array([0, 128, 255]);
    const file = new File([bytes], "a b'c.png", { type: "image/png" });
    Object.defineProperty(file, "arrayBuffer", { value: async () => bytes.buffer });
    native.invoke.mockResolvedValue("'/tmp/staged/a b'\\''c.png'");
    expect(await stageTerminalFiles([file])).toBe("'/tmp/staged/a b'\\''c.png'");
    expect(native.invoke).toHaveBeenCalledWith("cockpit_terminal_files_stage", { files: [{ name: "a b'c.png", bytes: [0, 128, 255] }] });
  });

  it("preserves browser text paste but rejects local-file staging instead of pasting a filename", async () => {
    const readText = vi.fn(async () => "browser text");
    vi.stubGlobal("navigator", { clipboard: { readText, writeText: vi.fn() } });
    expect(await readTerminalClipboardContent()).toBe("browser text");
    await expect(stageTerminalFiles([new File(["data"], "not-a-path.txt")])).rejects.toThrow("requires the native Cockpit app");
    expect(native.invoke).not.toHaveBeenCalled();
  });
});
