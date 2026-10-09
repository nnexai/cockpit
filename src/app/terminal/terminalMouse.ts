import type { TerminalCommand, TerminalMouseButton, TerminalMouseKind } from "../../protocol/generated/v1";

type TerminalBounds = Pick<DOMRect, "left" | "top" | "width" | "height">;
export function terminalCellPosition(clientX: number, clientY: number, bounds: TerminalBounds, cols: number, rows: number): { column: number; row: number } {
  const safeCols = Math.max(1, Math.floor(cols));
  const safeRows = Math.max(1, Math.floor(rows));
  const column = bounds.width > 0 ? Math.floor((clientX - bounds.left) / bounds.width * safeCols) : 0;
  const row = bounds.height > 0 ? Math.floor((clientY - bounds.top) / bounds.height * safeRows) : 0;
  return {
    column: Math.max(0, Math.min(safeCols - 1, column)),
    row: Math.max(0, Math.min(safeRows - 1, row)),
  };
}

type TerminalPointer = Pick<PointerEvent, "clientX" | "clientY" | "shiftKey" | "ctrlKey" | "altKey" | "metaKey">;
export function terminalMouseButton(button: number): TerminalMouseButton | null {
  if (button === 0) return "left";
  if (button === 1) return "middle";
  if (button === 2) return "right";
  return null;
}

export function terminalModifierBits(event: Pick<PointerEvent, "shiftKey" | "ctrlKey" | "altKey" | "metaKey">): number {
  return (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
}

export function terminalMouseCommand(
  kind: TerminalMouseKind,
  button: TerminalMouseButton | null,
  event: TerminalPointer,
  bounds: TerminalBounds,
  cols: number,
  rows: number,
): TerminalCommand {
  const position = terminalCellPosition(event.clientX, event.clientY, bounds, cols, rows);
  return {
    type: "terminal.mouse",
    kind,
    button,
    column: position.column,
    row: position.row,
    modifiers: terminalModifierBits(event),
  };
}

export function forwardTerminalMouse(
  enabled: boolean,
  send: (command: TerminalCommand) => void,
  kind: TerminalMouseKind,
  button: TerminalMouseButton | null,
  event: TerminalPointer,
  bounds: TerminalBounds,
  cols: number,
  rows: number,
): boolean {
  if (!enabled) return false;
  send(terminalMouseCommand(kind, button, event, bounds, cols, rows));
  return true;
}

export function terminalScrollCommand(event: WheelEvent, bounds: TerminalBounds | undefined, cols: number, rows: number, fallbackCellHeight: () => number): TerminalCommand {
  const position = bounds ? terminalCellPosition(event.clientX, event.clientY, bounds, cols, rows) : { column: 0, row: 0 };
  const cellHeight = bounds && rows > 0 ? bounds.height / rows : fallbackCellHeight();
  return {
    type: "terminal.scroll",
    direction: event.deltaY < 0 ? "up" : "down",
    lines: Math.min(65535, Math.max(1, Math.ceil(Math.abs(event.deltaY) / Math.max(1, cellHeight)))),
    source: "wheel",
    column: position.column,
    row: position.row,
    modifiers: terminalModifierBits(event),
  };
}
