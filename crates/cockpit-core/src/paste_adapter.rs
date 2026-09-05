use async_trait::async_trait;
use cockpit_protocol::comment_paste::CommentPasteTarget;

use crate::InspectionError;

/// Narrow raw-byte paste capability. It deliberately does not expose ordinary terminal input.
#[async_trait]
pub trait CommentPasteAdapter: Send + Sync {
    /// Derive agent targets from a fresh raw Herdr snapshot. Missing identity evidence excludes a pane.
    async fn comment_paste_targets(
        &self,
        session_id: &str,
    ) -> Result<Vec<CommentPasteTarget>, InspectionError>;

    /// Focus the exact agent target and require Herdr's affirmative acknowledgment.
    async fn focus_comment_paste_target(
        &self,
        target: &CommentPasteTarget,
    ) -> Result<(), InspectionError>;

    /// Re-snapshot after focus and require that this exact agent pane remains
    /// the authoritative focused pane before the raw write is eligible.
    async fn confirm_comment_paste_target_focus(
        &self,
        target: &CommentPasteTarget,
    ) -> Result<(), InspectionError>;

    /// Write one already-framed bracketed-paste byte sequence through the proven raw path.
    /// A returned success means Herdr accepted the queue operation, never agent semantic completion.
    async fn send_comment_paste(
        &self,
        target: &CommentPasteTarget,
        framed_payload: &str,
    ) -> Result<(), InspectionError>;
}
