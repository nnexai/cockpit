use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use cockpit_core::project_adapter::{
    ProjectHerdrAdapter, ProjectInventory, ProjectTerminalRequest, ProjectTerminalResult,
    ProjectWorktreeEntry, ProjectWorktreeRemoveRequest, ProjectWorktreeRequest,
    ProjectWorktreeResult,
};
use cockpit_core::{HerdrAdapter, InspectionError};
use cockpit_protocol::projects::WorkspaceSetupMode;
use cockpit_protocol::v1::HerdrCompatibility;
use serde_json::{Map, Value, json};
use tokio::process::Command;

use super::{
    HerdrCliAdapter, object, optional_string, required_array, required_bool, required_string,
    schema_fields, valid_pane_id,
};

const MAX_PROJECT_TEXT: usize = 4096;
const MAX_ENV_ENTRIES: usize = 16;

fn invalid(field: &str) -> InspectionError {
    InspectionError::new("invalid_project_request", format!("{field} is invalid"))
}

fn unsupported(message: impl Into<String>) -> InspectionError {
    InspectionError::new("unsupported_capability", message)
}

fn schema_methods(value: &Value) -> Result<(u32, BTreeSet<String>), InspectionError> {
    schema_fields(value).ok_or_else(|| {
        InspectionError::new(
            "malformed_schema",
            "Herdr schema declarations are malformed",
        )
    })
}

fn ensure_compatible(compatibility: HerdrCompatibility) -> Result<(), InspectionError> {
    match compatibility {
        HerdrCompatibility::Compatible { .. } => Ok(()),
        HerdrCompatibility::Incompatible { code, .. } => Err(unsupported(format!(
            "Herdr compatibility check failed: {code}"
        ))),
        HerdrCompatibility::Unavailable { code, .. } => Err(unsupported(format!(
            "Herdr endpoint is unavailable: {code}"
        ))),
    }
}

fn ensure_endpoint_identity(identity: &str) -> Result<(), InspectionError> {
    ensure_text(identity, "endpoint_identity")
}

impl HerdrCliAdapter {
    async fn project_methods(&self, session_id: &str) -> Result<BTreeSet<String>, InspectionError> {
        ensure_compatible(self.inspect_session(session_id).await?)?;
        let schema = self
            .run_json_for(Some(session_id), &["api", "schema", "--json"])
            .await?;
        let (_, methods) = schema_methods(&schema)?;
        Ok(methods)
    }
}

fn parse_worktree_list(
    value: &Value,
    source_cwd: &str,
) -> Result<(String, String, Vec<ProjectWorktreeEntry>), InspectionError> {
    let result = object(value, "worktree.list result")?;
    if required_string(result, "type", "worktree.list result")? != "worktree_list" {
        return Err(InspectionError::new(
            "malformed_provenance",
            "worktree.list result.type must be worktree_list",
        ));
    }
    let source = object(
        result.get("source").ok_or_else(|| {
            InspectionError::new(
                "malformed_provenance",
                "worktree.list result.source is required",
            )
        })?,
        "worktree.list result.source",
    )?;
    let repository_key = required_string(source, "repo_key", "worktree.list result.source")?;
    let repository_name = required_string(source, "repo_name", "worktree.list result.source")?;
    let repository_root = required_string(source, "repo_root", "worktree.list result.source")?;
    let source_checkout = required_string(
        source,
        "source_checkout_path",
        "worktree.list result.source",
    )?;
    if source_checkout != source_cwd || !Path::new(&source_checkout).is_absolute() {
        return Err(InspectionError::new(
            "provenance_conflict",
            "worktree.list source checkout does not exactly match the requested cwd",
        ));
    }
    if repository_key.is_empty()
        || repository_name.is_empty()
        || !Path::new(&repository_root).is_absolute()
    {
        return Err(InspectionError::new(
            "malformed_provenance",
            "worktree.list source repository identity is malformed",
        ));
    }
    let worktrees = required_array(result, "worktrees", "worktree.list result")?;
    let mut entries = Vec::with_capacity(worktrees.len());
    let mut paths = BTreeSet::new();
    let mut open_workspaces = BTreeSet::new();
    let mut primary_count = 0;
    for (index, value) in worktrees.iter().enumerate() {
        let context = format!("worktree.list result.worktrees[{index}]");
        let worktree = object(value, &context)?;
        let checkout_path = required_string(worktree, "path", &context)?;
        if !Path::new(&checkout_path).is_absolute() || !paths.insert(checkout_path.clone()) {
            return Err(InspectionError::new(
                "malformed_provenance",
                "worktree.list contains a malformed or repeated checkout path",
            ));
        }
        let is_detached = required_bool(worktree, "is_detached", &context)?;
        let is_linked = required_bool(worktree, "is_linked_worktree", &context)?;
        let open_workspace_id = optional_string(worktree, "open_workspace_id", &context)?;
        if let Some(id) = open_workspace_id.as_deref()
            && (!valid_pane_id(id) || !open_workspaces.insert(id.to_owned()))
        {
            return Err(InspectionError::new(
                "malformed_identity",
                "worktree.list contains a repeated or malformed workspace identity",
            ));
        }
        let is_primary = checkout_path == source_cwd;
        if is_primary {
            primary_count += 1;
            if is_linked {
                return Err(InspectionError::new(
                    "provenance_conflict",
                    "source checkout is unexpectedly marked as a linked worktree",
                ));
            }
        }
        entries.push(ProjectWorktreeEntry {
            checkout_path,
            branch: if is_detached {
                None
            } else {
                optional_string(worktree, "branch", &context)?
            },
            open_workspace_id,
            is_primary,
            is_linked_worktree: is_linked,
            dirty: None,
        });
    }
    if primary_count != 1 {
        return Err(InspectionError::new(
            "provenance_conflict",
            "worktree.list did not identify exactly one source checkout",
        ));
    }
    Ok((repository_key, repository_root, entries))
}

fn ensure_text(value: &str, field: &str) -> Result<(), InspectionError> {
    if value.is_empty()
        || value.len() > MAX_PROJECT_TEXT
        || value.contains('\0')
        || value.chars().any(char::is_control)
    {
        return Err(invalid(field));
    }
    Ok(())
}

fn worktree_params(
    request: &ProjectWorktreeRequest,
) -> Result<(&'static str, Value), InspectionError> {
    ensure_text(&request.source_cwd, "source_cwd")?;
    ensure_text(&request.checkout_path, "checkout_path")?;
    ensure_text(&request.label, "label")?;
    if !Path::new(&request.source_cwd).is_absolute()
        || !Path::new(&request.checkout_path).is_absolute()
    {
        return Err(invalid("cwd/path"));
    }
    if let Some(branch) = request.branch.as_deref() {
        ensure_text(branch, "branch")?;
    }
    if let Some(base) = request.base.as_deref() {
        ensure_text(base, "base")?;
    }
    let mut params = Map::new();
    params.insert("cwd".to_owned(), json!(request.source_cwd));
    params.insert("focus".to_owned(), json!(request.focus));
    params.insert("label".to_owned(), json!(request.label));
    match request.mode {
        WorkspaceSetupMode::Create => {
            if let Some(base) = &request.base {
                params.insert("base".to_owned(), json!(base));
            }
            if let Some(branch) = &request.branch {
                params.insert("branch".to_owned(), json!(branch));
            }
            params.insert("path".to_owned(), json!(request.checkout_path));
            Ok(("worktree.create", Value::Object(params)))
        }
        WorkspaceSetupMode::Open => {
            if request.open_existing_worktree {
                params.insert("path".to_owned(), json!(request.checkout_path));
                Ok(("worktree.open", Value::Object(params)))
            } else {
                params.insert("cwd".to_owned(), json!(request.checkout_path));
                params.insert("env".to_owned(), sanitized_env(&request.env)?);
                Ok(("workspace.create", Value::Object(params)))
            }
        }
    }
}

fn parse_workspace_result(
    value: &Value,
    checkout_path: &str,
) -> Result<ProjectWorktreeResult, InspectionError> {
    let result = object(value, "workspace result")?;
    if required_string(result, "type", "workspace result")? != "workspace_created" {
        return Err(InspectionError::new(
            "malformed_workspace_response",
            "workspace.create returned an unexpected result type",
        ));
    }
    let workspace = object(
        result.get("workspace").ok_or_else(|| {
            InspectionError::new("malformed_workspace_response", "workspace is required")
        })?,
        "workspace result.workspace",
    )?;
    let workspace_id = required_string(workspace, "workspace_id", "workspace result.workspace")?;
    let tab = object(
        result.get("tab").ok_or_else(|| {
            InspectionError::new("malformed_workspace_response", "tab is required")
        })?,
        "workspace result.tab",
    )?;
    let tab_id = required_string(tab, "tab_id", "workspace result.tab")?;
    let tab_workspace = required_string(tab, "workspace_id", "workspace result.tab")?;
    let root_pane = object(
        result.get("root_pane").ok_or_else(|| {
            InspectionError::new("malformed_workspace_response", "root_pane is required")
        })?,
        "workspace result.root_pane",
    )?;
    let pane_id = required_string(root_pane, "pane_id", "workspace result.root_pane")?;
    let pane_workspace = required_string(root_pane, "workspace_id", "workspace result.root_pane")?;
    let pane_tab = required_string(root_pane, "tab_id", "workspace result.root_pane")?;
    let cwd = required_string(root_pane, "cwd", "workspace result.root_pane")?;
    if !valid_pane_id(&workspace_id)
        || !valid_pane_id(&tab_id)
        || !valid_pane_id(&pane_id)
        || tab_workspace != workspace_id
        || pane_workspace != workspace_id
        || pane_tab != tab_id
        || cwd != checkout_path
    {
        return Err(InspectionError::new(
            "malformed_identity",
            "workspace.create returned resources or cwd that differ from the request",
        ));
    }
    Ok(ProjectWorktreeResult {
        workspace_id,
        tab_id: Some(tab_id),
        pane_id: Some(pane_id),
        checkout_path: checkout_path.to_owned(),
        branch: None,
        already_open: false,
    })
}

fn parse_worktree_result(
    value: &Value,
    mode: WorkspaceSetupMode,
    checkout_path: &str,
    requested_branch: Option<&str>,
) -> Result<ProjectWorktreeResult, InspectionError> {
    let result = object(value, "worktree result")?;
    let expected_type = match mode {
        WorkspaceSetupMode::Create => "worktree_created",
        WorkspaceSetupMode::Open => "worktree_opened",
    };
    if required_string(result, "type", "worktree result")? != expected_type {
        return Err(InspectionError::new(
            "malformed_worktree_response",
            "unexpected worktree result type",
        ));
    }
    let workspace = object(
        result.get("workspace").ok_or_else(|| {
            InspectionError::new("malformed_worktree_response", "workspace is required")
        })?,
        "worktree result.workspace",
    )?;
    let workspace_id = required_string(workspace, "workspace_id", "worktree result.workspace")?;
    if !valid_pane_id(&workspace_id) {
        return Err(InspectionError::new(
            "malformed_identity",
            "returned workspace identity is malformed",
        ));
    }
    let tab = object(
        result.get("tab").ok_or_else(|| {
            InspectionError::new("malformed_worktree_response", "tab is required")
        })?,
        "worktree result.tab",
    )?;
    let tab_id = required_string(tab, "tab_id", "worktree result.tab")?;
    let tab_workspace = required_string(tab, "workspace_id", "worktree result.tab")?;
    let root_pane = object(
        result.get("root_pane").ok_or_else(|| {
            InspectionError::new("malformed_worktree_response", "root_pane is required")
        })?,
        "worktree result.root_pane",
    )?;
    let pane_id = required_string(root_pane, "pane_id", "worktree result.root_pane")?;
    let pane_workspace = required_string(root_pane, "workspace_id", "worktree result.root_pane")?;
    let pane_tab = required_string(root_pane, "tab_id", "worktree result.root_pane")?;
    if !valid_pane_id(&tab_id)
        || !valid_pane_id(&pane_id)
        || tab_workspace != workspace_id
        || pane_workspace != workspace_id
        || pane_tab != tab_id
    {
        return Err(InspectionError::new(
            "malformed_identity",
            "returned worktree resource identities do not agree",
        ));
    }
    let worktree = object(
        result.get("worktree").ok_or_else(|| {
            InspectionError::new("malformed_worktree_response", "worktree is required")
        })?,
        "worktree result.worktree",
    )?;
    let returned_path = required_string(worktree, "path", "worktree result.worktree")?;
    if returned_path != checkout_path {
        return Err(InspectionError::new(
            "provenance_conflict",
            "returned checkout path differs from the requested path",
        ));
    }
    let branch = optional_string(worktree, "branch", "worktree result.worktree")?;
    if let Some(requested) = requested_branch
        && branch.as_deref() != Some(requested)
    {
        return Err(InspectionError::new(
            "provenance_conflict",
            "returned branch differs from the requested branch",
        ));
    }
    let open_workspace =
        optional_string(worktree, "open_workspace_id", "worktree result.worktree")?;
    if open_workspace.as_deref() != Some(workspace_id.as_str()) {
        return Err(InspectionError::new(
            "malformed_identity",
            "worktree open_workspace_id does not match workspace",
        ));
    }
    let already_open = match mode {
        WorkspaceSetupMode::Create => false,
        WorkspaceSetupMode::Open => required_bool(result, "already_open", "worktree result")?,
    };
    Ok(ProjectWorktreeResult {
        workspace_id,
        tab_id: Some(tab_id),
        pane_id: Some(pane_id),
        checkout_path: returned_path,
        branch,
        already_open,
    })
}

fn sanitized_env(env: &BTreeMap<String, String>) -> Result<Value, InspectionError> {
    if env.len() > MAX_ENV_ENTRIES {
        return Err(invalid("env"));
    }
    let mut result = Map::new();
    for (key, value) in env {
        if !key.starts_with("COCKPIT_")
            || key.len() > 64
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(InspectionError::new(
                "invalid_environment",
                "environment key is not allowlisted",
            ));
        }
        ensure_text(value, "env value")?;
        result.insert(key.clone(), Value::String(value.clone()));
    }
    Ok(Value::Object(result))
}

fn parse_tab_result(value: &Value, workspace_id: &str) -> Result<String, InspectionError> {
    let result = object(value, "tab result")?;
    if required_string(result, "type", "tab result")? != "tab_created" {
        return Err(InspectionError::new(
            "malformed_tab_response",
            "tab result.type must be tab_created",
        ));
    }
    let tab = object(
        result.get("tab").ok_or_else(|| {
            InspectionError::new("malformed_tab_response", "tab result.tab is required")
        })?,
        "tab result.tab",
    )?;
    let tab_id = required_string(tab, "tab_id", "tab result.tab")?;
    let returned_workspace = required_string(tab, "workspace_id", "tab result.tab")?;
    if returned_workspace != workspace_id || !valid_pane_id(&tab_id) {
        return Err(InspectionError::new(
            "malformed_identity",
            "returned tab identity does not match workspace",
        ));
    }
    Ok(tab_id)
}

fn parse_workspace_closed(value: &Value) -> Result<(), InspectionError> {
    let result = object(value, "workspace.close result")?;
    if required_string(result, "type", "workspace.close result")? != "workspace_closed" {
        return Err(InspectionError::new(
            "malformed_workspace_response",
            "workspace.close returned an unexpected result",
        ));
    }
    Ok(())
}

fn parse_worktree_removed(
    value: &Value,
    request: &ProjectWorktreeRemoveRequest,
) -> Result<(), InspectionError> {
    let result = object(value, "worktree.remove result")?;
    if required_string(result, "type", "worktree.remove result")? != "worktree_removed"
        || required_string(result, "workspace_id", "worktree.remove result")?
            != request.workspace_id
        || required_bool(result, "forced", "worktree.remove result")? != request.force
    {
        return Err(InspectionError::new(
            "malformed_worktree_response",
            "worktree.remove returned an unexpected result",
        ));
    }
    if required_string(result, "path", "worktree.remove result")? != request.checkout_path {
        return Err(InspectionError::new(
            "provenance_conflict",
            "worktree.remove returned a different checkout path",
        ));
    }
    Ok(())
}

#[async_trait]
impl ProjectHerdrAdapter for HerdrCliAdapter {
    async fn project_endpoint_identity(&self, session_id: &str) -> Result<String, InspectionError> {
        ensure_compatible(self.inspect_session(session_id).await?)?;
        let (_, endpoint_identity) = self
            .socket_request_with_identity(session_id, "session.snapshot", json!({}), None)
            .await?;
        ensure_endpoint_identity(&endpoint_identity)?;
        Ok(endpoint_identity)
    }

    async fn project_inventory(
        &self,
        session_id: &str,
        source_cwd: &str,
    ) -> Result<ProjectInventory, InspectionError> {
        ensure_text(source_cwd, "source_cwd")?;
        if !Path::new(source_cwd).is_absolute() {
            return Err(invalid("source_cwd"));
        }
        let methods = self.project_methods(session_id).await?;
        if !methods.contains("worktree.list") {
            return Err(unsupported("Herdr method worktree.list is unavailable"));
        }
        let (result, endpoint_identity) = self
            .socket_request_with_identity(
                session_id,
                "worktree.list",
                json!({"cwd": source_cwd}),
                None,
            )
            .await?;
        let (repository_key, repository_root, worktrees) =
            parse_worktree_list(&result, source_cwd)?;
        let supported_methods = methods.into_iter().collect();
        Ok(ProjectInventory {
            endpoint_identity,
            repository_key,
            repository_root,
            worktrees,
            supported_methods,
        })
    }

    async fn project_worktree(
        &self,
        session_id: &str,
        request: &ProjectWorktreeRequest,
    ) -> Result<ProjectWorktreeResult, InspectionError> {
        ensure_endpoint_identity(&request.endpoint_identity)?;
        let methods = self.project_methods(session_id).await?;
        let (method, params) = worktree_params(request)?;
        if !methods.contains(method) {
            return Err(unsupported(format!("Herdr method {method} is unavailable")));
        }
        let (result, _) = self
            .socket_request_with_identity(
                session_id,
                method,
                params,
                Some(&request.endpoint_identity),
            )
            .await?;
        match request.mode {
            WorkspaceSetupMode::Create => parse_worktree_result(
                &result,
                request.mode,
                &request.checkout_path,
                request.branch.as_deref(),
            ),
            WorkspaceSetupMode::Open if request.open_existing_worktree => parse_worktree_result(
                &result,
                request.mode,
                &request.checkout_path,
                request.branch.as_deref(),
            ),
            WorkspaceSetupMode::Open => parse_workspace_result(&result, &request.checkout_path),
        }
    }

    async fn project_terminal(
        &self,
        session_id: &str,
        request: &ProjectTerminalRequest,
    ) -> Result<ProjectTerminalResult, InspectionError> {
        ensure_endpoint_identity(&request.endpoint_identity)?;
        let methods = self.project_methods(session_id).await?;
        if !methods.contains("tab.create") {
            return Err(unsupported("Herdr method tab.create is unavailable"));
        }
        ensure_text(&request.workspace_id, "workspace_id")?;
        ensure_text(&request.cwd, "cwd")?;
        ensure_text(&request.label, "label")?;
        if !valid_pane_id(&request.workspace_id) || !Path::new(&request.cwd).is_absolute() {
            return Err(invalid("workspace_id/cwd"));
        }
        let params = json!({
            "workspace_id": request.workspace_id,
            "cwd": request.cwd,
            "label": request.label,
            "focus": request.focus,
            "env": sanitized_env(&request.env)?,
        });
        let (result, _) = self
            .socket_request_with_identity(
                session_id,
                "tab.create",
                params,
                Some(&request.endpoint_identity),
            )
            .await?;
        let tab_id = parse_tab_result(&result, &request.workspace_id)?;
        let snapshot = self
            .read_snapshot_with_identity(session_id, Some(&request.endpoint_identity))
            .await?;
        let panes: Vec<_> = snapshot
            .panes
            .iter()
            .filter(|pane| pane.space_id == request.workspace_id && pane.tab_id == tab_id)
            .collect();
        if panes.len() != 1 || !valid_pane_id(&panes[0].id) {
            return Err(InspectionError::new(
                "malformed_identity",
                "tab.create did not yield exactly one authoritative root pane",
            ));
        }
        Ok(ProjectTerminalResult {
            workspace_id: request.workspace_id.clone(),
            tab_id,
            pane_id: panes[0].id.clone(),
        })
    }

    async fn project_worktree_dirty(
        &self,
        checkout_path: &str,
        timeout_ms: u32,
        output_bytes: u32,
    ) -> Result<bool, InspectionError> {
        ensure_text(checkout_path, "checkout_path")?;
        if !Path::new(checkout_path).is_absolute() || timeout_ms == 0 || output_bytes == 0 {
            return Err(invalid("worktree status"));
        }
        let mut command = Command::new("git");
        command
            .current_dir(checkout_path)
            .arg("-c")
            .arg("core.hooksPath=/dev/null")
            .arg("-c")
            .arg("core.fsmonitor=false")
            .arg("status")
            .arg("--porcelain=v1")
            .arg("--untracked-files=all")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "");
        let output = cockpit_core::process::run_bounded_command(
            command,
            output_bytes as usize,
            output_bytes as usize,
            Duration::from_millis(timeout_ms as u64),
            "worktree_status",
        )
        .await?;
        if !output.status.success() {
            return Err(InspectionError::new(
                "worktree_status_failed",
                "Git could not determine worktree status",
            ));
        }
        Ok(!output.stdout.is_empty())
    }

    async fn project_close_workspace(
        &self,
        session_id: &str,
        endpoint_identity: &str,
        workspace_id: &str,
    ) -> Result<(), InspectionError> {
        ensure_endpoint_identity(endpoint_identity)?;
        ensure_text(workspace_id, "workspace_id")?;
        if !valid_pane_id(workspace_id) {
            return Err(invalid("workspace_id"));
        }
        let methods = self.project_methods(session_id).await?;
        if !methods.contains("workspace.close") {
            return Err(unsupported("Herdr method workspace.close is unavailable"));
        }
        let (result, _) = self
            .socket_request_with_identity(
                session_id,
                "workspace.close",
                json!({"workspace_id": workspace_id}),
                Some(endpoint_identity),
            )
            .await?;
        parse_workspace_closed(&result)
    }

    async fn project_remove_worktree(
        &self,
        session_id: &str,
        request: &ProjectWorktreeRemoveRequest,
    ) -> Result<(), InspectionError> {
        ensure_endpoint_identity(&request.endpoint_identity)?;
        ensure_text(&request.workspace_id, "workspace_id")?;
        ensure_text(&request.checkout_path, "checkout_path")?;
        if !valid_pane_id(&request.workspace_id)
            || !Path::new(&request.checkout_path).is_absolute()
            || request.force
        {
            return Err(invalid("worktree removal"));
        }
        let methods = self.project_methods(session_id).await?;
        if !methods.contains("worktree.remove") {
            return Err(unsupported("Herdr method worktree.remove is unavailable"));
        }
        let (result, _) = self
            .socket_request_with_identity(
                session_id,
                "worktree.remove",
                json!({"workspace_id": request.workspace_id, "force": false}),
                Some(&request.endpoint_identity),
            )
            .await?;
        parse_worktree_removed(&result, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worktree(path: &str, branch: Option<&str>, linked: bool, open: Option<&str>) -> Value {
        json!({
            "path": path,
            "branch": branch,
            "is_bare": false,
            "is_detached": branch.is_none(),
            "is_prunable": false,
            "is_linked_worktree": linked,
            "label": "task",
            "open_workspace_id": open,
        })
    }

    #[test]
    fn parses_exact_worktree_list_provenance() {
        let value = json!({
            "type": "worktree_list",
            "source": {"repo_key": "repo", "repo_name": "repo", "repo_root": "/repo", "source_checkout_path": "/repo"},
            "worktrees": [worktree("/repo", Some("main"), false, Some("w1")), worktree("/repo-task", Some("task"), true, Some("w2"))]
        });
        let (key, root, entries) = parse_worktree_list(&value, "/repo").unwrap();
        assert_eq!(key, "repo");
        assert_eq!(root, "/repo");
        assert!(entries[0].is_primary);
        assert_eq!(entries[1].open_workspace_id.as_deref(), Some("w2"));
    }

    #[test]
    fn rejects_reused_identity_and_wrong_source() {
        let value = json!({
            "type": "worktree_list",
            "source": {"repo_key": "repo", "repo_root": "/repo", "source_checkout_path": "/other"},
            "worktrees": [worktree("/repo", Some("main"), false, Some("w1")), worktree("/repo-task", Some("task"), true, Some("w1"))]
        });
        assert!(parse_worktree_list(&value, "/repo").is_err());
    }

    #[test]
    fn maps_create_and_directory_open_without_workspace_id() {
        let request = ProjectWorktreeRequest {
            endpoint_identity: "unix-socket:/tmp/herdr.sock:pid=1:uid=1:gid=1:start=1".into(),
            mode: WorkspaceSetupMode::Create,
            source_cwd: "/repo".into(),
            branch: Some("task".into()),
            base: Some("main".into()),
            checkout_path: "/task".into(),
            label: "Task".into(),
            focus: true,
            env: BTreeMap::new(),
            open_existing_worktree: false,
        };
        let (method, params) = worktree_params(&request).unwrap();
        assert_eq!(method, "worktree.create");
        assert_eq!(params.get("cwd").and_then(Value::as_str), Some("/repo"));
        assert!(params.get("workspace_id").is_none());
        let open = ProjectWorktreeRequest {
            mode: WorkspaceSetupMode::Open,
            ..request
        };
        let (method, params) = worktree_params(&open).unwrap();
        assert_eq!(method, "workspace.create");
        assert_eq!(params.get("cwd").and_then(Value::as_str), Some("/task"));
        assert!(params.get("env").is_some());
        assert!(params.get("workspace_id").is_none());
    }

    #[test]
    fn parses_worktree_and_tab_result_identity() {
        let value = json!({
            "type": "worktree_created",
            "workspace": {"workspace_id": "w1"},
            "tab": {"tab_id": "w1:t1", "workspace_id": "w1"},
            "root_pane": {"pane_id": "w1:p1", "workspace_id": "w1", "tab_id": "w1:t1"},
            "worktree": {"path": "/task", "branch": "task", "open_workspace_id": "w1"}
        });
        let parsed =
            parse_worktree_result(&value, WorkspaceSetupMode::Create, "/task", Some("task"))
                .unwrap();
        assert_eq!(parsed.workspace_id, "w1");
        assert_eq!(parsed.tab_id.as_deref(), Some("w1:t1"));
        assert_eq!(
            parse_tab_result(
                &json!({"type":"tab_created", "tab":{"tab_id":"w1:t2", "workspace_id":"w1"}}),
                "w1"
            )
            .unwrap(),
            "w1:t2"
        );
    }

    #[test]
    fn parses_installed_worktree_removal_receipt() {
        let request = ProjectWorktreeRemoveRequest {
            endpoint_identity: "unix-socket:/tmp/herdr.sock:pid=1:uid=1:gid=1:start=1".into(),
            workspace_id: "w2".into(),
            checkout_path: "/task".into(),
            force: false,
        };
        let result = json!({
            "type": "worktree_removed",
            "workspace_id": "w2",
            "forced": false,
            "path": "/task"
        });

        parse_worktree_removed(&result, &request).expect("installed Herdr removal receipt");
        for (field, replacement) in [
            ("workspace_id", json!("w3")),
            ("path", json!("/another-task")),
            ("forced", json!(true)),
        ] {
            let mut mismatched = result.clone();
            mismatched[field] = replacement;
            assert!(parse_worktree_removed(&mismatched, &request).is_err());
        }
    }

    #[test]
    fn rejects_unsupported_or_malformed_result() {
        assert!(parse_tab_result(&json!({"type":"other"}), "w1").is_err());
        assert!(
            parse_worktree_result(
                &json!({"type":"worktree_opened"}),
                WorkspaceSetupMode::Open,
                "/task",
                None
            )
            .is_err()
        );
        assert!(
            sanitized_env(&BTreeMap::from([(
                String::from("PATH"),
                String::from("secret")
            )]))
            .is_err()
        );
    }
}
