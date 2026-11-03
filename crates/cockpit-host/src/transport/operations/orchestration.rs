use axum::{
    Json,
    extract::{
        Path, Query,
        rejection::{JsonRejection, QueryRejection},
    },
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use cockpit_protocol::orchestration::{
    AgentSendTarget, InboxRequest, InboxView,
    OperatorOrigin, OrchestrationMutationRequest, OrchestrationMutationResponse,
    OrchestrationSnapshot, OrchestrationSnapshotRequest, OrchestrationWaitRequest,
    OrchestrationWaitResponse,
    LiaisonLookupRequest, LiaisonLookupResponse,
};
use cockpit_protocol::orchestration::{MessageImage, OrchestrationImageMedia, OrchestrationImagePut, OrchestrationImageRead};
use serde::Deserialize;

use crate::{
    OrchestrationRuntime,
    transport::{
        Transport,
        error::OperationError,
        http_input::{self, Reject},
    },
};

/// Installed command-line contract. This does not construct or contact a runtime.
/// Native GUI startup must not answer this as a GUI transport.
pub fn orchestration_protocol() -> serde_json::Value {
    serde_json::json!({
        "orchestration_protocol": 4,
        "store_schema": 4,
        "client_role": "host",
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotQuery {
    pub root_id: Option<String>,
}

type SnapshotInput = (Path<String>, Result<Query<SnapshotQuery>, QueryRejection>);

pub fn snapshot_input() -> impl FnOnce(SnapshotInput) -> Result<OrchestrationSnapshotRequest, OperationError> {
    |(path, query)| {
        let session_id =
            http_input::session(Reject("invalid_session_id", "Session ID is invalid"))((path,))?;
        let query: SnapshotQuery = http_input::mapped_query(Reject(
            "invalid_orchestration_request",
            "Invalid orchestration snapshot query",
        ))((query,))?;
        Ok(OrchestrationSnapshotRequest {
            session_id,
            root_id: query.root_id,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboxQuery {
    pub session_id: Option<String>,
}

type InboxInput = (Path<String>, Result<Query<InboxQuery>, QueryRejection>);

pub fn inbox_input() -> impl FnOnce(InboxInput) -> Result<InboxRequest, OperationError> {
    |(path, query)| {
        let session_id =
            http_input::session(Reject("invalid_session_id", "Session ID is invalid"))((path,))?;
        let query: InboxQuery = http_input::mapped_query(Reject(
            "invalid_orchestration_request",
            "Invalid orchestration inbox query",
        ))((query,))?;
        if query.session_id.as_ref().is_some_and(|id| id != &session_id) {
            return Err(OperationError::rejected(
                "session_mismatch",
                "Path and request session IDs differ",
            ));
        }
        Ok(InboxRequest { session_id })
    }
}

type MutationInput = (
    Path<String>,
    Result<Json<OrchestrationMutationRequest>, JsonRejection>,
);

pub fn mutation_input() -> impl FnOnce(MutationInput) -> Result<OrchestrationMutationRequest, OperationError> {
    |(path, body)| {
        let (session_id, request): (String, OrchestrationMutationRequest) =
            http_input::session_body(
                Reject("invalid_session_id", "Session ID is invalid"),
                Reject(
                    "invalid_orchestration_request",
                    "Expected a bounded JSON orchestration request with valid fields",
                ),
            )((path, body))?;
        if request.session_id != session_id {
            return Err(OperationError::rejected(
                "session_mismatch",
                "Path and request session IDs differ",
            ));
        }
        Ok(request)
    }
}

type ImagePutInput = (Path<String>, Result<Json<OrchestrationImagePut>, JsonRejection>);
type ImageReadInput = (Path<String>, Result<Json<OrchestrationImageRead>, JsonRejection>);

pub fn image_put_input() -> impl FnOnce(ImagePutInput) -> Result<OrchestrationImagePut, OperationError> {
    |(path, body)| {
        let (session_id, request): (String, OrchestrationImagePut) = http_input::session_sized_body(
            Reject("invalid_session_id", "Session ID is invalid"),
            Reject("invalid_orchestration_request", "Expected a bounded JSON orchestration request with valid fields"),
        )((path, body))?;
        if request.session_id != session_id {
            return Err(OperationError::rejected("session_mismatch", "Path and request session IDs differ"));
        }
        Ok(request)
    }
}

pub fn image_read_input() -> impl FnOnce(ImageReadInput) -> Result<OrchestrationImageRead, OperationError> {
    |(path, body)| {
        let (session_id, request): (String, OrchestrationImageRead) = http_input::session_sized_body(
            Reject("invalid_session_id", "Session ID is invalid"),
            Reject("invalid_orchestration_request", "Expected a bounded JSON orchestration request with valid fields"),
        )((path, body))?;
        if request.session_id != session_id {
            return Err(OperationError::rejected("session_mismatch", "Path and request session IDs differ"));
        }
        Ok(request)
    }
}

type LiaisonLookupInput = (Path<String>, Result<Json<LiaisonLookupRequest>, JsonRejection>);

pub fn liaison_lookup_input() -> impl FnOnce(LiaisonLookupInput) -> Result<LiaisonLookupRequest, OperationError> {
    |(path, body)| {
        let (session_id, request): (String, LiaisonLookupRequest) =
            http_input::session_body(
                Reject("invalid_session_id", "Session ID is invalid"),
                Reject("invalid_orchestration_request", "Expected a liaison lookup request"),
            )((path, body))?;
        if request.session_id != session_id {
            return Err(OperationError::rejected(
                "session_mismatch", "Path and request session IDs differ",
            ));
        }
        Ok(request)
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSendRequest {
    pub session_id: String,
    pub reference: String,
    pub text: String,
    #[serde(default)]
    pub expected_target: Option<AgentSendTarget>,
}

type AgentSendInput = (Path<String>, Result<Json<AgentSendRequest>, JsonRejection>);

pub fn agent_send_input() -> impl FnOnce(AgentSendInput) -> Result<AgentSendRequest, OperationError> {
    |(path, body)| {
        let (session_id, request): (String, AgentSendRequest) = http_input::session_body(
            Reject("invalid_session_id", "Session ID is invalid"),
            Reject("invalid_orchestration_request", "Expected an agent send request"),
        )((path, body))?;
        if request.session_id != session_id {
            return Err(OperationError::rejected("session_mismatch", "Path and request session IDs differ"));
        }
        Ok(request)
    }
}

pub async fn orchestration_agent_send(
    runtime: &OrchestrationRuntime, transport: Transport, request: AgentSendRequest,
) -> Result<serde_json::Value, OperationError> {
    let origin = match transport {
        Transport::Gateway => OperatorOrigin::Browser,
        Transport::Native => OperatorOrigin::Native,
    };
    runtime.send_agent(&request.session_id, &request.reference, request.text, origin, request.expected_target).await.map_err(Into::into)
}


pub async fn orchestration_liaison_lookup(
    runtime: &OrchestrationRuntime,
    _transport: Transport,
    request: LiaisonLookupRequest,
) -> Result<LiaisonLookupResponse, OperationError> {
    runtime.liaison_lookup(&request).await.map_err(Into::into)
}

pub async fn orchestration_snapshot(
    runtime: &OrchestrationRuntime,
    _transport: Transport,
    request: OrchestrationSnapshotRequest,
) -> Result<OrchestrationSnapshot, OperationError> {
    runtime.snapshot(&request).await.map_err(Into::into)
}

fn validate_inbox_session(request: &InboxRequest) -> Result<(), OperationError> {
    if !crate::transport::guard::valid_session_id(&request.session_id) {
        return Err(OperationError::rejected("invalid_session_id", "Session ID is invalid"));
    }
    Ok(())
}

pub async fn orchestration_inbox(
    runtime: &OrchestrationRuntime,
    _transport: Transport,
    request: InboxRequest,
) -> Result<InboxView, OperationError> {
    validate_inbox_session(&request)?;
    runtime.inbox(&request).await.map_err(Into::into)
}

pub async fn orchestration_mutate(
    runtime: &OrchestrationRuntime,
    transport: Transport,
    request: OrchestrationMutationRequest,
) -> Result<OrchestrationMutationResponse, OperationError> {
    // The route is protected by an exact Origin guard; no wire field chooses the actor.
    // Only the main native GUI capability exposes this command. Caller JSON never selects an actor.
    let origin = match transport {
        Transport::Gateway => OperatorOrigin::Browser,
        Transport::Native => OperatorOrigin::Native,
    };
    runtime.mutate(origin, request).await.map_err(Into::into)
}

pub async fn orchestration_image_put(
    runtime: &OrchestrationRuntime,
    _transport: Transport,
    request: OrchestrationImagePut,
) -> Result<MessageImage, OperationError> {
    if !crate::transport::guard::valid_session_id(&request.session_id) {
        return Err(OperationError::rejected("invalid_session_id", "Session ID is invalid"));
    }
    runtime.service.validate_image_session(&request.session_id)?;
    let bytes = decode_image(&request.data_base64)?;
    runtime.service.image_put(&request.session_id, &bytes).map_err(Into::into)
}

fn decode_image(data: &str) -> Result<Vec<u8>, OperationError> {
    let max = cockpit_core::orchestration::MAX_IMAGE_BYTES as usize;
    if data.len() > max.div_ceil(3) * 4 {
        return Err(OperationError::rejected("image_too_large", "Image exceeds the byte limit"));
    }
    BASE64.decode(data).map_err(|_| {
        OperationError::rejected("invalid_orchestration_request", "Image data is not valid base64")
    })
}

pub async fn orchestration_image_read(
    runtime: &OrchestrationRuntime,
    _transport: Transport,
    request: OrchestrationImageRead,
) -> Result<OrchestrationImageMedia, OperationError> {
    if !crate::transport::guard::valid_session_id(&request.session_id) {
        return Err(OperationError::rejected("invalid_session_id", "Session ID is invalid"));
    }
    runtime.service.validate_image_session(&request.session_id)?;
    runtime.service.image_for_session(&request.session_id, &request.image_id).map_err(Into::into)
}

pub async fn orchestration_wait(
    runtime: &OrchestrationRuntime,
    _transport: Transport,
    request: OrchestrationWaitRequest,
) -> Result<OrchestrationWaitResponse, OperationError> {
    // No service mutex is held across the long poll.
    runtime.wait(&request).await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_inputs_fence_sessions_and_decode_bounded_base64() {
        let put = OrchestrationImagePut { session_id: "other".into(), data_base64: "AA==".into() };
        let error: cockpit_protocol::v1::ErrorResponse =
            image_put_input()((Path("session".into()), Ok(Json(put)))).unwrap_err().into();
        assert_eq!(error.code, "session_mismatch");
        let read = OrchestrationImageRead { session_id: "session".into(), image_id: "id".into() };
        assert!(image_read_input()((Path("session".into()), Ok(Json(read)))).is_ok());
        assert_eq!(decode_image("AA==").unwrap(), vec![0]);
        let error: cockpit_protocol::v1::ErrorResponse = decode_image("!").unwrap_err().into();
        assert_eq!(error.code, "invalid_orchestration_request");
        let encoded_limit = (cockpit_core::orchestration::MAX_IMAGE_BYTES as usize).div_ceil(3) * 4;
        let error: cockpit_protocol::v1::ErrorResponse = decode_image(&"A".repeat(encoded_limit + 1)).unwrap_err().into();
        assert_eq!(error.code, "image_too_large");
    }

    #[tokio::test]
    async fn image_handlers_put_png_and_refuse_unreferenced_read() {
        use std::sync::Arc;
        use cockpit_core::{library::LibraryService, projects::ProjectService, sources::SourceService};
        use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
        use cockpit_protocol::projects::ProjectConfiguration;

        fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            png.extend_from_slice(kind);
            png.extend_from_slice(data);
            let mut crc = !0u32;
            for byte in kind.iter().chain(data) {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
                }
            }
            png.extend_from_slice(&(!crc).to_be_bytes());
        }
        let root = std::env::temp_dir().join(format!("cockpit-image-transport-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let config = ProjectConfiguration::for_tests(&root);
        let sources = Arc::new(SourceService::new(&config, Vec::new()).unwrap());
        let library = Arc::new(LibraryService::new(config.clone(), sources));
        let herdr = Arc::new(HerdrCliAdapter::new(HerdrCliConfig::from_options_with_environment(
            Some(root.join("not-installed-herdr")), Some("fixture".into()),
            Some(root.join("not-running.sock")), &Default::default(),
        ).unwrap()));
        let projects = Arc::new(ProjectService::new(config.clone(), herdr.clone()).unwrap());
        let runtime = OrchestrationRuntime::new(&config, herdr, projects, library,
            Some(root.join("config.toml")), crate::orchestration_runtime::CliProcessRole::Host).unwrap();
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut png, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
        chunk(&mut png, b"IDAT", &[0x78, 0x01, 0x01, 4, 0, 0xfb, 0xff, 0, 0, 0, 0, 0, 1, 0, 1]);
        chunk(&mut png, b"IEND", &[]);
        for transport in [Transport::Gateway, Transport::Native] {
            let bad = OrchestrationImagePut { session_id: "session".into(), data_base64: "!".into() };
            let error: cockpit_protocol::v1::ErrorResponse = orchestration_image_put(&runtime, transport, bad).await.unwrap_err().into();
            assert_eq!(error.code, "invalid_orchestration_request");
            let put = OrchestrationImagePut { session_id: "session".into(), data_base64: BASE64.encode(&png) };
            let image = orchestration_image_put(&runtime, transport, put).await.unwrap();
            assert_eq!((image.mime_type.as_str(), image.width, image.height), ("image/png", 1, 1));
            let read = OrchestrationImageRead { session_id: "session".into(), image_id: image.image_id };
            let error: cockpit_protocol::v1::ErrorResponse = orchestration_image_read(&runtime, transport, read).await.unwrap_err().into();
            assert_eq!(error.code, "image_not_found");
        }
        drop(runtime);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn agent_send_decodes_optional_reviewed_target_for_both_transports() {
        let body = serde_json::json!({
            "session_id":"session", "reference":"pane", "text":"hello",
            "expected_target":{"endpoint_identity":"endpoint", "workspace_id":"space",
                "tab_id":"tab", "pane_id":"pane", "terminal_id":"terminal", "agent_kind":"omp",
                "agent_name":null, "native_session_id":null, "native_session_file":null,
                "state_changed_at":null}
        });
        let request: AgentSendRequest = serde_json::from_value(body.clone()).unwrap();
        let request = agent_send_input()((Path("session".into()), Ok(Json(request)))).unwrap();
        assert_eq!(request.expected_target.unwrap().terminal_id, "terminal");
        let decode = crate::transport::native_input::value::<AgentSendRequest>(
            crate::transport::limits::MUTATION_BYTES, "orchestration",
        );
        assert_eq!(decode((body,)).unwrap().expected_target.unwrap().pane_id, "pane");
        let legacy: AgentSendRequest = serde_json::from_value(serde_json::json!({
            "session_id":"session", "reference":"pane", "text":"hello"
        })).unwrap();
        assert!(legacy.expected_target.is_none());
    }

    #[test]
    fn inbox_input_uses_path_and_rejects_mismatched_session() {
        let input = |path: &str, session_id: Option<&str>| {
            inbox_input()((
                Path(path.into()),
                Ok(Query(InboxQuery { session_id: session_id.map(str::to_owned) })),
            ))
        };
        assert_eq!(input("session", None).unwrap().session_id, "session");
        assert_eq!(input("session", Some("session")).unwrap().session_id, "session");
        let error: cockpit_protocol::v1::ErrorResponse =
            input("session", Some("other")).unwrap_err().into();
        assert_eq!(error.code, "session_mismatch");
        assert!(input("bad:id", None).is_err());
        assert!(serde_json::from_value::<InboxQuery>(serde_json::json!({"unexpected": true})).is_err());
    }

    #[test]
    fn inbox_native_input_requires_the_request_session() {
        let decode = crate::transport::native_input::value::<InboxRequest>(
            crate::transport::limits::MUTATION_BYTES,
            "orchestration",
        );
        assert_eq!(decode((serde_json::json!({"session_id": "session"}),)).unwrap().session_id, "session");
        let decode = crate::transport::native_input::value::<InboxRequest>(
            crate::transport::limits::MUTATION_BYTES,
            "orchestration",
        );
        assert!(decode((serde_json::json!({"session_id": "session", "other": true}),)).is_err());
        for id in ["", "bad/id", "bad:id", &"a".repeat(97)] {
            let request = InboxRequest { session_id: id.into() };
            let error: cockpit_protocol::v1::ErrorResponse =
                validate_inbox_session(&request).unwrap_err().into();
            assert_eq!(error.code, "invalid_session_id");
        }
        assert!(validate_inbox_session(&InboxRequest { session_id: "session-1_2".into() }).is_ok());
    }

    #[test]
    fn liaison_lookup_fences_path_session_and_unknown_fields() {
        let request = LiaisonLookupRequest {
            session_id: "session".into(),
            selected_root_id: None,
        };
        let matched = liaison_lookup_input()((Path("session".into()), Ok(Json(request.clone()))));
        assert!(matched.is_ok());
        let mismatched = liaison_lookup_input()((Path("other".into()), Ok(Json(request))));
        assert!(mismatched.is_err());
        assert!(serde_json::from_value::<LiaisonLookupRequest>(serde_json::json!({
            "session_id": "session", "selected_root_id": null,
            "project_key": "caller-asserted",
        })).is_err());
    }

}
