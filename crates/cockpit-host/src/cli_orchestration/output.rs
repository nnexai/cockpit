use cockpit_protocol::orchestration::{OrchestrationSnapshot, Run, RuntimeObservation, TaskBoard};
use serde::Serialize;

use super::CliError;

pub(super) fn emit(value: &impl Serialize, json: bool) -> Result<(), CliError> {
    let output = if json {
        serde_json::to_string(value)
    } else {
        serde_json::to_string_pretty(value)
    }
    .map_err(|error| CliError::new("orchestration_output", error.to_string()))?;
    println!("{output}");
    Ok(())
}

/// Stable recovery location for this exact issued run/attachment/credential.
/// Derivation is read-only so context can expose it even before the first batch.
pub(super) fn capture_directory(auth: &cockpit_core::orchestration::JobAuth, run_id: &str) -> Result<std::path::PathBuf, CliError> {
    use sha2::{Digest, Sha256};
    let credential = std::env::var_os("COCKPIT_JOB_CREDENTIAL_FILE")
        .ok_or_else(|| CliError::new("credential_required", "Inbox capture requires the issued credential file"))?;
    let credential = std::path::PathBuf::from(credential);
    let credential = if credential.is_absolute() { credential } else {
        std::env::current_dir().map_err(|error| CliError::new("inbox_capture", error.to_string()))?.join(credential)
    };
    let parent = credential.parent().ok_or_else(|| CliError::new("inbox_capture", "Credential has no private parent"))?;
    let binding = serde_json::to_vec(&(run_id, &auth.attachment_id, auth.attachment_epoch, &auth.credential_id))
        .map_err(|error| CliError::new("inbox_capture", error.to_string()))?;
    Ok(parent.join(format!("inbox-capture-{:x}", Sha256::digest(binding))))
}

pub(super) fn default_capture_file(auth: &cockpit_core::orchestration::JobAuth, run_id: &str) -> Result<std::path::PathBuf, CliError> {
    use std::{fs::{File, OpenOptions}, os::unix::fs::{MetadataExt, OpenOptionsExt}};
    use nix::{errno::Errno, fcntl::{OFlag, openat}, sys::stat::{Mode, mkdirat}, unistd::Uid};
    let path = capture_directory(auth, run_id)?;
    let parent = path.parent().expect("capture directory has credential parent");
    let name = path.file_name().expect("capture directory has generated name");
    let failed = |error: &dyn std::fmt::Display| CliError::new("inbox_capture", error.to_string());
    let directory = OpenOptions::new().read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(parent).map_err(|error| failed(&error))?;
    let private = |metadata: &std::fs::Metadata| metadata.uid() == Uid::current().as_raw() && metadata.mode() & 0o077 == 0;
    if !private(&directory.metadata().map_err(|error| failed(&error))?) {
        return Err(CliError::new("inbox_capture", "Credential capture parent must be owner-only and owned by this user"));
    }
    match mkdirat(&directory, name, Mode::from_bits_truncate(0o700)) {
        Ok(()) | Err(Errno::EEXIST) => {},
        Err(error) => return Err(failed(&error)),
    }
    let capture_directory = File::from(openat(&directory, name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC, Mode::empty())
        .map_err(|error| failed(&error))?);
    if !private(&capture_directory.metadata().map_err(|error| failed(&error))?) {
        return Err(CliError::new("inbox_capture", "Inbox capture directory must be owner-only and owned by this user"));
    }
    directory.sync_all().map_err(|error| failed(&error))?;
    Ok(path.join(format!("{}.json", uuid::Uuid::new_v4())))
}

/// Capture one exact inbox batch without clobbering another undelivered batch.
/// Directory-relative no-follow operations preserve the verified private parent.
pub(super) fn capture(path: &std::path::Path, value: &impl Serialize) -> Result<(), CliError> {
    use std::{fs::{File, OpenOptions}, io::Write, os::unix::fs::{MetadataExt, OpenOptionsExt}};
    use nix::{fcntl::{AtFlags, OFlag, openat}, sys::stat::Mode,
        unistd::{Uid, UnlinkatFlags, linkat, unlinkat}};
    let failed = |error: &dyn std::fmt::Display| CliError::new("inbox_capture", error.to_string());
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| CliError::usage("--capture-file requires an existing private parent directory"))?;
    let name = path.file_name().ok_or_else(|| CliError::usage("--capture-file requires a filename"))?;
    let directory = OpenOptions::new().read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(parent).map_err(|error| failed(&error))?;
    let metadata = directory.metadata().map_err(|error| failed(&error))?;
    if metadata.uid() != Uid::current().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(CliError::new("inbox_capture", "Capture parent must be owner-only and owned by this user"));
    }
    let bytes = serde_json::to_vec(value).map_err(|error| failed(&error))?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(CliError::new("inbox_capture", "Inbox capture exceeds 4 MiB; no messages acknowledged"));
    }
    let temporary = format!(".capture-{}.tmp", uuid::Uuid::new_v4());
    let mut file = File::from(openat(&directory, temporary.as_str(),
        OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::from_bits_truncate(0o600)).map_err(|error| failed(&error))?);
    let captured = (|| {
        file.write_all(&bytes).map_err(|error| failed(&error))?;
        file.sync_all().map_err(|error| failed(&error))?;
        linkat(&directory, temporary.as_str(), &directory, name, AtFlags::empty())
            .map_err(|error| failed(&error))?;
        directory.sync_all().map_err(|error| failed(&error))
    })();
    let removed = unlinkat(&directory, temporary.as_str(), UnlinkatFlags::NoRemoveDir)
        .map_err(|error| failed(&error));
    captured?;
    removed?;
    directory.sync_all().map_err(|error| failed(&error))
}

/// Render only core-validated accepted references; reported artifacts remain
/// untrusted opaque strings, never inferred paths, hashes or publication proof.
pub(super) fn task_inputs(
    root_id: &str, board: &TaskBoard, task: &cockpit_protocol::orchestration::Task,
    accepted: &[&Run],
) -> serde_json::Value {
    let dependencies: Vec<_> = task.depends_on.iter().map(|id| {
        let prerequisite = board.tasks.iter().find(|view| &view.task.task_id == id).map(|view| &view.task);
        let run = accepted.iter().find(|run| run.root_id == root_id && run.task_id.as_ref() == Some(id));
        let mut diagnostics: Vec<&str> = prerequisite.into_iter().flat_map(|task| [
            task.diagnostic.as_deref(), task.relations_diagnostic.as_deref(),
            task.description_diagnostic.as_deref(), task.steps_diagnostic.as_deref(),
        ]).flatten().collect();
        if prerequisite.is_none() {
            diagnostics.push("Prerequisite is missing from the current canonical task document");
        } else if run.is_none() {
            diagnostics.push("No valid accepted Result/source reference matches the current prerequisite scope");
        }
        let accepted_run = run.map(|run| serde_json::json!({
            "run_id": run.run_id, "attempt": run.attempt,
            "result": run.result.as_ref().map(|result| serde_json::json!({
                "message_id": result.message_id, "kind": result.kind, "outcome": result.outcome,
            })),
            "source_output": run.source_output, "integration": run.integration,
            "setup": run.setup.as_ref().map(|setup| serde_json::json!({
                "checkout_path": setup.checkout_path, "base_commit": setup.base_commit,
                "project_workspace_id": setup.project_workspace_id,
            })),
            "checkout": run.checkout, "project": run.project,
        }));
        serde_json::json!({
            "task_id": id,
            "title": prerequisite.map(|task| &task.title),
            "checked": prerequisite.map(|task| task.checked),
            "task_revision": prerequisite.map(|task| &task.task_revision),
            "diagnostics": diagnostics,
            "accepted_run": accepted_run,
        })
    }).collect();
    serde_json::json!({
        "root_id": root_id, "task_id": task.task_id, "task_revision": task.task_revision,
        "doc_revision": board.doc_revision, "dependencies": dependencies,
        "dependency_state": board.tasks.iter().find(|view| view.task.task_id == task.task_id)
            .map(|view| &view.dependencies),
    })
}

pub(super) fn emit_task_board(board: TaskBoard, json: bool) -> Result<(), CliError> {
    if json {
        return emit(&board, true);
    }
    println!("{} (revision {})", board.path, board.doc_revision);
    for view in board.tasks {
        println!(
            "{}\t[{}] {}\t{:?}\t{}\tdependencies={:?}\tsteps={}",
            view.task.task_id,
            if view.task.checked { 'x' } else { ' ' },
            view.task.title,
            view.lane,
            view.task.task_revision,
            view.dependencies.state,
            view.task
                .step_progress
                .as_ref()
                .map(|progress| format!("{}/{}", progress.done, progress.total))
                .unwrap_or_else(|| "unavailable".into()),
        );
    }
    Ok(())
}

pub(super) fn emit_runs(
    snapshot: &OrchestrationSnapshot,
    tree: bool,
    json: bool,
) -> Result<(), CliError> {
    if json {
        return emit(&snapshot, true);
    }
    for run in &snapshot.runs {
        let depth = if tree {
            run_depth(run, &snapshot.runs)
        } else {
            0
        };
        let observation = match &snapshot.runtime {
            RuntimeObservation::Fresh { runs, .. } => {
                runs.iter().find(|item| item.run_id == run.run_id)
            }
            RuntimeObservation::Unavailable { .. } => None,
        };
        println!(
            "{}{}\t{}\t{:?}\t{}",
            "  ".repeat(depth),
            run.run_id,
            run.label,
            run.stage,
            observation
                .and_then(|item| item.agent_status.as_deref())
                .unwrap_or("unobserved")
        );
    }
    Ok(())
}

pub(super) fn run_depth(run: &Run, runs: &[Run]) -> usize {
    let mut depth = 0;
    let mut parent = run.parent_run_id.as_deref();
    while let Some(id) = parent {
        let Some(ancestor) = runs.iter().find(|item| item.run_id == id) else {
            break;
        };
        depth += 1;
        if depth >= runs.len() {
            break;
        }
        parent = ancestor.parent_run_id.as_deref();
    }
    depth
}
