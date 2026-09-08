use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use std::io::Read;

use async_trait::async_trait;
use cockpit_core::{
    ExtensionHerdrAdapter as ExtensionHerdrAdapterTrait, ExtensionLaunch, ExtensionPaneEvidence,
    InspectionError,
};
use cockpit_protocol::context::{ContextSplitDirection, DetectionConfidence, ExtensionKind};
#[cfg(target_os = "macos")]
use libproc::proc_pid::pidpath;
#[cfg(target_os = "linux")]
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::Mutex;

use super::{HerdrCliAdapter, object, optional_string, required_array, required_string};
const MAX_SCHEMA_METHODS: usize = 256;
const MAX_SCHEMA_CACHE: usize = 8;
const MAX_MANIFEST_CACHE: usize = 8;
const MANIFEST_TTL: Duration = Duration::from_secs(5);
const MAX_RECEIPTS: usize = 64;
#[cfg(target_os = "linux")]
const MAX_PLUGIN_CONTEXT_ENV_BYTES: usize = 64 * 1024;
#[cfg(target_os = "linux")]
const MAX_PLUGIN_CONTEXT_JSON_BYTES: usize = 16 * 1024;
#[cfg(target_os = "linux")]
const MAX_PLUGIN_CONTEXT_TEXT_BYTES: usize = 4 * 1024;
#[cfg(target_os = "linux")]
const PLUGIN_CONTEXT_ENV_KEY: &[u8] = b"HERDR_PLUGIN_CONTEXT_JSON";
const FILE_VIEWER_ID: &str = "herdr-file-viewer";
const FILE_VIEWER_ENTRYPOINT: &str = "file-viewer";
const REVIEWR_ID: &str = "persiyanov.reviewr";
const REVIEWR_ENTRYPOINT: &str = "pane";
const CONTEXT_GIT_OUTPUT_BYTES: usize = 64 * 1024;
const CONTEXT_GIT_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
struct ManifestPane {
    id: String,
    command: Vec<String>,
}
#[derive(Debug, Clone)]
struct Manifest {
    plugin_id: String,
    plugin_root: PathBuf,
    enabled: bool,
    valid: bool,
    panes: Vec<ManifestPane>,
}

#[derive(Debug, Clone)]
struct ProcessRecord {
    pid: u32,
    name: String,
    argv0: Option<String>,
    argv: Option<Vec<String>>,
    cwd: Option<String>,
}

#[derive(Debug, Clone)]
struct ProcessInfo {
    foreground_processes: Vec<ProcessRecord>,
}

#[derive(Debug, Clone)]
struct LaunchReceipt {
    endpoint_identity: String,
    pane_id: String,
    terminal_id: String,
    workspace_id: String,
    tab_id: String,
    /// The cwd selected by stock `plugin_context_for_pane`: the target pane's
    /// cwd, rather than its foreground process cwd.
    viewer_cwd: Option<String>,
    process_identity: Option<String>,
    kind: ExtensionKind,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Deserialize)]
struct RawPluginContext {
    focused_pane_cwd: Option<String>,
    workspace_cwd: Option<String>,
    cwd: Option<String>,
}

#[derive(Debug)]
enum ProcessEvidence {
    SchemaAbsent,
    Unavailable,
    Available(ProcessInfo),
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
enum PluginContextValue {
    Found(Vec<u8>),
    ProcessCwdFallback,
    Unavailable,
}
#[derive(Debug, Clone)]
struct ManifestCacheEntry {
    manifests: Vec<Manifest>,
    refreshed: Instant,
}
#[derive(Debug, Default)]
struct Cache {
    schema_methods: BTreeMap<String, BTreeSet<String>>,
    schema_order: VecDeque<String>,
    manifests: BTreeMap<String, ManifestCacheEntry>,
    manifest_order: VecDeque<String>,
    receipts: BTreeMap<String, LaunchReceipt>,
    receipt_order: VecDeque<String>,
}

/// Adapter for the two stock graphical replacement targets.
///
/// All requests go through `HerdrCliAdapter`'s endpoint-pinned socket path and
/// bounded response machinery. Process evidence is deliberately reduced to an
/// executable match and a pid/start identity; argv and command lines never
/// enter `ExtensionPaneEvidence`.
#[derive(Clone)]
pub(crate) struct ExtensionHerdrAdapter {
    herdr: HerdrCliAdapter,
    cache: Arc<Mutex<Cache>>,
}

impl ExtensionHerdrAdapter {
    pub(crate) fn new(herdr: HerdrCliAdapter) -> Self {
        Self {
            herdr,
            cache: Arc::new(Mutex::new(Cache::default())),
        }
    }

    async fn schema_methods(
        &self,
        session_id: &str,
        endpoint_identity: &str,
    ) -> Result<BTreeSet<String>, InspectionError> {
        let key = cache_key(session_id, endpoint_identity);
        {
            let cache = self.cache.lock().await;
            if let Some(methods) = cache.schema_methods.get(&key) {
                return Ok(methods.clone());
            }
        }
        let schema = self
            .herdr
            .run_json_for(Some(session_id), &["api", "schema", "--json"])
            .await?;
        let (_, methods) = super::schema_fields(&schema).ok_or_else(|| {
            InspectionError::new(
                "malformed_schema",
                "Herdr schema omitted request declarations",
            )
        })?;
        if methods.len() > MAX_SCHEMA_METHODS {
            return Err(InspectionError::new(
                "bounded_output",
                "Herdr schema declares too many request methods",
            ));
        }
        let mut cache = self.cache.lock().await;
        cache.schema_methods.insert(key.clone(), methods.clone());
        cache.schema_order.retain(|existing| existing != &key);
        cache.schema_order.push_back(key);
        while cache.schema_order.len() > MAX_SCHEMA_CACHE {
            if let Some(old) = cache.schema_order.pop_front() {
                cache.schema_methods.remove(&old);
            }
        }
        Ok(methods)
    }

    async fn endpoint_identity(&self, session_id: &str) -> Result<String, InspectionError> {
        let (_, identity) = self
            .herdr
            .socket_request_with_identity(session_id, "ping", json!({}), None)
            .await?;
        Ok(identity)
    }

    async fn manifests(
        &self,
        session_id: &str,
        endpoint_identity: &str,
        methods: &BTreeSet<String>,
    ) -> Result<Option<Vec<Manifest>>, InspectionError> {
        if !methods.contains("plugin.list") {
            return Ok(None);
        }
        let key = cache_key(session_id, endpoint_identity);
        {
            let cache = self.cache.lock().await;
            if let Some(entry) = cache.manifests.get(&key)
                && entry.refreshed.elapsed() < MANIFEST_TTL
            {
                return Ok(Some(entry.manifests.clone()));
            }
        }
        let (result, _) = self
            .herdr
            .socket_request_with_identity(
                session_id,
                "plugin.list",
                json!({}),
                Some(endpoint_identity),
            )
            .await?;
        let result = object(&result, "plugin.list result")?;
        if required_string(result, "type", "plugin.list result")? != "plugin_list" {
            return Err(InspectionError::new(
                "malformed_response",
                "plugin.list returned an unexpected result type",
            ));
        }
        let plugins = required_array(result, "plugins", "plugin.list result")?;
        let mut parsed = Vec::new();
        for (index, plugin) in plugins.iter().enumerate() {
            let plugin = object(plugin, &format!("plugin.list plugins[{index}]"))?;
            let plugin_id = required_string(plugin, "plugin_id", "plugin.list plugin")?;
            let plugin_root = required_string(plugin, "plugin_root", "plugin.list plugin")?;
            let enabled = plugin
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let panes = match plugin.get("panes").and_then(Value::as_array) {
                Some(panes) => panes
                    .iter()
                    .enumerate()
                    .map(|(pane_index, pane)| {
                        let pane = object(
                            pane,
                            &format!("plugin.list plugins[{index}].panes[{pane_index}]"),
                        )?;
                        let command = required_array(pane, "command", "plugin pane")?
                            .iter()
                            .map(|value| {
                                value.as_str().map(str::to_owned).ok_or_else(|| {
                                    InspectionError::new(
                                        "malformed_response",
                                        "plugin pane command must contain strings",
                                    )
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(ManifestPane {
                            id: required_string(pane, "id", "plugin pane")?,
                            command,
                        })
                    })
                    .collect::<Result<Vec<_>, InspectionError>>()?,
                None => Vec::new(),
            };
            parsed.push(Manifest {
                plugin_id,
                plugin_root: PathBuf::from(plugin_root),
                enabled,
                valid: plugin
                    .get("warnings")
                    .map(|warnings| warnings.as_array().is_some_and(|items| items.is_empty()))
                    .unwrap_or(true),
                panes,
            });
        }
        let mut cache = self.cache.lock().await;
        cache.manifests.insert(
            key.clone(),
            ManifestCacheEntry {
                manifests: parsed.clone(),
                refreshed: Instant::now(),
            },
        );
        cache.manifest_order.retain(|existing| existing != &key);
        cache.manifest_order.push_back(key.clone());
        while cache.manifest_order.len() > MAX_MANIFEST_CACHE {
            if let Some(old) = cache.manifest_order.pop_front() {
                cache.manifests.remove(&old);
            }
        }
        Ok(Some(parsed))
    }

    async fn process_info(
        &self,
        session_id: &str,
        pane_id: &str,
        endpoint_identity: &str,
        methods: &BTreeSet<String>,
    ) -> Result<ProcessEvidence, InspectionError> {
        if !methods.contains("pane.process_info") {
            return Ok(ProcessEvidence::SchemaAbsent);
        }
        let (result, _) = self
            .herdr
            .socket_request_with_identity(
                session_id,
                "pane.process_info",
                json!({"pane_id": pane_id}),
                Some(endpoint_identity),
            )
            .await
            .or_else(|error| {
                if is_unsupported_capability(&error) {
                    Ok((Value::Null, endpoint_identity.to_owned()))
                } else {
                    Err(error)
                }
            })?;
        if result.is_null() {
            return Ok(ProcessEvidence::Unavailable);
        }
        let result = object(&result, "pane.process_info result")?;
        if required_string(result, "type", "pane.process_info result")? != "pane_process_info" {
            return Err(InspectionError::new(
                "malformed_response",
                "pane.process_info returned an unexpected result type",
            ));
        }
        let info = object(
            result.get("process_info").ok_or_else(|| {
                InspectionError::new("malformed_response", "process_info is required")
            })?,
            "process_info",
        )?;
        let reported_pane = required_string(info, "pane_id", "process_info")?;
        if reported_pane != pane_id {
            return Err(InspectionError::new(
                "stale_identity",
                "process_info belongs to another pane",
            ));
        }
        let processes = info
            .get("foreground_processes")
            .and_then(Value::as_array)
            .map(|processes| {
                processes
                    .iter()
                    .enumerate()
                    .map(|(index, process)| parse_process(process, index))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect();
        Ok(ProcessEvidence::Available(ProcessInfo {
            foreground_processes: processes,
        }))
    }

    async fn remember_receipt(&self, receipt: LaunchReceipt) {
        let key = receipt_key(&receipt.endpoint_identity, &receipt.pane_id);
        let mut cache = self.cache.lock().await;
        cache.receipts.insert(key.clone(), receipt);
        cache.receipt_order.push_back(key);
        while cache.receipt_order.len() > MAX_RECEIPTS {
            if let Some(old) = cache.receipt_order.pop_front() {
                cache.receipts.remove(&old);
            }
        }
    }

    async fn receipt(&self, endpoint_identity: &str, pane_id: &str) -> Option<LaunchReceipt> {
        self.cache
            .lock()
            .await
            .receipts
            .get(&receipt_key(endpoint_identity, pane_id))
            .cloned()
    }

    fn target_manifest<'a>(
        manifests: &'a [Manifest],
        kind: ExtensionKind,
    ) -> Option<(&'a Manifest, &'a ManifestPane)> {
        let (plugin_id, entrypoint) = match kind {
            ExtensionKind::Context => (FILE_VIEWER_ID, FILE_VIEWER_ENTRYPOINT),
            ExtensionKind::Review => (REVIEWR_ID, REVIEWR_ENTRYPOINT),
        };
        manifests.iter().find_map(|manifest| {
            if manifest.plugin_id != plugin_id || !manifest.enabled || !manifest.valid {
                return None;
            }
            manifest
                .panes
                .iter()
                .find(|pane| pane.id == entrypoint)
                .map(|pane| (manifest, pane))
        })
    }

    fn matching_process<'a>(
        process_info: &'a ProcessInfo,
        manifest: &Manifest,
        pane: &ManifestPane,
    ) -> Option<&'a ProcessRecord> {
        let expected = expected_executable(manifest, pane)?;
        let mut matches = process_info
            .foreground_processes
            .iter()
            .filter(|process| process_matches(process, &expected));
        let process = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(process)
    }

    fn classify_process(
        &self,
        process_info: &ProcessInfo,
        manifest: &Manifest,
        pane: &ManifestPane,
    ) -> Option<(String, Option<String>)> {
        let process = Self::matching_process(process_info, manifest, pane)?;
        // A PID without a kernel-reported generation is not an identity. Never
        // turn that unavailable value into a literal receipt component.
        let start = HerdrCliAdapter::process_start_identity(Some(process.pid as i32))?;
        Some((
            format!("pid={}:start={start}", process.pid),
            process.cwd.clone(),
        ))
    }
    async fn inspect_inner(
        &self,
        session_id: &str,
        pane_id: &str,
    ) -> Result<ExtensionPaneEvidence, InspectionError> {
        self.herdr.selected_session(session_id)?;
        if !super::valid_pane_id(pane_id) {
            return Err(InspectionError::new(
                "invalid_pane_id",
                "pane ID contains unsupported characters",
            ));
        }
        let endpoint_identity = self.endpoint_identity(session_id).await?;
        let methods = self.schema_methods(session_id, &endpoint_identity).await?;
        let snapshot = self
            .herdr
            .read_structure_with_identity(session_id, Some(&endpoint_identity))
            .await?;
        let pane = snapshot
            .panes
            .iter()
            .find(|pane| pane.id == pane_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "pane_not_found",
                    "pane not found in the authoritative snapshot",
                )
            })?;
        let manifests = self
            .manifests(session_id, &endpoint_identity, &methods)
            .await?;
        let process_evidence = self
            .process_info(session_id, pane_id, &endpoint_identity, &methods)
            .await?;
        let (pane_result, _) = self
            .herdr
            .socket_request_with_identity(
                session_id,
                "pane.get",
                json!({"pane_id": pane_id}),
                Some(&endpoint_identity),
            )
            .await?;
        let pane_result = object(&pane_result, "pane.get result")?;
        let pane_metadata = object(
            pane_result.get("pane").ok_or_else(|| {
                InspectionError::new("malformed_response", "pane.get omitted pane metadata")
            })?,
            "pane metadata",
        )?;
        if required_string(pane_metadata, "pane_id", "pane metadata")? != pane.id
            || required_string(pane_metadata, "terminal_id", "pane metadata")? != pane.terminal_id
        {
            return Err(InspectionError::new(
                "stale_identity",
                "pane changed during extension inspection",
            ));
        }

        let title = pane.title.as_deref().unwrap_or_default();
        let process_info = match &process_evidence {
            ProcessEvidence::Available(info) => Some(info),
            ProcessEvidence::SchemaAbsent | ProcessEvidence::Unavailable => None,
        };

        let mut kind = None;
        let mut manifest_target = None;
        if let (Some(manifests), Some(info)) = (manifests.as_deref(), process_info) {
            for candidate in [ExtensionKind::Context, ExtensionKind::Review] {
                if let Some((manifest, manifest_pane)) = Self::target_manifest(manifests, candidate)
                    && self
                        .classify_process(info, manifest, manifest_pane)
                        .is_some()
                {
                    kind = Some(candidate);
                    manifest_target = Some((manifest, manifest_pane));
                    break;
                }
            }
        }
        if kind.is_none() {
            kind = process_info.and_then(candidate_kind_from_process);
        }
        if kind.is_none() {
            kind = if title == "Files" {
                Some(ExtensionKind::Context)
            } else if title.eq_ignore_ascii_case("reviewr") {
                Some(ExtensionKind::Review)
            } else {
                None
            };
        }

        let mut confidence = DetectionConfidence::None;
        let mut reason = "pane is not a supported extension".to_owned();
        let mut process_hasher = std::collections::hash_map::DefaultHasher::new();
        if let Some(info) = process_info {
            for process in &info.foreground_processes {
                process.pid.hash(&mut process_hasher);
                HerdrCliAdapter::process_start_identity(Some(process.pid as i32))
                    .hash(&mut process_hasher);
                process.argv0.hash(&mut process_hasher);
                process.argv.hash(&mut process_hasher);
                process.cwd.hash(&mut process_hasher);
            }
        }
        let mut process_identity = format!("process:{:016x}", process_hasher.finish());
        let mut cwd = optional_string(pane_metadata, "cwd", "pane metadata")?;
        let mut foreground_cwd = optional_string(pane_metadata, "foreground_cwd", "pane metadata")?;
        let mut viewer_cwd = None;
        let can_open_context = methods.contains("plugin.pane.open")
            && manifests.as_deref().is_some_and(|manifests| {
                Self::target_manifest(manifests, ExtensionKind::Context).is_some()
            });
        let can_open_review = methods.contains("plugin.pane.open")
            && manifests.as_deref().is_some_and(|manifests| {
                Self::target_manifest(manifests, ExtensionKind::Review).is_some()
            });
        if let Some((manifest, manifest_pane)) = manifest_target {
            if let Some(info) = process_info
                && let Some(process) = Self::matching_process(info, manifest, manifest_pane)
                && let Some((identity, process_cwd)) =
                    self.classify_process(info, manifest, manifest_pane)
            {
                confidence = DetectionConfidence::VerifiedProcess;
                reason = "supported executable and process generation verified in the live foreground process list".to_owned();
                process_identity = identity;
                if kind == Some(ExtensionKind::Context) {
                    viewer_cwd = viewer_cwd_from_process(process, &process_identity);
                } else if kind == Some(ExtensionKind::Review) {
                    cwd = process_cwd.clone();
                    foreground_cwd = process_cwd;
                }
            } else {
                confidence = DetectionConfidence::Candidate;
                reason = match &process_evidence {
                    ProcessEvidence::SchemaAbsent => "installed entrypoint is known, but this endpoint does not expose process inspection".to_owned(),
                    ProcessEvidence::Unavailable => "installed entrypoint is known, but live process evidence is temporarily unavailable".to_owned(),
                    ProcessEvidence::Available(_) => "installed entrypoint is known, but live executable or process-generation evidence is unavailable".to_owned(),
                };
            }
        } else if let Some(kind_value) = kind {
            confidence = DetectionConfidence::Candidate;
            reason = match kind_value {
                ExtensionKind::Context => "pane title matches the file-viewer candidate; executable and process-generation evidence are required".to_owned(),
                ExtensionKind::Review => "pane title matches the Reviewr candidate; executable and process-generation evidence are required".to_owned(),
            };
        } else if !can_open_context && !can_open_review && manifests.is_some() {
            confidence = DetectionConfidence::Unsupported;
            reason = "supported extension is not installed and enabled at this endpoint".to_owned();
        }

        if let Some(receipt) = self.receipt(&endpoint_identity, pane_id).await
            && receipt.terminal_id == pane.terminal_id
            && receipt.workspace_id == pane.space_id
            && receipt.tab_id == pane.tab_id
        {
            let verified = match (
                receipt.process_identity.as_deref(),
                process_info,
                manifests
                    .as_deref()
                    .and_then(|items| Self::target_manifest(items, receipt.kind)),
            ) {
                (Some(expected), Some(info), Some((manifest, manifest_pane))) => self
                    .classify_process(info, manifest, manifest_pane)
                    .is_some_and(|(identity, _)| identity == expected),
                _ => false,
            };
            if verified {
                confidence = DetectionConfidence::VerifiedLaunch;
                reason = "verified plugin launch receipt matches the subsequent authoritative snapshot and process generation".to_owned();
                process_identity = receipt
                    .process_identity
                    .clone()
                    .expect("verified receipt has process identity");
                kind = Some(receipt.kind);
                viewer_cwd = if receipt.kind == ExtensionKind::Context {
                    receipt.viewer_cwd.clone()
                } else {
                    None
                };
            } else if process_info.is_none() {
                // Retain the launch as a candidate; missing live process
                // evidence cannot authorize graphical replacement.
                confidence = DetectionConfidence::Candidate;
                reason = match &process_evidence {
                    ProcessEvidence::SchemaAbsent => "known plugin launch receipt retained as a candidate because this endpoint does not expose process inspection".to_owned(),
                    ProcessEvidence::Unavailable => "known plugin launch receipt retained as a candidate because live process evidence is temporarily unavailable".to_owned(),
                    ProcessEvidence::Available(_) => unreachable!("process_info is Some above"),
                };
                process_identity = receipt.process_identity.clone().unwrap_or(process_identity);
                kind = Some(receipt.kind);
                viewer_cwd = if receipt.kind == ExtensionKind::Context {
                    receipt.viewer_cwd.clone()
                } else {
                    None
                };
            }
        }

        Ok(ExtensionPaneEvidence {
            endpoint_identity,
            pane_id: pane.id.clone(),
            terminal_id: pane.terminal_id.clone(),
            workspace_id: pane.space_id.clone(),
            tab_id: pane.tab_id.clone(),
            cwd,
            foreground_cwd,
            viewer_cwd,
            process_identity,
            extension: kind,
            confidence,
            reason,
            can_open_context,
            can_open_review,
        })
    }

    async fn launch_inner(
        &self,
        session_id: &str,
        request: &ExtensionLaunch,
        kind: ExtensionKind,
    ) -> Result<ExtensionPaneEvidence, InspectionError> {
        self.herdr.selected_session(session_id)?;
        if !super::valid_pane_id(&request.pane_id)
            || request.cwd.is_empty()
            || !Path::new(&request.cwd).is_absolute()
            || request.cwd.contains('\0')
        {
            return Err(InspectionError::new(
                "invalid_extension_launch",
                "extension launch target is invalid",
            ));
        }
        let endpoint_identity = self.endpoint_identity(session_id).await?;
        if endpoint_identity != request.endpoint_identity {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr endpoint changed before extension launch",
            ));
        }
        let methods = self.schema_methods(session_id, &endpoint_identity).await?;
        for method in ["plugin.list", "plugin.pane.open"] {
            if !methods.contains(method) {
                return Err(InspectionError::new(
                    "unsupported_capability",
                    format!("Herdr does not expose {method}"),
                ));
            }
        }
        let snapshot = self
            .herdr
            .read_snapshot_with_identity(session_id, Some(&endpoint_identity))
            .await?;
        let target = snapshot
            .panes
            .iter()
            .find(|pane| pane.id == request.pane_id)
            .ok_or_else(|| {
                InspectionError::new("pane_not_found", "launch target pane is not present")
            })?;
        if target.space_id != request.workspace_id || target.terminal_id != request.terminal_id {
            return Err(InspectionError::new(
                "stale_identity",
                "extension launch target identity changed",
            ));
        }
        let manifests = self
            .manifests(session_id, &endpoint_identity, &methods)
            .await?
            .ok_or_else(|| {
                InspectionError::new("unsupported_capability", "plugin.list is unavailable")
            })?;
        let (manifest, pane) = Self::target_manifest(&manifests, kind).ok_or_else(|| {
            InspectionError::new(
                "unsupported_capability",
                "installed extension entrypoint is unavailable",
            )
        })?;
        let (plugin_id, entrypoint) = match kind {
            ExtensionKind::Context => (FILE_VIEWER_ID, FILE_VIEWER_ENTRYPOINT),
            ExtensionKind::Review => (REVIEWR_ID, REVIEWR_ENTRYPOINT),
        };

        // Re-read the placement target immediately before opening. The plugin
        // cwd is the separately authorized request cwd, so a Context companion
        // can launch Reviewr for its associated checkout.
        let (target_result, _) = self
            .herdr
            .socket_request_with_identity(
                session_id,
                "pane.get",
                json!({"pane_id": request.pane_id}),
                Some(&endpoint_identity),
            )
            .await?;
        let target_result = object(&target_result, "target pane.get result")?;
        let target_metadata = object(
            target_result.get("pane").ok_or_else(|| {
                InspectionError::new(
                    "malformed_response",
                    "target pane.get omitted pane metadata",
                )
            })?,
            "target pane metadata",
        )?;
        if required_string(target_metadata, "pane_id", "target pane metadata")? != target.id
            || required_string(target_metadata, "terminal_id", "target pane metadata")?
                != target.terminal_id
            || required_string(target_metadata, "workspace_id", "target pane metadata")?
                != target.space_id
            || required_string(target_metadata, "tab_id", "target pane metadata")? != target.tab_id
        {
            return Err(InspectionError::new(
                "stale_identity",
                "extension launch target changed before plugin open",
            ));
        }
        if kind == ExtensionKind::Context {
            let source_cwd =
                optional_string(target_metadata, "foreground_cwd", "target pane metadata")?.or(
                    optional_string(target_metadata, "cwd", "target pane metadata")?,
                );
            let source_matches = match source_cwd.as_deref() {
                Some(cwd) => context_cwd_matches_approved(cwd, &request.cwd).await,
                None => false,
            };
            if !source_matches {
                return Err(InspectionError::new(
                    "stale_identity",
                    "source pane cwd no longer resolves to the approved file-viewer root",
                ));
            }
        }
        let direction = match request.direction {
            ContextSplitDirection::Right => "right",
            ContextSplitDirection::Down => "down",
        };
        let parameters = plugin_open_parameters(request, plugin_id, entrypoint, direction, kind);
        let (result, _) = self
            .herdr
            .socket_request_with_identity(
                session_id,
                "plugin.pane.open",
                parameters,
                Some(&endpoint_identity),
            )
            .await?;
        let opened = object(&result, "plugin.pane.open result").map_err(post_mutation_error)?;
        if required_string(opened, "type", "plugin.pane.open result")
            .map_err(post_mutation_error)?
            != "plugin_pane_opened"
        {
            return Err(post_mutation_error(InspectionError::new(
                "malformed_response",
                "plugin pane open result is invalid",
            )));
        }
        let plugin_pane = object(
            opened
                .get("plugin_pane")
                .ok_or_else(|| {
                    InspectionError::new("malformed_response", "plugin_pane is required")
                })
                .map_err(post_mutation_error)?,
            "plugin_pane",
        )
        .map_err(post_mutation_error)?;
        if required_string(plugin_pane, "plugin_id", "plugin_pane").map_err(post_mutation_error)?
            != plugin_id
            || required_string(plugin_pane, "entrypoint", "plugin_pane")
                .map_err(post_mutation_error)?
                != entrypoint
        {
            return Err(post_mutation_error(InspectionError::new(
                "stale_identity",
                "launch receipt names an unexpected plugin entrypoint",
            )));
        }
        let opened_pane = object(
            plugin_pane
                .get("pane")
                .ok_or_else(|| {
                    InspectionError::new("malformed_response", "plugin_pane.pane is required")
                })
                .map_err(post_mutation_error)?,
            "plugin pane",
        )
        .map_err(post_mutation_error)?;
        let pane_id =
            required_string(opened_pane, "pane_id", "plugin pane").map_err(post_mutation_error)?;
        let terminal_id = required_string(opened_pane, "terminal_id", "plugin pane")
            .map_err(post_mutation_error)?;
        let workspace_id = required_string(opened_pane, "workspace_id", "plugin pane")
            .map_err(post_mutation_error)?;
        let tab_id =
            required_string(opened_pane, "tab_id", "plugin pane").map_err(post_mutation_error)?;
        let opened_cwd =
            optional_string(opened_pane, "cwd", "plugin pane").map_err(post_mutation_error)?;
        let opened_cwd_matches = match kind {
            ExtensionKind::Context => opened_cwd.as_deref().is_some_and(|cwd| {
                actual_cwd_matches(cwd, &manifest.plugin_root.to_string_lossy())
            }),
            ExtensionKind::Review => opened_cwd
                .as_deref()
                .is_some_and(|cwd| actual_cwd_matches(cwd, &request.cwd)),
        };
        if !opened_cwd_matches {
            return Err(post_mutation_error(InspectionError::new(
                "launch_receipt_unverified",
                "opened extension cwd does not match the installed plugin or authorized checkout",
            )));
        }
        let subsequent = self
            .herdr
            .read_snapshot_with_identity(session_id, Some(&endpoint_identity))
            .await
            .map_err(post_mutation_error)?;
        let snapshot_pane = subsequent
            .panes
            .iter()
            .find(|candidate| candidate.id == pane_id)
            .ok_or_else(|| {
                post_mutation_error(InspectionError::new(
                    "launch_receipt_unverified",
                    "opened pane is absent from the subsequent snapshot",
                ))
            })?;
        if snapshot_pane.terminal_id != terminal_id
            || snapshot_pane.space_id != workspace_id
            || snapshot_pane.tab_id != tab_id
            || workspace_id != request.workspace_id
            || pane_id == request.pane_id
        {
            return Err(post_mutation_error(InspectionError::new(
                "launch_receipt_unverified",
                "opened pane identity changed before confirmation",
            )));
        }
        let process_evidence = self
            .process_info(session_id, &pane_id, &endpoint_identity, &methods)
            .await
            .map_err(post_mutation_error)?;
        let (process_identity, confidence, reason, viewer_cwd) = match process_evidence {
            ProcessEvidence::Available(info) => {
                let process = Self::matching_process(&info, manifest, pane).ok_or_else(|| {
                    post_mutation_error(InspectionError::new(
                        "launch_receipt_unverified",
                        "opened pane executable or process generation does not match the installed entrypoint",
                    ))
                })?;
                let (identity, _) = self.classify_process(&info, manifest, pane).expect("matched process is classifiable");
                let viewer_cwd = if kind == ExtensionKind::Context {
                    let viewer_cwd = match viewer_context_from_process(process, &identity) {
                        Some(viewer_cwd) => viewer_cwd,
                        #[cfg(target_os = "macos")]
                        None => {
                            // macOS does not expose HERDR_PLUGIN_CONTEXT_JSON.
                            // This fallback is safe only after the launch's
                            // executable and generation checks above: request.cwd
                            // was validated against the source pane immediately
                            // before this plugin mutation.
                            request.cwd.clone()
                        }
                        #[cfg(not(target_os = "macos"))]
                        None => {
                            return Err(post_mutation_error(InspectionError::new(
                                "launch_receipt_unverified",
                                "opened file viewer did not expose a verified plugin context",
                            )));
                        }
                    };
                    if !context_cwd_matches_approved(&viewer_cwd, &request.cwd).await {
                        return Err(post_mutation_error(InspectionError::new(
                            "launch_receipt_unverified",
                            "opened file viewer context does not resolve to the approved browsing root",
                        )));
                    }
                    Some(viewer_cwd)
                } else {
                    None
                };
                (
                    identity,
                    DetectionConfidence::VerifiedLaunch,
                    "plugin launch receipt verified against the subsequent authoritative snapshot and process generation".to_owned(),
                    viewer_cwd,
                )
            }
            ProcessEvidence::SchemaAbsent => (
                "process:unknown-generation".to_owned(),
                DetectionConfidence::Candidate,
                "plugin launch confirmed, but this endpoint does not expose process inspection; retaining a candidate receipt".to_owned(),
                None,
            ),
            ProcessEvidence::Unavailable => (
                "process:unknown-generation".to_owned(),
                DetectionConfidence::Candidate,
                "plugin launch confirmed, but live process evidence is temporarily unavailable; retaining a candidate receipt".to_owned(),
                None,
            ),
        };
        let receipt = LaunchReceipt {
            endpoint_identity: endpoint_identity.clone(),
            pane_id: pane_id.clone(),
            terminal_id: terminal_id.clone(),
            workspace_id: workspace_id.clone(),
            tab_id: tab_id.clone(),
            viewer_cwd: if kind == ExtensionKind::Context {
                viewer_cwd.clone()
            } else {
                None
            },
            process_identity: if confidence == DetectionConfidence::VerifiedLaunch {
                Some(process_identity.clone())
            } else {
                None
            },
            kind,
        };
        self.remember_receipt(receipt).await;
        Ok(ExtensionPaneEvidence {
            endpoint_identity,
            pane_id,
            terminal_id,
            workspace_id,
            tab_id,
            cwd: opened_cwd,
            foreground_cwd: optional_string(opened_pane, "foreground_cwd", "plugin pane")
                .map_err(post_mutation_error)?,
            viewer_cwd,
            process_identity,
            extension: Some(kind),
            confidence,
            reason,
            can_open_context: Self::target_manifest(&manifests, ExtensionKind::Context).is_some(),
            can_open_review: Self::target_manifest(&manifests, ExtensionKind::Review).is_some(),
        })
    }
}

#[async_trait]
impl ExtensionHerdrAdapterTrait for ExtensionHerdrAdapter {
    async fn inspect_extension_pane(
        &self,
        session_id: &str,
        pane_id: &str,
    ) -> Result<ExtensionPaneEvidence, InspectionError> {
        self.inspect_inner(session_id, pane_id).await
    }

    async fn launch_context_pane(
        &self,
        session_id: &str,
        request: &ExtensionLaunch,
    ) -> Result<ExtensionPaneEvidence, InspectionError> {
        self.launch_inner(session_id, request, ExtensionKind::Context)
            .await
    }

    async fn launch_review_pane(
        &self,
        session_id: &str,
        request: &ExtensionLaunch,
    ) -> Result<ExtensionPaneEvidence, InspectionError> {
        self.launch_inner(session_id, request, ExtensionKind::Review)
            .await
    }
}

fn plugin_open_parameters(
    request: &ExtensionLaunch,
    plugin_id: &str,
    entrypoint: &str,
    direction: &str,
    kind: ExtensionKind,
) -> Value {
    let mut parameters = json!({
        "plugin_id": plugin_id,
        "entrypoint": entrypoint,
        "placement": "split",
        "target_pane_id": request.pane_id,
        "direction": direction,
        "focus": true,
    });
    if kind == ExtensionKind::Review {
        parameters["cwd"] = Value::String(request.cwd.clone());
    }
    parameters
}

fn actual_cwd_matches(observed: &str, approved: &str) -> bool {
    if !Path::new(observed).is_absolute() || !Path::new(approved).is_absolute() {
        return false;
    }
    canonical_or_original(Path::new(observed), None)
        == canonical_or_original(Path::new(approved), None)
}

async fn context_cwd_matches_approved(observed: &str, approved: &str) -> bool {
    let Some(root) = context_root_for_cwd(observed).await else {
        return false;
    };
    actual_cwd_matches(&root, approved)
}

async fn context_root_for_cwd(cwd: &str) -> Option<String> {
    let cwd_path = Path::new(cwd);
    if !cwd_path.is_absolute() || cwd.contains('\0') || git_metadata_path(cwd_path) {
        return None;
    }
    let mut command = Command::new("git");
    command
        .current_dir(cwd_path)
        .arg("--attr-source=4b825dc642cb6eb9a060e54bf8d69288fbee4904")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .args(["rev-parse", "--show-toplevel"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "");
    let output = cockpit_core::process::run_bounded_command(
        command,
        CONTEXT_GIT_OUTPUT_BYTES,
        CONTEXT_GIT_OUTPUT_BYTES,
        CONTEXT_GIT_TIMEOUT,
        "file_viewer_root_resolution",
    )
    .await
    .ok()?;
    if !output.status.success() {
        return Some(cwd.to_owned());
    }
    let root = std::str::from_utf8(&output.stdout).ok()?.trim();
    if root.is_empty() || root.lines().count() != 1 || root.chars().any(char::is_control) {
        return None;
    }
    let root_path = Path::new(root);
    if !root_path.is_absolute() || git_metadata_path(root_path) {
        return None;
    }
    Some(root.to_owned())
}

fn git_metadata_path(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => name == ".git",
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(plugin_id: &str, pane: &str) -> Manifest {
        Manifest {
            plugin_id: plugin_id.to_owned(),
            plugin_root: PathBuf::from("/plugins/test"),
            enabled: true,
            valid: true,
            panes: vec![ManifestPane {
                id: pane.to_owned(),
                command: Vec::new(),
            }],
        }
    }

    #[test]
    fn review_launch_uses_only_the_declared_reviewr_pane() {
        let manifests = vec![
            manifest(FILE_VIEWER_ID, FILE_VIEWER_ENTRYPOINT),
            manifest(REVIEWR_ID, REVIEWR_ENTRYPOINT),
            manifest(REVIEWR_ID, "other"),
        ];
        let (plugin, pane) =
            ExtensionHerdrAdapter::target_manifest(&manifests, ExtensionKind::Review)
                .expect("Reviewr pane is present");
        assert_eq!(plugin.plugin_id, REVIEWR_ID);
        assert_eq!(pane.id, REVIEWR_ENTRYPOINT);
        assert!(
            ExtensionHerdrAdapter::target_manifest(
                &[manifest(REVIEWR_ID, "other")],
                ExtensionKind::Review
            )
            .is_none()
        );
    }

    #[test]
    fn context_launch_omits_cwd_while_review_keeps_the_authorized_checkout() {
        let request = ExtensionLaunch {
            endpoint_identity: "endpoint".to_owned(),
            pane_id: "pane".to_owned(),
            terminal_id: "terminal".to_owned(),
            workspace_id: "workspace".to_owned(),
            cwd: "/approved/root".to_owned(),
            direction: ContextSplitDirection::Right,
        };
        let context = plugin_open_parameters(
            &request,
            FILE_VIEWER_ID,
            FILE_VIEWER_ENTRYPOINT,
            "right",
            ExtensionKind::Context,
        );
        assert!(context.get("cwd").is_none());
        assert!(context.get("workspace_id").is_none());
        let review = plugin_open_parameters(
            &request,
            REVIEWR_ID,
            REVIEWR_ENTRYPOINT,
            "right",
            ExtensionKind::Review,
        );
        assert_eq!(
            review.get("cwd").and_then(Value::as_str),
            Some("/approved/root")
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn process_matching_uses_argv_zero_when_argv0_is_a_basename() {
        let expected = PathBuf::from("/plugins/test/bin/herdr-reviewr");
        let process = ProcessRecord {
            pid: if cfg!(target_os = "macos") {
                i32::MAX as u32
            } else {
                1
            },
            name: "herdr-reviewr".to_owned(),
            argv0: Some("herdr-reviewr".to_owned()),
            argv: Some(vec![expected.to_string_lossy().into_owned()]),
            cwd: Some("/plugins/test".to_owned()),
        };
        assert!(process_matches(&process, &expected));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn process_matching_ignores_plugin_path_in_non_executable_argument() {
        let expected = PathBuf::from("/plugins/test/bin/herdr-reviewr");
        let process = ProcessRecord {
            pid: if cfg!(target_os = "macos") {
                i32::MAX as u32
            } else {
                1
            },
            name: "sh".to_owned(),
            argv0: Some("/bin/sh".to_owned()),
            argv: Some(vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                expected.to_string_lossy().into_owned(),
            ]),
            cwd: Some("/plugins/test".to_owned()),
        };
        assert!(!process_matches(&process, &expected));
    }
    #[tokio::test]
    async fn context_root_uses_git_top_level_and_refuses_git_metadata() {
        let unique = format!(
            "cockpit-herdr-context-root-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let repository = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(repository.join("nested")).expect("fixture directory");
        let init = std::process::Command::new("git")
            .arg("init")
            .arg(&repository)
            .output()
            .expect("git starts");
        assert!(
            init.status.success(),
            "git init: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        assert_eq!(
            context_root_for_cwd(repository.join("nested").to_str().expect("utf8 path")).await,
            Some(repository.to_string_lossy().into_owned())
        );
        assert_eq!(
            context_root_for_cwd(repository.join(".git").to_str().expect("utf8 path")).await,
            None
        );
        std::fs::remove_dir_all(repository).expect("cleanup");
    }
}

fn viewer_cwd_from_process(process: &ProcessRecord, process_identity: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let pid = i32::try_from(process.pid).ok()?;
        let start_before = HerdrCliAdapter::process_start_identity(Some(pid))?;
        let (_, expected_start) = process_identity.split_once(":start=")?;
        if start_before != expected_start.parse::<u64>().ok()? {
            return None;
        }
        let value = read_plugin_context_value(process.pid);
        let start_after = HerdrCliAdapter::process_start_identity(Some(pid))?;
        if start_before != start_after {
            return None;
        }
        return match value {
            PluginContextValue::Found(bytes) => {
                parse_plugin_context(&bytes).or_else(|| process.cwd.clone())
            }
            PluginContextValue::ProcessCwdFallback => process.cwd.clone(),
            PluginContextValue::Unavailable => None,
        };
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (process, process_identity);
        None
    }
}

fn viewer_context_from_process(process: &ProcessRecord, process_identity: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let pid = i32::try_from(process.pid).ok()?;
        let start_before = HerdrCliAdapter::process_start_identity(Some(pid))?;
        let (_, expected_start) = process_identity.split_once(":start=")?;
        if start_before != expected_start.parse::<u64>().ok()? {
            return None;
        }
        let value = read_plugin_context_value(process.pid);
        let start_after = HerdrCliAdapter::process_start_identity(Some(pid))?;
        if start_before != start_after {
            return None;
        }
        return match value {
            PluginContextValue::Found(bytes) => parse_plugin_context(&bytes),
            PluginContextValue::ProcessCwdFallback | PluginContextValue::Unavailable => None,
        };
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (process, process_identity);
        None
    }
}

#[cfg(target_os = "linux")]
fn read_plugin_context_value(pid: u32) -> PluginContextValue {
    let mut file = match std::fs::File::open(format!("/proc/{pid}/environ")) {
        Ok(file) => file,
        Err(_) => return PluginContextValue::Unavailable,
    };
    let mut buffer = [0_u8; 4096];
    let mut bytes_read = 0usize;
    let mut key_index = 0usize;
    let mut candidate = true;
    let mut target = false;
    let mut value = Vec::new();
    loop {
        let remaining = MAX_PLUGIN_CONTEXT_ENV_BYTES.saturating_sub(bytes_read);
        if remaining == 0 {
            // The cap is intentionally strict: without observing a NUL or EOF
            // within it, absence of the documented entry is not demonstrable.
            return PluginContextValue::Unavailable;
        }
        let read_length = buffer.len().min(remaining);
        let read_buffer = &mut buffer[..read_length];
        let count = match file.read(read_buffer) {
            Ok(count) => count,
            Err(_) => return PluginContextValue::Unavailable,
        };
        if count == 0 {
            break;
        }
        bytes_read += count;
        for byte in &read_buffer[..count] {
            if *byte == 0 {
                if target {
                    return PluginContextValue::Found(value);
                }
                key_index = 0;
                candidate = true;
                target = false;
                value.clear();
                continue;
            }
            if target {
                if value.len() <= MAX_PLUGIN_CONTEXT_JSON_BYTES {
                    value.push(*byte);
                }
                continue;
            }
            if candidate {
                if *byte == b'=' {
                    target = key_index == PLUGIN_CONTEXT_ENV_KEY.len();
                    candidate = false;
                } else if key_index >= PLUGIN_CONTEXT_ENV_KEY.len()
                    || *byte != PLUGIN_CONTEXT_ENV_KEY[key_index]
                {
                    candidate = false;
                } else {
                    key_index += 1;
                }
            }
        }
    }
    if target {
        PluginContextValue::Found(value)
    } else {
        PluginContextValue::ProcessCwdFallback
    }
}

#[cfg(target_os = "linux")]
fn parse_plugin_context(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_PLUGIN_CONTEXT_JSON_BYTES {
        return None;
    }
    let raw = serde_json::from_slice::<RawPluginContext>(bytes).ok()?;
    for path in [raw.focused_pane_cwd, raw.workspace_cwd, raw.cwd]
        .into_iter()
        .flatten()
    {
        if path.is_empty() {
            continue;
        }
        return (path.len() <= MAX_PLUGIN_CONTEXT_TEXT_BYTES).then_some(path);
    }
    None
}

fn parse_process(value: &Value, index: usize) -> Result<Option<ProcessRecord>, InspectionError> {
    let process = object(value, &format!("foreground_processes[{index}]"))?;
    let Some(pid) = process.get("pid").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let name = required_string(process, "name", "process")?;
    let pid = u32::try_from(pid)
        .map_err(|_| InspectionError::new("malformed_response", "process pid exceeds u32"))?;
    let argv0 = optional_string(process, "argv0", "process")?;
    let argv = process
        .get("argv")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| {
                    InspectionError::new("malformed_response", "process argv must be an array")
                })?
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        InspectionError::new(
                            "malformed_response",
                            "process argv must contain strings",
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let cwd = optional_string(process, "cwd", "process")?;
    Ok(Some(ProcessRecord {
        pid,
        name,
        argv0,
        argv,
        cwd,
    }))
}
fn candidate_kind_from_process(process_info: &ProcessInfo) -> Option<ExtensionKind> {
    process_info
        .foreground_processes
        .iter()
        .find_map(|process| match process.name.as_str() {
            "herdr-file-viewer" => Some(ExtensionKind::Context),
            "herdr-reviewr" => Some(ExtensionKind::Review),
            _ => None,
        })
}

fn expected_executable(manifest: &Manifest, pane: &ManifestPane) -> Option<PathBuf> {
    if manifest.plugin_id == FILE_VIEWER_ID
        && pane.id == FILE_VIEWER_ENTRYPOINT
        && pane.command.len() == 1
    {
        let command = PathBuf::from(&pane.command[0]);
        if command.file_name().and_then(|name| name.to_str()) != Some("herdr-file-viewer") {
            return None;
        }
        return Some(if command.is_absolute() {
            command
        } else {
            manifest.plugin_root.join(command)
        });
    }
    if manifest.plugin_id == REVIEWR_ID
        && pane.id == REVIEWR_ENTRYPOINT
        && pane.command.len() == 3
        && pane.command[0] == "sh"
        && pane.command[1] == "-c"
        && pane.command[2] == "exec \"$HERDR_PLUGIN_ROOT/bin/herdr-reviewr\""
    {
        return Some(manifest.plugin_root.join("bin/herdr-reviewr"));
    }
    None
}

fn process_matches(process: &ProcessRecord, expected: &Path) -> bool {
    let expected = canonical_or_original(expected, None);
    #[cfg(target_os = "linux")]
    {
        let Ok(pid) = i32::try_from(process.pid) else {
            return false;
        };
        let Some(start) = HerdrCliAdapter::process_start_identity(Some(pid)) else {
            return false;
        };
        return std::fs::read_link(format!("/proc/{pid}/exe"))
            .is_ok_and(|executable| executable == expected)
            && HerdrCliAdapter::process_start_identity(Some(pid)) == Some(start);
    }
    #[cfg(target_os = "macos")]
    {
        let Ok(pid) = i32::try_from(process.pid) else {
            return false;
        };
        // Prefer Darwin's kernel-reported executable path. If process
        // inspection is unavailable, fall back only to argv[0]/argv0.
        if let Ok(executable) = pidpath(pid) {
            return canonical_or_original(Path::new(&executable), None) == expected;
        }
        let candidates = process
            .argv
            .as_deref()
            .and_then(|argv| argv.first())
            .map(String::as_str)
            .into_iter()
            .chain(process.argv0.iter().map(String::as_str));
        return candidates
            .filter(|candidate| candidate.contains('/') || candidate.contains('\\'))
            .any(|candidate| {
                canonical_or_original(Path::new(candidate), process.cwd.as_deref()) == expected
            });
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        // No native executable API is available on this platform. Never
        // inspect later arguments: they may merely contain the plugin path.
        let candidates = process
            .argv
            .as_deref()
            .and_then(|argv| argv.first())
            .map(String::as_str)
            .into_iter()
            .chain(process.argv0.iter().map(String::as_str));
        return candidates
            .filter(|candidate| candidate.contains('/') || candidate.contains('\\'))
            .any(|candidate| {
                canonical_or_original(Path::new(candidate), process.cwd.as_deref()) == expected
            });
    }
}

fn canonical_or_original(path: &Path, cwd: Option<&str>) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_owned()
    } else if let Some(cwd) = cwd {
        PathBuf::from(cwd).join(path)
    } else {
        path.to_owned()
    };
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn cache_key(session_id: &str, endpoint_identity: &str) -> String {
    format!("{session_id}\n{endpoint_identity}")
}

fn receipt_key(endpoint_identity: &str, pane_id: &str) -> String {
    format!("{endpoint_identity}\n{pane_id}")
}

fn is_unsupported_capability(error: &InspectionError) -> bool {
    matches!(
        error.code.as_str(),
        "unknown_method"
            | "method_not_found"
            | "unsupported"
            | "not_supported"
            | "capability_unsupported"
            | "unsupported_capability"
            | "capability_unavailable"
    )
}

/// Once plugin.pane.open has returned success, any confirmation failure is
/// explicitly uncertain: retrying the mutation could create a duplicate pane.
fn post_mutation_error(error: InspectionError) -> InspectionError {
    InspectionError::new(
        "mutation_applied_snapshot_failed",
        format!(
            "plugin pane mutation may already be applied; only resync is safe because confirmation failed ({}): {}",
            error.code, error.message
        ),
    )
}
