# Code guide

Use [CONTEXT](CONTEXT.md) for the mental model, [DECISIONS](DECISIONS.md) for current rules and [configuration](docs/configuration.md) for examples/defaults/precedence. This guide owns navigation, short call flows and development recipes. Historical runtime observations belong in [verification-log](docs/verification-log.md). The map below follows current source imports/declarations, not former monolith names.

## Where to change behavior

### Frontend shell, input and local layout

| Change | Current owner |
| --- | --- |
| Shell composition | `src/app/App.tsx`; `src/app/shell/Workbench.tsx`; `src/app/shell/WorkbenchContent.tsx` |
| Session hooks | `src/app/shell/useOrderedSession.ts`; `src/app/shell/useSessionStream.ts`; `src/app/shell/useSessionControl.ts` |
| Recovery and sidebar state | `src/app/shell/useSessionRecovery.ts`; `src/app/shell/useSidebarState.ts`; `src/app/shell/useWorkbenchUI.ts` |
| Workbench state/actions/input | `src/app/shell/useWorkbenchState.ts`; `src/app/shell/useWorkbenchActions.ts`; `src/app/shell/useWorkbenchInput.ts` |
| Workarea and canvas | `src/app/shell/useWorkarea.ts`; `src/app/shell/WorkbenchCanvas.tsx`; `src/app/shell/model.ts` |
| Viewer/widget/liaison shell coordination | `src/app/shell/useViewerSources.ts`; `src/app/shell/useWidgetWindowState.ts`; `src/app/shell/useLiaisonNavigation.ts` |
| Commands and tab strip | `src/app/shell/commands.ts`; `src/app/shell/CommandOverlay.tsx`; `src/app/shell/TabStrip.tsx` |
| Shell menus | `src/app/shell/ContextMenu.tsx`; `src/app/shell/ResourceContextMenu.tsx` |
| Shell dialogs/recovery | `src/app/shell/PaneDialogOverlay.tsx`; `src/app/shell/SessionDialogOverlay.tsx`; `src/app/shell/RecoveryPanel.tsx` |
| Compatibility notice | `src/app/shell/CompatibilityNotice.tsx` |
| Session ordering/focus/mutation | `src/app/session/sessionStore.ts`; `src/app/session/focusCoordinator.ts`; `src/app/session/mutationCoordinator.ts` |
| Local layout state and reconciliation | `src/app/layout/tabLayoutStore.ts`; `src/app/layout/reconcile.ts` |
| Local split geometry and canvas | `src/app/layout/splitTree.ts`; `src/app/layout/solveLayout.ts`; `src/app/layout/TabCanvas.tsx` |
| Viewer and Browser leaf lifecycle | `src/app/layout/viewerLifecycle.ts`; `src/app/layout/browserLifecycle.ts`; `src/app/layout/LeafHost.tsx` |
| Virtual leaves | `src/app/layout/FilesLeaf.tsx`; `src/app/layout/ReviewLeaf.tsx`; `src/app/layout/BrowserLeaf.tsx` |
| Terminal leaf and cleanup notices | `src/app/layout/TerminalLeaf.tsx`; `src/app/layout/BrowserCleanupNotices.tsx` |
| Sidebar presentation | `src/app/sidebar/`; `src/app/sidebar.css` |
| Git status/actions | `src/app/session/spaceGitStatus.ts`; `src/app/session/spaceGitActions.ts`; `src/app/sidebar/SpaceGitAction.tsx` |
| Running services (read-only) | `src/app/session/spaceServiceStatus.ts`; `src/app/sidebar/Spaces.tsx`; plugin-owned rows focus their live Herdr pane/tab, with no lifecycle controls |
| Keyboard registry and runtime aliases | `src/app/input/shortcuts.ts`; `src/app/input/herdrBindings.ts`; `src/app/input/keymap.ts` |
| Modal/file picker input | `src/app/input/modal.ts`; `src/app/input/fileIndexCache.ts`; `src/app/input/fileNavigation.ts` |
| Terminal and server popup | `src/app/TerminalPane.tsx`; `src/app/ServerPopup.tsx`; `src/app/terminal/cockpitTerminal.ts` |
| Terminal attachment/control | `src/app/terminal/useTerminalAttachment.ts`; `src/app/terminal/useTerminalControl.ts`; `src/app/terminal/paneState.ts` |
| Terminal frame/pointer/resize | `src/app/terminal/useTerminalFrameQueue.ts`; `src/app/terminal/useTerminalPointer.ts`; `src/app/terminal/useTerminalResize.ts` |
| Terminal input bindings | `src/app/terminal/useXtermInputBindings.ts`; `src/app/terminal/terminalInput.ts`; `src/app/terminal/terminalMouse.ts` |
| Terminal clipboard/theme | `src/app/terminal/terminalClipboard.ts`; `src/app/terminal/terminalTheme.ts` |

### Browser and source viewer frontend

| Change | Current owner |
| --- | --- |
| Browser facade/chrome/feedback | `src/app/browser/BrowserPane.tsx`; `src/app/browser/BrowserChrome.tsx`; `src/app/browser/BrowserFeedbackPanel.tsx` |
| Browser state/navigation | `src/app/browser/useBrowserPaneState.ts`; `src/app/browser/useBrowserNavigation.ts`; `src/app/browser/browserPaneModel.ts` |
| Browser view stream/commands | `src/app/browser/useBrowserViewStream.ts`; `src/app/browser/useBrowserViewCommand.ts` |
| Browser input | `src/app/browser/useBrowserInputQueue.ts`; `src/app/browser/useBrowserPageInput.ts`; `src/app/browser/useBrowserPointerInput.ts` |
| Browser drafts/editor/capture | `src/app/browser/useBrowserDrafts.ts`; `src/app/browser/useBrowserNoteEditor.ts`; `src/app/browser/useBrowserFeedbackCapture.ts` |
| Browser annotation/rendering | `src/app/browser/AnnotationControls.tsx`; `src/app/browser/browserCanvas.ts`; `src/app/browser/framePresenter.ts` |
| Browser annotation mutations | `src/app/browser/useBrowserAnnotationMutations.ts` |
| Browser transforms | `src/app/browser/transform.ts` |
| Viewer facade and frame | `src/app/context/ContextViewer.tsx`; `src/app/context/ContextViewerFrame.tsx`; `src/app/viewer/ViewerLayout.tsx` |
| Document and tree loading | `src/app/context/useDocumentLoader.ts`; `src/app/context/useDirectoryTree.ts`; `src/app/context/useFilePicker.ts` |
| Viewer files/search/links | `src/app/context/useViewerFiles.ts`; `src/app/context/useViewerSearch.ts`; `src/app/context/useDocumentLinks.ts` |
| Viewer comments/batches | `src/app/context/useViewerComments.ts`; `src/app/context/useCommentBatch.ts` |
| Document renderer ownership | `src/app/context/DocumentView.tsx`; `src/app/context/MarkdownView.tsx`; `src/app/context/HtmlPreview.tsx` |
| Diagram/image policy | `src/app/context/MermaidView.tsx`; `src/app/context/SafeImage.tsx`; `src/app/context/markdownPolicy.ts` |
| Source metadata/mapping | `src/app/context/providerDocument.ts`; `src/app/context/contextSource.ts`; `src/app/context/documentMetadata.ts` |
| Viewer mapping/state/links | `src/app/context/sourceLines.ts`; `src/app/context/viewerState.ts`; `src/app/context/linkResolver.ts` |
| Diagram isolation/highlight | `src/app/context/mermaidFrame.ts`; `src/app/viewer/highlight.ts` |
| Viewer file/tree/search presentation | `src/app/context/ContextDirectoryRows.tsx`; `src/app/context/ContextFilesView.tsx`; `src/app/context/ContextSearch.tsx` |
| Context resources and comment chrome | `src/app/context/ContextResources.tsx`; `src/app/context/CommentBar.tsx`; `src/app/context/CommentDrafts.tsx` |
| Comment editor/overview/paste | `src/app/context/CommentEditor.tsx`; `src/app/context/CommentOverview.tsx`; `src/app/context/CommentPasteControls.tsx` |
| Review facade | `src/app/review/ReviewPane.tsx`; `src/app/review/ReviewViewer.tsx` |

### Library and Notes frontend

| Change | Current owner |
| --- | --- |
| Library facade/toolbars | `src/app/library/LibraryView.tsx`; `src/app/library/LibraryToolbar.tsx`; `src/app/library/LibraryViewerView.tsx` |
| Library viewer source/operations | `src/app/library/useLibraryViewerController.ts`; `src/app/library/useLibraryViewerSource.ts`; `src/app/library/useLibraryViewerOperations.ts` |
| Library Space/dialog controllers | `src/app/library/useLibraryViewerSpace.ts`; `src/app/library/LibraryViewerDialogs.tsx`; `src/app/library/useLibraryOperation.ts` |
| Library tree/focus/rows | `src/app/library/LibraryTree.tsx`; `src/app/library/LibraryTreeFocus.ts`; `src/app/library/LibraryTreeRows.ts` |
| Library identity/details/shared preview | `src/app/library/LibraryItemHeader.tsx`; `src/app/library/LibraryDetails.tsx`; `src/app/context/DocumentView.tsx` |
| Library document/state | `src/app/library/LibraryDocumentView.tsx`; `src/app/library/libraryState.ts` |
| Library status/problems | `src/app/library/StatePill.tsx`; `src/app/library/ProviderMark.tsx`; `src/app/library/LibraryProblems.tsx` |
| Library confirmations/selection/report | `src/app/library/LibraryConfirmDialog.tsx`; `src/app/library/SpaceContextList.tsx`; `src/app/library/RefreshReport.tsx` |
| Library token UI | `src/app/library/ProviderCredentialsDialog.tsx`; `src/app/library/useProviderCredentials.tsx` |
| Library Add flow | `src/app/library/AddContextDialog.tsx`; `src/app/library/AddContextOperation.ts`; `src/app/library/AddContextSource.ts` |
| Library Add steps/clipboard | `src/app/library/AddContextSourceStep.tsx`; `src/app/library/AddContextOptionsStep.tsx`; `src/app/library/clipboard.ts` |
| Notes facade/source editor | `src/app/notes/NotesView.tsx`; `src/app/notes/useNotes.ts`; `src/app/notes/MarkdownEditor.tsx` |
| Notes task title/detail | `src/app/notes/TaskDetail.tsx`; `src/app/notes/TodoTitle.tsx` |
| Notes Kanban/drop validation | `src/app/notes/Kanban.tsx`; `src/app/notes/notesSensors.ts`; `src/app/notes/boardState.ts` |

### Supervisor frontend

| Change | Current owner |
| --- | --- |
| Liaison project-root lookup/start/reuse/resume/focus | `src/app/supervisor/useLiaison.ts`; `src/app/shell/useLiaisonNavigation.ts` |
| Scoped notification observer and start/resume actions | `src/app/supervisor/useSupervisor.ts` |
| Optional fullscreen Overview, shared task-history drill-down and conversation return | `src/app/supervisor/SupervisorView.tsx`; `src/app/shell/WorkbenchContent.tsx`; `src/app/shell/useWorkarea.ts` |
| Compact terminal task rail, task history and read-only board | `src/app/supervisor/SupervisorTasks.tsx`; `TaskHistoryDetail.tsx` (record disclosure); `taskHistory.ts` (selected-task evidence projection); `src/app/layout/LeafHost.tsx`; `src/app/layout/TerminalLeaf.tsx` |
| Task prerequisite graph and geometry | `src/app/supervisor/SupervisorGraph.tsx`; `src/app/supervisor/dependencies.ts`; `src/app/supervisor/dependencyLayout.ts` |
| Rail/overview presentation | `src/app/supervisor/supervisor.css` |

### Core services and persistent owners

| Change | Current owner |
| --- | --- |
| Configuration roots/limits | `crates/cockpit-core/src/config.rs`; `crates/cockpit-core/src/config/roots.rs`; `crates/cockpit-core/src/config/limits.rs` |
| Project facade/defaults/plan / unique automatic orchestration branches | `crates/cockpit-core/src/projects/mod.rs`; `crates/cockpit-core/src/projects/defaults.rs`; `crates/cockpit-core/src/projects/plan.rs` (`plan_orchestration`, shared private `plan_impl`; ordinary `plan` unchanged) |
| Project execute/reconcile/teardown | `crates/cockpit-core/src/projects/execute.rs`; `crates/cockpit-core/src/projects/reconcile.rs`; `crates/cockpit-core/src/projects/teardown.rs` |
| Accepted Git integration identity bridge | `crates/cockpit-core/src/project_teardown.rs` (`integrated_owned_head`); canonical source/target project keys bridge through recorded Git common-dir to exact owned raw repository/common-dir keys |
| Response-bound owned Space absence / foreign Git census | `crates/cockpit-core/src/projects/teardown.rs` (`inspect_owned_resource`, `SpaceIncarnation`, `captured_space_incarnation`, `fresh_owned_space_absence`, `response_bound_snapshot`, `current_physical_snapshot`); adapter `project_snapshot_at_endpoint`; snapshot/location `peer_process`, immutable retirement `herdr_peer` reuse `NativeProcessIdentity` |
| Adjacent immutable-retirement cleanup fence | `crates/cockpit-core/src/projects/teardown.rs` (`validate_cleanup_incarnation`); `orchestration/retirement.rs` (`server_fence`); empty AttemptDirectory inspection and adjacent rmdir require fresh exact original Retired identity, never newer attachment/scope backfill |
| Orchestration filesystem-only preparation / exact moved-Space binding | `crates/cockpit-core/src/projects/prepare.rs` (`prepare_owned_checkout`, `prepare_directory`, `bind_moved_space`); existing `StoredOperation`/`OwnedCheckoutJournal`; explicit primary `LibraryService` dependency |
| Proved-exit worker existing-checkout resume binding | `crates/cockpit-core/src/projects/resume.rs` (`bind_resumed_space`); new move/epoch operation in existing journal, immutable previous-owned/borrowed-None lineage, original FS/checkout metadata preserved |
| Private Directory attempt preparation | `crates/cockpit-core/src/projects/attempts.rs` (`prepare_owned_attempt_directory`); existing operation/creation journal, exact four-field `OwnedAttemptDirectoryIdentity`; primary `LibraryService`, no Git/source copies |
| Project store and repository discovery | `crates/cockpit-core/src/project_store.rs`; `crates/cockpit-core/src/repository_cache.rs`; `crates/cockpit-core/src/repositories.rs` |
| Viewer/context/index | `crates/cockpit-core/src/viewer.rs`; `crates/cockpit-core/src/context.rs`; `crates/cockpit-core/src/file_index_cache.rs` |
| Context media/search | `crates/cockpit-core/src/context_media.rs`; `crates/cockpit-core/src/context_search.rs` |
| Review facade/parse/source | `crates/cockpit-core/src/review/mod.rs`; `crates/cockpit-core/src/review/parse.rs`; `crates/cockpit-core/src/review/source.rs` |
| Review snapshot/cache/Git | `crates/cockpit-core/src/review/snapshot.rs`; `crates/cockpit-core/src/review/cache.rs`; `crates/cockpit-core/src/review/git.rs` |
| Review safe filesystem/process | `crates/cockpit-core/src/review/safe_fs.rs`; `crates/cockpit-core/src/process.rs` |
| Browser facade/service/CDP | `crates/cockpit-core/src/browser/mod.rs`; `crates/cockpit-core/src/browser/service.rs`; `crates/cockpit-core/src/browser/cdp.rs` |
| Browser process/receipt/cleanup | `crates/cockpit-core/src/browser/process.rs`; `crates/cockpit-core/src/browser/receipts.rs`; `crates/cockpit-core/src/browser/cleanup.rs` |
| Browser delivery/drafts | `crates/cockpit-core/src/browser/delivery.rs`; `crates/cockpit-core/src/browser/drafts.rs`; `crates/cockpit-core/src/browser/drafts/service.rs` |
| Ephemeral owner reset/feedback | `crates/cockpit-core/src/ephemeral.rs`; `crates/cockpit-core/src/browser_feedback.rs` |
| Git status/actions | `crates/cockpit-core/src/space_git.rs`; `crates/cockpit-core/src/space_git_action.rs` |
| Plugin service status | `crates/cockpit-core/src/space_services.rs`; reads `nnex.action-launcher`'s `persistent-panes.json`, drops dead Herdr IDs and probes loopback health targets only |
| Quota cache/services and display | `crates/cockpit-core/src/quota.rs`; `crates/cockpit-core/src/quota/`; `src/app/limits/` |
| OMP quota parsing | `crates/cockpit-core/src/quota/parse.rs` |
| Notes durable service/filesystem | `crates/cockpit-core/src/notes.rs`; `crates/cockpit-core/src/notes/registry.rs`; `crates/cockpit-core/src/notes/fs.rs` |
| Notes records | `crates/cockpit-core/src/notes/todos.rs`; `crates/cockpit-core/src/notes/decisions.rs`; `crates/cockpit-core/src/notes/comments.rs` |
| Comment delivery | `crates/cockpit-core/src/comments/` |
| Comment paste batch/payload send checks | `crates/cockpit-core/src/comments/paste.rs`; `crates/cockpit-core/src/comments/paste/send.rs` |
| Widget facade/target/store | `crates/cockpit-core/src/widget.rs`; `crates/cockpit-core/src/widget/target.rs`; `crates/cockpit-core/src/widget/store.rs` |
| Widget HTML/choice preflight | `crates/cockpit-core/src/widget/preflight.rs`; `crates/cockpit-core/src/widget/choices.rs`; `src/app/widgets/` |

### Library, sources and providers

| Change | Current owner |
| --- | --- |
| Library facade/read/store | `crates/cockpit-core/src/library.rs`; `crates/cockpit-core/src/library/reader.rs`; `crates/cockpit-core/src/library/store.rs` |
| Library filesystem inventory | `crates/cockpit-core/src/library/store/inventory.rs` |
| Library snapshot add/refresh helpers | `crates/cockpit-core/src/library/snapshot.rs` |
| Store publication and current-journal recovery | `crates/cockpit-core/src/library/store/publication.rs`; `crates/cockpit-core/src/library/store/recovery.rs` |
| Library operations/layout/refs | `crates/cockpit-core/src/library/operations.rs`; `crates/cockpit-core/src/library/layout.rs`; `crates/cockpit-core/src/library/refs.rs` |
| Library folder capture/Space selections | `crates/cockpit-core/src/library/folder.rs`; `crates/cockpit-core/src/library/folder_io.rs`; `crates/cockpit-core/src/library/space.rs` |
| Manual follow refresh and background sync | `crates/cockpit-core/src/library/follow.rs`; `crates/cockpit-core/src/library/jira_follow.rs`; `crates/cockpit-core/src/library/sync.rs` |
| Follow planner facade/discovery/drain | `crates/cockpit-core/src/library/follow_plan/mod.rs`; `crates/cockpit-core/src/library/follow_plan/discover.rs`; `crates/cockpit-core/src/library/follow_plan/drain.rs` |
| Follow planner provider branches | `crates/cockpit-core/src/library/follow_plan/confluence.rs`; `crates/cockpit-core/src/library/follow_plan/jira.rs` |
| Follow planner reasons/related/standalone | `crates/cockpit-core/src/library/follow_plan/reason.rs`; `crates/cockpit-core/src/library/follow_plan/related_due.rs`; `crates/cockpit-core/src/library/follow_plan/standalone.rs` |
| Follow planner cancellation | `crates/cockpit-core/src/library/follow_plan/cancel.rs` |
| Library related assets and attachment publication | `crates/cockpit-core/src/library/related.rs`; `crates/cockpit-core/src/library/attachments.rs` |
| Source facade/validation | `crates/cockpit-core/src/sources.rs`; `crates/cockpit-core/src/sources/validation.rs` |
| Source traversal/pacing/JQL | `crates/cockpit-core/src/sources/references.rs`; `crates/cockpit-core/src/sources/lane.rs`; `crates/cockpit-core/src/jira_query.rs` |
| Credentials and OS vault | `crates/cockpit-core/src/credentials.rs`; `crates/cockpit-secrets/src/lib.rs` |
| Provider registration/forge | `crates/cockpit-providers/src/lib.rs`; `crates/cockpit-providers/src/forge/mod.rs`; `crates/cockpit-providers/src/forge/gitlab.rs` |
| Forge adapters | `crates/cockpit-providers/src/github.rs`; `crates/cockpit-providers/src/gitlab.rs`; `crates/cockpit-providers/src/tea.rs` |
| Atlassian adapters/storage | `crates/cockpit-providers/src/jira.rs`; `crates/cockpit-providers/src/confluence.rs`; `crates/cockpit-providers/src/confluence_storage.rs` |
| Jira attachment validation/download | `crates/cockpit-providers/src/jira_attachments.rs`; `crates/cockpit-providers/src/jira_attachments/download.rs` |
| Shared HTTP/pacing | `crates/cockpit-providers/src/site_http.rs`; `crates/cockpit-providers/src/site_http/pacing.rs` |
| Jira wiki conversion | `crates/cockpit-providers/src/jira_wiki.rs` |

### Orchestration service, CLI and harness launch

| Change | Current owner |
| --- | --- |
| Service/caller identity and credentialed root-scoped read projection | `crates/cockpit-core/src/orchestration.rs` (`job_snapshot_saved`); `crates/cockpit-core/src/orchestration/caller.rs`; `crates/cockpit-core/src/orchestration/agent.rs` |
| Canonical tasks/store/assignment | `crates/cockpit-core/src/orchestration/tasks_md.rs`; `crates/cockpit-core/src/orchestration/store.rs`; `crates/cockpit-core/src/orchestration/assignments.rs` |
| Prerequisites/checklist parsing | `crates/cockpit-core/src/orchestration/dependencies.rs`; `crates/cockpit-core/src/orchestration/steps.rs`; `crates/cockpit-core/src/orchestration/steps/parser.rs` |
| Direct prerequisite accepted references / input binding | `crates/cockpit-core/src/orchestration/dependency_inputs.rs` (`accepted_dependency_run_ids`); CLI `cli_orchestration/mod.rs`/`output.rs` (`task inputs` compact projection) |
| Checklist marker layout and operation planning | `crates/cockpit-core/src/orchestration/steps/layout.rs`; `crates/cockpit-core/src/orchestration/steps/planning.rs` |
| Action dispatcher/mutation facade/tasks/runs | `crates/cockpit-core/src/orchestration/mutate/mod.rs`; `crates/cockpit-core/src/orchestration/mutate/tasks.rs`; `crates/cockpit-core/src/orchestration/mutate/runs.rs` |
| Atomic activation/unassign/replacement generation fencing | `crates/cockpit-core/src/orchestration/mutate/ownership.rs`; `crates/cockpit-core/src/orchestration/mutate/runs.rs` |
| Durable schedules and locked attempt creation | `crates/cockpit-core/src/orchestration/mutate/runs.rs` (`create_attempt`, `schedule_candidates`, `scheduled_start`, `worker_replace`) |
| Role credentials/same-worker scope | `crates/cockpit-core/src/orchestration/credentials.rs`; `crates/cockpit-core/src/orchestration/scope_intents.rs`; `crates/cockpit-core/src/orchestration/mutate/tasks.rs` |
| Source acceptance/integration/send-back | `crates/cockpit-core/src/orchestration/mutate/intents.rs`; `crates/cockpit-core/src/orchestration/mutate/integration.rs` (live Result claim versus immutable accepted source, actual target catalog and proof CAS); `crates/cockpit-core/src/orchestration/mutate/grants.rs` (send-back) |
| Scheduler/placement/input-aware dispatch/Herdr/escalation | `crates/cockpit-core/src/orchestration/dispatch.rs` (`resolve_schedule_target`, `render_run_bootstrap`); `crates/cockpit-core/src/orchestration/herdr.rs`; `crates/cockpit-core/src/orchestration/escalation.rs`; exact close capability guard in `crates/cockpit-herdr/src/cli/orchestration.rs` |
| Messages/inbox/reports | `crates/cockpit-core/src/orchestration/messages.rs`; `crates/cockpit-core/src/orchestration/messages/inbox.rs`; `crates/cockpit-core/src/orchestration/messages/report.rs` |
| Ownership projection/routing | `crates/cockpit-core/src/orchestration/projection.rs`; `crates/cockpit-core/src/orchestration/routing.rs` |
| Event-driven cleanup/capture/ref watches/blockers | `crates/cockpit-core/src/orchestration/cleanup.rs` (`capture_owned_resources(projects, herdr)`, `cleanup_queue`, `cleanup_ref_watch_paths`, `cleanup_owned_resources`); shared service `watch_cleanup_refs` |
| Accepted integration reconciliation | `crates/cockpit-core/src/orchestration/mutate/integration.rs` (`reconcile_accepted_integrations`); actual passed-check `IntegrationRecord` path retained |
| Retirement/resource safety | `crates/cockpit-core/src/orchestration/retire.rs` (`retirement_queue`, `retire_run`); `retirement.rs`; `fixture_resources.rs`; unchanged `resource_queue.rs` |
| Runtime owner/Herdr event waiting and literal launch | `crates/cockpit-host/src/orchestration_runtime.rs`; `crates/cockpit-herdr/src/cli/orchestration.rs` |
| Harness-neutral owned process scope/helper | `crates/cockpit-core/src/process_scope.rs` (`scope_launch`, `capture`, `observe`, `stop_exact`); `crates/cockpit-host/src/bin/cockpit/contain.rs`; registration in `bin/cockpit.rs` |
| Per-incarnation placement-gated agent execution | `crates/cockpit-host/src/bin/cockpit/agent_exec.rs`; core `process_scope/latch.rs` (`agent_launch`, pure `latch_ready`, `latch_startup` / `ScopeStartup`, `release_latch`, reexported by `process_scope`); protocol `AgentLaunchReceipt` (`latch`, `nonce`, `latch_device`, `latch_inode`) |
| Scope receipt/seal/release proof | protocol `orchestration.rs` (`OwnedProcessScope`, `ScopeKind`, `attachment.process_scope`), `projects.rs` (`ScopeMode`); core `orchestration/store.rs`, `claims.rs`, `project_identity.rs`; dispatch `scope_unit`/TabReceipt capture |
| Proved-exit worker epoch/consumer cutover | `crates/cockpit-core/src/orchestration/claims.rs` (`revoke_and_reserve_replacement`); one validated CAS discharges only the exact exited incarnation's active consumer reference, preserves foreign consumers/resource identity/history; RootMainRecovery grants no consumer discharge |
| Directory attempt identity / typed replacement | protocol `orchestration.rs` (`OwnedAttemptDirectoryIdentity`: `creation_operation_id`, `root`, `device`, `inode`; `OwnedAttemptDirectory`, `IsolatedDirectory`); core `ReplacementSource` (`GitHead`, `Directory`), immutable existing project operation/creation journal |
| Accepted-worker sealed scope cleanup / completion fences | `crates/cockpit-core/src/orchestration/cleanup/scopes.rs` (`cleanup_worker_scopes`, `stop_output_proven`, `stop_output_preflight`, `canonical_stop_eligible`, private `seal_scope_release`/`release_scope_claim`); `project_identity.rs` (`checkout_consumers_clear`) |
| Service-issued owned-Git consumer proof | `crates/cockpit-core/src/orchestration/cleanup.rs` (`OwnedConsumerProof`, private `owned_consumer_proof` / `validate_consumer_release`); `crates/cockpit-core/src/projects/teardown.rs` and `crates/cockpit-core/src/project_teardown.rs` validate fresh proof at inspection/adjacent apply |
| Replacement retirement authority | protocol `RetirementTrigger::Replacement`, `ReplacementRetirementAuthority`, optional Result/task-revision pins; core `orchestration/retirement.rs`, `retire.rs`, `cleanup/scopes.rs` |
| Optional embedded per-launch OMP hook | `integrations/omp-hook/cockpit-hook.ts`; `crates/cockpit-core/src/orchestration/omp_hook.rs` (`materialize`) |
| Orchestration contract tests | `crates/cockpit-core/src/orchestration/service_tests.rs` (shared fixture); `crates/cockpit-core/src/orchestration/service_tests/` (domain suites); core `orchestration/{dispatch,messages,credentials,cleanup}/tests.rs`; `crates/cockpit-host/src/orchestration_runtime/tests.rs` |
| Agent CLI facade/args/caller | `crates/cockpit-host/src/cli_orchestration/mod.rs`; `crates/cockpit-host/src/cli_orchestration/args.rs`; `crates/cockpit-host/src/cli_orchestration/caller.rs` |
| Agent CLI activation/replacement/internal inbox capture | `crates/cockpit-host/src/cli_orchestration/context.rs` (`replacement_action`); `wait.rs` (`inbox_next`); `output.rs` (`capture`) |
| CLI binary/Notes/skills | `crates/cockpit-host/src/bin/cockpit.rs`; `crates/cockpit-host/src/bin/cockpit/notes.rs`; `crates/cockpit-host/src/bin/cockpit/skills.rs` |
| CLI endpoint resolution/portable guides | `crates/cockpit-host/src/bin/cockpit/endpoint.rs`; `integrations/agent-skills/` |
| Bounded cleanup/integration payload guidance | `run cleanup-finish --help`, `run integration-record --help` expose complete field shapes/examples from CLI `mod.rs`; authoritative `CleanupFinish`/`IntegrationRecord` and nested types in `crates/cockpit-protocol/src/orchestration.rs`. Samples are parser illustrations, never production evidence. |
| Receipt-owned native installer | `scripts/install-native.py` |

### Protocol, clients and host adapters

| Change | Current owner |
| --- | --- |
| Rust wire domains and exporter entry | `crates/cockpit-protocol/src/lib.rs`; `crates/cockpit-protocol/src/typescript.rs` |
| Rust descriptor parsing | `crates/cockpit-protocol/src/typescript/wire_model.rs`; `crates/cockpit-protocol/src/typescript/wire_model_attrs.rs`; `crates/cockpit-protocol/src/typescript/wire_model_source.rs` |
| Descriptor-derived validators | `crates/cockpit-protocol/src/typescript/validators.rs`; `crates/cockpit-protocol/src/typescript/validators_index.rs`; `crates/cockpit-protocol/src/typescript/validate_runtime.ts` |
| Exporter CLI/generated TypeScript | `crates/cockpit-protocol/src/bin/export-typescript.rs`; `src/protocol/generated/v1.ts`; `src/protocol/generated/validate/` |
| Typed requests and transport contract | `src/client/CockpitClient.ts`; `src/client/operations.ts`; `src/client/wire.ts` |
| Client policy and transport adapters | `src/client/clientPolicy.ts`; `src/client/browser.ts`; `src/client/native.ts` |
| Shared browser decoder and ordered streams | `src/client/browserViewDecoder.ts`; `src/client/streamOrder.ts`; `src/client/widgetTransport.ts` |
| Client domain decoders | `src/client/orchestrationProtocol.ts`; `src/client/libraryProtocol.ts`; `src/client/credentialProtocol.ts` |
| Client Notes/widget/quota decoders | `src/client/notesProtocol.ts`; `src/client/widgetProtocol.ts`; `src/client/quotaProtocol.ts` |
| Client Context/media/search decoders | `src/client/contextProtocol.ts`; `src/client/contextMediaProtocol.ts`; `src/client/contextSearchProtocol.ts` |
| Client review/comments decoders | `src/client/reviewProtocol.ts`; `src/client/commentProtocol.ts`; `src/client/commentPasteProtocol.ts` |
| Client project/teardown decoders | `src/client/projectProtocol.ts`; `src/client/projectTeardownProtocol.ts` |
| Shared operation declaration/registry | `crates/cockpit-host/src/transport/operations.rs`; `crates/cockpit-host/src/transport/registry.rs` |
| Shared typed session/project/viewer handlers | `crates/cockpit-host/src/transport/operations/session.rs`; `crates/cockpit-host/src/transport/operations/projects.rs`; `crates/cockpit-host/src/transport/operations/viewer.rs` |
| Shared typed library/credentials/Notes handlers | `crates/cockpit-host/src/transport/operations/library.rs`; `crates/cockpit-host/src/transport/operations/credentials.rs`; `crates/cockpit-host/src/transport/operations/notes.rs` |
| Shared typed orchestration/review/Context handlers | `crates/cockpit-host/src/transport/operations/orchestration.rs`; `crates/cockpit-host/src/transport/operations/review.rs`; `crates/cockpit-host/src/transport/operations/context.rs` |
| Shared typed Browser/comment/widget handlers | `crates/cockpit-host/src/transport/operations/browser.rs`; `crates/cockpit-host/src/transport/operations/comments.rs`; `crates/cockpit-host/src/transport/operations/widgets.rs` |
| HTTP route adapter | `crates/cockpit-host/src/server.rs`; `crates/cockpit-host/src/server/operations.rs` |
| HTTP origin/input/errors/limits | `crates/cockpit-host/src/transport/guard.rs`; `crates/cockpit-host/src/transport/http_input.rs`; `crates/cockpit-host/src/transport/error.rs` |
| Transport bounds/native input | `crates/cockpit-host/src/transport/limits.rs`; `crates/cockpit-host/src/transport/native_input.rs` |
| Shared host shutdown policy | `crates/cockpit-host/src/transport/shutdown.rs` |
| Session reducer/pump and HTTP socket | `crates/cockpit-host/src/transport/session_stream.rs`; `crates/cockpit-host/src/server/session_socket.rs` |
| Terminal stream and widget HTTP stream | `crates/cockpit-host/src/transport/terminal.rs`; `crates/cockpit-host/src/server/widgets.rs` |
| Browser binary frame/relay | `crates/cockpit-host/src/transport/browser_frame.rs`; `crates/cockpit-host/src/transport/browser_relay.rs`; `crates/cockpit-host/src/browser_view.rs` |
| Native command/startup facade | `src-tauri/src/lib.rs`; `src-tauri/src/commands.rs`; `src-tauri/src/startup.rs` |
| Native streams/clipboard/window lifecycle | `src-tauri/src/streams.rs`; `src-tauri/src/clipboard.rs`; `src-tauri/src/main.rs` |
| Browser owner/helper process | `crates/cockpit-host/src/browser_runtime.rs`; `crates/cockpit-host/src/browser_helper.rs` |
| Chromium capture and page UA policy | `browser-runtime/browser-helper.mjs`; `browser-runtime/browser-user-agent.cjs` |
| Herdr adapters and terminal/shell wire | `crates/cockpit-herdr/src/cli/`; `crates/cockpit-herdr/src/terminal_wire.rs`; `crates/cockpit-herdr/src/shell_wire.rs` |
| Herdr command implementation | `crates/cockpit-herdr/src/cli/shell.rs`; `crates/cockpit-herdr/src/cli/operations.rs` |

### Protocol domain declarations and renderers

| Change | Current owner |
| --- | --- |
| browser wire declaration/rendering | `crates/cockpit-protocol/src/browser.rs`; `crates/cockpit-protocol/src/typescript/browser.rs` |
| browser_feedback wire declaration/rendering | `crates/cockpit-protocol/src/browser_feedback.rs`; `crates/cockpit-protocol/src/typescript/browser_feedback.rs` |
| browser_view wire declaration/rendering | `crates/cockpit-protocol/src/browser_view.rs`; `crates/cockpit-protocol/src/typescript/browser_view.rs` |
| comment_paste wire declaration/rendering | `crates/cockpit-protocol/src/comment_paste.rs`; `crates/cockpit-protocol/src/typescript/comment_paste.rs` |
| comments wire declaration/rendering | `crates/cockpit-protocol/src/comments.rs`; `crates/cockpit-protocol/src/typescript/comments.rs` |
| context wire declaration/rendering | `crates/cockpit-protocol/src/context.rs`; `crates/cockpit-protocol/src/typescript/context.rs` |
| context_media wire declaration/rendering | `crates/cockpit-protocol/src/context_media.rs`; `crates/cockpit-protocol/src/typescript/context_media.rs` |
| context_search wire declaration/rendering | `crates/cockpit-protocol/src/context_search.rs`; `crates/cockpit-protocol/src/typescript/context_search.rs` |
| credentials wire declaration/rendering | `crates/cockpit-protocol/src/credentials.rs`; `crates/cockpit-protocol/src/typescript/credentials.rs` |
| herdr_shell wire declaration/rendering | `crates/cockpit-protocol/src/herdr_shell.rs`; `crates/cockpit-protocol/src/typescript/herdr_shell.rs` |
| library wire declaration/rendering | `crates/cockpit-protocol/src/library.rs`; `crates/cockpit-protocol/src/typescript/library.rs` |
| notes wire declaration/rendering | `crates/cockpit-protocol/src/notes.rs`; `crates/cockpit-protocol/src/typescript/notes.rs` |
| orchestration wire declaration/rendering | `crates/cockpit-protocol/src/orchestration.rs`; `crates/cockpit-protocol/src/typescript/orchestration.rs` |
| project_defaults wire declaration/rendering | `crates/cockpit-protocol/src/project_defaults.rs`; `crates/cockpit-protocol/src/typescript/project_defaults.rs` |
| project_teardown wire declaration/rendering | `crates/cockpit-protocol/src/project_teardown.rs`; `crates/cockpit-protocol/src/typescript/project_teardown.rs` |
| projects wire declaration/rendering | `crates/cockpit-protocol/src/projects.rs`; `crates/cockpit-protocol/src/typescript/projects.rs` |
| quota wire declaration/rendering | `crates/cockpit-protocol/src/quota.rs`; `crates/cockpit-protocol/src/typescript/quota.rs` |
| review wire declaration/rendering | `crates/cockpit-protocol/src/review.rs`; `crates/cockpit-protocol/src/typescript/review.rs` |
| sources wire declaration/rendering | `crates/cockpit-protocol/src/sources.rs`; `crates/cockpit-protocol/src/typescript/sources.rs` |
| v1 wire declaration/rendering | `crates/cockpit-protocol/src/v1.rs`; `crates/cockpit-protocol/src/typescript/v1.rs` |
| viewer wire declaration/rendering | `crates/cockpit-protocol/src/viewer.rs`; `crates/cockpit-protocol/src/typescript/viewer.rs` |
| widget wire declaration/rendering | `crates/cockpit-protocol/src/widget.rs`; `crates/cockpit-protocol/src/typescript/widget.rs` |

### Style ownership

| Change | Current owner |
| --- | --- |
| Ordered style facade and retained viewer/sidebar | `src/app/styles.css`; `src/app/viewer.css`; `src/app/sidebar.css` |
| Shared tokens/shell/tabs | `src/app/styles/tokens.css`; `src/app/styles/shell.css`; `src/app/styles/tabs.css` |
| Terminal/overlays | `src/app/styles/terminal.css`; `src/app/styles/overlays.css` |
| Feature styles | `src/app/browser/browser.css`; `src/app/context/context.css`; `src/app/context/comments.css` |
| Library/layout/Notes styles | `src/app/library/library.css`; `src/app/layout/tabCanvas.css`; `src/app/notes/notes.css` |
| Project/review/limits styles | `src/app/projects/setup.css`; `src/app/projects/taskSetup.css`; `src/app/review/review.css` |
| Shared errors/limits/file navigation | `src/app/errorSlot.css`; `src/app/limits/limits.css`; `src/app/input/fileNavigation.css` |

## Short call flows

### Requests, streams and native composition

Rust DTOs → descriptor-derived domain renderers/validators → generated TypeScript → `src/client/operations.ts` argument/response mapping → browser HTTP or native invoke adapter → shared typed host handler → core service → identity-checked response. Host registry and stable typed errors are shared; adapters retain their transport origin, serialization, lifecycle and cancellation responsibilities.

Four stream methods remain transport-local: `subscribeSession`, `openTerminal`, `subscribeWidgets` and `openBrowserView`. `src/client/browserViewDecoder.ts` shares browser event/frame identity and ordering decode only; release, cancellation and lifecycle stay in `src/client/browser.ts` and `src/client/native.ts`. Session reducer/pump, terminal streams and bounded Browser relay have separate host owners in the map above.

### Supervisor tasks, execution and messages

Agent CLI → private attempt credential plus run/root/attachment epoch and pinned task generation → `Actor::Agent`; explicit local `--operator` CLI → `Actor::Operator` with named root/task/run targets, ignoring stale ambient job metadata. Both paths use canonical Markdown/store mutations → notifications/snapshot/inbox → shared host operation → frontend decoder. Supplied actual agent conversation metadata is recorded/revalidated; ordinary coordination needs no identity-hunt turn. Process labels, pane presence and focus grant no semantic authority; physical effects retain their separate fresh target proofs.

Credentialed saved reads authenticate under the store lock and project only the caller's root; workers receive their own inbox bodies, with same-root task/run records for collaboration. Launch capability bytes are redacted from agent projections without changing durable launch state or generic operator/UI snapshots. Working task content writes by the root use the existing scope journal/instruction path; checklist writes retain ordinary canonical CAS. Neither grants sibling worker writes nor changes accepted-task immutability.

`run context` claims pending generations. Schedules survive root credential/attachment rotation via immutable receipt/exact root/current binding checks. Atomic replacement preserves old work/claims and uses typed GitHead isolated worktree or Directory private no-copy attempt with original project/context. Same-checkout reuse requires precise reboot or exact drained scope plus registered consumer proof/fresh Herdr inventory. Directory outputs remain retained: acceptance/NotApplicable never permits recursive deletion, and only an accepted retired drained EMPTY private attempt directory can be removed after durable output publication/move.

Input-aware scheduling freezes the base SHA and requires same-repository accepted Code output containment, either original head/commits or exact validated Integrated target/mapping with passed target checks. Missing input stays queued with deduplicated `dependency_input_not_materialized`; Git-ref wakes persist after old resource removal. Publication rechecks accepted Result/task/source/integration/candidate identities. Cross-repository/conceptual/manual/no-source edges differ. NonCode references stay read-only/retained without inferred publication, copies or deletion; no second task store/checkpoint table or per-stage approval is added.

For ExistingSpace/Open, preserve the candidate target/HEAD: fresh post-setup clean HEAD/input proof reserves `base_commit`, then immediate pre-latch-release reinspection and Domain release CAS fence that SHA/current accepted inputs.

`task inputs TASK --json` or worker `task inputs --self --json` reads direct prerequisites/current scope and valid accepted Result/source references from existing stores. Invalid/missing prerequisites retain diagnostics rather than guessed accepted data; artifact references are untrusted, with no path/hash inference or automatic publication.

Checklist stages and canonical prerequisites define work boundaries, not per-step model approvals. Working scope changes use dual CAS and one same-worker instruction. A succeeded Result ties SourceOutput to the exact Result ID and current canonical revision; source acceptance, target Integration and Cleanup remain independent. Cleanup reevaluates relevant state/pane/ref events, preserves unsafe resources and sends deduplicated blockers to the liaison. Adapted integration still uses actual target passed-check evidence.

Harness-neutral DirectProcess launch → optional `process_scope` wrapper → exact membership/InvocationID/cgroup receipt. `scope_launch` receives `kind: &RunKind` for worker/root naming; descriptions grant no identity. Cleanup seals/journals before narrowly eligible exact accepted-worker scope stop, independent of target integration. Fresh drain, registered descendant/resource proof and Herdr checkout-consumer inventory support release, then non-force Git cleanup with verified integration or fresh accepted-head containment in the actual root checkout. Old/uncontained runs retain reboot-based proof; no retrofit/arbitrary scope stop. D-Bus services, Docker and self-migration can escape and need separate registered consumer proof; this is not a sandbox.

Idle waits register notifications before reads and coalesce/resync without a full-snapshot tick. Extension-free/hook-disabled `inbox next --wait 600 --deliverable --json` waits/pulls/fsyncs/captures without an ACK model turn. An effective materialized hook marks `COCKPIT_INBOX_HOOK=1` and owns that consumer; do not add manual long waits/capture. Its public busy `sendUserMessage` uses safe-boundary `deliverAs: "steer"` without aborting tools, killing writers or submitting drafts. Queue/capture proves neither a question Answer nor model processing/first-response latency.

The optional OMP hook spawns registration/wait CLI children in a new POSIX session/process group (`detached: true`) with ignored stdin and captured output, while retaining inherited owned-cgroup membership. Exact AbortSignal cancellation and no `unref` keep lifecycle ownership; detached waiters do not become terminal foreground consumers or grant writer-exit proof. A live foreground/cgroup smoke, not the option alone, establishes the affected runtime behavior.

Start reuses/reattaches the project root. Literal harness configuration, whole-argument `{prompt}`, `{hook_path}` and optional `{session_id}` resume rules live in [configuration](docs/configuration.md#supervisor-launch-and-attempt-authority). Launch acknowledgement, observed physical readiness, credential activation and first model response are distinct. The real liaison terminal owns conversation; its compact rail and optional fullscreen board/prerequisite graph are observational. Unknown effects and old consumers remain protected; this navigation grants no persistent-session repair authority. Additive schema-4 metadata needs no converter; incompatible schemas preserve their bytes on rejection.

### Library follow refresh, sources and token storage

Library refresh request → shared typed handler → `LibraryService` → `crates/cockpit-core/src/library/follow.rs` and its `crates/cockpit-core/src/library/follow_plan/mod.rs` planner, or Jira manual refresh in `crates/cockpit-core/src/library/jira_follow.rs` → provider discovery/read → source validation → journaled store publication/ref merge → listing generation → Library controller/preview. Background scheduling is separately owned by `crates/cockpit-core/src/library/sync.rs`.

Filesystem inventory is `crates/cockpit-core/src/library/store/inventory.rs`; fetched asset validation is `crates/cockpit-core/src/sources/validation.rs`. Source traversal fetches; Library owns persistence/inclusion reasons. Setup passes validated results into project start and can reuse committed IDs during recovery. Viewer Context composes Library through its builder, avoiding a ProjectService cycle.

Token entry → `src/app/library/ProviderCredentialsDialog.tsx` → typed credential operation → `crates/cockpit-core/src/credentials.rs` → `crates/cockpit-secrets/src/lib.rs` → the running user's Linux Secret Service vault. [configuration](docs/configuration.md#provider-token-storage-and-entry-points) lists actual singular item/menu entries and plural Commands entry, authentication defaults, timeout/cache and macOS-unverified status. [DECISIONS](DECISIONS.md) owns credential/redirect/attachment/publication safety.

### Terminal, popup, local layout and viewers

Herdr snapshot/stream → session ordering → authoritative membership/focus reconciliation → run-local split tree → stable `src/app/layout/TabCanvas.tsx` leaves → control-attached terminal hooks. Herdr owns membership and real focus; Cockpit owns painted placement. Files/Review start from a same-tab real source terminal, pin viewer/binding/root identity and keep per-source frontend state rather than retargeting on `cd` or source closure.

Herdr client-shell projection → runtime shortcut aliases → confirmed-focus command invocation → singleton popup metadata → `src/app/ServerPopup.tsx` reusing terminal attachment. Decoder provenance, input gating, disconnected retention, focus return and the active 120×40 shell-subscription resize caveat are specified in [DECISIONS](DECISIONS.md), not inferred from local layout geometry.

### Browser and trusted widgets

Tab Browser action → core tab association/service → private owner runtime → supervised Node/CDP helper attached to the CLI-managed Chromium session → bounded relay → transport-local stream → shared decoder → Browser hooks/frame presenter. `browser-runtime/browser-user-agent.cjs` owns the narrow running-version HeadlessChrome substitution policy; it is not general anti-bot/media parity. Fresh-open/default URL and prerequisites are in [configuration](docs/configuration.md); lifetime/reset/cleanup/delivery guards are in [DECISIONS](DECISIONS.md).

Widget CLI show → bounded file/stdin/choices preflight → fresh source/target authorization → owner-memory widget store → ordered summaries/content/selection streams → tab dock → opaque script-only iframe bridge → validated retained selection. Widgets do not use Chromium capture. Limits, source/revision/nonce checks, fragment anchors, focus, tombstones and selection uncertainty remain in [DECISIONS](DECISIONS.md).

### Durable Notes and portable guides

Notes request → Origin-checked HTTP or native shared operation (standalone CLI calls core directly) → target authorization/registry → bounded durable Markdown/CAS publication → Notes response decoder → UUID-pinned hooks/drafts → editor or task detail. Opaque change tokens are not CAS revisions. [configuration](docs/configuration.md#durable-space-notes) owns root/UUID recipes; [DECISIONS](DECISIONS.md) owns external-editor limits, read-before-retry, CodeMirror caret/focus and real dnd-kit listener/drop semantics.

Use `cockpit-cli notes <area> --help` for payload/revision sources and `cockpit-cli task|run|inbox|route --help` for caller/lifecycle contracts. `run context` is the orchestration entry; the separate `cockpit-cli context --current` command reads Space/Library context. `cockpit-cli library add` saves and selects for the current Space through the existing Library add-with-target operation; see `cockpit-cli library add --help` for current/explicit targeting and input options. `cockpit-cli skills list` discovers portable guides; `cockpit-cli skills show NAME` reads without installation. Installation targeting, collision/refusal, serialized writes and same-UID race limits remain in [DECISIONS](DECISIONS.md).

## Development and verification recipes

`bun run browser` builds and serves the browser app at `http://127.0.0.1:4173`; append flags such as `--herdr-session my-session`. `bun run tauri:dev` starts native development. Native installation is a separate action documented in [native-install](docs/native-install.md).

For focused frontend changes use `bun run typecheck` and `bun run test -- <affected-test-file>`; for Rust use the affected package/test filter. At an integration boundary:

```sh
cargo run -q -p cockpit-protocol --bin export-typescript -- --write src/protocol/generated/v1.ts
bun run typecheck
bun run test
cargo test --workspace --exclude cockpit-tauri
cargo check -p cockpit-tauri
bun run build
```

Use the repository Rust toolchain and pinned Bun dependencies, with `cargo fmt --check` on final Rust scope. Generate TypeScript from Rust DTOs, never a second handwritten schema. Runtime failures need a reproduction through the actual user action and authoritative result, not just a mocked success. Native drag/drop, Linux app-paintable and frame-cadence constraints are in [DECISIONS](DECISIONS.md).

For focused HTTP pacing scenarios use `cargo test -p cockpit-providers site_http::pacing::tests:: -- --test-threads=1`. Tests specify `BackgroundPolicy`, fill only configured in-flight capacity and bound acquisition waits. Learned spacing cannot relax stricter background policy; see [DECISIONS](DECISIONS.md).

### Disposable UI smoke

Use the owned disposable fixture recipe, never the default session or manual gateway:

```sh
ROOT=$(python3 scripts/verify/ui_polish_runtime.py start)
GATEWAY_URL=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["gateway_url"])' "$ROOT/runtime.json")
# Point browser/native clients at "$GATEWAY_URL" and the owned session/configuration.
# Preserve evidence before stopping only the fixture's recorded resources.
python3 scripts/verify/ui_polish_runtime.py stop "$ROOT"
```

`scripts/verify/ui_polish_runtime.py` creates isolated runtime resources; `scripts/verify/resource_guard.py` checks executable/session/socket/ownership before Herdr commands. Both clients must address the same owned fixture. These are future-verification commands, not a claim that this documentation change exercised the product.

`start` prints only the root after its own gateway serves HTTP. `runtime.json` records `gateway_url` as the exact loopback origin without a trailing slash, suitable for the browser URL and `Origin` header; `fixture_url` instead addresses the separate annotation test page. Startup failure exits nonzero, retains the root and logs, and prints the explicit `stop` command without automatically stopping any process.

### Changed-scope quality

`bun run quality:probe` inspects pinned providers; `bun run quality:report --base <review-base>` reports staged/unstaged/untracked scope; `bun run quality:gate --base <review-base> --strict` applies the strict gate. Missing providers are inconclusive (exit 2), never a pass. Ignored reports are under `quality/reports/`; [quality/README.md](quality/README.md) owns inputs, baseline and exception rules.

Manual source-targeted mutation is opt-in: `bun run quality:mutation -- rust|ts --file <source>`. Ordinary tests, the gate and CI never invoke it. Stage only owned paths and inspect the staged diff before committing; installation, publication and user-session changes are separate actions. Authoritative capability, canonical-source and exact-preview paste boundaries remain in [DECISIONS](DECISIONS.md).

### Scenario selection by changed contract

These are verification recipes, not exercised results. Select affected checks and a real affected-path smoke; [verification-log](docs/verification-log.md) preserves prior observations and limitations.

| Contract | Useful scenarios |
| --- | --- |
| Public protocol | Protocol tests and generated-file check |
| Session ordering | Ordering corpus and reducer tests |
| Placement/membership/focus | Tree/focus/creation transitions, live membership changes and disposable-runtime smoke |
| Sidebar | Herdr ordering, state shapes, roving focus and disposable-session screenshots |
| Git actions | Real Git upstream/worktree fixtures, dirty/diverged refusal, fetch-map/mirror safety, queued target changes, browser/native row and Commands actions |
| Subscription limits | Fake-CLI cache/lease/backoff/privacy, scoped parser/model mutations, authenticated browser/native values and cross-host call counters |
| Keyboard | Shortcut-doc generation, runtime aliases/reload, prefix collisions and literal passthrough |
| Terminal | Lifecycle/race tests and successive runtime frames |
| Herdr schema/receipts | Supported-schema adapter fixtures and real split/move receipts |
| Herdr command/popup | Live advertised invocation, program input/closure, unchanged split geometry, disconnected recovery and DOM focus return |
| Project setup/recovery | Ownership, idempotency and uncertain outcomes |
| Canonical tasks | Exact-byte Markdown CAS, dependencies/checklist stages, safe overflow, current credential/generation authority, stale/ABA rejection and current-scope source acceptance |
| Scheduling/replacement/owner recovery | Disposable dependency release and bounded parallelism, atomic credential claim, generation-fenced replacement while an old writer works, isolated committed-base placement, old dirty work/claims retained and liaison reattachment |
| Supervisor transports/CLI | Root-scoped agent coordination and explicit local `--operator` management; stale ambient credentials bypassed only in operator mode, explicit targets, agent generation fences, internal inbox capture, physical versus semantic proof and durable revisions |
| Supervisor UI | Real liaison terminal with a stable open/all-Done task rail; fullscreen Board/Graph remains visible beside shared task history, stacking below when narrow; Close/Escape restores task focus/scroll before Conversation return; root waiting hint and no dead operational forms; see `docs/supervisor-surfaces.md` |
| Harness/install/inbox | Literal initial prompt and optional per-launch hook, wait/pull/internal capture, unknown delivery without replay, separate launch/model timestamps, installer protections, ordinary offline tools and unchanged OMP quota |
| Notes | Real Markdown/CAS, sibling bytes, concurrent/external edits, orphans, boot-scoped attach and disposable browser/native drag/comment/draft recovery |
| Notes transports/CLI | Standalone CLI without Herdr, Origin rejection, equivalent DTOs, custom-root copied recipes and real native IPC |
| CLI/skills | Nested help/interpretation, standalone CLI with disposable HOME/project, collision/refusal/concurrent installer behavior |
| Viewer authorization | Same-tab source, stale binding/root replacement, pluginless Files/Review and source-state retention |
| Repository cache | Mutation-generation invalidation, stale-while-refill and disposable gateway request counts |
| File index/ranking | Git ignore/symlink/caps, persisted restart and mixed Unicode ranking parity |
| Library read/recovery | Journal recovery, cross-Store identity invalidation, inherited-descriptor lease release, real reader/writer contention and bounded reader latency |
| Review streaming | Large real-Git fixture, stable identity, changed-token invalidation and linear duplicate marking |
| Folder/Space selection | Source boundaries/limits, durable selection, retention and unknown-file safety |
| Provider fetch | Configured-instance and canonical-identity fixtures |
| Library collections | Paging, ancestor/move detection, exclusion/removal safety, strict schema/journal recovery, selection retention, failed/empty/truncated listing never dropping members and tombstone purge |
| Library automatic sync | Fixed windows, retries, resumable inventory budgets, quiet scans, partial binaries, publication/removal races and live Cloud browser/native catch-up |
| Library listing/preview | Atomic generations, unchanged-page probes, stale cancellation, visible-window catch-up, rename/same-path snapshots and manual refresh/comments |
| Reference depth | Provider traversal cycles/caps, incomplete follow pass retaining members, explicit depth-zero inputs and live Jira/Confluence graph in a disposable Library |
| Vault/HTTP downloads | MemoryVault service tests, HTTP paging/redirect/downgrade fixtures and isolated `dbus-run-session` gnome-keyring smoke |
| Library transports | Equivalent browser/native DTOs and owned-runtime smoke |
| Tea CLI | Real Tea against a localhost fixture with fake login |
| Diff/frozen sources | Real Git fixtures; compare index/working files before and after |
| Comments | Exact payload bytes, CAS recovery and acknowledged paste |
| Markdown/media | Source mapping, hostile input, byte/pixel caps and browser/native rendering |
| Host composition | Equivalent DTOs and real native startup |
| Library UI | Selection/live-repository actions, refresh visibility, real-path comments and disposable browser/native smoke |
| Browser lifecycle | Independent tabs, fresh open, discard-on-close, owner-only startup reset and observer isolation |
| Widget owner/CLI | Source/target identity, same-ID replace, Remove/reopen, limits, pull/wait cancellation and owner lifetime |
| Widget transport/dock | Native/browser content/select/remove streams, stale revision/source rejection, multi-ID dock and DOM focus/shortcut routing |
