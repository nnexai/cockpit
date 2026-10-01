import type { HerdrCommand } from "../../protocol/generated/v1";

export type HerdrBindingEvent = Pick<KeyboardEvent, "key" | "shiftKey" | "ctrlKey" | "altKey" | "metaKey"> & { code?: string };
export type HerdrChord = { prefix: boolean; key: string; ctrl: boolean; alt: boolean; shift: boolean; meta: boolean; display: string };
export type HerdrBinding = HerdrChord & { command: HerdrCommand };
const keys: Record<string, string> = {
  space: " ", enter: "Enter", return: "Enter", esc: "Escape", escape: "Escape", tab: "Tab", backspace: "Backspace", bs: "Backspace", delete: "Delete", insert: "Insert", home: "Home", end: "End", pageup: "PageUp", pagedown: "PageDown",
  left: "ArrowLeft", right: "ArrowRight", up: "ArrowUp", down: "ArrowDown", minus: "-", comma: ",", period: ".", slash: "/", backslash: "\\", quote: "'", double_quote: '"', "double-quote": '"', semicolon: ";", colon: ":", percent: "%", ampersand: "&", backtick: "`", plus: "+",
};
function parse(label: string): HerdrChord | null {
  const parts = label.split("+");
  const name = parts.pop();
  if (!name) return null;
  const binding: HerdrChord = { prefix: false, key: keys[name.toLowerCase()] ?? (/^f\d+$/i.test(name) ? name.toUpperCase() : name), ctrl: false, alt: false, shift: /^[A-Z]$/.test(name), meta: false, display: "" };
  for (const modifier of parts) {
    switch (modifier.toLowerCase()) {
      case "prefix": binding.prefix = true; break;
      case "ctrl": case "control": binding.ctrl = true; break;
      case "alt": case "option": case "meta": binding.alt = true; break;
      case "shift": binding.shift = true; break;
      case "cmd": case "command": case "super": binding.meta = true; break;
      default: return null;
    }
  }
  if (!(name.toLowerCase() in keys) && name.length !== 1 && !/^f\d+$/i.test(name)) return null;
  const displayKey = binding.key === " " ? "Space" : binding.key === "Escape" ? "Esc" : binding.key.replace(/^Arrow/, "");
  binding.display = `${binding.ctrl ? "Ctrl+" : ""}${binding.alt ? "Alt+" : ""}${binding.meta ? "Cmd+" : ""}${binding.shift ? "Shift+" : ""}${displayKey.length === 1 ? displayKey.toUpperCase() : displayKey}`;
  return binding;
}
export function herdrBindings(commands: readonly HerdrCommand[]): readonly HerdrBinding[] {
  return commands.filter(command => command.action !== "unknown").flatMap(command => command.binding_labels.flatMap(label => { const binding = parse(label); return binding ? [{ ...binding, command }] : []; }));
}
export function herdrPrefixes(labels: readonly string[]): readonly HerdrChord[] {
  return labels.flatMap(label => { const chord = parse(label); return chord && !chord.prefix ? [chord] : []; });
}
export function herdrBindingMatches(binding: HerdrChord, event: HerdrBindingEvent): boolean {
  const shiftedSymbol = event.shiftKey && !binding.shift && binding.key.length === 1 && !/^[a-z ]$/i.test(binding.key) && event.key === binding.key;
  if (binding.ctrl !== event.ctrlKey || binding.alt !== event.altKey || (!shiftedSymbol && binding.shift !== event.shiftKey) || binding.meta !== event.metaKey) return false;
  // Alt/Option can replace a letter with a symbol or dead key, but an actual
  // letter still follows the active keyboard layout (for example Colemak).
  if (binding.alt && /^[a-z]$/i.test(binding.key) && !/^[a-z]$/i.test(event.key) && /^Key[A-Z]$/.test(event.code ?? "")) return event.code === `Key${binding.key.toUpperCase()}`;
  return binding.key.toLowerCase() === event.key.toLowerCase();
}
export function matchingHerdrCommand(bindings: readonly HerdrBinding[], event: HerdrBindingEvent, prefix: boolean): HerdrCommand | null {
  return bindings.find(binding => binding.prefix === prefix && herdrBindingMatches(binding, event))?.command ?? null;
}
export function herdrCommandShortcut(command: HerdrCommand, prefixes: readonly HerdrChord[]): string {
  return herdrBindings([command]).flatMap(binding => binding.prefix ? prefixes.map(prefix => `${prefix.display} ${binding.display}`) : [binding.display]).join(" or ");
}

// Cockpit has one active workbench. Its authoritative manifest replaces this derived table each render.
let effectiveBindings: readonly HerdrBinding[] = [];
let effectivePrefixes: readonly HerdrChord[] = [];
export function setEffectiveHerdrBindings(bindings: readonly HerdrBinding[], prefixes: readonly HerdrChord[] = []): void { effectiveBindings = bindings; effectivePrefixes = prefixes; }
export function herdrShadowsPrefix(key: string, shift: boolean | "any"): boolean {
  return effectivePrefixes.some(prefix => prefix.ctrl && !prefix.alt && !prefix.shift && !prefix.meta && prefix.key.toLowerCase() === "b") && effectiveBindings.some(binding => binding.prefix && !binding.ctrl && !binding.alt && !binding.meta && binding.key.toLowerCase() === key.toLowerCase() && (shift === "any" || binding.shift === shift));
}
export function herdrShadowsChord(event: HerdrBindingEvent): boolean { return matchingHerdrCommand(effectiveBindings, event, false) !== null; }
