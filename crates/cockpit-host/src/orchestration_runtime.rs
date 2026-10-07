//! Shared browser/native composition; only the private runtime owner dispatches.
use cockpit_core::{
    InspectionError,
    library::LibraryService,
    orchestration::{
        Actor, OrchestrationService,
        dispatch::{Dispatcher, DispatcherSettings},
        herdr::OrchestrationHerdr,
    },
    projects::ProjectService,
};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_protocol::{orchestration::*, projects::ProjectConfiguration};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub enum CliProcessRole {
    Host,
    Native,
}

pub struct OrchestrationRuntime {
    pub service: Arc<OrchestrationService>,
    herdr: Arc<dyn OrchestrationHerdr>,
    dispatcher: Mutex<Option<Dispatcher>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown: CancellationToken,
}
impl OrchestrationRuntime {
    pub fn new(
        configuration: &ProjectConfiguration,
        herdr: Arc<HerdrCliAdapter>,
        projects: Arc<ProjectService>,
        library: Arc<LibraryService>,
        config_path: Option<PathBuf>,
        cli_role: CliProcessRole,
    ) -> Result<Self, InspectionError> {
        let service = Arc::new(OrchestrationService::open(configuration)?);
        let extension =
            std::path::absolute(extension_path(configuration)?).map_err(extension_error)?;
        // Workers have a different cwd; transport selection must not become cwd-relative there.
        let config_path = cockpit_core::config::configuration_path(config_path.as_deref())?
            .map(std::path::absolute)
            .transpose()
            .map_err(extension_error)?;
        let herdr_socket = herdr
            .config()
            .socket
            .clone()
            .map(std::path::absolute)
            .transpose()
            .map_err(extension_error)?;
        let executable = &herdr.config().executable;
        let herdr_executable = Some(if executable.components().count() > 1 {
            std::path::absolute(executable).map_err(extension_error)?
        } else {
            executable.clone()
        });
        let settings = DispatcherSettings {
            omp_extension: extension,
            agent_kind: "omp".into(),
            start_timeout_ms: 60_000,
            cli_path: cli_path(cli_role)?,
            config_path,
            herdr_socket,
            herdr_executable,
            model: configuration.orchestration.model.clone(),
            extra_args: configuration.orchestration.extra_args.clone(),
        };
        let dispatcher =
            Dispatcher::new(service.clone(), projects, library, herdr.clone(), settings);
        Ok(Self {
            service,
            herdr,
            dispatcher: Mutex::new(Some(dispatcher)),
            task: Mutex::new(None),
            shutdown: CancellationToken::new(),
        })
    }
    pub async fn start_owner(&self, owner: bool) {
        if !owner || self.shutdown.is_cancelled() {
            return;
        }
        let mut task = self.task.lock().await;
        if let Some(dispatcher) = self.dispatcher.lock().await.take() {
            *task = Some(dispatcher.spawn(self.shutdown.clone()));
        }
    }
    pub async fn snapshot(
        &self,
        request: &OrchestrationSnapshotRequest,
    ) -> Result<OrchestrationSnapshot, InspectionError> {
        self.service.snapshot(self.herdr.as_ref(), request).await
    }
    pub async fn mutate(
        &self,
        origin: OperatorOrigin,
        request: OrchestrationMutationRequest,
    ) -> Result<OrchestrationMutationResponse, InspectionError> {
        let reviewed = if let OrchestrationAction::RetryLaunch { run_id } = &request.action {
            let run = self.service.run_for_review(&request.session_id, run_id)?;
            retry_launch_preflight(self.herdr.as_ref(), &run).await?;
            Some(run)
        } else {
            None
        };
        let service = self.service.clone();
        tokio::task::spawn_blocking(move || match reviewed {
            Some(reviewed) => service.mutate_operator_reviewed(origin, request, &reviewed),
            None => service.mutate(&Actor::Operator(origin), request),
        })
        .await
        .map_err(|error| InspectionError::new("orchestration_runtime_failed", error.to_string()))?
    }
    pub async fn wait(
        &self,
        request: &OrchestrationWaitRequest,
    ) -> Result<OrchestrationWaitResponse, InspectionError> {
        self.service.wait(request).await
    }
    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some(mut task) = self.task.lock().await.take() {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

/// Both GUI transports use this live check, never a cached Missing badge.
/// The state commit separately fences this exact run incarnation. Herdr may
/// still change after this read; an explicit retry never kills old occupants.
async fn retry_launch_preflight(
    herdr: &dyn OrchestrationHerdr,
    run: &Run,
) -> Result<(), InspectionError> {
    let runtime = herdr.runtime(&run.session_id).await.map_err(|error| {
        InspectionError::new(
            "retry_observation_unavailable",
            format!(
                "Cannot safely restart while Herdr is unavailable: {}",
                error.message
            ),
        )
    })?;
    let tag = run
        .dispatch
        .as_ref()
        .and_then(|dispatch| dispatch.launch_tag.as_deref());
    let original_present = runtime.panes.iter().any(|pane| {
        let tagged = tag.is_some_and(|tag| pane.agent_name.as_deref() == Some(tag));
        let terminal = run.location.as_ref().is_some_and(|location| {
            location.endpoint_identity == runtime.endpoint_identity
                && location
                    .terminal_id
                    .as_ref()
                    .is_some_and(|id| pane.terminal_id.as_ref() == Some(id))
        });
        let native = run
            .bound_omp_session
            .as_ref()
            .is_some_and(|id| pane.native_session_id.as_ref() == Some(id));
        (tagged || terminal || native)
            && (pane.agent_kind.as_deref() == Some("omp") || pane.launch_pending)
    });
    if original_present {
        return Err(InspectionError::new(
            "original_agent_present",
            "The original OMP agent or its accepted pending start is still present. Check its terminal instead of launching a duplicate.",
        ));
    }
    Ok(())
}

fn cli_path(role: CliProcessRole) -> Result<PathBuf, InspectionError> {
    if let Some(path) = std::env::var_os("COCKPIT_CLI_EXECUTABLE") {
        let path = PathBuf::from(path);
        return if path.components().count() > 1 {
            std::path::absolute(path).map_err(extension_error)
        } else {
            Ok(path)
        };
    }
    let executable = std::env::current_exe()
        .map_err(|error| InspectionError::new("orchestration_runtime_failed", error.to_string()))?;
    Ok(resolve_cli_path(&executable, role))
}

fn resolve_cli_path(executable: &Path, role: CliProcessRole) -> PathBuf {
    match role {
        // The browser gateway is the actual host/CLI process, regardless of
        // its filename. Installed native GUI binaries are also named cockpit.
        CliProcessRole::Host => executable.to_owned(),
        CliProcessRole::Native => {
            let sibling = executable.with_file_name("cockpit-cli");
            if sibling.is_file() {
                return sibling;
            }
            // Cargo names the GUI cockpit-tauri and the host cockpit. This
            // development-only name must never match installed GUI cockpit.
            if executable
                .file_name()
                .is_some_and(|name| name == "cockpit-tauri")
            {
                let host = executable.with_file_name("cockpit");
                if host != executable && host.is_file() {
                    return host;
                }
            }
            PathBuf::from("cockpit-cli")
        }
    }
}

fn extension_path(configuration: &ProjectConfiguration) -> Result<PathBuf, InspectionError> {
    if let Some(path) = std::env::var_os("COCKPIT_OMP_EXTENSION") {
        return Ok(path.into());
    }
    if let Some(path) = &configuration.orchestration.omp_extension {
        return Ok(path.into());
    }
    let root = Path::new(&configuration.state_root).join("orchestration");
    let directory = root.join("omp");
    private_directory(&directory)?;
    let path = directory.join("cockpit-orchestration.ts");
    let content = include_bytes!("../../../integrations/omp/cockpit-orchestration.ts");
    let existing = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&path);
    match existing {
        Ok(file) => {
            if !file.metadata().map_err(extension_error)?.is_file() {
                return Err(extension_error("Extension is not a regular file"));
            }
            let mut bytes = Vec::new();
            file.take(content.len() as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(extension_error)?;
            if bytes.as_slice() == content.as_slice() {
                return Ok(path);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(extension_error(error)),
    }
    let temp = directory.join(format!(".extension-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&temp)
            .map_err(extension_error)?;
        file.write_all(content).map_err(extension_error)?;
        file.sync_all().map_err(extension_error)?;
        fs::rename(&temp, &path).map_err(extension_error)?;
        fs::File::open(&directory)
            .and_then(|directory| directory.sync_all())
            .map_err(extension_error)?;
        Ok(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn private_directory(path: &Path) -> Result<(), InspectionError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(extension_error(error)),
    }
    let metadata = fs::symlink_metadata(path).map_err(extension_error)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != nix::unistd::Uid::current().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(extension_error(
            "Bundled extension directory must be private and owned by the current user",
        ));
    }
    Ok(())
}
fn extension_error(error: impl std::fmt::Display) -> InspectionError {
    InspectionError::new("omp_extension_unavailable", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_core::orchestration::herdr::{
        AgentStartRequest, AgentTabRequest, RuntimePane, RuntimeView,
    };

    struct ObservedHerdr(Option<RuntimeView>);
    #[async_trait::async_trait]
    impl OrchestrationHerdr for ObservedHerdr {
        async fn runtime(&self, _: &str) -> Result<RuntimeView, InspectionError> {
            self.0
                .clone()
                .ok_or_else(|| InspectionError::new("offline", "No fresh endpoint"))
        }
        async fn create_agent_tab(
            &self,
            _: &str,
            _: &AgentTabRequest,
        ) -> Result<RunLocation, InspectionError> {
            panic!("a retry preflight must not create a terminal")
        }
        async fn start_agent(&self, _: &str, _: &AgentStartRequest) -> Result<(), InspectionError> {
            panic!("a retry preflight must not start a process")
        }
        async fn pane_process_info(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<cockpit_core::orchestration::herdr::PaneProcessInfo, InspectionError> {
            panic!("a retry preflight must not inspect retirement processes")
        }
        async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
            panic!("a retry preflight must not close a terminal")
        }
    }

    fn reviewed() -> Run {
        serde_json::from_value(serde_json::json!({
            "session_id":"fixture","prepare_brief":"","run_id":"run","kind":"supervisor",
            "label":"Supervisor","root_id":"run","attempt":1,"stage":"active",
            "dispatch":{"launch_tag":"tag","endpoint_identity":"endpoint","agent_started":true,
                "step":"launch_unknown","launch_attempt":0,"updated_at":"2026-10-06T00:00:00Z"},
            "grants":[],"annotations":[],
            "location":{"endpoint_identity":"endpoint","session_id":"fixture","workspace_id":"space",
                "tab_id":"tab","pane_id":"pane","launch_tag":"tag","terminal_id":"terminal"},
            "bound_omp_session":"native","created_at":"2026-10-06T00:00:00Z","updated_at":"2026-10-06T00:00:00Z",
            "launch_shell_identity":{"process":{"pid":123,"start_ticks":1,"kernel_boot_id":"00000000-0000-0000-0000-000000000001"},
                "executable_device":"1","executable_inode":"2","argv_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
        })).unwrap()
    }

    fn observed() -> RuntimeView {
        RuntimeView {
            endpoint_identity: "endpoint".into(),
            boot_id: None,
            workspaces: vec![],
            panes: vec![RuntimePane {
                workspace_id: "space".into(),
                workspace_label: "Space".into(),
                tab_id: "tab".into(),
                tab_label: "tag".into(),
                pane_id: "pane".into(),
                terminal_id: Some("terminal".into()),
                native_session_id: None,
                agent_name: Some("tag".into()),
                agent_kind: Some("omp".into()),
                launch_pending: false,
                interactive_ready: false,
                agent_status: Some("working".into()),
                state_changed_at: None,
            }],
        }
    }

    #[tokio::test]
    async fn fresh_retry_rejects_offline_actual_omp_and_pending_without_effects() {
        let run = reviewed();
        assert_eq!(
            retry_launch_preflight(&ObservedHerdr(None), &run)
                .await
                .unwrap_err()
                .code,
            "retry_observation_unavailable"
        );
        let runtime = observed();
        assert_eq!(
            retry_launch_preflight(&ObservedHerdr(Some(runtime.clone())), &run)
                .await
                .unwrap_err()
                .code,
            "original_agent_present"
        );
        let mut pending = runtime.clone();
        pending.panes[0].agent_kind = None;
        pending.panes[0].launch_pending = true;
        assert_eq!(
            retry_launch_preflight(&ObservedHerdr(Some(pending)), &run)
                .await
                .unwrap_err()
                .code,
            "original_agent_present"
        );
        let mut shell = runtime.clone();
        shell.panes[0].agent_kind = None;
        shell.panes[0].agent_name = None;
        retry_launch_preflight(&ObservedHerdr(Some(shell)), &run)
            .await
            .unwrap();
        let mut missing = runtime;
        missing.panes.clear();
        retry_launch_preflight(&ObservedHerdr(Some(missing)), &run)
            .await
            .unwrap();
    }
    #[test]
    fn installed_native_cockpit_uses_sibling_cli_never_gui_self() {
        let root =
            std::env::temp_dir().join(format!("cockpit-installed-cli-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let native = root.join("cockpit");
        let sibling = root.join("cockpit-cli");
        fs::write(&native, b"native-gui").unwrap();
        fs::write(&sibling, b"actual-cli").unwrap();
        assert_eq!(resolve_cli_path(&native, CliProcessRole::Native), sibling);
        assert_eq!(resolve_cli_path(&native, CliProcessRole::Host), native);
        fs::remove_file(&sibling).unwrap();
        assert_eq!(
            resolve_cli_path(&native, CliProcessRole::Native),
            PathBuf::from("cockpit-cli")
        );
        // Cargo's explicitly named native GUI can use the separate host
        // sibling, while the installed native GUI never resolves to itself.
        assert_eq!(
            resolve_cli_path(&root.join("cockpit-tauri"), CliProcessRole::Native),
            native
        );
        fs::remove_dir_all(root).unwrap();
    }
}
