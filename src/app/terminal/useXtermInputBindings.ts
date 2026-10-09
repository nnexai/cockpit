import { useEffect } from "react";
import { detectPlatform } from "../input/shortcuts";
import type { TerminalRefs, TerminalPaneRequest } from "./paneState";
import type { TerminalControl } from "./useTerminalControl";
import type { TerminalPointer } from "./useTerminalPointer";
import type { TerminalClipboard } from "./terminalClipboard";
import type { TerminalResizeController } from "./useTerminalResize";
import { commandInput, POPUP_CURSOR_KEYS, terminalModifiedEnterInput } from "./terminalInput";

export function useXtermInputBindings(refs: TerminalRefs, request: TerminalPaneRequest, control: TerminalControl, clipboard: TerminalClipboard, sizing: TerminalResizeController, pointer: TerminalPointer) {
  const { terminalRef } = refs;
  const { sendInput } = control;
  const { copySelection, pasteClipboard } = clipboard;
  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    const data = terminal.onData((text) => sendInput(commandInput(text, null)));
    const binary = terminal.onBinary((bytes) => sendInput(commandInput(null, btoa(bytes))));
    terminal.attachCustomKeyEventHandler((event) => {
      const key = event.key.toLowerCase();
      const shortcutModifier = event.ctrlKey || event.metaKey;
      const macCommand = detectPlatform() === "mac" && event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;
      // Popup frames are ANSI screen snapshots and omit DEC cursor-key mode.
      // Herdr's curses popups enable application cursor keys, so forward those
      // sequences explicitly instead of letting xterm emit normal-mode arrows.
      if (request.target_kind === "popup" && event.type === "keydown"
        && !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) {
        const sequence = POPUP_CURSOR_KEYS[event.key];
        if (sequence) {
          event.preventDefault();
          sendInput(commandInput(sequence, null));
          return false;
        }
      }
      // macOS: Cmd+C copies only with a selection (otherwise it falls through), Cmd+V pastes.
      if (event.type === "keydown" && macCommand && key === "c" && terminal.hasSelection()) {
        void copySelection();
        event.preventDefault();
        return false;
      }
      if (event.type === "keydown" && macCommand && key === "v") {
        event.preventDefault();
        void pasteClipboard();
        return false;
      }
      if (event.type === "keydown" && shortcutModifier && event.shiftKey && key === "c" && !event.altKey) {
        if (!terminal.hasSelection()) return true;
        void copySelection();
        event.preventDefault();
        return false;
      }
      if (event.type === "keydown" && shortcutModifier && event.shiftKey && key === "v" && !event.altKey) {
        event.preventDefault();
        void pasteClipboard();
        return false;
      }
      const text = terminalModifiedEnterInput(event);
      if (text === null) return true;
      event.preventDefault();
      sendInput(commandInput(text, null));
      return false;
    });
    const resize = terminal.onResize((grid) => sizing.handleTerminalResize(terminal, grid));
    terminal.attachCustomWheelEventHandler((event) => pointer.handleWheel(terminal, event));
    return () => {
      data.dispose();
      binary.dispose();
      resize.dispose();
      terminal.attachCustomWheelEventHandler(() => true);
      terminal.attachCustomKeyEventHandler(() => true);
    };
  }, []);
}
