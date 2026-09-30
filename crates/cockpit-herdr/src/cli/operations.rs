use cockpit_protocol::v1::{
    FocusKind, FocusRequest, PaneMoveDestination, PaneSplitDirection, ResourceMutationRequest,
};
use serde_json::{Value, json};

pub(crate) fn request_is_mutating(method: &str) -> bool {
    matches!(
        method,
        "workspace.create"
            | "worktree.create"
            | "worktree.open"
            | "workspace.rename"
            | "workspace.move_block"
            | "workspace.close"
            | "worktree.remove"
            | "workspace.focus"
            | "tab.create"
            | "tab.rename"
            | "tab.move"
            | "tab.close"
            | "tab.focus"
            | "pane.split"
            | "pane.rename"
            | "pane.move"
            | "pane.close"
            | "pane.focus"
            | "agent.focus"
            | "pane.send_text"
            | "command.invoke"
    )
}

/// Reads may be dispatched before peer-identity validation because they cannot
/// mutate Herdr state. Unknown methods remain verify-before-dispatch.
pub(crate) fn request_is_safe_to_dispatch_before_identity(method: &str) -> bool {
    matches!(
        method,
        "ping"
            | "session.snapshot"
            | "pane.get"
            | "worktree.list"
            | "workspace.list"
            | "workspace.get"
            | "tab.list"
            | "tab.get"
    )
}

pub(crate) fn focus_call(request: &FocusRequest) -> (&'static str, Value) {
    match request.kind {
        FocusKind::Space => (
            "workspace.focus",
            json!({"workspace_id": request.target_id}),
        ),
        FocusKind::Tab => ("tab.focus", json!({"tab_id": request.target_id})),
        FocusKind::Pane => ("pane.focus", json!({"pane_id": request.target_id})),
        FocusKind::Agent => ("agent.focus", json!({"target": request.target_id})),
    }
}

fn split_direction(direction: PaneSplitDirection) -> &'static str {
    match direction {
        PaneSplitDirection::Right => "right",
        PaneSplitDirection::Down => "down",
    }
}

fn insert_optional(params: &mut serde_json::Map<String, Value>, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        params.insert(key.to_owned(), Value::String(value.clone()));
    }
}

fn insert_optional_number(
    params: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<f64>,
) {
    if let Some(value) = value {
        params.insert(key.to_owned(), json!(value));
    }
}

pub(crate) fn pane_move_destination(destination: &PaneMoveDestination) -> Value {
    match destination {
        PaneMoveDestination::ExistingTab {
            tab_id,
            direction,
            target_pane_id,
            ratio,
        } => {
            let mut params = serde_json::Map::from_iter([
                ("type".to_owned(), json!("tab")),
                ("tab_id".to_owned(), json!(tab_id)),
                ("split".to_owned(), json!(split_direction(*direction))),
            ]);
            insert_optional(&mut params, "target_pane_id", target_pane_id);
            insert_optional_number(&mut params, "ratio", *ratio);
            Value::Object(params)
        }
        PaneMoveDestination::NewTab { space_id, label } => {
            let mut params = serde_json::Map::from_iter([("type".to_owned(), json!("new_tab"))]);
            if let Some(space_id) = space_id {
                params.insert("workspace_id".to_owned(), json!(space_id));
            }
            insert_optional(&mut params, "label", label);
            Value::Object(params)
        }
        PaneMoveDestination::NewSpace { label, tab_label } => {
            let mut params =
                serde_json::Map::from_iter([("type".to_owned(), json!("new_workspace"))]);
            insert_optional(&mut params, "label", label);
            insert_optional(&mut params, "tab_label", tab_label);
            Value::Object(params)
        }
    }
}

pub(crate) fn mutation_call(request: &ResourceMutationRequest) -> (&'static str, Value) {
    match request {
        ResourceMutationRequest::CommandInvoke { command_id, space_id, tab_id, pane_id } => {
            let mut params = serde_json::Map::from_iter([
                ("command_id".to_owned(), json!(command_id)),
                ("workspace_id".to_owned(), json!(space_id)),
                ("tab_id".to_owned(), json!(tab_id)),
            ]);
            insert_optional(&mut params, "pane_id", pane_id);
            ("command.invoke", Value::Object(params))
        }
        ResourceMutationRequest::SpaceCreate { cwd, label } => {
            let mut params = serde_json::Map::from_iter([("focus".to_owned(), json!(true))]);
            insert_optional(&mut params, "cwd", cwd);
            insert_optional(&mut params, "label", label);
            ("workspace.create", Value::Object(params))
        }
        ResourceMutationRequest::SpaceRename { space_id, label } => (
            "workspace.rename",
            json!({"workspace_id": space_id, "label": label}),
        ),
        ResourceMutationRequest::SpaceMoveBlock {
            space_ids,
            before_space_id,
        } => {
            let mut params =
                serde_json::Map::from_iter([("workspace_ids".to_owned(), json!(space_ids))]);
            if let Some(before_space_id) = before_space_id {
                params.insert("before_workspace_id".to_owned(), json!(before_space_id));
            }
            ("workspace.move_block", Value::Object(params))
        }
        ResourceMutationRequest::SpaceClose { space_id } => {
            ("workspace.close", json!({"workspace_id": space_id}))
        }
        ResourceMutationRequest::TabCreate { space_id, label } => {
            let mut params = serde_json::Map::from_iter([
                ("workspace_id".to_owned(), json!(space_id)),
                ("focus".to_owned(), json!(true)),
            ]);
            insert_optional(&mut params, "label", label);
            ("tab.create", Value::Object(params))
        }
        ResourceMutationRequest::TabRename { tab_id, label } => {
            ("tab.rename", json!({"tab_id": tab_id, "label": label}))
        }
        ResourceMutationRequest::TabMove {
            tab_id,
            insert_index,
        } => (
            "tab.move",
            json!({"tab_id": tab_id, "insert_index": insert_index}),
        ),
        ResourceMutationRequest::TabClose { tab_id } => ("tab.close", json!({"tab_id": tab_id})),
        ResourceMutationRequest::PaneSplit {
            pane_id,
            direction,
            ratio,
        } => {
            let mut params = serde_json::Map::from_iter([
                ("target_pane_id".to_owned(), json!(pane_id)),
                ("direction".to_owned(), json!(split_direction(*direction))),
                ("focus".to_owned(), json!(true)),
            ]);
            insert_optional_number(&mut params, "ratio", *ratio);
            ("pane.split", Value::Object(params))
        }
        ResourceMutationRequest::PaneRename { pane_id, label } => {
            ("pane.rename", json!({"pane_id": pane_id, "label": label}))
        }
        ResourceMutationRequest::PaneMove {
            pane_id,
            destination,
        } => (
            "pane.move",
            json!({
                "pane_id": pane_id,
                "destination": pane_move_destination(destination),
                "focus": true
            }),
        ),
        ResourceMutationRequest::PaneClose { pane_id } => {
            ("pane.close", json!({"pane_id": pane_id}))
        }
    }
}

#[cfg(test)]
mod tests {
    use cockpit_protocol::v1::{FocusKind, FocusRequest};
    use serde_json::json;

    use super::focus_call;

    #[test]
    fn focus_calls_use_the_documented_resource_identifiers() {
        let cases = [
            (
                FocusKind::Space,
                "workspace.focus",
                json!({"workspace_id": "space-1"}),
            ),
            (FocusKind::Tab, "tab.focus", json!({"tab_id": "space-1"})),
            (FocusKind::Pane, "pane.focus", json!({"pane_id": "space-1"})),
            (
                FocusKind::Agent,
                "agent.focus",
                json!({"target": "space-1"}),
            ),
        ];

        for (kind, expected_method, expected_params) in cases {
            let request = FocusRequest {
                kind,
                target_id: "space-1".into(),
            };
            let (method, params) = focus_call(&request);
            assert_eq!(method, expected_method);
            assert_eq!(params, expected_params);
        }
    }
    #[test]
    fn only_allowlisted_reads_dispatch_before_peer_identity_check() {
        use super::request_is_safe_to_dispatch_before_identity;

        assert!(request_is_safe_to_dispatch_before_identity("pane.get"));
        assert!(request_is_safe_to_dispatch_before_identity("session.snapshot"));
        assert!(request_is_safe_to_dispatch_before_identity("worktree.list"));
        assert!(!request_is_safe_to_dispatch_before_identity("pane.send_text"));
        assert!(!request_is_safe_to_dispatch_before_identity("unknown.future_method"));
    }
}
