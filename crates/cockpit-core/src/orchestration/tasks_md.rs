use std::{borrow::Cow, collections::HashMap, io, ops::Range};

use cap_std::fs::Dir;
use cockpit_protocol::orchestration::Task;
use pulldown_cmark::{Event, Options, Parser, Tag};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    InspectionError,
    project_store::{LockGuard, atomic_write_bytes_checked, read_bytes_bounded},
};

use super::steps::{self, DocumentEol, StepIntent, StepLayout, StepParseContext};

/// Absolute offsets into the original document; unchanged move bytes can be borrowed.
pub(crate) struct BytePatch<'a> {
    pub range: Range<usize>,
    pub replacement: Cow<'a, [u8]>,
}

pub(crate) const MAX_TASK_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TASK_TEXT_BYTES: usize = 16 * 1024;
const MARKER: &str = "<!-- cockpit-task:";

/// An exact-byte snapshot. Read-only projection documents have no lock; mutation
/// documents borrow the named state guard and cannot outlive its transaction.
pub(crate) struct TaskDocument<'a> {
    pub tasks: Vec<Task>,
    pub doc_revision: String,
    pub unidentified_items: u32,
    dir: &'a Dir,
    name: String,
    bytes: Vec<u8>,
    items: Vec<ItemLayout>,
    lock: Option<&'a LockGuard>,
}

struct ItemLayout {
    range: Range<usize>,
    header_end: usize,
    content_end: usize,
    title_range: Range<usize>,
    task_marker_range: Option<Range<usize>>,
    task_index: Option<usize>,
    relations: RelationsLayout,
    description_range: Option<Range<usize>>,
    context: StepParseContext,
    steps: StepLayout,
}

#[derive(Default)]
struct RelationsLayout {
    record_range: Option<Range<usize>>,
    inner_range: Option<Range<usize>>,
    depends_tokens: Vec<RelationToken>,
    follow_token: Option<RelationToken>,
    protected_ranges: Vec<Range<usize>>,
    depends_on: Vec<String>,
    follow_up_of: Option<String>,
    diagnostic: Option<String>,
    repairable: bool,
}

struct RelationToken {
    range: Range<usize>,
    value_range: Range<usize>,
}

pub(crate) fn validate_uuid(id: &str) -> Result<Uuid, InspectionError> {
    Uuid::parse_str(id).map_err(|_| {
        InspectionError::new("invalid_identity", "task and root identities must be UUIDs")
    })
}

pub(crate) fn root_filename(root_id: &str) -> Result<String, InspectionError> {
    Ok(format!("{}.md", validate_uuid(root_id)?))
}

#[cfg(test)]
pub(crate) fn read_document<'a>(
    dir: &'a Dir,
    root_id: &str,
) -> Result<TaskDocument<'a>, InspectionError> {
    let name = root_filename(root_id)?;
    let bytes = read_bytes(dir, &name)?;
    parse_document(dir, name, bytes, None)
}

pub(crate) fn read_locked_document<'a>(
    dir: &'a Dir,
    root_id: &str,
    lock: &'a LockGuard,
) -> Result<TaskDocument<'a>, InspectionError> {
    let name = root_filename(root_id)?;
    let bytes = read_bytes(dir, &name)?;
    parse_document(dir, name, bytes, Some(lock))
}

fn read_bytes(dir: &Dir, name: &str) -> Result<Vec<u8>, InspectionError> {
    match dir.symlink_metadata(name) {
        Ok(_) => read_bytes_bounded(dir, name, MAX_TASK_DOCUMENT_BYTES as u64),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(InspectionError::new("tasks_read", error.to_string())),
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest_matches_revision(digest: &[u8], revision: &str) -> bool {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    revision.len() == digest.len() * 2
        && digest
            .iter()
            .zip(revision.as_bytes().chunks_exact(2))
            .all(|(byte, pair)| {
                pair[0] == HEX[(byte >> 4) as usize] && pair[1] == HEX[(byte & 15) as usize]
            })
}
fn conflict() -> InspectionError {
    InspectionError::new(
        "task_revision_conflict",
        "task document changed; refresh before applying this edit",
    )
}

impl<'a> TaskDocument<'a> {
    pub(crate) fn task(&self, task_id: &str) -> Result<&Task, InspectionError> {
        let task_id = validate_uuid(task_id)?.to_string();
        let task = self
            .tasks
            .iter()
            .find(|task| task.task_id == task_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "task_not_found",
                    "task is not in this root's canonical document",
                )
            })?;
        if task.diagnostic.as_deref() == Some("task_id_duplicate") {
            return Err(InspectionError::new(
                "task_id_duplicate",
                "duplicate task identity must be corrected in the canonical Markdown",
            ));
        }
        Ok(task)
    }

    fn item(&self, task_id: &str, expected: &str) -> Result<&ItemLayout, InspectionError> {
        let task = self.task(task_id)?;
        if task.task_revision != expected {
            return Err(conflict());
        }
        self.items
            .iter()
            .find(|item| {
                item.task_index
                    .is_some_and(|i| self.tasks[i].task_id == task.task_id)
            })
            .ok_or_else(|| InspectionError::new("task_not_found", "task item is missing"))
    }

    pub(crate) fn create_with_id(
        &self,
        task_id: &str,
        title: &str,
        body: &str,
    ) -> Result<Task, InspectionError> {
        validate_creation(title, body)?;
        let task_id = validate_uuid(task_id)?.to_string();
        if self.tasks.iter().any(|task| task.task_id == task_id) {
            return Err(InspectionError::new(
                "task_id_conflict",
                "task identity already exists in the canonical Markdown",
            ));
        }
        let eol = self.eol();
        let mut bytes = self.bytes.clone();
        if bytes.is_empty() {
            bytes.extend_from_slice(b"# Tasks\n\n");
        }
        if !bytes.ends_with(b"\n") {
            bytes.extend_from_slice(eol.as_bytes());
        }
        bytes.extend_from_slice(
            format!("- [ ] {title} <!-- cockpit-task: {task_id} -->{eol}").as_bytes(),
        );
        bytes.extend_from_slice(encode_body(body, eol).as_bytes());
        self.commit_task(bytes, &task_id)
    }

    /// Compare the exact intended canonical item, not normalized title/body.
    /// Checkbox, body bytes and marker edits all stop automatic assignment.
    pub(crate) fn matches_creation(&self, task: &Task, title: &str, body: &str) -> bool {
        // Creation uses the document's newline convention. Unrelated prose can
        // subsequently change that convention without changing this task.
        ["\n", "\r\n"].into_iter().any(|eol| {
            let mut digest = Sha256::new();
            digest.update(b"- [ ] ");
            digest.update(title.as_bytes());
            digest.update(b" <!-- cockpit-task: ");
            digest.update(task.task_id.as_bytes());
            digest.update(b" -->");
            digest.update(eol.as_bytes());
            for line in body.lines() {
                digest.update(b"  ");
                digest.update(line.as_bytes());
                digest.update(eol.as_bytes());
            }
            digest_matches_revision(&digest.finalize(), &task.task_revision)
        })
    }

    /// Graph validation and fresh actor authority belong to the locked core caller.
    /// Exact retries perform no append and therefore need not replay old graph fences.
    pub(crate) fn create_authoring_with_id(
        &self, task_id: &str, title: &str, description: &str,
        depends_on: &[String], follow_up_of: Option<&str>,
        expected_doc_revision: Option<&str>, source_revision: Option<&str>,
    ) -> Result<Task, InspectionError> {
        self.require_lock()?;
        validate_authoring_creation(title, description)?;
        let task_id = validate_uuid(task_id)?.to_string();
        let edges = normalized_dependencies(depends_on)?;
        let follow = follow_up_of.map(validate_uuid).transpose()?.map(|id| id.to_string());
        if (!edges.is_empty() || follow.is_some()) && expected_doc_revision.is_none() {
            return Err(InspectionError::new("invalid_task", "graph-bearing creation requires document revision"));
        }
        if follow.is_some() && source_revision.is_none() {
            return Err(InspectionError::new("invalid_task", "follow-up creation requires source revision"));
        }
        if follow.is_none() && source_revision.is_some() {
            return Err(InspectionError::new("invalid_task", "source revision requires follow_up_of"));
        }
        let eol = self.eol();
        let continuation = authoring_continuation(description, &edges, follow.as_deref(), eol);
        if continuation.len() > MAX_TASK_TEXT_BYTES {
            return Err(InspectionError::new("invalid_task", "complete task continuation exceeds 16 KiB"));
        }
        if self.tasks.iter().any(|task| task.task_id == task_id) {
            let task = self.task(&task_id)?;
            if self.matches_authoring_creation(task, title, description, &edges, follow.as_deref()) {
                if hash(&read_bytes(self.dir, &self.name)?) != self.doc_revision {
                    return Err(conflict());
                }
                return Ok(task.clone());
            }
            return Err(InspectionError::new("task_id_conflict",
                "task identity exists with different canonical bytes"));
        }
        if !edges.is_empty() || follow.is_some() {
            if expected_doc_revision != Some(self.doc_revision.as_str()) {
                return Err(conflict());
            }
        } else if expected_doc_revision.is_some_and(|revision| revision != self.doc_revision) {
            return Err(conflict());
        }
        if let Some(source) = follow.as_deref() {
            if source_revision != Some(self.task(source)?.task_revision.as_str()) {
                return Err(conflict());
            }
        }
        let mut append = String::new();
        if self.bytes.is_empty() { append.push_str("# Tasks\n\n"); }
        else if !self.bytes.ends_with(b"\n") { append.push_str(eol); }
        append.push_str(&format!("- [ ] {title} <!-- cockpit-task: {task_id} -->{eol}"));
        append.push_str(&continuation);
        let bytes = splice(&self.bytes, &[BytePatch {
            range: self.bytes.len()..self.bytes.len(),
            replacement: Cow::Owned(append.into_bytes()),
        }])?;
        self.commit_task(bytes, &task_id)
    }

    pub(crate) fn matches_authoring_creation(
        &self, task: &Task, title: &str, description: &str,
        depends_on: &[String], follow_up_of: Option<&str>,
    ) -> bool {
        let Ok(edges) = normalized_dependencies(depends_on) else { return false };
        let Ok(follow) = follow_up_of.map(validate_uuid).transpose() else { return false };
        let follow = follow.map(|id| id.to_string());
        ["\n", "\r\n"].into_iter().any(|eol| {
            let mut digest = Sha256::new();
            digest.update(b"- [ ] ");
            digest.update(title.as_bytes());
            digest.update(b" <!-- cockpit-task: ");
            digest.update(task.task_id.as_bytes());
            digest.update(b" -->");
            digest.update(eol.as_bytes());
            if !edges.is_empty() || follow.is_some() {
                digest.update(b"  <!-- cockpit-relations:");
                if !edges.is_empty() {
                    digest.update(b" depends_on=");
                    for (index, edge) in edges.iter().enumerate() {
                        if index != 0 { digest.update(b","); }
                        digest.update(edge.as_bytes());
                    }
                }
                if let Some(follow) = &follow {
                    digest.update(b" follow_up_of=");
                    digest.update(follow.as_bytes());
                }
                digest.update(b" -->");
                digest.update(eol.as_bytes());
            }
            for line in description.lines() {
                digest.update(b"  ");
                digest.update(line.as_bytes());
                digest.update(eol.as_bytes());
            }
            digest_matches_revision(&digest.finalize(), &task.task_revision)
        })
    }

    /// Own only prerequisite tokens; never reserialize immutable provenance.
    /// A live remove-only request is independently validated by the core caller.
    pub(crate) fn set_dependencies(
        &self, task_id: &str, expected_task_revision: &str,
        expected_doc_revision: &str, depends_on: &[String],
    ) -> Result<Task, InspectionError> {
        let item = self.item(task_id, expected_task_revision)?;
        if expected_doc_revision != self.doc_revision { return Err(conflict()); }
        let edges = normalized_dependencies(depends_on)?;
        let relations = &item.relations;
        if !relations.repairable {
            return Err(InspectionError::new("task_relations_invalid",
                "relationship record is not safely bounded; correct canonical source"));
        }
        let mut patches = Vec::new();
        if let Some(record) = relations.record_range.as_ref() {
            if edges.is_empty() && relations.follow_token.is_none() {
                patches.push(BytePatch { range: record.clone(), replacement: Cow::Borrowed(b"") });
            } else if let Some(first) = relations.depends_tokens.first() {
                if edges.is_empty() {
                    patches.push(BytePatch { range: first.range.clone(), replacement: Cow::Borrowed(b"") });
                } else {
                    patches.push(BytePatch {
                        range: first.value_range.clone(),
                        replacement: Cow::Owned(edges.join(",").into_bytes()),
                    });
                }
                for token in relations.depends_tokens.iter().skip(1) {
                    patches.push(BytePatch { range: token.range.clone(), replacement: Cow::Borrowed(b"") });
                }
            } else if !edges.is_empty() {
                let gap = relations.inner_range.as_ref().ok_or_else(invalid_patch)?.start;
                patches.push(BytePatch {
                    range: gap..gap,
                    replacement: Cow::Owned(format!(" depends_on={} ", edges.join(",")).into_bytes()),
                });
            }
        } else if !edges.is_empty() {
            let mut prefix = String::new();
            if item.header_end == item.content_end { prefix.push_str(self.eol()); }
            prefix.push_str(&format!("  <!-- cockpit-relations: depends_on={} -->{}", edges.join(","), self.eol()));
            patches.push(BytePatch {
                range: item.header_end..item.header_end,
                replacement: Cow::Owned(prefix.into_bytes()),
            });
        }
        self.publish_item(task_id, item, patches)
    }

    pub(crate) fn update(
        &self,
        task_id: &str,
        expected_task_revision: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<Task, InspectionError> {
        let item = self.item(task_id, expected_task_revision)?;
        let mut patches = Vec::with_capacity(2);
        if let Some(title) = title {
            validate_authoring_title(title)?;
            patches.push(BytePatch {
                range: item.title_range.clone(),
                replacement: Cow::Borrowed(title.as_bytes()),
            });
        }
        if let Some(description) = description {
            validate_description(description)?;
            let range = item.description_range.clone().ok_or_else(|| {
                InspectionError::new("task_description_ambiguous",
                    "description has no single safe prose slot; edit canonical source instead")
            })?;
            let mut replacement = String::new();
            if item.header_end == item.content_end && !description.is_empty() {
                replacement.push_str(self.eol());
            }
            replacement.push_str(&encode_body(description, self.eol()));
            patches.push(BytePatch { range, replacement: Cow::Owned(replacement.into_bytes()) });
        }
        self.publish_item(task_id, item, patches)
    }

    pub(crate) fn step(
        &self,
        task_id: &str,
        expected_task_revision: &str,
        intent: StepIntent<'_>,
    ) -> Result<Task, InspectionError> {
        let item = self.item(task_id, expected_task_revision)?;
        let source = std::str::from_utf8(&self.bytes).expect("checked UTF-8");
        let patches = steps::plan(source, &item.context, &item.steps, intent)?;
        steps::validate_patches(source, &item.context, &item.steps, &patches)?;
        for patch in &patches {
            if patch.range.start < item.header_end
                || item.task_marker_range.as_ref().is_some_and(|r| touches(&patch.range, r))
                || item.relations.protected_ranges.iter().any(|r| {
                    touches(&patch.range, r) || (patch.range.is_empty() && r.contains(&patch.range.start))
                })
            {
                return Err(invalid_patch());
            }
        }
        self.publish_item(task_id, item, patches)
    }

    fn publish_item(
        &self, task_id: &str, item: &ItemLayout, mut patches: Vec<BytePatch<'_>>,
    ) -> Result<Task, InspectionError> {
        self.require_lock()?;
        patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
        for patch in &patches {
            if patch.range.start < item.range.start || patch.range.end > item.range.end {
                return Err(invalid_patch());
            }
        }
        let old_continuation = item.range.end - item.header_end;
        let delta = patches.iter().filter(|p| p.range.start >= item.header_end)
            .try_fold(0isize, |delta, p| {
                delta.checked_add(p.replacement.len() as isize - p.range.len() as isize)
            }).ok_or_else(invalid_patch)?;
        let continuation = old_continuation.checked_add_signed(delta).ok_or_else(invalid_patch)?;
        if continuation > MAX_TASK_TEXT_BYTES && continuation > old_continuation {
            return Err(InspectionError::new("invalid_task",
                "complete task continuation including relations and steps exceeds 16 KiB"));
        }
        if patches.is_empty() {
            if hash(&read_bytes(self.dir, &self.name)?) != self.doc_revision {
                return Err(conflict());
            }
            return Ok(self.task(task_id)?.clone());
        }
        let bytes = splice(&self.bytes, &patches)?;
        self.commit_task(bytes, task_id)
    }

    /// The checkbox byte is the entire edit: bullets, line endings, marker,
    /// title, free Markdown body, and every unrelated byte remain untouched.
    pub(crate) fn check(
        &self,
        task_id: &str,
        expected_task_revision: &str,
        checked: bool,
    ) -> Result<Task, InspectionError> {
        let item = self.item(task_id, expected_task_revision)?;
        let replacement = [if checked { b'x' } else { b' ' }];
        self.publish_item(task_id, item, vec![BytePatch {
            range: item.context.top_checkbox_offset..item.context.top_checkbox_offset + 1,
            replacement: Cow::Borrowed(&replacement),
        }])
    }

    /// Recover acceptance only when checking was the entire canonical edit.
    pub(crate) fn checked_matches_revision(
        &self,
        task_id: &str,
        expected_unchecked_revision: &str,
    ) -> Result<bool, InspectionError> {
        let task = self.task(task_id)?;
        if !task.checked {
            return Ok(false);
        }
        let item = self
            .items
            .iter()
            .find(|item| {
                item.task_index
                    .is_some_and(|index| self.tasks[index].task_id == task.task_id)
            })
            .ok_or_else(|| InspectionError::new("task_not_found", "task item is missing"))?;
        let checkbox = item.range.start + 3;
        let mut digest = Sha256::new();
        digest.update(&self.bytes[item.range.start..checkbox]);
        digest.update(b" ");
        digest.update(&self.bytes[checkbox + 1..item.range.end]);
        Ok(digest_matches_revision(
            &digest.finalize(),
            expected_unchecked_revision,
        ))
    }

    pub(crate) fn assign_ids(
        &self,
        expected_doc_revision: &str,
    ) -> Result<(u32, String), InspectionError> {
        self.require_lock()?;
        if self.doc_revision != expected_doc_revision {
            return Err(conflict());
        }
        let mut patches = Vec::new();
        for item in self.items.iter().filter(|item| item.task_index.is_none()) {
            let id = self.new_id();
            patches.push(BytePatch {
                range: item.content_end..item.content_end,
                replacement: Cow::Owned(format!(" <!-- cockpit-task: {id} -->").into_bytes()),
            });
        }
        let assigned = patches.len() as u32;
        if assigned == 0 {
            if hash(&read_bytes(self.dir, &self.name)?) != self.doc_revision {
                return Err(conflict());
            }
            return Ok((0, self.doc_revision.clone()));
        }
        let updated = self.commit(splice(&self.bytes, &patches)?)?;
        Ok((assigned, updated.doc_revision))
    }

    fn new_id(&self) -> String {
        loop {
            let id = Uuid::new_v4().to_string();
            if !self.tasks.iter().any(|task| task.task_id == id) {
                return id;
            }
        }
    }

    fn eol(&self) -> &'static str {
        match self.bytes.iter().position(|byte| *byte == b'\n') {
            Some(index) if index > 0 && self.bytes[index - 1] == b'\r' => "\r\n",
            _ => "\n",
        }
    }

    fn require_lock(&self) -> Result<(), InspectionError> {
        if self.lock.is_none() {
            return Err(InspectionError::new(
                "orchestration_state_lock",
                "task mutation requires the named state transaction",
            ));
        }
        Ok(())
    }

    fn prepare_commit(&self, bytes: Vec<u8>) -> Result<TaskDocument<'a>, InspectionError> {
        self.require_lock()?;
        if bytes.len() > MAX_TASK_DOCUMENT_BYTES {
            return Err(InspectionError::new(
                "tasks_full",
                "canonical task document exceeds 8 MiB",
            ));
        }
        parse_document(self.dir, self.name.clone(), bytes, self.lock)
    }

    fn commit_task(&self, bytes: Vec<u8>, task_id: &str) -> Result<Task, InspectionError> {
        let task_id = validate_uuid(task_id)?.to_string();
        let mut updated = self.prepare_commit(bytes)?;
        let target = updated.tasks.iter().position(|task| task.task_id == task_id)
            .ok_or_else(|| InspectionError::new("task_not_found",
                "planned mutation would lose the intended canonical task identity"))?;
        if updated.tasks[target].diagnostic.as_deref() == Some("task_id_duplicate") {
            return Err(InspectionError::new("task_id_duplicate",
                "planned mutation would duplicate the intended canonical task identity"));
        }
        // The complete return payload is already parsed and its identity proved.
        // Moving it out avoids cloning and any fallible lookup after publication.
        let task = updated.tasks.swap_remove(target);
        self.publish_prepared(&updated)?;
        Ok(task)
    }

    fn commit(&self, bytes: Vec<u8>) -> Result<TaskDocument<'a>, InspectionError> {
        let updated = self.prepare_commit(bytes)?;
        self.publish_prepared(&updated)?;
        Ok(updated)
    }

    fn publish_prepared(&self, updated: &TaskDocument<'_>) -> Result<(), InspectionError> {
        // External editors do not take .state.lock. Compare after temp fsync,
        // immediately before rename; only the reread-to-rename window remains.
        atomic_write_bytes_checked(self.dir, &self.name, &updated.bytes, || {
            let current = read_bytes(self.dir, &self.name).map_err(io::Error::other)?;
            if hash(&current) != self.doc_revision {
                return Err(io::Error::other(conflict()));
            }
            Ok(())
        })
        .map_err(|error| {
            error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<InspectionError>())
                .cloned()
                .unwrap_or_else(|| InspectionError::new("tasks_write", error.to_string()))
        })?;
        Ok(())
    }
}

pub(crate) fn validate_creation(title: &str, body: &str) -> Result<(), InspectionError> {
    validate_title(title)?;
    validate_body(body)
}

fn validate_title(title: &str) -> Result<(), InspectionError> {
    if title.trim().is_empty()
        || title.len() > MAX_TASK_TEXT_BYTES
        || title.contains(['\r', '\n', '\0'])
    {
        return Err(InspectionError::new(
            "invalid_task",
            "task title must be nonempty, single-line, and at most 16 KiB",
        ));
    }
    Ok(())
}
fn validate_body(body: &str) -> Result<(), InspectionError> {
    if body.len() > MAX_TASK_TEXT_BYTES || body.contains('\0') {
        return Err(InspectionError::new(
            "invalid_task",
            "task body must be at most 16 KiB and contain no NUL",
        ));
    }
    Ok(())
}

const RELATIONS_MARKER: &str = "<!-- cockpit-relations:";
const RESERVED_MARKERS: [&str; 4] = [
    MARKER, RELATIONS_MARKER, "<!-- cockpit-checklist:", "<!-- cockpit-step:",
];

pub(crate) fn validate_authoring_creation(title: &str, description: &str) -> Result<(), InspectionError> {
    validate_authoring_title(title)?;
    validate_description(description)
}

fn validate_authoring_title(title: &str) -> Result<(), InspectionError> {
    validate_title(title)?;
    if RESERVED_MARKERS.iter().any(|marker| title.contains(marker)) {
        return Err(InspectionError::new("invalid_task", "task title contains reserved metadata"));
    }
    Ok(())
}

/// Code and quoted examples have no managed authoring authority.
fn example_ranges(source: &str, offset: usize) -> Vec<Range<usize>> {
    Parser::new_ext(source, Options::ENABLE_TASKLISTS).into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Start(Tag::CodeBlock(_) | Tag::BlockQuote(_)) | Event::Code(_) =>
                Some(range.start + offset..range.end + offset),
            _ => None,
        }).collect()
}

fn validate_description(description: &str) -> Result<(), InspectionError> {
    validate_body(description)?;
    let examples = example_ranges(description, 0);
    let in_example = |offset| examples.iter().any(|r| r.contains(&offset));
    for marker in RESERVED_MARKERS {
        if description.match_indices(marker).any(|(offset, _)| !in_example(offset)) {
            return Err(InspectionError::new("invalid_task",
                "description contains reserved metadata; use Dependencies or Steps"));
        }
    }
    for (event, range) in Parser::new_ext(description, Options::ENABLE_TASKLISTS).into_offset_iter() {
        if matches!(event, Event::TaskListMarker(_)) && !in_example(range.start) {
            return Err(InspectionError::new("invalid_task",
                "description contains an unfenced checklist; use Steps or a fenced example"));
        }
    }
    Ok(())
}

fn normalized_dependencies(ids: &[String]) -> Result<Vec<String>, InspectionError> {
    if ids.len() > 32 {
        return Err(InspectionError::new("task_relations_invalid", "at most 32 prerequisites are allowed"));
    }
    let mut ids = ids.iter().map(|id| validate_uuid(id).map(|id| id.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

fn authoring_continuation(description: &str, edges: &[String], follow: Option<&str>, eol: &str) -> String {
    let mut output = String::new();
    if !edges.is_empty() || follow.is_some() {
        output.push_str("  <!-- cockpit-relations:");
        if !edges.is_empty() { output.push_str(&format!(" depends_on={}", edges.join(","))); }
        if let Some(follow) = follow { output.push_str(&format!(" follow_up_of={follow}")); }
        output.push_str(" -->");
        output.push_str(eol);
    }
    output.push_str(&encode_body(description, eol));
    output
}

fn encode_body(body: &str, eol: &str) -> String {
    let mut encoded = String::new();
    for line in body.lines() {
        encoded.push_str("  ");
        encoded.push_str(line);
        encoded.push_str(eol);
    }
    encoded
}

fn invalid_patch() -> InspectionError {
    InspectionError::new("task_patch_invalid", "patches must own disjoint original UTF-8 byte slots")
}

fn touches(a: &Range<usize>, b: &Range<usize>) -> bool {
    if a.is_empty() { b.start < a.start && a.start < b.end }
    else { a.start < b.end && b.start < a.end }
}

fn splice(bytes: &[u8], patches: &[BytePatch<'_>]) -> Result<Vec<u8>, InspectionError> {
    let source = std::str::from_utf8(bytes).map_err(|_| invalid_patch())?;
    let mut position = 0;
    let mut previous: Option<&Range<usize>> = None;
    let mut size = bytes.len();
    for patch in patches {
        let range = &patch.range;
        if range.start > range.end || range.end > bytes.len()
            || !source.is_char_boundary(range.start) || !source.is_char_boundary(range.end)
            || range.start < position
            || previous.is_some_and(|p| p.start == range.start && (p.is_empty() || range.is_empty()))
            || std::str::from_utf8(&patch.replacement).is_err()
        { return Err(invalid_patch()); }
        size = size.checked_sub(range.len()).and_then(|s| s.checked_add(patch.replacement.len()))
            .ok_or_else(invalid_patch)?;
        position = range.end;
        previous = Some(range);
    }
    if size > MAX_TASK_DOCUMENT_BYTES {
        return Err(InspectionError::new("tasks_full", "canonical task document exceeds 8 MiB"));
    }
    let mut output = Vec::with_capacity(size);
    position = 0;
    for patch in patches {
        output.extend_from_slice(&bytes[position..patch.range.start]);
        output.extend_from_slice(&patch.replacement);
        position = patch.range.end;
    }
    output.extend_from_slice(&bytes[position..]);
    Ok(output)
}

struct Line {
    start: usize,
    content_end: usize,
    end: usize,
}
fn lines(bytes: &[u8]) -> Vec<Line> {
    let mut output = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let content_end = if index > start && bytes[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            output.push(Line {
                start,
                content_end,
                end: index + 1,
            });
            start = index + 1;
        }
    }
    if start < bytes.len() {
        output.push(Line {
            start,
            content_end: bytes.len(),
            end: bytes.len(),
        });
    }
    output
}

fn is_header(content: &[u8]) -> bool {
    content.len() >= 6
        && matches!(content[0], b'-' | b'*')
        && content[1..3] == *b" ["
        && matches!(content[3], b' ' | b'x')
        && content[4..6] == *b"] "
}

fn marker(title: &str) -> Option<(usize, String)> {
    let trimmed = title.trim_end_matches([' ', '\t']);
    let start = trimmed.rfind(MARKER)?;
    let id = trimmed[start + MARKER.len()..].strip_suffix("-->")?.trim();
    Some((start, Uuid::parse_str(id).ok()?.to_string()))
}

fn decode_body(bytes: &[u8]) -> String {
    let body = std::str::from_utf8(bytes).expect("document UTF-8 was checked");
    let mut output = String::new();
    for line in body.split_inclusive('\n') {
        let line = line.strip_prefix("  ").unwrap_or(line);
        output.push_str(line);
    }
    if output.ends_with('\n') {
        output.pop();
        if output.ends_with('\r') {
            output.pop();
        }
    }
    output
}

fn parse_relations(text: &str, continuation: &[Line], examples: &[Range<usize>]) -> RelationsLayout {
    let mut layout = RelationsLayout { repairable: true, ..Default::default() };
    for (line_index, line) in continuation.iter().enumerate() {
        let raw = &text[line.start..line.content_end];
        let direct = raw.strip_prefix("  ").unwrap_or(raw);
        let trimmed = direct.trim_start_matches([' ', '\t']);
        if !trimmed.starts_with(RELATIONS_MARKER)
            || examples.iter().any(|r| r.contains(&(line.start + raw.len() - trimmed.len())))
        { continue; }
        layout.protected_ranges.push(line.start..line.end);
        if line_index != 0 || direct != trimmed || layout.record_range.is_some() {
            layout.diagnostic = Some("misplaced or duplicate relationship record".into());
            layout.repairable = false;
            continue;
        }
        layout.record_range = Some(line.start..line.end);
        let Some(inner) = trimmed.strip_prefix(RELATIONS_MARKER)
            .and_then(|value| value.trim_end_matches([' ', '\t']).strip_suffix("-->"))
        else {
            layout.diagnostic = Some("unclosed relationship record".into());
            layout.repairable = false;
            continue;
        };
        let inner_start = line.start + 2 + RELATIONS_MARKER.len();
        layout.inner_range = Some(inner_start..inner_start + inner.len());
        let mut cursor = 0;
        let mut follows = 0;
        while cursor < inner.len() {
            while cursor < inner.len() && matches!(inner.as_bytes()[cursor], b' ' | b'\t') { cursor += 1; }
            let start = cursor;
            while cursor < inner.len() && !matches!(inner.as_bytes()[cursor], b' ' | b'\t') { cursor += 1; }
            if start == cursor { break; }
            let token = &inner[start..cursor];
            let Some((key, value)) = token.split_once('=') else {
                layout.diagnostic = Some("relationship token requires key=value".into());
                layout.repairable = false;
                continue;
            };
            let owned = RelationToken {
                range: inner_start + start..inner_start + cursor,
                value_range: inner_start + start + key.len() + 1..inner_start + cursor,
            };
            match key {
                "depends_on" => {
                    layout.depends_tokens.push(owned);
                    let parsed = value.split(',').map(|id| Uuid::parse_str(id).map(|id| id.to_string()))
                        .collect::<Result<Vec<_>, _>>();
                    match parsed {
                        Ok(ids) if ids.len() <= 32 => layout.depends_on.extend(ids),
                        _ => layout.diagnostic = Some("invalid or oversized depends_on UUID list".into()),
                    }
                    if layout.depends_tokens.len() > 1 {
                        layout.diagnostic = Some("duplicate depends_on key".into());
                    }
                }
                "follow_up_of" => {
                    follows += 1;
                    match Uuid::parse_str(value) {
                        Ok(id) if follows == 1 => {
                            layout.follow_up_of = Some(id.to_string());
                            layout.follow_token = Some(owned);
                        }
                        _ => {
                            layout.diagnostic = Some("invalid or duplicate follow_up_of key".into());
                            layout.repairable = false;
                        }
                    }
                }
                _ => {
                    layout.diagnostic = Some("unknown relationship key".into());
                    layout.repairable = false;
                }
            }
        }
    }
    layout.depends_on.sort_unstable();
    layout.depends_on.dedup();
    layout
}

/// Project disjoint prose in source order, but author only a proved contiguous slot.
fn description_layout(
    bytes: &[u8], continuation: Range<usize>, protected: &[Range<usize>], ambiguous: bool,
) -> (String, Option<Range<usize>>) {
    let mut spans = protected.to_vec();
    spans.sort_by_key(|range| (range.start, range.end));
    let mut gaps = Vec::new();
    let mut cursor = continuation.start;
    for range in spans {
        let start = range.start.max(continuation.start).min(continuation.end);
        let end = range.end.max(start).min(continuation.end);
        if cursor < start { gaps.push(cursor..start); }
        cursor = cursor.max(end);
    }
    if cursor < continuation.end { gaps.push(cursor..continuation.end); }
    let nonempty: Vec<_> = gaps.iter().filter(|range| {
        !bytes[(**range).clone()].iter().all(u8::is_ascii_whitespace)
    }).collect();
    let editable = if ambiguous || nonempty.len() > 1 { None }
        else if let Some(range) = nonempty.first() { Some((**range).clone()) }
        else { Some(gaps.first().cloned().unwrap_or(continuation.start..continuation.start)) };
    if gaps.len() == 1 {
        return (decode_body(&bytes[gaps[0].clone()]), editable);
    }
    let mut prose = Vec::new();
    for range in gaps { prose.extend_from_slice(&bytes[range]); }
    (decode_body(&prose), editable)
}

fn parse_document<'a>(
    dir: &'a Dir,
    name: String,
    bytes: Vec<u8>,
    lock: Option<&'a LockGuard>,
) -> Result<TaskDocument<'a>, InspectionError> {
    let text = std::str::from_utf8(&bytes).map_err(|e| {
        InspectionError::new("tasks_corrupt", format!("task document is not UTF-8: {e}"))
    })?;
    let doc_revision = hash(&bytes);
    let lines = lines(&bytes);
    let document_eol = match bytes.iter().position(|byte| *byte == b'\n') {
        Some(index) if index > 0 && bytes[index - 1] == b'\r' => DocumentEol::CrLf,
        _ => DocumentEol::Lf,
    };
    let mut tasks = Vec::new();
    let mut items = Vec::new();
    let mut unidentified_items = 0u32;
    let mut index = 0;
    while index < lines.len() {
        let line = &lines[index];
        let content = &bytes[line.start..line.content_end];
        if !is_header(content) {
            index += 1;
            continue;
        }
        let mut next = index + 1;
        while next < lines.len() {
            let candidate = &bytes[lines[next].start..lines[next].content_end];
            if !(candidate.iter().all(|byte| matches!(byte, b' ' | b'\t'))
                || candidate.starts_with(b"  "))
            {
                break;
            }
            next += 1;
        }
        let end = lines[next.saturating_sub(1)].end;
        let examples = example_ranges(&text[line.start..end], line.start);
        let relations = parse_relations(text, &lines[index + 1..next], &examples);
        let context = StepParseContext {
            item_range: line.start..end,
            item_line: index + 1,
            continuation_range: line.end..end,
            top_checkbox_offset: line.start + 3,
            relation_record_range: relations.record_range.clone(),
            eol: document_eol,
            continuation_write_limit: MAX_TASK_TEXT_BYTES,
        };
        let step_layout = steps::parse(text, &context)?;
        let projection = steps::project(text, &context, &step_layout);
        let mut protected = relations.protected_ranges.clone();
        protected.extend_from_slice(steps::protected_ranges(&step_layout));
        if let Some(tail) = steps::managed_tail(&step_layout) {
            protected.push(tail.range.clone());
        }
        let (description, description_range) = description_layout(
            &bytes, relations.record_range.as_ref().map_or(line.end, |r| r.end)..end, &protected,
            projection.progress.is_none() || relations.diagnostic.is_some(),
        );
        let description_diagnostic = description_range.is_none().then(|| {
            "description has ambiguous or interleaved protected source; edit canonical source".to_owned()
        });
        let title_start = line.start + 6;
        let raw_title = &text[title_start..line.content_end];
        let mut title_end = line.content_end;
        let mut task_marker_range = None;
        let task_index = if let Some((marker_start, task_id)) = marker(raw_title) {
            let title = raw_title[..marker_start].trim_end_matches([' ', '\t']);
            title_end = title_start + title.len();
            task_marker_range = Some(title_start + marker_start..line.content_end);
            let task_index = tasks.len();
            tasks.push(Task {
                task_id,
                title: title.to_owned(),
                body: decode_body(&bytes[line.end..end]),
                description,
                description_editable: description_range.is_some(),
                description_diagnostic,
                depends_on: relations.depends_on.clone(),
                follow_up_of: relations.follow_up_of.clone(),
                relations_diagnostic: relations.diagnostic.clone(),
                steps: projection.steps,
                step_progress: projection.progress,
                steps_diagnostic: projection.diagnostic,
                checked: content[3] == b'x',
                line: context.item_line as u32,
                task_revision: hash(&bytes[line.start..end]),
                diagnostic: None,
            });
            Some(task_index)
        } else {
            unidentified_items += 1;
            None
        };
        items.push(ItemLayout {
            range: line.start..end,
            header_end: line.end,
            content_end: line.content_end,
            title_range: title_start..title_end,
            task_marker_range,
            task_index,
            relations,
            description_range,
            context,
            steps: step_layout,
        });
        index = next;
    }
    let mut counts = HashMap::new();
    for task in &tasks {
        *counts.entry(task.task_id.as_str()).or_insert(0usize) += 1;
    }
    let duplicate_indices: Vec<usize> = tasks
        .iter()
        .enumerate()
        .filter_map(|(index, task)| (counts[task.task_id.as_str()] > 1).then_some(index))
        .collect();
    drop(counts);
    for index in duplicate_indices {
        tasks[index].diagnostic = Some("task_id_duplicate".into());
    }
    Ok(TaskDocument {
        tasks,
        doc_revision,
        unidentified_items,
        dir,
        name,
        bytes,
        items,
        lock,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::store::OrchestrationStore;
    use std::path::PathBuf;

    struct Fixture {
        root: PathBuf,
        root_id: String,
        store: OrchestrationStore,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("cockpit-task-markdown-{}", Uuid::new_v4()));
            let store = OrchestrationStore::open(&root).unwrap();
            Self {
                root,
                root_id: Uuid::new_v4().to_string(),
                store,
            }
        }
        fn write(&self, bytes: &[u8]) {
            self.store
                .tasks_dir()
                .write(root_filename(&self.root_id).unwrap(), bytes)
                .unwrap();
        }
        fn bytes(&self) -> Vec<u8> {
            self.store
                .tasks_dir()
                .read(root_filename(&self.root_id).unwrap())
                .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn duplicate_ids_are_both_visible_and_fail_targeted_mutations() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4().to_string();
        fixture.write(format!("- [ ] First <!-- cockpit-task: {id} -->\n- [x] Second <!-- cockpit-task: {id} -->\n").as_bytes());
        let lock = fixture.store.lock().unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        assert_eq!(doc.tasks.len(), 2);
        assert!(
            doc.tasks
                .iter()
                .all(|task| task.diagnostic.as_deref() == Some("task_id_duplicate"))
        );
        let before = fixture.bytes();
        assert_eq!(
            doc.check(&id, &doc.tasks[0].task_revision, true)
                .unwrap_err()
                .code,
            "task_id_duplicate"
        );
        assert_eq!(
            doc.update(&id, &doc.tasks[0].task_revision, Some("Changed"), None)
                .unwrap_err()
                .code,
            "task_id_duplicate"
        );
        assert_eq!(fixture.bytes(), before);
    }

    #[test]
    fn checking_and_title_edit_preserve_unrelated_bytes_and_crlf() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4().to_string();
        let original = format!(
            "# Outside\r\n\r\n* [ ] Old title  <!-- cockpit-task: {id} --> \t\r\n  **body**\r\n\r\nUnrelated\ttext\r\n- [ ] No identity\r\n"
        );
        fixture.write(original.as_bytes());
        let lock = fixture.store.lock().unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        assert_eq!(doc.tasks[0].line, 3);
        assert_eq!(doc.unidentified_items, 1);
        doc.check(&id, &doc.tasks[0].task_revision, true).unwrap();
        assert_eq!(
            fixture.bytes(),
            original.replacen("* [ ]", "* [x]", 1).as_bytes()
        );
        let doc = lock.tasks(&fixture.root_id).unwrap();
        doc.update(&id, &doc.tasks[0].task_revision, Some("New title"), None)
            .unwrap();
        assert_eq!(
            fixture.bytes(),
            original
                .replacen("* [ ]", "* [x]", 1)
                .replacen("Old title", "New title", 1)
                .as_bytes()
        );
    }

    #[test]
    fn task_cas_and_final_document_reread_prevent_external_editor_overwrite() {
        let fixture = Fixture::new();
        let lock = fixture.store.lock().unwrap();
        let created = lock
            .tasks(&fixture.root_id)
            .unwrap()
            .create_with_id(&Uuid::new_v4().to_string(), "One", "body")
            .unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        assert_eq!(
            doc.update(&created.task_id, "stale", Some("wrong"), None)
                .unwrap_err()
                .code,
            "task_revision_conflict"
        );
        let mut external = fixture.bytes();
        external.extend_from_slice(b"External note\n");
        fixture.write(&external);
        assert_eq!(
            doc.check(&created.task_id, &created.task_revision, true)
                .unwrap_err()
                .code,
            "task_revision_conflict"
        );
        assert_eq!(fixture.bytes(), external);
        let refreshed = lock.tasks(&fixture.root_id).unwrap();
        // Unrelated edits do not invalidate an item's independent CAS token.
        assert_eq!(refreshed.tasks[0].task_revision, created.task_revision);
        assert!(
            refreshed
                .check(&created.task_id, &created.task_revision, true)
                .unwrap()
                .checked
        );
    }

    #[test]
    fn assigning_ids_preserves_all_preexisting_bytes_and_ids_are_stable() {
        let fixture = Fixture::new();
        let original = b"# Tasks\n\n- [ ] Plain\n  body\n\nParagraph\n* [x] Checked";
        fixture.write(original);
        let lock = fixture.store.lock().unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        assert_eq!(
            doc.assign_ids("stale").unwrap_err().code,
            "task_revision_conflict"
        );
        assert_eq!(doc.assign_ids(&doc.doc_revision).unwrap().0, 2);
        let doc = lock.tasks(&fixture.root_id).unwrap();
        assert_eq!(doc.tasks.len(), 2);
        assert!(
            doc.tasks
                .iter()
                .all(|task| Uuid::parse_str(&task.task_id).is_ok())
        );
        let mut restored = String::from_utf8(fixture.bytes()).unwrap();
        for task in &doc.tasks {
            restored = restored.replace(&format!(" <!-- cockpit-task: {} -->", task.task_id), "");
        }
        assert_eq!(restored.as_bytes(), original);
        let ids: Vec<_> = doc.tasks.iter().map(|task| task.task_id.clone()).collect();
        assert_eq!(doc.assign_ids(&doc.doc_revision).unwrap().0, 0);
        assert_eq!(
            lock.tasks(&fixture.root_id)
                .unwrap()
                .tasks
                .iter()
                .map(|task| task.task_id.clone())
                .collect::<Vec<_>>(),
            ids
        );
    }

    #[test]
    fn accept_recovery_can_observe_already_checked_task_after_reopen() {
        let fixture = Fixture::new();
        let lock = fixture.store.lock().unwrap();
        let task = lock
            .tasks(&fixture.root_id)
            .unwrap()
            .create_with_id(&Uuid::new_v4().to_string(), "Accept me", "work")
            .unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        doc.check(&task.task_id, &task.task_revision, true).unwrap();
        drop(doc);
        drop(lock);
        let reopened = OrchestrationStore::open(&fixture.root).unwrap();
        let lock = reopened.lock().unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        assert!(doc.task(&task.task_id).unwrap().checked);
        assert_ne!(
            doc.task(&task.task_id).unwrap().task_revision,
            task.task_revision
        );
    }

    #[test]
    fn read_only_projection_document_cannot_mutate() {
        let fixture = Fixture::new();
        let doc = read_document(fixture.store.tasks_dir(), &fixture.root_id).unwrap();
        assert_eq!(
            doc.create_with_id(&Uuid::new_v4().to_string(), "No lock", "").unwrap_err().code,
            "orchestration_state_lock"
        );
    }

    #[test]
    fn final_precondition_runs_after_temp_write_and_cleans_up_on_conflict() {
        let fixture = Fixture::new();
        let name = root_filename(&fixture.root_id).unwrap();
        fixture.write(b"external original\n");
        let error =
            atomic_write_bytes_checked(fixture.store.tasks_dir(), &name, b"replacement\n", || {
                let temp = fixture
                    .store
                    .tasks_dir()
                    .entries()
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name())
                    .find(|name| name.to_string_lossy().ends_with(".tmp"))
                    .unwrap();
                assert_eq!(
                    fixture.store.tasks_dir().read(temp).unwrap(),
                    b"replacement\n"
                );
                fixture.write(b"external winner\n");
                Err(io::Error::other(conflict()))
            })
            .unwrap_err();
        assert!(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<InspectionError>()
                .is_some()
        );
        assert_eq!(fixture.bytes(), b"external winner\n");
        assert!(!fixture.store.tasks_dir().entries().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[test]
    fn checked_recovery_requires_exact_item_bytes_except_checkbox() {
        let fixture = Fixture::new();
        let task_id = Uuid::new_v4().to_string();
        fixture.write(format!(
            "# User notes\r\n* [ ] Custom title  <!-- cockpit-task: {task_id} -->\r\n  body\ttext\r\n"
        ).as_bytes());
        let locked = fixture.store.lock().unwrap();
        let original = locked
            .tasks(&fixture.root_id)
            .unwrap()
            .task(&task_id)
            .unwrap()
            .clone();
        assert!(
            !locked
                .tasks(&fixture.root_id)
                .unwrap()
                .checked_matches_revision(&task_id, &original.task_revision)
                .unwrap()
        );
        locked
            .tasks(&fixture.root_id)
            .unwrap()
            .check(&task_id, &original.task_revision, true)
            .unwrap();
        assert!(
            locked
                .tasks(&fixture.root_id)
                .unwrap()
                .checked_matches_revision(&task_id, &original.task_revision)
                .unwrap()
        );
        let checked = locked
            .tasks(&fixture.root_id)
            .unwrap()
            .task(&task_id)
            .unwrap()
            .clone();
        locked
            .tasks(&fixture.root_id)
            .unwrap()
            .update(
                &task_id,
                &checked.task_revision,
                None,
                Some("externally changed body"),
            )
            .unwrap();
        assert!(
            !locked
                .tasks(&fixture.root_id)
                .unwrap()
                .checked_matches_revision(&task_id, &original.task_revision)
                .unwrap()
        );
        let changed = locked
            .tasks(&fixture.root_id)
            .unwrap()
            .task(&task_id)
            .unwrap()
            .clone();
        locked
            .tasks(&fixture.root_id)
            .unwrap()
            .update(
                &task_id,
                &changed.task_revision,
                Some("Changed title"),
                None,
            )
            .unwrap();
        assert!(
            !locked
                .tasks(&fixture.root_id)
                .unwrap()
                .checked_matches_revision(&task_id, &original.task_revision)
                .unwrap()
        );
        let mut bytes = fixture.bytes();
        bytes.extend_from_slice(
            format!("* [x] Duplicate <!-- cockpit-task: {task_id} -->\r\n").as_bytes(),
        );
        fixture.write(&bytes);
        assert_eq!(
            locked
                .tasks(&fixture.root_id)
                .unwrap()
                .checked_matches_revision(&task_id, &original.task_revision)
                .unwrap_err()
                .code,
            "task_id_duplicate"
        );
    }

    #[test]
    fn stable_creation_uses_supplied_uuid_and_rejects_duplicate_without_editing() {
        let fixture = Fixture::new();
        fixture.write(b"# User tasks\r\n\r\nUnrelated prose\r\n");
        let lock = fixture.store.lock().unwrap();
        let task_id = Uuid::new_v4().to_string();
        let task = lock
            .tasks(&fixture.root_id)
            .unwrap()
            .create_with_id(&task_id.to_uppercase(), "Stable title", "First\r\nSecond\n")
            .unwrap();
        assert_eq!(task.task_id, task_id);
        let document = lock.tasks(&fixture.root_id).unwrap();
        assert!(document.matches_creation(&task, "Stable title", "First\r\nSecond\n"));
        let before = fixture.bytes();
        assert!(before.starts_with(b"# User tasks\r\n\r\nUnrelated prose\r\n"));
        assert_eq!(
            document
                .create_with_id(&task_id, "Other", "")
                .unwrap_err()
                .code,
            "task_id_conflict"
        );
        assert_eq!(fixture.bytes(), before);
        assert_eq!(
            document
                .create_with_id("not-a-uuid", "Other", "")
                .unwrap_err()
                .code,
            "invalid_identity"
        );
        assert_eq!(fixture.bytes(), before);
    }

    #[test]
    fn bounded_task_document_reads_reject_oversized_files() {
        let fixture = Fixture::new();
        fixture.write(&vec![b'x'; MAX_TASK_DOCUMENT_BYTES + 1]);
        let lock = fixture.store.lock().unwrap();
        assert!(lock.tasks(&fixture.root_id).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn task_symlinks_and_nonregular_nodes_fail_closed() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = fixture.root.join("outside.md");
        std::fs::write(&outside, b"do not overwrite").unwrap();
        symlink(
            &outside,
            fixture
                .store
                .base()
                .join("tasks")
                .join(root_filename(&fixture.root_id).unwrap()),
        )
        .unwrap();
        let lock = fixture.store.lock().unwrap();
        assert!(lock.tasks(&fixture.root_id).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"do not overwrite");
    }

    const TASK: &str = "11111111-1111-4111-8111-111111111111";
    const SOURCE: &str = "abcdef22-2222-4222-8222-222222222222";
    const STEP: &str = "33333333-3333-4333-8333-333333333333";

    #[test]
    fn step_coordinates_use_original_document_lines_and_utf8_byte_offsets() {
        for eol in ["\n", "\r\n"] {
            let fixture = Fixture::new();
            let parent_id = "44444444-4444-4444-8444-444444444444";
            let child_id = "55555555-5555-4555-8555-555555555555";
            let original = format!(
                "# Contexte 日本語{eol}{eol}Préambule café.{eol}{eol}\
                 - [ ] Première tâche <!-- cockpit-task: {TASK} -->{eol}\
                 \x20\x20<!-- cockpit-relations: depends_on={SOURCE} -->{eol}\
                 \x20\x20Prose naïve.{eol}{eol}\
                 \x20\x20<!-- cockpit-checklist: begin -->{eol}\
                 \x20\x20- [ ] Première étape <!-- cockpit-step: {STEP} -->{eol}\
                 \x20\x20<!-- cockpit-checklist: end -->{eol}{eol}\
                 Texte entre les tâches.{eol}{eol}\
                 - [ ] Deuxième tâche <!-- cockpit-task: {SOURCE} -->{eol}\
                 \x20\x20Corps 日本語.{eol}{eol}\
                 \x20\x20- [x] Parent 日本語 <!-- cockpit-step: {parent_id} -->{eol}\
                 \x20\x20\x20\x20- [ ] Enfant café <!-- cockpit-step: {child_id} -->"
            );
            fixture.write(original.as_bytes());
            let lock = fixture.store.lock().unwrap();
            let doc = lock.tasks(&fixture.root_id).unwrap();
            assert_eq!(doc.tasks.len(), 2);
            let first = doc.task(TASK).unwrap();
            let second = doc.task(SOURCE).unwrap();
            assert_eq!((first.line, second.line), (5, 15));
            assert_eq!((first.steps.len(), second.steps.len()), (1, 2));
            let first_start = original.find("- [ ] Première tâche").unwrap();
            let second_start = original.find("- [ ] Deuxième tâche").unwrap();
            for (step, item_start, line, header, step_id) in [
                (&first.steps[0], first_start, 10, "  - [ ] Première étape", STEP),
                (&second.steps[0], second_start, 18, "  - [x] Parent 日本語", parent_id),
                (&second.steps[1], second_start, 19, "    - [ ] Enfant café", child_id),
            ] {
                assert_eq!(step.line, line);
                assert_eq!(step.step_id.as_deref(), Some(step_id));
                assert_eq!(step.source_offset as usize, original.find(header).unwrap() - item_start);
                let absolute_offset = item_start + step.source_offset as usize;
                assert!(original[absolute_offset..].starts_with(header));
            }
            assert_eq!(second.steps[1].parent_step_id.as_deref(), Some(parent_id));
            assert_eq!(second.steps[1].depth, 1);
            assert_eq!(fixture.bytes(), original.as_bytes());
        }
    }

    #[test]
    fn prose_and_dependency_writes_preserve_managed_bytes_and_raw_provenance() {
        for eol in ["\n", "\r\n"] {
            let fixture = Fixture::new();
            let provenance = format!("follow_up_of={}", SOURCE.to_uppercase());
            let original = format!(
                "* [ ] Café  <!-- cockpit-task: {TASK} --> \t{eol}  <!-- cockpit-relations: {provenance}\tdepends_on={SOURCE} --> \t{eol}  Old prose{eol}  <!-- cockpit-checklist: begin -->{eol}  - [X] Saved <!-- cockpit-step: {STEP} -->{eol}  <!-- cockpit-checklist: end -->{eol}- [ ] Sibling <!-- cockpit-task: {SOURCE} -->{eol}"
            );
            fixture.write(original.as_bytes());
            let lock = fixture.store.lock().unwrap();
            let doc = lock.tasks(&fixture.root_id).unwrap();
            let task = doc.task(TASK).unwrap();
            assert_eq!(task.description, "Old prose");
            assert!(task.body.contains("cockpit-relations:"));
            assert!(task.body.contains("cockpit-step:"));
            assert_eq!(task.steps.len(), 1);
            let updated = doc.update(TASK, &task.task_revision, Some("New café"), Some("New prose")).unwrap();
            assert_eq!(updated.description, "New prose");
            let expected = original.replacen("Café", "New café", 1).replacen("Old prose", "New prose", 1);
            assert_eq!(fixture.bytes(), expected.as_bytes());
            let doc = lock.tasks(&fixture.root_id).unwrap();
            let cleared = doc.set_dependencies(TASK, &updated.task_revision, &doc.doc_revision, &[]).unwrap();
            assert!(cleared.depends_on.is_empty());
            assert_eq!(cleared.follow_up_of.as_deref(), Some(SOURCE));
            assert_eq!(fixture.bytes(), expected.replacen(&format!("depends_on={SOURCE}"), "", 1).as_bytes());
        }
    }

    #[test]
    fn interleaved_legacy_prose_refuses_description_but_preserves_title_slot() {
        let fixture = Fixture::new();
        let original = format!(
            "- [ ] Original <!-- cockpit-task: {TASK} -->\n  Before\n  - [ ] Legacy\n  After\n"
        );
        fixture.write(original.as_bytes());
        let lock = fixture.store.lock().unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        let task = doc.task(TASK).unwrap();
        assert!(!task.description_editable);
        assert!(task.description.contains("Before"));
        assert!(task.description.contains("After"));
        assert_eq!(task.steps.len(), 1);
        assert_eq!(doc.update(TASK, &task.task_revision, None, Some("Replacement")).unwrap_err().code,
            "task_description_ambiguous");
        assert_eq!(fixture.bytes(), original.as_bytes());
        doc.update(TASK, &task.task_revision, Some("Changed"), None).unwrap();
        assert_eq!(fixture.bytes(), original.replacen("Original", "Changed", 1).as_bytes());
    }

    #[test]
    fn fenced_examples_remain_prose_and_unfenced_control_injection_is_refused() {
        let fixture = Fixture::new();
        let lock = fixture.store.lock().unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        let description = format!(
            "Example\n```md\n<!-- cockpit-relations: depends_on={SOURCE} -->\n- [ ] Example <!-- cockpit-step: {STEP} -->\n```\n"
        );
        let task = doc.create_authoring_with_id(TASK, "Examples", &description, &[], None, None, None).unwrap();
        assert!(task.depends_on.is_empty());
        assert!(task.relations_diagnostic.is_none());
        assert!(task.steps.is_empty());
        assert_eq!(task.description, description.trim_end_matches('\n'));
        let before = fixture.bytes();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        for injected in [
            format!("<!-- cockpit-relations: depends_on={SOURCE} -->"),
            format!("<!-- cockpit-step: {STEP} -->"),
            "- [ ] Forged checklist".into(),
            "<!-- cockpit-checklist: begin -->".into(),
        ] {
            assert_eq!(doc.update(TASK, &task.task_revision, None, Some(&injected)).unwrap_err().code,
                "invalid_task");
        }
        assert_eq!(doc.update(TASK, &task.task_revision,
            Some(&format!("Forged <!-- cockpit-task: {SOURCE} -->")), None).unwrap_err().code, "invalid_task");
        assert_eq!(fixture.bytes(), before);
    }

    #[test]
    fn relations_are_read_without_rewriting_and_bounded_depends_repair_preserves_provenance() {
        let fixture = Fixture::new();
        let original = format!(
            "- [ ] Repair <!-- cockpit-task: {TASK} -->\n  <!-- cockpit-relations: depends_on=bad\tfollow_up_of={SOURCE} depends_on={SOURCE} -->\n  Saved\n"
        );
        fixture.write(original.as_bytes());
        let lock = fixture.store.lock().unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        let task = doc.task(TASK).unwrap();
        assert!(task.relations_diagnostic.is_some());
        assert_eq!(fixture.bytes(), original.as_bytes());
        let repaired = doc.set_dependencies(TASK, &task.task_revision, &doc.doc_revision,
            &[SOURCE.to_owned()]).unwrap();
        assert!(repaired.relations_diagnostic.is_none());
        assert_eq!(fixture.bytes(), original.replacen("depends_on=bad", &format!("depends_on={SOURCE}"), 1)
            .replacen(&format!(" depends_on={SOURCE} -->"), "  -->", 1).as_bytes());
    }

    #[test]
    fn unsafe_relation_records_refuse_without_source_loss() {
        for continuation in [
            format!("  Prose\n  <!-- cockpit-relations: depends_on={SOURCE} -->\n"),
            format!("  <!-- cockpit-relations: depends_on={SOURCE}\n"),
            format!("  <!-- cockpit-relations: unknown={SOURCE} -->\n"),
            format!("  <!-- cockpit-relations: follow_up_of=bad depends_on={SOURCE} -->\n"),
            format!("  <!-- cockpit-relations: depends_on={SOURCE} -->\n  <!-- cockpit-relations: depends_on={SOURCE} -->\n"),
            format!("  <!-- cockpit-relations: depends_on={} -->\n",
                std::iter::repeat_n(SOURCE, 33).collect::<Vec<_>>().join(",")),
        ] {
            let fixture = Fixture::new();
            let original = format!("- [ ] Invalid <!-- cockpit-task: {TASK} -->\n{continuation}");
            fixture.write(original.as_bytes());
            let lock = fixture.store.lock().unwrap();
            let doc = lock.tasks(&fixture.root_id).unwrap();
            let task = doc.task(TASK).unwrap();
            assert!(task.relations_diagnostic.is_some());
            // Oversized but bounded depends_on is deliberately repairable outside live work.
            if !continuation.contains(&std::iter::repeat_n(SOURCE, 33).collect::<Vec<_>>().join(",")) {
                assert_eq!(doc.set_dependencies(TASK, &task.task_revision, &doc.doc_revision, &[])
                    .unwrap_err().code, "task_relations_invalid");
            }
            assert_eq!(fixture.bytes(), original.as_bytes());
        }
    }

    #[test]
    fn stable_authoring_retries_are_exact_and_graph_fences_cover_source_and_document() {
        let fixture = Fixture::new();
        let lock = fixture.store.lock().unwrap();
        let source = lock.tasks(&fixture.root_id).unwrap()
            .create_authoring_with_id(SOURCE, "Source", "", &[], None, None, None).unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        let edges = vec![SOURCE.to_owned()];
        let before = fixture.bytes();
        assert_eq!(doc.create_authoring_with_id(TASK, "Follow-up", "Description", &edges, Some(SOURCE),
            Some("stale"), Some(&source.task_revision)).unwrap_err().code, "task_revision_conflict");
        assert_eq!(doc.create_authoring_with_id(TASK, "Follow-up", "Description", &edges, Some(SOURCE),
            Some(&doc.doc_revision), Some("stale")).unwrap_err().code, "task_revision_conflict");
        assert_eq!(fixture.bytes(), before);
        let task = doc.create_authoring_with_id(TASK, "Follow-up", "Description", &edges, Some(SOURCE),
            Some(&doc.doc_revision), Some(&source.task_revision)).unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        let before = fixture.bytes();
        assert_eq!(doc.create_authoring_with_id(TASK, "Follow-up", "Description", &edges, Some(SOURCE),
            None, Some("stale")).unwrap_err().code, "invalid_task");
        assert_eq!(doc.create_authoring_with_id(TASK, "Follow-up", "Description", &edges, Some(SOURCE),
            Some("stale"), None).unwrap_err().code, "invalid_task");
        assert_eq!(doc.create_authoring_with_id(SOURCE, "Source", "", &[], None,
            None, Some("stale")).unwrap_err().code, "invalid_task");
        assert_eq!(doc.create_authoring_with_id(TASK, "Follow-up",
            &"x".repeat(MAX_TASK_TEXT_BYTES - 1), &edges, Some(SOURCE),
            Some("stale"), Some("stale")).unwrap_err().code, "invalid_task");
        assert_eq!(fixture.bytes(), before);
        let retry = doc.create_authoring_with_id(TASK, "Follow-up", "Description", &edges, Some(SOURCE),
            Some("stale"), Some("stale")).unwrap();
        assert_eq!(retry.task_revision, task.task_revision);
        assert_eq!(fixture.bytes(), before);
        assert_eq!(doc.create_authoring_with_id(TASK, "Changed", "Description", &edges, Some(SOURCE),
            Some(&doc.doc_revision), Some(&source.task_revision)).unwrap_err().code, "task_id_conflict");
        assert_eq!(doc.set_dependencies(TASK, &task.task_revision, "stale", &edges).unwrap_err().code,
            "task_revision_conflict");
        assert_eq!(doc.set_dependencies(TASK, "stale", &doc.doc_revision, &edges).unwrap_err().code,
            "task_revision_conflict");
        assert_eq!(fixture.bytes(), before);
    }

    #[test]
    fn relations_only_and_no_eol_description_slots_do_not_relocate_metadata() {
        for original in [
            format!("- [ ] Plain <!-- cockpit-task: {TASK} -->"),
            format!("- [ ] Plain <!-- cockpit-task: {TASK} -->\n  <!-- cockpit-relations: depends_on={SOURCE} -->\n"),
        ] {
            let fixture = Fixture::new();
            fixture.write(original.as_bytes());
            let lock = fixture.store.lock().unwrap();
            let doc = lock.tasks(&fixture.root_id).unwrap();
            let task = doc.task(TASK).unwrap();
            doc.update(TASK, &task.task_revision, None, Some("Added")).unwrap();
            let expected = if original.ends_with('\n') { format!("{original}  Added\n") }
                else { format!("{original}\n  Added\n") };
            assert_eq!(fixture.bytes(), expected.as_bytes());
        }
    }

    #[test]
    fn real_step_intents_share_item_cas_and_preserve_relation_prose_and_task_checkbox() {
        let fixture = Fixture::new();
        let original = format!(
            "- [ ] Task <!-- cockpit-task: {TASK} -->\n  <!-- cockpit-relations: depends_on={SOURCE} -->\n  Prose\n"
        );
        fixture.write(original.as_bytes());
        let lock = fixture.store.lock().unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        let task = doc.task(TASK).unwrap();
        let added = doc.step(TASK, &task.task_revision, StepIntent::Add {
            step_id: Uuid::parse_str(STEP).unwrap(), parent_step_id: None,
            before_step_id: None, title: "New step",
        }).unwrap();
        assert_eq!(added.steps.len(), 1);
        assert_eq!(added.steps[0].step_id.as_deref(), Some(STEP));
        assert!(!added.checked);
        assert!(fixture.bytes().starts_with(original.as_bytes()));
        let doc = lock.tasks(&fixture.root_id).unwrap();
        let before = fixture.bytes();
        assert_eq!(doc.update(TASK, &task.task_revision, None, Some("Stale")).unwrap_err().code,
            "task_revision_conflict");
        assert_eq!(fixture.bytes(), before);
        let checked = doc.step(TASK, &added.task_revision, StepIntent::SetChecked {
            step_id: Uuid::parse_str(STEP).unwrap(), checked: true,
            scope: cockpit_protocol::orchestration::TaskStepScope::Leaf,
        }).unwrap();
        assert!(!checked.checked);
        assert_eq!(checked.depends_on, vec![SOURCE.to_owned()]);
        assert!(checked.steps[0].checked);
        assert_eq!(fixture.bytes(), String::from_utf8(before).unwrap()
            .replacen("- [ ] New step", "- [x] New step", 1).as_bytes());
    }

    #[test]
    fn complete_raw_continuation_budget_includes_relations_and_steps_but_allows_shrinking() {
        let fixture = Fixture::new();
        let lock = fixture.store.lock().unwrap();
        let doc = lock.tasks(&fixture.root_id).unwrap();
        let description = "x".repeat(MAX_TASK_TEXT_BYTES - 1);
        assert_eq!(doc.create_authoring_with_id(TASK, "Too large", &description, &[], None, None, None)
            .unwrap_err().code, "invalid_task");
        assert!(read_document(fixture.store.tasks_dir(), &fixture.root_id).unwrap().tasks.is_empty());
        let original = format!(
            "- [ ] Oversized <!-- cockpit-task: {TASK} -->\n  {}\n", "x".repeat(MAX_TASK_TEXT_BYTES)
        );
        fixture.write(original.as_bytes());
        let doc = lock.tasks(&fixture.root_id).unwrap();
        let task = doc.task(TASK).unwrap();
        assert_eq!(doc.set_dependencies(TASK, &task.task_revision, &doc.doc_revision,
            &[SOURCE.to_owned()]).unwrap_err().code, "invalid_task");
        assert_eq!(fixture.bytes(), original.as_bytes());
        let checked = doc.check(TASK, &task.task_revision, true).unwrap();
        assert!(checked.checked);
        let doc = lock.tasks(&fixture.root_id).unwrap();
        doc.update(TASK, &checked.task_revision, None, Some("Smaller")).unwrap();
        assert_eq!(fixture.bytes(), format!(
            "- [x] Oversized <!-- cockpit-task: {TASK} -->\n  Smaller\n"
        ).as_bytes());
    }
}
