use std::collections::HashMap;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use cockpit_protocol::browser::{
    BrowserFeedbackImage, BrowserFeedbackImageRequest, BrowserFeedbackSendRequest,
    BrowserFeedbackSendResponse,
};
use cockpit_protocol::browser_feedback::BrowserFeedbackCapture;
use cockpit_protocol::comment_paste::{CommentPasteState, CommentPasteTarget};
use serde_json::{Value, json};

use super::{BrowserService, association_key};
use crate::InspectionError;
use crate::browser_feedback::BrowserDeliveryReceipt;

const MAX_PASTE_BYTES: usize = 64 * 1024;
const MAX_IDS: usize = 64 * 64;
const PASTE_PREFIX: &str = "\u{1b}[200~";
const PASTE_SUFFIX: &str = "\u{1b}[201~";

impl BrowserService {
    pub async fn feedback_image(
        &self,
        request: BrowserFeedbackImageRequest,
    ) -> Result<BrowserFeedbackImage, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let target = self.resolve_target(&request.target).await?;
        let key = association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.space_id,
        );
        let bytes = self.feedback.read_image(&key, &request.capture_id)?;
        Ok(BrowserFeedbackImage {
            mime_type: "image/png".to_owned(),
            data_base64: BASE64.encode(bytes),
        })
    }

    pub async fn send_feedback(
        &self,
        request: BrowserFeedbackSendRequest,
    ) -> Result<BrowserFeedbackSendResponse, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        validate_operation_id(&request.operation_id)?;
        let ids = normalized_ids(&request.ids)?;
        let target = self.resolve_target(&request.target).await?;
        let association = association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.space_id,
        );

        if let Some(existing) = self.feedback.load_delivery(&request.operation_id)? {
            if existing.association_key != association || existing.selected_ids != ids {
                return Err(InspectionError::new(
                    "browser_feedback_operation_conflict",
                    "operation ID was already used for another Space or annotation selection",
                ));
            }
            return self.recover_delivery(&association, existing).await;
        }

        if self.feedback.has_delivery_overlap(&association, &ids)?
            && !request.acknowledge_duplicate_risk
        {
            return Err(InspectionError::new(
                "browser_feedback_duplicate_risk",
                "a prior browser feedback paste outcome is unknown; inspect the terminal and explicitly acknowledge duplicate risk before retrying",
            ));
        }

        let browser = self.load(&association)?;
        let browser = browser.as_ref().map(|receipt| {
            self.association(
                receipt,
                cockpit_protocol::browser::BrowserConnectionState::Disconnected,
            )
        });
        let feedback = self.feedback.list(&association)?;
        let selected = select_pending(&feedback.captures, &ids)?;
        let payload = feedback_payload(&association, &target, browser.as_ref(), &selected, &ids)?;
        let framed = format!("{PASTE_PREFIX}{payload}{PASTE_SUFFIX}");
        if framed.as_bytes().len() > MAX_PASTE_BYTES {
            let receipt = self.persist_outcome(
                &request.operation_id,
                &association,
                &ids,
                None,
                CommentPasteState::Rejected,
                "browser feedback payload exceeds the 64 KiB framed paste limit",
            )?;
            return self.delivery_response(&association, receipt);
        }

        let adapter = self
            .paste_adapter
            .as_ref()
            .ok_or_else(|| {
                InspectionError::new(
                    "browser_feedback_unavailable",
                    "acknowledged Herdr paste is not configured",
                )
            })?
            .clone();
        let paste_target = match self.select_target(&target).await {
            Ok(Some(value)) => value,
            Ok(None) => {
                let receipt = self.persist_outcome(
                    &request.operation_id,
                    &association,
                    &ids,
                    None,
                    CommentPasteState::Rejected,
                    "no eligible agent target is available in this Space",
                )?;
                return self.delivery_response(&association, receipt);
            }
            Err(error) => {
                let receipt = self.persist_outcome(
                    &request.operation_id,
                    &association,
                    &ids,
                    None,
                    CommentPasteState::Rejected,
                    &format!(
                        "could not resolve an eligible agent target: {}",
                        error.message
                    ),
                )?;
                return self.delivery_response(&association, receipt);
            }
        };

        self.persist_outcome(
            &request.operation_id,
            &association,
            &ids,
            Some(paste_target.clone()),
            CommentPasteState::Pending,
            "browser feedback paste is pending Herdr acknowledgement",
        )?;

        if let Err(error) = adapter.focus_comment_paste_target(&paste_target).await {
            let receipt = self.persist_outcome(
                &request.operation_id,
                &association,
                &ids,
                Some(paste_target),
                CommentPasteState::Rejected,
                &format!("Herdr did not acknowledge target focus: {}", error.message),
            )?;
            return self.delivery_response(&association, receipt);
        }
        if let Err(error) = adapter
            .confirm_comment_paste_target_focus(&paste_target)
            .await
        {
            let receipt = self.persist_outcome(
                &request.operation_id,
                &association,
                &ids,
                Some(paste_target),
                CommentPasteState::Rejected,
                &format!("paste target lost confirmed focus: {}", error.message),
            )?;
            return self.delivery_response(&association, receipt);
        }
        // Revalidate authoritative Space, tab, and agent identity immediately before dispatch.
        if let Err(error) = self
            .revalidate_paste_target(&target, &paste_target, adapter.as_ref())
            .await
        {
            let receipt = self.persist_outcome(
                &request.operation_id,
                &association,
                &ids,
                Some(paste_target),
                CommentPasteState::Rejected,
                &format!("paste target changed before dispatch: {}", error.message),
            )?;
            return self.delivery_response(&association, receipt);
        }

        match adapter.send_comment_paste(&paste_target, &framed).await {
            Ok(()) => {
                let accepted = self.persist_outcome(
                    &request.operation_id,
                    &association,
                    &ids,
                    Some(paste_target),
                    CommentPasteState::Accepted,
                    "Herdr accepted one raw bracketed-paste write; no Enter was sent",
                )?;
                let ack = self.feedback.ack(&association, &ids)?;
                let mut accepted = accepted;
                accepted.acknowledged_ids = ack.acknowledged_ids;
                let accepted = self.feedback.save_delivery(accepted)?;
                self.delivery_response(&association, accepted)
            }
            Err(error) if error.code == "comments_paste_rejected" => {
                let receipt = self.persist_outcome(
                    &request.operation_id,
                    &association,
                    &ids,
                    Some(paste_target),
                    CommentPasteState::Rejected,
                    &format!(
                        "Herdr rejected the raw paste queue write: {}",
                        error.message
                    ),
                )?;
                self.delivery_response(&association, receipt)
            }
            Err(error) => {
                let receipt = self.persist_outcome(
                    &request.operation_id,
                    &association,
                    &ids,
                    Some(paste_target),
                    CommentPasteState::OutcomeUnknown,
                    &format!("paste dispatch outcome is unknown: {}", error.message),
                )?;
                self.delivery_response(&association, receipt)
            }
        }
    }

    async fn select_target(
        &self,
        target: &super::ResolvedTarget,
    ) -> Result<Option<CommentPasteTarget>, InspectionError> {
        let snapshot = self.adapter.browser_snapshot(&target.session_id).await?;
        if snapshot.endpoint_identity != target.endpoint_identity {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr endpoint changed while resolving a feedback target",
            ));
        }
        if snapshot.snapshot.focused_space_id.as_deref() != Some(target.space_id.as_str()) {
            return Err(InspectionError::new(
                "browser_feedback_space_inactive",
                "browser feedback can only be sent to the active Space",
            ));
        }
        let active_tab = snapshot.snapshot.focused_tab_id.as_deref().ok_or_else(|| {
            InspectionError::new(
                "browser_feedback_tab_inactive",
                "the active Space has no active tab",
            )
        })?;
        let adapter = self.paste_adapter.as_ref().ok_or_else(|| {
            InspectionError::new(
                "browser_feedback_unavailable",
                "acknowledged Herdr paste is not configured",
            )
        })?;
        let mut panes = HashMap::new();
        for (index, pane) in snapshot.snapshot.panes.iter().enumerate() {
            let tab_index = snapshot
                .snapshot
                .tabs
                .iter()
                .position(|tab| tab.id == pane.tab_id)
                .unwrap_or(usize::MAX);
            let space_index = snapshot
                .snapshot
                .spaces
                .iter()
                .position(|space| space.id == pane.space_id)
                .unwrap_or(usize::MAX);
            panes.insert(pane.id.clone(), (space_index, tab_index, index));
        }
        let mut candidates = adapter
            .comment_paste_targets(&target.session_id)
            .await?
            .into_iter()
            .filter(|candidate| {
                candidate.endpoint_identity == target.endpoint_identity
                    && candidate.session_id == target.session_id
                    && panes
                        .get(&candidate.pane_id)
                        .is_some_and(|(space_index, tab_index, _)| {
                            *space_index != usize::MAX
                                && snapshot.snapshot.spaces[*space_index].id == target.space_id
                                && snapshot
                                    .snapshot
                                    .tabs
                                    .get(*tab_index)
                                    .is_some_and(|tab| tab.id == active_tab)
                        })
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| {
            panes
                .get(&candidate.pane_id)
                .copied()
                .unwrap_or((usize::MAX, usize::MAX, usize::MAX))
        });
        Ok(candidates.into_iter().next())
    }
    async fn revalidate_paste_target(
        &self,
        target: &super::ResolvedTarget,
        paste_target: &CommentPasteTarget,
        adapter: &dyn crate::paste_adapter::CommentPasteAdapter,
    ) -> Result<(), InspectionError> {
        let snapshot = self.adapter.browser_snapshot(&target.session_id).await?;
        if snapshot.endpoint_identity != target.endpoint_identity
            || snapshot.endpoint_path != target.endpoint_path
        {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr endpoint changed before paste dispatch",
            ));
        }
        if snapshot.snapshot.session_id != target.session_id {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr session changed before paste dispatch",
            ));
        }
        if paste_target.session_id != target.session_id
            || paste_target.workspace_id != target.space_id
            || snapshot.snapshot.focused_space_id.as_deref() != Some(target.space_id.as_str())
        {
            return Err(InspectionError::new(
                "browser_feedback_space_inactive",
                "browser feedback Space changed before paste dispatch",
            ));
        }
        if snapshot.snapshot.focused_tab_id.as_deref() != Some(paste_target.tab_id.as_str()) {
            return Err(InspectionError::new(
                "browser_feedback_target_changed",
                "browser feedback tab changed before paste dispatch",
            ));
        }
        if snapshot.snapshot.focused_pane_id.as_deref() != Some(paste_target.pane_id.as_str()) {
            return Err(InspectionError::new(
                "browser_feedback_target_changed",
                "browser feedback agent pane changed before paste dispatch",
            ));
        }
        let pane = snapshot
            .snapshot
            .panes
            .iter()
            .find(|pane| pane.id == paste_target.pane_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "browser_feedback_target_changed",
                    "browser feedback agent pane disappeared before paste dispatch",
                )
            })?;
        if pane.space_id != target.space_id
            || pane.tab_id != paste_target.tab_id
            || pane.terminal_id != paste_target.terminal_id
        {
            return Err(InspectionError::new(
                "browser_feedback_target_changed",
                "browser feedback agent destination changed before paste dispatch",
            ));
        }
        let targets = adapter.comment_paste_targets(&target.session_id).await?;
        if !targets
            .iter()
            .any(|candidate| same_paste_target(candidate, paste_target))
        {
            return Err(InspectionError::new(
                "browser_feedback_target_changed",
                "browser feedback agent identity changed before paste dispatch",
            ));
        }
        Ok(())
    }

    async fn recover_delivery(
        &self,
        association: &str,
        mut receipt: BrowserDeliveryReceipt,
    ) -> Result<BrowserFeedbackSendResponse, InspectionError> {
        if receipt.state == CommentPasteState::Pending {
            receipt.state = CommentPasteState::OutcomeUnknown;
            receipt.message =
                "a prior browser feedback paste was interrupted before its outcome was durable"
                    .to_owned();
            receipt = self.feedback.save_delivery(receipt)?;
        }
        if receipt.state == CommentPasteState::Accepted {
            let ack = self.feedback.ack(association, &receipt.selected_ids)?;
            receipt.acknowledged_ids = merge_ids(&receipt.acknowledged_ids, &ack.acknowledged_ids);
            receipt = self.feedback.save_delivery(receipt)?;
        }
        self.delivery_response(association, receipt)
    }

    fn persist_outcome(
        &self,
        operation_id: &str,
        association: &str,
        ids: &[String],
        target: Option<CommentPasteTarget>,
        state: CommentPasteState,
        message: &str,
    ) -> Result<BrowserDeliveryReceipt, InspectionError> {
        self.feedback.save_delivery(BrowserDeliveryReceipt {
            operation_id: operation_id.to_owned(),
            association_key: association.to_owned(),
            selected_ids: ids.to_owned(),
            state,
            target,
            acknowledged_ids: Vec::new(),
            message: bounded_message(message),
            created_at: 0,
            updated_at: 0,
        })
    }

    fn delivery_response(
        &self,
        association: &str,
        receipt: BrowserDeliveryReceipt,
    ) -> Result<BrowserFeedbackSendResponse, InspectionError> {
        let pending_count = self.feedback.list(association)?.pending_count;
        Ok(BrowserFeedbackSendResponse {
            operation_id: receipt.operation_id,
            state: receipt.state,
            target: receipt.target,
            acknowledged_ids: receipt.acknowledged_ids,
            pending_count,
            message: receipt.message,
        })
    }
}

fn normalized_ids(ids: &[String]) -> Result<Vec<String>, InspectionError> {
    if ids.len() > MAX_IDS {
        return Err(InspectionError::new(
            "browser_feedback_ids",
            "feedback send contains too many annotation IDs",
        ));
    }
    let mut normalized = ids.to_owned();
    for id in &normalized {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(InspectionError::new(
                "browser_feedback_id",
                "annotation ID must be a UUID",
            ));
        }
    }
    normalized.sort();
    normalized.dedup();
    if normalized.len() != ids.len() {
        return Err(InspectionError::new(
            "browser_feedback_ids",
            "feedback send IDs must be unique",
        ));
    }

    Ok(normalized)
}
fn same_paste_target(left: &CommentPasteTarget, right: &CommentPasteTarget) -> bool {
    left.endpoint_identity == right.endpoint_identity
        && left.session_id == right.session_id
        && left.workspace_id == right.workspace_id
        && left.tab_id == right.tab_id
        && left.pane_id == right.pane_id
        && left.terminal_id == right.terminal_id
        && left.agent_fingerprint == right.agent_fingerprint
}

fn validate_operation_id(value: &str) -> Result<(), InspectionError> {
    if value.is_empty() || value.len() > 128 || value.contains(['/', '\\', '\0']) {
        return Err(InspectionError::new(
            "browser_feedback_operation",
            "operation ID is invalid",
        ));
    }
    Ok(())
}

fn strip_annotation_points(mut annotation: Value) -> Value {
    if let Value::Object(fields) = &mut annotation {
        fields.remove("points");
    }
    annotation
}

fn select_pending<'a>(
    captures: &'a [BrowserFeedbackCapture],
    ids: &[String],
) -> Result<Vec<(&'a BrowserFeedbackCapture, Vec<Value>)>, InspectionError> {
    let requested = ids
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let mut found = std::collections::HashSet::new();
    let mut selected = Vec::new();
    for capture in captures {
        let annotations = capture
            .annotations
            .iter()
            .filter(|annotation| {
                capture.pending_ids.iter().any(|id| id == &annotation.id)
                    && requested.contains(annotation.id.as_str())
            })
            .map(serde_json::to_value)
            .map(|result| result.map(strip_annotation_points))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| InspectionError::new("browser_feedback_payload", error.to_string()))?;
        if !annotations.is_empty() {
            found.extend(annotations.iter().filter_map(|annotation| {
                annotation
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }));
            selected.push((capture, annotations));
        }
    }
    if found.len() != ids.len() {
        return Err(InspectionError::new(
            "browser_feedback_pending",
            "one or more requested annotation IDs are not pending in this Space",
        ));
    }
    Ok(selected)
}

fn feedback_payload(
    association: &str,
    target: &super::ResolvedTarget,
    browser: Option<&cockpit_protocol::browser::BrowserAssociation>,
    captures: &[(&BrowserFeedbackCapture, Vec<Value>)],
    ids: &[String],
) -> Result<String, InspectionError> {
    let captures = captures
        .iter()
        .map(|(capture, annotations)| {
            json!({
                "capture_id": capture.id,
                "context": capture.context,
                "page": capture.page,
                "image_path": capture.image_path,
                "annotations": annotations,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({
        "kind": "browser_feedback",
        "addressing": {
            "association_key": association,
            "session_id": target.session_id,
            "space_id": target.space_id,
            "space_label": target.space_label,
            "endpoint_identity": target.endpoint_identity,
            "playwright_session": browser.map(|value| value.playwright_session.as_str()).unwrap_or(""),
            "working_directory": browser.map(|value| value.working_directory.as_str()).unwrap_or(""),
            "invocation": browser.map(|value| value.invocation.as_str()).unwrap_or(""),
        },
        "instructions": [
            "Open Cockpit's Browser feedback view to review, acknowledge, and send these annotations.",
            "If you are using a terminal, the companion Cockpit CLI is `cockpit-cli`; run `cockpit-cli browser status --current` to refresh browser addressing.",
            "Use `cockpit-cli browser feedback --current` to read pending feedback; after reviewing images, run `cockpit-cli browser feedback ack --current --id <annotation-id>` for the exact reviewed IDs.",
        ],
        "requested_annotation_ids": ids,
        "captures": captures,
    }))
    .map_err(|error| InspectionError::new("browser_feedback_payload", error.to_string()))
}

fn merge_ids(existing: &[String], additional: &[String]) -> Vec<String> {
    let mut ids = existing.to_owned();
    ids.extend_from_slice(additional);
    ids.sort();
    ids.dedup();
    ids
}

fn bounded_message(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control())
        .take(8 * 1024)
        .collect()
}
#[cfg(test)]
mod tests {
    use super::strip_annotation_points;
    use serde_json::json;

    #[test]
    fn strip_annotation_points_preserves_feedback_metadata() {
        let annotation = json!({
            "id": "annotation-1",
            "kind": "freehand",
            "comment": "Needs work",
            "color": "#ff0000",
            "bounds": {"x": 1, "y": 2, "width": 3, "height": 4},
            "element": {"tag": "button", "text": "Save"},
            "points": [{"x": 1, "y": 2}, {"x": 3, "y": 4}],
        });

        let stripped = strip_annotation_points(annotation);

        assert_eq!(stripped["id"], "annotation-1");
        assert_eq!(stripped["comment"], "Needs work");
        assert_eq!(stripped["bounds"]["width"], 3);
        assert_eq!(stripped["element"]["text"], "Save");
        assert!(stripped.get("points").is_none());
    }
    #[test]
    fn pending_feedback_selection_omits_points_for_agent_payload() {
        use cockpit_protocol::browser_feedback::{
            BrowserAnnotation, BrowserAnnotationKind, BrowserCaptureContext,
            BrowserFeedbackCapture, BrowserPageEvidence, BrowserViewport,
        };

        let capture = BrowserFeedbackCapture {
            id: "capture-1".to_owned(),
            context: BrowserCaptureContext {
                association_key: "association".to_owned(),
                session_id: "session".to_owned(),
                space_id: "space".to_owned(),
                space_label: "Space".to_owned(),
                playwright_session: "playwright".to_owned(),
                working_directory: "/tmp".to_owned(),
                invocation: "open".to_owned(),
                browser_instance: "browser".to_owned(),
                inline_provenance: None,
            },
            page: BrowserPageEvidence {
                url: "https://example.test".to_owned(),
                title: "Example".to_owned(),
                tab_id: Some(1),
                document_id: "document".to_owned(),
                captured_at: "2026-09-09T00:00:00Z".to_owned(),
                viewport: BrowserViewport {
                    width: 100.0,
                    height: 100.0,
                    scroll_x: 0.0,
                    scroll_y: 0.0,
                    device_pixel_ratio: 1.0,
                    visual_scale: 1.0,
                },
                image_width: 100,
                image_height: 100,
            },
            annotations: vec![BrowserAnnotation {
                id: "annotation-1".to_owned(),
                kind: BrowserAnnotationKind::Freehand,
                comment: "Needs work".to_owned(),
                color: "#ff0000".to_owned(),
                points: vec![
                    cockpit_protocol::browser_feedback::BrowserPoint { x: 1.0, y: 2.0 },
                    cockpit_protocol::browser_feedback::BrowserPoint { x: 3.0, y: 4.0 },
                ],
                bounds: None,
                element: None,
            }],
            pending_ids: vec!["annotation-1".to_owned()],
            image_path: "/tmp/capture.png".to_owned(),
        };

        let captures = [capture];
        let ids = ["annotation-1".to_owned()];
        let selected = super::select_pending(&captures, &ids).unwrap();
        let annotation = &selected[0].1[0];
        assert_eq!(annotation["comment"], "Needs work");
        assert!(annotation.get("points").is_none());
    }
}
