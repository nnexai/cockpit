use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    backend::{self, BackendProbe, RestartTicket},
    installation::{self, Role},
    lifecycle::{HerdrEndpoint, ImageIdentity},
    transport::{Transport, error::OperationError},
};

/// Immutable surface identity and the exact endpoint selected at startup.
pub struct BackendContext {
    pub state_root: PathBuf,
    pub cli: PathBuf,
    pub herdr: HerdrEndpoint,
    config_path: Option<PathBuf>,
    kind: Transport,
    frontend_entry: Option<String>,
    image: Option<ImageIdentity>,
    installed: bool,
}

impl BackendContext {
    pub fn new(
        state_root: PathBuf,
        cli: PathBuf,
        herdr: HerdrEndpoint,
        kind: Transport,
        frontend_entry: Option<String>,
    ) -> Self {
        let paths = installation::installed_paths(&std::env::vars().collect::<BTreeMap<_, _>>());
        let identity = installation::startup_image().map_err(|error| {
            eprintln!("cockpit: cannot determine {kind:?} surface image identity; update detection is unavailable: {error}");
        }).ok();
        let installed = identity
            .as_ref()
            .is_some_and(|(executable, _)| installation::is_installed(executable, &paths));
        let image = identity.map(|(_, image)| image);
        Self {
            state_root,
            cli,
            herdr,
            config_path: None,
            kind,
            frontend_entry,
            image,
            installed,
        }
    }

    pub fn set_frontend_entry(&mut self, entry: Option<String>) {
        self.frontend_entry = entry;
    }

    pub fn set_config_path(&mut self, path: Option<PathBuf>) {
        self.config_path = path;
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendRestartRequest {
    pub expected_instance: String,
    pub confirm: String,
}

pub fn native_restart_input(
    expected_instance: String,
) -> Result<BackendRestartRequest, OperationError> {
    Ok(BackendRestartRequest {
        expected_instance,
        confirm: "restart".into(),
    })
}

#[derive(Serialize)]
pub struct BackendSurfaceStatus {
    pub backend: Value,
    pub surface: Value,
    pub installed_build_commit: Option<String>,
    pub restart: Option<backend::RestartRecord>,
}

pub async fn status(
    context: &BackendContext,
    _transport: Transport,
    (): (),
) -> Result<BackendSurfaceStatus, OperationError> {
    let paths = installation::installed_paths(&std::env::vars().collect::<BTreeMap<_, _>>());
    let mut backend = json!({
        "state": "stopped", "instance_id": null, "started_at": null,
        "build_commit": null, "installed": false, "update_installed": false,
        "dispatcher": false, "library_sync": false, "ephemeral": null,
        "herdr": null, "expected_herdr": null, "last_exit": null,
    });
    match backend::probe(&context.state_root, &context.herdr).await {
        BackendProbe::Running(info) => {
            backend = json!({
                "state": "running", "instance_id": info.instance_id,
                "started_at": info.started_at, "build_commit": info.build_commit,
                "installed": info.installed,
                "update_installed": info.installed && installation::update_installed(Role::Cli, &info.image, &paths),
                "dispatcher": info.dispatcher.unwrap_or(false),
                "library_sync": info.library_sync.unwrap_or(false),
                "ephemeral": info.ephemeral, "herdr": info.herdr,
                "expected_herdr": null, "last_exit": null,
            });
        }
        BackendProbe::Starting => backend["state"] = json!("starting"),
        BackendProbe::Stopped { last_exit } => backend["last_exit"] = json!(last_exit),
        BackendProbe::LegacyOwner => backend["state"] = json!("legacy_owner"),
        BackendProbe::Unresponsive => backend["state"] = json!("unresponsive"),
        BackendProbe::EndpointMismatch { running, expected } => {
            backend["state"] = json!("endpoint_mismatch");
            backend["herdr"] = json!(running);
            backend["expected_herdr"] = json!(expected);
        }
    }
    let (kind, role) = match context.kind {
        Transport::Native => ("native", Role::Gui),
        Transport::Gateway => ("web", Role::Cli),
    };
    let surface = json!({
        "kind": kind, "build_commit": option_env!("COCKPIT_BUILD_COMMIT"),
        "installed": context.installed,
        "update_installed": context.installed && context.image.as_ref().is_some_and(|image| installation::update_installed(role, image, &paths)),
        "frontend_entry": context.frontend_entry,
    });
    Ok(BackendSurfaceStatus {
        backend,
        surface,
        installed_build_commit: if context.installed {
            installation::installed_build_commit(&paths)
        } else {
            None
        },
        restart: backend::read_restart(&context.state_root)?,
    })
}

pub async fn start(
    context: &BackendContext,
    transport: Transport,
    (): (),
) -> Result<BackendSurfaceStatus, OperationError> {
    backend::ensure_with_config(
        &context.state_root,
        &context.cli,
        &context.herdr,
        context.config_path.as_deref(),
    )
    .await?;
    status(context, transport, ()).await
}

pub async fn restart(
    context: &BackendContext,
    _transport: Transport,
    request: BackendRestartRequest,
) -> Result<RestartTicket, OperationError> {
    let instance = validate_restart(request)?;
    backend::spawn_restart_helper_with_config(
        &context.state_root,
        Some(instance),
        false,
        &context.herdr,
        context.config_path.as_deref(),
    )
    .await
    .map_err(Into::into)
}

fn validate_restart(request: BackendRestartRequest) -> Result<Uuid, OperationError> {
    if request.confirm != "restart" {
        return Err(OperationError::rejected(
            "invalid_update_confirmation",
            "Confirm the background service restart",
        ));
    }
    Uuid::parse_str(&request.expected_instance).map_err(|_| {
        OperationError::rejected("invalid_instance_id", "Expected a backend instance UUID")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_requires_confirmation_and_instance() {
        let id = Uuid::new_v4();
        assert_eq!(
            validate_restart(native_restart_input(id.to_string()).unwrap()).unwrap(),
            id
        );
        let wrong_confirmation = BackendRestartRequest {
            expected_instance: id.to_string(),
            confirm: "yes".into(),
        };
        let error: cockpit_protocol::v1::ErrorResponse =
            validate_restart(wrong_confirmation).unwrap_err().into();
        assert_eq!(error.code, "invalid_update_confirmation");
        let error: cockpit_protocol::v1::ErrorResponse =
            validate_restart(native_restart_input("bad".into()).unwrap())
                .unwrap_err()
                .into();
        assert_eq!(error.code, "invalid_instance_id");
    }

    #[test]
    fn restart_json_rejects_extra_fields() {
        assert!(
            serde_json::from_value::<BackendRestartRequest>(json!({
                "expected_instance": Uuid::new_v4().to_string(),
                "confirm": "restart", "relaunch_app": true
            }))
            .is_err()
        );
    }
}
