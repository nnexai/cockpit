use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use cockpit_protocol::projects::{
    OrchestrationConfiguration, ProjectConfiguration, ProjectProvider, ProviderDeployment,
    ProviderKind,
};
use serde::Deserialize;

use crate::InspectionError;

mod limits;
mod roots;

use limits::TomlLimits;

const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_TEXT_BYTES: usize = 4096;
const CONFIG_VERSION: u32 = 1;
const DEFAULT_WINDOW_SCALE_FACTOR: f64 = 1.0;
const DEFAULT_WINDOW_DECORATIONS: bool = true;
const OMP_SYSTEM_DIRS: [&str; 3] = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/home/linuxbrew/.linuxbrew/bin",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowConfiguration {
    pub scale_factor: f64,
    pub decorations: bool,
}

/// OMP uses its own existing authentication; Cockpit supplies no tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaConfiguration {
    pub omp_executable: PathBuf,
}

/// Eventual upstream synchronization policy, separate from Library item limits.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LibrarySyncConfiguration {
    pub enabled: bool,
    pub delta_minutes: u32,
    pub lag_allowance_minutes: u32,
    pub overlap_minutes: u32,
    pub inventory_hours: u32,
    pub audit_days: u32,
    pub related_hours: u32,
    pub background_min_interval_seconds: u32,
    pub background_in_flight: u32,
    pub hourly_request_cap: u32,
}

impl Default for LibrarySyncConfiguration {
    fn default() -> Self {
        Self {
            enabled: true,
            delta_minutes: 60,
            lag_allowance_minutes: 5,
            overlap_minutes: 30,
            inventory_hours: 24,
            audit_days: 7,
            related_hours: 24,
            background_min_interval_seconds: 10,
            background_in_flight: 1,
            hourly_request_cap: 300,
        }
    }
}

impl LibrarySyncConfiguration {
    /// Validate both TOML-loaded and programmatically supplied policies.
    pub fn validate(&self) -> Result<(), InspectionError> {
        for (name, value, minimum, maximum) in [
            ("delta_minutes", self.delta_minutes, 1, 10_080),
            ("lag_allowance_minutes", self.lag_allowance_minutes, 0, 1440),
            ("overlap_minutes", self.overlap_minutes, 0, 10_080),
            ("inventory_hours", self.inventory_hours, 1, 8760),
            ("audit_days", self.audit_days, 1, 365),
            ("related_hours", self.related_hours, 1, 8760),
            (
                "background_min_interval_seconds",
                self.background_min_interval_seconds,
                1,
                3600,
            ),
            ("background_in_flight", self.background_in_flight, 1, 32),
            ("hourly_request_cap", self.hourly_request_cap, 1, 100_000),
        ] {
            if !(minimum..=maximum).contains(&value) {
                return Err(InspectionError::new(
                    "invalid_library_sync_configuration",
                    format!("library_sync.{name} must be {minimum}–{maximum}"),
                ));
            }
        }
        Ok(())
    }
}

/// Browser launch settings and paths for tooling Cockpit is allowed to invoke.
///
/// The CLI is resolved from the owner's environment. Its normal browser selection
/// is preserved unless an executable override is configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserConfiguration {
    pub playwright_cli: PathBuf,
    pub default_url: String,
    pub chromium_executable: Option<PathBuf>,
    /// Optional Node executable used only by the private inline-browser helper.
    pub node_executable: Option<PathBuf>,
    /// Optional installed helper module. When absent, the host packages its own.
    pub browser_helper: Option<PathBuf>,
    /// Optional pinned `playwright-core` module directory used by the helper.
    pub playwright_core: Option<PathBuf>,
    pub feedback_retention_seconds: u64,
    pub feedback_max_store_bytes: u64,
}

const DEFAULT_PLAYWRIGHT_CLI: &str = "playwright-cli";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlConfiguration {
    version: Option<u32>,
    repository_roots: Option<Vec<String>>,
    worktree_root: Option<String>,
    state_root: Option<String>,
    cache_root: Option<String>,
    branch_template: Option<String>,
    checkout_template: Option<String>,
    library_root: Option<String>,
    notes_root: Option<String>,
    providers: Option<Vec<ProjectProvider>>,
    limits: Option<TomlLimits>,
    window: Option<TomlWindow>,
    browser: Option<TomlBrowser>,
    quota: Option<TomlQuota>,
    library_sync: Option<LibrarySyncConfiguration>,
    orchestration: Option<OrchestrationConfiguration>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlWindow {
    scale_factor: Option<f64>,
    decorations: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlBrowser {
    playwright_cli: Option<String>,
    default_url: Option<String>,
    chromium_executable: Option<String>,
    node_executable: Option<String>,
    browser_helper: Option<String>,
    playwright_core: Option<String>,
    feedback_retention_seconds: Option<u64>,
    feedback_max_store_bytes: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlQuota {
    omp_executable: Option<String>,
}

/// One bounded read and parse of the shared Cockpit TOML, split by move into sections.
///
/// Each section resolves at most once and validates only its own policy. Explicit
/// arguments override environment, then TOML, then safe defaults. Resolution does
/// not create directories; the repository catalog reports missing roots.
#[derive(Debug)]
pub struct ConfigurationFile {
    pub project: ProjectSection,
    pub browser: BrowserSection,
    pub quota: QuotaSection,
    pub library_sync: LibrarySyncSection,
    pub window: WindowSection,
}

#[derive(Debug)]
pub struct ProjectSection(TomlConfiguration);

#[derive(Debug)]
pub struct BrowserSection(Option<TomlBrowser>);

#[derive(Debug)]
pub struct QuotaSection(Option<TomlQuota>);

#[derive(Debug)]
pub struct LibrarySyncSection(Option<LibrarySyncConfiguration>);

#[derive(Debug)]
pub struct WindowSection(Option<TomlWindow>);

impl ProjectSection {
    pub fn resolve(
        self,
        repository_roots: Option<&[PathBuf]>,
    ) -> Result<ProjectConfiguration, InspectionError> {
        let file = self.0;
        let roots = roots::resolve(
            roots::RawRoots {
                repository_roots: file.repository_roots,
                cache_root: file.cache_root,
                worktree_root: file.worktree_root,
                state_root: file.state_root,
                library_root: file.library_root,
                notes_root: file.notes_root,
            },
            repository_roots,
        )?;
        let branch_template = file
            .branch_template
            .unwrap_or_else(|| "cockpit/{repo}/{task_id}".into());
        let checkout_template = file
            .checkout_template
            .unwrap_or_else(|| "{repo}-{task_id}".into());
        validate_template(&branch_template, false, "branch_template")?;
        validate_template(&checkout_template, true, "checkout_template")?;
        let limits = file.limits.unwrap_or_default().resolve()?;
        let mut providers = file.providers.unwrap_or_default();
        if providers.len() > 128 {
            return Err(InspectionError::new(
                "invalid_providers",
                "providers cannot contain more than 128 entries",
            ));
        }
        for provider in &mut providers {
            validate_provider(provider)?;
            if matches!(provider.kind, ProviderKind::Jira | ProviderKind::Confluence) {
                provider.deployment = Some(resolved_deployment(provider)?);
            }
        }
        let mut orchestration = file.orchestration.unwrap_or_default();
        if let Some(extension) = env::var_os("COCKPIT_OMP_EXTENSION") {
            orchestration.omp_extension = Some(path_text(Path::new(&extension), "omp_extension")?);
        }
        validate_orchestration(&orchestration)?;
        Ok(ProjectConfiguration {
            version: file.version.unwrap_or(CONFIG_VERSION),
            repository_roots: roots.repository,
            worktree_root: roots.worktree,
            cache_root: roots.cache,
            state_root: roots.state,
            library_root: roots.library,
            notes_root: roots.notes,
            branch_template,
            checkout_template,
            providers,
            limits,
            orchestration,
        })
    }
}

impl BrowserSection {
    /// Environment overrides `[browser]`; missing optional paths remain absent.
    pub fn resolve(self) -> Result<BrowserConfiguration, InspectionError> {
        let browser = self.0.unwrap_or_default();
        let playwright_cli = configured("COCKPIT_PLAYWRIGHT_CLI", browser.playwright_cli)?
            .unwrap_or_else(|| DEFAULT_PLAYWRIGHT_CLI.into());
        let default_url = configured("COCKPIT_BROWSER_DEFAULT_URL", browser.default_url)?
            .unwrap_or_else(|| "about:blank".into());
        crate::browser::validate_url(&default_url)?;
        let chromium_executable =
            configured("COCKPIT_CHROMIUM_EXECUTABLE", browser.chromium_executable)?;
        let node_executable = configured("COCKPIT_NODE_EXECUTABLE", browser.node_executable)?;
        let browser_helper = configured("COCKPIT_BROWSER_HELPER", browser.browser_helper)?;
        let playwright_core = configured("COCKPIT_PLAYWRIGHT_CORE", browser.playwright_core)?;
        let feedback_retention_seconds = browser.feedback_retention_seconds.unwrap_or(3600);
        let feedback_max_store_bytes = browser
            .feedback_max_store_bytes
            .unwrap_or(256 * 1024 * 1024);
        if !(1..=7 * 24 * 3600).contains(&feedback_retention_seconds)
            || !(4 * 1024 * 1024..=4 * 1024 * 1024 * 1024).contains(&feedback_max_store_bytes)
        {
            return Err(InspectionError::new(
                "invalid_browser_configuration",
                "Browser feedback retention must be 1–604800 seconds and storage must be 4 MiB–4 GiB",
            ));
        }
        Ok(BrowserConfiguration {
            playwright_cli: PathBuf::from(playwright_cli),
            default_url,
            chromium_executable: chromium_executable.map(PathBuf::from),
            node_executable: node_executable.map(PathBuf::from),
            browser_helper: browser_helper.map(PathBuf::from),
            playwright_core: playwright_core.map(PathBuf::from),
            feedback_retention_seconds,
            feedback_max_store_bytes,
        })
    }
}

impl QuotaSection {
    /// Discover OMP only when neither environment nor `[quota]` selects it.
    pub fn resolve(self) -> Result<QuotaConfiguration, InspectionError> {
        let quota = self.0.unwrap_or_default();
        let omp_executable = match configured("COCKPIT_OMP_EXECUTABLE", quota.omp_executable)? {
            Some(omp) => PathBuf::from(omp),
            None => {
                let path = env::var_os("PATH");
                let home = env::var_os("HOME").map(PathBuf::from);
                default_omp_executable(
                    path.as_deref(),
                    home.as_deref(),
                    &OMP_SYSTEM_DIRS.map(Path::new),
                )
            }
        };
        Ok(QuotaConfiguration { omp_executable })
    }
}

impl LibrarySyncSection {
    /// Resolve synchronization policy without changing project item limits.
    pub fn resolve(self) -> Result<LibrarySyncConfiguration, InspectionError> {
        let configuration = self.0.unwrap_or_default();
        configuration.validate()?;
        Ok(configuration)
    }
}

fn default_omp_executable(
    path: Option<&OsStr>,
    home: Option<&Path>,
    system_dirs: &[&Path],
) -> PathBuf {
    let executable_name = if cfg!(windows) { "omp.exe" } else { "omp" };
    if let Some(path) = path {
        for directory in
            env::split_paths(path).filter(|directory| !directory.as_os_str().is_empty())
        {
            let candidate = directory.join(executable_name);
            if is_omp_executable_file(&candidate) {
                return candidate;
            }
        }
    }
    if let Some(home) = home.filter(|home| !home.as_os_str().is_empty()) {
        for relative in [".local/bin", ".bun/bin"] {
            let candidate = home.join(relative).join(executable_name);
            if is_omp_executable_file(&candidate) {
                return candidate;
            }
        }
    }
    for directory in system_dirs {
        let candidate = directory.join(executable_name);
        if is_omp_executable_file(&candidate) {
            return candidate;
        }
    }
    PathBuf::from("omp")
}

#[cfg(unix)]
fn is_omp_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_omp_executable_file(path: &Path) -> bool {
    path.is_file()
}

impl WindowSection {
    /// All platforms default to a 1.0 webview scale and native decorations.
    pub fn resolve(self) -> Result<WindowConfiguration, InspectionError> {
        let window = self.0.unwrap_or_default();
        let scale_factor = window.scale_factor.unwrap_or(DEFAULT_WINDOW_SCALE_FACTOR);
        if !scale_factor.is_finite() || !(0.2..=10.0).contains(&scale_factor) {
            return Err(InspectionError::new(
                "invalid_window_scale_factor",
                "window.scale_factor must be finite and between 0.2 and 10.0",
            ));
        }
        Ok(WindowConfiguration {
            scale_factor,
            decorations: window.decorations.unwrap_or(DEFAULT_WINDOW_DECORATIONS),
        })
    }
}

/// The same invocation/environment/default resolution used by every config loader.
/// A missing implicit default means no file; an explicit path is never guessed.
pub fn configuration_path(config_path: Option<&Path>) -> Result<Option<PathBuf>, InspectionError> {
    let path = if let Some(path) = config_path {
        Some(path.to_owned())
    } else if let Some(path) = env::var_os("COCKPIT_CONFIG") {
        Some(PathBuf::from(path))
    } else {
        let default = xdg_directory("XDG_CONFIG_HOME", ".config")?.join("cockpit/config.toml");
        match default.try_exists() {
            Ok(true) => Some(default),
            Ok(false) => None,
            Err(_) => {
                return Err(InspectionError::new(
                    "config_unavailable",
                    "Cannot inspect the default Cockpit configuration",
                ));
            }
        }
    };
    Ok(path)
}

impl ConfigurationFile {
    pub fn load(config_path: Option<&Path>) -> Result<Self, InspectionError> {
        let path = configuration_path(config_path)?;
        let Some(path) = path else {
            return Ok(Self::from_toml(TomlConfiguration::default()));
        };
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(nix::libc::O_NONBLOCK);
        }
        let file = options.open(&path).map_err(|_| {
            InspectionError::new(
                "config_unavailable",
                format!("Cannot open configuration {}", path.display()),
            )
        })?;
        if !file
            .metadata()
            .map_err(|_| {
                InspectionError::new("config_unavailable", "Cannot inspect configuration")
            })?
            .is_file()
        {
            return Err(InspectionError::new(
                "invalid_config",
                "Configuration must be a regular file",
            ));
        }
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| InspectionError::new("config_unavailable", "Cannot read configuration"))?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(InspectionError::new(
                "config_too_large",
                format!("configuration exceeds {MAX_CONFIG_BYTES} bytes"),
            ));
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            InspectionError::new("invalid_config", "configuration is not valid UTF-8")
        })?;
        let value: TomlConfiguration =
            toml::from_str(&text).map_err(|error: toml::de::Error| {
                InspectionError::new(
                    "invalid_config",
                    format!("Invalid configuration TOML: {}", error.message()),
                )
            })?;
        let version = value.version.unwrap_or(CONFIG_VERSION);
        if version != CONFIG_VERSION {
            return Err(InspectionError::new(
                "unsupported_config_version",
                format!(
                    "configuration version {version} is unsupported; expected {CONFIG_VERSION}"
                ),
            ));
        }
        Ok(Self::from_toml(value))
    }

    fn from_toml(mut file: TomlConfiguration) -> Self {
        Self {
            browser: BrowserSection(file.browser.take()),
            quota: QuotaSection(file.quota.take()),
            library_sync: LibrarySyncSection(file.library_sync.take()),
            window: WindowSection(file.window.take()),
            project: ProjectSection(file),
        }
    }
}

fn configured(name: &str, file: Option<String>) -> Result<Option<String>, InspectionError> {
    let value = if let Some(value) = env::var_os(name) {
        Some(value.into_string().map_err(|_| {
            InspectionError::new(
                format!("invalid_{name}"),
                "Configured path is not valid UTF-8",
            )
        })?)
    } else {
        file
    };
    if let Some(value) = &value {
        validate_text(value, name)?;
    }
    Ok(value)
}

fn validate_paths(paths: &[String], field: &str) -> Result<(), InspectionError> {
    for path in paths {
        validate_text(path, field)?;
        if !Path::new(path).is_absolute()
            || Path::new(path)
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(InspectionError::new(
                format!("invalid_{field}"),
                format!("{field} must be an absolute path without parent traversal"),
            ));
        }
    }
    Ok(())
}

fn validate_orchestration(
    configuration: &OrchestrationConfiguration,
) -> Result<(), InspectionError> {
    if let Some(path) = &configuration.omp_extension {
        validate_text(path, "omp_extension")?;
        if !Path::new(path).is_absolute() {
            return Err(InspectionError::new(
                "invalid_omp_extension",
                "omp_extension must be absolute",
            ));
        }
    }
    if let Some(model) = &configuration.model {
        validate_text(model, "orchestration_model")?;
        if model.starts_with('-') {
            return Err(InspectionError::new(
                "invalid_orchestration_model",
                "model cannot be an option",
            ));
        }
    }
    validate_orchestration_args(&configuration.extra_args)?;
    if configuration.routes.len() > 256 {
        return Err(InspectionError::new(
            "invalid_orchestration_routes",
            "At most 256 routes are supported",
        ));
    }
    for route in &configuration.routes {
        validate_text(&route.provider, "route_provider")?;
        validate_text(&route.instance, "route_instance")?;
        validate_text(&route.project_id_prefix, "route_project_id_prefix")?;
        validate_text(&route.repository_id, "route_repository_id")?;
        let url = url::Url::parse(&route.instance).map_err(|_| {
            InspectionError::new(
                "invalid_route_instance",
                "instance must be an HTTP(S) origin",
            )
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(InspectionError::new(
                "invalid_route_instance",
                "instance must be an HTTP(S) origin without credentials or a path",
            ));
        }
    }
    Ok(())
}

/// Only process options that do not load code, resume sessions, or bypass grants.
pub(crate) fn validate_orchestration_args(args: &[String]) -> Result<(), InspectionError> {
    if args.len() > 16 {
        return Err(InspectionError::new(
            "invalid_orchestration_args",
            "At most 16 OMP arguments are supported",
        ));
    }
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--no-extensions" | "--no-skills" | "--no-rules" => {}
            "--thinking"
                if args.get(index + 1).is_some_and(|value| {
                    matches!(
                        value.as_str(),
                        "off" | "minimal" | "low" | "medium" | "high"
                    )
                }) =>
            {
                index += 1
            }
            _ => {
                return Err(InspectionError::new(
                    "invalid_orchestration_args",
                    "Only --no-extensions, --no-skills, --no-rules and --thinking LEVEL are supported",
                ));
            }
        }
        index += 1;
    }
    Ok(())
}

fn validate_text(value: &str, field: &str) -> Result<(), InspectionError> {
    if value.trim().is_empty()
        || value.len() > MAX_TEXT_BYTES
        || value.chars().any(|c| c == '\0' || c.is_control())
    {
        return Err(InspectionError::new(
            format!("invalid_{field}"),
            format!(
                "{field} must be nonempty, printable, and no longer than {MAX_TEXT_BYTES} bytes"
            ),
        ));
    }
    Ok(())
}

fn validate_template(
    template: &str,
    path_template: bool,
    field: &str,
) -> Result<(), InspectionError> {
    validate_text(template, field)?;
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        if rest[..start].contains('}') {
            return Err(InspectionError::new(
                format!("invalid_{field}"),
                format!("{field} contains an unmatched closing brace"),
            ));
        }
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            return Err(InspectionError::new(
                format!("invalid_{field}"),
                format!("{field} contains an unterminated template variable"),
            ));
        };
        let variable = &after[..end];
        if !matches!(variable, "repo" | "task_id" | "slug") {
            return Err(InspectionError::new(
                format!("invalid_{field}"),
                format!("{field} contains unsupported variable {{{variable}}}"),
            ));
        }
        rest = &after[end + 1..];
    }
    if rest.contains('}')
        || (path_template
            && (Path::new(template).is_absolute()
                || template.split(['/', '\\']).any(|part| part == "..")))
    {
        return Err(InspectionError::new(
            format!("invalid_{field}"),
            format!("{field} is not a safe relative destination"),
        ));
    }
    Ok(())
}

fn validate_provider(provider: &ProjectProvider) -> Result<(), InspectionError> {
    validate_text(&provider.id, "provider_id")?;
    validate_text(&provider.base_url, "provider_base_url")?;
    if matches!(provider.kind, ProviderKind::Jira | ProviderKind::Confluence) {
        if provider.executable.is_some() || provider.login.is_some() {
            return Err(InspectionError::new(
                "invalid_config",
                "provider fields are incompatible with kind",
            ));
        }
    } else {
        let executable = provider.executable.as_deref().ok_or_else(|| {
            InspectionError::new(
                "invalid_provider_executable",
                "GitHub, GitLab and Gitea providers require executable",
            )
        })?;
        validate_text(executable, "provider_executable")?;
        if provider.deployment.is_some() {
            return Err(InspectionError::new(
                "invalid_provider_deployment",
                "deployment is only supported for Jira and Confluence",
            ));
        }
    }
    let parsed = url::Url::parse(&provider.base_url).map_err(|_| {
        InspectionError::new(
            "invalid_provider_base_url",
            "provider base_url must be an absolute URL",
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(InspectionError::new(
            "invalid_provider_base_url",
            "provider base_url must be credential-free HTTP(S)",
        ));
    }
    if provider.kind == ProviderKind::Confluence
        && resolved_deployment(provider)? == ProviderDeployment::Cloud
        && parsed.path() != "/wiki"
    {
        return Err(InspectionError::new(
            "invalid_provider_base_url",
            "Confluence Cloud base_url must have exactly the /wiki path",
        ));
    }
    if let Some(login) = &provider.login {
        if login.is_empty() || login.len() > 256 || login.chars().any(char::is_control) {
            return Err(InspectionError::new(
                "invalid_provider_login",
                "provider login must be a non-empty control-free value of at most 256 bytes",
            ));
        }
    }
    Ok(())
}

fn resolved_deployment(provider: &ProjectProvider) -> Result<ProviderDeployment, InspectionError> {
    if let Some(deployment) = provider.deployment {
        return Ok(deployment);
    }
    let url = url::Url::parse(&provider.base_url).map_err(|_| {
        InspectionError::new(
            "invalid_provider_base_url",
            "provider base_url must be an absolute URL",
        )
    })?;
    // URL parsing canonicalizes DNS hosts to ASCII lowercase.
    Ok(
        if url
            .host_str()
            .is_some_and(|host| host.ends_with(".atlassian.net"))
        {
            ProviderDeployment::Cloud
        } else {
            ProviderDeployment::DataCenter
        },
    )
}

fn path_text(path: &Path, field: &str) -> Result<String, InspectionError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        InspectionError::new(
            format!("invalid_{field}"),
            "Configured path is not valid UTF-8",
        )
    })
}

fn xdg_directory(variable: &str, home_suffix: &str) -> Result<PathBuf, InspectionError> {
    let path = if let Some(value) = env::var_os(variable) {
        PathBuf::from(value)
    } else {
        PathBuf::from(env::var_os("HOME").ok_or_else(|| {
            InspectionError::new(
                "configuration_unavailable",
                format!("Set {variable} or HOME to locate Cockpit configuration/state"),
            )
        })?)
        .join(home_suffix)
    };
    validate_paths(&[path_text(&path, variable)?], variable)?;
    Ok(path)
}

#[cfg(test)]
mod tests;
