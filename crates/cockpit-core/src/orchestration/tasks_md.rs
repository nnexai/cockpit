use std::{collections::HashMap, io, ops::Range};

use cap_std::fs::Dir;
use cockpit_protocol::orchestration::Task;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    InspectionError,
    project_store::{LockGuard, atomic_write_bytes_checked, read_bytes_bounded},
};

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
    items: Vec<Item>,
    lock: Option<&'a LockGuard>,
}

#[derive(Debug)]
struct Item {
    range: Range<usize>,
    header_end: usize,
    content_end: usize,
    title_range: Range<usize>,
    task_index: Option<usize>,
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

    fn item(&self, task_id: &str, expected: &str) -> Result<&Item, InspectionError> {
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

    pub(crate) fn create(&self, title: &str, body: &str) -> Result<Task, InspectionError> {
        validate_title(title)?;
        validate_body(body)?;
        let task_id = self.new_id();
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
        let updated = self.commit(bytes)?;
        Ok(updated.task(&task_id)?.clone())
    }

    pub(crate) fn update(
        &self,
        task_id: &str,
        expected_task_revision: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<Task, InspectionError> {
        let item = self.item(task_id, expected_task_revision)?;
        if let Some(title) = title {
            validate_title(title)?;
        }
        if let Some(body) = body {
            validate_body(body)?;
        }
        let mut patches = Vec::with_capacity(2);
        if let Some(title) = title {
            patches.push((item.title_range.clone(), title.as_bytes().to_vec()));
        }
        if let Some(body) = body {
            let mut replacement = String::new();
            if item.header_end == item.content_end && !body.is_empty() {
                replacement.push_str(self.eol());
            }
            replacement.push_str(&encode_body(body, self.eol()));
            patches.push((item.header_end..item.range.end, replacement.into_bytes()));
        }
        let bytes = splice(&self.bytes, &patches);
        let updated = self.commit(bytes)?;
        Ok(updated.task(task_id)?.clone())
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
        let mut bytes = self.bytes.clone();
        bytes[item.range.start + 3] = if checked { b'x' } else { b' ' };
        let updated = self.commit(bytes)?;
        Ok(updated.task(task_id)?.clone())
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
            patches.push((
                item.content_end..item.content_end,
                format!(" <!-- cockpit-task: {id} -->").into_bytes(),
            ));
        }
        let assigned = patches.len() as u32;
        if assigned == 0 {
            if hash(&read_bytes(self.dir, &self.name)?) != self.doc_revision {
                return Err(conflict());
            }
            return Ok((0, self.doc_revision.clone()));
        }
        let updated = self.commit(splice(&self.bytes, &patches))?;
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

    fn commit(&self, bytes: Vec<u8>) -> Result<TaskDocument<'a>, InspectionError> {
        self.require_lock()?;
        if bytes.len() > MAX_TASK_DOCUMENT_BYTES {
            return Err(InspectionError::new(
                "tasks_full",
                "canonical task document exceeds 8 MiB",
            ));
        }
        let updated = parse_document(self.dir, self.name.clone(), bytes, self.lock)?;
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
        Ok(updated)
    }
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

fn encode_body(body: &str, eol: &str) -> String {
    let mut encoded = String::new();
    for line in body.lines() {
        encoded.push_str("  ");
        encoded.push_str(line);
        encoded.push_str(eol);
    }
    encoded
}

fn splice(bytes: &[u8], patches: &[(Range<usize>, Vec<u8>)]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut position = 0;
    for (range, replacement) in patches {
        output.extend_from_slice(&bytes[position..range.start]);
        output.extend_from_slice(replacement);
        position = range.end;
    }
    output.extend_from_slice(&bytes[position..]);
    output
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
        let title_start = line.start + 6;
        let raw_title = &text[title_start..line.content_end];
        let mut title_end = line.content_end;
        let task_index = if let Some((marker_start, task_id)) = marker(raw_title) {
            let title = raw_title[..marker_start].trim_end_matches([' ', '\t']);
            title_end = title_start + title.len();
            let task_index = tasks.len();
            tasks.push(Task {
                task_id,
                title: title.to_owned(),
                body: decode_body(&bytes[line.end..end]),
                checked: content[3] == b'x',
                line: (index + 1) as u32,
                task_revision: hash(&bytes[line.start..end]),
                diagnostic: None,
            });
            Some(task_index)
        } else {
            unidentified_items += 1;
            None
        };
        items.push(Item {
            range: line.start..end,
            header_end: line.end,
            content_end: line.content_end,
            title_range: title_start..title_end,
            task_index,
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
            .create("One", "body")
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
            .create("Accept me", "work")
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
            doc.create("No lock", "").unwrap_err().code,
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
}
