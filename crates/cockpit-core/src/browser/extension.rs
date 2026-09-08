use super::*;
use cockpit_protocol::browser::{BrowserFeedbackAckRequest, BrowserFeedbackLookup};
use cockpit_protocol::browser_feedback::{
    BrowserCaptureContext, BrowserCaptureSaved, BrowserCaptureSubmission, BrowserFeedbackAck,
};
use std::io::Write;

pub const ANNOTATION_EXTENSION_ID: &str = "fblkilbfbmpndnfjaacljmcljhepakok";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pairing {
    endpoint: String,
    token: String,
    association_key: String,
    space_label: String,
    browser_instance: String,
}

#[derive(Serialize)]
pub struct ExtensionStatus {
    pub association_key: String,
    pub space_label: String,
    pub connected: bool,
    pub pending_count: usize,
}

const ASSETS: &[(&str, &[u8])] = &[
    (
        "manifest.json",
        include_bytes!("../../../../browser-extension/manifest.json"),
    ),
    (
        "background.js",
        include_bytes!("../../../../browser-extension/background.js"),
    ),
    (
        "content.js",
        include_bytes!("../../../../browser-extension/content.js"),
    ),
    (
        "popup.html",
        include_bytes!("../../../../browser-extension/popup.html"),
    ),
    (
        "popup.js",
        include_bytes!("../../../../browser-extension/popup.js"),
    ),
    (
        "popup.css",
        include_bytes!("../../../../browser-extension/popup.css"),
    ),
];

impl BrowserService {
    pub fn browser_state_directory(&self) -> &Path {
        self.root.as_path()
    }

    pub fn feedback_port(&self) -> Result<Option<u16>, InspectionError> {
        read_json(&self.root.join("feedback-port.json"))
    }

    pub fn persist_feedback_port(&self, port: u16) -> Result<(), InspectionError> {
        atomic_write_json(&self.root.join("feedback-port.json"), &port)
    }

    pub fn configure_feedback_endpoint(&self, endpoint: String) -> Result<(), InspectionError> {
        self.feedback_endpoint.set(endpoint).map_err(|_| {
            InspectionError::new(
                "browser_feedback_endpoint",
                "Browser feedback endpoint is already configured",
            )
        })
    }

    pub(super) fn prepare_annotation_extension(
        &self,
        receipt: &BrowserReceipt,
    ) -> Result<(), InspectionError> {
        let endpoint = self.feedback_endpoint.get().ok_or_else(|| {
            InspectionError::new(
                "browser_feedback_unavailable",
                "The owning Cockpit runtime has no annotation endpoint",
            )
        })?;
        let instance = Uuid::new_v4().to_string();
        let bundle = self
            .root
            .join("extensions")
            .join(&receipt.association_key)
            .join(&instance);
        prepare_root(&bundle)?;
        let directory =
            crate::project_store::open_dir_nofollow_absolute(&bundle).map_err(|_| {
                InspectionError::new("unsafe_path", "Cannot open annotation extension directory")
            })?;
        // Chromium can reuse a cached worker when an unpacked bundle changes without a version change.
        let mut digest = Sha256::new();
        for (name, bytes) in ASSETS {
            digest.update(name.as_bytes());
            digest.update(bytes);
        }
        let digest = digest.finalize();
        let version = format!(
            "1.{}.{}.{}",
            u16::from_be_bytes([digest[0], digest[1]]),
            u16::from_be_bytes([digest[2], digest[3]]),
            u16::from_be_bytes([digest[4], digest[5]])
        );
        for (name, bytes) in ASSETS {
            let mut file = directory.create(name).map_err(|_| {
                InspectionError::new(
                    "browser_extension_write",
                    "Cannot create annotation extension asset",
                )
            })?;
            if *name == "manifest.json" {
                let mut manifest: Value = serde_json::from_slice(bytes).map_err(|_| {
                    InspectionError::new(
                        "browser_extension_write",
                        "Invalid bundled annotation manifest",
                    )
                })?;
                manifest["version"] = Value::String(version.clone());
                serde_json::to_writer(&mut file, &manifest).map_err(|_| {
                    InspectionError::new(
                        "browser_extension_write",
                        "Cannot persist annotation manifest",
                    )
                })?;
            } else {
                file.write_all(bytes).map_err(|_| {
                    InspectionError::new(
                        "browser_extension_write",
                        "Cannot persist annotation extension asset",
                    )
                })?;
            }
            file.sync_all().map_err(|_| {
                InspectionError::new(
                    "browser_extension_write",
                    "Cannot persist annotation extension asset",
                )
            })?;
        }
        let pairing = Pairing {
            endpoint: endpoint.clone(),
            token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
            association_key: receipt.association_key.clone(),
            space_label: receipt.space_label.clone(),
            browser_instance: instance,
        };
        atomic_write_json(&bundle.join("pairing.json"), &pairing)?;
        atomic_write_json(
            &self
                .root
                .join("pairings")
                .join(format!("{}.json", receipt.association_key)),
            &pairing,
        )?;
        let mut config = launch_configuration(&self.configuration)?;
        let launch = config
            .pointer_mut("/browser/launchOptions")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                InspectionError::new("browser_config_invalid", "Missing browser launch options")
            })?;
        launch.insert(
            "ignoreDefaultArgs".into(),
            serde_json::json!(["--disable-extensions"]),
        );
        atomic_write_json(Path::new(&receipt.config_path), &config)
    }

    pub(super) async fn ensure_annotation_extension(
        &self,
        receipt: &BrowserReceipt,
    ) -> Result<(), InspectionError> {
        let pairing: Pairing = read_json(
            &self
                .root
                .join("pairings")
                .join(format!("{}.json", receipt.association_key)),
        )?
        .ok_or_else(|| {
            InspectionError::new(
                "browser_annotation_unavailable",
                "Close and reopen this Space browser to enable annotations",
            )
        })?;
        if Uuid::parse_str(&pairing.browser_instance).is_err()
            || pairing.association_key != receipt.association_key
        {
            return Err(InspectionError::new(
                "browser_annotation_unavailable",
                "Annotation pairing is invalid",
            ));
        }
        let bundle = self
            .root
            .join("extensions")
            .join(&receipt.association_key)
            .join(&pairing.browser_instance);
        let path = serde_json::to_string(&path_string(&bundle)?).map_err(|error| {
            InspectionError::new("browser_annotation_unavailable", error.to_string())
        })?;
        let id =
            serde_json::to_string(ANNOTATION_EXTENSION_ID).expect("fixed extension ID serializes");
        let code = format!(
            "async page => {{ const client = await page.context().browser().newBrowserCDPSession(); try {{ const current = await client.send('Extensions.getExtensions'); if (current.extensions.some(extension => extension.id === {id} && extension.path === {path} && extension.enabled)) return; const loaded = await client.send('Extensions.loadUnpacked', {{ path: {path} }}); if (loaded.id !== {id}) throw new Error('Unexpected annotation extension identity'); }} finally {{ await client.detach(); }} }}"
        );
        self.run_cli(
            receipt,
            &[
                format!("-s={}", receipt.playwright_session),
                "run-code".into(),
                code,
            ],
        )
        .await
        .map(|_| ())
        .map_err(|error| {
            InspectionError::new(
                "browser_annotation_unavailable",
                format!(
                    "Browser is open, but annotations could not initialize: {}",
                    error.message
                ),
            )
        })
    }

    fn authenticate_annotation(
        &self,
        key: &str,
        token: &str,
    ) -> Result<BrowserReceipt, InspectionError> {
        if key.len() != 24 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) || token.len() != 64
        {
            return Err(InspectionError::new(
                "browser_annotation_unauthorized",
                "Annotation pairing is missing or expired",
            ));
        }
        let pairing: Pairing = read_json(&self.root.join("pairings").join(format!("{key}.json")))?
            .ok_or_else(|| {
                InspectionError::new(
                    "browser_annotation_unauthorized",
                    "Annotation pairing is missing or expired",
                )
            })?;
        let difference = pairing
            .token
            .as_bytes()
            .iter()
            .zip(token.as_bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            });
        if pairing.association_key != key || pairing.token.len() != token.len() || difference != 0 {
            return Err(InspectionError::new(
                "browser_annotation_unauthorized",
                "Annotation pairing is missing or expired",
            ));
        }
        self.load(key)?.ok_or_else(|| {
            InspectionError::new(
                "browser_annotation_unavailable",
                "The associated Space browser record is unavailable",
            )
        })
    }

    pub fn authorize_annotation(&self, key: &str, token: &str) -> Result<(), InspectionError> {
        self.authenticate_annotation(key, token).map(|_| ())
    }

    pub fn extension_status(
        &self,
        key: &str,
        token: &str,
    ) -> Result<ExtensionStatus, InspectionError> {
        let receipt = self.authenticate_annotation(key, token)?;
        let pending = self.feedback.list(key)?;
        Ok(ExtensionStatus {
            association_key: key.to_owned(),
            space_label: receipt.space_label,
            connected: true,
            pending_count: pending.pending_count,
        })
    }

    pub fn save_extension_capture(
        &self,
        key: &str,
        token: &str,
        submission: BrowserCaptureSubmission,
    ) -> Result<BrowserCaptureSaved, InspectionError> {
        let receipt = self.authenticate_annotation(key, token)?;
        if submission.association_key != key {
            return Err(InspectionError::new(
                "browser_annotation_target",
                "Capture belongs to another Space browser",
            ));
        }
        // Tab/document/instance describe historical evidence, not a currently live target.
        let address = self.association(&receipt, BrowserConnectionState::Closed);
        let context = BrowserCaptureContext {
            association_key: address.association_key,
            session_id: address.session_id,
            space_id: address.space_id,
            space_label: address.space_label,
            playwright_session: address.playwright_session,
            working_directory: address.working_directory,
            invocation: address.invocation,
            browser_instance: submission.browser_instance.clone(),
        };
        self.feedback.save(context, submission)
    }

    pub async fn feedback(
        &self,
        target: &BrowserTarget,
    ) -> Result<BrowserFeedbackLookup, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let target = self.resolve_target(target).await?;
        let key = association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.space_id,
        );
        let browser = match self.load(&key)? {
            Some(mut receipt) => self.status(&mut receipt).await?,
            None => BrowserResponse {
                association: None,
                connection: BrowserConnectionState::Absent,
                message: "No browser association exists for this Space".into(),
            },
        };
        Ok(BrowserFeedbackLookup {
            browser,
            feedback: self.feedback.list(&key)?,
        })
    }

    pub async fn acknowledge_feedback(
        &self,
        request: BrowserFeedbackAckRequest,
    ) -> Result<BrowserFeedbackAck, InspectionError> {
        let _operation = self.operation_lock.lock().await;
        let target = self.resolve_target(&request.target).await?;
        let key = association_key(
            &target.endpoint_identity,
            &target.session_id,
            &target.space_id,
        );
        self.feedback.ack(&key, &request.ids)
    }

    pub fn prune_feedback(&self) -> Result<(), InspectionError> {
        self.feedback.prune()
    }
}
