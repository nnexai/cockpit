const BROWSER_USER_AGENT_HOOK: &str =
    include_str!("../../../../browser-runtime/browser-user-agent.cjs");

use async_trait::async_trait;
use cockpit_protocol::browser::{
    BrowserCleanupFailure, BrowserCleanupState, BrowserConnectionState, BrowserResponse,
};
use cockpit_protocol::v1::SessionSnapshotResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Duration;
use tokio::sync::Mutex;

use crate::InspectionError;
use crate::config::BrowserConfiguration;
use receipts::{BrowserReceipt, ReceiptIntent, ReceiptState, association_key};

mod cdp;
mod cleanup;
mod delivery;
pub mod drafts;
#[cfg(all(test, unix))]
mod lifecycle_tests;
mod process;
mod receipts;
mod service;

pub(crate) const STATE_DIRS: &[&str] = &[
    "tab-associations",
    "profiles",
    "workspaces",
    "configs",
    "feedback",
    "artifacts",
    "drafts",
];

/// Inspect the configured state's browser store without creating or changing it.
pub fn space_has_saved_state(
    state_root: &Path,
    endpoint: &str,
    session_id: &str,
    space_id: &str,
) -> Result<bool, InspectionError> {
    let root = if state_root.is_absolute() {
        state_root.join("browser")
    } else {
        let mut root = std::env::current_dir()
            .map_err(|error| InspectionError::new("browser_state_unavailable", error.to_string()))?;
        root.push(state_root);
        root.push("browser");
        root
    };
    let Some(_directory) = receipts::optional_state_dir(&root)? else {
        return Ok(false);
    };
    let receipts = receipts::saved_receipts(&root)?;
    if receipts.iter().any(|receipt| {
        receipt.endpoint_identity == endpoint
            && receipt.session_id == session_id
            && receipt.space_id == space_id
    }) {
        // Closed and recovery receipts are still persisted Space consumers.
        return Ok(true);
    }
    drafts::space_has_saved_state(&root, &receipts, endpoint, session_id, space_id)
}

/// A snapshot pinned to the exact Herdr endpoint that served it.
#[derive(Clone, Debug)]
pub struct BrowserHerdrSnapshot {
    pub endpoint_identity: String,
    pub endpoint_path: String,
    pub snapshot: SessionSnapshotResponse,
}

/// The browser service's fresh-only Herdr authority.
#[async_trait]
pub trait BrowserHerdrAdapter: Send + Sync {
    async fn browser_snapshot(
        &self,
        session_id: &str,
    ) -> Result<BrowserHerdrSnapshot, InspectionError>;

    /// Return the identity of the live socket peer without requesting a session snapshot.
    /// Implementations should use the same peer-credential/process-generation identity as
    /// `browser_snapshot`; the default keeps existing adapters correct, though less cheaply.
    async fn browser_endpoint_identity(
        &self,
        session_id: &str,
    ) -> Result<(String, String), InspectionError> {
        let snapshot = self.browser_snapshot(session_id).await?;
        Ok((snapshot.endpoint_identity, snapshot.endpoint_path))
    }
}

#[derive(Clone)]
pub struct BrowserService {
    configuration: BrowserConfiguration,
    root: Arc<PathBuf>,
    owner_id: String,
    adapter: Arc<dyn BrowserHerdrAdapter>,
    operation_lock: Arc<Mutex<()>>,
    shutting_down: Arc<AtomicBool>,
    feedback: Arc<crate::browser_feedback::BrowserFeedbackStore>,
    paste_adapter: Option<Arc<dyn crate::paste_adapter::CommentPasteAdapter>>,
    cleanup_failures: Arc<parking_lot::Mutex<Vec<BrowserCleanupFailure>>>,
}

/// A host-only capability for attaching the private inline helper. This type is
/// intentionally assembled only after fresh endpoint and target ownership
/// checks.
#[derive(Clone)]
pub struct BrowserRuntimeAttachment {
    pub association_key: String,
    pub browser_incarnation: String,
    pub session_id: String,
    pub space_id: String,
    pub tab_id: String,
    pub endpoint_identity: String,
    pub endpoint_path: String,
    pub profile_path: PathBuf,
    pub cdp_endpoint: String,
    /// Stable CDP target identity selected for this association.
    pub target_id: String,
    pub playwright_core: Option<PathBuf>,
    pub node_executable: Option<PathBuf>,
    pub helper_module: Option<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct ResolvedTarget {
    pub(crate) endpoint_identity: String,
    pub(crate) endpoint_path: String,
    pub(crate) session_id: String,
    pub(crate) space_id: String,
    pub(crate) space_label: String,
    pub(crate) tab_id: String,
    pub(crate) tab_label: String,
    pub(crate) tab_present: bool,
}

pub(crate) struct ResolvedWorkScope {
    pub(crate) association_key: String,
    pub(crate) tab: ResolvedTarget,
}

pub(crate) fn validate_url(value: &str) -> Result<(), InspectionError> {
    let parsed = url::Url::parse(value)
        .map_err(|_| InspectionError::new("invalid_browser_url", "browser URL must be absolute"))?;
    if !matches!(parsed.scheme(), "http" | "https" | "about") {
        return Err(InspectionError::new(
            "invalid_browser_url",
            "browser URL has an unsupported or unsafe scheme",
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(InspectionError::new(
            "invalid_browser_url",
            "browser URL must not contain userinfo",
        ));
    }
    Ok(())
}
#[cfg(test)]
use cockpit_protocol::browser::{BrowserAction, BrowserRequest, BrowserTarget};
#[cfg(test)]
use process::{
    compatible_playwright_cli_version, daemon_session_path, may_launch_after_inspection_failure,
    process_start_identity, shell_quote, tab_index,
};
#[cfg(test)]
use receipts::prepare_root;
#[cfg(all(test, unix))]
use sha1::Sha1;
#[cfg(all(test, unix))]
use sha2::{Digest, Sha256};
#[cfg(all(test, unix))]
use std::{env, sync::atomic::Ordering};
#[cfg(all(test, unix))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(all(test, unix))]
use uuid::Uuid;
#[cfg(test)]
mod tests {
    use super::association_key;
    #[test]
    fn playwright_cli_accepts_later_bugfix_releases_only() {
        assert!(super::compatible_playwright_cli_version("0.1.5"));
        assert!(super::compatible_playwright_cli_version("0.1.17"));
        assert!(!super::compatible_playwright_cli_version("0.1.4"));
        assert!(!super::compatible_playwright_cli_version("0.2.0"));
        assert!(!super::compatible_playwright_cli_version("1.1.5"));
        assert!(!super::compatible_playwright_cli_version("0.1"));
        assert!(!super::compatible_playwright_cli_version("0.1.17.1"));
        assert!(!super::compatible_playwright_cli_version("0.1.17-alpha"));
    }
    #[test]
    fn readonly_saved_state_protects_receipts_in_every_lifecycle_state() {
        let root = std::env::temp_dir()
            .join(format!("cockpit-browser-consumer-{}", uuid::Uuid::new_v4()));
        assert!(!super::space_has_saved_state(&root, "boot", "session", "space").unwrap());
        assert!(!root.exists());
        let browser = root.join("browser");
        std::fs::create_dir_all(browser.join("tab-associations")).unwrap();
        let key = association_key("boot", "session", "space:tab");
        let path = browser.join("tab-associations").join(format!("{key}.json"));
        let mut receipt = serde_json::json!({
            "association_key": key, "owner_id": "owner",
            "endpoint_identity": "boot", "endpoint_path": "/herdr.sock",
            "session_id": "session", "space_id": "space", "space_label": "Space",
            "tab_id": "space:tab", "tab_label": "Browser",
            "artifacts": {
                "profile": {"dev": 1, "inode": 1},
                "workspace": {"dev": 1, "inode": 2},
                "config": {"dev": 1, "inode": 3}
            },
            "cleanup_reason": null, "unproven_paths": [],
            "playwright_session": format!("cockpit-{key}"),
            "working_directory": browser.join("workspaces").join(&key),
            "profile_path": browser.join("profiles").join(&key),
            "config_path": browser.join("configs").join(format!("{key}.json")),
            "cdp_endpoint": null, "cdp_browser_identity": null,
            "intent": "none", "state": "closed",
            "target_id": null, "opened_tab": null, "incarnation": null
        });
        for state in [
            "pending_open", "open", "closing", "closed", "outcome_unknown",
            "disconnected", "cleanup_pending", "cleanup_failed",
        ] {
            receipt["state"] = state.into();
            let bytes = serde_json::to_vec(&receipt).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            assert!(super::space_has_saved_state(&root, "boot", "session", "space").unwrap());
            assert!(!super::space_has_saved_state(&root, "other-boot", "session", "space").unwrap());
            assert!(!super::space_has_saved_state(&root, "boot", "other-session", "space").unwrap());
            assert!(!super::space_has_saved_state(&root, "boot", "session", "other-space").unwrap());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        std::fs::write(&path, "{}").unwrap();
        assert!(super::space_has_saved_state(&root, "boot", "session", "space").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn association_key_changes_with_endpoint_process_generation() {
        let first = association_key(
            "unix-socket:/run/herdr.sock:pid=41:uid=1000:gid=1000:start=101",
            "daily",
            "w1",
        );
        let restarted = association_key(
            "unix-socket:/run/herdr.sock:pid=41:uid=1000:gid=1000:start=102",
            "daily",
            "w1",
        );
        assert_ne!(first, restarted);
    }

    #[test]
    fn opened_tab_address_selects_current_tab_not_first_tab() {
        let output = "### Result\n- 0: [](about:blank)\n- 1: [Checkout](http://localhost/checkout)\n- 2: (current) [Checkout](http://localhost/checkout)\n";
        assert_eq!(super::tab_index(output).as_deref(), Some("2"));
    }

    #[test]
    fn inspection_uncertainty_never_authorizes_a_replacement_launch() {
        for code in [
            "browser_daemon_unavailable",
            "browser_receipt_replaced",
            "browser_unowned_daemon",
            "unsafe_path",
        ] {
            assert!(!super::may_launch_after_inspection_failure(
                &crate::InspectionError::new(code, "verification failed")
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn browser_state_refuses_symlinked_parents_without_creating_targets() {
        let root =
            std::env::temp_dir().join(format!("cockpit-browser-symlink-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(super::prepare_root(&link.join("browser")).is_err());
        assert!(super::space_has_saved_state(&link, "boot", "session", "space").is_err());
        assert!(!outside.join("browser").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
