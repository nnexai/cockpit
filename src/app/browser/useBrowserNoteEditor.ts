import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import type { BrowserViewDraftAnnotation } from "../../protocol/generated/v1";
import { errorMessage, noteEditorPlacement } from "./browserPaneModel";
import type { BrowserPaneState } from "./useBrowserPaneState";
import type { BrowserDrafts } from "./useBrowserDrafts";
import type { BrowserAnnotationMutations } from "./useBrowserAnnotationMutations";

export interface BrowserNoteEditorApi {
  noteEditorRef: RefObject<HTMLDivElement | null>;
  noteEditorStyle: CSSProperties | undefined;
  dismissNoteEditor(): void;
  saveNote(): void;
  selectAnnotation(annotation: BrowserViewDraftAnnotation): void;
  openNoteEditor(annotation: BrowserViewDraftAnnotation): void;
  changeNote(value: string): void;
}

export function useBrowserNoteEditor(state: BrowserPaneState, drafts: BrowserDrafts, marks: BrowserAnnotationMutations): BrowserNoteEditorApi {
  const { frame, noteId, selectedId, noteValue, noteEditorDismissed, noteIdRef, draftRef, surfaceRef, frameRef, editorDirtyRef, associationOwner, associationOwnerRef, setNoteEditorDismissed, markEditorDirty, setNoteId, setNoteValue, setSelectedId, setMessage } = state;
  const { persistEditor } = drafts;
  const { persist } = marks;
  const noteEditorRef = useRef<HTMLDivElement>(null);
  const [noteEditorPosition, setNoteEditorPosition] = useState<{ left: number; top: number } | null>(null);
  const descriptor = frame?.descriptor;
  const dismissNoteEditor = (): void => {
    setNoteEditorDismissed(true);
    markEditorDirty();
    surfaceRef.current?.focus({ preventScroll: true });
  };
  const saveNote = (): void => {
    const editingNoteId = noteId;
    const annotation = draftRef.current?.annotations.find((candidate) => candidate.id === editingNoteId);
    if (!annotation) return;
    void persist({ ...annotation, comment: noteValue.trim() || null }).then((accepted) => {
      if (accepted && noteIdRef.current === editingNoteId) {
        setNoteId(null); setNoteValue(""); markEditorDirty();
        surfaceRef.current?.focus({ preventScroll: true });
      }
    });
  };
  const selectAnnotation = (annotation: BrowserViewDraftAnnotation): void => {
    if (!draftRef.current?.annotations.some((candidate) => candidate.id === annotation.id)) return;
    const select = () => {
      setSelectedId(annotation.id);
      if (noteId !== annotation.id) {
        setNoteId(annotation.id);
        setNoteValue((annotation.comment ?? "").slice(0, 4000));
        markEditorDirty();
      }
      setNoteEditorDismissed(false);
    };
    const previous = noteId ? draftRef.current?.annotations.find((candidate) => candidate.id === noteId) : null;
    const nextComment = noteValue.trim() || null;
    if (noteId !== annotation.id && (editorDirtyRef.current || (previous && (previous.comment ?? null) !== nextComment))) {
      const requestedGeneration = associationOwner.editorGeneration;
      void (async () => {
        try {
          if (previous && (previous.comment ?? null) !== nextComment) {
            if (!await persist({ ...previous, comment: nextComment })) return;
          } else await persistEditor(requestedGeneration);
          if (associationOwnerRef.current !== associationOwner || associationOwner.editorGeneration !== requestedGeneration
            || associationOwner.noteId !== noteId || !draftRef.current?.annotations.some((candidate) => candidate.id === annotation.id)) return;
          select();
        } catch (error) {
          setMessage(`Could not preserve the previous annotation note: ${errorMessage(error)}`);
        }
      })();
      return;
    }
    select();
  };
  const noteEditorStyle = noteEditorPosition ? { left: noteEditorPosition.left, top: noteEditorPosition.top } : undefined;
  useLayoutEffect(() => {
    if (!noteId || noteId !== selectedId || !descriptor) { setNoteEditorPosition(null); return; }
    const place = () => {
      const surface = surfaceRef.current;
      const annotation = draftRef.current?.annotations.find((candidate) => candidate.id === selectedId);
      const shown = frameRef.current?.descriptor;
      if (!surface || !annotation || !shown) return;
      const position = noteEditorPlacement(annotation, shown, surface.clientWidth, surface.clientHeight, noteEditorRef.current?.offsetHeight || 128);
      if (!position) return;
      setNoteEditorPosition((previous) => previous?.left === position.left && previous.top === position.top ? previous : position);
    };
    place();
    const observer = surfaceRef.current && typeof ResizeObserver !== "undefined" ? new ResizeObserver(place) : null;
    if (surfaceRef.current) observer?.observe(surfaceRef.current);
    if (noteEditorRef.current) observer?.observe(noteEditorRef.current);
    window.addEventListener("resize", place);
    return () => { observer?.disconnect(); window.removeEventListener("resize", place); };
  }, [descriptor, frame, noteId, selectedId]);
  useEffect(() => {
    if (!noteId || selectedId !== noteId || noteEditorDismissed) return;
    // Pointer-down selects the mark; focus after the matching pointer-up so
    // the SVG click cannot take focus back from the editor.
    const frame = window.requestAnimationFrame(() => {
      if (noteIdRef.current === noteId) noteEditorRef.current?.querySelector("textarea")?.focus({ preventScroll: true });
    });
    return () => window.cancelAnimationFrame(frame);
  }, [noteId, selectedId, noteEditorDismissed]);
  const openNoteEditor = (annotation: BrowserViewDraftAnnotation): void => {
    if (noteId !== annotation.id) setNoteValue(annotation.comment ?? "");
    setNoteId(annotation.id); setNoteEditorDismissed(false); markEditorDirty();
  };
  const changeNote = (value: string): void => { markEditorDirty(); setNoteValue(value.slice(0, 4000)); };
  return { noteEditorRef, noteEditorStyle, dismissNoteEditor, saveNote, selectAnnotation, openNoteEditor, changeNote };
}
