// Authoritative request/response inventory for both transport callbacks.
// Each callback emits its selected transport; this module emits no handlers.
// Input expressions are closures called with one tuple of the listed bindings:
// () for no arguments, (argument,) for one, (first, second, ...) otherwise.
// The tuple is unpacked by the adapter, not by transport-specific business logic.
// Gateway order: extract, first guard, input, after guard, operation, response.
// Native order: Tauri deserialization, input, operation, ErrorResponse conversion.
#[macro_export]
macro_rules! cockpit_operations {
    ($callback:ident) => {
        $callback! {
            { id: status, ctx: service, out: ::cockpit_protocol::v1::StatusResponse,
              call: $crate::transport::operations::session::status,
              native: cockpit_status()
                  => $crate::transport::native_input::none(),
              http: get "/api/v1/status"
                  ()
                  => $crate::transport::http_input::none();
                  limit = none; origin = none;
                  guard = none; status = service
            }
            { id: quota_status, ctx: service, out: ::cockpit_protocol::quota::QuotaStatusResponse,
              call: $crate::transport::operations::session::quota_status,
              native: cockpit_quota_status(request: ::cockpit_protocol::quota::QuotaStatusRequest)
                  => $crate::transport::native_input::typed(),
              http: get "/api/v1/quota"
                  (query: ::axum::extract::Query<::cockpit_protocol::quota::QuotaStatusRequest>)
                  => $crate::transport::http_input::query();
                  limit = none; origin = none;
                  guard = none; status = service
            }
            { id: sessions, ctx: service, out: ::cockpit_protocol::v1::SessionListResponse,
              call: $crate::transport::operations::session::sessions,
              native: cockpit_sessions()
                  => $crate::transport::native_input::none(),
              http: get "/api/v1/sessions"
                  ()
                  => $crate::transport::http_input::none();
                  limit = none; origin = none;
                  guard = none; status = service
            }
            { id: session_snapshot, ctx: service, out: ::cockpit_protocol::v1::SessionSnapshotResponse,
              call: $crate::transport::operations::session::session_snapshot,
              native: cockpit_session_snapshot(session_id: ::std::string::String)
                  => $crate::transport::native_input::typed(),
              http: get "/api/v1/sessions/{session_id}/snapshot"
                  (path: ::axum::extract::Path<::std::string::String>)
                  => $crate::transport::http_input::session($crate::transport::http_input::Reject("invalid_session_id", "Invalid session id"));
                  limit = none; origin = none;
                  guard = none; status = service
            }
            { id: space_git_status, ctx: service, out: ::cockpit_protocol::v1::SpaceGitStatusResponse,
              call: $crate::transport::operations::session::space_git_status,
              native: cockpit_space_git_status(session_id: ::std::string::String)
                  => $crate::transport::native_input::typed(),
              http: get "/api/v1/sessions/{session_id}/space-git"
                  (path: ::axum::extract::Path<::std::string::String>)
                  => $crate::transport::http_input::session($crate::transport::http_input::Reject("invalid_session_id", "Invalid session id"));
                  limit = none; origin = none;
                  guard = none; status = service
            }
            { id: space_git_action, ctx: service, out: ::cockpit_protocol::v1::SpaceGitActionResponse,
              call: $crate::transport::operations::session::space_git_action,
              native: cockpit_space_git_action(session_id: ::std::string::String, request: ::cockpit_protocol::v1::SpaceGitActionRequest)
                  => $crate::transport::native_input::typed2(),
              http: post "/api/v1/sessions/{session_id}/space-git/actions"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::v1::SpaceGitActionRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::session_sized_body($crate::transport::http_input::Reject("invalid_session_id", "Invalid session id"), $crate::transport::http_input::Reject("invalid_space_git_action", "Expected a bounded JSON Space Git action request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = method;
                  guard = none; status = service
            }
            { id: focus, ctx: service, out: ::cockpit_protocol::v1::FocusResponse,
              call: $crate::transport::operations::session::focus,
              native: cockpit_focus(session_id: ::std::string::String, request: ::cockpit_protocol::v1::FocusRequest)
                  => $crate::transport::native_input::typed2(),
              http: post "/api/v1/sessions/{session_id}/focus"
                  (path: ::axum::extract::Path<::std::string::String>, request: ::axum::Json<::cockpit_protocol::v1::FocusRequest>)
                  => $crate::transport::http_input::session_json();
                  limit = none; origin = none;
                  guard = none; status = service
            }
            { id: mutate, ctx: service, out: ::cockpit_protocol::v1::ResourceMutationResponse,
              call: $crate::transport::operations::session::mutate,
              native: cockpit_mutate(session_id: ::std::string::String, request: ::cockpit_protocol::v1::ResourceMutationRequest)
                  => $crate::transport::native_input::typed2(),
              http: post "/api/v1/sessions/{session_id}/mutations"
                  (path: ::axum::extract::Path<::std::string::String>, request: ::axum::Json<::cockpit_protocol::v1::ResourceMutationRequest>)
                  => $crate::transport::http_input::session_json();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = none;
                  guard = none; status = service
            }
            { id: browser_action, ctx: browser, out: ::cockpit_protocol::browser::BrowserResponse,
              call: $crate::transport::operations::browser::browser_action,
              native: cockpit_browser_action(request: ::cockpit_protocol::browser::BrowserRequest)
                  => $crate::transport::native_input::typed(),
              http: post "/api/v1/browser/action"
                  (request: ::axum::Json<::cockpit_protocol::browser::BrowserRequest>)
                  => $crate::transport::http_input::json();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = none;
                  guard = after(Browser); status = service
            }
            { id: browser_feedback, ctx: browser, out: ::cockpit_protocol::browser::BrowserFeedbackLookup,
              call: $crate::transport::operations::browser::browser_feedback,
              native: cockpit_browser_feedback(request: ::cockpit_protocol::browser::BrowserFeedbackRequest)
                  => $crate::transport::native_input::typed(),
              http: post "/api/v1/browser/feedback"
                  (request: ::axum::Json<::cockpit_protocol::browser::BrowserFeedbackRequest>)
                  => $crate::transport::http_input::json();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = none;
                  guard = after(Browser); status = service
            }
            { id: browser_feedback_ack, ctx: browser, out: ::cockpit_protocol::browser_feedback::BrowserFeedbackAck,
              call: $crate::transport::operations::browser::browser_feedback_ack,
              native: cockpit_browser_feedback_ack(request: ::cockpit_protocol::browser::BrowserFeedbackAckRequest)
                  => $crate::transport::native_input::typed(),
              http: post "/api/v1/browser/feedback/ack"
                  (request: ::axum::Json<::cockpit_protocol::browser::BrowserFeedbackAckRequest>)
                  => $crate::transport::http_input::json();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = none;
                  guard = after(Browser); status = service
            }
            { id: browser_feedback_image, ctx: browser, out: ::cockpit_protocol::browser::BrowserFeedbackImage,
              call: $crate::transport::operations::browser::browser_feedback_image,
              native: cockpit_browser_feedback_image(request: ::cockpit_protocol::browser::BrowserFeedbackImageRequest)
                  => $crate::transport::native_input::typed(),
              http: post "/api/v1/browser/feedback/image"
                  (request: ::axum::Json<::cockpit_protocol::browser::BrowserFeedbackImageRequest>)
                  => $crate::transport::http_input::json();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = none;
                  guard = after(Browser); status = service
            }
            { id: browser_feedback_send, ctx: browser, out: ::cockpit_protocol::browser::BrowserFeedbackSendResponse,
              call: $crate::transport::operations::browser::browser_feedback_send,
              native: cockpit_browser_feedback_send(request: ::cockpit_protocol::browser::BrowserFeedbackSendRequest)
                  => $crate::transport::native_input::typed(),
              http: post "/api/v1/browser/feedback/send"
                  (request: ::axum::Json<::cockpit_protocol::browser::BrowserFeedbackSendRequest>)
                  => $crate::transport::http_input::json();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = none;
                  guard = after(Browser); status = service
            }
            { id: browser_cleanup_retry, ctx: browser, out: ::cockpit_protocol::browser::BrowserCleanupStatus,
              call: $crate::transport::operations::browser::browser_cleanup_retry,
              native: cockpit_browser_cleanup_retry(request: ::cockpit_protocol::browser::BrowserCleanupRetryRequest)
                  => $crate::transport::native_input::typed(),
              http: post "/api/v1/browser/cleanup/retry"
                  (request: ::axum::Json<::cockpit_protocol::browser::BrowserCleanupRetryRequest>)
                  => $crate::transport::http_input::json();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = none;
                  guard = after(Browser); status = service
            }
            { id: browser_cleanup_status, ctx: browser, out: ::cockpit_protocol::browser::BrowserCleanupStatus,
              call: $crate::transport::operations::browser::browser_cleanup_status,
              native: cockpit_browser_cleanup_status()
                  => $crate::transport::native_input::none(),
              http: get "/api/v1/browser/cleanup"
                  ()
                  => $crate::transport::http_input::none();
                  limit = none; origin = none;
                  guard = after(Browser); status = service
            }
            { id: browser_view_open, ctx: browser, out: $crate::transport::operations::browser::BrowserViewOpened,
              call: $crate::transport::operations::browser::browser_view_open,
              native: cockpit_browser_view_open(request: ::cockpit_protocol::browser_view::BrowserViewOpenRequest)
                  => $crate::transport::native_input::validated($crate::transport::operations::browser::validate_view_open),
              http: post "/api/v1/browser/view/open"
                  (request: ::axum::Json<::cockpit_protocol::browser_view::BrowserViewOpenRequest>)
                  => $crate::transport::http_input::validated_json($crate::transport::operations::browser::validate_view_open);
                  limit = $crate::transport::limits::BROWSER_VIEW_JSON_BYTES; origin = none;
                  guard = after(Browser); status = bad_request
            }
            { id: browser_draft_recovery, ctx: browser, out: ::cockpit_protocol::browser_view::BrowserViewCommandOutcome,
              call: $crate::transport::operations::browser::browser_draft_recovery,
              native: cockpit_browser_draft_recovery(request: ::cockpit_protocol::browser_view::BrowserDraftRecoveryRequest)
                  => $crate::transport::native_input::typed(),
              http: post "/api/v1/browser/drafts/recovery"
                  (request: ::axum::Json<::cockpit_protocol::browser_view::BrowserDraftRecoveryRequest>)
                  => $crate::transport::http_input::json();
                  limit = $crate::transport::limits::BROWSER_VIEW_JSON_BYTES; origin = none;
                  guard = after(Browser); status = bad_request
            }
            { id: browser_view_command, ctx: browser, out: ::cockpit_protocol::browser_view::BrowserViewCommandResponse,
              call: $crate::transport::operations::browser::browser_view_command,
              native: cockpit_browser_view_command(request: ::cockpit_protocol::browser_view::BrowserViewCommandRequest)
                  => $crate::transport::native_input::validated($crate::transport::operations::browser::validate_view_command),
              http: post "/api/v1/browser/view/command"
                  (request: ::axum::Json<::cockpit_protocol::browser_view::BrowserViewCommandRequest>)
                  => $crate::transport::http_input::validated_json($crate::transport::operations::browser::validate_view_command);
                  limit = $crate::transport::limits::BROWSER_VIEW_COMMAND_BYTES; origin = none;
                  guard = after(Browser); status = bad_request
            }
            { id: browser_view_release, ctx: browser, out: (),
              call: $crate::transport::operations::browser::browser_view_release,
              native: cockpit_browser_view_release(view_id: ::std::string::String)
                  => $crate::transport::native_input::view_id(),
              http: none
            }
            { id: widget_content, ctx: browser, out: ::cockpit_protocol::widget::WidgetContent,
              call: $crate::transport::operations::widgets::widget_content,
              native: cockpit_widget_content(request: ::cockpit_protocol::widget::WidgetContentRequest)
                  => $crate::transport::native_input::widget(),
              http: post "/api/v1/widgets/content"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::widget::WidgetContentRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::widget_body($crate::transport::http_input::Reject("widget_usage", "Malformed or oversized widget request"));
                  limit = $crate::transport::limits::WIDGET_BYTES; origin = route;
                  guard = first(Widget); status = service
            }
            { id: widget_remove, ctx: browser, out: ::cockpit_protocol::widget::WidgetRemoveResponse,
              call: $crate::transport::operations::widgets::widget_remove,
              native: cockpit_widget_remove(request: ::cockpit_protocol::widget::WidgetRemoveRequest)
                  => $crate::transport::native_input::widget(),
              http: post "/api/v1/widgets/remove"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::widget::WidgetRemoveRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::widget_body($crate::transport::http_input::Reject("widget_usage", "Malformed or oversized widget request"));
                  limit = $crate::transport::limits::WIDGET_BYTES; origin = route;
                  guard = first(Widget); status = service
            }
            { id: widget_select, ctx: browser, out: ::cockpit_protocol::widget::WidgetSelectResponse,
              call: $crate::transport::operations::widgets::widget_select,
              native: cockpit_widget_select(request: ::cockpit_protocol::widget::WidgetSelectRequest)
                  => $crate::transport::native_input::widget(),
              http: post "/api/v1/widgets/select"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::widget::WidgetSelectRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::widget_body($crate::transport::http_input::Reject("widget_usage", "Malformed or oversized widget request"));
                  limit = $crate::transport::limits::WIDGET_SELECT_BYTES; origin = route;
                  guard = first(Widget); status = service
            }
            { id: notes_execute, ctx: service, out: ::cockpit_protocol::notes::NotesResponse,
              call: $crate::transport::operations::notes::notes_execute,
              native: cockpit_notes_execute(request: ::serde_json::Value)
                  => $crate::transport::native_input::notes(),
              http: post "/api/v1/notes"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::notes::NotesRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::notes_body();
                  limit = $crate::transport::limits::NOTES_BYTES; origin = route;
                  guard = none; status = notes
            }
            { id: orchestration_snapshot, ctx: orchestration, out: ::cockpit_protocol::orchestration::OrchestrationSnapshot,
              call: $crate::transport::operations::orchestration::orchestration_snapshot,
              native: orchestration_snapshot(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "orchestration"),
              http: get "/api/v1/sessions/{session_id}/orchestration"
                  (path: ::axum::extract::Path<::std::string::String>, query: ::std::result::Result<::axum::extract::Query<$crate::transport::operations::orchestration::SnapshotQuery>, ::axum::extract::rejection::QueryRejection>)
                  => $crate::transport::operations::orchestration::snapshot_input();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = after(Orchestration); status = service
            }
            { id: orchestration_mutate, ctx: orchestration, out: ::cockpit_protocol::orchestration::OrchestrationMutationResponse,
              call: $crate::transport::operations::orchestration::orchestration_mutate,
              native: orchestration_mutate(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "orchestration"),
              http: post "/api/v1/sessions/{session_id}/orchestration/mutations"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::orchestration::OrchestrationMutationRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::operations::orchestration::mutation_input();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = after(Orchestration); status = service
            }
            { id: orchestration_wait, ctx: orchestration, out: ::cockpit_protocol::orchestration::OrchestrationWaitResponse,
              call: $crate::transport::operations::orchestration::orchestration_wait,
              native: orchestration_wait(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "orchestration"),
              http: post "/api/v1/orchestration/wait"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::orchestration::OrchestrationWaitRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_orchestration_request", "Expected a bounded JSON orchestration request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = after(Orchestration); status = service
            }
            { id: project_configuration, ctx: service, out: ::cockpit_protocol::projects::ProjectConfiguration,
              call: $crate::transport::operations::projects::project_configuration,
              native: cockpit_project_configuration()
                  => $crate::transport::native_input::none(),
              http: get "/api/v1/project/configuration"
                  ()
                  => $crate::transport::http_input::none();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: repositories, ctx: service, out: ::cockpit_protocol::projects::RepositoryListResponse,
              call: $crate::transport::operations::projects::repositories,
              native: cockpit_repositories()
                  => $crate::transport::native_input::none(),
              http: get "/api/v1/project/repositories"
                  ()
                  => $crate::transport::http_input::none();
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_defaults, ctx: service, out: ::cockpit_protocol::project_defaults::WorkspaceDefaults,
              call: $crate::transport::operations::projects::workspace_defaults,
              native: cockpit_resolve_workspace_defaults(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "workspace defaults"),
              http: post "/api/v1/project/defaults"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::project_defaults::WorkspaceDefaultsRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_project_request", "Expected a bounded JSON project request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_plan, ctx: service, out: ::cockpit_protocol::projects::WorkspaceSetupPlan,
              call: $crate::transport::operations::projects::workspace_plan,
              native: cockpit_workspace_plan(session_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::session_value($crate::transport::limits::MUTATION_BYTES, "project"),
              http: post "/api/v1/sessions/{session_id}/workspace-plans"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::projects::WorkspaceSetupRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::session_body($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"), $crate::transport::http_input::Reject("invalid_project_request", "Expected a bounded JSON project request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_start, ctx: service, out: ::cockpit_protocol::projects::WorkspaceOperation,
              call: $crate::transport::operations::projects::workspace_start,
              native: cockpit_workspace_start(session_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::session_value($crate::transport::limits::MUTATION_BYTES, "project"),
              http: post "/api/v1/sessions/{session_id}/workspace-operations"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::projects::WorkspaceOperationRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::session_body($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"), $crate::transport::http_input::Reject("invalid_project_request", "Expected a bounded JSON project request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_resume, ctx: service, out: ::cockpit_protocol::projects::WorkspaceOperation,
              call: $crate::transport::operations::projects::workspace_resume,
              native: cockpit_workspace_resume(session_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::session_value($crate::transport::limits::MUTATION_BYTES, "project"),
              http: post "/api/v1/sessions/{session_id}/workspace-operations/resume"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::projects::WorkspaceOperationRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::session_body($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"), $crate::transport::http_input::Reject("invalid_project_request", "Expected a bounded JSON project request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_cancel, ctx: service, out: ::cockpit_protocol::projects::WorkspaceOperation,
              call: $crate::transport::operations::projects::workspace_cancel,
              native: cockpit_workspace_cancel(session_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::session_value($crate::transport::limits::MUTATION_BYTES, "project"),
              http: post "/api/v1/sessions/{session_id}/workspace-operations/cancel"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::projects::WorkspaceOperationRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::session_body($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"), $crate::transport::http_input::Reject("invalid_project_request", "Expected a bounded JSON project request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_reconcile, ctx: service, out: ::cockpit_protocol::projects::WorkspaceOperation,
              call: $crate::transport::operations::projects::workspace_reconcile,
              native: cockpit_workspace_reconcile(session_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::session_value($crate::transport::limits::MUTATION_BYTES, "project"),
              http: post "/api/v1/sessions/{session_id}/workspace-operations/reconcile"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::projects::WorkspaceReconcileRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::session_body($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"), $crate::transport::http_input::Reject("invalid_project_request", "Expected a bounded JSON project request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_teardown_preview, ctx: service, out: ::cockpit_protocol::project_teardown::WorkspaceTeardownPreview,
              call: $crate::transport::operations::projects::workspace_teardown_preview,
              native: cockpit_workspace_teardown_preview(session_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::session_value($crate::transport::limits::MUTATION_BYTES, "project teardown"),
              http: post "/api/v1/sessions/{session_id}/workspace-teardown/preview"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::project_teardown::WorkspaceTeardownPreviewRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::session_body($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"), $crate::transport::http_input::Reject("invalid_project_request", "Expected a bounded JSON project request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_teardown_execute, ctx: service, out: ::cockpit_protocol::project_teardown::WorkspaceTeardownResult,
              call: $crate::transport::operations::projects::workspace_teardown_execute,
              native: cockpit_workspace_teardown_execute(session_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::session_value($crate::transport::limits::MUTATION_BYTES, "project teardown"),
              http: post "/api/v1/sessions/{session_id}/workspace-teardown/execute"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::project_teardown::WorkspaceTeardownExecuteRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::session_body($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"), $crate::transport::http_input::Reject("invalid_project_request", "Expected a bounded JSON project request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_operation, ctx: service, out: ::cockpit_protocol::projects::WorkspaceOperation,
              call: $crate::transport::operations::projects::workspace_operation,
              native: cockpit_workspace_operation(session_id: ::std::string::String, operation_id: ::std::string::String)
                  => $crate::transport::native_input::typed2(),
              http: get "/api/v1/sessions/{session_id}/workspace-operations/{operation_id}"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>)
                  => $crate::transport::http_input::session_pair_path($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: workspace_teardown_recoveries, ctx: service, out: ::cockpit_protocol::project_teardown::WorkspaceTeardownRecoveryList,
              call: $crate::transport::operations::projects::workspace_teardown_recoveries,
              native: cockpit_workspace_teardown_recoveries(session_id: ::std::string::String)
                  => $crate::transport::native_input::typed(),
              http: get "/api/v1/sessions/{session_id}/workspace-teardown/recoveries"
                  (path: ::axum::extract::Path<::std::string::String>)
                  => $crate::transport::http_input::session($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: provider_credentials, ctx: service, out: ::cockpit_protocol::credentials::ProviderCredentialStatusList,
              call: $crate::transport::operations::credentials::provider_credentials,
              native: cockpit_provider_credentials()
                  => $crate::transport::native_input::none(),
              http: get "/api/v1/provider-credentials"
                  ()
                  => $crate::transport::http_input::none();
                  limit = $crate::transport::limits::CREDENTIAL_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: provider_credential_set, ctx: service, out: ::cockpit_protocol::credentials::ProviderCredentialStatus,
              call: $crate::transport::operations::credentials::provider_credential_set,
              native: cockpit_provider_credential_set(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "credential"),
              http: post "/api/v1/provider-credentials/set"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::credentials::ProviderCredentialSetRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_credential_request", "Expected a bounded JSON credential request with valid fields"));
                  limit = $crate::transport::limits::CREDENTIAL_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: provider_credential_clear, ctx: service, out: ::cockpit_protocol::credentials::ProviderCredentialStatus,
              call: $crate::transport::operations::credentials::provider_credential_clear,
              native: cockpit_provider_credential_clear(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "credential"),
              http: post "/api/v1/provider-credentials/clear"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::credentials::ProviderCredentialClearRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_credential_request", "Expected a bounded JSON credential request with valid fields"));
                  limit = $crate::transport::limits::CREDENTIAL_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: viewer_sources, ctx: service, out: ::cockpit_protocol::viewer::ViewerSourceOptions,
              call: $crate::transport::operations::viewer::viewer_sources,
              native: cockpit_viewer_sources(session_id: ::std::string::String, pane_id: ::std::string::String)
                  => $crate::transport::native_input::typed2(),
              http: get "/api/v1/sessions/{session_id}/panes/{pane_id}/viewer-sources"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>)
                  => $crate::transport::http_input::pair($crate::transport::http_input::Reject("invalid_viewer_request", "Session or pane ID is invalid"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: viewer_open, ctx: service, out: ::cockpit_protocol::viewer::ViewerContext,
              call: $crate::transport::operations::viewer::viewer_open,
              native: cockpit_viewer_open(session_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::session_value($crate::transport::limits::MUTATION_BYTES, "viewer"),
              http: post "/api/v1/sessions/{session_id}/viewers/open"
                  (path: ::axum::extract::Path<::std::string::String>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::viewer::ViewerOpenRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::session_body($crate::transport::http_input::Reject("invalid_session_id", "Session ID is invalid"), $crate::transport::http_input::Reject("invalid_viewer_request", "Expected a bounded JSON viewer request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: viewer_release, ctx: service, out: (),
              call: $crate::transport::operations::viewer::viewer_release,
              native: cockpit_viewer_release(session_id: ::std::string::String, viewer_id: ::std::string::String)
                  => $crate::transport::native_input::typed2(),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/release"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>)
                  => $crate::transport::http_input::pair($crate::transport::http_input::Reject("invalid_viewer_request", "Session or viewer ID is invalid"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: context_directory, ctx: service, out: ::cockpit_protocol::context::ContextDirectory,
              call: $crate::transport::operations::context::context_directory,
              native: cockpit_context_directory(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "context"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/directory"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::context::ContextDirectoryRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_context_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_context_request", "Expected a bounded JSON context request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: context_file_index, ctx: service, out: ::cockpit_protocol::context::ContextFileIndex,
              call: $crate::transport::operations::context::context_file_index,
              native: cockpit_context_file_index(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "context file index"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/files"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::context::ContextFileIndexRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_context_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_context_request", "Expected a bounded JSON context request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: context_document, ctx: service, out: ::cockpit_protocol::context::ContextDocument,
              call: $crate::transport::operations::context::context_document,
              native: cockpit_context_document(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "context"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/document"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::context::ContextDocumentRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_context_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_context_request", "Expected a bounded JSON context request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: context_search, ctx: service, out: ::cockpit_protocol::context_search::ContextSearchResponse,
              call: $crate::transport::operations::context::context_search,
              native: cockpit_context_search(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "context search"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/search"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::context_search::ContextSearchRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_context_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_context_request", "Expected a bounded JSON context request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: context_invalidate, ctx: service, out: ::cockpit_protocol::context_search::ContextInvalidationResponse,
              call: $crate::transport::operations::context::context_invalidate,
              native: cockpit_context_invalidate(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "context invalidation"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/invalidate"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::context_search::ContextInvalidationRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_context_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_context_request", "Expected a bounded JSON context request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: context_media, ctx: service, out: ::cockpit_protocol::context_media::ContextMedia,
              call: $crate::transport::operations::context::context_media,
              native: cockpit_context_media(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "context media"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/context/media"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::context_media::ContextMediaRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_context_media_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_context_media_request", "Expected a bounded JSON Context media request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: review_snapshot, ctx: service, out: ::cockpit_protocol::review::ReviewSnapshot,
              call: $crate::transport::operations::review::review_snapshot,
              native: cockpit_review_snapshot(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "review snapshot"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/review/snapshot"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::review::ReviewSnapshotRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("review_invalid_request", "Invalid session or viewer"), $crate::transport::http_input::Reject("review_invalid_request", "Expected a bounded review snapshot request"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: review_file, ctx: service, out: ::cockpit_protocol::review::ReviewFileDiff,
              call: $crate::transport::operations::review::review_file,
              native: cockpit_review_file(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "review file"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/review/file"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::review::ReviewFileRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("review_invalid_request", "Invalid session or viewer"), $crate::transport::http_input::Reject("review_invalid_request", "Expected a bounded review file request"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_list, ctx: service, out: ::cockpit_protocol::comments::CommentBatchList,
              call: $crate::transport::operations::comments::comments_list,
              native: cockpit_comments_list(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/list"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comments::CommentRequestScope>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_batch, ctx: service, out: ::cockpit_protocol::comments::CommentBatch,
              call: $crate::transport::operations::comments::comments_batch,
              native: cockpit_comments_batch(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/batch"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comments::CommentBatchRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_upsert, ctx: service, out: ::cockpit_protocol::comments::CommentBatch,
              call: $crate::transport::operations::comments::comments_upsert,
              native: cockpit_comments_upsert(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/upsert"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comments::CommentUpsertRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_remove, ctx: service, out: ::cockpit_protocol::comments::CommentBatch,
              call: $crate::transport::operations::comments::comments_remove,
              native: cockpit_comments_remove(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/remove"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comments::CommentRemoveRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_discard, ctx: service, out: ::cockpit_protocol::comments::CommentBatchList,
              call: $crate::transport::operations::comments::comments_discard,
              native: cockpit_comments_discard(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/discard"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comments::CommentBatchMutation>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_attach, ctx: service, out: ::cockpit_protocol::comments::CommentBatch,
              call: $crate::transport::operations::comments::comments_attach,
              native: cockpit_comments_attach(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/attach"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comments::CommentBatchMutation>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_preview, ctx: service, out: ::cockpit_protocol::comments::CommentPreview,
              call: $crate::transport::operations::comments::comments_preview,
              native: cockpit_comments_preview(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/preview"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comments::CommentPreviewRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_paste_prepare, ctx: service, out: ::cockpit_protocol::comment_paste::CommentPastePrepareResponse,
              call: $crate::transport::operations::comments::comments_paste_prepare,
              native: cockpit_comments_paste_prepare(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/paste-prepare"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comment_paste::CommentPastePrepareRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_paste_send, ctx: service, out: ::cockpit_protocol::comment_paste::CommentPasteReceipt,
              call: $crate::transport::operations::comments::comments_paste_send,
              native: cockpit_comments_paste_send(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/paste-send"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comment_paste::CommentPasteSendRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: comments_paste_mark_pasted, ctx: service, out: ::cockpit_protocol::comment_paste::CommentPasteReceipt,
              call: $crate::transport::operations::comments::comments_paste_mark_pasted,
              native: cockpit_comments_paste_mark_pasted(session_id: ::std::string::String, viewer_id: ::std::string::String, request: ::serde_json::Value)
                  => $crate::transport::native_input::pair_value($crate::transport::limits::MUTATION_BYTES, "comments"),
              http: post "/api/v1/sessions/{session_id}/viewers/{viewer_id}/comments/paste-mark-pasted"
                  (path: ::axum::extract::Path<(::std::string::String, ::std::string::String)>, body: ::std::result::Result<::axum::Json<::cockpit_protocol::comment_paste::CommentPasteMarkPastedRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::pair_body($crate::transport::http_input::Reject("invalid_comments_request", "Session or viewer ID is invalid"), $crate::transport::http_input::Reject("invalid_comments_request", "Expected a bounded JSON comments request with valid fields"));
                  limit = $crate::transport::limits::MUTATION_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_listing, ctx: service, out: ::cockpit_protocol::library::LibraryListing,
              call: $crate::transport::operations::library::library_listing,
              native: cockpit_library_listing(offset: ::std::option::Option<u32>)
                  => $crate::transport::native_input::typed(),
              http: get "/api/v1/library"
                  (query: ::std::result::Result<::axum::extract::Query<$crate::transport::operations::library::ListingQuery>, ::axum::extract::rejection::QueryRejection>)
                  => $crate::transport::operations::library::listing_input();
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_resolve, ctx: service, out: ::cockpit_protocol::library::LibraryResolution,
              call: $crate::transport::operations::library::library_resolve,
              native: cockpit_library_resolve(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library resolve"),
              http: post "/api/v1/library/resolve"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryResolveRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_confluence_spaces, ctx: service, out: ::std::vec::Vec<::cockpit_protocol::library::LibraryResolution>,
              call: $crate::transport::operations::library::library_confluence_spaces,
              native: cockpit_library_confluence_spaces(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library Confluence spaces"),
              http: post "/api/v1/library/confluence/spaces"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryConfluenceSpacesRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_add, ctx: service, out: ::cockpit_protocol::library::LibraryOperation,
              call: $crate::transport::operations::library::library_add,
              native: cockpit_library_add(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library add"),
              http: post "/api/v1/library/add"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryAddRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_attachments, ctx: service, out: ::cockpit_protocol::library::LibraryOperation,
              call: $crate::transport::operations::library::library_attachments,
              native: cockpit_library_attachments(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library attachments"),
              http: post "/api/v1/library/attachments"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryAttachmentRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_refresh, ctx: service, out: ::cockpit_protocol::library::LibraryOperation,
              call: $crate::transport::operations::library::library_refresh,
              native: cockpit_library_refresh(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library refresh"),
              http: post "/api/v1/library/refresh"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryRefreshRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_replace, ctx: service, out: ::cockpit_protocol::library::LibraryOperation,
              call: $crate::transport::operations::library::library_replace,
              native: cockpit_library_replace(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::LIBRARY_BYTES, "library replace"),
              http: post "/api/v1/library/replace"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryReplaceRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_remove, ctx: service, out: ::cockpit_protocol::library::LibraryListing,
              call: $crate::transport::operations::library::library_remove,
              native: cockpit_library_remove(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::LIBRARY_BYTES, "library remove"),
              http: post "/api/v1/library/remove"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryRemoveRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_directory, ctx: service, out: ::cockpit_protocol::context::ContextDirectory,
              call: $crate::transport::operations::library::library_directory,
              native: cockpit_library_directory(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library directory"),
              http: post "/api/v1/library/directory"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryDirectoryRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_file_index, ctx: service, out: ::cockpit_protocol::context::ContextFileIndex,
              call: $crate::transport::operations::library::library_file_index,
              native: cockpit_library_file_index(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library file index"),
              http: post "/api/v1/library/files"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryFileIndexRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_document, ctx: service, out: ::cockpit_protocol::context::ContextDocument,
              call: $crate::transport::operations::library::library_document,
              native: cockpit_library_document(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library document"),
              http: post "/api/v1/library/document"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryDocumentRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_media, ctx: service, out: ::cockpit_protocol::context_media::ContextMedia,
              call: $crate::transport::operations::library::library_media,
              native: cockpit_library_media(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library media"),
              http: post "/api/v1/library/media"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::LibraryMediaRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_space_list, ctx: service, out: ::cockpit_protocol::library::SpaceContextListing,
              call: $crate::transport::operations::library::library_space_list,
              native: cockpit_library_space_list(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::MUTATION_BYTES, "library space list"),
              http: post "/api/v1/library/space/list"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::SpaceContextRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_space_add, ctx: service, out: ::cockpit_protocol::library::LibraryOperation,
              call: $crate::transport::operations::library::library_space_add,
              native: cockpit_library_space_add(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::LIBRARY_BYTES, "library space add"),
              http: post "/api/v1/library/space/add"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::SpaceAddRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_space_repositories, ctx: service, out: ::cockpit_protocol::library::SpaceContextListing,
              call: $crate::transport::operations::library::library_space_repositories,
              native: cockpit_library_space_repositories(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::LIBRARY_BYTES, "library space repositories"),
              http: post "/api/v1/library/space/repositories"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::SpaceRepositoriesRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_space_remove, ctx: service, out: ::cockpit_protocol::library::SpaceContextListing,
              call: $crate::transport::operations::library::library_space_remove,
              native: cockpit_library_space_remove(request: ::serde_json::Value)
                  => $crate::transport::native_input::value($crate::transport::limits::LIBRARY_BYTES, "library space remove"),
              http: post "/api/v1/library/space/remove"
                  (body: ::std::result::Result<::axum::Json<::cockpit_protocol::library::SpaceRemoveRequest>, ::axum::extract::rejection::JsonRejection>)
                  => $crate::transport::http_input::body($crate::transport::http_input::Reject("invalid_library_request", "Expected a valid bounded Library request"));
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_operation, ctx: service, out: ::cockpit_protocol::library::LibraryOperation,
              call: $crate::transport::operations::library::library_operation,
              native: cockpit_library_operation(operation_id: ::std::string::String)
                  => $crate::transport::native_input::typed(),
              http: get "/api/v1/library/operations/{id}"
                  (path: ::axum::extract::Path<::std::string::String>)
                  => $crate::transport::http_input::id_path();
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
            { id: library_operation_cancel, ctx: service, out: ::cockpit_protocol::library::LibraryOperation,
              call: $crate::transport::operations::library::library_operation_cancel,
              native: cockpit_library_operation_cancel(operation_id: ::std::string::String)
                  => $crate::transport::native_input::typed(),
              http: post "/api/v1/library/operations/{id}/cancel"
                  (path: ::axum::extract::Path<::std::string::String>)
                  => $crate::transport::http_input::id_path();
                  limit = $crate::transport::limits::LIBRARY_BYTES; origin = route;
                  guard = none; status = service
            }
        }
    };
}
