import { createContext, useContext, useEffect, useRef, useState } from "react";
import { EditorState, StateEffect, type Extension } from "@codemirror/state";
import { EditorView, keymap, lineNumbers, placeholder } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { syntaxHighlighting, defaultHighlightStyle } from "@codemirror/language";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

export const NotesEditorScope = createContext("");
const editorSessions = new Map<string, { state: EditorState; scrollTop: number; preview: boolean }>();

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
      markdown(), history(), lineNumbers(), syntaxHighlighting(defaultHighlightStyle), EditorView.lineWrapping,
      placeholder(hint), EditorView.contentAttributes.of({ "aria-label": label, role: "textbox", "aria-multiline": "true" }),
      EditorState.readOnly.of(disabled), EditorView.editable.of(!disabled),
      keymap.of([{ key: "Mod-s", run: () => { callbacks.current.onSave?.(); return true; } }, ...defaultKeymap, ...historyKeymap]),
      EditorView.domEventHandlers({ keydown: event => {
        if (event.repeat && (event.ctrlKey || event.metaKey) && (event.key.toLowerCase() === "s" || event.key === "Enter")) { event.preventDefault(); return true; }
        return false;
      } }),
      EditorView.updateListener.of(update => { if (update.docChanged) callbacks.current.onChange(update.state.doc.toString()); }),
      EditorView.theme({ "&": { height: "100%", fontSize: "13px", backgroundColor: "transparent" }, ".cm-scroller": { overflow: "auto", fontFamily: "'IBM Plex Mono', monospace" }, ".cm-content": { padding: "12px 0", minHeight: "100px" }, ".cm-gutters": { backgroundColor: "transparent", border: "none", color: "#727983" }, "&.cm-focused": { outline: "none" } }),
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
    <div className="notes-editor-toolbar"><span>Markdown</span><button type="button" aria-pressed={preview} onClick={() => { setPreview(!preview); const saved = editorSessions.get(sessionKey); if (saved) editorSessions.set(sessionKey, { ...saved, preview: !preview }); }}>{preview ? "Source" : "Preview"}</button></div>
    {preview ? <div className="notes-preview-scroll"><MarkdownPreview content={value} /></div> : <div className="notes-editor-host" ref={host} />}
  </div>;
}
