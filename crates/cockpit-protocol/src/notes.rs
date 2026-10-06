use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct NotesRequest {
    pub target: NotesTarget,
    pub operation: NotesOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum NotesTarget {
    #[serde(deserialize_with = "deserialize_empty")]
    Root,
    Notes {
        notes_id: String,
    },
    Space {
        session_id: String,
        space_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum NotesLane {
    Backlog,
    Doing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum NotesColumn {
    Backlog,
    Doing,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum NotesTodoFilter {
    All,
    Open,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum NotesDecisionFilter {
    Current,
    History,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "by", rename_all = "snake_case")]
pub enum NotesTodoSelector {
    Id {
        id: String,
        expected_revision: String,
    },
    Ref {
        #[serde(rename = "ref")]
        #[ts(rename = "ref")]
        reference: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "op", rename_all = "snake_case")]
pub enum NotesOperation {
    #[serde(deserialize_with = "deserialize_empty")]
    CatalogList,
    #[serde(deserialize_with = "deserialize_empty")]
    TargetResolve,
    #[serde(deserialize_with = "deserialize_empty")]
    TargetCreate,
    TargetAttach {
        notes_id: String,
    },
    #[serde(deserialize_with = "deserialize_empty")]
    ScratchpadRead,
    ScratchpadAppend {
        text: String,
        expected_revision: Option<String>,
    },
    ScratchpadReplace {
        content: String,
        expected_revision: String,
    },
    TodoList {
        filter: NotesTodoFilter,
    },
    TodoAdd {
        text: String,
        lane: Option<NotesLane>,
    },
    TodoUpdate {
        todo: NotesTodoSelector,
        text: Option<String>,
    },
    TodoSetDone {
        todo: NotesTodoSelector,
        done: bool,
    },
    TodoRemove {
        todo: NotesTodoSelector,
    },
    #[serde(deserialize_with = "deserialize_empty")]
    KanbanList,
    KanbanPromote {
        todo: NotesTodoSelector,
    },
    KanbanMove {
        todo: NotesTodoSelector,
        to: NotesColumn,
    },
    KanbanUnboard {
        todo: NotesTodoSelector,
    },
    DecisionList {
        status: NotesDecisionFilter,
        query: Option<String>,
    },
    DecisionGet {
        decision_id: String,
    },
    DecisionCreate {
        title: String,
        body: String,
        decided: Option<String>,
    },
    DecisionUpdate {
        decision_id: String,
        expected_revision: String,
        title: Option<String>,
        body: Option<String>,
    },
    DecisionReplace {
        decision_id: String,
        expected_revision: String,
        title: String,
        body: String,
        decided: Option<String>,
    },
    CommentList {
        todo_id: String,
    },
    CommentGet {
        todo_id: String,
        comment_id: String,
    },
    CommentAdd {
        todo_id: String,
        body: String,
        author: Option<String>,
    },
    CommentUpdate {
        todo_id: String,
        comment_id: String,
        expected_revision: String,
        body: String,
    },
    CommentRemove {
        todo_id: String,
        comment_id: String,
        expected_revision: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesResponse {
    pub notes_id: Option<String>,
    pub changed: bool,
    pub result: NotesResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum NotesResult {
    Catalog {
        entries: Vec<NotesCatalogEntry>,
    },
    Target {
        info: NotesTargetInfo,
    },
    Scratchpad {
        document: NotesDocument,
    },
    Todos {
        revision: String,
        todos: Vec<NotesTodo>,
    },
    Todo {
        revision: String,
        todo: NotesTodo,
    },
    TodoRemoved {
        revision: String,
    },
    Board {
        revision: String,
        columns: NotesBoard,
    },
    Decisions {
        decisions: Vec<NotesDecisionSummary>,
    },
    Decision {
        decision: NotesDecision,
    },
    Comments {
        todo_id: String,
        comments: Vec<NotesComment>,
    },
    Comment {
        comment: NotesComment,
    },
    CommentRemoved {
        todo_id: String,
        comment_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesCatalogEntry {
    pub notes_id: String,
    pub label: Option<String>,
    pub created: Option<String>,
    pub bound: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesTargetInfo {
    pub notes_id: String,
    /// Absolute path to the durable notes folder.
    pub folder: String,
    pub space: Option<NotesSpaceInfo>,
    pub change_tokens: NotesChangeTokens,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesSpaceInfo {
    pub session_id: String,
    pub space_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesChangeTokens {
    pub scratchpad: String,
    pub todos: String,
    pub decisions: String,
    pub comments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesDocument {
    pub content: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesTodo {
    pub id: Option<String>,
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub reference: String,
    pub text: String,
    pub done: bool,
    pub lane: Option<NotesLane>,
    pub revision: String,
    pub line: u32,
    pub depth: u32,
    pub problems: Vec<NotesTodoProblem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum NotesTodoProblem {
    DuplicateId,
    UnknownLane,
    MetadataMalformed,
    LazyContinuation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesBoard {
    pub backlog: Vec<NotesTodo>,
    pub doing: Vec<NotesTodo>,
    pub done: Vec<NotesTodo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum NotesDecisionStatus {
    Current,
    Replaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesDecisionSummary {
    pub decision_id: String,
    pub title: String,
    pub recorded: Option<String>,
    pub decided: Option<String>,
    pub replaces: Option<String>,
    pub replaced_by: Vec<String>,
    pub status: NotesDecisionStatus,
    pub revision: String,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesDecision {
    pub summary: NotesDecisionSummary,
    pub body: String,
    pub relative_path: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NotesComment {
    pub todo_id: String,
    pub comment_id: String,
    pub created: Option<String>,
    pub author: Option<String>,
    pub body: String,
    pub revision: String,
}

// Serde's internally tagged unit visitor otherwise ignores unknown fields,
// even when the enum has deny_unknown_fields.
fn deserialize_empty<'de, D>(deserializer: D) -> Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    Empty::deserialize(deserializer).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn requests_reject_unknown_fields_at_every_request_boundary() {
        for value in [
            json!({"target": {"kind": "root"}, "operation": {"op": "catalog_list"}, "extra": true}),
            json!({"target": {"kind": "root", "extra": true}, "operation": {"op": "catalog_list"}}),
            json!({"target": {"kind": "notes", "notes_id": "n", "extra": true}, "operation": {"op": "kanban_list"}}),
            json!({"target": {"kind": "space", "session_id": "s", "space_id": "w", "extra": true}, "operation": {"op": "target_resolve"}}),
            json!({"target": {"kind": "root"}, "operation": {"op": "catalog_list", "extra": true}}),
            json!({"target": {"kind": "notes", "notes_id": "n"}, "operation": {"op": "todo_remove", "todo": {"by": "ref", "ref": "L1@absent", "extra": true}}}),
            json!({"target": {"kind": "notes", "notes_id": "n"}, "operation": {"op": "todo_remove", "todo": {"by": "id", "id": "a", "expected_revision": "r", "extra": true}}}),
        ] {
            assert!(serde_json::from_value::<NotesRequest>(value).is_err());
        }
    }

    #[test]
    fn optional_fields_serialize_as_null_and_references_use_ref() {
        let request = NotesRequest {
            target: NotesTarget::Notes {
                notes_id: "n".into(),
            },
            operation: NotesOperation::TodoUpdate {
                todo: NotesTodoSelector::Ref {
                    reference: "L1@absent".into(),
                },
                text: None,
            },
        };
        let value = serde_json::to_value(&request).expect("serialize request");
        assert_eq!(
            value["operation"]["todo"],
            json!({"by": "ref", "ref": "L1@absent"})
        );
        assert_eq!(value["operation"]["text"], Value::Null);
        assert_eq!(
            serde_json::from_value::<NotesRequest>(value).expect("round trip"),
            request
        );
        let response = NotesResponse {
            notes_id: None,
            changed: false,
            result: NotesResult::Catalog { entries: vec![] },
        };
        assert_eq!(
            serde_json::to_value(response).expect("serialize response"),
            json!({
                "notes_id": null,
                "changed": false,
                "result": {"kind": "catalog", "entries": []},
            })
        );
    }
}
