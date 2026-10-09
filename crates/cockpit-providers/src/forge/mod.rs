use std::ffi::OsStr;
use std::time::Duration;

use cockpit_core::InspectionError;
use cockpit_core::process::run_bounded_command;
use cockpit_core::sources::{SourceAsset, SourceFetchRequest};
use tokio::process::Command;

pub(crate) const COMMENTS_PER_PAGE: usize = 100;
pub(crate) const MAX_COMMENT_PAGES: usize = 5;

pub(crate) struct CliRunner<'a> {
    pub executable: &'a str,
    pub limits: (usize, Duration),
    pub label: &'static str,
    pub failure: fn(&[u8]) -> InspectionError,
    pub execution_error: fn(InspectionError, &[String]) -> InspectionError,
    pub empty_response: Option<&'static str>,
}

impl CliRunner<'_> {
    pub async fn run(
        &self,
        args: &[String],
        env: &[(&str, &OsStr)],
    ) -> Result<Vec<u8>, InspectionError> {
        let mut command = Command::new(self.executable);
        command.args(args).envs(env.iter().copied());
        let output = run_bounded_command(
            command,
            self.limits.0,
            self.limits.0,
            self.limits.1,
            self.label,
        )
        .await
        .map_err(|error| (self.execution_error)(error, args))?;
        if !output.status.success() {
            return Err((self.failure)(&output.stderr));
        }
        if let Some(message) = self.empty_response.filter(|_| output.stdout.is_empty()) {
            return Err(InspectionError::new("source_provider_contract", message));
        }
        Ok(output.stdout)
    }
}

pub(crate) fn unchanged_execution_error(error: InspectionError, _: &[String]) -> InspectionError {
    error
}

/// Accounting policy belongs to the adapter: GitLab counts metadata, raw pages
/// and rendering cumulatively; GitHub and Tea count only their rendered output.
pub(crate) struct ByteBudget {
    pub limit: usize,
    pub used: usize,
}

impl ByteBudget {
    pub fn new(limit: usize) -> Self {
        Self { limit, used: 0 }
    }

    pub fn for_output(limit: usize, output: &str) -> Self {
        Self {
            limit,
            used: output.len(),
        }
    }

    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.used)
    }

    pub fn can_account(&self, bytes: usize) -> bool {
        bytes <= self.remaining()
    }

    pub fn account(&mut self, bytes: usize) {
        self.used = self.used.saturating_add(bytes);
    }

    pub fn append(&mut self, output: &mut String, value: &str) -> bool {
        if !self.can_account(value.len()) {
            return false;
        }
        output.push_str(value);
        self.account(value.len());
        true
    }

    /// Preserve the rendered adapters' append-then-error semantics, including
    /// checking an already oversized output when appending an empty string.
    pub fn append_checked(&mut self, output: &mut String, value: &str) -> bool {
        output.push_str(value);
        self.account(value.len());
        self.used <= self.limit
    }
}

pub(crate) trait Forge: Sync {
    type Identity: Send + Sync;
    type Item: Send + Sync;
    type Comments: Send;

    fn resolve(&self, request: &SourceFetchRequest) -> Result<Self::Identity, InspectionError>;
    fn budget(&self) -> ByteBudget;

    /// Fetch and verify the item identity before any comment network reads.
    fn fetch_item(
        &self,
        request: &SourceFetchRequest,
        identity: &Self::Identity,
        budget: &mut ByteBudget,
    ) -> impl Future<Output = Result<Self::Item, InspectionError>> + Send;

    fn fetch_comments(
        &self,
        identity: &Self::Identity,
        item: &Self::Item,
        budget: &mut ByteBudget,
    ) -> impl Future<Output = Result<Self::Comments, InspectionError>> + Send;

    fn assemble(
        &self,
        request: &SourceFetchRequest,
        identity: Self::Identity,
        item: Self::Item,
        comments: Self::Comments,
        budget: &mut ByteBudget,
    ) -> Result<SourceAsset, InspectionError>;
}

pub(crate) async fn fetch<F: Forge>(
    forge: &F,
    request: &SourceFetchRequest,
) -> Result<Vec<SourceAsset>, InspectionError> {
    let identity = forge.resolve(request)?;
    let mut budget = forge.budget();
    let item = forge.fetch_item(request, &identity, &mut budget).await?;
    let comments = forge.fetch_comments(&identity, &item, &mut budget).await?;
    Ok(vec![forge.assemble(
        request,
        identity,
        item,
        comments,
        &mut budget,
    )?])
}
