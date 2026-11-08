//! Shared voice operations; transport identity is selected by the host.
use cockpit_protocol::voice::{VoiceConnectRequest, VoiceConnectResponse, VoiceControlRequest, VoiceEventsRequest, VoiceEventsResponse};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use crate::transport::{Transport, error::OperationError, limits::check_serialized_size, operations::backend::BackendContext};
use super::{client::Client, socket::Origin};

const REQUEST_LIMIT: usize = 96 * 1024;
fn client(context: &BackendContext, transport: Transport) -> Client {
    Client::new(&context.state_root, match transport { Transport::Gateway => Origin::Web, Transport::Native => Origin::Native })
}
fn encode<T: Serialize>(request: &T) -> Result<Value, OperationError> {
    check_serialized_size(request, REQUEST_LIMIT).map_err(|_| OperationError::rejected("invalid_voice_request", "Voice request exceeds the size limit"))?;
    serde_json::to_value(request).map_err(|_| OperationError::rejected("invalid_voice_request", "Invalid voice request"))
}
fn decode<T: DeserializeOwned>(value: Value) -> Result<T, OperationError> {
    serde_json::from_value(value).map_err(|_| failure("Voice owner returned an invalid response"))
}
fn failure(message: impl Into<String>) -> OperationError {
    OperationError::Service(cockpit_core::InspectionError { code: "voice_unavailable".into(), message: message.into() })
}
pub async fn connect(context: &BackendContext, transport: Transport, request: VoiceConnectRequest) -> Result<VoiceConnectResponse, OperationError> {
    let value = client(context, transport).connect(encode(&request)?).await.map_err(|e| failure(e.to_string()))?;
    decode(value)
}
pub async fn events(context: &BackendContext, transport: Transport, request: VoiceEventsRequest) -> Result<VoiceEventsResponse, OperationError> {
    let value = client(context, transport).events(encode(&request)?).await.map_err(|e| failure(e.to_string()))?;
    decode(value)
}
pub async fn control(context: &BackendContext, transport: Transport, request: VoiceControlRequest) -> Result<Value, OperationError> {
    client(context, transport).control(encode(&request)?).await.map_err(|e| failure(e.to_string()))
}
