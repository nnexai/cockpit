use std::path::Path;

use cockpit_core::{orchestration::{herdr::{OrchestrationHerdr, RuntimeView}, Actor}, repositories::RepositoryCatalog};
use cockpit_protocol::{orchestration::{OrchestrationSnapshot, OrchestrationSnapshotRequest, ProjectSource, RunKind, RuntimeObservation, TaskLane}, projects::RepositoryListResponse};
use serde::Serialize;

use super::{args::OrchestrationArgs, context::Context, output::emit, CliError};

#[derive(Serialize)]
struct WhereOutput {
    projects: Vec<Project>,
    general_tasks: Vec<Task>,
    general_spaces: Vec<Space>,
    catalog: Vec<CatalogEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    catalog_diagnostics: Vec<cockpit_protocol::projects::ProjectDiagnostic>,
}

#[derive(Serialize)]
struct Project {
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    kind: &'static str,
    path: String,
    spaces: Vec<Space>,
    tasks: Vec<Task>,
}

#[derive(Serialize)]
struct Space { workspace_id: String, label: String, cwd: String, agents: Vec<Agent> }
#[derive(Serialize)]
struct Agent {
    pane_id: String,
    name: String,
    kind: String,
    status: String,
    managed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<AgentTask>,
}
#[derive(Clone, Serialize)]
struct AgentTask { task_id: String, title: String, lane: TaskLane }
#[derive(Clone, Serialize)]
struct Task {
    task_id: String,
    title: String,
    lane: TaskLane,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
}
#[derive(Serialize)]
struct CatalogEntry { id: String, label: String, path: String }

/// Entry point for the top-level `where` command. Uses the same endpoint and
/// configuration options as other orchestration commands.
pub(super) async fn execute(args: &OrchestrationArgs) -> Result<(), CliError> {
    let context = Context::open(args, false).await?;
    let root = match &context.actor {
        Some(Actor::Agent(caller)) => {
            let run = context.authenticated_run(caller)?;
            // A credential identifies the root whose task board is requested.
            if args.root.as_ref().is_some_and(|root| root != &run.root_id) {
                return Err(CliError::new("actor_forbidden", "credential belongs to another root"));
            }
            Some(run.root_id)
        }
        _ => args.root.clone(),
    };
    let mut request = OrchestrationSnapshotRequest { session_id: context.session.clone(), root_id: root };
    let mut snapshot = context.service.snapshot(context.adapter.as_ref(), &request).await?;
    if request.root_id.is_none() {
        if let Some(liaison) = snapshot.runs.iter()
            .filter(|run| run.kind == RunKind::Supervisor && run.parent_run_id.is_none()
                && run.stage != cockpit_protocol::orchestration::RunStage::Closed)
            .max_by(|a, b| (&a.updated_at, &a.run_id).cmp(&(&b.updated_at, &b.run_id)))
        {
            request.root_id = Some(liaison.root_id.clone());
            snapshot = context.service.snapshot(context.adapter.as_ref(), &request).await?;
        } else {
            snapshot.board = None;
        }
    }
    let endpoint = match &snapshot.runtime {
        RuntimeObservation::Fresh { endpoint_identity, .. } => endpoint_identity,
        RuntimeObservation::Unavailable { error } => return Err(CliError::new(&error.code, error.message.clone())),
    };
    let runtime = context.adapter.runtime(&context.session).await?;
    if &runtime.endpoint_identity != endpoint {
        return Err(CliError::new("runtime_changed", "Herdr endpoint changed while reading the workspace"));
    }
    let catalog = RepositoryCatalog::new(context.configuration.clone()).list().await?;
    context.check_caller().await?;
    let output = build_output(&snapshot, &runtime, catalog).await?;
    if args.json {
        emit(&output, true)
    } else {
        print_tree(&output);
        Ok(())
    }
}

async fn build_output(snapshot: &OrchestrationSnapshot, runtime: &RuntimeView, catalog: RepositoryListResponse) -> Result<WhereOutput, CliError> {
    let mut output = WhereOutput {
        projects: Vec::new(), general_tasks: Vec::new(), general_spaces: Vec::new(),
        catalog: catalog.repositories.iter().map(|repo| CatalogEntry {
            id: repo.repository_id.clone(), label: repo.name.clone(), path: repo.checkout_path.clone(),
        }).collect(),
        catalog_diagnostics: catalog.diagnostics,
    };
    let board = snapshot.board.as_ref();
    // Private general-work attempts share the liaison's dedicated directory project.
    let liaison_key = snapshot.runs.iter()
        .find(|run| run.kind == RunKind::Supervisor && board.is_some_and(|board| board.root_id == run.run_id))
        .and_then(|run| run.project.as_ref()).map(|project| project.key.clone());
    for workspace in &runtime.workspaces {
        // The liaison's own Space is not a project target.
        if snapshot.runs.iter().any(|run| run.kind == RunKind::Supervisor && run.stage != cockpit_protocol::orchestration::RunStage::Closed && run.attachment.as_ref()
            .and_then(|attachment| attachment.location.as_ref())
            .is_some_and(|location| location.workspace_id == workspace.workspace_id)) { continue; }
        let project_run = snapshot.runs.iter().find(|run| run.project.is_some() && run.attachment.as_ref()
            .and_then(|attachment| attachment.location.as_ref())
            .is_some_and(|location| location.workspace_id == workspace.workspace_id));
        let (key, kind, path, label) = if let Some(run) = project_run {
            let project = run.project.as_ref().unwrap();
            let (kind, path) = match &project.source {
                ProjectSource::Git { common_dir } => ("git", run.checkout.as_ref().map(|checkout| checkout.path.clone()).unwrap_or_else(|| common_dir.clone())),
                ProjectSource::Directory { root } => ("directory", root.clone()),
            };
            let label = match &project.source {
                ProjectSource::Git { common_dir } => catalog.repositories.iter().find(|repo| &repo.common_dir == common_dir).map(|repo| repo.name.clone())
                    .or_else(|| Path::new(common_dir).parent().and_then(|parent| parent.file_name()).map(|name| name.to_string_lossy().into_owned())),
                ProjectSource::Directory { root } => Path::new(root).file_name().map(|name| name.to_string_lossy().into_owned()),
            }.unwrap_or_else(|| workspace.label.clone());
            (Some(project.key.clone()), kind, path, label)
        } else {
            let (project, checkout) = cockpit_core::orchestration::project_identity::resolve_directory(
                Path::new(&workspace.cwd), None, Some(workspace.workspace_id.clone())).await?;
            let kind = match project.source { ProjectSource::Git { .. } => "git", ProjectSource::Directory { .. } => "directory" };
            let label = catalog.repositories.iter().find(|repo| repo.common_dir == checkout.git.as_ref().map(|git| git.common_dir.as_str()).unwrap_or(""))
                .map(|repo| repo.name.clone()).unwrap_or_else(|| workspace.label.clone());
            (Some(project.key), kind, checkout.path, label)
        };
        let general = key.is_some() && key == liaison_key;
        let index = if general { None } else { Some(output.projects.iter().position(|project| match (&project.key, &key) {
            (Some(a), Some(b)) => a == b || project.path == path,
            _ => project.path == path,
        }).unwrap_or_else(|| {
            output.projects.push(Project { label, key, kind, path, spaces: Vec::new(), tasks: Vec::new() });
            output.projects.len() - 1
        })) };
        // Herdr detects agents by kind; names exist only when an agent reports one.
        let agents = runtime.panes.iter().filter(|pane| pane.workspace_id == workspace.workspace_id
            && pane.agent_kind.as_deref().is_some_and(|kind| !kind.is_empty())).map(|pane| {
            let observed_run = match &snapshot.runtime {
                RuntimeObservation::Fresh { runs, .. } => runs.iter().find(|observation| observation.actual_agent && observation.pane_id.as_deref() == Some(&pane.pane_id)),
                RuntimeObservation::Unavailable { .. } => None,
            };
            let run = observed_run.and_then(|observed| snapshot.runs.iter().find(|run| run.run_id == observed.run_id));
            let task = run.and_then(|run| run.task_id.as_ref()).and_then(|id| board.and_then(|board| board.tasks.iter().find(|view| &view.task.task_id == id)))
                .map(|view| AgentTask { task_id: view.task.task_id.clone(), title: view.task.title.clone(), lane: view.lane });
            Agent { pane_id: pane.pane_id.clone(), name: pane.agent_name.clone().filter(|name| !name.is_empty()).or_else(|| pane.agent_kind.clone()).unwrap_or_default(), kind: pane.agent_kind.clone().unwrap_or_else(|| "unknown".into()),
                status: pane.agent_status.clone().unwrap_or_else(|| "unknown".into()), managed: run.is_some(), run_id: run.map(|run| run.run_id.clone()), task }
        }).collect();
        let space = Space { workspace_id: workspace.workspace_id.clone(), label: workspace.label.clone(), cwd: workspace.cwd.clone(), agents };
        match index {
            Some(index) => output.projects[index].spaces.push(space),
            None => output.general_spaces.push(space),
        }
    }
    if let Some(board) = board {
        for view in &board.tasks {
            let task = Task { task_id: view.task.task_id.clone(), title: view.task.title.clone(), lane: view.lane, run_id: view.current_run_id.clone() };
            let Some(identity) = &view.project else {
                output.general_tasks.push(task);
                continue;
            };
            let target = snapshot.task_ownership.iter().find(|record| record.root_id == board.root_id && record.task_id == view.task.task_id)
                .and_then(|record| record.schedule.as_ref()).and_then(|schedule| schedule.target.as_ref());
            let target_space = match target {
                Some(cockpit_protocol::orchestration::DispatchTarget::ExistingSpace { workspace_id }
                    | cockpit_protocol::orchestration::DispatchTarget::SpaceWorktree { workspace_id, .. }) => Some(workspace_id),
                _ => None,
            };
            let existing = output.projects.iter().position(|project| {
                identity.key.as_ref().is_some_and(|key| project.key.as_ref() == Some(key))
                    || target_space.is_some_and(|id| project.spaces.iter().any(|space| &space.workspace_id == id))
            });
            let index = if let Some(index) = existing { index } else {
                let run = snapshot.runs.iter().filter(|run| run.task_id.as_deref() == Some(&view.task.task_id) && run.project.is_some())
                    .max_by(|a, b| (a.attempt, &a.created_at, &a.run_id).cmp(&(b.attempt, &b.created_at, &b.run_id)));
                let details = run.and_then(|run| run.project.as_ref().map(|project| match &project.source {
                    ProjectSource::Git { common_dir } => ("git", run.checkout.as_ref().map(|checkout| checkout.path.clone()).unwrap_or_else(|| common_dir.clone())),
                    ProjectSource::Directory { root } => ("directory", root.clone()),
                }));
                let (key, kind, path) = if let Some((kind, path)) = details {
                    (identity.key.clone(), kind, path)
                } else {
                    let target_path = match target {
                        Some(cockpit_protocol::orchestration::DispatchTarget::Setup { request: cockpit_protocol::projects::WorkspaceSetupRequest::Open { path, .. } }) => Some(path.as_str()),
                        Some(cockpit_protocol::orchestration::DispatchTarget::Setup { request: cockpit_protocol::projects::WorkspaceSetupRequest::Create { repository_id, .. } }) =>
                            catalog.repositories.iter().find(|repo| &repo.repository_id == repository_id).map(|repo| repo.checkout_path.as_str()),
                        _ => None,
                    }.ok_or_else(|| CliError::new("project_unavailable", format!("Cannot determine project location for task {}", view.task.task_id)))?;
                    let (project, checkout) = cockpit_core::orchestration::project_identity::resolve_directory(Path::new(target_path), None, None).await?;
                    let kind = match project.source { ProjectSource::Git { .. } => "git", ProjectSource::Directory { .. } => "directory" };
                    (Some(project.key), kind, checkout.path)
                };
                if let Some(index) = output.projects.iter().position(|project| key.as_ref().is_some_and(|key| project.key.as_ref() == Some(key))) {
                    output.projects[index].tasks.push(task);
                    continue;
                }
                output.projects.push(Project { label: identity.label.clone(), key, kind, path, spaces: Vec::new(), tasks: Vec::new() });
                output.projects.len() - 1
            };
            output.projects[index].tasks.push(task);
        }
    }
    output.catalog.retain(|entry| !output.projects.iter().any(|project| project.path == entry.path));
    Ok(output)
}

fn print_space(space: &Space) {
    println!("  Space {} [{}] {}", space.label, space.workspace_id, space.cwd);
    for agent in &space.agents {
        println!("    Agent {} [{}] {}{}{}", agent.name, agent.pane_id, agent.status,
            if agent.managed { "" } else { " (unmanaged)" },
            agent.task.as_ref().map(|task| format!(" — {}", task.title)).unwrap_or_default());
    }
}

fn print_tree(output: &WhereOutput) {
    for project in &output.projects {
        println!("{} ({}, {})", project.label, project.kind, project.path);
        for space in &project.spaces { print_space(space); }
        for task in &project.tasks { println!("  Task {} [{}] {:?}", task.title, task.task_id, task.lane); }
    }
    if !output.general_tasks.is_empty() || !output.general_spaces.is_empty() {
        println!("General");
        for space in &output.general_spaces { print_space(space); }
        for task in &output.general_tasks { println!("  Task {} [{}] {:?}", task.title, task.task_id, task.lane); }
    }
    if !output.catalog.is_empty() {
        println!("Catalog");
        for entry in &output.catalog { println!("  {} [{}] {}", entry.label, entry.id, entry.path); }
    }
    for diagnostic in &output.catalog_diagnostics { eprintln!("{}: {}", diagnostic.code, diagnostic.message); }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_output_includes_projects_spaces_agents_and_catalog() {
        let output = WhereOutput {
            projects: vec![Project {
                label: "Project".into(), key: Some("key".into()), kind: "git", path: "/project".into(),
                spaces: vec![Space { workspace_id: "space".into(), label: "Project".into(), cwd: "/project".into(),
                    agents: vec![Agent { pane_id: "pane".into(), name: "agent".into(), kind: "omp".into(),
                        status: "working".into(), managed: true, run_id: Some("run".into()),
                        task: Some(AgentTask { task_id: "task".into(), title: "Fix".into(), lane: TaskLane::Working }) }] }],
                tasks: vec![Task { task_id: "task".into(), title: "Fix".into(), lane: TaskLane::Working, run_id: Some("run".into()) }],
            }],
            general_tasks: vec![], general_spaces: vec![],
            catalog: vec![CatalogEntry { id: "repo".into(), label: "Project".into(), path: "/project".into() }],
            catalog_diagnostics: vec![],
        };
        let value = serde_json::to_value(output).unwrap();
        assert_eq!(value["projects"][0]["kind"], "git");
        assert_eq!(value["projects"][0]["spaces"][0]["agents"][0]["task"]["lane"], "working");
        assert_eq!(value["projects"][0]["tasks"][0]["run_id"], "run");
        assert_eq!(value["catalog"][0]["id"], "repo");
        assert_eq!(value["general_tasks"], serde_json::json!([]));
    }
}
