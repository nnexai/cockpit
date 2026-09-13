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

async function withClipboardFallback<T>(primary: () => Promise<T>, fallback: ClipboardAccess | null, operation: (clipboard: ClipboardAccess) => Promise<T>): Promise<T> {
  try {
    return await primary();
  } catch (error) {
    if (!fallback) throw error;
    return operation(fallback);
  }
}

/** Select the platform clipboard at the user gesture that invokes the operation. */
export function createClipboardAccess(): ClipboardAccess {
  const web = webClipboard();
  const userAgent = globalThis.navigator?.userAgent ?? "";
  const isNativePlatform = /Linux|Mac OS X/i.test(userAgent);
  if (!isTauri() || !isNativePlatform) {
    if (!web) throw new Error("Clipboard access is unavailable");
    return web;
  }
  const native = nativeClipboard();
  return {
    readText: () => withClipboardFallback(() => native.readText(), web, (clipboard) => clipboard.readText()),
    writeText: (text) => withClipboardFallback(() => native.writeText(text), web, (clipboard) => clipboard.writeText(text)),
  };
}
