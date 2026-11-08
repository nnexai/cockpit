use std::{collections::HashSet, io::{ErrorKind, Read}, path::{Component, Path}};

use cap_fs_ext::{OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::orchestration::{ActorRef, InboxArtifact, InboxArtifactKind, Message, MessageKind, OrchestrationArtifactMedia, ReportKind, Run, RunKind, SourceOutput, WorkerCheckpoint};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::InspectionError;

fn kind(path: &str) -> InboxArtifactKind {
    match Path::new(path).extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "png" | "jpg" | "jpeg" | "webp" => InboxArtifactKind::Image,
        "html" | "htm" => InboxArtifactKind::Html,
        "md" => InboxArtifactKind::Markdown,
        _ => InboxArtifactKind::Other,
    }
}

fn add_artifact(result: &mut Vec<InboxArtifact>, seen: &mut HashSet<String>, path: &str, recorded: bool) {
    if !seen.insert(path.to_owned()) { return; }
    result.push(InboxArtifact {
        path: path.to_owned(),
        name: Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path).to_owned(),
        kind: kind(path), recorded,
    });
}

/// The same list is used for display and authorization; recorded paths take precedence.
pub(super) fn artifacts_for_run(run: &Run, messages: &[Message]) -> Vec<InboxArtifact> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    if let Some(SourceOutput::NonCode { artifacts, .. }) = &run.source_output {
        for path in artifacts { add_artifact(&mut result, &mut seen, path, true); }
    }
    if run.kind == RunKind::Worker {
        for message in messages.iter().filter(|message| !message.stale
            && message.kind == MessageKind::Report
            && matches!(&message.from, ActorRef::Run { run_id } if run_id == &run.run_id)) {
            if let Some(checkpoint) = message.report.as_ref()
                .filter(|report| report.kind == ReportKind::Progress)
                .and_then(|report| WorkerCheckpoint::decode(report.plan.as_deref())) {
                for path in checkpoint.artifacts { add_artifact(&mut result, &mut seen, &path, true); }
            }
        }
    }
    if let Some(report) = &run.result {
        // Paths commonly appear in prose, Markdown links, or code spans.
        for token in report.summary.split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '<' | '>')) {
            if token.starts_with('/') {
                let path = token.trim_end_matches([',', '.', ';', ':', '!', '?']);
                if !matches!(kind(path), InboxArtifactKind::Other) {
                    add_artifact(&mut result, &mut seen, path, false);
                }
            }
        }
    }
    result
}

fn failure(code: &'static str, message: impl std::fmt::Display) -> InspectionError {
    InspectionError::new(code, message.to_string())
}

fn open_error(error: std::io::Error) -> InspectionError {
    if error.kind() == ErrorKind::NotFound {
        failure("artifact_not_found", "Artifact not found")
    } else {
        failure("artifact_not_allowed", format!("Cannot read artifact: {error}"))
    }
}

fn read_authorized(run: &Run, messages: &[Message], path: &str) -> Result<OrchestrationArtifactMedia, InspectionError> {
    let path_obj = Path::new(path);
    if !path_obj.is_absolute() || path_obj.components().any(|part| matches!(part, Component::ParentDir)) {
        return Err(failure("artifact_not_allowed", "Artifact path is not allowed"));
    }
    let artifact = artifacts_for_run(run, messages).into_iter().find(|item| item.path == path)
        .ok_or_else(|| failure("artifact_not_allowed", "Artifact path is not allowed"))?;
    let limit = match artifact.kind {
        InboxArtifactKind::Image => 8 * 1024 * 1024,
        InboxArtifactKind::Html => 1024 * 1024,
        InboxArtifactKind::Markdown => 512 * 1024,
        InboxArtifactKind::Other => return Err(failure("artifact_unsupported", "Unsupported artifact type")),
    };
    // Open every component without following symlinks, including parent directories.
    let mut dir = Dir::open_ambient_dir(Path::new(std::path::MAIN_SEPARATOR_STR), cap_std::ambient_authority()).map_err(open_error)?;
    let components: Vec<_> = path_obj.components().filter_map(|component| match component {
        Component::Normal(name) => Some(name), _ => None,
    }).collect();
    let (name, parents) = components.split_last().ok_or_else(|| failure("artifact_not_allowed", "Artifact path is not allowed"))?;
    let mut options = OpenOptions::new();
    options.read(true).follow(cap_fs_ext::FollowSymlinks::No).nonblock(true);
    for parent in parents {
        let file = dir.open_with(parent, &options).map_err(open_error)?;
        if !file.metadata().map_err(open_error)?.is_dir() {
            return Err(failure("artifact_not_allowed", "Unsafe artifact directory"));
        }
        dir = Dir::from_std_file(file.into_std());
    }
    let file = dir.open_with(name, &options).map_err(open_error)?;
    let metadata = file.metadata().map_err(open_error)?;
    if !metadata.is_file() { return Err(failure("artifact_not_allowed", "Artifact is not a regular file")); }
    if metadata.len() > limit { return Err(failure("artifact_too_large", "Artifact exceeds the byte limit")); }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1).read_to_end(&mut bytes).map_err(open_error)?;
    if bytes.len() as u64 > limit { return Err(failure("artifact_too_large", "Artifact exceeds the byte limit")); }
    let mime_type = match artifact.kind {
        InboxArtifactKind::Image => {
            let ext = path_obj.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            match ext.as_str() {
                "png" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => "image/png",
                "jpg" | "jpeg" if bytes.starts_with(&[0xff, 0xd8, 0xff]) => "image/jpeg",
                "webp" if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" => "image/webp",
                _ => return Err(failure("artifact_unsupported", "Artifact content does not match its image type")),
            }
        }
        InboxArtifactKind::Html | InboxArtifactKind::Markdown => {
            std::str::from_utf8(&bytes).map_err(|_| failure("artifact_unsupported", "Artifact is not UTF-8"))?;
            if matches!(artifact.kind, InboxArtifactKind::Html) { "text/html" } else { "text/markdown" }
        }
        InboxArtifactKind::Other => unreachable!(),
    };
    Ok(OrchestrationArtifactMedia { path: path.to_owned(), kind: artifact.kind, mime_type: mime_type.to_owned(), data_base64: BASE64.encode(bytes) })
}

pub(super) fn read(run: &Run, messages: &[Message], path: &str) -> Result<OrchestrationArtifactMedia, InspectionError> {
    read_authorized(run, messages, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::{new_run, RunLineage};
    use cockpit_protocol::orchestration::{Report, ReportKind, RunKind};
    #[test]
    fn authorization_and_png_sniff() {
        let root = std::env::temp_dir().join(format!("cockpit-artifact-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let png = root.join("image.png");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\ncontent").unwrap();
        let path = png.to_str().unwrap();
        let mut run = new_run("run", "session", RunKind::Supervisor, "run".into(), "run", RunLineage::default(), 1);
        run.result = Some(Report { message_id: "m".into(), kind: ReportKind::Result, outcome: None, summary: format!("See `{path}`"), plan: None, source_output: None, card: None, at: "now".into() });
        let service = crate::orchestration::OrchestrationService::open(
            &cockpit_protocol::projects::ProjectConfiguration::for_tests(&root),
        ).unwrap();
        {
            let locked = service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs.push(run.clone());
            locked.save(&mut state).unwrap();
        }
        let request = |session: &str, run_id: &str| cockpit_protocol::orchestration::OrchestrationArtifactRead {
            session_id: session.into(), run_id: run_id.into(), path: path.into(),
        };
        assert_eq!(service.artifact_read(request("session", "run")).unwrap().mime_type, "image/png");
        assert_eq!(service.artifact_read(request("other-session", "run")).unwrap_err().code, "artifact_not_found");
        assert_eq!(service.artifact_read(request("session", "other-run")).unwrap_err().code, "artifact_not_found");
        assert_eq!(read(&run, &[], path).unwrap().mime_type, "image/png");
        assert_eq!(read(&run, &[], root.join("other.png").to_str().unwrap()).unwrap_err().code, "artifact_not_allowed");
        let traversal = format!("{}/../{}/image.png", root.display(), root.file_name().unwrap().to_string_lossy());
        run.source_output = Some(SourceOutput::NonCode { result_message_id: "m".into(), task_revision: "r".into(), artifacts: vec![traversal.clone()], focused_checks: vec![] });
        assert_eq!(read(&run, &[], &traversal).unwrap_err().code, "artifact_not_allowed");
        std::fs::write(&png, b"not a png").unwrap();
        assert_eq!(read(&run, &[], path).unwrap_err().code, "artifact_unsupported");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checkpoint_artifacts_are_scoped_to_non_stale_worker_reports() {
        use cockpit_protocol::orchestration::{DeliveryStage, WorkerCheckpointKind};
        let root = std::env::temp_dir().join(format!("cockpit-checkpoint-artifact-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("preview.html");
        let stale_path = root.join("stale.html");
        let other_path = root.join("other.md");
        let symlink = root.join("link.html");
        let large = root.join("large.md");
        std::fs::write(&path, "<p>checkpoint</p>").unwrap();
        std::fs::write(&stale_path, "stale").unwrap();
        std::fs::write(&other_path, "other").unwrap();
        std::fs::write(&large, vec![b'x'; 512 * 1024 + 1]).unwrap();
        std::os::unix::fs::symlink(&path, &symlink).unwrap();
        let path = path.to_str().unwrap().to_owned();
        let stale_path = stale_path.to_str().unwrap().to_owned();
        let other_path = other_path.to_str().unwrap().to_owned();
        let symlink = symlink.to_str().unwrap().to_owned();
        let large = large.to_str().unwrap().to_owned();
        let run = new_run("worker", "session", RunKind::Worker, "worker".into(), "worker", RunLineage::default(), 1);
        let checkpoint = |id: &str, sender: &str, stale: bool, paths: Vec<String>| Message {
            message_id: id.into(), to_run_id: "root".into(), seq: 1,
            from: ActorRef::Run { run_id: sender.into() }, kind: MessageKind::Report,
            text: "progress".into(), images: vec![], in_reply_to: None,
            report: Some(Report { message_id: id.into(), kind: ReportKind::Progress, outcome: None,
                summary: "progress".into(), plan: Some(WorkerCheckpoint {
                    kind: WorkerCheckpointKind::Interim, artifacts: paths,
                }.encode()), source_output: None, card: None, at: "now".into() }),
            thread_id: None, handoff: None, stale, escalated_from: None,
            stage: DeliveryStage::Stored, created_at: "now".into(), acked_at: None,
        };
        let mut messages = vec![
            checkpoint("current", "worker", false, vec![path.clone(), symlink.clone(), large.clone()]),
            checkpoint("duplicate", "worker", false, vec![path.clone()]),
            checkpoint("stale", "worker", true, vec![stale_path.clone()]),
            checkpoint("other", "other-worker", false, vec![other_path.clone()]),
        ];
        messages[1].report.as_mut().unwrap().plan = Some(WorkerCheckpoint {
            kind: WorkerCheckpointKind::Waiting, artifacts: vec![path.clone()],
        }.encode());
        let mut resumed = checkpoint("resumed", "worker", false, vec![path.clone()]);
        resumed.report.as_mut().unwrap().plan = Some(WorkerCheckpoint {
            kind: WorkerCheckpointKind::Resumed, artifacts: vec![path.clone()],
        }.encode());
        messages.push(resumed);
        let mut wrong_kind = checkpoint("not-progress", "worker", false, vec![other_path.clone()]);
        wrong_kind.report.as_mut().unwrap().kind = ReportKind::Result;
        messages.push(wrong_kind);
        let listed = artifacts_for_run(&run, &messages);
        assert_eq!(listed.len(), 3);
        assert!(listed.iter().any(|artifact| artifact.path == path && artifact.recorded));
        assert_eq!(read(&run, &messages, &path).unwrap().mime_type, "text/html");
        assert_eq!(read(&run, &messages, &stale_path).unwrap_err().code, "artifact_not_allowed");
        assert_eq!(read(&run, &messages, &other_path).unwrap_err().code, "artifact_not_allowed");
        assert_eq!(read(&run, &messages, &symlink).unwrap_err().code, "artifact_not_allowed");
        assert_eq!(read(&run, &messages, &large).unwrap_err().code, "artifact_too_large");

        let service = crate::orchestration::OrchestrationService::open(
            &cockpit_protocol::projects::ProjectConfiguration::for_tests(&root),
        ).unwrap();
        {
            let locked = service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs.push(run);
            state.messages.append(&mut messages);
            locked.save(&mut state).unwrap();
        }
        let request = |session: &str, run_id: &str, artifact: &str| cockpit_protocol::orchestration::OrchestrationArtifactRead {
            session_id: session.into(), run_id: run_id.into(), path: artifact.into(),
        };
        assert_eq!(service.artifact_read(request("session", "worker", &path)).unwrap().mime_type, "text/html");
        assert_eq!(service.artifact_read(request("other-session", "worker", &path)).unwrap_err().code, "artifact_not_found");
        assert_eq!(service.artifact_read(request("session", "other-worker", &path)).unwrap_err().code, "artifact_not_found");
        assert_eq!(service.artifact_read(request("session", "worker", &stale_path)).unwrap_err().code, "artifact_not_allowed");
        std::fs::remove_dir_all(root).unwrap();
    }
}
