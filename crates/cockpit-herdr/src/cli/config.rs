use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAX_SESSION_NAME: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Resolved command configuration. `None` means Herdr's own default behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrCliConfig {
    pub executable: PathBuf,
    pub session: Option<String>,
    pub socket: Option<PathBuf>,
}

impl HerdrCliConfig {
    pub fn from_options(
        executable: Option<PathBuf>,
        session: Option<String>,
        socket: Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let environment: BTreeMap<String, String> = std::env::vars().collect();
        Self::from_options_with_environment(executable, session, socket, &environment)
    }

    /// Resolve command options without touching process-global environment state.
    pub fn from_options_with_environment(
        executable: Option<PathBuf>,
        session: Option<String>,
        socket: Option<PathBuf>,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, ConfigError> {
        let executable = executable
            .or_else(|| nonempty(environment.get("COCKPIT_HERDR_EXECUTABLE")).map(PathBuf::from))
            .unwrap_or_else(|| default_herdr_executable(environment));
        let session = session.or_else(|| nonempty(environment.get("COCKPIT_HERDR_SESSION")));
        if let Some(session_name) = session.as_deref()
            && !valid_session_name(session_name)
        {
            return Err(ConfigError {
                code: "invalid_session_name".into(),
                message: "session name contains unsupported characters".into(),
            });
        }
        let socket =
            socket.or_else(|| nonempty(environment.get("COCKPIT_HERDR_SOCKET")).map(PathBuf::from));
        if socket.is_some() && session.is_none() {
            return Err(ConfigError {
                code: "missing_socket_session".into(),
                message: "--herdr-socket/COCKPIT_HERDR_SOCKET requires an explicit --herdr-session/COCKPIT_HERDR_SESSION".into(),
            });
        }
        Ok(Self {
            executable,
            session,
            socket,
        })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    pub fn socket(&self) -> Option<&Path> {
        self.socket.as_deref()
    }
}

fn nonempty(value: Option<&String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty()).cloned()
}

fn default_herdr_executable(environment: &BTreeMap<String, String>) -> PathBuf {
    let executable_name = format!("herdr{}", std::env::consts::EXE_SUFFIX);
    if let Some(path) = environment.get("PATH") {
        for directory in std::env::split_paths(path) {
            let candidate = directory.join(&executable_name);
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
    }

    if let Some(home) = nonempty(environment.get("HOME")).map(PathBuf::from) {
        for relative in [".local/bin", ".linuxbrew/bin", ".cargo/bin"] {
            let candidate = home.join(relative).join(&executable_name);
            if is_executable_file(&candidate) {
                return candidate;
            }
        }
    }

    for directory in [
        "/home/linuxbrew/.linuxbrew/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
    ] {
        let candidate = Path::new(directory).join(&executable_name);
        if is_executable_file(&candidate) {
            return candidate;
        }
    }

    PathBuf::from(executable_name)
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

pub(crate) fn valid_session_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SESSION_NAME
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::HerdrCliConfig;

    #[test]
    fn rejects_a_socket_without_a_selected_session() {
        let error = HerdrCliConfig::from_options_with_environment(
            Some(PathBuf::from("herdr")),
            None,
            Some(PathBuf::from("/tmp/herdr.sock")),
            &BTreeMap::new(),
        )
        .unwrap_err();

        assert_eq!(error.code, "missing_socket_session");
    }

    #[test]
    fn accepts_environment_overrides_without_reading_process_environment() {
        let environment = BTreeMap::from([
            (
                "COCKPIT_HERDR_EXECUTABLE".to_owned(),
                "/opt/herdr".to_owned(),
            ),
            ("COCKPIT_HERDR_SESSION".to_owned(), "review-1".to_owned()),
            (
                "COCKPIT_HERDR_SOCKET".to_owned(),
                "/tmp/review-1.sock".to_owned(),
            ),
        ]);

        let config =
            HerdrCliConfig::from_options_with_environment(None, None, None, &environment).unwrap();

        assert_eq!(config.executable(), PathBuf::from("/opt/herdr"));
        assert_eq!(config.session(), Some("review-1"));
        assert_eq!(
            config.socket(),
            Some(PathBuf::from("/tmp/review-1.sock").as_path())
        );
    }
}
