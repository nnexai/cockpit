use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::Path,
};

use cockpit_core::InspectionError;
use cockpit_herdr::HerdrCliConfig;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const LIFECYCLE_VERSION: u32 = 1;
pub const LIFECYCLE_SOCKET: &str = "lifecycle.v1.sock";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageIdentity {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HerdrEndpoint {
    pub executable: String,
    pub session: Option<String>,
    pub socket: Option<String>,
}

impl HerdrEndpoint {
    /// Resolve the same options and environment as the Herdr adapter, then pin
    /// a bare executable to the supplied PATH when it can be found.
    pub fn resolve(
        env: &BTreeMap<String, String>,
        path: Option<&OsStr>,
    ) -> Result<Self, InspectionError> {
        let config = HerdrCliConfig::from_options_with_environment(None, None, None, env)
            .map_err(|error| InspectionError::new(error.code, error.message))?;
        Ok(Self::from_config(&config, path))
    }

    pub fn from_config(config: &HerdrCliConfig, path: Option<&OsStr>) -> Self {
        let executable = &config.executable;
        let resolved = if executable.components().count() == 1 && !executable.is_absolute() {
            path.and_then(|path| {
                std::env::split_paths(path)
                    .map(|dir| dir.join(executable))
                    .find(|candidate| is_executable(candidate))
            })
            .unwrap_or_else(|| executable.clone())
        } else {
            executable.clone()
        };
        let resolved = std::fs::canonicalize(&resolved).unwrap_or(resolved);
        Self {
            executable: resolved.to_string_lossy().into_owned(),
            session: config.session.clone(),
            socket: config
                .socket
                .as_ref()
                .map(|socket| socket.to_string_lossy().into_owned()),
        }
    }

    pub fn to_args(&self) -> Vec<OsString> {
        let mut args = vec!["--herdr".into(), self.executable.clone().into()];
        if let Some(session) = &self.session {
            args.extend(["--herdr-session".into(), session.into()]);
        }
        if let Some(socket) = &self.socket {
            args.extend(["--herdr-socket".into(), socket.into()]);
        }
        args
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LifecycleRequest {
    Hello { lifecycle: u32 },
    Status,
    Stop { expected_instance: Uuid },
}

/// Hello and status share an identity; status-only fields are absent in hello.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleInfo {
    pub instance_id: Uuid,
    pub pid: u32,
    pub started_at: String,
    pub owner_protocol: u32,
    pub build_commit: Option<String>,
    pub image: ImageIdentity,
    pub installed: bool,
    pub herdr: HerdrEndpoint,
    pub state_root: String,
    #[serde(default)]
    pub dispatcher: Option<bool>,
    #[serde(default)]
    pub library_sync: Option<bool>,
    #[serde(default)]
    pub ephemeral: Option<EphemeralCounts>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EphemeralCounts {
    pub browser_associations: u32,
    pub comment_batches: u32,
    pub widgets: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_use_fixed_wire_format() {
        let instance = Uuid::nil();
        assert_eq!(
            serde_json::to_value(LifecycleRequest::Hello {
                lifecycle: LIFECYCLE_VERSION
            })
            .unwrap(),
            json!({"op":"hello", "lifecycle":1})
        );
        assert_eq!(
            serde_json::to_value(LifecycleRequest::Status).unwrap(),
            json!({"op":"status"})
        );
        assert_eq!(
            serde_json::to_value(LifecycleRequest::Stop {
                expected_instance: instance
            })
            .unwrap(),
            json!({"op":"stop", "expected_instance":instance})
        );
        for (fixture, request) in [
            (
                include_str!("../tests/fixtures/lifecycle-v1/hello.json"),
                LifecycleRequest::Hello {
                    lifecycle: LIFECYCLE_VERSION,
                },
            ),
            (
                include_str!("../tests/fixtures/lifecycle-v1/status.json"),
                LifecycleRequest::Status,
            ),
            (
                include_str!("../tests/fixtures/lifecycle-v1/stop.json"),
                LifecycleRequest::Stop {
                    expected_instance: instance,
                },
            ),
        ] {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(fixture).unwrap(),
                serde_json::to_value(request).unwrap()
            );
        }
    }

    #[test]
    fn hello_accepts_missing_status_and_unknown_fields() {
        let value = json!({"instance_id":Uuid::nil(), "pid":42, "started_at":"now", "owner_protocol":1,
            "build_commit":null, "image":{"dev":1,"ino":2}, "installed":false,
            "herdr":{"executable":"herdr","session":null,"socket":null}, "state_root":"/tmp/state", "future":true});
        let info: LifecycleInfo = serde_json::from_value(value).unwrap();
        assert_eq!(info.dispatcher, None);
        assert_eq!(info.library_sync, None);
        assert_eq!(info.ephemeral, None);
    }

    #[test]
    fn resolution_preserves_validation_and_arguments() {
        let mut env = BTreeMap::new();
        env.insert("COCKPIT_HERDR_EXECUTABLE".into(), "/some/herdr".into());
        env.insert("COCKPIT_HERDR_SOCKET".into(), "/tmp/herdr.sock".into());
        assert_eq!(
            HerdrEndpoint::resolve(&env, None).unwrap_err().code,
            "missing_socket_session"
        );
        env.insert("COCKPIT_HERDR_SESSION".into(), "example".into());
        let endpoint = HerdrEndpoint::resolve(&env, None).unwrap();
        assert_eq!(
            endpoint.to_args(),
            [
                "--herdr",
                "/some/herdr",
                "--herdr-session",
                "example",
                "--herdr-socket",
                "/tmp/herdr.sock"
            ]
            .map(OsString::from)
        );
        env.insert("COCKPIT_HERDR_SESSION".into(), "bad session".into());
        assert_eq!(
            HerdrEndpoint::resolve(&env, None).unwrap_err().code,
            "invalid_session_name"
        );
    }

    #[cfg(unix)]
    #[test]
    fn bare_executable_uses_first_executable_on_supplied_path() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("cockpit-endpoint-{}", Uuid::new_v4()));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("herdr-test"), "").unwrap();
        std::fs::write(second.join("herdr-test"), "").unwrap();
        std::fs::set_permissions(
            second.join("herdr-test"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let path = std::env::join_paths([&first, &second]).unwrap();
        let config = HerdrCliConfig {
            executable: std::path::PathBuf::from("herdr-test"),
            session: None,
            socket: None,
        };
        let endpoint = HerdrEndpoint::from_config(&config, Some(&path));
        assert_eq!(
            endpoint.executable,
            std::fs::canonicalize(second.join("herdr-test"))
                .unwrap()
                .to_string_lossy()
        );
        assert_eq!(
            endpoint.to_args(),
            [
                OsString::from("--herdr"),
                OsString::from(endpoint.executable)
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
