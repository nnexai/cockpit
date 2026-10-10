use cockpit_protocol::{
    browser::{
        BrowserCleanupRetryRequest, BrowserCleanupStatus, BrowserFeedbackAckRequest,
        BrowserFeedbackImage, BrowserFeedbackImageRequest, BrowserFeedbackLookup,
        BrowserFeedbackRequest, BrowserFeedbackSendRequest, BrowserFeedbackSendResponse,
        BrowserRequest, BrowserResponse,
    },
    browser_feedback::BrowserFeedbackAck,
    browser_view::{
        BrowserDraftRecoveryRequest, BrowserViewCommandOutcome, BrowserViewCommandRequest,
        BrowserViewCommandResponse, BrowserViewFrameDescriptor, BrowserViewOpenRequest,
        BrowserViewSnapshot,
    },
};
use serde::Serialize;

use crate::{
    BrowserRuntime,
    browser_runtime::validate_browser_view_command,
    transport::{Transport, error::OperationError},
};

#[derive(Clone, Debug, Serialize)]
pub struct BrowserViewOpenResponse {
    pub snapshot: BrowserViewSnapshot,
    pub first_frame: BrowserViewFrameDescriptor,
    /// Always a gateway route. The private helper endpoint never crosses this seam.
    pub frame_endpoint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BrowserViewOpenNativeResponse {
    pub snapshot: BrowserViewSnapshot,
    pub first_frame: BrowserViewFrameDescriptor,
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum BrowserViewOpened {
    Gateway(BrowserViewOpenResponse),
    Native(BrowserViewOpenNativeResponse),
}

/// Input-stage validation must precede the gateway's runtime guard.
pub fn validate_view_open(request: &BrowserViewOpenRequest) -> Result<(), OperationError> {
    request
        .validate()
        .map_err(|message| OperationError::rejected("invalid_browser_view_open", message))
}

/// Input-stage validation must precede the gateway's runtime guard.
pub fn validate_view_command(request: &BrowserViewCommandRequest) -> Result<(), OperationError> {
    validate_browser_view_command(request)
        .map_err(|error| OperationError::rejected(&error.code, error.message))
}

pub async fn browser_action(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: BrowserRequest,
) -> Result<BrowserResponse, OperationError> {
    runtime.execute(request).await.map_err(Into::into)
}

pub async fn browser_feedback(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: BrowserFeedbackRequest,
) -> Result<BrowserFeedbackLookup, OperationError> {
    runtime.feedback(request).await.map_err(Into::into)
}

pub async fn browser_feedback_ack(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: BrowserFeedbackAckRequest,
) -> Result<BrowserFeedbackAck, OperationError> {
    runtime
        .acknowledge_feedback(request)
        .await
        .map_err(Into::into)
}

pub async fn browser_feedback_image(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: BrowserFeedbackImageRequest,
) -> Result<BrowserFeedbackImage, OperationError> {
    runtime.feedback_image(request).await.map_err(Into::into)
}

pub async fn browser_feedback_send(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: BrowserFeedbackSendRequest,
) -> Result<BrowserFeedbackSendResponse, OperationError> {
    runtime.send_feedback(request).await.map_err(Into::into)
}

pub async fn browser_cleanup_status(
    runtime: &BrowserRuntime,
    _transport: Transport,
    (): (),
) -> Result<BrowserCleanupStatus, OperationError> {
    runtime.cleanup_status().await.map_err(Into::into)
}

pub async fn browser_cleanup_retry(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: BrowserCleanupRetryRequest,
) -> Result<BrowserCleanupStatus, OperationError> {
    runtime.retry_cleanup(request).await.map_err(Into::into)
}

pub async fn browser_view_open(
    runtime: &BrowserRuntime,
    transport: Transport,
    request: BrowserViewOpenRequest,
) -> Result<BrowserViewOpened, OperationError> {
    let opened = runtime.open_browser_view(request).await?;
    Ok(match transport {
        Transport::Gateway => {
            let frame_endpoint = format!(
                "/api/v1/browser/view/frame/{}",
                opened.snapshot.identity.view_id,
            );
            BrowserViewOpened::Gateway(BrowserViewOpenResponse {
                snapshot: opened.snapshot,
                first_frame: opened.first_frame,
                frame_endpoint,
            })
        }
        Transport::Native => BrowserViewOpened::Native(BrowserViewOpenNativeResponse {
            snapshot: opened.snapshot,
            first_frame: opened.first_frame,
        }),
    })
}

pub async fn browser_draft_recovery(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: BrowserDraftRecoveryRequest,
) -> Result<BrowserViewCommandOutcome, OperationError> {
    runtime
        .browser_draft_recovery(request)
        .await
        .map_err(Into::into)
}

pub async fn browser_view_command(
    runtime: &BrowserRuntime,
    _transport: Transport,
    request: BrowserViewCommandRequest,
) -> Result<BrowserViewCommandResponse, OperationError> {
    runtime
        .browser_view_command(request)
        .await
        .map_err(Into::into)
}

pub async fn browser_view_release(
    runtime: &BrowserRuntime,
    _transport: Transport,
    view_id: String,
) -> Result<(), OperationError> {
    runtime.browser_view_detach(&view_id).await?;
    Ok(())
}
