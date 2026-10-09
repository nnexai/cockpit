import type { Terminal } from "@xterm/xterm";
import type { TerminalStreamMessage } from "../../protocol/generated/v1";
import type { TerminalRefs } from "./paneState";
import type { TerminalResizeController } from "./useTerminalResize";
import { validTerminalGrid } from "./cockpitTerminal";

const MAX_QUEUED_FRAME_COUNT = 64;
const MAX_QUEUED_FRAME_BYTES = 8 * 1024 * 1024;
/** DEC 2026: xterm holds rendering until the frame's closing sequence. */
const SYNC_OUTPUT_BEGIN = "\x1b[?2026h";
function decodeFrame(bytes: string): Uint8Array {
  const binary = globalThis.atob(bytes);
  const data = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) data[index] = binary.charCodeAt(index);
  return data;
}
type QueuedFrame = {
  text: Uint8Array | null;
  firstFrame: boolean;
  retainedFocus: boolean;
  width: number;
  height: number;
};
export type FrameAttachment = {
  terminal: Terminal; cancelled(): boolean; isLatest(): boolean;
  fail(code: string, message: string): void; onBacklog?: () => void;
};

export function useTerminalFrameQueue(refs: TerminalRefs, resize: TerminalResizeController, setFramePainted: (painted: boolean) => void) {
  const { terminalRef, ownershipRef, controlAllowedRef, selectedRef, presentedRef, onReadyRef } = refs;
  const start = ({ terminal, cancelled, isLatest, fail, onBacklog }: FrameAttachment) => {
    let lastSequence: bigint | null = null;
    let readinessRender: { dispose(): void } | null = null;
    let frameQueue: QueuedFrame[] = [];
    let frameQueueBytes = 0;
    let frameApplying = false;
    let frameQueueCount = 0;
    const releaseFrame = (frame: QueuedFrame) => {
      if (frame.text === null) return;
      frameQueueBytes = Math.max(0, frameQueueBytes - frame.text.byteLength);
      frameQueueCount = Math.max(0, frameQueueCount - 1);
      frame.text = null;
    };
    const clearFrameQueue = () => {
      for (const frame of frameQueue) releaseFrame(frame);
      frameQueue = [];
    };
    const drainFrameQueue = () => {
      if (frameApplying || cancelled() || !isLatest() || terminalRef.current !== terminal) return;
      const frame = frameQueue.shift();
      if (!frame) return;
      frameApplying = true;
      let completed = false;
      const finishFrame = () => {
        if (completed) return;
        completed = true;
        try {
          if (cancelled() || !isLatest() || terminalRef.current !== terminal) return;
          if (frame.firstFrame) {
            readinessRender = terminal.onRender(() => {
              readinessRender?.dispose();
              readinessRender = null;
              if (!cancelled() && isLatest() && terminalRef.current === terminal) { setFramePainted(true); onReadyRef.current?.(); }
            });
            terminal.refresh(0, terminal.rows - 1);
          }
          if (frame.retainedFocus && selectedRef.current && presentedRef.current && controlAllowedRef.current && ownershipRef.current === "owned" && document.activeElement === document.body) terminal.focus();
        } catch {
          if (!cancelled() && isLatest()) fail("terminal_frame", "Terminal could not render a frame");
        } finally {
          releaseFrame(frame);
          frameApplying = false;
          if (cancelled()) clearFrameQueue();
          else drainFrameQueue();
        }
      };
      const text = frame.text;
      if (!text || cancelled() || !isLatest() || terminalRef.current !== terminal) {
        finishFrame();
        return;
      }
      try {
        const frameGrid = { cols: frame.width, rows: frame.height };
        resize.acceptFrameGrid(frameGrid);
        const present = () => {
          resize.resizeToFrame(terminal, frameGrid);
          terminal.write(text, finishFrame);
        };
        if (terminal.cols === frameGrid.cols && terminal.rows === frameGrid.rows) {
          resize.markRendered(frameGrid);
          terminal.write(text, finishFrame);
        } else {
          // xterm reflows the old buffer on resize and could paint it before
          // this frame replaces it; hold rendering until the frame completes.
          // resize() flushes xterm's write queue, so it must run after the
          // write loop has returned, never inside one of its callbacks.
          terminal.write(SYNC_OUTPUT_BEGIN, () => queueMicrotask(() => {
            if (cancelled() || !isLatest() || terminalRef.current !== terminal) {
              finishFrame();
              return;
            }
            try {
              present();
            } catch {
              fail("terminal_frame", "Terminal could not render a frame");
              finishFrame();
            }
          }));
        }
      } catch {
        if (!cancelled() && isLatest()) fail("terminal_frame", "Terminal could not render a frame");
        finishFrame();
      }
    };
    const receive = (message: Extract<TerminalStreamMessage, { type: "frame" }>) => {
        let sequence: bigint;
        try {
          sequence = BigInt(message.seq);
        } catch {
          fail("terminal_sequence", "Terminal sent an invalid sequence");
          return;
        }
        const firstFrame = lastSequence === null;
        if (lastSequence === null && !message.full) {
          fail("terminal_sequence", "Terminal stream must begin with a full frame");
          return;
        }
        if (lastSequence !== null && sequence !== lastSequence + 1n) {
          fail("terminal_sequence", "Terminal output sequence is not consecutive");
          return;
        }
        if (!validTerminalGrid(message.width, message.height)) {
          fail("terminal_frame", "Terminal sent invalid frame dimensions");
          return;
        }
        lastSequence = sequence;
        let text: Uint8Array;
        try {
          text = decodeFrame(message.bytes);
        } catch {
          fail("terminal_frame", "Terminal sent an invalid frame");
          return;
        }
        if (
          frameQueueCount >= MAX_QUEUED_FRAME_COUNT
          || frameQueueBytes + text.byteLength > MAX_QUEUED_FRAME_BYTES
        ) {
          onBacklog?.();
          fail("terminal_frame_backlog", "Terminal frame backlog exceeded memory bounds");
          return;
        }
        frameQueue.push({
          text,
          firstFrame,
          retainedFocus: terminal.element?.contains(document.activeElement) ?? false,
          width: message.width,
          height: message.height,
        });
        frameQueueCount += 1;
        frameQueueBytes += text.byteLength;
        drainFrameQueue();
    };
    const disposeReadiness = () => { readinessRender?.dispose(); };
    return { receive, clear: clearFrameQueue, disposeReadiness };
  };
  return { start };
}
export type TerminalFrameQueues = ReturnType<typeof useTerminalFrameQueue>;
