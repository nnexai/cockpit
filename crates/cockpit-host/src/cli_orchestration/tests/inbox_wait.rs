use cockpit_protocol::orchestration::{DeliveryStage, MessageKind, Report, ReportKind};
use super::{message, super::wait::inbox_counts};

#[test]
fn wake_envelope_contains_only_counts_not_bodies_or_other_runs() {
    let messages = vec![
        message("own", 1, MessageKind::Instruction, DeliveryStage::Stored, "BELOW CURSOR"),
        message("own", 2, MessageKind::Report, DeliveryStage::Read, "PRIVATE REPORT"),
        message("own", 3, MessageKind::Report, DeliveryStage::Stored, "PRIVATE REPORT 2"),
        message("own", 4, MessageKind::Instruction, DeliveryStage::Acked, "ACKED"),
        message("other", 9, MessageKind::Instruction, DeliveryStage::Stored, "OTHER RUN"),
    ];
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
    assert_eq!(counts.through_seq, 3);
    assert_eq!(counts.counts[0].count, 2);
}
