//! Read-only action-launcher services. Requests refresh observations; probes never
//! run on a timer and never delay a response. Only live Herdr IDs establish ownership.

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cockpit_protocol::v1::{
    ServiceStatus, SessionSnapshotResponse, SpaceServiceStatus, SpaceServiceStatusResponse,
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
    client: Option<reqwest::Client>,
}

impl Default for SpaceServices {
    fn default() -> Self {
        Self {
            probes: Mutex::new(HashMap::new()),
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
        snapshot: &SessionSnapshotResponse,
        socket_path: Option<PathBuf>,
    ) -> SpaceServiceStatusResponse {
        let entries = match (state_path(), socket_path) {
            (Some(path), Some(socket)) => {
                entries_for_endpoint(load_entries(&path).await, &snapshot.session_id, &socket).await
            }
            _ => Vec::new(),
        };
        self.observe(snapshot, entries)
    }

    fn observe(
        &self,
        snapshot: &SessionSnapshotResponse,
        entries: Vec<PersistentPane>,
    ) -> SpaceServiceStatusResponse {
        let mut services = Vec::new();
        let mut live_keys = HashSet::new();
        let mut seen_keys = HashSet::new();
        let mut probes = self.probes.lock();
        for entry in entries {
            let Some(mut row) = map_entry(snapshot, &entry) else { continue };
            if !seen_keys.insert(entry.key.clone()) { continue; }
            if let Some(target) = ProbeTarget::from_health(&entry.health) {
                row.port = Some(target.port());
                let key = (snapshot.session_id.clone(), entry.key);
                live_keys.insert(key.clone());
                let identity = ProbeIdentity {
                    server_instance: snapshot.server_instance.clone(),
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
        probes.retain(|key, _| key.0 != snapshot.session_id || live_keys.contains(key));
        SpaceServiceStatusResponse { session_id: snapshot.session_id.clone(), services }
    }
}

fn map_entry(snapshot: &SessionSnapshotResponse, entry: &PersistentPane) -> Option<SpaceServiceStatus> {
    if entry.session != snapshot.session_id { return None; }
    let pane = snapshot.panes.iter().find(|pane| pane.id == entry.pane_id)?;
    let tab = snapshot.tabs.iter().find(|tab| tab.id == pane.tab_id)?;
    if pane.space_id != tab.space_id
        || !snapshot.spaces.iter().any(|space| space.id == tab.space_id)
        || (entry.kind == PersistentKind::Tab && entry.tab_id.as_deref() != Some(tab.id.as_str()))
    {
        return None;
    }
    Some(SpaceServiceStatus {
        key: entry.key.clone(),
        space_id: tab.space_id.clone(),
        pane_id: pane.id.clone(),
        tab_id: (entry.kind == PersistentKind::Tab).then(|| tab.id.clone()),
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

fn state_path() -> Option<PathBuf> {
    // Match HerdrCliAdapter's config-directory override precedence.
    let root = std::env::var_os("HERDR_CONFIG_DIR").map(PathBuf::from)
        .or_else(|| std::env::var_os("HERDR_CONFIG_PATH")
            .and_then(|path| PathBuf::from(path).parent().map(PathBuf::from)))
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(|path| PathBuf::from(path).join("herdr")))
        .or_else(|| std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".config/herdr")))?;
    Some(root.join("plugins/state/nnex.action-launcher/persistent-panes.json"))
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
    let mut matching = Vec::new();
    for entry in entries {
        if entry.session != session || !std::path::Path::new(&entry.socket_path).is_absolute() {
            continue;
        }
        if tokio::fs::canonicalize(&entry.socket_path).await.is_ok_and(|path| path == selected) {
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

    fn snapshot() -> SessionSnapshotResponse {
        serde_json::from_value(json!({
            "session_id":"session", "server_instance":"boot", "version":"1", "protocol":1,
            "focused_space_id":null, "focused_tab_id":null, "focused_pane_id":null,
            "spaces":[{"id":"space", "label":"same title", "number":1, "tab_count":1,
                "pane_count":1, "focused":false, "agent_status":"idle", "git":null}],
            "tabs":[{"id":"tab", "space_id":"space", "label":"same title", "number":1,
                "pane_count":1, "focused":false, "focused_pane_id":null}],
            "panes":[{"id":"pane", "terminal_id":"terminal", "space_id":"space", "tab_id":"tab",
                "title":"same title", "focused":false, "agent":null, "agent_status":"idle", "revision":1}],
            "agents":[]
        })).unwrap()
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
        assert!(map_entry(&snapshot(), &{
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
    fn live_ids_are_the_only_mapping_and_tab_entries_must_match() {
        let mut snapshot = snapshot();
        let mut entry = entry();
        let row = map_entry(&snapshot, &entry).unwrap();
        assert_eq!(row.space_id, "space");
        assert_eq!(row.tab_id.as_deref(), Some("tab"));
        entry.tab_id = Some("other-tab".into());
        assert!(map_entry(&snapshot, &entry).is_none());
        entry.kind = PersistentKind::Pane;
        assert!(map_entry(&snapshot, &entry).unwrap().tab_id.is_none());
        entry.pane_id = "dead-pane".into();
        assert!(map_entry(&snapshot, &entry).is_none());
        entry.pane_id = "pane".into();
        snapshot.tabs.clear();
        assert!(map_entry(&snapshot, &entry).is_none());
        snapshot = self::snapshot();
        snapshot.panes[0].space_id = "other-space".into();
        assert!(map_entry(&snapshot, &entry).is_none());
        snapshot = self::snapshot();
        snapshot.spaces.clear();
        assert!(map_entry(&snapshot, &entry).is_none());
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
        let rows = service.observe(&snapshot(), vec![entry]);
        assert_eq!(rows.services[0].status, ServiceStatus::Healthy);
        assert_eq!(rows.services[0].port, None);
        assert!(service.probes.lock().is_empty());
    }

    #[tokio::test]
    async fn new_launch_resets_history_and_dead_entries_drop_cache() {
        let service = SpaceServices::default();
        let snapshot = snapshot();
        let mut entry = entry();
        entry.health = json!({"port":5173});
        let rows = service.observe(&snapshot, vec![entry.clone()]);
        assert_eq!(rows.services[0].status, ServiceStatus::Starting);
        let key = (snapshot.session_id.clone(), entry.key.clone());
        let old_state = service.probes.lock()[&key].state.clone();
        old_state.lock().finish(true);
        assert_eq!(service.observe(&snapshot, vec![entry.clone()]).services[0].status, ServiceStatus::Healthy);
        entry.started_at = json!(456);
        assert_eq!(service.observe(&snapshot, vec![entry]).services[0].status, ServiceStatus::Starting);
        // A late result belongs to the old Arc, never the replacement launch.
        old_state.lock().finish(false);
        assert_eq!(service.probes.lock()[&key].state.lock().status, ServiceStatus::Starting);
        service.observe(&snapshot, Vec::new());
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
