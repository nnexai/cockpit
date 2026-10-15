import { invoke as tauriInvoke, isTauri } from "@tauri-apps/api/core";

export type ClipboardAccess = Pick<Clipboard, "readText" | "writeText">;

function webClipboard(): ClipboardAccess | null {
  const clipboard = globalThis.navigator?.clipboard;
  return clipboard ?? null;
}

function nativeClipboard(): ClipboardAccess {
  return {
    readText: () => tauriInvoke<string>("cockpit_clipboard_read"),
    writeText: (text) => tauriInvoke("cockpit_clipboard_write", { text }).then(() => undefined),
  };
}

function hasNativeClipboard(): boolean {
  const userAgent = globalThis.navigator?.userAgent ?? "";
  return isTauri() && /Linux|Mac OS X/i.test(userAgent);
}

/** Select the platform clipboard at the user gesture that invokes the operation. */
export function createClipboardAccess(): ClipboardAccess {
  if (hasNativeClipboard()) return nativeClipboard();
  const web = webClipboard();
  if (!web) throw new Error("Clipboard access is unavailable");
  return web;
}

/** Terminal paste may resolve native files/images without changing readText's text-only contract. */
export async function readTerminalClipboardContent(): Promise<string> {
  if (hasNativeClipboard()) return tauriInvoke<string>("cockpit_clipboard_read_terminal");
  return createClipboardAccess().readText();
}

/** Browser File names are not paths: only the native host can stage usable terminal files. */
export async function stageTerminalFiles(files: readonly File[]): Promise<string> {
  if (!isTauri()) throw new Error("Dropping files into a terminal requires the native Cockpit app");
  const payload = await Promise.all(files.map(async (file) => ({
    name: file.name,
    bytes: Array.from(new Uint8Array(await file.arrayBuffer())),
  })));
  return tauriInvoke<string>("cockpit_terminal_files_stage", { files: payload });
}
