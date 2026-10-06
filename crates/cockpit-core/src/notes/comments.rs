use std::path::Path;

use cap_std::fs::Dir;
use cockpit_protocol::notes::*;
use uuid::Uuid;

use super::{decisions, fs, todos};
use crate::InspectionError;

const FILE_MAX: usize = 96 * 1024;
const BODY_MAX: usize = 64 * 1024;
const ENTRY_MAX: usize = 1000;
const AGGREGATE_MAX: usize = 16 * 1024 * 1024;
const AUTHOR_MAX: usize = 128;

struct Record {
    comment: NotesComment,
    document: NotesDocument,
    body_start: usize,
    unterminated: bool,
    instant: Option<i128>,
}

impl Record {
    fn into_comment(mut self) -> NotesComment {
        self.document.content.drain(..self.body_start);
        self.comment.body = self.document.content;
        self.comment
    }
}

fn parse(
    todo_id: &str,
    comment_id: String,
    document: NotesDocument,
) -> Result<Record, InspectionError> {
    let fm = decisions::frontmatter(&document.content);
    let mut created = fm.values.get("created").cloned();
    let instant = created.as_deref().and_then(decisions::timestamp);
    if instant.is_none() {
        created = None;
    }
    let author = fm
        .values
        .get("author")
        .filter(|author| valid_author(author))
        .cloned();
    let body = &document.content[fm.body_start..];
    if body.len() > BODY_MAX {
        return Err(too_large());
    }
    Ok(Record {
        comment: NotesComment {
            todo_id: todo_id.to_owned(),
            comment_id,
            created,
            author,
            body: String::new(),
            revision: document.revision.clone(),
        },
        document,
        body_start: fm.body_start,
        unterminated: fm.unterminated,
        instant,
    })
}

struct Collection {
    records: Vec<Record>,
    entries: usize,
    bytes: usize,
}

fn collection(dir: &Dir, todo_id: &str) -> Result<Collection, InspectionError> {
    let names = fs::entries(dir, ENTRY_MAX)?;
    let mut records = Vec::new();
    let mut bytes = 0usize;
    for name in &names {
        let Some(id) = name.strip_suffix(".md") else {
            continue;
        };
        if name.starts_with('.') || fs::validate_uuid(id).is_err() {
            continue;
        }
        if !decisions::regular_entry(dir, name)? {
            continue;
        }
        let document = fs::read(dir, name, FILE_MAX)?;
        if document.revision == "absent" {
            continue;
        }
        bytes += document.content.len();
        if bytes > AGGREGATE_MAX {
            return Err(too_large());
        }
        records.push(parse(todo_id, id.to_owned(), document)?);
    }
    records.sort_by(|a, b| {
        match (a.instant, b.instant) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| a.comment.comment_id.cmp(&b.comment.comment_id))
    });
    Ok(Collection {
        records,
        entries: names.len(),
        bytes,
    })
}

fn comment_dir(dir: &Dir, todo_id: &str) -> Result<Option<Dir>, InspectionError> {
    match decisions::optional_child(dir, "comments")? {
        Some(comments) => decisions::optional_child(&comments, todo_id),
        None => Ok(None),
    }
}

fn too_large() -> InspectionError {
    fs::error(
        "notes_too_large",
        "Comment collection or payload exceeds its limit",
    )
}

fn valid_author(author: &str) -> bool {
    author.len() <= AUTHOR_MAX && !author.chars().any(char::is_control)
}

fn validate_body(body: &str) -> Result<(), InspectionError> {
    if body.len() > BODY_MAX {
        return Err(too_large());
    }
    Ok(())
}

fn check_revision(record: &Record, expected: &str) -> Result<(), InspectionError> {
    if record.document.revision != expected {
        return Err(fs::error(
            "notes_conflict",
            "Comment changed; re-read before editing",
        ));
    }
    Ok(())
}

pub(super) fn execute(
    dir: &Dir,
    _folder: &Path,
    op: NotesOperation,
) -> Result<(bool, NotesResult), InspectionError> {
    let todo_id = match &op {
        NotesOperation::CommentList { todo_id }
        | NotesOperation::CommentGet { todo_id, .. }
        | NotesOperation::CommentAdd { todo_id, .. }
        | NotesOperation::CommentUpdate { todo_id, .. }
        | NotesOperation::CommentRemove { todo_id, .. } => todo_id,
        _ => return Err(fs::error("notes_usage", "Not a comment operation")),
    };
    fs::validate_id(todo_id, 64)?;
    match &op {
        NotesOperation::CommentGet { comment_id, .. }
        | NotesOperation::CommentUpdate { comment_id, .. }
        | NotesOperation::CommentRemove { comment_id, .. } => {
            fs::validate_uuid(comment_id).map_err(|_| {
                fs::error(
                    "notes_invalid_input",
                    "Comment identifier must be a lowercase hyphenated UUID",
                )
            })?;
        }
        _ => {}
    }
    match &op {
        NotesOperation::CommentAdd { body, author, .. } => {
            validate_body(body)?;
            if let Some(author) = author {
                if author.len() > AUTHOR_MAX {
                    return Err(too_large());
                }
                if !valid_author(author) {
                    return Err(fs::error(
                        "notes_invalid_input",
                        "Author must contain no control characters",
                    ));
                }
            }
            todos::require_unique_id(dir, todo_id)?;
        }
        NotesOperation::CommentUpdate { body, .. } => validate_body(body)?,
        _ => {}
    }
    let existing_dir = comment_dir(dir, todo_id)?;
    let mut existing = match &existing_dir {
        Some(dir) => collection(dir, todo_id)?,
        None => Collection {
            records: Vec::new(),
            entries: 0,
            bytes: 0,
        },
    };
    match op {
        NotesOperation::CommentList { todo_id } => Ok((
            false,
            NotesResult::Comments {
                todo_id,
                comments: existing
                    .records
                    .into_iter()
                    .map(Record::into_comment)
                    .collect(),
            },
        )),
        NotesOperation::CommentGet { comment_id, .. } => {
            let index = find_record(&existing, &comment_id)?;
            Ok((
                false,
                NotesResult::Comment {
                    comment: existing.records.remove(index).into_comment(),
                },
            ))
        }
        NotesOperation::CommentAdd {
            todo_id,
            body,
            author,
        } => {
            let mut content = format!("---\ncreated: {}\n", fs::now());
            if let Some(author) = author {
                // Quoting prevents a label containing '#', ':', or YAML syntax
                // from becoming metadata structure or losing part of its value.
                let scalar = serde_json::to_string(&author)
                    .map_err(|error| fs::error("notes_invalid_input", &error.to_string()))?;
                content.push_str("author: ");
                content.push_str(&scalar);
                content.push('\n');
            }
            content.push_str("---\n");
            content.push_str(&body);
            if content.len() > FILE_MAX
                || existing.entries >= ENTRY_MAX
                || existing.bytes + content.len() > AGGREGATE_MAX
            {
                return Err(too_large());
            }
            if let Some(dir) = &existing_dir {
                fs::entries(dir, ENTRY_MAX - 1)?;
            }
            let destination = match existing_dir {
                Some(dir) => dir,
                None => {
                    let comments = fs::child(dir, "comments", true)?;
                    fs::child(&comments, &todo_id, true)?
                }
            };
            let id = Uuid::new_v4().to_string();
            let name = format!("{id}.md");
            let base = fs::read(&destination, &name, FILE_MAX)?;
            if base.revision != "absent" {
                return Err(fs::error(
                    "notes_conflict",
                    "Generated comment already exists",
                ));
            }
            fs::publish(&destination, &name, &base, &content, FILE_MAX)?;
            let record = parse(
                &todo_id,
                id,
                NotesDocument {
                    revision: fs::revision(content.as_bytes()),
                    content,
                },
            )?;
            Ok((
                true,
                NotesResult::Comment {
                    comment: record.into_comment(),
                },
            ))
        }
        NotesOperation::CommentUpdate {
            todo_id,
            comment_id,
            expected_revision,
            body,
        } => {
            let index = find_record(&existing, &comment_id)?;
            let record = existing.records.remove(index);
            check_revision(&record, &expected_revision)?;
            if record.unterminated {
                return Err(fs::error(
                    "notes_invalid_input",
                    "Cannot safely edit unterminated front matter",
                ));
            }
            let mut content = String::with_capacity(record.body_start + body.len());
            content.push_str(&record.document.content[..record.body_start]);
            content.push_str(&body);
            if content.len() > FILE_MAX
                || existing.bytes - record.document.content.len() + content.len() > AGGREGATE_MAX
            {
                return Err(too_large());
            }
            let changed = content != record.document.content;
            if changed {
                fs::publish(
                    existing_dir.as_ref().expect("existing comment directory"),
                    &format!("{comment_id}.md"),
                    &record.document,
                    &content,
                    FILE_MAX,
                )?;
            }
            let updated = parse(
                &todo_id,
                comment_id,
                NotesDocument {
                    revision: fs::revision(content.as_bytes()),
                    content,
                },
            )?;
            Ok((
                changed,
                NotesResult::Comment {
                    comment: updated.into_comment(),
                },
            ))
        }
        NotesOperation::CommentRemove {
            todo_id,
            comment_id,
            expected_revision,
        } => {
            let index = find_record(&existing, &comment_id)?;
            let record = existing.records.remove(index);
            check_revision(&record, &expected_revision)?;
            fs::remove(
                existing_dir.as_ref().expect("existing comment directory"),
                &format!("{comment_id}.md"),
                &record.document,
            )?;
            Ok((
                true,
                NotesResult::CommentRemoved {
                    todo_id,
                    comment_id,
                },
            ))
        }
        _ => Err(fs::error("notes_usage", "Not a comment operation")),
    }
}

fn find_record(collection: &Collection, id: &str) -> Result<usize, InspectionError> {
    collection
        .records
        .iter()
        .position(|record| record.comment.comment_id == id)
        .ok_or_else(|| fs::error("notes_not_found", "Comment not found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        path: std::path::PathBuf,
        dir: Dir,
    }

    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("cockpit-notes-comments-{}", Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
            Self { path, dir }
        }

        fn run(&self, op: NotesOperation) -> Result<(bool, NotesResult), InspectionError> {
            execute(&self.dir, &self.path, op)
        }

        fn put(&self, todo: &str, id: &str, content: &str) {
            let path = self.path.join("comments").join(todo);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join(format!("{id}.md")), content).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn comment(result: (bool, NotesResult)) -> NotesComment {
        match result.1 {
            NotesResult::Comment { comment } => comment,
            _ => panic!("Expected comment"),
        }
    }

    fn todo(result: (bool, NotesResult)) -> NotesTodo {
        match result.1 {
            NotesResult::Todo { todo, .. } => todo,
            _ => panic!("Expected todo"),
        }
    }

    #[test]
    fn comments_survive_unboard_and_todo_removal_and_remain_editable() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.path.join("todos.md"),
            "- [ ] Task <!-- cockpit id=task lane=doing -->\n",
        )
        .unwrap();
        let added = comment(
            fixture
                .run(NotesOperation::CommentAdd {
                    todo_id: "task".to_owned(),
                    body: "Durable 🧭".to_owned(),
                    author: Some("Ada: # team".to_owned()),
                })
                .unwrap(),
        );
        assert_eq!(added.author.as_deref(), Some("Ada: # team"));
        assert!(added.created.is_some());
        let (_, NotesResult::Todos { todos, .. }) = todos::execute(
            &fixture.dir,
            &fixture.path,
            NotesOperation::TodoList {
                filter: NotesTodoFilter::All,
            },
        )
        .unwrap() else {
            panic!("Expected todos");
        };
        let item = &todos[0];
        let unboarded = todo(
            todos::execute(
                &fixture.dir,
                &fixture.path,
                NotesOperation::KanbanUnboard {
                    todo: NotesTodoSelector::Id {
                        id: "task".to_owned(),
                        expected_revision: item.revision.clone(),
                    },
                },
            )
            .unwrap(),
        );
        assert_eq!(unboarded.lane, None);
        let after_unboard = comment(
            fixture
                .run(NotesOperation::CommentGet {
                    todo_id: "task".to_owned(),
                    comment_id: added.comment_id.clone(),
                })
                .unwrap(),
        );
        assert_eq!(after_unboard, added);
        todos::execute(
            &fixture.dir,
            &fixture.path,
            NotesOperation::TodoRemove {
                todo: NotesTodoSelector::Id {
                    id: "task".to_owned(),
                    expected_revision: unboarded.revision,
                },
            },
        )
        .unwrap();
        let (_, NotesResult::Comments { comments, .. }) = fixture
            .run(NotesOperation::CommentList {
                todo_id: "task".to_owned(),
            })
            .unwrap()
        else {
            panic!("Expected comments");
        };
        assert_eq!(comments, vec![added.clone()]);
        let updated = comment(
            fixture
                .run(NotesOperation::CommentUpdate {
                    todo_id: "task".to_owned(),
                    comment_id: added.comment_id.clone(),
                    expected_revision: added.revision,
                    body: "Orphan updated".to_owned(),
                })
                .unwrap(),
        );
        assert_eq!(updated.created, added.created);
        assert_eq!(updated.author, added.author);
        fixture
            .run(NotesOperation::CommentRemove {
                todo_id: "task".to_owned(),
                comment_id: updated.comment_id,
                expected_revision: updated.revision,
            })
            .unwrap();
        let (_, NotesResult::Comments { comments, .. }) = fixture
            .run(NotesOperation::CommentList {
                todo_id: "task".to_owned(),
            })
            .unwrap()
        else {
            panic!("Expected comments");
        };
        assert!(comments.is_empty());
    }

    #[test]
    fn add_requires_one_unique_todo_without_creating_directories() {
        let fixture = Fixture::new();
        let op = || NotesOperation::CommentAdd {
            todo_id: "task".to_owned(),
            body: "hi".to_owned(),
            author: None,
        };
        assert_eq!(fixture.run(op()).unwrap_err().code, "notes_not_found");
        assert!(!fixture.path.join("comments").exists());
        std::fs::write(
            fixture.path.join("todos.md"),
            "- [ ] A <!-- cockpit id=task -->\n- [ ] B <!-- cockpit id=task -->\n",
        )
        .unwrap();
        assert_eq!(fixture.run(op()).unwrap_err().code, "notes_todo_ambiguous");
        assert!(!fixture.path.join("comments").exists());
    }

    #[test]
    fn orphan_body_updates_preserve_frontmatter_exactly_and_cas() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4().to_string();
        let prefix = "---\r\ncreated: '2026-10-05T12:00:00Z'\r\nauthor: \"Ada: # team\"\r\ncustom: exact\r\n---\r\n";
        let source = format!("{prefix}Old 🧭\r\n");
        fixture.put("orphan", &id, &source);
        for op in [
            NotesOperation::CommentUpdate {
                todo_id: "orphan".to_owned(),
                comment_id: id.clone(),
                expected_revision: "stale".to_owned(),
                body: "bad".to_owned(),
            },
            NotesOperation::CommentRemove {
                todo_id: "orphan".to_owned(),
                comment_id: id.clone(),
                expected_revision: "stale".to_owned(),
            },
        ] {
            assert_eq!(fixture.run(op).unwrap_err().code, "notes_conflict");
            assert_eq!(
                std::fs::read_to_string(
                    fixture
                        .path
                        .join("comments/orphan")
                        .join(format!("{id}.md"))
                )
                .unwrap(),
                source
            );
        }
        let updated = comment(
            fixture
                .run(NotesOperation::CommentUpdate {
                    todo_id: "orphan".to_owned(),
                    comment_id: id.clone(),
                    expected_revision: fs::revision(source.as_bytes()),
                    body: "New\r\n".to_owned(),
                })
                .unwrap(),
        );
        assert_eq!(updated.created.as_deref(), Some("2026-10-05T12:00:00Z"));
        assert_eq!(updated.author.as_deref(), Some("Ada: # team"));
        assert_eq!(updated.body, "New\r\n");
        assert_eq!(
            std::fs::read_to_string(
                fixture
                    .path
                    .join("comments/orphan")
                    .join(format!("{id}.md"))
            )
            .unwrap(),
            format!("{prefix}New\r\n")
        );
    }

    #[test]
    fn imported_comments_sort_by_instant_with_unknown_last() {
        let fixture = Fixture::new();
        let ids: Vec<String> = (0..4).map(|_| Uuid::new_v4().to_string()).collect();
        fixture.put(
            "orphan",
            &ids[0],
            "---\ncreated: 2026-01-01T12:00:00+02:00\n---\nEarlier",
        );
        fixture.put(
            "orphan",
            &ids[1],
            "---\ncreated: 2026-01-01T10:30:00Z\n---\nLater",
        );
        fixture.put("orphan", &ids[2], "---\ncreated: invalid\n---\nInvalid");
        fixture.put("orphan", &ids[3], "No front matter");
        let (_, NotesResult::Comments { comments, .. }) = fixture
            .run(NotesOperation::CommentList {
                todo_id: "orphan".to_owned(),
            })
            .unwrap()
        else {
            panic!("Expected comments");
        };
        assert_eq!(comments[0].comment_id, ids[0]);
        assert_eq!(comments[1].comment_id, ids[1]);
        assert!(
            comments[2..]
                .iter()
                .all(|comment| comment.created.is_none())
        );
        assert!(comments[2].comment_id < comments[3].comment_id);
        let imported = comment(
            fixture
                .run(NotesOperation::CommentGet {
                    todo_id: "orphan".to_owned(),
                    comment_id: ids[3].clone(),
                })
                .unwrap(),
        );
        assert_eq!(imported.body, "No front matter");
        let updated = comment(
            fixture
                .run(NotesOperation::CommentUpdate {
                    todo_id: "orphan".to_owned(),
                    comment_id: ids[3].clone(),
                    expected_revision: imported.revision,
                    body: "Updated imported body".to_owned(),
                })
                .unwrap(),
        );
        assert_eq!(updated.created, None);
        assert_eq!(updated.author, None);
        assert_eq!(updated.body, "Updated imported body");
    }

    #[test]
    fn malformed_frontmatter_is_null_and_unterminated_edits_are_refused() {
        let fixture = Fixture::new();
        let id = Uuid::new_v4().to_string();
        let source = "---\ncreated: 2026-01-01T00:00:00Z\nauthor: [not scalar]\n---\nbody";
        fixture.put("orphan", &id, source);
        let imported = comment(
            fixture
                .run(NotesOperation::CommentGet {
                    todo_id: "orphan".to_owned(),
                    comment_id: id.clone(),
                })
                .unwrap(),
        );
        assert_eq!(imported.created, None);
        assert_eq!(imported.author, None);
        assert_eq!(imported.body, "body");
        let source = "---\ncreated: 2026-01-01T00:00:00Z\nmissing close";
        fixture.put("orphan", &id, source);
        assert_eq!(
            fixture
                .run(NotesOperation::CommentUpdate {
                    todo_id: "orphan".to_owned(),
                    comment_id: id.clone(),
                    expected_revision: fs::revision(source.as_bytes()),
                    body: "new".to_owned(),
                })
                .unwrap_err()
                .code,
            "notes_invalid_input"
        );
        assert_eq!(
            std::fs::read_to_string(
                fixture
                    .path
                    .join("comments/orphan")
                    .join(format!("{id}.md"))
            )
            .unwrap(),
            source
        );
    }

    #[test]
    fn payload_and_directory_limits_are_fail_closed() {
        let fixture = Fixture::new();
        for (body, author, code) in [
            ("x".repeat(BODY_MAX + 1), None, "notes_too_large"),
            (
                "body".to_owned(),
                Some("x".repeat(AUTHOR_MAX + 1)),
                "notes_too_large",
            ),
            (
                "body".to_owned(),
                Some("bad\nlabel".to_owned()),
                "notes_invalid_input",
            ),
        ] {
            assert_eq!(
                fixture
                    .run(NotesOperation::CommentAdd {
                        todo_id: "task".to_owned(),
                        body,
                        author
                    })
                    .unwrap_err()
                    .code,
                code
            );
            assert!(!fixture.path.join("comments").exists());
        }
        assert_eq!(
            fixture
                .run(NotesOperation::CommentList {
                    todo_id: "../escape".to_owned()
                })
                .unwrap_err()
                .code,
            "notes_invalid_input"
        );
        for comment_id in [
            "../escape",
            "not-a-uuid",
            "00000000000000000000000000000000",
        ] {
            assert_eq!(
                fixture
                    .run(NotesOperation::CommentGet {
                        todo_id: "orphan".to_owned(),
                        comment_id: comment_id.to_owned()
                    })
                    .unwrap_err()
                    .code,
                "notes_invalid_input"
            );
        }
        let path = fixture.path.join("comments/orphan");
        std::fs::create_dir_all(&path).unwrap();
        for index in 0..=ENTRY_MAX {
            std::fs::write(path.join(format!("ignored-{index}.txt")), "").unwrap();
        }
        assert_eq!(
            fixture
                .run(NotesOperation::CommentList {
                    todo_id: "orphan".to_owned()
                })
                .unwrap_err()
                .code,
            "notes_too_large"
        );
    }

    #[test]
    fn aggregate_limit_is_enforced_even_for_single_comment_get() {
        let fixture = Fixture::new();
        let body = "x".repeat(BODY_MAX);
        let first = Uuid::new_v4().to_string();
        fixture.put("orphan", &first, &body);
        for _ in 0..(AGGREGATE_MAX / BODY_MAX) {
            fixture.put("orphan", &Uuid::new_v4().to_string(), &body);
        }
        assert_eq!(
            fixture
                .run(NotesOperation::CommentGet {
                    todo_id: "orphan".to_owned(),
                    comment_id: first
                })
                .unwrap_err()
                .code,
            "notes_too_large"
        );
    }
}

#[cfg(all(test, unix))]
mod unsafe_path_tests {
    use super::*;

    #[test]
    fn symlinked_comment_file_is_refused() {
        let path =
            std::env::temp_dir().join(format!("cockpit-notes-comment-symlink-{}", Uuid::new_v4()));
        std::fs::create_dir_all(path.join("comments/orphan")).unwrap();
        std::fs::write(path.join("outside.md"), "do not touch").unwrap();
        let id = Uuid::new_v4().to_string();
        std::os::unix::fs::symlink(
            path.join("outside.md"),
            path.join("comments/orphan").join(format!("{id}.md")),
        )
        .unwrap();
        let dir = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        let result = execute(
            &dir,
            &path,
            NotesOperation::CommentGet {
                todo_id: "orphan".to_owned(),
                comment_id: id,
            },
        );
        assert_eq!(result.unwrap_err().code, "notes_unsafe_path");
        assert_eq!(
            std::fs::read_to_string(path.join("outside.md")).unwrap(),
            "do not touch"
        );
        std::fs::remove_dir_all(path).unwrap();
    }
}
