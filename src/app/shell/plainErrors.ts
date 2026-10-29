const descriptions: Record<string, string> = {
  root_main_exit_unproven: "Cockpit cannot confirm that the previous supervisor stopped. Check the old supervisor and its terminal before resuming.",
  consumer_activity_unproven: "Cockpit cannot confirm that the previous terminal or checkout is safe to reuse. Check the old supervisor and its terminal before resuming.",
  harness_resume_unsupported: "This agent cannot resume the saved conversation with its current configuration. Configure resume arguments for this agent before trying again.",
  attachment_conflict: "The agent's attachment changed or could not be verified. Refresh its state before trying again.",
  root_stop_confirmation_unavailable: "This supervisor can no longer be confirmed stopped from this prompt. Refresh its state before trying again.",
  request_outcome_unknown: "Herdr did not confirm the outcome. Resync and check the session before trying again.",
  mutation_applied_snapshot_failed: "The change was applied, but the updated session could not be loaded. Resync to see the current state.",
  protocol_mismatch: "This Herdr version uses an incompatible protocol. Install a compatible Herdr version and retry status.",
  schema_version_mismatch: "This Herdr version uses an incompatible schema. Install a compatible Herdr version and retry status.",
  missing_methods: "This Herdr version is missing required features. Install a compatible Herdr version and retry status.",
  herdr_incompatible: "Herdr is incompatible with Cockpit. Install a compatible Herdr version and retry status.",
  space_repository_mismatch: "Cockpit could not match this Space to its checkout in Herdr's inventory. Check the Space and repository, then try again.",
  project_space_open: "This repository already has an open Space. Select that Space or use a worktree for independent work.",
  widget_no_owner: "No Cockpit widget owner is available. Open Cockpit for this session and try again.",
  stale_identity: "The selected target or its state changed. Refresh the review before trying again.",
  teardown_not_allowed: "This removal is not allowed for the current target. Refresh the review and choose an available action.",
  confirmation_required: "The removal was not confirmed. Review the target and confirm the removal again.",
  operation_busy: "Another removal is already in progress. Wait for it to finish and refresh the review.",
  workspace_not_found: "The selected Space is no longer open. Refresh the session before trying again.",
  ownership_unproven: "Cockpit could not verify ownership of this resource. Review its details before attempting cleanup.",
  association_conflict: "Cockpit could not verify that the selected resources belong together. Refresh the state and review the target before trying again.",
  foreign_terminal_consumer: "A terminal is still using this checkout. Close or move that terminal before removing the worktree.",
  terminal_cwd_unknown: "Cockpit could not determine a terminal's working directory. Check the open terminals before trying cleanup again.",
  checkout_identity_changed: "The saved checkout no longer matches the current directory or worktree. Check the checkout before resuming or cleaning up.",
  resume_incarnation_changed: "The saved agent placement changed while resuming. Refresh its state before trying again.",
  space_boot_changed: "The Herdr server no longer matches the recorded Space. Check the session before attempting cleanup.",
  space_peer_changed: "The Herdr server no longer matches the recorded Space. Check the session before attempting cleanup.",
  space_incarnation_unproven: "Cockpit cannot verify the Herdr server associated with this Space. Check the session before attempting cleanup.",
  server_instance_changed: "The Herdr server changed during the request. Refresh the session before trying again.",
  endpoint_changed: "The observed Herdr session or server does not match the requested target. Reconnect to the correct session before trying again.",
  terminal_peer_unproven: "Cockpit could not verify the Herdr server needed to inspect terminals. Reconnect to the correct session before trying again.",
};

/** Translate recognized refusals while preserving host diagnostics as secondary detail. */
export function plainError(code: string | undefined, message: string): { text: string; detail?: string } {
  const text = code && Object.hasOwn(descriptions, code) ? descriptions[code] : undefined;
  if (!text) return code ? { text: message, detail: `${code}: ${message}` } : { text: message };
  return { text, detail: `${code}: ${message}` };
}
