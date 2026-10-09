import type { CommentDraftActions } from "./CommentDrafts";
export function CommentBar({ actions, selectedLines, status: commentStatus }: {
  actions: CommentDraftActions; selectedLines: string | null;
  status: { count: number; canCreateLines: boolean; canCreateWholeFile: boolean };
}) {
  return (
            <footer className="context-comment-status" aria-label="Context comment shortcuts"><span>{selectedLines ?? ""}</span><span className="context-comment-status-actions"><button type="button" onClick={() => actions.createLines()} disabled={!commentStatus.canCreateLines} title={commentStatus.canCreateLines ? "Comment on selected lines (C)" : "Select source lines before commenting"}><kbd>C</kbd> comment</button><button type="button" onClick={actions.createWholeFile} disabled={!commentStatus.canCreateWholeFile} title={commentStatus.canCreateWholeFile ? "Comment on whole file (Shift+C)" : "Source is not ready"}><kbd>Shift+C</kbd> file</button><button type="button" onClick={() => actions.openOverview()} title="Open comments overview">{commentStatus.count} comments</button></span></footer>
  );
}
