import { useEffect, useRef, useState } from "react";
import type { Terminal } from "@xterm/xterm";
import { createClipboardAccess, readTerminalClipboardContent, stageTerminalFiles, type ClipboardAccess } from "../../client/clipboard";
import type { TerminalOwnershipState } from "../../protocol/generated/v1";
import type { TerminalRefs } from "./paneState";
import type { TerminalControl } from "./useTerminalControl";
import { isTerminalFileDropTarget } from "../input/fileDrop";

export async function copyTerminalSelection(terminal: Pick<Terminal, "getSelection">, clipboard: ClipboardAccess = createClipboardAccess()): Promise<boolean> {
  const selection = terminal.getSelection();
  if (!selection) return false;
  await clipboard.writeText(selection);
  return true;
}

export async function readTerminalClipboard(clipboard?: ClipboardAccess): Promise<string> {
  return clipboard ? clipboard.readText() : readTerminalClipboardContent();
}

export function useTerminalClipboard(refs: TerminalRefs, control: TerminalControl) {
  const { hostRef, terminalRef, streamRef, ownershipRef, controlAllowedRef, controlPendingRef } = refs;
  const { currentIntent, requestControl } = control;
  const [clipboardError, setClipboardError] = useState<{ operation: "copy" | "paste" | "drop"; message: string } | null>(null);
  const [clipboardBusy, setClipboardBusy] = useState(false);
  const [terminalContextOpen, setTerminalContextOpen] = useState(false);
  const [terminalContextPosition, setTerminalContextPosition] = useState<{ x: number; y: number } | null>(null);
  const contextSelectionRef = useRef<string | null>(null);
  const pendingPasteRef = useRef<{ text: string | null; intent: { epoch: number; paneId: string; token: number }; sawControlPending: boolean; focusAfterPaste: boolean } | null>(null);
  const dropPendingPaste = () => {
    pendingPasteRef.current = null;
    setClipboardBusy(false);
  };
  const flushPendingPaste = () => {
    const pending = pendingPasteRef.current;
    const terminal = terminalRef.current;
    if (!pending || !terminal) return;
    const current = currentIntent();
    if (pending.focusAfterPaste && !isTerminalFileDropTarget(hostRef.current)) {
      dropPendingPaste();
      return;
    }
    if (pending.intent.epoch !== current.epoch || pending.intent.paneId !== current.paneId) {
      dropPendingPaste();
      return;
    }
    if (pending.intent.token !== current.token) {
      if (!controlPendingRef.current && !controlAllowedRef.current) {
        dropPendingPaste();
        return;
      }
      pending.intent = current;
    }
    if (pending.sawControlPending && !controlPendingRef.current && !controlAllowedRef.current) {
      dropPendingPaste();
      return;
    }
    pending.sawControlPending ||= controlPendingRef.current;
    if (pending.text === null || !controlAllowedRef.current || ownershipRef.current !== "owned" || !streamRef.current) return;
    pendingPasteRef.current = null;
    setClipboardBusy(false);
    terminal.paste(pending.text);
    if (pending.focusAfterPaste && document.hasFocus() && isTerminalFileDropTarget(hostRef.current)) terminal.focus();
  };

  const copySelection = async () => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const capturedSelection = contextSelectionRef.current;
    contextSelectionRef.current = null;
    setClipboardBusy(true);
    setClipboardError(null);
    try {
      if (capturedSelection !== null) {
        if (capturedSelection) await createClipboardAccess().writeText(capturedSelection);
      } else {
        await copyTerminalSelection(terminal);
      }
    } catch (cause) {
      setClipboardError({ operation: "copy", message: cause instanceof Error ? cause.message : typeof cause === "string" ? cause : "Could not copy terminal selection" });
    } finally {
      setClipboardBusy(false);
    }
  };

  const pastePayload = async (operation: "paste" | "drop", read: () => Promise<string>) => {
    if (!terminalRef.current) return;
    requestControl();
    const pending = { text: null as string | null, intent: currentIntent(), sawControlPending: controlPendingRef.current, focusAfterPaste: operation === "drop" };
    pendingPasteRef.current = pending;
    setClipboardBusy(true);
    setClipboardError(null);
    try {
      const text = await read();
      // A reconnect, refused control request, newer paste or pane switch invalidates the read too.
      if (pendingPasteRef.current !== pending) return;
      pending.text = text;
      flushPendingPaste();
    } catch (cause) {
      if (pendingPasteRef.current !== pending) return;
      dropPendingPaste();
      setClipboardError({ operation, message: cause instanceof Error ? cause.message : typeof cause === "string" ? cause : operation === "drop" ? "Could not read the dropped file" : "Could not read the clipboard" });
    }
  };
  const pasteClipboard = () => pastePayload("paste", () => readTerminalClipboard());
  const pasteFiles = (files: readonly File[]) => pastePayload("drop", () => stageTerminalFiles(files));
  return { clipboardError, clipboardBusy, terminalContextOpen, setTerminalContextOpen, terminalContextPosition, setTerminalContextPosition, contextSelectionRef, copySelection, pasteClipboard, pasteFiles, flushPendingPaste, dropPendingPaste };
}
export type TerminalClipboard = ReturnType<typeof useTerminalClipboard>;

export function useTerminalClipboardEffects(clipboard: TerminalClipboard, { controlAllowed, controlPending, focusEpoch, focusToken, ownership }: { controlAllowed: boolean; controlPending: boolean; focusEpoch: number; focusToken: number; ownership: TerminalOwnershipState }) {
  const { flushPendingPaste, dropPendingPaste } = clipboard;
  useEffect(() => {
    flushPendingPaste();
  }, [controlAllowed, controlPending, focusEpoch, focusToken, ownership]);

  useEffect(() => () => {
    dropPendingPaste();
  }, []);
}
