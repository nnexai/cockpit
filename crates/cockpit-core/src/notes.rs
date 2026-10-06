//! Durable ordinary-Markdown Notes. UUID targets intentionally need no Herdr or registry.
mod comments;
mod decisions;
mod fs;
mod registry;
mod todos;

use crate::{InspectionError, ProjectHerdrAdapter};
use cockpit_protocol::notes::*;
use registry::Authority;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct NotesService {
    root: PathBuf,
    herdr: Option<Arc<dyn ProjectHerdrAdapter>>,
}
impl NotesService {
    pub fn new(root: PathBuf) -> Self {
        Self { root, herdr: None }
    }
    pub fn with_herdr(mut self, adapter: Arc<dyn ProjectHerdrAdapter>) -> Self {
        self.herdr = Some(adapter);
        self
    }
    pub fn root_path(&self) -> &Path {
        &self.root
    }
    async fn authorize(
        &self,
        session_id: &str,
        space_id: &str,
    ) -> Result<Authority, InspectionError> {
        if [session_id, space_id]
            .iter()
            .any(|v| v.is_empty() || v.len() > 256 || v.chars().any(char::is_control))
        {
            return Err(fs::error("notes_invalid_target", "Invalid Space target"));
        }
        let adapter = self.herdr.as_ref().ok_or_else(|| {
            fs::error(
                "notes_space_unavailable",
                "Herdr is unavailable for Space targets",
            )
        })?;
        let endpoint = adapter.project_endpoint_identity(session_id).await?;
        if endpoint.is_empty() || endpoint.len() > 4096 || endpoint.chars().any(char::is_control) {
            return Err(fs::error(
                "stale_identity",
                "Herdr endpoint identity is missing or invalid",
            ));
        }
        let snapshot = adapter.session_snapshot(session_id).await?;
        if snapshot.session_id != session_id {
            return Err(fs::error(
                "stale_identity",
                "Herdr returned a different session",
            ));
        }
        let space = snapshot
            .spaces
            .iter()
            .find(|s| s.id == space_id)
            .ok_or_else(|| {
                fs::error(
                    "notes_space_unavailable",
                    "Space is absent from the fresh Herdr snapshot",
                )
            })?;
        if adapter.project_endpoint_identity(session_id).await? != endpoint {
            return Err(fs::error(
                "stale_identity",
                "Herdr endpoint changed while resolving Notes",
            ));
        }
        if space.label.len() > 4096 || space.label.chars().any(char::is_control) {
            return Err(fs::error(
                "notes_invalid_target",
                "Space label is not bounded text",
            ));
        }
        Ok(Authority {
            endpoint,
            session_id: session_id.into(),
            space_id: space_id.into(),
            label: space.label.clone(),
        })
    }
    pub async fn execute(&self, request: NotesRequest) -> Result<NotesResponse, InspectionError> {
        let authority = match &request.target {
            NotesTarget::Root if matches!(request.operation, NotesOperation::CatalogList) => None,
            NotesTarget::Root => {
                return Err(fs::error(
                    "notes_target_required",
                    "This operation requires a Notes UUID target",
                ));
            }
            NotesTarget::Space {
                session_id,
                space_id,
            } => {
                if !matches!(
                    request.operation,
                    NotesOperation::TargetResolve
                        | NotesOperation::TargetCreate
                        | NotesOperation::TargetAttach { .. }
                ) {
                    return Err(fs::error(
                        "notes_target_required",
                        "Resolve the Space once, then pin its Notes UUID for every content operation",
                    ));
                }
                Some(self.authorize(session_id, space_id).await?)
            }
            NotesTarget::Notes { notes_id } => {
                fs::validate_uuid(notes_id)?;
                if matches!(
                    request.operation,
                    NotesOperation::CatalogList
                        | NotesOperation::TargetCreate
                        | NotesOperation::TargetAttach { .. }
                ) {
                    return Err(fs::error(
                        "notes_invalid_target",
                        "Catalog requires Root; create and attach require Space",
                    ));
                }
                None
            }
        };
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || execute_blocking(&root, request, authority))
            .await
            .map_err(|e| {
                fs::error(
                    "notes_outcome_unknown",
                    format!("Notes worker did not complete: {e}; re-read before retrying"),
                )
            })?
    }
}
fn mutates(op: &NotesOperation) -> bool {
    !matches!(
        op,
        NotesOperation::CatalogList
            | NotesOperation::TargetResolve
            | NotesOperation::ScratchpadRead
            | NotesOperation::TodoList { .. }
            | NotesOperation::KanbanList
            | NotesOperation::DecisionList { .. }
            | NotesOperation::DecisionGet { .. }
            | NotesOperation::CommentList { .. }
            | NotesOperation::CommentGet { .. }
    )
}
fn execute_blocking(
    root_path: &Path,
    request: NotesRequest,
    authority: Option<Authority>,
) -> Result<NotesResponse, InspectionError> {
    let create = matches!(request.operation, NotesOperation::TargetCreate);
    let (root_path, root) = match fs::root(root_path, create) {
        Err(error)
            if error.code == "notes_not_found"
                && authority.is_some()
                && matches!(request.target, NotesTarget::Space { .. })
                && matches!(request.operation, NotesOperation::TargetResolve) =>
        {
            return Err(fs::error(
                "notes_unbound",
                "This Space has no Notes association; explicitly create or attach Notes",
            ));
        }
        result => result?,
    };
    if matches!(request.operation, NotesOperation::CatalogList) {
        return Ok(NotesResponse {
            notes_id: None,
            changed: false,
            result: registry::catalog(&root)?,
        });
    }
    let (id, binding_changed) = match &request.target {
        NotesTarget::Notes { notes_id } => (notes_id.clone(), false),
        NotesTarget::Space { .. } => {
            let auth = authority.as_ref().expect("Space was freshly authorized");
            match &request.operation {
                NotesOperation::TargetCreate => registry::bind(&root, auth, None)?,
                NotesOperation::TargetAttach { notes_id } => {
                    registry::bind(&root, auth, Some(notes_id))?
                }
                _ => (registry::resolve(&root, auth)?, false),
            }
        }
        NotesTarget::Root => {
            return Err(fs::error(
                "notes_target_required",
                "Notes UUID target required",
            ));
        }
    };
    let dir = fs::child(&root, &id, false)?;
    let folder = root_path.join(&id);
    if matches!(
        request.operation,
        NotesOperation::TargetResolve
            | NotesOperation::TargetCreate
            | NotesOperation::TargetAttach { .. }
    ) {
        return Ok(NotesResponse {
            notes_id: Some(id.clone()),
            changed: binding_changed,
            result: registry::target(&folder, &dir, id, authority.as_ref())?,
        });
    }
    // Persistent lock files are outside the ordinary Markdown folder. A bad registry
    // cannot prevent a pinned UUID from reading or writing its files.
    let _lock = if mutates(&request.operation) {
        let state = fs::child(&root, ".cockpit", true)?;
        let locks = fs::child(&state, "locks", true)?;
        Some(fs::lock(&locks, &format!("{id}.lock"))?)
    } else {
        None
    };
    let (changed, result) = match request.operation {
        op @ (NotesOperation::ScratchpadRead
        | NotesOperation::ScratchpadAppend { .. }
        | NotesOperation::ScratchpadReplace { .. }) => scratchpad(&dir, op)?,
        op @ (NotesOperation::TodoList { .. }
        | NotesOperation::TodoAdd { .. }
        | NotesOperation::TodoUpdate { .. }
        | NotesOperation::TodoSetDone { .. }
        | NotesOperation::TodoRemove { .. }
        | NotesOperation::KanbanList
        | NotesOperation::KanbanPromote { .. }
        | NotesOperation::KanbanMove { .. }
        | NotesOperation::KanbanUnboard { .. }) => todos::execute(&dir, &folder, op)?,
        op @ (NotesOperation::DecisionList { .. }
        | NotesOperation::DecisionGet { .. }
        | NotesOperation::DecisionCreate { .. }
        | NotesOperation::DecisionUpdate { .. }
        | NotesOperation::DecisionReplace { .. }) => decisions::execute(&dir, &folder, op)?,
        op @ (NotesOperation::CommentList { .. }
        | NotesOperation::CommentGet { .. }
        | NotesOperation::CommentAdd { .. }
        | NotesOperation::CommentUpdate { .. }
        | NotesOperation::CommentRemove { .. }) => comments::execute(&dir, &folder, op)?,
        _ => {
            return Err(fs::error(
                "notes_usage",
                "Unsupported operation for this target",
            ));
        }
    };
    Ok(NotesResponse {
        notes_id: Some(id),
        changed,
        result,
    })
}
fn scratchpad(
    dir: &cap_std::fs::Dir,
    op: NotesOperation,
) -> Result<(bool, NotesResult), InspectionError> {
    const MAX: usize = 1024 * 1024;
    let base = fs::read(dir, "scratchpad.md", MAX)?;
    let content = match op {
        NotesOperation::ScratchpadRead => {
            return Ok((false, NotesResult::Scratchpad { document: base }));
        }
        NotesOperation::ScratchpadAppend {
            text,
            expected_revision,
        } => {
            if expected_revision
                .as_ref()
                .is_some_and(|r| r != &base.revision)
            {
                return Err(fs::error(
                    "notes_conflict",
                    "Scratchpad changed; re-read before retrying",
                ));
            }
            if text.is_empty() {
                return Ok((false, NotesResult::Scratchpad { document: base }));
            }
            let separator = if base.content.is_empty() || base.content.ends_with('\n') {
                ""
            } else if base.content.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            if base
                .content
                .len()
                .saturating_add(separator.len())
                .saturating_add(text.len())
                > MAX
            {
                return Err(fs::error("notes_too_large", "Scratchpad exceeds 1 MiB"));
            }
            format!("{}{separator}{text}", base.content)
        }
        NotesOperation::ScratchpadReplace {
            content,
            expected_revision,
        } => {
            if expected_revision != base.revision {
                return Err(fs::error(
                    "notes_conflict",
                    "Scratchpad changed; draft remains unsaved",
                ));
            }
            content
        }
        _ => return Err(fs::error("notes_usage", "Invalid scratchpad operation")),
    };
    if content.len() > MAX {
        return Err(fs::error("notes_too_large", "Scratchpad exceeds 1 MiB"));
    }
    if content == base.content {
        return Ok((false, NotesResult::Scratchpad { document: base }));
    }
    fs::publish(dir, "scratchpad.md", &base, &content, MAX)?;
    Ok((
        true,
        NotesResult::Scratchpad {
            document: NotesDocument {
                revision: fs::revision(content.as_bytes()),
                content,
            },
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        id: String,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("cockpit-notes-test-{}", uuid::Uuid::new_v4()));
            let id = uuid::Uuid::new_v4().to_string();
            std::fs::create_dir_all(root.join(&id)).unwrap();
            Self { root, id }
        }
        fn service(&self) -> NotesService {
            NotesService::new(self.root.clone())
        }
        fn request(&self, operation: NotesOperation) -> NotesRequest {
            NotesRequest {
                target: NotesTarget::Notes {
                    notes_id: self.id.clone(),
                },
                operation,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[tokio::test]
    async fn notes_pinned_uuid_survives_corrupt_registry_and_stale_cas() {
        let f = Fixture::new();
        std::fs::create_dir(f.root.join(".cockpit")).unwrap();
        std::fs::write(f.root.join(".cockpit/registry.json"), "broken").unwrap();
        let service = f.service();
        let response = service
            .execute(f.request(NotesOperation::ScratchpadAppend {
                text: "first 🧭".into(),
                expected_revision: None,
            }))
            .await
            .unwrap();
        let NotesResult::Scratchpad { document } = response.result else {
            panic!()
        };
        service
            .execute(f.request(NotesOperation::ScratchpadAppend {
                text: "second".into(),
                expected_revision: None,
            }))
            .await
            .unwrap();
        assert_eq!(
            service
                .execute(f.request(NotesOperation::ScratchpadReplace {
                    content: "lost".into(),
                    expected_revision: document.revision
                }))
                .await
                .unwrap_err()
                .code,
            "notes_conflict"
        );
        assert_eq!(
            std::fs::read_to_string(f.root.join(&f.id).join("scratchpad.md")).unwrap(),
            "first 🧭\nsecond"
        );
        assert_eq!(
            service
                .execute(NotesRequest {
                    target: NotesTarget::Root,
                    operation: NotesOperation::CatalogList
                })
                .await
                .unwrap_err()
                .code,
            "notes_registry_corrupt"
        );
        assert_eq!(
            std::fs::read_to_string(f.root.join(".cockpit/registry.json")).unwrap(),
            "broken"
        );
    }
    #[tokio::test]
    async fn notes_cooperating_appends_do_not_lose_updates() {
        let f = Fixture::new();
        let service = f.service();
        let mut tasks = vec![];
        for n in 0..16 {
            let s = service.clone();
            let request = f.request(NotesOperation::ScratchpadAppend {
                text: format!("entry{n}"),
                expected_revision: None,
            });
            tasks.push(tokio::spawn(async move {
                s.execute(request).await.unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let contents = std::fs::read_to_string(f.root.join(&f.id).join("scratchpad.md")).unwrap();
        assert_eq!(contents.lines().count(), 16);
        for n in 0..16 {
            assert!(contents.lines().any(|line| line == format!("entry{n}")));
        }
    }
    #[tokio::test]
    async fn notes_read_does_not_create_missing_roots() {
        let root =
            std::env::temp_dir().join(format!("cockpit-notes-missing-{}", uuid::Uuid::new_v4()));
        let result = NotesService::new(root.clone())
            .execute(NotesRequest {
                target: NotesTarget::Root,
                operation: NotesOperation::CatalogList,
            })
            .await;
        assert_eq!(result.unwrap_err().code, "notes_not_found");
        assert!(!root.exists());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn notes_symlink_files_and_components_are_refused() {
        let f = Fixture::new();
        let external = f.root.join("external");
        std::fs::write(&external, "keep").unwrap();
        std::os::unix::fs::symlink(&external, f.root.join(&f.id).join("scratchpad.md")).unwrap();
        assert_eq!(
            f.service()
                .execute(f.request(NotesOperation::ScratchpadRead))
                .await
                .unwrap_err()
                .code,
            "notes_unsafe_path"
        );
        assert_eq!(
            f.service()
                .execute(f.request(NotesOperation::ScratchpadAppend {
                    text: "oops".into(),
                    expected_revision: None
                }))
                .await
                .unwrap_err()
                .code,
            "notes_unsafe_path"
        );
        assert_eq!(std::fs::read_to_string(external).unwrap(), "keep");
        let alias = f.root.join("alias");
        std::os::unix::fs::symlink(&f.root, &alias).unwrap();
        assert_eq!(
            NotesService::new(alias)
                .execute(f.request(NotesOperation::ScratchpadRead))
                .await
                .unwrap_err()
                .code,
            "notes_unsafe_path"
        );
    }
}

#[cfg(test)]
mod first_use_tests {
    use super::*;

    #[test]
    fn notes_authorized_space_without_root_is_unbound_until_explicit_create() {
        let root =
            std::env::temp_dir().join(format!("cockpit-notes-first-use-{}", uuid::Uuid::new_v4()));
        let authority = Authority {
            endpoint: "fresh-boot".into(),
            session_id: "session".into(),
            space_id: "w1".into(),
            label: "First-use Space".into(),
        };
        let request = |operation| NotesRequest {
            target: NotesTarget::Space {
                session_id: authority.session_id.clone(),
                space_id: authority.space_id.clone(),
            },
            operation,
        };
        assert_eq!(
            execute_blocking(
                &root,
                request(NotesOperation::TargetResolve),
                Some(authority.clone())
            )
            .unwrap_err()
            .code,
            "notes_unbound",
        );
        assert!(!root.exists());
        let created = execute_blocking(
            &root,
            request(NotesOperation::TargetCreate),
            Some(authority.clone()),
        )
        .unwrap();
        assert!(created.changed);
        let id = created.notes_id.unwrap();
        let resolved = execute_blocking(
            &root,
            request(NotesOperation::TargetResolve),
            Some(authority.clone()),
        )
        .unwrap();
        assert_eq!(resolved.notes_id.as_deref(), Some(id.as_str()));
        assert!(!resolved.changed);
        std::fs::remove_dir(root.join(&id)).unwrap();
        assert_eq!(
            execute_blocking(
                &root,
                request(NotesOperation::TargetResolve),
                Some(authority)
            )
            .unwrap_err()
            .code,
            "notes_not_found",
        );
        assert_eq!(
            execute_blocking(
                &root,
                NotesRequest {
                    target: NotesTarget::Notes { notes_id: id },
                    operation: NotesOperation::TargetResolve,
                },
                None
            )
            .unwrap_err()
            .code,
            "notes_not_found",
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
