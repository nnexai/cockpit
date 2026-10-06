use std::collections::BTreeSet;

use cockpit_protocol::projects::{ProjectArtifact, ProjectConfiguration};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSource {
    Configured,
    ForgeOrigin,
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteResolution {
    pub candidates: Vec<String>,
    pub source: RouteSource,
    pub diagnostic: Option<String>,
}

/// Explicit provider-instance routes take precedence over existing forge hints.
/// Ambiguity is returned to the caller, never resolved using cwd or current Space.
pub fn resolve(
    configuration: &ProjectConfiguration,
    artifact: &ProjectArtifact,
    forge_candidates: &[String],
) -> RouteResolution {
    let instance = url::Url::parse(&artifact.canonical_url)
        .ok()
        .map(|url| url.origin().ascii_serialization());
    let configured: BTreeSet<_> = configuration
        .orchestration
        .routes
        .iter()
        .filter(|route| {
            route.provider == artifact.provider_id
                && instance.as_ref().is_some_and(|origin| {
                    url::Url::parse(&route.instance)
                        .ok()
                        .is_some_and(|url| url.origin().ascii_serialization() == *origin)
                })
                && artifact.canonical_id.starts_with(&route.project_id_prefix)
        })
        .map(|route| route.repository_id.clone())
        .collect();
    let (candidates, source) = if !configured.is_empty() {
        (
            configured.into_iter().collect::<Vec<_>>(),
            RouteSource::Configured,
        )
    } else {
        let candidates: Vec<_> = forge_candidates
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let source = if candidates.is_empty() {
            RouteSource::None
        } else {
            RouteSource::ForgeOrigin
        };
        (candidates, source)
    };
    let diagnostic = (candidates.len() > 1).then(|| "route_ambiguous".into());
    RouteResolution {
        candidates,
        source,
        diagnostic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::projects::OrchestrationRoute;

    fn configuration() -> ProjectConfiguration {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-routing-config-{}-{nonce}.toml", std::process::id()));
        std::fs::write(&path, "version = 1\n").expect("write isolated routing configuration");
        let configuration = crate::config::load_project_configuration(Some(&path), None);
        std::fs::remove_file(path).expect("remove routing configuration");
        configuration.expect("isolated routing configuration")
    }

    #[test]
    fn configured_routes_require_instance_and_preserve_ambiguity() {
        let mut config = configuration();
        config.orchestration.routes = vec![
            OrchestrationRoute {
                provider: "jira".into(),
                instance: "https://issues.example".into(),
                project_id_prefix: "APP-".into(),
                repository_id: "api".into(),
            },
            OrchestrationRoute {
                provider: "jira".into(),
                instance: "https://issues.example/".into(),
                project_id_prefix: "APP-".into(),
                repository_id: "ui".into(),
            },
        ];
        let mut artifact = ProjectArtifact {
            provider_id: "jira".into(),
            kind: "issue".into(),
            canonical_id: "APP-1".into(),
            original_url: "https://issues.example/browse/APP-1".into(),
            canonical_url: "https://issues.example/browse/APP-1".into(),
        };
        let resolution = resolve(&config, &artifact, &["forge".into()]);
        assert_eq!(resolution.candidates, ["api", "ui"]);
        assert_eq!(resolution.diagnostic.as_deref(), Some("route_ambiguous"));
        artifact.canonical_url = "https://other.example/browse/APP-1".into();
        let resolution = resolve(&config, &artifact, &[]);
        assert_eq!(resolution.source, RouteSource::None);
        assert!(resolution.candidates.is_empty());
    }
    #[test]
    fn forge_fallback_preserves_all_distinct_matches() {
        let mut config = configuration();
        config.orchestration.routes.clear();
        let artifact = ProjectArtifact {
            provider_id: "github".into(),
            kind: "issue".into(),
            canonical_id: "owner/repo#1".into(),
            original_url: "https://github.com/owner/repo/issues/1".into(),
            canonical_url: "https://github.com/owner/repo/issues/1".into(),
        };
        let resolution = resolve(
            &config,
            &artifact,
            &["clone-b".into(), "clone-a".into(), "clone-b".into()],
        );
        assert_eq!(resolution.source, RouteSource::ForgeOrigin);
        assert_eq!(resolution.candidates, ["clone-a", "clone-b"]);
        assert_eq!(resolution.diagnostic.as_deref(), Some("route_ambiguous"));
    }
}
