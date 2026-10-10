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
