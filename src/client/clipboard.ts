import { invoke as tauriInvoke, isTauri } from "@tauri-apps/api/core";

export type ClipboardAccess = Pick<Clipboard, "readText" | "writeText">;

function webClipboard(): ClipboardAccess {
  const clipboard = globalThis.navigator?.clipboard;
  if (!clipboard) throw new Error("Clipboard access is unavailable");
  return clipboard;
}

function nativeClipboard(): ClipboardAccess {
  return {
    readText: () => tauriInvoke<string>("cockpit_clipboard_read"),
    writeText: (text) => tauriInvoke("cockpit_clipboard_write", { text }).then(() => undefined),
  };
}

/** Select the platform clipboard at the user gesture that invokes the operation. */
export function createClipboardAccess(): ClipboardAccess {
  return isTauri() ? nativeClipboard() : webClipboard();
}
