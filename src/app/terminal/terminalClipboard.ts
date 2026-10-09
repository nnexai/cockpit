import { useEffect, useRef, useState } from "react";
import type { Terminal } from "@xterm/xterm";
import { createClipboardAccess, type ClipboardAccess } from "../../client/clipboard";
import type { TerminalOwnershipState } from "../../protocol/generated/v1";
import type { TerminalRefs } from "./paneState";
import type { TerminalControl } from "./useTerminalControl";

export async function copyTerminalSelection(terminal: Pick<Terminal, "getSelection">, clipboard: ClipboardAccess = createClipboardAccess()): Promise<boolean> {
  const selection = terminal.getSelection();
  if (!selection) return false;
  await clipboard.writeText(selection);
  return true;
}

export async function readTerminalClipboard(clipboard: ClipboardAccess = createClipboardAccess()): Promise<string> {
  return clipboard.readText();
}

export function useTerminalClipboard(refs: TerminalRefs, control: TerminalControl) {
  const { terminalRef, streamRef, ownershipRef, controlAllowedRef, controlPendingRef } = refs;
  const { currentIntent, requestControl } = control;
  const [clipboardError, setClipboardError] = useState<{ operation: "copy" | "paste"; message: string } | null>(null);
  const [clipboardBusy, setClipboardBusy] = useState(false);
  const [terminalContextOpen, setTerminalContextOpen] = useState(false);
  const [terminalContextPosition, setTerminalContextPosition] = useState<{ x: number; y: number } | null>(null);
  const contextSelectionRef = useRef<string | null>(null);
  const pendingPasteRef = useRef<{ text: string; intent: { epoch: number; paneId: string; token: number } } | null>(null);
  const flushPendingPaste = () => {
    const pending = pendingPasteRef.current;
    const terminal = terminalRef.current;
    if (!pending || !terminal) return;
    const current = currentIntent();
    if (pending.intent.epoch !== current.epoch || pending.intent.paneId !== current.paneId) {
      pendingPasteRef.current = null;
      return;
    }
    if (pending.intent.token !== current.token) {
      if (!controlPendingRef.current && !controlAllowedRef.current) {
        pendingPasteRef.current = null;
        return;
      }
      pending.intent = current;
    }
    if (!controlAllowedRef.current || ownershipRef.current !== "owned" || !streamRef.current) return;
    pendingPasteRef.current = null;
    terminal.paste(pending.text);
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
      setClipboardError({ operation: "copy", message: cause instanceof Error ? cause.message : "Could not copy terminal selection" });
    } finally {
      setClipboardBusy(false);
    }
  };

  const pasteClipboard = async () => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    requestControl();
    const intent = currentIntent();
    setClipboardBusy(true);
    setClipboardError(null);
    try {
      const text = await readTerminalClipboard();
      pendingPasteRef.current = { text, intent };
      flushPendingPaste();
    } catch (cause) {
      setClipboardError({ operation: "paste", message: cause instanceof Error ? cause.message : "Could not read the clipboard" });
    } finally {
      setClipboardBusy(false);
    }
  };
  const dropPendingPaste = () => { pendingPasteRef.current = null; };
  return { clipboardError, clipboardBusy, terminalContextOpen, setTerminalContextOpen, terminalContextPosition, setTerminalContextPosition, contextSelectionRef, copySelection, pasteClipboard, flushPendingPaste, dropPendingPaste };
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
