//! Shared browser/native composition; only the private runtime owner dispatches.
use cockpit_core::{
    InspectionError,
    library::LibraryService,
    orchestration::{
        Actor, OrchestrationService,
        dispatch::{Dispatcher, DispatcherSettings},
        herdr::OrchestrationHerdr,
    },
    process_identity::{incarnation_running, kernel_boot_id},
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
            retry_launch_preflight(self.herdr.as_ref(), &run, RetryAuthority::Operator).await?;
            Some(run)
        } else {
            None
        };
        let service = self.service.clone();
        tokio::task::spawn_blocking(move || match reviewed {
            Some(reviewed) => service.mutate_reviewed(&Actor::Operator(origin), request, &reviewed),
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

#[derive(Clone, Copy)]
pub enum RetryAuthority {
    Operator,
    Supervisor,
}

/// All retry transports use this live check, never a cached Missing badge.
/// The state commit separately fences this exact run incarnation. Herdr may
/// still change after this read; an explicit retry never kills old occupants.
pub async fn retry_launch_preflight(
    herdr: &dyn OrchestrationHerdr,
    run: &Run,
    authority: RetryAuthority,
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
    match original_process(run) {
        OriginalProcess::Running => {
            return Err(InspectionError::new(
                "original_agent_present",
                "The original OMP process incarnation is still alive, even if its pane is missing. A stopped process must not be duplicated.",
            ));
        }
        OriginalProcess::Unverifiable if matches!(authority, RetryAuthority::Supervisor) => {
            return Err(InspectionError::new(
                "original_identity_unverifiable",
                "The original OMP process cannot be proved absent. Only the operator can decide whether to restart with unverifiable identity evidence.",
            ));
        }
        OriginalProcess::NeverBound if matches!(authority, RetryAuthority::Supervisor) => {
            if let Some(location) = run.location.as_ref() {
                let terminal = location.terminal_id.as_deref().filter(|id| !id.is_empty());
                let same_endpoint = location.session_id == run.session_id
                    && location.endpoint_identity == runtime.endpoint_identity
                    && location.boot_id.as_ref()
                        .is_none_or(|boot| runtime.boot_id.as_ref() == Some(boot));
                let old_terminal_absent = terminal.is_some_and(|terminal| {
                    !runtime.panes.iter().any(|pane| {
                        pane.pane_id == location.pane_id
                            || pane.terminal_id.as_deref() == Some(terminal)
                    })
                });
                // Herdr's close ACK/layout removal does not join PTY shutdown.
                // Its recorded shell can still execute the accepted command.
                let current_kernel_boot = kernel_boot_id();
                let old_shell_exited = run.launch_shell_identity.as_ref().is_some_and(|shell| {
                    shell.process.start_ticks > 0
                        && shell.process.kernel_boot_id.as_deref().is_some_and(|boot| {
                            !boot.is_empty() && current_kernel_boot.as_deref() == Some(boot)
                        })
                        && original_process_at_boot(&shell.process, current_kernel_boot.as_deref())
                            == OriginalProcess::Exited
                });
                if !same_endpoint || !old_terminal_absent || !old_shell_exited {
                    return Err(InspectionError::new(
                        "launch_command_unproven",
                        "The accepted unbound launch command is not proved canceled. Wait for owned recovery to confirm the old pane and terminal absent and the recorded launch shell exited before retrying.",
                    ));
                }
            }
        }
        OriginalProcess::NeverBound | OriginalProcess::Exited | OriginalProcess::Unverifiable => {}
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum OriginalProcess {
    NeverBound,
    Running,
    Exited,
    Unverifiable,
}

fn original_process(run: &Run) -> OriginalProcess {
    let Some(process) = run.bound_omp_process.as_ref() else {
        return if run.bound_omp_session.is_none() {
            OriginalProcess::NeverBound
        } else {
            OriginalProcess::Unverifiable
        };
    };
    original_process_at_boot(process, kernel_boot_id().as_deref())
}

fn original_process_at_boot(
    process: &NativeProcessIdentity,
    current_boot: Option<&str>,
) -> OriginalProcess {
    if let Some(expected_boot) = process.kernel_boot_id.as_deref() {
        match current_boot {
            Some(actual_boot) if actual_boot != expected_boot => return OriginalProcess::Exited,
            None => return OriginalProcess::Unverifiable,
            Some(_) => {}
        }
    }
    let Ok(pid) = i32::try_from(process.pid) else {
        return OriginalProcess::Unverifiable;
    };
    match incarnation_running(pid, process.start_ticks) {
        Ok(true) => OriginalProcess::Running,
        Ok(false) => OriginalProcess::Exited,
        Err(_) => OriginalProcess::Unverifiable,
    }
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
        async fn expire_pending_agent(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
            panic!("a retry preflight must not reconcile pending metadata")
        }
        async fn close_owned_launch_tab(&self, _: &str, _: &RunLocation) -> Result<(), InspectionError> {
            panic!("a retry preflight must not close a launch tab")
        }
        async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
            panic!("a retry preflight must not close a terminal")
        }
    }

    fn reviewed() -> Run {
        serde_json::from_value(serde_json::json!({
            "session_id":"fixture","prepare_brief":"","run_id":"run","kind":"supervisor",
            "label":"Supervisor","root_id":"run","attempt":1,"stage":"active",
            "dispatch":{"launch_tag":"tag","endpoint_identity":"endpoint","agent_started":false,
                "step":"launch_unknown","launch_attempt":0,"updated_at":"2026-10-06T00:00:00Z"},
            "grants":[],"annotations":[],
            "location":{"endpoint_identity":"endpoint","session_id":"fixture","workspace_id":"space",
                "tab_id":"tab","pane_id":"pane","launch_tag":"tag","terminal_id":"terminal"},
            "bound_omp_session":null,"created_at":"2026-10-06T00:00:00Z","updated_at":"2026-10-06T00:00:00Z"
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
        for authority in [RetryAuthority::Operator, RetryAuthority::Supervisor] {
            assert_eq!(
                retry_launch_preflight(&ObservedHerdr(None), &run, authority)
                    .await
                    .unwrap_err()
                    .code,
                "retry_observation_unavailable"
            );
            let runtime = observed();
            assert_eq!(
                retry_launch_preflight(&ObservedHerdr(Some(runtime.clone())), &run, authority)
                    .await
                    .unwrap_err()
                    .code,
                "original_agent_present"
            );
            let mut pending = runtime.clone();
            pending.panes[0].agent_kind = None;
            pending.panes[0].agent_name = None;
            pending.panes[0].launch_pending = true;
            assert_eq!(
                retry_launch_preflight(&ObservedHerdr(Some(pending)), &run, authority)
                    .await
                    .unwrap_err()
                    .code,
                "original_agent_present"
            );
            let mut nameless = runtime.clone();
            nameless.panes[0].agent_name = None;
            assert_eq!(
                retry_launch_preflight(&ObservedHerdr(Some(nameless)), &run, authority)
                    .await
                    .unwrap_err()
                    .code,
                "original_agent_present"
            );
            let mut shell = runtime.clone();
            shell.panes[0].agent_kind = None;
            shell.panes[0].agent_name = None;
            let shell_result = retry_launch_preflight(&ObservedHerdr(Some(shell)), &run, authority).await;
            if matches!(authority, RetryAuthority::Supervisor) {
                assert_eq!(shell_result.unwrap_err().code, "launch_command_unproven");
            } else {
                shell_result.unwrap();
            }
            let mut missing = runtime;
            missing.panes.clear();
            let missing_result = retry_launch_preflight(&ObservedHerdr(Some(missing)), &run, authority).await;
            if matches!(authority, RetryAuthority::Supervisor) {
                assert_eq!(missing_result.unwrap_err().code, "launch_command_unproven");
            } else {
                missing_result.unwrap();
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn supervisor_unbound_retry_requires_exact_owned_terminal_absence() {
        let mut run = reviewed();
        run.location.as_mut().unwrap().boot_id = Some("boot".into());
        let pid = std::process::id() as i32;
        let start = cockpit_core::process_identity::start_identity(pid).unwrap();
        let mut exited_shell = cockpit_core::process_identity::executable_identity(pid, start).unwrap();
        exited_shell.process.start_ticks += 1;
        run.launch_shell_identity = Some(exited_shell);
        let mut shell = observed();
        shell.boot_id = Some("boot".into());
        shell.panes[0].agent_kind = None;
        shell.panes[0].agent_name = None;
        shell.panes[0].launch_pending = false;
        for change in 0..7 {
            let mut run = run.clone();
            let mut runtime = shell.clone();
            match change {
                0 => {}
                1 => runtime.panes[0].terminal_id = Some("replacement-terminal".into()),
                2 => runtime.panes[0].pane_id = "moved-pane".into(),
                3 => { runtime.panes.clear(); runtime.endpoint_identity = "replacement-endpoint".into(); }
                4 => { runtime.panes.clear(); runtime.boot_id = Some("replacement-boot".into()); }
                5 => { runtime.panes.clear(); runtime.boot_id = None; }
                _ => { runtime.panes.clear(); run.location.as_mut().unwrap().terminal_id = None; }
            }
            assert_eq!(
                retry_launch_preflight(&ObservedHerdr(Some(runtime.clone())), &run, RetryAuthority::Supervisor)
                    .await.unwrap_err().code,
                "launch_command_unproven", "change {change}",
            );
            retry_launch_preflight(&ObservedHerdr(Some(runtime)), &run, RetryAuthority::Operator)
                .await.unwrap();
        }
        let mut absent = shell.clone();
        absent.panes.clear();
        retry_launch_preflight(&ObservedHerdr(Some(absent)), &run, RetryAuthority::Supervisor)
            .await.unwrap();
        // Core's completed close/absence transition removes the old location.
        run.location = None;
        retry_launch_preflight(&ObservedHerdr(Some(shell)), &run, RetryAuthority::Supervisor)
            .await.unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn supervisor_layout_absence_does_not_prove_launch_shell_exit() {
        let mut run = reviewed();
        let mut absent = observed();
        absent.panes.clear();
        let pid = std::process::id() as i32;
        let start = cockpit_core::process_identity::start_identity(pid).unwrap();
        let live_shell = cockpit_core::process_identity::executable_identity(pid, start).unwrap();
        for change in 0..5 {
            run.launch_shell_identity = Some(live_shell.clone());
            match change {
                0 => {}
                1 => run.launch_shell_identity = None,
                2 => run.launch_shell_identity.as_mut().unwrap().process.pid = 0,
                3 => run.launch_shell_identity.as_mut().unwrap().process.kernel_boot_id = None,
                _ => run.launch_shell_identity.as_mut().unwrap().process.start_ticks = 0,
            }
            assert_eq!(
                retry_launch_preflight(&ObservedHerdr(Some(absent.clone())), &run, RetryAuthority::Supervisor)
                    .await.unwrap_err().code,
                "launch_command_unproven", "change {change}",
            );
            retry_launch_preflight(&ObservedHerdr(Some(absent.clone())), &run, RetryAuthority::Operator)
                .await.unwrap();
        }
        // A reused PID is not the recorded shell incarnation.
        run.launch_shell_identity = Some(live_shell);
        run.launch_shell_identity.as_mut().unwrap().process.start_ticks += 1;
        retry_launch_preflight(&ObservedHerdr(Some(absent.clone())), &run, RetryAuthority::Supervisor)
            .await.unwrap();
        // Changed kernel evidence cannot authorize this owned endpoint's retry.
        let boot = run.launch_shell_identity.as_mut().unwrap().process.kernel_boot_id.as_mut().unwrap();
        let replacement = if boot.starts_with('0') { "1" } else { "0" };
        boot.replace_range(0..1, replacement);
        assert_eq!(
            retry_launch_preflight(&ObservedHerdr(Some(absent)), &run, RetryAuthority::Supervisor)
                .await.unwrap_err().code,
            "launch_command_unproven",
        );
    }

    #[tokio::test]
    async fn fresh_retry_matches_native_session_without_name_or_terminal() {
        let mut run = reviewed();
        run.bound_omp_session = Some("native".into());
        let mut runtime = observed();
        runtime.panes[0].agent_name = None;
        runtime.panes[0].terminal_id = Some("different-terminal".into());
        runtime.panes[0].native_session_id = Some("native".into());
        for authority in [RetryAuthority::Operator, RetryAuthority::Supervisor] {
            assert_eq!(
                retry_launch_preflight(&ObservedHerdr(Some(runtime.clone())), &run, authority)
                    .await
                    .unwrap_err()
                    .code,
                "original_agent_present"
            );
        }
    }

    fn empty_observation() -> ObservedHerdr {
        let mut runtime = observed();
        runtime.panes.clear();
        ObservedHerdr(Some(runtime))
    }

    #[tokio::test]
    async fn supervisor_requires_provable_original_identity_but_operator_can_override_unknown() {
        let mut run = reviewed();
        assert_eq!(original_process(&run), OriginalProcess::NeverBound);
        run.bound_omp_session = Some("native".into());
        assert_eq!(original_process(&run), OriginalProcess::Unverifiable);
        assert_eq!(
            retry_launch_preflight(&empty_observation(), &run, RetryAuthority::Supervisor)
                .await
                .unwrap_err()
                .code,
            "original_identity_unverifiable"
        );
        retry_launch_preflight(&empty_observation(), &run, RetryAuthority::Operator)
            .await
            .unwrap();

        run.bound_omp_process = Some(NativeProcessIdentity {
            pid: 0,
            start_ticks: 1,
            kernel_boot_id: kernel_boot_id(),
        });
        assert_eq!(original_process(&run), OriginalProcess::Unverifiable);
        assert_eq!(
            retry_launch_preflight(&empty_observation(), &run, RetryAuthority::Supervisor)
                .await
                .unwrap_err()
                .code,
            "original_identity_unverifiable"
        );
        retry_launch_preflight(&empty_observation(), &run, RetryAuthority::Operator)
            .await
            .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn bind_process(run: &mut Run, pid: u32) {
        run.bound_omp_session = Some("native".into());
        run.bound_omp_process = Some(NativeProcessIdentity {
            pid,
            start_ticks: cockpit_core::process_identity::start_identity(pid as i32).unwrap(),
            kernel_boot_id: kernel_boot_id(),
        });
        run.dispatch.as_mut().unwrap().agent_started = true;
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn fresh_retry_never_duplicates_a_live_original_even_without_its_pane() {
        let mut run = reviewed();
        bind_process(&mut run, std::process::id());
        assert_eq!(original_process(&run), OriginalProcess::Running);
        for authority in [RetryAuthority::Operator, RetryAuthority::Supervisor] {
            assert_eq!(
                retry_launch_preflight(&empty_observation(), &run, authority)
                    .await
                    .unwrap_err()
                    .code,
                "original_agent_present"
            );
        }
        // Reusing the PID with a different start identity proves the old
        // incarnation exited; PID-only liveness must not veto that proof.
        run.bound_omp_process.as_mut().unwrap().start_ticks += 1;
        assert_eq!(original_process(&run), OriginalProcess::Exited);
        for authority in [RetryAuthority::Operator, RetryAuthority::Supervisor] {
            retry_launch_preflight(&empty_observation(), &run, authority)
                .await
                .unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn suspended_original_is_not_absent_but_reaped_original_is() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "read line"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut run = reviewed();
        bind_process(&mut run, child.id());
        // This signal targets only this test-owned disposable process.
        let stopped = unsafe { nix::libc::kill(child.id() as i32, nix::libc::SIGSTOP) };
        let suspended = stopped == 0 && {
            let mut status = 0;
            // Observe the stop, not merely successful signal delivery.
            let waited = unsafe {
                nix::libc::waitpid(child.id() as i32, &mut status, nix::libc::WUNTRACED)
            };
            waited == child.id() as i32 && nix::libc::WIFSTOPPED(status)
        };
        let operator = retry_launch_preflight(&empty_observation(), &run, RetryAuthority::Operator)
            .await;
        let supervisor = retry_launch_preflight(&empty_observation(), &run, RetryAuthority::Supervisor)
            .await;
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(suspended);
        assert_eq!(operator.unwrap_err().code, "original_agent_present");
        assert_eq!(supervisor.unwrap_err().code, "original_agent_present");
        assert_eq!(original_process(&run), OriginalProcess::Exited);
        for authority in [RetryAuthority::Operator, RetryAuthority::Supervisor] {
            retry_launch_preflight(&empty_observation(), &run, authority)
                .await
                .unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn changed_boot_proves_exit_but_unavailable_boot_does_not() {
        let mut run = reviewed();
        bind_process(&mut run, std::process::id());
        let process = run.bound_omp_process.as_mut().unwrap();
        assert_eq!(
            original_process_at_boot(process, None),
            OriginalProcess::Unverifiable
        );
        let recorded_boot = process.kernel_boot_id.as_mut().unwrap();
        let different = if recorded_boot.starts_with('0') { "1" } else { "0" };
        recorded_boot.replace_range(0..1, different);
        assert_eq!(original_process(&run), OriginalProcess::Exited);
        for authority in [RetryAuthority::Operator, RetryAuthority::Supervisor] {
            retry_launch_preflight(&empty_observation(), &run, authority)
                .await
                .unwrap();
        }
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
