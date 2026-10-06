use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use cockpit_herdr::HerdrCliConfig;

/// Caller endpoint inputs captured once, separate from explicit CLI options.
#[derive(Debug, Default, Clone)]
pub(crate) struct AmbientEndpoint {
    pub launch_session: Option<String>,
    pub inherited_socket: Option<PathBuf>,
    pub inherited_session: Option<String>,
}

impl AmbientEndpoint {
    pub(crate) fn from_process() -> Self {
        let nonempty = |name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        Self {
            launch_session: nonempty("COCKPIT_SESSION_ID"),
            inherited_socket: std::env::var_os("HERDR_SOCKET_PATH")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            inherited_session: nonempty("HERDR_SESSION_NAME").or_else(|| nonempty("HERDR_SESSION")),
        }
    }
}

#[derive(Debug)]
pub(crate) struct EndpointError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Endpoint {
    pub session: String,
    pub config: HerdrCliConfig,
}

impl Endpoint {
    pub(crate) fn socket(&self) -> Option<&Path> {
        self.config.socket()
    }

    pub(crate) fn executable(&self) -> &Path {
        self.config.executable()
    }
}

pub(crate) fn resolve_endpoint(
    executable: Option<PathBuf>,
    explicit_session: Option<String>,
    explicit_socket: Option<PathBuf>,
    ambient: &AmbientEndpoint,
) -> Result<Endpoint, EndpointError> {
    let launch_session = ambient
        .launch_session
        .as_ref()
        .filter(|value| !value.trim().is_empty());
    let inherited_socket = ambient
        .inherited_socket
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty());
    let inherited_session = ambient
        .inherited_session
        .as_ref()
        .filter(|value| !value.trim().is_empty());
    let implicit_session = || {
        launch_session
            .or(inherited_session)
            .map(|session| Cow::Borrowed(session.as_str()))
            .or_else(|| {
                inherited_socket
                    .and_then(|path| layout_session(path))
                    .map(Cow::Owned)
            })
    };
    let explicit_socket = explicit_socket.filter(|path| !path.as_os_str().is_empty());
    let (session, socket) = match (explicit_session, explicit_socket) {
        (Some(session), socket) => {
            let socket = socket.or_else(|| {
                inherited_socket
                    .filter(|_| implicit_session().as_deref() == Some(session.as_str()))
                    .cloned()
            });
            (Some(session), socket)
        }
        (None, Some(socket)) => {
            let session = launch_session.cloned().ok_or_else(|| EndpointError {
                code: "missing_socket_session".into(),
                message: "--herdr-socket requires --herdr-session".into(),
            })?;
            (Some(session), Some(socket))
        }
        (None, None) => (
            implicit_session().map(Cow::into_owned),
            inherited_socket.cloned(),
        ),
    };
    let session = session
        .filter(|session| !session.is_empty())
        .ok_or_else(|| EndpointError {
            code: "session_required".into(),
            message: "a resolvable Herdr session is required; pass --herdr-session outside Herdr"
                .into(),
        })?;
    // Explicit selectors have already been captured by clap. Keep the adapter's
    // executable discovery without letting its env fallback reintroduce a socket.
    let environment = std::env::vars()
        .filter(|(name, _)| name != "COCKPIT_HERDR_SOCKET" && name != "COCKPIT_HERDR_SESSION")
        .collect();
    let config = HerdrCliConfig::from_options_with_environment(
        executable,
        Some(session.clone()),
        socket,
        &environment,
    )
    .map_err(|error| EndpointError {
        code: error.code,
        message: error.message,
    })?;
    Ok(Endpoint { session, config })
}

fn layout_session(path: &Path) -> Option<String> {
    if path.file_name()? == "herdr.sock" {
        let parent = path.parent()?;
        if parent.parent().and_then(Path::file_name) == Some(std::ffi::OsStr::new("sessions")) {
            return parent.file_name()?.to_str().map(str::to_owned);
        }
        return Some("default".into());
    }
    path.file_stem()?
        .to_str()
        .map(|stem| stem.trim_start_matches("herdr-").to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ambient(
        launch: Option<&str>,
        session: Option<&str>,
        socket: Option<&str>,
    ) -> AmbientEndpoint {
        AmbientEndpoint {
            launch_session: launch.map(str::to_owned),
            inherited_session: session.map(str::to_owned),
            inherited_socket: socket.map(PathBuf::from),
        }
    }

    fn resolve(
        session: Option<&str>,
        socket: Option<&str>,
        ambient: &AmbientEndpoint,
    ) -> Result<Endpoint, EndpointError> {
        resolve_endpoint(
            Some("/test/discovered-herdr".into()),
            session.map(str::to_owned),
            socket.map(PathBuf::from),
            ambient,
        )
    }

    #[test]
    fn explicit_session_does_not_inherit_foreign_socket() {
        let endpoint = resolve(
            Some("chosen"),
            None,
            &ambient(None, Some("inherited"), Some("/tmp/inherited.sock")),
        )
        .unwrap();
        assert_eq!(endpoint.session, "chosen");
        assert!(endpoint.socket().is_none());
        assert!(
            resolve(
                None,
                Some("/tmp/explicit.sock"),
                &ambient(None, Some("inherited"), None)
            )
            .is_err()
        );
        let endpoint = resolve(
            None,
            None,
            &ambient(None, None, Some("/tmp/herdr-fixture.sock")),
        )
        .unwrap();
        assert_eq!(endpoint.session, "fixture");
    }

    #[test]
    fn launcher_logical_session_accepts_its_explicit_socket_without_focus_guessing() {
        let endpoint = resolve(
            Some("launcher-session"),
            Some("/tmp/launcher.sock"),
            &ambient(None, Some("foreign-session"), Some("/tmp/foreign.sock")),
        )
        .unwrap();
        assert_eq!(endpoint.session, "launcher-session");
        assert_eq!(endpoint.socket(), Some(Path::new("/tmp/launcher.sock")));
    }

    #[test]
    fn endpoint_precedence_and_socket_layout() {
        let cases = [
            (
                Some("chosen"),
                Some("/tmp/chosen.sock"),
                ambient(Some("launch"), Some("native"), Some("/tmp/native.sock")),
                Ok(("chosen", Some("/tmp/chosen.sock"))),
            ),
            (
                Some("foreign"),
                None,
                ambient(Some("launch"), Some("foreign"), Some("/tmp/custom.sock")),
                Ok(("foreign", None)),
            ),
            (
                Some("launch"),
                None,
                ambient(Some("launch"), None, Some("/tmp/custom.sock")),
                Ok(("launch", Some("/tmp/custom.sock"))),
            ),
            (
                Some("native"),
                None,
                ambient(None, Some("native"), Some("/tmp/native.sock")),
                Ok(("native", Some("/tmp/native.sock"))),
            ),
            (
                None,
                Some("/tmp/explicit.sock"),
                ambient(Some("launch"), Some("native"), Some("/tmp/native.sock")),
                Ok(("launch", Some("/tmp/explicit.sock"))),
            ),
            (
                None,
                Some("/tmp/explicit.sock"),
                ambient(None, None, None),
                Err("missing_socket_session"),
            ),
            (
                None,
                Some("/tmp/explicit.sock"),
                ambient(None, Some("native"), Some("/tmp/native.sock")),
                Err("missing_socket_session"),
            ),
            (
                None,
                None,
                ambient(Some("launch"), Some("native"), Some("/tmp/custom.sock")),
                Ok(("launch", Some("/tmp/custom.sock"))),
            ),
            (
                None,
                None,
                ambient(Some("launch"), Some("native"), None),
                Ok(("launch", None)),
            ),
            (
                None,
                None,
                ambient(None, None, Some("/home/test/.config/herdr/herdr.sock")),
                Ok(("default", Some("/home/test/.config/herdr/herdr.sock"))),
            ),
            (
                None,
                None,
                ambient(
                    None,
                    None,
                    Some("/home/test/.config/herdr/sessions/work/herdr.sock"),
                ),
                Ok((
                    "work",
                    Some("/home/test/.config/herdr/sessions/work/herdr.sock"),
                )),
            ),
            (
                None,
                None,
                ambient(None, None, Some("/tmp/herdr-fixture.sock")),
                Ok(("fixture", Some("/tmp/herdr-fixture.sock"))),
            ),
            (
                None,
                None,
                ambient(None, Some("native"), Some("/tmp/herdr-fixture.sock")),
                Ok(("native", Some("/tmp/herdr-fixture.sock"))),
            ),
            (
                None,
                None,
                ambient(None, Some("native"), None),
                Ok(("native", None)),
            ),
            (
                None,
                None,
                ambient(None, None, None),
                Err("session_required"),
            ),
            (
                None,
                None,
                ambient(Some(""), Some(""), Some("")),
                Err("session_required"),
            ),
            (
                Some(""),
                None,
                ambient(Some("launch"), None, Some("/tmp/custom.sock")),
                Err("session_required"),
            ),
            (
                Some("invalid/name"),
                None,
                ambient(None, None, None),
                Err("invalid_session_name"),
            ),
            (
                None,
                None,
                ambient(Some("invalid/name"), None, Some("/tmp/custom.sock")),
                Err("invalid_session_name"),
            ),
        ];
        for (session, socket, ambient, expected) in cases {
            let endpoint = resolve(session, socket, &ambient);
            match expected {
                Ok((session, socket)) => {
                    let endpoint = endpoint.unwrap();
                    assert_eq!(endpoint.session, session);
                    assert_eq!(endpoint.config.session(), Some(session));
                    assert_eq!(endpoint.socket(), socket.map(Path::new));
                    assert_eq!(endpoint.executable(), Path::new("/test/discovered-herdr"));
                }
                Err(code) => assert_eq!(endpoint.unwrap_err().code, code),
            }
        }
    }
}
