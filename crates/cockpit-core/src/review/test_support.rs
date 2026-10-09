use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cockpit_protocol::projects::ProjectConfiguration;
use cockpit_protocol::review::{
    ReviewComparison, ReviewFileDiff, ReviewFileRequest, ReviewFileStatus, ReviewSnapshot,
    ReviewSnapshotRequest,
};
use cockpit_protocol::viewer::ViewerKind;
use uuid::Uuid;

use crate::InspectionError;
use crate::context::ContextService;
use crate::extension_adapter::{SourcePaneAdapter, SourcePaneEvidence, TabEvidence};
use crate::repositories::RepositoryCatalog;

use super::ReviewService;
use super::parse::truncated_diff;

use cockpit_protocol::{
    projects::{ProjectLimits, ProjectProvider},
    v1::{
        FocusRequest, FocusResponse, HerdrCompatibility, ResourceMutationRequest,
        ResourceMutationResponse, SessionListResponse, SessionSnapshotResponse,
        TerminalOpenRequest,
    },
};

use crate::{
    HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
    project_adapter::{
        ProjectInventory, ProjectTerminalRequest, ProjectTerminalResult,
        ProjectWorktreeRemoveRequest, ProjectWorktreeRequest, ProjectWorktreeResult,
    },
    projects::ProjectService,
};

#[derive(Default)]
pub(super) struct NoopAdapter {
    pub(super) source_evidence: Option<SourcePaneEvidence>,
    pub(super) source_closed: AtomicBool,
}

fn unavailable<T>() -> Result<T, InspectionError> {
    Err(InspectionError::new(
        "test_adapter_unused",
        "test adapter method is not expected",
    ))
}

#[async_trait::async_trait]
impl HerdrAdapter for NoopAdapter {
    async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
        unavailable()
    }
    async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> {
        unavailable()
    }
    async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
        unavailable()
    }
    async fn session_snapshot(&self, _: &str) -> Result<SessionSnapshotResponse, InspectionError> {
        unavailable()
    }
    async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> {
        unavailable()
    }
    async fn mutate(
        &self,
        _: &str,
        _: &ResourceMutationRequest,
    ) -> Result<ResourceMutationResponse, InspectionError> {
        unavailable()
    }
    async fn subscribe_session(
        &self,
        _: &str,
        _: &SessionSnapshotResponse,
    ) -> Result<SessionSubscription, InspectionError> {
        unavailable()
    }
    async fn open_terminal(
        &self,
        _: &TerminalOpenRequest,
    ) -> Result<TerminalSession, InspectionError> {
        unavailable()
    }
}

#[async_trait::async_trait]
impl ProjectHerdrAdapter for NoopAdapter {
    async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> {
        unavailable()
    }

    async fn project_inventory(
        &self,
        _: &str,
        _: &str,
    ) -> Result<ProjectInventory, InspectionError> {
        unavailable()
    }
    async fn project_worktree(
        &self,
        _: &str,
        _: &ProjectWorktreeRequest,
    ) -> Result<ProjectWorktreeResult, InspectionError> {
        unavailable()
    }
    async fn project_terminal(
        &self,
        _: &str,
        _: &ProjectTerminalRequest,
    ) -> Result<ProjectTerminalResult, InspectionError> {
        unavailable()
    }
    async fn project_worktree_dirty(
        &self,
        _: &str,
        _: u32,
        _: u32,
    ) -> Result<bool, InspectionError> {
        unavailable()
    }
    async fn project_close_workspace(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), InspectionError> {
        unavailable()
    }
    async fn project_remove_worktree(
        &self,
        _: &str,
        _: &ProjectWorktreeRemoveRequest,
    ) -> Result<(), InspectionError> {
        unavailable()
    }
}

#[async_trait::async_trait]
impl SourcePaneAdapter for NoopAdapter {
    async fn source_pane_evidence(
        &self,
        _: &str,
        _: &str,
    ) -> Result<SourcePaneEvidence, InspectionError> {
        if self.source_closed.load(Ordering::Relaxed) {
            return unavailable();
        }
        self.source_evidence
            .clone()
            .ok_or_else(|| InspectionError::new("test_adapter_unused", "source unavailable"))
    }
    async fn tab_evidence(&self, _: &str, _: &str) -> Result<TabEvidence, InspectionError> {
        Ok(TabEvidence {
            endpoint_identity: "endpoint".to_owned(),
            server_instance: "server".to_owned(),
            workspace_id: "workspace".to_owned(),
            present: true,
        })
    }
}

pub(super) fn configuration(root: &Path) -> ProjectConfiguration {
    ProjectConfiguration {
        notes_root: root.with_extension("notes").to_string_lossy().into_owned(),
        branch_template: "{repo}/{task_id}".to_owned(),
        checkout_template: "{repo}-{task_id}".to_owned(),
        providers: vec![ProjectProvider {
            id: "test".to_owned(),
            kind: cockpit_protocol::projects::ProviderKind::Gitea,
            base_url: "https://example.test/".to_owned(),
            executable: Some("false".to_owned()),
            login: None,
            deployment: None,
        }],
        limits: ProjectLimits {
            catalog_depth: 2,
            catalog_entries: 32,
            git_timeout_ms: 2_000,
            git_output_bytes: 2 * 1024 * 1024,
            operation_timeout_ms: 2_000,
            context_preview_bytes: 1024 * 1024,
            context_preview_lines: 2_000,
            context_directory_entries: 64,
            context_tree_depth: 8,
            library_folder_files: 512,
            library_folder_bytes: 32 * 1024 * 1024,
            library_file_bytes: 4 * 1024 * 1024,
            library_space_pages: 200,
            library_attachment_bytes: 25 * 1024 * 1024,
            library_item_attachment_bytes: 100 * 1024 * 1024,
            library_max_items: 20_000,
        },
        ..ProjectConfiguration::for_tests(root)
    }
}

pub(super) fn service(root: &Path) -> ReviewService {
    service_with_configuration(configuration(root))
}

pub(super) fn service_with_configuration(configuration: ProjectConfiguration) -> ReviewService {
    service_with_adapter(configuration, Arc::new(NoopAdapter::default()))
}

pub(super) fn service_with_adapter(
    configuration: ProjectConfiguration,
    adapter: Arc<NoopAdapter>,
) -> ReviewService {
    let projects = Arc::new(
        ProjectService::new(configuration.clone(), adapter.clone()).expect("project service"),
    );
    let context = ContextService::new(configuration.clone(), adapter.clone(), projects);
    let viewers = Arc::new(crate::viewer::ViewerService::new(Arc::new(context.clone())));
    ReviewService::new(configuration, Arc::new(context.with_viewers(viewers)))
        .expect("review service")
}

pub(super) fn fixture(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("cockpit-review-{label}-{}", Uuid::new_v4()));
    fixture_at(&root);
    root
}

pub(super) fn fixture_at(root: &Path) {
    std::fs::create_dir_all(&root).expect("fixture directory");
    for args in [
        ["init"].as_slice(),
        ["config", "user.email", "fixture@example.test"].as_slice(),
        ["config", "user.name", "Fixture"].as_slice(),
    ] {
        let status = std::process::Command::new("git")
            .current_dir(&root)
            .args(args)
            .status()
            .expect("git starts");
        assert!(status.success(), "git {args:?}");
    }
}

pub(super) fn git_bytes(root: &Path, args: &[&str]) -> Vec<u8> {
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
    output.stdout
}

pub(super) fn commit(root: &Path, message: &str) {
    git_bytes(root, &["add", "--all"]);
    git_bytes(root, &["commit", "-m", message]);
}

pub(super) async fn collect_real(
    service: &ReviewService,
    root: &Path,
    comparison: ReviewComparison,
    base_ref: Option<&str>,
) -> BTreeMap<String, ReviewFileDiff> {
    let request = ReviewSnapshotRequest {
        binding_id: "test-binding".to_owned(),
        repository_id: "test-repository".to_owned(),
        comparison,
        base_ref: base_ref.map(str::to_owned),
    };
    let revisions = service
        .revision_tokens(root)
        .await
        .expect("revision tokens");
    let base_revision = if comparison == ReviewComparison::Branch {
        Some(
            service
                .git_text(
                    root,
                    &["merge-base", "--", base_ref.expect("branch base"), "HEAD"],
                )
                .await
                .expect("merge base"),
        )
    } else {
        None
    };
    let (_, changes, _, _, base_revision) = service
        .collect(root, &request, &revisions, base_revision)
        .await
        .expect("real review collection");
    let mut diffs = BTreeMap::new();
    for (file_id, change) in changes {
        let result = service
            .diff(
                root,
                &file_id,
                &change,
                base_revision.as_deref(),
                &revisions,
            )
            .await;
        let (diff, _) = match result {
            Ok(value) => value,
            Err(error) if error.code == "bounded_output" => (
                truncated_diff(
                    &file_id,
                    &change,
                    base_revision.as_deref(),
                    &revisions,
                    "review_diff_bounded",
                    "file diff exceeds the configured read limit",
                ),
                true,
            ),
            Err(error) if error.code == "review_unreadable" => (
                truncated_diff(
                    &file_id,
                    &change,
                    base_revision.as_deref(),
                    &revisions,
                    "review_unreadable",
                    "file is unavailable or nonregular",
                ),
                false,
            ),
            Err(error) => panic!("real review file: {error:?}"),
        };
        diffs.insert(file_id, diff);
    }
    diffs
}

pub(super) struct ServiceFixture {
    pub(super) workspace: PathBuf,
    pub(super) checkout: PathBuf,
    pub(super) service: ReviewService,
    pub(super) viewers: Arc<crate::viewer::ViewerService>,
    pub(super) adapter: Arc<NoopAdapter>,
    pub(super) viewer_id: String,
    pub(super) binding_id: String,
    pub(super) repository_id: String,
}

pub(super) const FIXTURE_SESSION: &str = "session";
const FIXTURE_PANE: &str = "pane";

/// A real Git checkout below a workspace directory with a local Review
/// viewer authorized from a real source terminal, without any plugin proof.
pub(super) async fn service_fixture(label: &str) -> ServiceFixture {
    let workspace = std::env::temp_dir().join(format!("cockpit-review-{label}-{}", Uuid::new_v4()));
    let checkout = workspace.join("repo");
    fixture_at(&checkout);
    std::fs::write(checkout.join("tracked.txt"), "base\n").expect("write tracked base");
    commit(&checkout, "base");
    let configured = configuration(&workspace);
    let checkout_text = checkout.to_string_lossy().into_owned();
    let repository_id = RepositoryCatalog::new(configured.clone())
        .list()
        .await
        .expect("catalog listing")
        .repositories
        .into_iter()
        .find(|candidate| candidate.checkout_path == checkout_text)
        .expect("fixture checkout in catalog")
        .repository_id;
    let adapter = Arc::new(NoopAdapter {
        source_evidence: Some(SourcePaneEvidence {
            endpoint_identity: "endpoint".to_owned(),
            pane_id: FIXTURE_PANE.to_owned(),
            terminal_id: "terminal".to_owned(),
            workspace_id: "workspace".to_owned(),
            tab_id: "tab".to_owned(),
            cwd: Some(checkout_text),
            foreground_cwd: None,
        }),
        ..Default::default()
    });
    let projects =
        Arc::new(ProjectService::new(configured.clone(), adapter.clone()).expect("projects"));
    let context = ContextService::new(configured.clone(), adapter.clone(), projects);
    let viewers = Arc::new(crate::viewer::ViewerService::new(Arc::new(context.clone())));
    let context = Arc::new(context.with_viewers(viewers.clone()));
    let service = ReviewService::new(configured, context).expect("review service");
    let viewer = viewers
        .open(
            FIXTURE_SESSION,
            &cockpit_protocol::viewer::ViewerOpenRequest {
                tab_id: "tab".to_owned(),
                kind: ViewerKind::Review,
                source_pane_id: FIXTURE_PANE.to_owned(),
                source: cockpit_protocol::viewer::ViewerSourceSelector::Review {
                    repository_id: repository_id.clone(),
                },
                client_id: "client".to_owned(),
            },
        )
        .await
        .expect("open review viewer");
    let binding_id = viewer.binding_id;
    ServiceFixture {
        workspace,
        checkout,
        service,
        viewers,
        adapter,
        viewer_id: viewer.viewer_id,
        binding_id,
        repository_id,
    }
}

impl ServiceFixture {
    pub(super) fn request(&self, comparison: ReviewComparison) -> ReviewSnapshotRequest {
        ReviewSnapshotRequest {
            binding_id: self.binding_id.clone(),
            repository_id: self.repository_id.clone(),
            comparison,
            base_ref: None,
        }
    }

    pub(super) async fn snapshot(&self, comparison: ReviewComparison) -> ReviewSnapshot {
        self.service
            .snapshot(FIXTURE_SESSION, &self.viewer_id, &self.request(comparison))
            .await
            .expect("review snapshot")
    }

    pub(super) async fn file(&self, snapshot: &ReviewSnapshot, file_id: &str) -> ReviewFileDiff {
        self.service
            .file(
                FIXTURE_SESSION,
                &self.viewer_id,
                &ReviewFileRequest {
                    binding_id: self.binding_id.clone(),
                    review_id: snapshot.review_id.clone(),
                    generation: snapshot.generation,
                    file_id: file_id.to_owned(),
                    source_side: None,
                    source_offset: 0,
                    source_revision: None,
                },
            )
            .await
            .expect("review file diff")
    }
}

pub(super) fn untracked_file_id(snapshot: &ReviewSnapshot, path: &str) -> String {
    snapshot
        .files
        .iter()
        .find(|file| {
            file.status == ReviewFileStatus::Untracked && file.new_path.as_deref() == Some(path)
        })
        .unwrap_or_else(|| panic!("untracked {path} in snapshot"))
        .file_id
        .clone()
}

pub(super) fn diff_text(diff: &ReviewFileDiff) -> String {
    let mut text = diff
        .hunks
        .iter()
        .flat_map(|hunk| hunk.lines.iter())
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text.push_str(diff.new_source.as_deref().unwrap_or_default());
    text
}
