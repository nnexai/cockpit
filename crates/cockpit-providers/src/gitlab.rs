use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::process::run_bounded_command;
use cockpit_core::repositories::resolve_gitlab_artifact;
use cockpit_core::sources::{
    SourceAsset, SourceFetchRequest, SourceMetadata, SourceProvider, SourceRef,
};
use cockpit_protocol::projects::{ProjectArtifact, ProjectConfiguration, ProjectDiagnostic};
use cockpit_protocol::sources::SourceCapability;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use url::Url;

const COMMENTS_PER_PAGE: usize = 100;
const MAX_COMMENT_PAGES: usize = 5;
const MAX_SOURCE_BYTES: usize = 1024 * 1024;

pub(crate) fn executable(value: &str) -> bool {
    Path::new(value)
        .file_name()
        .is_some_and(|name| name == "glab")
}

/// Read-only GitLab issue access through the owner's authenticated `glab` CLI.
/// The CLI remains responsible for credentials; Cockpit only supplies explicit
/// GET requests and never reads, copies, or injects a token.
#[derive(Debug)]
pub struct GitlabSourceProvider {
    provider_id: String,
    executable: String,
    base_url: Url,
    limits: (usize, Duration),
}

impl GitlabSourceProvider {
    pub fn configured(
        configuration: &ProjectConfiguration,
        provider_id: &str,
    ) -> Result<Self, InspectionError> {
        let provider = configuration
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_unsupported",
                    "GitLab provider is not configured",
                )
            })?;
        let base_url = Url::parse(&provider.base_url).map_err(|_| {
            InspectionError::new(
                "source_provider_invalid",
                "GitLab provider base URL is invalid",
            )
        })?;
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(InspectionError::new(
                "source_provider_invalid",
                "GitLab provider URL must be credential-free HTTP(S)",
            ));
        }
        // glab 1.118 rejects ports, slashes, and non-ASCII host selectors.
        // The endpoint itself still retains any configured port and base path.
        cli_hostname(&base_url)?;
        Ok(Self {
            provider_id: provider.id.clone(),
            executable: provider.executable.clone(),
            base_url,
            limits: (
                (configuration.limits.git_output_bytes as usize).max(1),
                Duration::from_millis(configuration.limits.operation_timeout_ms as u64),
            ),
        })
    }

    async fn command(&self, args: &[String]) -> Result<Vec<u8>, InspectionError> {
        let mut command = Command::new(&self.executable);
        command
            .args(args)
            .env("GLAB_PROMPT_DISABLED", "1")
            .env("GIT_TERMINAL_PROMPT", "0");
        let output = run_bounded_command(
            command,
            self.limits.0,
            self.limits.0,
            self.limits.1,
            "GitLab source",
        )
        .await
        .map_err(|error| match error.code.as_str() {
            "execution_timeout" => InspectionError::new(
                "source_provider_timeout",
                "GitLab CLI request exceeded the configured deadline",
            ),
            "bounded_output" => InspectionError::new(
                "source_truncated",
                "GitLab CLI response exceeded Cockpit's explicit process limit",
            ),
            "execution_failed" => {
                InspectionError::new("source_cli_unavailable", "GitLab CLI could not be started")
            }
            _ => InspectionError::new("source_provider_failed", "GitLab CLI request failed"),
        })?;
        if !output.status.success() {
            return Err(classify_cli_failure(&output.stderr));
        }
        if output.stdout.is_empty() {
            return Err(InspectionError::new(
                "source_provider_contract",
                "GitLab CLI returned an empty response",
            ));
        }
        Ok(output.stdout)
    }

    fn endpoint(&self, suffix: &str) -> Result<(Url, String), InspectionError> {
        let hostname = cli_hostname(&self.base_url)?;
        let prefix = self.base_url.as_str().trim_end_matches('/');
        let endpoint = Url::parse(&format!("{prefix}/api/v4/{suffix}")).map_err(|_| {
            InspectionError::new(
                "source_provider_invalid",
                "GitLab API endpoint could not be constructed",
            )
        })?;
        Ok((endpoint, hostname))
    }

    async fn api_get(&self, suffix: &str) -> Result<Vec<u8>, InspectionError> {
        let (endpoint, hostname) = self.endpoint(suffix)?;
        let args = api_args(&endpoint, "GET", &hostname)?;
        self.command(&args).await
    }

    fn request_identity(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<ResolvedIdentity, InspectionError> {
        let artifact =
            resolve_gitlab_artifact(&self.provider_id, &self.base_url, &request.artifact_url)?;
        if artifact.provider_id != self.provider_id
            || request.authority.provider_instance != provider_instance(&self.base_url)
        {
            return Err(identity_error(
                "GitLab artifact does not match the configured provider",
            ));
        }
        let (project_path, iid) = artifact
            .canonical_id
            .rsplit_once('#')
            .and_then(|(path, iid)| iid.parse::<u64>().ok().map(|iid| (path.to_owned(), iid)))
            .filter(|(path, iid)| !path.is_empty() && *iid > 0)
            .ok_or_else(|| identity_error("GitLab artifact identity is invalid"))?;
        let expected_project = if request.authority.owner.is_empty() {
            request.authority.repository.clone()
        } else {
            format!(
                "{}/{}",
                request.authority.owner, request.authority.repository
            )
        };
        if project_path != expected_project {
            return Err(identity_error(
                "GitLab artifact does not match the verified primary repository",
            ));
        }
        Ok(ResolvedIdentity {
            artifact,
            project_path,
            iid,
        })
    }

    fn review_identity(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<ResolvedIdentity, InspectionError> {
        let artifact =
            resolve_gitlab_artifact(&self.provider_id, &self.base_url, &request.artifact_url)?;
        if artifact.kind != "review"
            || artifact.provider_id != self.provider_id
            || request.authority.provider_instance != provider_instance(&self.base_url)
        {
            return Err(identity_error(
                "GitLab merge request does not match the configured provider",
            ));
        }
        let (project_path, iid) = artifact
            .canonical_id
            .rsplit_once('!')
            .and_then(|(path, iid)| iid.parse::<u64>().ok().map(|iid| (path.to_owned(), iid)))
            .filter(|(path, iid)| !path.is_empty() && *iid > 0)
            .ok_or_else(|| identity_error("GitLab merge request identity is invalid"))?;
        let expected_project = if request.authority.owner.is_empty() {
            request.authority.repository.clone()
        } else {
            format!(
                "{}/{}",
                request.authority.owner, request.authority.repository
            )
        };
        if project_path != expected_project {
            return Err(identity_error(
                "GitLab merge request does not match the verified primary repository",
            ));
        }
        Ok(ResolvedIdentity {
            artifact,
            project_path,
            iid,
        })
    }
    fn artifact_is_review(&self, request: &SourceFetchRequest) -> Result<bool, InspectionError> {
        Ok(
            resolve_gitlab_artifact(&self.provider_id, &self.base_url, &request.artifact_url)?.kind
                == "review",
        )
    }

    async fn fetch_project(
        &self,
        identity: &ResolvedIdentity,
        budget: &mut ByteBudget,
    ) -> Result<ProjectFacts, InspectionError> {
        let encoded = encode_component(&identity.project_path);
        let value = parse_object(
            &self.api_get(&format!("projects/{encoded}")).await?,
            "project",
        )?;
        budget.account_json(&value)?;
        let id = value_u64(&value, "id")
            .ok_or_else(|| contract_error("GitLab project has no numeric ID"))?;
        if id == 0 {
            return Err(identity_error("GitLab project ID is invalid"));
        }
        let path = value_string(&value, "path_with_namespace")
            .ok_or_else(|| contract_error("GitLab project has no full path"))?;
        if path != identity.project_path {
            return Err(identity_error("GitLab returned a different project path"));
        }
        let web_url = value_string(&value, "web_url")
            .ok_or_else(|| contract_error("GitLab project has no web URL"))?;
        verify_project_url(&self.base_url, &web_url, &identity.project_path)?;
        Ok(ProjectFacts { id, path, web_url })
    }

    async fn fetch_issue(
        &self,
        identity: &ResolvedIdentity,
        project: &ProjectFacts,
        budget: &mut ByteBudget,
    ) -> Result<IssueFacts, InspectionError> {
        let value = parse_object(
            &self
                .api_get(&format!("projects/{}/issues/{}", project.id, identity.iid))
                .await?,
            "issue",
        )?;
        budget.account_json(&value)?;
        let project_id = value_u64(&value, "project_id")
            .ok_or_else(|| contract_error("GitLab issue has no project ID"))?;
        if project_id != project.id {
            return Err(identity_error("GitLab returned a different issue project"));
        }
        let iid =
            value_u64(&value, "iid").ok_or_else(|| contract_error("GitLab issue has no IID"))?;
        if iid != identity.iid {
            return Err(identity_error("GitLab returned a different issue IID"));
        }
        let title = required_string(&value, "title", "GitLab issue")?;
        let issue_type = required_string(&value, "issue_type", "GitLab issue")?;
        if issue_type != "issue" {
            return Err(unsupported_type_error(
                "GitLab work-item URL is not an issue",
            ));
        }
        let reference = value
            .get("references")
            .and_then(|references| references.get("full"))
            .and_then(Value::as_str)
            .ok_or_else(|| contract_error("GitLab issue has no full reference"))?;
        if reference != format!("{}#{}", identity.project_path, identity.iid) {
            return Err(identity_error(
                "GitLab returned a different issue reference",
            ));
        }
        let web_url = required_string(&value, "web_url", "GitLab issue")?;
        verify_issue_url(
            &self.base_url,
            &web_url,
            &identity.project_path,
            identity.iid,
        )?;
        let author = value
            .get("author")
            .and_then(|author| author.get("username"))
            .and_then(Value::as_str)
            .ok_or_else(|| contract_error("GitLab issue has no author username"))?
            .to_owned();
        let state = required_string(&value, "state", "GitLab issue")?;
        let labels = required_string_array(&value, "labels", "GitLab issue")?;
        let mut assignees = value
            .get("assignees")
            .and_then(Value::as_array)
            .ok_or_else(|| contract_error("GitLab issue has no assignees array"))?
            .iter()
            .map(|assignee| {
                assignee
                    .get("username")
                    .and_then(Value::as_str)
                    .ok_or_else(|| contract_error("GitLab issue assignee has no username"))
                    .map(str::to_owned)
            })
            .collect::<Result<Vec<_>, _>>()?;
        assignees.sort();
        assignees.dedup();
        let milestone = match value.get("milestone") {
            Some(Value::Null) => None,
            Some(milestone) => Some(
                milestone
                    .get("title")
                    .and_then(Value::as_str)
                    .ok_or_else(|| contract_error("GitLab issue milestone has no title"))?
                    .to_owned(),
            ),
            None => return Err(contract_error("GitLab issue has no milestone field")),
        };
        let created_at = required_string(&value, "created_at", "GitLab issue")?;
        let updated_at = required_string(&value, "updated_at", "GitLab issue")?;
        let description = match value.get("description") {
            Some(Value::Null) => String::new(),
            Some(Value::String(description)) => description.clone(),
            Some(_) => {
                return Err(contract_error(
                    "GitLab issue description has an invalid type",
                ));
            }
            None => return Err(contract_error("GitLab issue has no description field")),
        };
        Ok(IssueFacts {
            title,
            description,
            author,
            state,
            labels,
            assignees,
            milestone,
            created_at,
            updated_at,
            web_url,
        })
    }

    async fn fetch_project_by_id(
        &self,
        project_id: u64,
        budget: &mut ByteBudget,
    ) -> Result<ProjectIdentity, InspectionError> {
        let value = parse_object(
            &self.api_get(&format!("projects/{project_id}")).await?,
            "source project",
        )?;
        budget.account_json(&value)?;
        parse_project_identity(&value, &self.base_url)
    }

    async fn fetch_review(
        &self,
        identity: &ResolvedIdentity,
        project: &ProjectFacts,
        budget: &mut ByteBudget,
    ) -> Result<ReviewFacts, InspectionError> {
        let value = parse_object(
            &self
                .api_get(&format!(
                    "projects/{}/merge_requests/{}",
                    project.id, identity.iid
                ))
                .await?,
            "merge request",
        )?;
        budget.account_json(&value)?;
        let iid = value_u64(&value, "iid")
            .ok_or_else(|| contract_error("GitLab merge request has no IID"))?;
        if iid != identity.iid {
            return Err(identity_error(
                "GitLab returned a different merge request IID",
            ));
        }
        let title = required_string(&value, "title", "GitLab merge request")?;
        let description = match value.get("description") {
            Some(Value::Null) => String::new(),
            Some(Value::String(description)) => description.clone(),
            Some(_) => {
                return Err(contract_error(
                    "GitLab merge request description has an invalid type",
                ));
            }
            None => {
                return Err(contract_error(
                    "GitLab merge request has no description field",
                ));
            }
        };
        let state = required_string(&value, "state", "GitLab merge request")?;
        let author = value
            .get("author")
            .and_then(|author| author.get("username"))
            .and_then(Value::as_str)
            .ok_or_else(|| contract_error("GitLab merge request has no author username"))?
            .to_owned();
        let labels = required_string_array(&value, "labels", "GitLab merge request")?;
        let assignees = username_array(&value, "assignees", "GitLab merge request")?;
        let reviewers = username_array(&value, "reviewers", "GitLab merge request")?;
        let created_at = required_string(&value, "created_at", "GitLab merge request")?;
        let updated_at = required_string(&value, "updated_at", "GitLab merge request")?;
        let web_url = required_string(&value, "web_url", "GitLab merge request")?;
        verify_review_url(
            &self.base_url,
            &web_url,
            &identity.project_path,
            identity.iid,
        )?;
        let reference = value
            .get("references")
            .and_then(|references| references.get("full"))
            .and_then(Value::as_str)
            .ok_or_else(|| contract_error("GitLab merge request has no full reference"))?;
        if reference != format!("{}!{}", identity.project_path, identity.iid) {
            return Err(identity_error(
                "GitLab returned a different merge request reference",
            ));
        }
        let target_branch = required_string(&value, "target_branch", "GitLab merge request")?;
        if !valid_branch(&target_branch) {
            return Err(contract_error(
                "GitLab merge request target branch is invalid",
            ));
        }
        let source_branch = optional_string(&value, "source_branch", "GitLab merge request")?;
        if source_branch
            .as_deref()
            .is_some_and(|branch| !valid_branch(branch))
        {
            return Err(contract_error(
                "GitLab merge request source branch is invalid",
            ));
        }
        let head_sha = value
            .get("sha")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .get("diff_refs")
                    .and_then(|refs| refs.get("head_sha"))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned);
        if let Some(sha) = &head_sha {
            if !valid_commit(sha) {
                return Err(contract_error(
                    "GitLab merge request head commit is invalid",
                ));
            }
        }
        let target_project_id = value_u64(&value, "target_project_id")
            .or_else(|| value_u64(&value, "project_id"))
            .ok_or_else(|| contract_error("GitLab merge request has no target project ID"))?;
        if target_project_id != project.id {
            return Err(identity_error(
                "GitLab merge request target project mismatches requested project",
            ));
        }
        let target_project = ProjectIdentity {
            id: project.id,
            path: identity.project_path.clone(),
            web_url: project.web_url.clone(),
        };
        let source_project_id = value_u64(&value, "source_project_id");
        let source_project = match source_project_id {
            None => None,
            Some(id) if id == project.id => Some(ProjectIdentity {
                id: project.id,
                path: identity.project_path.clone(),
                web_url: project.web_url.clone(),
            }),
            Some(id) => self.fetch_project_by_id(id, budget).await.ok(),
        };
        Ok(ReviewFacts {
            title,
            description,
            state,
            author,
            labels,
            assignees,
            reviewers,
            created_at,
            updated_at,
            web_url,
            target_project,
            source_project,
            source_branch,
            target_branch,
            head_sha,
        })
    }

    async fn fetch_discussions(
        &self,
        identity: &ResolvedIdentity,
        project: &ProjectFacts,
        review_url: &str,
        budget: &mut ByteBudget,
    ) -> Result<(Vec<ReviewNote>, bool), InspectionError> {
        let mut notes = Vec::new();
        let mut complete = true;
        for page in 1..=MAX_COMMENT_PAGES {
            let response = self
                .api_get(&format!(
                    "projects/{}/merge_requests/{}/discussions?per_page={COMMENTS_PER_PAGE}&page={page}",
                    project.id, identity.iid
                ))
                .await?;
            if !budget.can_account(response.len()) {
                complete = false;
                break;
            }
            let values: Vec<Value> = serde_json::from_slice(&response).map_err(|_| {
                contract_error("GitLab merge-request discussions did not match the JSON contract")
            })?;
            budget.account(response.len());
            let full_page = values.len() == COMMENTS_PER_PAGE;
            for discussion in values {
                let discussion_id = required_string(&discussion, "id", "GitLab discussion")?;
                let raw_notes = discussion
                    .get("notes")
                    .and_then(Value::as_array)
                    .ok_or_else(|| contract_error("GitLab discussion has no notes array"))?;
                for note in raw_notes {
                    let id = value_u64(note, "id").ok_or_else(|| {
                        contract_error("GitLab discussion note has no numeric ID")
                    })?;
                    let body = required_string(note, "body", "GitLab discussion note")?;
                    let author = note
                        .get("author")
                        .and_then(|author| author.get("username"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            contract_error("GitLab discussion note has no author username")
                        })?
                        .to_owned();
                    let created_at = required_string(note, "created_at", "GitLab discussion note")?;
                    let updated_at = required_string(note, "updated_at", "GitLab discussion note")?;
                    let url = comment_url(review_url, note, id)?;
                    let position = note
                        .get("position")
                        .map(position_text)
                        .transpose()?
                        .unwrap_or_else(|| "unsupported local review anchor".into());
                    notes.push(ReviewNote {
                        discussion_id: discussion_id.clone(),
                        id,
                        author,
                        created_at,
                        updated_at,
                        url,
                        body,
                        position,
                    });
                }
            }
            if !full_page {
                break;
            }
            if page == MAX_COMMENT_PAGES {
                complete = false;
            }
        }
        notes.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| left.discussion_id.cmp(&right.discussion_id))
                .then_with(|| left.body.cmp(&right.body))
        });
        notes.dedup_by_key(|note| note.id);
        notes.sort_by(|left, right| left.id.cmp(&right.id));
        Ok((notes, complete))
    }

    async fn fetch_approvals(
        &self,
        identity: &ResolvedIdentity,
        project: &ProjectFacts,
        budget: &mut ByteBudget,
    ) -> (Option<ApprovalFacts>, bool) {
        let response = match self
            .api_get(&format!(
                "projects/{}/merge_requests/{}/approvals",
                project.id, identity.iid
            ))
            .await
        {
            Ok(response) => response,
            Err(_) => return (None, false),
        };
        if !budget.can_account(response.len()) {
            return (None, false);
        }
        let value: Value = match serde_json::from_slice::<Value>(&response) {
            Ok(value) if value.is_object() => value,
            _ => return (None, false),
        };
        budget.account(response.len());
        let approved = match value.get("approved").and_then(Value::as_bool) {
            Some(value) => value,
            None => return (None, false),
        };
        let approvals_left = match value_u64(&value, "approvals_left") {
            Some(value) => value,
            None => return (None, false),
        };
        let values = match value.get("approved_by").and_then(Value::as_array) {
            Some(values) => values,
            None => return (None, false),
        };
        let mut approved_by = Vec::with_capacity(values.len());
        for value in values {
            let username = match value
                .get("user")
                .and_then(|user| user.get("username"))
                .and_then(Value::as_str)
            {
                Some(username) => username.to_owned(),
                None => return (None, false),
            };
            approved_by.push(username);
        }
        approved_by.sort();
        approved_by.dedup();
        (
            Some(ApprovalFacts {
                approved: Some(approved),
                approvals_left: Some(approvals_left),
                approved_by: Some(approved_by),
            }),
            true,
        )
    }

    async fn fetch_comments(
        &self,
        identity: &ResolvedIdentity,
        project: &ProjectFacts,
        issue_url: &str,
        budget: &mut ByteBudget,
    ) -> Result<(Vec<CommentFacts>, bool), InspectionError> {
        let mut comments = Vec::new();
        let mut complete = true;
        for page in 1..=MAX_COMMENT_PAGES {
            let response = self
                .api_get(&format!(
                    "projects/{}/issues/{}/notes?per_page={COMMENTS_PER_PAGE}&page={page}",
                    project.id, identity.iid
                ))
                .await?;
            if !budget.can_account(response.len()) {
                complete = false;
                break;
            }
            let values: Vec<Value> = serde_json::from_slice(&response).map_err(|_| {
                contract_error("GitLab issue comments did not match the JSON contract")
            })?;
            budget.account(response.len());
            let full_page = values.len() == COMMENTS_PER_PAGE;
            for value in values {
                let id = value_u64(&value, "id")
                    .ok_or_else(|| contract_error("GitLab issue comment has no numeric ID"))?;
                let body = required_string(&value, "body", "GitLab issue comment")?;
                let author = value
                    .get("author")
                    .and_then(|author| author.get("username"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| contract_error("GitLab issue comment has no author username"))?
                    .to_owned();
                let created_at = required_string(&value, "created_at", "GitLab issue comment")?;
                let updated_at = required_string(&value, "updated_at", "GitLab issue comment")?;
                let url = comment_url(issue_url, &value, id)?;
                comments.push(CommentFacts {
                    id,
                    author,
                    created_at,
                    updated_at,
                    url,
                    body,
                });
            }
            if !full_page {
                break;
            }
            if page == MAX_COMMENT_PAGES {
                complete = false;
            }
        }
        comments.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| left.created_at.cmp(&right.created_at))
                .then_with(|| left.author.cmp(&right.author))
                .then_with(|| left.url.cmp(&right.url))
                .then_with(|| left.body.cmp(&right.body))
        });
        comments.dedup_by_key(|comment| comment.id);
        Ok((comments, complete))
    }

    async fn fetch_review_inner(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<SourceAsset, InspectionError> {
        let identity = self.review_identity(request)?;
        let mut budget = ByteBudget::new(self.limits.0.min(MAX_SOURCE_BYTES));
        let project = self.fetch_project(&identity, &mut budget).await?;
        let review = self.fetch_review(&identity, &project, &mut budget).await?;
        let (notes, discussions_complete) = match self
            .fetch_discussions(&identity, &project, &review.web_url, &mut budget)
            .await
        {
            Ok(result) => result,
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "source_permission_denied"
                        | "source_auth_required"
                        | "source_rate_limited"
                        | "source_not_found"
                        | "source_truncated"
                        | "source_provider_timeout"
                        | "source_cli_unavailable"
                        | "source_provider_failed"
                ) =>
            {
                (Vec::new(), false)
            }
            Err(error) => return Err(error),
        };
        let (approvals, approvals_complete) =
            self.fetch_approvals(&identity, &project, &mut budget).await;
        let mut diagnostics = Vec::new();
        if review.source_branch.as_deref().map_or(true, str::is_empty) {
            diagnostics.push(review_diagnostic(
                "source_branch_unavailable",
                "GitLab merge request source branch is unavailable",
                &request.artifact_url,
            ));
        }
        if review
            .head_sha
            .as_deref()
            .map_or(true, |sha| !valid_commit(sha))
        {
            diagnostics.push(review_diagnostic(
                "source_commit_unavailable",
                "GitLab merge request source commit is unavailable",
                &request.artifact_url,
            ));
        }
        if review.source_project.is_none() {
            diagnostics.push(review_diagnostic(
                "source_project_unavailable",
                "GitLab merge request source project provenance is unavailable",
                &request.artifact_url,
            ));
        }
        if !discussions_complete {
            diagnostics.push(review_diagnostic(
                "source_discussions_incomplete",
                "GitLab merge request discussions are unavailable or bounded",
                &request.artifact_url,
            ));
        }
        if !approvals_complete {
            diagnostics.push(review_diagnostic(
                "source_approvals_unavailable",
                "GitLab merge request approvals are unavailable or bounded",
                &request.artifact_url,
            ));
        }
        let labels = review.labels.join(", ");
        let assignees = review.assignees.join(", ");
        let reviewers = review.reviewers.join(", ");
        let metadata = format!(
            "## GitLab merge request metadata\n\nAuthor: {}\nState: {}\nCreated: {}\nUpdated: {}\nLabels: {}\nAssignees: {}\nReviewers: {}\nTarget project: {} ({})\nSource project: {} ({})\nTarget branch: {}\nSource branch: {}\nHead commit: {}\n\n",
            review.author,
            review.state,
            review.created_at,
            review.updated_at,
            if review.labels.is_empty() {
                "(none)"
            } else {
                labels.as_str()
            },
            if review.assignees.is_empty() {
                "(none)"
            } else {
                assignees.as_str()
            },
            if review.reviewers.is_empty() {
                "(none)"
            } else {
                reviewers.as_str()
            },
            review.target_project.path,
            review.target_project.web_url,
            review
                .source_project
                .as_ref()
                .map(|project| project.path.as_str())
                .unwrap_or("(unavailable)"),
            review
                .source_project
                .as_ref()
                .map(|project| project.web_url.as_str())
                .unwrap_or("(unavailable)"),
            review.target_branch,
            review.source_branch.as_deref().unwrap_or("(unavailable)"),
            review.head_sha.as_deref().unwrap_or("(unavailable)"),
        );
        let mut body = String::new();
        if !budget.append(&mut body, &metadata) || !budget.append(&mut body, &review.description) {
            diagnostics.push(review_diagnostic(
                "source_review_truncated",
                "GitLab merge request metadata exceeded Cockpit's explicit byte budget",
                &request.artifact_url,
            ));
        }
        for note in &notes {
            let rendered = format!(
                "\n\n## GitLab discussion {} note {}\nAuthor: {}\nCreated: {}\nUpdated: {}\nURL: {}\nPosition: {}\n\n{}",
                note.discussion_id,
                note.id,
                note.author,
                note.created_at,
                note.updated_at,
                note.url,
                note.position,
                note.body
            );
            if !budget.append(&mut body, &rendered) {
                diagnostics.push(review_diagnostic(
                    "source_review_truncated",
                    "GitLab merge request discussions exceeded Cockpit's explicit byte budget",
                    &request.artifact_url,
                ));
                break;
            }
        }
        if let Some(approvals) = approvals.as_ref() {
            let approved_by = approvals.approved_by.as_deref().unwrap_or(&[]).join(", ");
            let rendered = format!(
                "\n\n## GitLab approvals\nApproved: {}\nApprovals left: {}\nApproved by: {}\n",
                approvals
                    .approved
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "(unknown)".into()),
                approvals
                    .approvals_left
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "(unknown)".into()),
                if approved_by.is_empty() {
                    "(none)"
                } else {
                    &approved_by
                },
            );
            if !budget.append(&mut body, &rendered) {
                diagnostics.push(review_diagnostic(
                    "source_review_truncated",
                    "GitLab merge request approvals exceeded Cockpit's explicit byte budget",
                    &request.artifact_url,
                ));
            }
        }
        diagnostics.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then(left.message.cmp(&right.message))
        });
        diagnostics
            .dedup_by(|left, right| left.code == right.code && left.message == right.message);
        let complete = diagnostics.is_empty();
        let source_revision = review_revision(&review, &notes, approvals.as_ref(), complete);
        let title = review.title;
        let web_url = review.web_url;
        Ok(SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "review".into(),
                canonical_id: identity.artifact.canonical_id,
            },
            title,
            source_url: Some(web_url),
            original_url: Some(request.artifact_url.clone()),
            source_revision: Some(source_revision),
            body,
            complete,
            diagnostics,
        })
    }

    async fn fetch_inner(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<SourceAsset, InspectionError> {
        let identity = self.request_identity(request)?;
        let mut budget = ByteBudget::new(self.limits.0.min(MAX_SOURCE_BYTES));
        let project = self.fetch_project(&identity, &mut budget).await?;
        let issue = self.fetch_issue(&identity, &project, &mut budget).await?;
        let (comments, comments_complete) = self
            .fetch_comments(&identity, &project, &issue.web_url, &mut budget)
            .await?;
        let mut body = String::new();
        let mut diagnostics = Vec::new();
        let labels = issue.labels.join(", ");
        let assignees = issue.assignees.join(", ");
        let metadata = format!(
            "## GitLab issue metadata\n\nAuthor: {}\nState: {}\nCreated: {}\nUpdated: {}\nLabels: {}\nAssignees: {}\nMilestone: {}\n\n",
            issue.author,
            issue.state,
            issue.created_at,
            issue.updated_at,
            if issue.labels.is_empty() {
                "(none)"
            } else {
                labels.as_str()
            },
            if issue.assignees.is_empty() {
                "(none)"
            } else {
                assignees.as_str()
            },
            issue.milestone.as_deref().unwrap_or("(none)"),
        );
        if !budget.append(&mut body, &metadata) || !budget.append(&mut body, &issue.description) {
            diagnostics.push(truncation_diagnostic(&request.artifact_url));
        }
        for comment in &comments {
            let rendered = format!(
                "\n\n## GitLab comment {}\nAuthor: {}\nCreated: {}\nUpdated: {}\nURL: {}\n\n{}",
                comment.id,
                comment.author,
                comment.created_at,
                comment.updated_at,
                comment.url,
                comment.body,
            );
            if !budget.append(&mut body, &rendered) {
                diagnostics.push(truncation_diagnostic(&request.artifact_url));
                break;
            }
        }
        if !comments_complete {
            diagnostics.push(truncation_diagnostic(&request.artifact_url));
        }
        diagnostics.sort_by(|left, right| {
            left.code
                .cmp(&right.code)
                .then(left.message.cmp(&right.message))
        });
        diagnostics
            .dedup_by(|left, right| left.code == right.code && left.message == right.message);
        let complete = diagnostics.is_empty();
        let source_revision = revision(&issue, &comments, complete);
        Ok(SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "issue".into(),
                canonical_id: identity.artifact.canonical_id,
            },
            title: issue.title,
            source_url: Some(issue.web_url),
            original_url: Some(request.artifact_url.clone()),
            source_revision: Some(source_revision),
            body,
            complete,
            diagnostics,
        })
    }
}

#[derive(Debug)]
struct ResolvedIdentity {
    artifact: ProjectArtifact,
    project_path: String,
    iid: u64,
}

#[derive(Debug)]
struct ProjectFacts {
    id: u64,
    path: String,
    web_url: String,
}

#[derive(Debug)]
struct IssueFacts {
    title: String,
    description: String,
    author: String,
    state: String,
    labels: Vec<String>,
    assignees: Vec<String>,
    milestone: Option<String>,
    created_at: String,
    updated_at: String,
    web_url: String,
}

#[derive(Debug)]
struct ProjectIdentity {
    id: u64,
    path: String,
    web_url: String,
}

#[derive(Debug)]
struct ReviewFacts {
    title: String,
    description: String,
    state: String,
    author: String,
    labels: Vec<String>,
    assignees: Vec<String>,
    reviewers: Vec<String>,
    created_at: String,
    updated_at: String,
    web_url: String,
    target_project: ProjectIdentity,
    source_project: Option<ProjectIdentity>,
    source_branch: Option<String>,
    target_branch: String,
    head_sha: Option<String>,
}

#[derive(Debug)]
struct ReviewNote {
    discussion_id: String,
    id: u64,
    author: String,
    created_at: String,
    updated_at: String,
    url: String,
    body: String,
    position: String,
}

#[derive(Debug)]
struct ApprovalFacts {
    approved: Option<bool>,
    approvals_left: Option<u64>,
    approved_by: Option<Vec<String>>,
}

#[derive(Debug)]
struct CommentFacts {
    id: u64,
    author: String,
    created_at: String,
    updated_at: String,
    url: String,
    body: String,
}

struct ByteBudget {
    limit: usize,
    used: usize,
}

impl ByteBudget {
    fn new(limit: usize) -> Self {
        Self { limit, used: 0 }
    }

    fn can_account(&self, bytes: usize) -> bool {
        bytes <= self.limit.saturating_sub(self.used)
    }

    fn account(&mut self, bytes: usize) {
        self.used = self.used.saturating_add(bytes);
    }

    fn account_json(&mut self, value: &Value) -> Result<(), InspectionError> {
        let bytes = serde_json::to_vec(value)
            .map_err(|_| contract_error("GitLab JSON could not be read"))?
            .len();
        if !self.can_account(bytes) {
            return Err(InspectionError::new(
                "source_truncated",
                "GitLab issue metadata exceeded Cockpit's explicit byte budget",
            ));
        }
        self.account(bytes);
        Ok(())
    }

    fn append(&mut self, output: &mut String, value: &str) -> bool {
        if !self.can_account(value.len()) {
            return false;
        }
        output.push_str(value);
        self.account(value.len());

        true
    }

    fn exhausted(&self) -> bool {
        self.used >= self.limit
    }
}
fn optional_string(
    value: &Value,
    field: &str,
    resource: &str,
) -> Result<Option<String>, InspectionError> {
    match value.get(field) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(contract_error(&format!("{resource} has invalid {field}"))),
        None => Ok(None),
    }
}

fn username_array(
    value: &Value,
    field: &str,
    resource: &str,
) -> Result<Vec<String>, InspectionError> {
    let mut values = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| contract_error(&format!("{resource} has missing or invalid {field}")))?
        .iter()
        .map(|value| {
            value
                .get("username")
                .and_then(Value::as_str)
                .ok_or_else(|| contract_error(&format!("{resource} has invalid {field} entry")))
                .map(str::to_owned)
        })
        .collect::<Result<Vec<_>, _>>()?;
    values.sort();
    values.dedup();
    Ok(values)
}

fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_branch(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn project_identity(
    value: &Value,
    field: &str,
    base: &Url,
) -> Result<ProjectIdentity, InspectionError> {
    let project = value
        .get(field)
        .ok_or_else(|| contract_error(&format!("GitLab merge request has no {field}")))?;
    parse_project_identity(project, base)
}

fn parse_project_identity(value: &Value, base: &Url) -> Result<ProjectIdentity, InspectionError> {
    let id = value_u64(value, "id")
        .ok_or_else(|| contract_error("GitLab project identity has no numeric ID"))?;
    let path = value_string(value, "path_with_namespace")
        .ok_or_else(|| contract_error("GitLab project identity has no full path"))?;
    let web_url = required_string(value, "web_url", "GitLab project identity")?;
    let url = verify_web_authority(base, &web_url)?;
    let expected = format!("{}/{}", base.path().trim_end_matches('/'), path)
        .trim_start_matches('/')
        .to_owned();
    if url.path().trim_matches('/') != expected {
        return Err(identity_error(
            "GitLab project identity web path mismatches its full path",
        ));
    }
    Ok(ProjectIdentity { id, path, web_url })
}

fn position_text(value: &Value) -> Result<String, InspectionError> {
    let position = value
        .as_object()
        .ok_or_else(|| contract_error("GitLab discussion position has an invalid type"))?;
    let position_type = position
        .get("position_type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let old_path = position.get("old_path").and_then(Value::as_str);
    let new_path = position.get("new_path").and_then(Value::as_str);
    let old_line = position.get("old_line").and_then(Value::as_u64);
    let new_line = position.get("new_line").and_then(Value::as_u64);
    if old_path.is_none() && new_path.is_none() && old_line.is_none() && new_line.is_none() {
        return Ok("unsupported local review anchor".into());
    }
    Ok(format!(
        "position_type={position_type}; old_path={}; new_path={}; old_line={old_line:?}; new_line={new_line:?}",
        old_path.unwrap_or(""),
        new_path.unwrap_or("")
    ))
}

fn api_args(endpoint: &Url, method: &str, hostname: &str) -> Result<Vec<String>, InspectionError> {
    if method != "GET" {
        return Err(InspectionError::new(
            "source_provider_contract",
            "GitLab source requests are restricted to GET",
        ));
    }
    if hostname.is_empty() || hostname.contains([':', '/', '\\']) {
        return Err(InspectionError::new(
            "source_provider_invalid",
            "configured GitLab hostname is unsupported by glab",
        ));
    }
    Ok(vec![
        "api".into(),
        endpoint.as_str().to_owned(),
        "--method".into(),
        "GET".into(),
        "--hostname".into(),
        hostname.to_owned(),
    ])
}

fn cli_hostname(url: &Url) -> Result<String, InspectionError> {
    let host = url.host_str().unwrap_or_default();
    if host.is_empty()
        || !host.is_ascii()
        || host
            .chars()
            .any(|character| character.is_whitespace() || matches!(character, ':' | '/' | '\\'))
    {
        return Err(InspectionError::new(
            "source_provider_invalid",
            "configured GitLab hostname is unsupported by glab 1.118",
        ));
    }
    Ok(host.to_owned())
}

fn provider_instance(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let default_port = match url.scheme() {
        "http" => 80,
        "https" => 443,
        _ => 0,
    };
    let port = url
        .port()
        .filter(|port| *port != default_port)
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    format!(
        "{}://{}{}{}",
        url.scheme().to_ascii_lowercase(),
        host,
        port,
        path
    )
}

fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn parse_object(bytes: &[u8], name: &str) -> Result<Value, InspectionError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| contract_error(&format!("GitLab {name} JSON did not match the contract")))?;
    if !value.is_object() {
        return Err(contract_error(&format!(
            "GitLab {name} response was not an object"
        )));
    }
    Ok(value)
}

fn value_string(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn required_string(value: &Value, field: &str, resource: &str) -> Result<String, InspectionError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| contract_error(&format!("{resource} has missing or invalid {field}")))
}
fn required_string_array(
    value: &Value,
    field: &str,
    resource: &str,
) -> Result<Vec<String>, InspectionError> {
    let mut values = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| contract_error(&format!("{resource} has missing or invalid {field}")))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| contract_error(&format!("{resource} has invalid {field} entry")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    values.sort();
    values.dedup();
    Ok(values)
}

fn value_u64(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn verify_project_url(base: &Url, value: &str, project_path: &str) -> Result<(), InspectionError> {
    let url = verify_web_authority(base, value)?;
    let base_path = base.path().trim_end_matches('/');
    let path = url.path().trim_end_matches('/');
    let expected = format!("{base_path}/{project_path}")
        .trim_start_matches('/')
        .to_owned();
    if path.trim_start_matches('/') != expected {
        return Err(identity_error(
            "GitLab returned a different project web path",
        ));
    }
    Ok(())
}

fn verify_issue_url(
    base: &Url,
    value: &str,
    project_path: &str,
    iid: u64,
) -> Result<(), InspectionError> {
    let url = verify_web_authority(base, value)?;
    let base_path = base.path().trim_end_matches('/');
    let prefix = format!("{base_path}/{project_path}")
        .trim_start_matches('/')
        .to_owned();
    let path = url.path().trim_matches('/');
    let expected_issue = format!("{prefix}/-/issues/{iid}");
    let expected_work_item = format!("{prefix}/-/work_items/{iid}");

    let expected_legacy = format!("{prefix}/issues/{iid}");
    if path != expected_issue && path != expected_work_item && path != expected_legacy {
        return Err(identity_error("GitLab returned a different issue web path"));
    }
    Ok(())
}
fn verify_review_url(
    base: &Url,
    value: &str,
    project_path: &str,
    iid: u64,
) -> Result<(), InspectionError> {
    let url = verify_web_authority(base, value)?;
    let prefix = format!("{}/{}", base.path().trim_end_matches('/'), project_path)
        .trim_start_matches('/')
        .to_owned();
    let expected = format!("{prefix}/-/merge_requests/{iid}");
    if url.path().trim_matches('/') != expected {
        return Err(identity_error(
            "GitLab returned a different merge request web path",
        ));
    }
    Ok(())
}

fn comment_url(issue_url: &str, note: &Value, id: u64) -> Result<String, InspectionError> {
    let issue = Url::parse(issue_url)
        .map_err(|_| contract_error("GitLab issue URL could not be parsed"))?;
    let Some(value) = note.get("web_url") else {
        return Ok(format!("{issue_url}#note_{id}"));
    };
    let value = value
        .as_str()
        .ok_or_else(|| contract_error("GitLab issue comment URL has an invalid type"))?;
    let url =
        Url::parse(value).map_err(|_| contract_error("GitLab issue comment URL is invalid"))?;
    let expected_fragment = format!("note_{id}");
    if url.scheme() != issue.scheme()
        || url.host_str() != issue.host_str()
        || url.port_or_known_default() != issue.port_or_known_default()
        || url.path() != issue.path()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment() != Some(expected_fragment.as_str())
    {
        return Err(identity_error(
            "GitLab issue comment URL does not match the verified issue",
        ));
    }
    Ok(value.to_owned())
}
fn verify_web_authority(base: &Url, value: &str) -> Result<Url, InspectionError> {
    let url =
        Url::parse(value).map_err(|_| contract_error("GitLab returned an invalid web URL"))?;
    if url.scheme() != base.scheme()
        || url.host_str() != base.host_str()
        || url.port_or_known_default() != base.port_or_known_default()
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(identity_error(
            "GitLab returned a web URL on another authority",
        ));
    }
    let base_path = base.path().trim_end_matches('/');
    let returned_path = url.path().trim_end_matches('/');
    if !base_path.is_empty()
        && returned_path != base_path
        && !returned_path.starts_with(&format!("{base_path}/"))
    {
        return Err(identity_error(
            "GitLab returned a web URL outside the configured base path",
        ));
    }
    Ok(url)
}

fn classify_cli_failure(stderr: &[u8]) -> InspectionError {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if text.contains("not found") || text.contains("404") {
        InspectionError::new("source_not_found", "GitLab resource was not found")
    } else if text.contains("forbidden") || text.contains("permission") || text.contains("403") {
        InspectionError::new("source_permission_denied", "GitLab denied the read request")
    } else if text.contains("unauthorized")
        || text.contains("authentication")
        || text.contains("logged in")
        || text.contains("token")
        || text.contains("credential")
    {
        InspectionError::new(
            "source_auth_required",
            "GitLab CLI authentication is unavailable",
        )
    } else if text.contains("rate limit") || text.contains("429") {
        InspectionError::new(
            "source_rate_limited",
            "GitLab rate-limited the read request",
        )
    } else {
        InspectionError::new("source_provider_failed", "GitLab read request failed")
    }
}

fn contract_error(message: &str) -> InspectionError {
    InspectionError::new("source_provider_contract", message)
}

fn identity_error(message: &str) -> InspectionError {
    InspectionError::new("source_identity_mismatch", message)
}

fn unsupported_type_error(message: &str) -> InspectionError {
    InspectionError::new("source_unsupported_type", message)
}

fn truncation_diagnostic(path: &str) -> ProjectDiagnostic {
    ProjectDiagnostic {
        code: "source_comments_truncated".into(),
        message: "GitLab issue comments exceeded Cockpit's explicit page or byte budget".into(),
        path: Some(path.to_owned()),
    }
}

fn review_diagnostic(code: &str, message: &str, path: &str) -> ProjectDiagnostic {
    ProjectDiagnostic {
        code: code.into(),
        message: message.into(),
        path: Some(path.into()),
    }
}

fn review_revision(
    review: &ReviewFacts,
    notes: &[ReviewNote],
    approvals: Option<&ApprovalFacts>,
    complete: bool,
) -> String {
    let mut input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        review.title,
        review.description,
        review.state,
        review.author,
        review.created_at,
        review.updated_at,
        review.target_project.path,
        review
            .source_project
            .as_ref()
            .map(|project| project.path.as_str())
            .unwrap_or_default(),
        review.target_branch,
        review.source_branch.as_deref().unwrap_or_default(),
        review.head_sha.as_deref().unwrap_or_default(),
    );
    input.push_str(&review.target_project.web_url);
    input.push('\n');
    input.push_str(
        review
            .source_project
            .as_ref()
            .map(|project| project.web_url.as_str())
            .unwrap_or_default(),
    );
    input.push('\n');
    input.push_str(&review.labels.join("\u{1f}"));
    input.push('\n');
    input.push_str(&review.assignees.join("\u{1f}"));
    input.push('\n');
    input.push_str(&review.reviewers.join("\u{1f}"));
    input.push('\n');
    input.push_str(if complete { "complete" } else { "incomplete" });
    for note in notes {
        input.push_str(&format!(
            "\n{}|{}|{}|{}|{}|{}|{}|{}",
            note.discussion_id,
            note.id,
            note.author,
            note.created_at,
            note.updated_at,
            note.url,
            note.position,
            note.body
        ));
    }
    if let Some(approvals) = approvals {
        input.push_str(&format!(
            "\napproved={:?}|left={:?}|by={:?}",
            approvals.approved, approvals.approvals_left, approvals.approved_by
        ));
    }
    format!("sha256:{}", hex_digest(input.as_bytes()))
}

fn revision(issue: &IssueFacts, comments: &[CommentFacts], complete: bool) -> String {
    let mut input = String::new();
    for value in [
        issue.title.as_str(),
        issue.description.as_str(),
        issue.author.as_str(),
        issue.state.as_str(),
        issue.created_at.as_str(),
        issue.updated_at.as_str(),
        issue.web_url.as_str(),
    ] {
        input.push_str(value);
        input.push('\n');
    }
    input.push_str(&issue.labels.join("\u{1f}"));
    input.push('\n');
    input.push_str(&issue.assignees.join("\u{1f}"));
    input.push('\n');
    input.push_str(issue.milestone.as_deref().unwrap_or_default());
    input.push('\n');
    input.push_str(if complete { "complete" } else { "incomplete" });
    for comment in comments {
        input.push('\n');
        input.push_str(&comment.id.to_string());
        input.push('|');
        input.push_str(&comment.author);
        input.push('|');
        input.push_str(&comment.created_at);
        input.push('|');
        input.push_str(&comment.updated_at);
        input.push('|');
        input.push_str(&comment.url);
        input.push('|');
        input.push_str(&comment.body);
    }
    format!("sha256:{}", hex_digest(input.as_bytes()))
}

fn hex_digest(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[async_trait]
impl SourceProvider for GitlabSourceProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![
            SourceCapability::Issue,
            SourceCapability::IssueComments,
            SourceCapability::Review,
        ]
    }

    async fn metadata(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
        if self.artifact_is_review(request)? {
            let identity = self.review_identity(request)?;
            let mut budget = ByteBudget::new(self.limits.0.min(MAX_SOURCE_BYTES));
            let project = self.fetch_project(&identity, &mut budget).await?;
            let review = self.fetch_review(&identity, &project, &mut budget).await?;
            let source_branch = review
                .source_branch
                .clone()
                .filter(|branch| !branch.is_empty())
                .ok_or_else(|| {
                    InspectionError::new(
                        "source_branch_unavailable",
                        "GitLab merge request has no usable source branch",
                    )
                })?;
            let source_commit = review
                .head_sha
                .clone()
                .filter(|sha| valid_commit(sha))
                .ok_or_else(|| {
                    InspectionError::new(
                        "source_commit_unavailable",
                        "GitLab merge request has no full source commit",
                    )
                })?;
            return Ok(SourceMetadata {
                title: review.title,
                source_branch: Some(source_branch),
                source_url: Some(review.web_url),
                source_commit: Some(source_commit),
                description: Some(bounded_description(review.description)),
            });
        }
        let identity = self.request_identity(request)?;
        let mut budget = ByteBudget::new(self.limits.0.min(MAX_SOURCE_BYTES));
        let project = self.fetch_project(&identity, &mut budget).await?;
        let issue = self.fetch_issue(&identity, &project, &mut budget).await?;
        Ok(SourceMetadata {
            title: issue.title,
            source_branch: None,
            source_url: Some(issue.web_url),
            source_commit: None,
            description: None,
        })
    }

    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        if self.artifact_is_review(request)? {
            Ok(vec![self.fetch_review_inner(request).await?])
        } else {
            Ok(vec![self.fetch_inner(request).await?])
        }
    }
}

/// Linked work items are found in the first part of a description; a very
/// long one is cut at a character boundary rather than rejected.
fn bounded_description(mut description: String) -> String {
    const MAX_DESCRIPTION_BYTES: usize = 64 * 1024;
    if description.len() > MAX_DESCRIPTION_BYTES {
        let mut end = MAX_DESCRIPTION_BYTES;
        while !description.is_char_boundary(end) {
            end -= 1;
        }
        description.truncate(end);
    }
    description
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalFacts, ProjectIdentity, ReviewFacts, ReviewNote, api_args, comment_url,
        encode_component, review_revision, verify_issue_url,
    };
    use url::Url;

    #[test]
    fn api_builder_rejects_non_get_methods() {
        let endpoint = Url::parse("https://gitlab.test/api/v4/projects/1").unwrap();
        let error = api_args(&endpoint, "POST", "gitlab.test").unwrap_err();
        assert_eq!(error.code, "source_provider_contract");
    }

    #[test]
    fn api_builder_keeps_base_path_and_port_in_absolute_endpoint() {
        let base = Url::parse("https://gitlab.test:9443/subfolder").unwrap();
        let endpoint = Url::parse(&format!(
            "{}/api/v4/projects/{}",
            base,
            encode_component("group/project")
        ))
        .unwrap();
        assert_eq!(
            endpoint.as_str(),
            "https://gitlab.test:9443/subfolder/api/v4/projects/group%2Fproject"
        );
    }

    #[test]
    fn issue_web_authority_rejects_another_host() {
        let base = Url::parse("https://gitlab.test/subfolder").unwrap();
        let error = verify_issue_url(
            &base,
            "https://other.test/subfolder/group/project/-/issues/1",
            "group/project",
            1,
        )
        .unwrap_err();
        assert_eq!(error.code, "source_identity_mismatch");
    }

    #[test]
    fn note_url_derives_from_verified_issue_when_api_omits_web_url() {
        let note = serde_json::json!({"id": 7});
        let url = comment_url(
            "https://gitlab.test/subfolder/group/project/-/issues/1",
            &note,
            7,
        )
        .unwrap();
        assert_eq!(
            url,
            "https://gitlab.test/subfolder/group/project/-/issues/1#note_7"
        );
    }

    #[test]
    fn bounded_budget_reports_exhaustion_without_partial_string() {
        let mut budget = super::ByteBudget::new(4);
        let mut body = String::new();
        assert!(!budget.append(&mut body, "five!"));
        assert!(body.is_empty());
        assert!(!budget.exhausted());
        assert!(budget.append(&mut body, "four"));
        assert!(budget.exhausted());
    }

    #[test]
    fn cli_failure_classification_does_not_expose_stderr() {
        let error = super::classify_cli_failure(b"unauthorized token=secret-value");
        assert_eq!(error.code, "source_auth_required");
        assert!(!error.message.contains("secret-value"));
    }

    #[test]
    fn malformed_required_note_body_is_a_contract_error() {
        let value = serde_json::json!({"body": {"unexpected": true}});
        let error = super::required_string(&value, "body", "GitLab issue comment").unwrap_err();
        assert_eq!(error.code, "source_provider_contract");
    }

    #[test]
    fn review_revision_changes_for_same_sha_content_updates() {
        let project = ProjectIdentity {
            id: 1,
            path: "group/project".into(),
            web_url: "https://gitlab.test/group/project".into(),
        };
        let mut review = ReviewFacts {
            title: "Title".into(),
            description: "Body".into(),
            state: "opened".into(),
            author: "alice".into(),
            labels: vec!["one".into()],
            assignees: vec!["bob".into()],
            reviewers: vec!["carol".into()],
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            web_url: "https://gitlab.test/group/project/-/merge_requests/1".into(),
            target_project: project,
            source_project: None,
            source_branch: Some("feature".into()),
            target_branch: "main".into(),
            head_sha: Some("0123456789abcdef0123456789abcdef01234567".into()),
        };
        let mut notes = vec![ReviewNote {
            discussion_id: "discussion".into(),
            id: 7,
            author: "alice".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            url: "https://gitlab.test/group/project/-/merge_requests/1#note_7".into(),
            body: "comment".into(),
            position: "unsupported local review anchor".into(),
        }];
        let approvals = ApprovalFacts {
            approved: Some(false),
            approvals_left: Some(1),
            approved_by: Some(Vec::new()),
        };
        let before = review_revision(&review, &notes, Some(&approvals), true);
        review.title = "Changed title".into();
        assert_ne!(
            before,
            review_revision(&review, &notes, Some(&approvals), true)
        );
        review.title = "Title".into();
        review.description = "Changed body".into();
        assert_ne!(
            before,
            review_revision(&review, &notes, Some(&approvals), true)
        );
        review.description = "Body".into();
        notes[0].body = "changed comment".into();
        assert_ne!(
            before,
            review_revision(&review, &notes, Some(&approvals), true)
        );
        notes[0].body = "comment".into();
        notes[0].updated_at = "2026-01-02T00:00:00Z".into();
        assert_ne!(
            before,
            review_revision(&review, &notes, Some(&approvals), true)
        );
    }
}
