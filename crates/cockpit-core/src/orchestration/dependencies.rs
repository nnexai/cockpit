use std::collections::{HashMap, HashSet};

use cockpit_protocol::orchestration::{
    Task, TaskDependencies, TaskDependencyBlocker, TaskDependencyBlockerReason, TaskDependencyState,
};
use cockpit_protocol::v1::ErrorResponse;
use uuid::Uuid;

use crate::InspectionError;

const MAX_DEPENDENCIES: usize = 32;
const MAX_DIAGNOSTIC_BYTES: usize = 2048;

#[derive(Clone, Copy)]
enum Identity {
    Unique(usize),
    Ambiguous,
}

/// One immutable, same-document index and iterative SCC analysis. Checked facts
/// are direct prerequisites only; neither run history nor transitive checks apply.
pub(crate) struct DependencyGraph<'a> {
    tasks: &'a [Task],
    identities: HashMap<Uuid, Identity>,
    component: Vec<usize>,
    cycle_messages: Vec<Option<String>>,
}

impl<'a> DependencyGraph<'a> {
    pub(crate) fn new(tasks: &'a [Task]) -> Self {
        let identities = identity_index(tasks);
        // Flat CSR adjacency avoids a separate heap allocation for every task's
        // incoming/outgoing list. Resolve UUID references once while filling it.
        let edge_capacity = tasks.iter().map(|task| task.depends_on.len()).sum();
        let mut edges = Vec::with_capacity(edge_capacity);
        let mut offsets = Vec::with_capacity(tasks.len() + 1);
        let mut reverse_offsets = vec![0; tasks.len() + 1];
        offsets.push(0);
        for task in tasks {
            for dependency in &task.depends_on {
                if let Ok(id) = Uuid::parse_str(dependency)
                    && let Some(Identity::Unique(target)) = identities.get(&id)
                {
                    edges.push(*target);
                    reverse_offsets[*target + 1] += 1;
                }
            }
            offsets.push(edges.len());
        }
        for index in 1..reverse_offsets.len() {
            reverse_offsets[index] += reverse_offsets[index - 1];
        }
        let mut reverse = vec![0; edges.len()];
        // Reuse prefix ends as fill cursors; descending sources preserve the
        // original ascending predecessor order without a separate cursor copy.
        for source in (0..tasks.len()).rev() {
            for &target in edges[offsets[source]..offsets[source + 1]].iter().rev() {
                reverse_offsets[target + 1] -= 1;
                reverse[reverse_offsets[target + 1]] = source;
            }
        }
        // The decremented ends now hold starts, shifted one place to the right.
        reverse_offsets.copy_within(1.., 0);
        reverse_offsets[tasks.len()] = reverse.len();

        // Kosaraju's finishing-order pass, with explicit DFS frames instead of
        // recursion: an externally authored deep chain cannot overflow the stack.
        let mut visited = vec![false; tasks.len()];
        let mut order = Vec::with_capacity(tasks.len());
        let mut frames = Vec::new();
        for start in 0..tasks.len() {
            if visited[start] {
                continue;
            }
            visited[start] = true;
            frames.push((start, 0));
            while let Some((node, next)) = frames.last_mut() {
                let neighbors = &edges[offsets[*node]..offsets[*node + 1]];
                if *next < neighbors.len() {
                    let target = neighbors[*next];
                    *next += 1;
                    if !visited[target] {
                        visited[target] = true;
                        frames.push((target, 0));
                    }
                } else {
                    order.push(*node);
                    frames.pop();
                }
            }
        }

        let mut component = vec![usize::MAX; tasks.len()];
        let mut cycle_messages = Vec::new();
        let mut pending = Vec::new();
        let mut members = Vec::new();
        for start in order.into_iter().rev() {
            if component[start] != usize::MAX {
                continue;
            }
            let component_id = cycle_messages.len();
            members.clear();
            component[start] = component_id;
            pending.push(start);
            while let Some(node) = pending.pop() {
                members.push(node);
                for &target in &reverse[reverse_offsets[node]..reverse_offsets[node + 1]] {
                    if component[target] == usize::MAX {
                        component[target] = component_id;
                        pending.push(target);
                    }
                }
            }
            let cyclic =
                members.len() > 1 || edges[offsets[start]..offsets[start + 1]].contains(&start);
            cycle_messages.push(cyclic.then(|| {
                let mut message = String::from("Dependency cycle contains tasks: ");
                for (position, &member) in members.iter().take(32).enumerate() {
                    if position != 0 {
                        message.push_str(", ");
                    }
                    message.push_str(bounded(&tasks[member].task_id, 80));
                }
                if members.len() > 32 {
                    message.push_str(&format!(" ({} more members)", members.len() - 32));
                }
                bounded(&message, MAX_DIAGNOSTIC_BYTES).to_owned()
            }));
        }
        Self {
            tasks,
            identities,
            component,
            cycle_messages,
        }
    }

    pub(crate) fn evaluate(&self, task: &Task) -> TaskDependencies {
        let mut result = TaskDependencies {
            state: TaskDependencyState::None,
            unmet: Vec::new(),
            problems: Vec::new(),
        };
        let mut invalid = false;
        let own_id = Uuid::parse_str(&task.task_id).ok();
        match own_id.and_then(|id| self.identities.get(&id)) {
            Some(Identity::Unique(index)) => {
                if let Some(message) = &self.cycle_messages[self.component[*index]] {
                    invalid = true;
                    result
                        .problems
                        .push(problem("task_dependencies_invalid", message));
                }
            }
            _ => {
                invalid = true;
                result.problems.push(problem(
                    "task_dependencies_invalid",
                    &format!(
                        "Task {} has missing, invalid or ambiguous canonical identity",
                        bounded(&task.task_id, 80),
                    ),
                ));
            }
        }
        if let Some(diagnostic) = &task.relations_diagnostic {
            invalid = true;
            result.problems.push(problem(
                "task_dependencies_invalid",
                &format!(
                    "Task {}: {}",
                    bounded(&task.task_id, 80),
                    bounded(diagnostic, MAX_DIAGNOSTIC_BYTES - 100),
                ),
            ));
        }
        if task.depends_on.len() > MAX_DEPENDENCIES {
            invalid = true;
            result.problems.push(problem(
                "task_dependencies_invalid",
                &format!(
                    "Task {} has {} prerequisite entries; maximum is {MAX_DEPENDENCIES}",
                    bounded(&task.task_id, 80),
                    task.depends_on.len(),
                ),
            ));
        }
        let mut seen = HashSet::new();
        for dependency in &task.depends_on {
            let Ok(id) = Uuid::parse_str(dependency) else {
                invalid = true;
                result.problems.push(problem(
                    "task_dependencies_invalid",
                    &format!(
                        "Task {} has invalid prerequisite UUID {}",
                        bounded(&task.task_id, 80),
                        bounded(dependency, 80),
                    ),
                ));
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            if Some(id) == own_id {
                invalid = true;
                result.problems.push(problem(
                    "task_dependencies_invalid",
                    &format!("Task {} depends on itself", bounded(&task.task_id, 80),),
                ));
            }
            let reason = match self.identities.get(&id) {
                None => Some(TaskDependencyBlockerReason::Missing),
                Some(Identity::Ambiguous) => Some(TaskDependencyBlockerReason::Ambiguous),
                Some(Identity::Unique(index)) if !self.tasks[*index].checked => {
                    Some(TaskDependencyBlockerReason::Unchecked)
                }
                Some(Identity::Unique(_)) => None,
            };
            if let Some(reason) = reason {
                if reason != TaskDependencyBlockerReason::Unchecked {
                    invalid = true;
                    result.problems.push(problem(
                        "task_dependencies_invalid",
                        &format!(
                            "Task {} prerequisite {id} is {}",
                            bounded(&task.task_id, 80),
                            if reason == TaskDependencyBlockerReason::Missing {
                                "missing from this root"
                            } else {
                                "ambiguous in this root"
                            },
                        ),
                    ));
                }
                result.unmet.push(TaskDependencyBlocker {
                    task_id: id.to_string(),
                    reason,
                });
            }
        }
        result.state = if invalid {
            TaskDependencyState::Invalid
        } else if !result.unmet.is_empty() {
            TaskDependencyState::Blocked
        } else if !task.depends_on.is_empty() {
            TaskDependencyState::Satisfied
        } else {
            TaskDependencyState::None
        };

        // Provenance remains visible but never creates a scheduling prerequisite.
        if let Some(source) = &task.follow_up_of {
            let status = match Uuid::parse_str(source)
                .ok()
                .and_then(|id| self.identities.get(&id))
            {
                Some(Identity::Unique(_)) => None,
                Some(Identity::Ambiguous) => Some("ambiguous"),
                None => Some("missing or invalid"),
            };
            if let Some(status) = status {
                result.problems.push(problem(
                    "task_follow_up_source_unavailable",
                    &format!(
                        "Task {} follow-up source {} is {status}; provenance is nonblocking",
                        bounded(&task.task_id, 80),
                        bounded(source, 80),
                    ),
                ));
            }
        }
        result
    }

    /// Prepublication eligibility only. Already-published acceptance recovery
    /// must deliberately skip this gate rather than roll canonical effects back.
    pub(crate) fn require(&self, task: &Task) -> Result<(), InspectionError> {
        let facts = self.evaluate(task);
        let code = match facts.state {
            TaskDependencyState::None | TaskDependencyState::Satisfied => return Ok(()),
            TaskDependencyState::Invalid => "task_dependencies_invalid",
            TaskDependencyState::Blocked => "task_blocked",
        };
        let mut message = format!("Task {}: ", bounded(&task.task_id, 80));
        for issue in &facts.problems {
            if issue.code == "task_dependencies_invalid" {
                append_bounded(&mut message, &issue.message);
            }
        }
        for blocker in &facts.unmet {
            append_bounded(
                &mut message,
                &format!("prerequisite {} is {:?}", blocker.task_id, blocker.reason,),
            );
        }
        Err(InspectionError::new(code, message))
    }
}

/// Validate replacement outgoing edges or edges of a not-yet-created task.
/// No task clones, checkbox requirement or global-corruption veto is involved.
pub(crate) fn validate_dependencies(
    tasks: &[Task],
    task_id: &str,
    depends_on: &[String],
) -> Result<Vec<String>, InspectionError> {
    let invalid = |message: String| InspectionError::new("task_dependencies_invalid", message);
    if depends_on.len() > MAX_DEPENDENCIES {
        return Err(invalid(format!(
            "Task {} exceeds {MAX_DEPENDENCIES} prerequisite entries",
            bounded(task_id, 80)
        )));
    }
    let target_id = Uuid::parse_str(task_id)
        .map_err(|_| invalid(format!("Task UUID {} is invalid", bounded(task_id, 80))))?;
    let identities = identity_index(tasks);
    if matches!(identities.get(&target_id), Some(Identity::Ambiguous)) {
        return Err(invalid(format!(
            "Task {target_id} has ambiguous canonical identity"
        )));
    }
    let mut normalized = Vec::with_capacity(depends_on.len());
    for dependency in depends_on {
        let id = Uuid::parse_str(dependency).map_err(|_| {
            invalid(format!(
                "Task {target_id} prerequisite UUID {} is invalid",
                bounded(dependency, 80),
            ))
        })?;
        if id == target_id {
            return Err(invalid(format!("Task {target_id} cannot depend on itself")));
        }
        match identities.get(&id) {
            Some(Identity::Unique(_)) => normalized.push(id),
            Some(Identity::Ambiguous) => {
                return Err(invalid(format!(
                    "Task {target_id} prerequisite {id} is ambiguous in this root",
                )));
            }
            None => {
                return Err(invalid(format!(
                    "Task {target_id} prerequisite {id} is missing from this root",
                )));
            }
        }
    }
    normalized.sort_unstable();
    normalized.dedup();

    // A replacement creates a target cycle iff a requested prerequisite reaches
    // the target. Stop on that identity: old outgoing target edges do not apply.
    // Inspect raw references too, since an appended target was formerly missing.
    let mut visited = vec![false; tasks.len()];
    let mut pending = Vec::new();
    for dependency in &normalized {
        if let Some(Identity::Unique(index)) = identities.get(dependency)
            && !visited[*index]
        {
            visited[*index] = true;
            pending.push(*index);
        }
    }
    while let Some(index) = pending.pop() {
        for dependency in &tasks[index].depends_on {
            let Ok(id) = Uuid::parse_str(dependency) else {
                continue;
            };
            if id == target_id {
                return Err(invalid(format!(
                    "Task {target_id} prerequisites would create a cycle through {}",
                    bounded(&tasks[index].task_id, 80),
                )));
            }
            if let Some(Identity::Unique(next)) = identities.get(&id)
                && !visited[*next]
            {
                visited[*next] = true;
                pending.push(*next);
            }
        }
    }
    Ok(normalized.into_iter().map(|id| id.to_string()).collect())
}

fn identity_index(tasks: &[Task]) -> HashMap<Uuid, Identity> {
    let mut identities = HashMap::with_capacity(tasks.len());
    for (index, task) in tasks.iter().enumerate() {
        if let Ok(id) = Uuid::parse_str(&task.task_id) {
            identities
                .entry(id)
                .and_modify(|identity| *identity = Identity::Ambiguous)
                .or_insert(Identity::Unique(index));
        }
    }
    identities
}

fn bounded(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn problem(code: &str, message: &str) -> ErrorResponse {
    ErrorResponse {
        code: code.to_owned(),
        message: bounded(message, MAX_DIAGNOSTIC_BYTES).to_owned(),
    }
}

fn append_bounded(message: &mut String, fragment: &str) {
    if message.len() + 2 < MAX_DIAGNOSTIC_BYTES {
        message.push_str(bounded(fragment, MAX_DIAGNOSTIC_BYTES - message.len() - 2));
        message.push_str("; ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(number: u128) -> String {
        Uuid::from_u128(number).to_string()
    }

    fn task(number: u128, prerequisites: &[u128], checked: bool) -> Task {
        Task {
            task_id: id(number),
            title: format!("Task {number}"),
            body: String::new(),
            description: String::new(),
            description_editable: true,
            description_diagnostic: None,
            steps: Vec::new(),
            step_progress: None,
            steps_diagnostic: None,
            depends_on: prerequisites.iter().map(|n| id(*n)).collect(),
            follow_up_of: None,
            relations_diagnostic: None,
            checked,
            line: 1,
            task_revision: "a".repeat(64),
            diagnostic: None,
        }
    }

    #[test]
    fn direct_checked_facts_not_results_or_transitive_completion_control_readiness() {
        let mut tasks = vec![
            task(1, &[], false),
            task(2, &[1, 3], false),
            task(3, &[4], true),
            task(4, &[], false),
        ];
        tasks[0].body = "Successful Result, executed grants, worker done".into();
        let graph = DependencyGraph::new(&tasks);
        let facts = graph.evaluate(&tasks[1]);
        assert_eq!(facts.state, TaskDependencyState::Blocked);
        assert_eq!(facts.unmet.len(), 1);
        assert_eq!(facts.unmet[0].task_id, id(1));
        assert_eq!(
            facts.unmet[0].reason,
            TaskDependencyBlockerReason::Unchecked
        );
        assert_eq!(graph.require(&tasks[1]).unwrap_err().code, "task_blocked");
        tasks[0].checked = true;
        tasks[0].description = "Changed after canonical acceptance".into();
        assert_eq!(
            DependencyGraph::new(&tasks).evaluate(&tasks[1]).state,
            TaskDependencyState::Satisfied
        );
    }

    #[test]
    fn invalid_components_do_not_disable_independent_or_checked_prerequisite_tasks() {
        let mut tasks = vec![
            task(1, &[2], true),
            task(2, &[1], false),
            task(3, &[], false),
            task(4, &[1], false),
            task(5, &[], false),
        ];
        tasks[4].relations_diagnostic = Some("Malformed relation record".into());
        let graph = DependencyGraph::new(&tasks);
        assert_eq!(
            graph.evaluate(&tasks[0]).state,
            TaskDependencyState::Invalid
        );
        let cycle = graph.evaluate(&tasks[1]);
        assert!(
            cycle
                .problems
                .iter()
                .any(|p| p.message.contains(&id(1)) && p.message.contains(&id(2)))
        );
        assert_eq!(graph.evaluate(&tasks[2]).state, TaskDependencyState::None);
        assert!(graph.require(&tasks[2]).is_ok());
        assert_eq!(
            graph.evaluate(&tasks[3]).state,
            TaskDependencyState::Satisfied
        );
        assert_eq!(
            graph.require(&tasks[4]).unwrap_err().code,
            "task_dependencies_invalid"
        );
    }

    #[test]
    fn missing_and_duplicate_prerequisites_fail_closed_including_checked_duplicates() {
        let tasks = vec![
            task(1, &[2, 3], false),
            task(2, &[], true),
            task(2, &[], true),
        ];
        let graph = DependencyGraph::new(&tasks);
        let facts = graph.evaluate(&tasks[0]);
        assert_eq!(facts.state, TaskDependencyState::Invalid);
        assert_eq!(facts.unmet.len(), 2);
        assert_eq!(
            facts.unmet[0].reason,
            TaskDependencyBlockerReason::Ambiguous
        );
        assert_eq!(facts.unmet[1].reason, TaskDependencyBlockerReason::Missing);
        assert_eq!(
            graph.evaluate(&tasks[1]).state,
            TaskDependencyState::Invalid
        );
    }

    #[test]
    fn unavailable_follow_up_provenance_warns_without_blocking() {
        let mut tasks = vec![task(1, &[], false), task(2, &[], true), task(2, &[], true)];
        tasks[0].follow_up_of = Some(id(3));
        let facts = DependencyGraph::new(&tasks).evaluate(&tasks[0]);
        assert_eq!(facts.state, TaskDependencyState::None);
        assert_eq!(facts.problems[0].code, "task_follow_up_source_unavailable");
        assert!(DependencyGraph::new(&tasks).require(&tasks[0]).is_ok());
        tasks[0].follow_up_of = Some(id(2));
        assert_eq!(
            DependencyGraph::new(&tasks).evaluate(&tasks[0]).state,
            TaskDependencyState::None
        );
    }

    #[test]
    fn authoring_canonicalizes_and_does_not_require_completion_or_global_validity() {
        let tasks = vec![
            task(1, &[], false),
            task(2, &[], false),
            task(3, &[99], false),
        ];
        let input = vec![id(2).to_uppercase(), id(1), id(2)];
        assert_eq!(
            validate_dependencies(&tasks, &id(4), &input).unwrap(),
            vec![id(1), id(2)]
        );
        assert!(validate_dependencies(&tasks, &id(3), &[]).is_ok());
        assert!(validate_dependencies(&tasks, &id(4), &vec![id(1); 33]).is_err());
    }

    #[test]
    fn hypothetical_edges_reject_existing_and_new_target_cycles_without_mutation() {
        let tasks = vec![
            task(1, &[], false),
            task(2, &[1], false),
            task(3, &[4], false),
        ];
        assert!(validate_dependencies(&tasks, &id(1), &[id(2)]).is_err());
        assert!(validate_dependencies(&tasks, &id(4), &[id(3)]).is_err());
        assert!(validate_dependencies(&tasks, &id(2), &[]).is_ok());
        assert!(tasks[0].depends_on.is_empty());
        assert_eq!(tasks[1].depends_on, vec![id(1)]);
        assert!(validate_dependencies(&tasks, &id(1), &[id(1)]).is_err());
        assert!(validate_dependencies(&tasks, &id(1), &[id(99)]).is_err());
        let duplicate = vec![task(1, &[], false), task(2, &[], false), task(2, &[], true)];
        assert!(validate_dependencies(&duplicate, &id(1), &[id(2)]).is_err());
        assert!(validate_dependencies(&duplicate, &id(2), &[]).is_err());
    }

    #[test]
    fn deep_saved_chains_use_iterative_analysis_and_direct_satisfaction() {
        let tasks: Vec<_> = (1..=20_000)
            .map(|number| {
                if number < 20_000 {
                    task(number, &[number + 1], true)
                } else {
                    task(number, &[], true)
                }
            })
            .collect();
        let graph = DependencyGraph::new(&tasks);
        assert_eq!(
            graph.evaluate(&tasks[0]).state,
            TaskDependencyState::Satisfied
        );
        assert_eq!(
            graph.evaluate(&tasks[19_999]).state,
            TaskDependencyState::None
        );
        assert!(validate_dependencies(&tasks, &id(20_000), &[id(1)]).is_err());
    }

    #[test]
    fn self_malformed_and_oversized_source_stay_invalid_with_bounded_messages() {
        let mut tasks = vec![
            task(1, &[1], false),
            task(2, &[], false),
            task(3, &[], false),
        ];
        tasks[1].depends_on = vec![id(3); 33];
        tasks[2].relations_diagnostic = Some("é".repeat(10_000));
        let graph = DependencyGraph::new(&tasks);
        for task in &tasks {
            let facts = graph.evaluate(task);
            assert_eq!(facts.state, TaskDependencyState::Invalid);
            assert!(
                facts
                    .problems
                    .iter()
                    .all(|p| p.message.len() <= MAX_DIAGNOSTIC_BYTES)
            );
            assert!(graph.require(task).unwrap_err().message.len() <= MAX_DIAGNOSTIC_BYTES);
        }
    }
}
