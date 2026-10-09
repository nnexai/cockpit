use std::path::Path;
use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use cockpit_protocol::projects::{
    ProjectConfiguration, RepositoryCandidate, WorkspaceOperation, WorkspaceOperationState,
    WorkspaceSetupMode,
};
use cockpit_protocol::v1::{
    FocusRequest, FocusResponse, HerdrCompatibility, ResourceMutationRequest,
    ResourceMutationResponse, SessionListResponse, SessionSnapshotResponse, TerminalOpenRequest,
};

use crate::project_adapter::{
    ProjectInventory, ProjectTerminalRequest, ProjectTerminalResult, ProjectWorktreeRemoveRequest,
    ProjectWorktreeRequest, ProjectWorktreeResult,
};
use crate::repositories;
use crate::sources::{SourceFetchRequest, instance_authority};
use crate::{
    HerdrAdapter, InspectionError, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
};

use super::ProjectService;

#[derive(Default)]
pub(super) struct NestedDirectoryAdapter {
    pub(super) inventory_calls: AtomicUsize,
    pub(super) worktree_requests: StdMutex<Vec<ProjectWorktreeRequest>>,
    pub(super) terminal_requests: StdMutex<Vec<ProjectTerminalRequest>>,
    pub(super) closed_workspaces: StdMutex<Vec<String>>,
    pub(super) repository: std::sync::OnceLock<RepositoryCandidate>,
    pub(super) inventory: parking_lot::Mutex<Option<ProjectInventory>>,
    pub(super) selection_available: AtomicBool,
}

pub(super) fn unused<T>() -> Result<T, InspectionError> {
    Err(InspectionError::new(
        "test_adapter_unused",
        "test adapter method is not expected",
    ))
}

#[async_trait::async_trait]
impl HerdrAdapter for NestedDirectoryAdapter {
    async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
        unused()
    }

    async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> {
        unused()
    }

    async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
        unused()
    }

    async fn session_snapshot(
        &self,
        session: &str,
    ) -> Result<SessionSnapshotResponse, InspectionError> {
        if !self.selection_available.load(Ordering::SeqCst) {
            return Err(InspectionError::new("disconnected", "fixture unavailable"));
        }
        Ok(SessionSnapshotResponse {
            session_id: session.into(),
            version: "test".into(),
            protocol: 1,
            server_instance: "0123456789abcdef".into(),
            herdr_shell: None,
            focused_space_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            spaces: vec![cockpit_protocol::v1::SpaceSummary {
                id: "workspace".into(),
                label: "Setup".into(),
                number: 1,
                tab_count: 0,
                pane_count: 0,
                focused: false,
                agent_status: "none".into(),
                git: None,
            }],
            tabs: vec![],
            panes: vec![],
            agents: vec![],
        })
    }

    async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> {
        unused()
    }

    async fn mutate(
        &self,
        _: &str,
        _: &ResourceMutationRequest,
    ) -> Result<ResourceMutationResponse, InspectionError> {
        unused()
    }

    async fn subscribe_session(
        &self,
        _: &str,
        _: &SessionSnapshotResponse,
    ) -> Result<SessionSubscription, InspectionError> {
        unused()
    }

    async fn open_terminal(
        &self,
        _: &TerminalOpenRequest,
    ) -> Result<TerminalSession, InspectionError> {
        unused()
    }
}

#[async_trait::async_trait]
impl ProjectHerdrAdapter for NestedDirectoryAdapter {
    async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> {
        Ok("endpoint".to_owned())
    }

    async fn project_inventory(
        &self,
        _: &str,
        _: &str,
    ) -> Result<ProjectInventory, InspectionError> {
        self.inventory_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(inventory) = self.inventory.lock().clone() {
            return Ok(inventory);
        }
        let repository = self.repository.get().cloned();
        let Some(repository) = repository else {
            return unused();
        };
        Ok(ProjectInventory {
            endpoint_identity: "endpoint".into(),
            repository_key: repository.common_dir,
            repository_root: repository.root,
            supported_methods: vec![],
            worktrees: self
                .worktree_requests
                .lock()
                .unwrap()
                .iter()
                .map(|request| crate::project_adapter::ProjectWorktreeEntry {
                    checkout_path: request.checkout_path.clone(),
                    branch: request.branch.clone(),
                    open_workspace_id: Some("workspace".into()),
                    is_primary: false,
                    is_linked_worktree: true,
                    dirty: Some(false),
                })
                .collect(),
        })
    }

    async fn project_worktree(
        &self,
        _: &str,
        request: &ProjectWorktreeRequest,
    ) -> Result<ProjectWorktreeResult, InspectionError> {
        if request.mode == WorkspaceSetupMode::Create {
            git(
                Path::new(&request.source_cwd),
                &[
                    "worktree",
                    "add",
                    "-b",
                    request.branch.as_deref().unwrap(),
                    &request.checkout_path,
                ],
            );
        }
        self.worktree_requests
            .lock()
            .expect("worktree requests")
            .push(request.clone());
        Ok(ProjectWorktreeResult {
            workspace_id: "workspace".to_owned(),
            tab_id: Some("workspace:root".to_owned()),
            pane_id: None,
            checkout_path: request.checkout_path.clone(),
            branch: request.branch.clone(),
            already_open: false,
        })
    }

    async fn project_terminal(
        &self,
        _: &str,
        request: &ProjectTerminalRequest,
    ) -> Result<ProjectTerminalResult, InspectionError> {
        self.terminal_requests
            .lock()
            .expect("terminal requests")
            .push(request.clone());
        Ok(ProjectTerminalResult {
            workspace_id: request.workspace_id.clone(),
            tab_id: "workspace:context".to_owned(),
            pane_id: "workspace:context-pane".to_owned(),
        })
    }

    async fn project_worktree_dirty(
        &self,
        _: &str,
        _: u32,
        _: u32,
    ) -> Result<bool, InspectionError> {
        unused()
    }

    async fn project_close_workspace(
        &self,
        _: &str,
        _: &str,
        workspace_id: &str,
    ) -> Result<(), InspectionError> {
        self.closed_workspaces
            .lock()
            .expect("closed workspaces")
            .push(workspace_id.to_owned());
        Ok(())
    }

    async fn project_remove_worktree(
        &self,
        _: &str,
        _: &ProjectWorktreeRemoveRequest,
    ) -> Result<(), InspectionError> {
        unused()
    }
}

pub(super) fn configuration(root: &Path) -> ProjectConfiguration {
    ProjectConfiguration {
        notes_root: root.with_extension("notes").to_string_lossy().into_owned(),
        branch_template: "{repo}/{task_id}".to_owned(),
        checkout_template: "{repo}-{task_id}".to_owned(),
        ..ProjectConfiguration::for_tests(root)
    }
}

pub(super) fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(super) struct SetupProvider {
    pub(super) configuration: ProjectConfiguration,
    pub(super) provider_id: String,
    pub(super) calls: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl crate::sources::SourceProvider for SetupProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }
    fn capabilities(&self) -> Vec<cockpit_protocol::sources::SourceCapability> {
        vec![cockpit_protocol::sources::SourceCapability::Issue]
    }
    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<crate::sources::SourceAsset>, InspectionError> {
        let read = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let artifact = repositories::resolve_artifact(&self.configuration, &request.artifact_url)?;
        assert_eq!(
            request.authority,
            instance_authority(
                &self.configuration,
                &self.provider_id,
                &request.artifact_url
            )?
        );
        Ok(vec![crate::sources::SourceAsset {
            source: crate::sources::SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: artifact.kind,
                canonical_id: artifact.canonical_id.clone(),
            },
            title: artifact.canonical_id,
            source_url: Some(artifact.canonical_url),
            original_url: Some(request.artifact_url.clone()),
            source_revision: Some("1".into()),
            complete: true,
            diagnostics: vec![],
            body: format!("Setup context body: fetch {read}"),
            container: None,
            fields: vec![],
            attachments: vec![],
        }])
    }
}

pub(super) async fn settled_setup(service: &ProjectService, id: &str) -> WorkspaceOperation {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let operation = service.get("session", id).await.unwrap();
            if !matches!(
                operation.state,
                WorkspaceOperationState::Planned | WorkspaceOperationState::Running
            ) {
                break operation;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("setup settles")
}
