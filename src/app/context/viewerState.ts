export function keyFor(rootId: string, path: string): string {
  return `${rootId}\u0000${path}`;
}
export function readableError(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  if (typeof error === "object" && error !== null && "message" in error) {
    const message = error.message;
    if (typeof message === "string" && message) return message;
  }
  return "The Context request could not be completed.";
}

export function isEditingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target.isContentEditable);
}

export const MAX_RETAINED_FILE_STATES = 64;
