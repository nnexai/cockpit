export function tabDropInsertionIndex(sourceIndex: number, targetIndex: number, afterTarget: boolean): number | null {
  if (sourceIndex < 0 || targetIndex < 0 || sourceIndex === targetIndex) return null;
  const insertionIndex = targetIndex + (afterTarget ? 1 : 0);
  const finalIndex = insertionIndex > sourceIndex ? insertionIndex - 1 : insertionIndex;
  return finalIndex === sourceIndex ? null : insertionIndex;
}
