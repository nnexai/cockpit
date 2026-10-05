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
            cli_path: cli_path()?,
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
        let service = self.service.clone();
        tokio::task::spawn_blocking(move || service.mutate(&Actor::Operator(origin), request))
            .await
            .map_err(|error| {
                InspectionError::new("orchestration_runtime_failed", error.to_string())
            })?
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

fn cli_path() -> Result<PathBuf, InspectionError> {
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
    // Browser builds run the CLI itself; native installs place cockpit-cli beside it.
    if executable
        .file_name()
        .is_some_and(|name| name == "cockpit" || name == "cockpit-cli")
    {
        return Ok(executable);
    }
    let sibling = executable.with_file_name("cockpit-cli");
    if sibling.is_file() {
        return Ok(sibling);
    }
    // Repository builds keep the host binary beside cockpit-tauri under its Cargo name.
    let built_sibling = executable.with_file_name("cockpit");
    Ok(if built_sibling.is_file() {
        built_sibling
    } else {
        PathBuf::from("cockpit-cli")
    })
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
