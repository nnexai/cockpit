import { Terminal } from "@xterm/xterm";
import { WebglAddon } from "@xterm/addon-webgl";
import { snapTerminalFontSize, TERMINAL_THEME, TERMINAL_FONT_FAMILY } from "./terminalTheme";

export function createCockpitTerminal(fontSize = snapTerminalFontSize()): Terminal {
  return new Terminal({
    convertEol: false,
    cursorBlink: false,
    fontFamily: TERMINAL_FONT_FAMILY,
    fontSize,
    lineHeight: 1,
    // Herdr owns terminal scroll position. A local full-height scrollbar has
    // no authoritative position and reads as a second pane divider.
    scrollbar: { showScrollbar: false, width: 8 },
    theme: TERMINAL_THEME,
    scrollback: 5000,
    vtExtensions: { kittyKeyboard: true },
  });
}

// The DOM renderer draws box/block glyphs from the font, so their edges land on
// fractional device pixels and leave seams between rows at most display scales.
// The WebGL renderer draws them procedurally on the pixel grid. Falls back to
// the DOM renderer when WebGL is unavailable or the context is lost.
export function loadGpuRenderer(terminal: Terminal): void {
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => webgl.dispose());
    terminal.loadAddon(webgl);
  } catch {
    // DOM renderer stays active.
  }
}

export type TerminalGrid = { cols: number; rows: number };
export type CellGeometry = { cell_width_px: number; cell_height_px: number };
export function terminalScreenBounds(terminal: Terminal): DOMRect | undefined {
  return terminal.element?.querySelector<HTMLElement>(".xterm-screen")?.getBoundingClientRect();
}
export function terminalCellGeometry(terminal: Terminal, grid?: { cols: number; rows: number }): { cell_width_px: number; cell_height_px: number } {
  const bounds = terminalScreenBounds(terminal);
  const cols = grid?.cols ?? terminal.cols;
  const rows = grid?.rows ?? terminal.rows;
  return {
    cell_width_px: bounds && cols > 0 ? Math.max(1, Math.round(bounds.width / cols)) : 0,
    cell_height_px: bounds && rows > 0 ? Math.max(1, Math.round(bounds.height / rows)) : 0,
  };
}
export function validTerminalGrid(cols: number, rows: number): boolean {
  return Number.isInteger(cols) && Number.isInteger(rows)
    && cols > 0 && rows > 0 && cols <= 65535 && rows <= 65535;
}
