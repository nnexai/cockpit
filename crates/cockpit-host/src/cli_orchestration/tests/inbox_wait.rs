use cockpit_protocol::orchestration::{DeliveryStage, MessageKind, Report, ReportKind};
use super::{message, super::{output::capture, wait::{inbox_counts, meaningful}}};

#[test]
fn wake_envelope_contains_only_counts_not_bodies_or_other_runs() {
    let mut messages = vec![
        message("own", 1, MessageKind::Instruction, DeliveryStage::Stored, "BELOW CURSOR"),
        message("own", 2, MessageKind::Report, DeliveryStage::Read, "PRIVATE REPORT"),
        message("own", 3, MessageKind::Report, DeliveryStage::Stored, "PRIVATE REPORT 2"),
        message("own", 4, MessageKind::Instruction, DeliveryStage::Acked, "ACKED"),
        message("other", 9, MessageKind::Instruction, DeliveryStage::Stored, "OTHER RUN"),
    ];
    for item in &mut messages[1..=2] {
        item.report = Some(Report { message_id: item.message_id.clone(), kind: ReportKind::Result,
            outcome: None, summary: String::new(), plan: None, source_output: None, at: item.created_at.clone() });
    }
    let result = inbox_counts(&messages, "own", 1);
    assert!(result.pending);
    assert_eq!(result.through_seq, 3);
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["counts"], serde_json::json!([{"kind":"report","count":2}]));
    assert_eq!(value.as_object().unwrap().len(), 4);
    assert!(!value.to_string().contains("PRIVATE"));
    assert!(!inbox_counts(&messages, "own", 3).pending);
    assert_eq!(messages[1].stage, DeliveryStage::Read);
}

#[test]
fn newer_progress_coalesces_without_hiding_questions() {
    let mut older = message("own", 1, MessageKind::Report, DeliveryStage::Stored, "old progress");
    older.report = Some(Report {
        message_id: older.message_id.clone(), kind: ReportKind::Progress, outcome: None,
        summary: "old".into(), plan: None, source_output: None, at: older.created_at.clone(),
    });
    let mut newer = older.clone();
    newer.seq = 3;
    newer.message_id = "new-progress".into();
    let mut question = older.clone();
    question.seq = 2;
    question.report.as_mut().unwrap().kind = ReportKind::NeedsInput;
    let counts = inbox_counts(&[older, question, newer], "own", 0);
    assert!(counts.pending);
    assert_eq!(counts.through_seq, 2);
    assert_eq!(counts.counts[0].count, 1);
}

#[test]
fn meaningful_filters_housekeeping_without_hiding_decisions() {
    for kind in [ReportKind::Progress, ReportKind::Initialized, ReportKind::NeedsInput, ReportKind::Result] {
        let mut item = message("own", 1, MessageKind::Report, DeliveryStage::Stored, "Progress text is not a type");
        item.report = Some(Report {
            message_id: item.message_id.clone(), kind, outcome: None,
            summary: "Result text is not a type".into(), plan: None, source_output: None,
            at: item.created_at.clone(),
        });
        assert_eq!(meaningful(&item), matches!(kind, ReportKind::NeedsInput | ReportKind::Result));
    }
    for kind in [MessageKind::Instruction, MessageKind::Answer, MessageKind::CancelRequest, MessageKind::Observation] {
        assert!(meaningful(&message("own", 1, kind, DeliveryStage::Stored, "progress initialized")));
    }
    for id in ["cleanup:resource:integration_pending", "scope-cleanup:run:1:scope_dirty_checkout",
        "scheduled-start:task", "dispatch-recovered:run:1", "observation:run:endpoint_changed:1:1"] {
        let mut item = message("own", 1, MessageKind::Observation, DeliveryStage::Stored, "retained");
        item.from = cockpit_protocol::orchestration::ActorRef::Dispatcher;
        item.message_id = id.into();
        assert!(!meaningful(&item), "{id}");
        assert!(!inbox_counts(&[item], "own", 0).pending, "{id}");
    }
    for id in ["dispatch-failure:run:1:project_space_open", "observation:run:exited_without_report:1:1",
        "observation:run:runtime_blocked:1:1", "observation:run:idle_without_report:1:1"] {
        let mut item = message("own", 1, MessageKind::Observation, DeliveryStage::Stored, "decision");
        item.from = cockpit_protocol::orchestration::ActorRef::Dispatcher;
        item.message_id = id.into();
        assert!(meaningful(&item), "{id}");
        assert!(inbox_counts(&[item], "own", 0).pending, "{id}");
    }
}

fn private_capture_directory() -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("cockpit-cli-capture-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

#[test]
fn capture_keeps_exact_json_messages_and_recipient_prefix_private() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = private_capture_directory();
    let path = root.join("batch.json");
    let value = serde_json::json!({
        "messages": [
            message("own", 2, MessageKind::Instruction, DeliveryStage::Read, "Exact\nUnicode λ"),
            message("own", 3, MessageKind::Answer, DeliveryStage::Read, "Untrusted \"body\""),
        ],
        "through_sequence": 3,
        "pending_count": 1,
        "counts": [{"kind": "report", "count": 1}],
    });
    capture(&path, &value).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes, serde_json::to_vec(&value).unwrap());
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(), value);
    let metadata = std::fs::metadata(&path).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(metadata.uid(), nix::unistd::Uid::current().as_raw());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "no temporary spool is retained");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capture_never_clobbers_existing_files_or_follows_symlinks() {
    use std::os::unix::fs::symlink;
    let root = private_capture_directory();
    let destination = root.join("existing.json");
    std::fs::write(&destination, b"retained batch").unwrap();
    let alias = root.join("alias.json");
    symlink(&destination, &alias).unwrap();
    let missing_target = root.join("missing.json");
    let dangling = root.join("dangling.json");
    symlink(&missing_target, &dangling).unwrap();
    for path in [&destination, &alias, &dangling] {
        assert_eq!(capture(path, &serde_json::json!({"messages": []})).unwrap_err().code, "inbox_capture");
    }
    assert_eq!(std::fs::read(&destination).unwrap(), b"retained batch");
    assert_eq!(std::fs::read_link(&alias).unwrap(), destination);
    assert_eq!(std::fs::read_link(&dangling).unwrap(), missing_target);
    assert!(!missing_target.exists());
    let parent_alias = root.join("parent-alias");
    symlink(&root, &parent_alias).unwrap();
    assert_eq!(capture(&parent_alias.join("new.json"), &()).unwrap_err().code, "inbox_capture");
    assert!(!root.join("new.json").exists());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 4, "failed captures remove temporary files");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capture_rejects_public_or_missing_parent_before_creation() {
    use std::os::unix::fs::PermissionsExt;
    let root = private_capture_directory();
    let path = root.join("mail.json");
    for mode in [0o755, 0o710, 0o701] {
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(capture(&path, &()).unwrap_err().code, "inbox_capture");
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(capture(&root.join("absent/mail.json"), &()).unwrap_err().code, "inbox_capture");
    assert_eq!(capture(std::path::Path::new("mail.json"), &()).unwrap_err().code, "orchestration_usage");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capture_bounds_exact_serialized_json_not_unencoded_text() {
    const LIMIT: usize = 4 * 1024 * 1024;
    let root = private_capture_directory();
    let exact = "a".repeat(LIMIT - 2);
    let path = root.join("exact.json");
    capture(&path, &exact).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len(), LIMIT);
    assert_eq!(serde_json::from_slice::<String>(&bytes).unwrap(), exact);
    let overflow = root.join("overflow.json");
    assert_eq!(capture(&overflow, &"a".repeat(LIMIT - 1)).unwrap_err().code, "inbox_capture");
    assert!(!overflow.exists());
    let escaped = root.join("escaped.json");
    assert_eq!(capture(&escaped, &"\n".repeat(LIMIT / 2)).unwrap_err().code, "inbox_capture");
    assert!(!escaped.exists(), "JSON escaping counts toward the byte bound");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
