export function applicationFontSize(): number {
  const fontSize = Number.parseFloat(globalThis.getComputedStyle(document.body).fontSize);
  return Number.isFinite(fontSize) && fontSize > 0 ? fontSize : 16;
}

// 16-color palette from Solarized Dark Patched (the Ghostty theme this terminal
// is matched against) on Cockpit's neutral background. Applications' true-color
// ANSI sequences remain untouched; only terminal palette indices use these colors.
export const TERMINAL_THEME = {
  background: "#0c1016",
  foreground: "#708284",
  cursor: "#708284",
  cursorAccent: "#0c1016",
  selectionBackground: "#315a8f99",
  black: "#002831",
  red: "#d11c24",
  green: "#738a05",
  yellow: "#a57706",
  blue: "#2176c7",
  magenta: "#c61c6f",
  cyan: "#259286",
  white: "#eae3cb",
  brightBlack: "#475b62",
  brightRed: "#bd3613",
  brightGreen: "#475b62",
  brightYellow: "#536870",
  brightBlue: "#708284",
  brightMagenta: "#5956ba",
  brightCyan: "#819090",
  brightWhite: "#fcf4dc",
} as const;

// 12.5pt: 12pt (16px) rendered too dense and 13pt (17.33px) too large next to Ghostty/kitty.
const TERMINAL_FONT_SIZE = (12.5 * 96) / 72;

// Iosevka's advance is 0.5em, so an even device-pixel font size gives whole-pixel
// cells. Fractional cells make the WebGL glyph atlas resample and look soft.
export function snapTerminalFontSize(dpr = globalThis.devicePixelRatio || 1): number {
  return Math.max(2, Math.round((TERMINAL_FONT_SIZE * dpr) / 2) * 2) / dpr;
}
export const TERMINAL_FONT_FAMILY = '"IosevkaTerm Nerd Font Mono", ui-monospace, "FiraCode Nerd Font Mono", "Hack Nerd Font Mono", "IBM Plex Mono", "Noto Sans Mono", monospace';
