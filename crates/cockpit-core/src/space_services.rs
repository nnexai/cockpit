//! Read-only action-launcher services. Requests refresh observations; probes never
//! run on a timer and never delay a response. Only live Herdr IDs establish ownership.

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cockpit_protocol::v1::{
    ServiceStatus, SpaceServiceStatus, SpaceServiceStatusRequest, SpaceServiceStatusResponse,
};
use parking_lot::Mutex;
use serde::Deserialize;
use url::{Host, Url};

const PROBE_INTERVAL: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistentPane {
    key: String,
    kind: PersistentKind,
    title: String,
    pane_id: String,
    session: String,
    socket_path: String,
    #[serde(default)]
    tab_id: Option<String>,
    #[serde(default)]
    started_at: serde_json::Value,
    #[serde(default)]
    health: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum PersistentKind {
    Pane,
    Tab,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProbeTarget {
    Tcp(u16),
    Http(Url),
}

impl ProbeTarget {
    fn from_health(health: &serde_json::Value) -> Option<Self> {
        // A malformed or non-local health configuration is not a failing service.
        if let Some(value) = health.get("url") {
            let mut url = Url::parse(value.as_str()?).ok()?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return None;
            }
            match url.host()? {
                // The client pins this name to a literal loopback address while
                // preserving Host/TLS SNI and normal certificate verification.
                Host::Domain(name) if name.eq_ignore_ascii_case("localhost") => {}
                Host::Ipv4(ip) if ip.is_loopback() => {}
                Host::Ipv6(ip) if ip.is_loopback() => {}
                _ => return None,
            }
            if url.port_or_known_default()? == 0 {
                return None;
            }
            url.set_fragment(None);
            return Some(Self::Http(url));
        }
        let port = u16::try_from(health.get("port")?.as_u64()?).ok()?;
        (port != 0).then_some(Self::Tcp(port))
    }

    fn port(&self) -> u16 {
        match self {
            Self::Tcp(port) => *port,
            Self::Http(url) => url.port_or_known_default().expect("validated HTTP port"),
        }
    }
}

#[derive(PartialEq, Eq)]
struct ProbeIdentity {
    server_instance: String,
    pane_id: String,
    tab_id: Option<String>,
    started_at: serde_json::Value,
    target: ProbeTarget,
}

struct ProbeState {
    status: ServiceStatus,
    ever_healthy: bool,
    last_started: Option<Instant>,
    in_flight: bool,
}

impl Default for ProbeState {
    fn default() -> Self {
        Self {
            status: ServiceStatus::Starting,
            ever_healthy: false,
            last_started: None,
            in_flight: false,
        }
    }
}

impl ProbeState {
    fn begin(&mut self, now: Instant) -> bool {
        if self.in_flight
            || self.last_started.is_some_and(|last| now.duration_since(last) < PROBE_INTERVAL)
        {
            return false;
        }
        self.in_flight = true;
        self.last_started = Some(now);
        true
    }

    fn finish(&mut self, healthy: bool) {
        self.ever_healthy |= healthy;
        self.status = if healthy {
            ServiceStatus::Healthy
        } else if self.ever_healthy {
            ServiceStatus::Unhealthy
        } else {
            ServiceStatus::Starting
        };
        self.in_flight = false;
    }
}

struct CachedProbe {
    identity: ProbeIdentity,
    state: Arc<Mutex<ProbeState>>,
}

pub struct SpaceServices {
    probes: Mutex<HashMap<(String, String), CachedProbe>>,
    /// Parsed registry, reused while the file's modification time and size are unchanged.
    registry: Mutex<Option<CachedRegistry>>,
    client: Option<reqwest::Client>,
}

struct CachedRegistry {
    stamp: (std::time::SystemTime, u64),
    entries: Arc<Vec<PersistentPane>>,
}

impl Default for SpaceServices {
    fn default() -> Self {
        Self {
            probes: Mutex::new(HashMap::new()),
            registry: Mutex::new(None),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .resolve("localhost", SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .timeout(PROBE_TIMEOUT)
                .connect_timeout(PROBE_TIMEOUT)
                .build()
                .ok(),
        }
    }
}

impl SpaceServices {
    pub async fn read(
        &self,
        session_id: &str,
        request: &SpaceServiceStatusRequest,
        socket_path: Option<PathBuf>,
    ) -> SpaceServiceStatusResponse {
        let entries = match socket_path {
            Some(socket) => {
                let registry = self.registry_entries(&state_path()).await;
                entries_for_endpoint(registry.as_ref().clone(), session_id, &socket).await
            }
            None => Vec::new(),
        };
        self.observe(session_id, request, entries)
    }

    async fn registry_entries(&self, path: &std::path::Path) -> Arc<Vec<PersistentPane>> {
        let stamp = tokio::fs::metadata(path).await.ok()
            .and_then(|meta| meta.modified().ok().map(|modified| (modified, meta.len())));
        let Some(stamp) = stamp else {
            *self.registry.lock() = None;
            return Arc::new(Vec::new());
        };
        if let Some(cached) = self.registry.lock().as_ref().filter(|cached| cached.stamp == stamp) {
            return cached.entries.clone();
        }
        let entries = Arc::new(load_entries(path).await);
        *self.registry.lock() = Some(CachedRegistry { stamp, entries: entries.clone() });
        entries
    }

    fn observe(
        &self,
        session_id: &str,
        request: &SpaceServiceStatusRequest,
        entries: Vec<PersistentPane>,
    ) -> SpaceServiceStatusResponse {
        let mut services = Vec::new();
        let mut live_keys = HashSet::new();
        let mut seen_keys = HashSet::new();
        let pane_ids: HashSet<&str> = request.pane_ids.iter().map(String::as_str).collect();
        let mut probes = self.probes.lock();
        for entry in entries {
            let Some(mut row) = map_entry(session_id, &pane_ids, &entry) else { continue };
            if !seen_keys.insert(entry.key.clone()) { continue; }
            if let Some(target) = ProbeTarget::from_health(&entry.health) {
                row.port = Some(target.port());
                let key = (session_id.to_owned(), entry.key);
                live_keys.insert(key.clone());
                let identity = ProbeIdentity {
                    server_instance: request.server_instance.clone(),
                    pane_id: row.pane_id.clone(),
                    tab_id: row.tab_id.clone(),
                    started_at: entry.started_at,
                    target,
                };
                let cached = match probes.entry(key) {
                    std::collections::hash_map::Entry::Occupied(slot) => {
                        let cached = slot.into_mut();
                        if cached.identity != identity {
                            *cached = CachedProbe {
                                identity,
                                state: Arc::new(Mutex::new(ProbeState::default())),
                            };
                        }
                        cached
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => slot.insert(CachedProbe {
                        identity,
                        state: Arc::new(Mutex::new(ProbeState::default())),
                    }),
                };
                let mut state = cached.state.lock();
                row.status = state.status;
                if state.begin(Instant::now()) {
                    let state = cached.state.clone();
                    let target = cached.identity.target.clone();
                    let client = self.client.clone();
                    tokio::spawn(async move {
                        let healthy = probe(&target, client.as_ref()).await;
                        state.lock().finish(healthy);
                    });
                }
            }
            services.push(row);
        }
        // Detached probe tasks only own their old state; late results cannot
        // overwrite a removed service or a new launch reusing the same key.
        probes.retain(|key, _| key.0 != session_id || live_keys.contains(key));
        SpaceServiceStatusResponse { session_id: session_id.to_owned(), services }
    }
}

fn map_entry(session_id: &str, pane_ids: &HashSet<&str>, entry: &PersistentPane) -> Option<SpaceServiceStatus> {
    if entry.session != session_id || !pane_ids.contains(entry.pane_id.as_str()) {
        return None;
    }
    let tab_id = match entry.kind {
        PersistentKind::Tab => Some(entry.tab_id.clone().filter(|id| !id.is_empty())?),
        PersistentKind::Pane => None,
    };
    Some(SpaceServiceStatus {
        key: entry.key.clone(),
        pane_id: entry.pane_id.clone(),
        tab_id,
        title: entry.title.clone(),
        status: ServiceStatus::Healthy,
        port: None,
    })
}

async fn probe(target: &ProbeTarget, client: Option<&reqwest::Client>) -> bool {
    tokio::time::timeout(PROBE_TIMEOUT, async {
        match target {
            ProbeTarget::Tcp(port) => tokio::net::TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, *port))).await.is_ok(),
            ProbeTarget::Http(url) => {
                let Some(client) = client else { return false };
                client.get(url.clone()).send().await
                    .is_ok_and(|response| (200..500).contains(&response.status().as_u16()))
            }
        }
    }).await.unwrap_or(false)
}

fn state_path() -> PathBuf {
    state_path_from(
        std::env::var("XDG_STATE_HOME").ok(),
        std::env::var("HOME").ok(),
        std::env::temp_dir(),
    )
}

fn state_path_from(xdg_state_home: Option<String>, home: Option<String>, temp_dir: PathBuf) -> PathBuf {
    // Match Herdr's config::state_dir() and plugin_paths::plugin_state_dir().
    let root = match (xdg_state_home, home) {
        (Some(path), _) => PathBuf::from(path).join("herdr"),
        (None, Some(path)) => PathBuf::from(path).join(".local/state/herdr"),
        (None, None) => temp_dir.join("herdr-state"),
    };
    root.join("plugins/nnex.action-launcher/persistent-panes.json")
}

async fn load_entries(path: &std::path::Path) -> Vec<PersistentPane> {
    let Ok(bytes) = tokio::fs::read(path).await else { return Vec::new() };
    let Ok(entries) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) else { return Vec::new() };
    entries.into_iter().filter_map(|entry| serde_json::from_value(entry).ok()).collect()
}

async fn entries_for_endpoint(
    entries: Vec<PersistentPane>,
    session: &str,
    socket: &std::path::Path,
) -> Vec<PersistentPane> {
    let Ok(selected) = tokio::fs::canonicalize(socket).await else { return Vec::new() };
    // The registry accumulates entries of every Herdr endpoint; resolve each distinct socket once.
    let mut same_endpoint: HashMap<String, bool> = HashMap::new();
    let mut matching = Vec::new();
    for entry in entries {
        if entry.session != session || !std::path::Path::new(&entry.socket_path).is_absolute() {
            continue;
        }
        let accepted = match same_endpoint.get(&entry.socket_path) {
            Some(accepted) => *accepted,
            None => {
                let accepted = tokio::fs::canonicalize(&entry.socket_path).await.is_ok_and(|path| path == selected);
                same_endpoint.insert(entry.socket_path.clone(), accepted);
                accepted
            }
        };
        if accepted {
            matching.push(entry);
        }
    }
    matching
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn state_path_follows_herdr_plugin_state_dir() {
        let cases = [
            (Some("/state"), Some("/home/test"), "/state/herdr/plugins/nnex.action-launcher/persistent-panes.json"),
            (Some("/state"), None, "/state/herdr/plugins/nnex.action-launcher/persistent-panes.json"),
            (None, Some("/home/test"), "/home/test/.local/state/herdr/plugins/nnex.action-launcher/persistent-panes.json"),
            (None, None, "/temporary/herdr-state/plugins/nnex.action-launcher/persistent-panes.json"),
            (Some(""), Some("/home/test"), "herdr/plugins/nnex.action-launcher/persistent-panes.json"),
            (None, Some(""), ".local/state/herdr/plugins/nnex.action-launcher/persistent-panes.json"),
        ];
        for (xdg, home, expected) in cases {
            assert_eq!(
                state_path_from(xdg.map(String::from), home.map(String::from), PathBuf::from("/temporary")),
                PathBuf::from(expected),
            );
        }
    }

    fn request() -> SpaceServiceStatusRequest {
        SpaceServiceStatusRequest { server_instance: "boot".into(), pane_ids: vec!["pane".into()] }
    }

    fn entry() -> PersistentPane {
        serde_json::from_value(json!({"key":"service", "kind":"tab", "title":"Web",
            "paneId":"pane", "tabId":"tab", "startedAt":123, "health":null,
            "session":"session", "socketPath":"/tmp/herdr-test.sock"})).unwrap()
    }

    #[tokio::test]
    async fn persisted_provenance_rejects_other_sessions_endpoints_and_legacy_entries() {
        let root = std::env::temp_dir().join(format!("cockpit-service-provenance-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&root).await.unwrap();
        let selected = root.join("selected.sock");
        let other = root.join("other.sock");
        tokio::fs::write(&selected, []).await.unwrap();
        tokio::fs::write(&other, []).await.unwrap();
        let mut matching = entry();
        matching.socket_path = selected.to_string_lossy().into_owned();
        let mut wrong_session = matching.clone();
        wrong_session.session = "another-session".into();
        let mut wrong_socket = matching.clone();
        wrong_socket.socket_path = other.to_string_lossy().into_owned();
        let mut relative_socket = matching.clone();
        relative_socket.socket_path = "selected.sock".into();
        let state_path = root.join("persistent-panes.json");
        let state = json!([
            {"key":"legacy", "kind":"tab", "title":"Legacy", "paneId":"pane", "tabId":"tab"},
            {"key":"current", "kind":"tab", "title":"Current", "paneId":"pane", "tabId":"tab",
                "session":"session", "socketPath":selected}
        ]);
        tokio::fs::write(&state_path, serde_json::to_vec(&state).unwrap()).await.unwrap();
        let loaded = load_entries(&state_path).await;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].key, "current");
        let accepted = entries_for_endpoint(
            vec![matching, wrong_session, wrong_socket, relative_socket], "session", &selected,
        ).await;
        assert_eq!(accepted.len(), 1);
        assert!(entries_for_endpoint(accepted, "session", &root.join("missing.sock")).await.is_empty());
        assert!(map_entry("session", &HashSet::from(["pane"]), &{
            let mut entry = entry();
            entry.session = "another-session".into();
            entry
        }).is_none());
        #[cfg(unix)]
        {
            let alias = root.join("alias.sock");
            std::os::unix::fs::symlink(&selected, &alias).unwrap();
            let mut entry = entry();
            entry.socket_path = alias.to_string_lossy().into_owned();
            assert_eq!(entries_for_endpoint(vec![entry], "session", &selected).await.len(), 1);
        }
        tokio::fs::remove_dir_all(root).await.unwrap();
    }


    #[test]
    fn status_history_and_request_throttle() {
        let mut state = ProbeState::default();
        let now = Instant::now();
        assert_eq!(state.status, ServiceStatus::Starting);
        assert!(state.begin(now));
        assert!(!state.begin(now + PROBE_INTERVAL));
        state.finish(false);
        assert_eq!(state.status, ServiceStatus::Starting);
        assert!(!state.begin(now + Duration::from_secs(4)));
        assert!(state.begin(now + PROBE_INTERVAL));
        state.finish(true);
        assert_eq!(state.status, ServiceStatus::Healthy);
        state.finish(false);
        assert_eq!(state.status, ServiceStatus::Unhealthy);
        state.finish(true);
        assert_eq!(state.status, ServiceStatus::Healthy);
    }

    #[test]
    fn live_panes_filter_registry_entries_without_resolving_spaces() {
        let panes = HashSet::from(["pane"]);
        let mut entry = entry();
        let row = map_entry("session", &panes, &entry).unwrap();
        assert_eq!(row.pane_id, "pane");
        assert_eq!(row.tab_id.as_deref(), Some("tab"));
        // The frontend checks this registry tab ID against its live pane.
        entry.tab_id = Some("other-tab".into());
        assert_eq!(map_entry("session", &panes, &entry).unwrap().tab_id.as_deref(), Some("other-tab"));
        entry.tab_id = None;
        assert!(map_entry("session", &panes, &entry).is_none());
        entry.kind = PersistentKind::Pane;
        assert!(map_entry("session", &panes, &entry).unwrap().tab_id.is_none());
        entry.pane_id = "dead-pane".into();
        assert!(map_entry("session", &panes, &entry).is_none());
        assert!(map_entry("session", &HashSet::new(), &entry).is_none());
    }

    #[test]
    fn health_targets_are_loopback_only_and_pin_localhost() {
        for url in ["https://example.com", "http://localhost.example.com", "http://192.168.1.1",
            "http://0.0.0.0", "http://[::]", "file:///tmp/test", "ftp://localhost",
            "http://user:pass@localhost", "http://localhost:0"] {
            assert!(ProbeTarget::from_health(&json!({"url":url})).is_none(), "{url}");
        }
        for url in ["http://127.0.0.1:5173", "https://[::1]:5173", "http://127.0.0.2:5173"] {
            assert_eq!(ProbeTarget::from_health(&json!({"url":url})).unwrap().port(), 5173);
        }
        let Some(ProbeTarget::Http(url)) = ProbeTarget::from_health(&json!({"url":"http://localhost:5173/health"})) else { panic!("local URL") };
        assert_eq!(url.host_str(), Some("localhost"));
        assert!(ProbeTarget::from_health(&json!({"port":0})).is_none());
        assert!(ProbeTarget::from_health(&json!({"port":65536})).is_none());
        assert_eq!(ProbeTarget::from_health(&json!({"port":5173})), Some(ProbeTarget::Tcp(5173)));
    }

    #[tokio::test]
    async fn absent_state_and_unprobed_services_are_silent() {
        let path = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()).join("missing.json");
        assert!(load_entries(&path).await.is_empty());
        let service = SpaceServices::default();
        let mut entry = entry();
        entry.health = json!({"url":"https://remote.invalid"});
        let rows = service.observe("session", &request(), vec![entry]);
        assert_eq!(rows.services[0].status, ServiceStatus::Healthy);
        assert_eq!(rows.services[0].port, None);
        assert!(service.probes.lock().is_empty());
    }

    #[tokio::test]
    async fn new_launch_resets_history_and_dead_entries_drop_cache() {
        let service = SpaceServices::default();
        let request = request();
        let mut entry = entry();
        entry.health = json!({"port":5173});
        let rows = service.observe("session", &request, vec![entry.clone()]);
        assert_eq!(rows.services[0].status, ServiceStatus::Starting);
        let key = ("session".to_owned(), entry.key.clone());
        let old_state = service.probes.lock()[&key].state.clone();
        old_state.lock().finish(true);
        assert_eq!(service.observe("session", &request, vec![entry.clone()]).services[0].status, ServiceStatus::Healthy);
        entry.started_at = json!(456);
        assert_eq!(service.observe("session", &request, vec![entry]).services[0].status, ServiceStatus::Starting);
        // A late result belongs to the old Arc, never the replacement launch.
        old_state.lock().finish(false);
        assert_eq!(service.probes.lock()[&key].state.lock().status, ServiceStatus::Starting);
        service.observe("session", &request, Vec::new());
        assert!(service.probes.lock().is_empty());
    }

    #[tokio::test]
    async fn dead_panes_are_never_probed_and_server_changes_reset_health() {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let service = SpaceServices::default();
        let mut entry = entry();
        entry.health = json!({"port":listener.local_addr().unwrap().port()});
        let mut request = request();
        request.pane_ids.clear();
        assert!(service.observe("session", &request, vec![entry.clone()]).services.is_empty());
        assert!(service.probes.lock().is_empty());
        assert!(tokio::time::timeout(Duration::from_millis(25), listener.accept()).await.is_err());

        request.pane_ids.push("pane".into());
        service.observe("session", &request, vec![entry.clone()]);
        let key = ("session".to_owned(), entry.key.clone());
        let old_state = service.probes.lock()[&key].state.clone();
        old_state.lock().finish(true);
        request.server_instance = "new-boot".into();
        assert_eq!(service.observe("session", &request, vec![entry.clone()]).services[0].status, ServiceStatus::Starting);
        old_state.lock().finish(true);
        assert_eq!(service.probes.lock()[&key].state.lock().status, ServiceStatus::Starting);
        request.pane_ids.clear();
        assert!(service.observe("session", &request, vec![entry]).services.is_empty());
        assert!(service.probes.lock().is_empty());
    }

    #[tokio::test]
    async fn http_statuses_and_redirects_use_only_the_local_destination() {
        let service = SpaceServices::default();
        for (code, healthy) in [(200, true), (302, true), (404, true), (500, false)] {
            let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 1024];
                let _ = stream.read(&mut request).await.unwrap();
                // A redirect is healthy but must not be followed to this remote URL.
                stream.write_all(format!("HTTP/1.1 {code} Test\r\nLocation: http://192.0.2.1:1/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            });
            let target = ProbeTarget::from_health(&json!({"url":format!("http://localhost:{port}/")})).unwrap();
            assert_eq!(probe(&target, service.client.as_ref()).await, healthy);
            server.await.unwrap();
        }
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        assert!(probe(&ProbeTarget::Tcp(listener.local_addr().unwrap().port()), None).await);
    }
}
