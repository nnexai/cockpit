import type { QuickAskHarness } from "../../protocol/generated/v1";

export const QUICK_ASK_PROMPT_LIMIT = 16_384;

/** Display order; OMP is the default. */
export const QUICK_ASK_AGENTS: readonly { id: QuickAskHarness; label: string }[] = [
  { id: "omp", label: "OMP" },
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "pi", label: "Pi" },
  { id: "copilot", label: "Copilot" },
  { id: "opencode", label: "OpenCode" },
];

export function quickAskPromptValidation(prompt: string): string | null {
  if (new TextEncoder().encode(prompt).length > QUICK_ASK_PROMPT_LIMIT) return "The question is too long (16 KB max).";
  if (/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/u.test(prompt)) return "The question contains a control character.";
  return null;
}
