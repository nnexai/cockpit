import type { QuickAskHarness, QuickAskPhase } from "../../protocol/generated/v1";

export const QUICK_ASK_PROMPT_LIMIT = 16_384;
export const QUICK_ASK_HARNESSES: QuickAskHarness[] = ["omp", "pi", "claude", "copilot", "codex", "opencode"];

export function quickAskPromptValidation(prompt: string): string | null {
  if (!prompt.trim()) return "Enter a question.";
  if (new TextEncoder().encode(prompt).length > QUICK_ASK_PROMPT_LIMIT) return "Question exceeds 16,384 UTF-8 bytes.";
  if (/[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/u.test(prompt)) return "Question contains a control character that cannot be sent safely.";
  return null;
}

export function quickAskTerminal(phase: QuickAskPhase): boolean {
  return phase === "submitted" || phase === "initial_question_supplied" || phase === "cancelled" || phase === "failed" || phase === "outcome_unknown";
}
