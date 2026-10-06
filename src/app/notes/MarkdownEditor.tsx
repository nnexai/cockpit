import { createContext, useContext, useEffect, useRef, useState } from "react";
import { EditorState, StateEffect, type Extension } from "@codemirror/state";
import { EditorView, drawSelection, keymap, lineNumbers, placeholder } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags } from "@lezer/highlight";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { UiIcon } from "../UiIcon";

export const NotesEditorScope = createContext("");
const editorSessions = new Map<string, { state: EditorState; scrollTop: number; preview: boolean }>();

const markdownHighlightStyle = HighlightStyle.define([
  { tag: tags.heading, color: "var(--accent)", fontWeight: "600" },
  { tag: tags.link, color: "var(--accent)", textDecoration: "underline" },
  { tag: tags.monospace, color: "var(--done)" },
  { tag: tags.quote, color: "var(--text-secondary)" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.strong, fontWeight: "bold" },
  { tag: tags.strikethrough, textDecoration: "line-through" },
  { tag: tags.string, color: "var(--idle)" },
  { tag: tags.escape, color: "var(--working)" },
  { tag: tags.invalid, color: "var(--blocked)" },
  { tag: [tags.meta, tags.url, tags.contentSeparator, tags.comment, tags.labelName, tags.processingInstruction], color: "var(--text-muted)" },
]);

export function MarkdownPreview({ content }: { content: string }) {
  return <div className="notes-markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={{
    a: ({ children, href }) => <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>,
    img: ({ alt }) => <span className="notes-image-label">[Image: {alt || "image"}]</span>,
  }}>{content}</ReactMarkdown></div>;
}

export function MarkdownEditor({ value, onChange, label, draftKey = label, onSave, disabled = false, hint = "Write Markdown…" }: {
  value: string; onChange(value: string): void; label: string; draftKey?: string; onSave?: () => void; disabled?: boolean; hint?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const scope = useContext(NotesEditorScope);
  const sessionKey = `${scope}/${draftKey}`;
  const callbacks = useRef({ onChange, onSave });
  callbacks.current = { onChange, onSave };
  const [preview, setPreview] = useState(() => editorSessions.get(sessionKey)?.preview ?? false);
  const previewRef = useRef(preview);
  previewRef.current = preview;
  useEffect(() => {
    if (!host.current || preview) return;
    const extensions: Extension[] = [
      markdown(), history(), lineNumbers(), drawSelection(), syntaxHighlighting(markdownHighlightStyle), EditorView.lineWrapping,
      placeholder(hint), EditorView.contentAttributes.of({ "aria-label": label, role: "textbox", "aria-multiline": "true" }),
      EditorState.readOnly.of(disabled), EditorView.editable.of(!disabled),
      keymap.of([{ key: "Mod-s", run: () => { callbacks.current.onSave?.(); return true; } }, ...defaultKeymap, ...historyKeymap]),
      EditorView.domEventHandlers({ keydown: event => {
        if (event.repeat && (event.ctrlKey || event.metaKey) && (event.key.toLowerCase() === "s" || event.key === "Enter")) { event.preventDefault(); return true; }
        return false;
      } }),
      EditorView.updateListener.of(update => { if (update.docChanged) callbacks.current.onChange(update.state.doc.toString()); }),
      EditorView.theme({
        "&": { height: "100%", fontSize: "var(--font-size-sm)", color: "var(--text-primary)", backgroundColor: "transparent" },
        ".cm-scroller": { overflow: "auto", fontFamily: "var(--font-mono)" },
        ".cm-content": { padding: "16px 0", minHeight: "100px", caretColor: "var(--text-primary)" },
        ".cm-cursor, .cm-dropCursor": { borderLeft: "2px solid var(--text-primary)", marginLeft: "-1px" },
        "& > .cm-scroller > .cm-selectionLayer .cm-selectionBackground": { backgroundColor: "var(--select-fill)" },
        "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground": { backgroundColor: "color-mix(in srgb, var(--accent) 30%, transparent)" },
        ".cm-content ::selection": { backgroundColor: "var(--select-fill)" },
        ".cm-gutters": { backgroundColor: "transparent", border: "none", color: "var(--text-muted)" },
        ".cm-placeholder": { color: "var(--text-muted)" },
        "&.cm-focused": { outline: "none" },
      }, { dark: true }),
    ];
    const saved = editorSessions.get(sessionKey);
    let state = saved ? saved.state.update({ effects: StateEffect.reconfigure.of(extensions) }).state : EditorState.create({ doc: value, extensions });
    if (state.doc.toString() !== value) state = state.update({ changes: { from: 0, to: state.doc.length, insert: value } }).state;
    const editor = new EditorView({ parent: host.current, state });
    if (saved) editor.scrollDOM.scrollTop = saved.scrollTop;
    view.current = editor;
    return () => {
      editorSessions.set(sessionKey, { state: editor.state, scrollTop: editor.scrollDOM.scrollTop, preview: previewRef.current });
      if (editorSessions.size > 256) { const oldest = editorSessions.keys().next().value; if (oldest !== undefined) editorSessions.delete(oldest); }
      editor.destroy(); view.current = null;
    };
    // Value changes are applied to the existing editor below, preserving selection and history.
  }, [preview, label, disabled, hint, sessionKey]);
  useEffect(() => {
    const editor = view.current;
    if (editor && editor.state.doc.toString() !== value) editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: value } });
  }, [value]);
  return <div className="notes-editor" data-editor-label={label}>
    <div className="notes-editor-toolbar"><span>markdown</span><div className="notes-editor-modes" aria-label="Markdown display"><button type="button" aria-label="Source" title="Source" aria-pressed={!preview} onClick={() => { setPreview(false); const saved = editorSessions.get(sessionKey); if (saved) editorSessions.set(sessionKey, { ...saved, preview: false }); }}><UiIcon name="code" /></button><button type="button" aria-label="Preview" title="Preview" aria-pressed={preview} onClick={() => { setPreview(true); const saved = editorSessions.get(sessionKey); if (saved) editorSessions.set(sessionKey, { ...saved, preview: true }); }}><UiIcon name="eye" /></button></div></div>
    {preview ? <div className="notes-preview-scroll"><MarkdownPreview content={value} /></div> : <div className="notes-editor-host" ref={host} />}
  </div>;
}
