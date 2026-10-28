use cockpit_core::orchestration::herdr::OrchestrationHerdr;
use super::{args::{LaunchArgs, RunCommand, TaskCommand}, context::Context};

async fn launch(context: &Context, launch: &mut LaunchArgs) -> Result<(), CliError> {
    if [&launch.space, &launch.space_worktree].into_iter().flatten().any(|value| !canonical_uuid(value)) {
        let runtime = match context.adapter.runtime(&context.session).await {
            Ok(runtime) => runtime,
            Err(error) if error.code == "request_not_dispatched" => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for value in [&mut launch.space, &mut launch.space_worktree].into_iter().flatten() {
            if !canonical_uuid(value) { *value = space(value, &runtime.workspaces)?; }
        }
    }
    Ok(())
}

fn fence(value: &mut Option<String>, current: &str, name: &str) {
    if value.is_none() {
        eprintln!("Resolved {name}: {current}");
        *value = Some(current.to_owned());
    }
}
async fn reference_snapshot(context: &Context, root: Option<String>) -> Result<OrchestrationSnapshot, CliError> {
    let snapshot = context.snapshot(root).await?;
    if snapshot.board.is_some() { return Ok(snapshot); }
    let selected = snapshot.runs.iter()
        .filter(|run| run.kind == cockpit_protocol::orchestration::RunKind::Supervisor
            && run.parent_run_id.is_none() && run.stage != cockpit_protocol::orchestration::RunStage::Closed)
        .max_by(|a, b| (&a.updated_at, &a.run_id).cmp(&(&b.updated_at, &b.run_id)));
    if let Some(run) = selected { context.snapshot(Some(run.root_id.clone())).await } else { Ok(snapshot) }
}


pub(super) async fn task_command(context: &Context, root: &str, command: &mut TaskCommand) -> Result<(), CliError> {
    let snapshot = context.snapshot(Some(root.to_owned())).await?;
    let board = snapshot.board.as_ref().ok_or_else(|| CliError::new("root_not_found", "Task board does not exist"))?;
    let id = match &mut *command {
        TaskCommand::Show { task: Some(id), .. } | TaskCommand::Inputs { task: Some(id), .. }
        | TaskCommand::Schedule { task: id, .. } | TaskCommand::ScheduleCancel { task: id, .. }
        | TaskCommand::Restart { task: id, .. } | TaskCommand::Unassign { task: id, .. }
        | TaskCommand::Update { task: id, .. } | TaskCommand::ScopeUpdate { task: id, .. } => Some(id),
        TaskCommand::DependenciesSet { task, .. } | TaskCommand::StepAdd { task, .. }
        | TaskCommand::StepRename { task, .. } | TaskCommand::StepSetChecked { task, .. }
        | TaskCommand::StepMove { task, .. } | TaskCommand::StepRemove { task, .. } => Some(&mut task.task),
        _ => None,
    };
    if let Some(id) = id { *id = task(&snapshot, id)?; }
    match command {
        TaskCommand::Create { depends_on, follow_up_of, doc_revision, .. } => {
            for id in depends_on.iter_mut() { *id = task(&snapshot, id)?; }
            if let Some(id) = follow_up_of.as_mut() { *id = task(&snapshot, id)?; }
            if !depends_on.is_empty() || follow_up_of.is_some() { fence(doc_revision, &board.doc_revision, "doc revision"); }
        }
        TaskCommand::DependenciesSet { depends_on, .. } => {
            for id in depends_on { *id = task(&snapshot, id)?; }
        }
        TaskCommand::Update { task: id, revision, .. } if revision.is_none() => {
            let view = board.tasks.iter().find(|v| v.task.task_id == *id)
                .ok_or_else(|| CliError::new("task_not_found", "Task is not in the selected root"))?;
            fence(revision, &view.task.task_revision, "task revision");
        }
        TaskCommand::ScopeUpdate { task: id, run: target, revision, doc_revision, .. } => {
            let view = board.tasks.iter().find(|v| v.task.task_id == *id);
            if revision.is_none() {
                let view = view.ok_or_else(|| CliError::new("task_not_found", "Task is not in the selected root"))?;
                fence(revision, &view.task.task_revision, "task revision");
            }
            fence(doc_revision, &board.doc_revision, "doc revision");
            if let Some(id) = target.as_mut() { *id = run(&snapshot, id)?; }
            else {
                let view = view.ok_or_else(|| CliError::new("task_not_found", "Task is not in the selected root"))?;
                if view.lane != cockpit_protocol::orchestration::TaskLane::Working {
                    return Err(CliError::new("reference_not_found", "Task has no current Working run"));
                }
                *target = Some(view.current_run_id.clone().ok_or_else(|| CliError::new("reference_not_found", "Task has no current Working run"))?);
                eprintln!("Resolved run: {}", target.as_deref().unwrap());
            }
        }
        TaskCommand::Schedule { launch: target, .. } => launch(context, target).await?,
        _ => {}
    }
    Ok(())
}

pub(super) async fn run_command(context: &Context, root: Option<String>, command: &mut RunCommand) -> Result<(), CliError> {
    if let RunCommand::Start { space, space_worktree, .. } = &mut *command {
        if [space.as_ref(), space_worktree.as_ref()].into_iter().flatten().any(|value| !canonical_uuid(value)) {
            let runtime = match context.adapter.runtime(&context.session).await {
                Ok(runtime) => runtime,
                Err(error) if error.code == "request_not_dispatched" => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            for value in [space, space_worktree].into_iter().flatten() {
                if !canonical_uuid(value) { *value = self::space(value, &runtime.workspaces)?; }
            }
        }
        return Ok(());
    }
    if let RunCommand::WorkerStart(args) = &mut *command {
        let snapshot = reference_snapshot(context, root).await?;
        args.task = task(&snapshot, &args.task)?;
        launch(context, &mut args.launch).await?;
        return Ok(());
    }
    let id = match &mut *command {
        RunCommand::Resume { run, .. } | RunCommand::WorkerReplace { run, .. }
        | RunCommand::Accept { run, .. } | RunCommand::SendBack { run, .. }
        | RunCommand::Cancel { run } | RunCommand::Reconcile { run, .. }
        | RunCommand::Message { run, .. } | RunCommand::Annotate { run, .. }
        | RunCommand::Show { run: Some(run), .. } => Some(run),
        RunCommand::Report { to: Some(run), .. } => Some(run),
        _ => None,
    };
    if let Some(id) = id {
        let snapshot = reference_snapshot(context, root).await?;
        *id = run(&snapshot, id)?;
        if let RunCommand::Accept { run, task_revision } = command && task_revision.is_none() {
            let task_id = snapshot.runs.iter().find(|item| item.run_id == *run).and_then(|r| r.task_id.as_ref())
                .ok_or_else(|| CliError::new("reference_not_found", "Run has no task"))?;
            let view = snapshot.board.as_ref().and_then(|b| b.tasks.iter().find(|v| v.task.task_id == *task_id))
                .ok_or_else(|| CliError::new("reference_not_found", "Run task is not in the selected board"))?;
            fence(task_revision, &view.task.task_revision, "task revision");
        }
    }
    Ok(())
}

use cockpit_protocol::orchestration::OrchestrationSnapshot;
use super::CliError;

fn canonical_uuid(reference: &str) -> bool {
    uuid::Uuid::parse_str(reference).is_ok_and(|id| id.hyphenated().to_string().eq_ignore_ascii_case(reference))
}

fn choose<'a>(reference: &str, candidates: impl IntoIterator<Item = (&'a str, &'a str)>) -> Result<String, CliError> {
    let items: Vec<_> = candidates.into_iter().collect();
    if let Some((id, _)) = items.iter().find(|(id, _)| *id == reference) { return Ok((*id).to_owned()); }
    let needle = reference.to_lowercase();
    let prefix = reference.len() >= 4 && reference.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-');
    let ids: Vec<_> = items.iter().copied().filter(|(id, _)| prefix && id.to_ascii_lowercase().starts_with(&needle)).collect();
    let matches: Vec<_> = if ids.is_empty() {
        items.into_iter().filter(|(_, label)| !needle.is_empty() && label.to_lowercase().contains(&needle)).collect()
    } else { ids };
    match matches.as_slice() {
        [(id, _)] => Ok((*id).to_owned()),
        [] => Err(CliError::new("reference_not_found", format!("No match for {reference:?}"))),
        _ => Err(CliError::new("ambiguous_reference", format!("Ambiguous {reference:?}: {}", matches.iter().take(5).map(|(id, label)| format!("{id}  {label}")).collect::<Vec<_>>().join(", ")))),
    }
}

pub(super) fn task(snapshot: &OrchestrationSnapshot, reference: &str) -> Result<String, CliError> {
    if canonical_uuid(reference) { return Ok(reference.to_owned()); }
    choose(reference, snapshot.board.iter().flat_map(|board| board.tasks.iter()).map(|view| (view.task.task_id.as_str(), view.task.title.as_str())))
}

pub(super) fn run(snapshot: &OrchestrationSnapshot, reference: &str) -> Result<String, CliError> {
    if canonical_uuid(reference) { return Ok(reference.to_owned()); }
    if let Some(run) = snapshot.runs.iter().find(|run| run.run_id == reference) { return Ok(run.run_id.clone()); }
    let prefix = reference.len() >= 4 && reference.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-');
    if prefix {
        let matches: Vec<_> = snapshot.runs.iter().filter(|run| run.run_id.to_ascii_lowercase().starts_with(&reference.to_ascii_lowercase())).collect();
        if matches.len() == 1 { return Ok(matches[0].run_id.clone()); }
        if matches.len() > 1 { return Err(CliError::new("ambiguous_reference", format!("Ambiguous {reference:?}: {}", matches.iter().take(5).map(|run| format!("{}  {}", run.run_id, run.label)).collect::<Vec<_>>().join(", ")))); }
    }
    let id = task(snapshot, reference)?;
    snapshot.board.iter().flat_map(|b| &b.tasks).find(|v| v.task.task_id == id).and_then(|v| v.current_run_id.clone())
        .ok_or_else(|| CliError::new("reference_not_found", format!("Task {reference:?} has no current run")))
}

pub(super) fn space(reference: &str, workspaces: &[cockpit_core::orchestration::herdr::RuntimeWorkspace]) -> Result<String, CliError> {
    // Space labels are exact, case-insensitive; unlike task titles they are not substrings.
    if let Some(workspace) = workspaces.iter().find(|w| w.workspace_id == reference) { return Ok(workspace.workspace_id.clone()); }
    let matches: Vec<_> = workspaces.iter().filter(|w| w.label.eq_ignore_ascii_case(reference)).collect();
    match matches.as_slice() {
        [workspace] => Ok(workspace.workspace_id.clone()),
        [] => Err(CliError::new("reference_not_found", format!("No Space matches {reference:?}"))),
        _ => Err(CliError::new("ambiguous_reference", format!("Ambiguous Space {reference:?}: {}", matches.iter().take(5).map(|w| format!("{}  {}", w.workspace_id, w.label)).collect::<Vec<_>>().join(", ")))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifiers_titles_and_ambiguity() {
        let entries = [("abcd1234-0000", "Build widgets"), ("abcd9876-0000", "Fix widgets")];
        assert_eq!(choose("abcd1234-0000", entries).unwrap(), "abcd1234-0000");
        assert_eq!(choose("abcd1", entries).unwrap(), "abcd1234-0000");
        assert_eq!(choose("BUILD", entries).unwrap(), "abcd1234-0000");
        assert_eq!(choose("abcd", entries).unwrap_err().code, "ambiguous_reference");
        assert_eq!(choose("zzz", entries).unwrap_err().code, "reference_not_found");
    }
    #[test]
    fn canonical_uuid_is_distinct_from_prefix_and_compact_uuid() {
        assert!(canonical_uuid("00000000-0000-4000-8000-000000000001"));
        assert!(!canonical_uuid("00000000"));
        assert!(!canonical_uuid("00000000000040008000000000000001"));
    }
    #[test]
    fn omitted_fence_is_filled_without_overwriting_explicit() {
        let mut missing = None;
        fence(&mut missing, "current", "revision");
        assert_eq!(missing.as_deref(), Some("current"));
        let mut supplied = Some("explicit".to_string());
        fence(&mut supplied, "current", "revision");
        assert_eq!(supplied.as_deref(), Some("explicit"));
    }

    #[test]
    fn spaces_resolve_ids_labels_and_ambiguity() {
        use cockpit_core::orchestration::herdr::RuntimeWorkspace;
        let spaces = vec![
            RuntimeWorkspace { workspace_id: "one".into(), label: "Project".into(), cwd: "/one".into() },
            RuntimeWorkspace { workspace_id: "two".into(), label: "Other".into(), cwd: "/two".into() },
        ];
        assert_eq!(space("one", &spaces).unwrap(), "one");
        assert_eq!(space("pROJECT", &spaces).unwrap(), "one");
        assert_eq!(space("missing", &spaces).unwrap_err().code, "reference_not_found");
        let mut duplicate = spaces;
        duplicate[1].label = "project".into();
        assert_eq!(space("project", &duplicate).unwrap_err().code, "ambiguous_reference");
    }
}
