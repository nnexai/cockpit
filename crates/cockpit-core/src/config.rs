use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use cockpit_protocol::projects::{OrchestrationConfiguration, ProjectConfiguration, ProjectLimits, ProjectProvider, ProviderKind, ProviderDeployment};
use serde::Deserialize;

use crate::InspectionError;

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
            ("background_min_interval_seconds", self.background_min_interval_seconds, 1, 3600),
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
    companion_root: Option<String>,
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlLimits {
    catalog_depth: Option<u32>,
    catalog_entries: Option<u32>,
    git_timeout_ms: Option<u32>,
    git_output_bytes: Option<u32>,
    operation_timeout_ms: Option<u32>,
    context_preview_bytes: Option<u32>,
    context_preview_lines: Option<u32>,
    context_directory_entries: Option<u32>,
    context_tree_depth: Option<u32>,
    library_folder_files: Option<u32>,
    library_folder_bytes: Option<u64>,
    library_file_bytes: Option<u64>,
    library_space_pages: Option<u32>,
    library_attachment_bytes: Option<u64>,
    library_item_attachment_bytes: Option<u64>,
    library_max_items: Option<u32>,
}


/// Load the effective project policy. Explicit arguments override environment,
/// which overrides the versioned TOML file, which overrides safe defaults.
///
/// The loader intentionally does not create any configured directory. Missing
/// roots are reported by the repository catalog with remediation guidance.
pub fn load_project_configuration(
    config_path: Option<&Path>,
    repository_roots: Option<&[PathBuf]>,
) -> Result<ProjectConfiguration, InspectionError> {
    let (file, file_origin) = load_file_configuration(config_path)?;
    let mut origins = BTreeMap::new();

    origins.insert(
        "version".into(),
        origin(file.version.is_some(), &file_origin, false),
    );

    let (roots, roots_origin) = if let Some(roots) = repository_roots {
        if roots.is_empty() {
            return Err(InspectionError::new(
                "invalid_repository_roots",
                "repository_roots invocation override must contain at least one path",
            ));
        }
        (
            roots
                .iter()
                .map(|path| path_text(path, "repository_roots"))
                .collect::<Result<Vec<_>, _>>()?,
            "invocation",
        )
    } else if let Some(value) = env::var_os("COCKPIT_REPOSITORY_ROOTS") {
        let roots: Vec<String> = env::split_paths(&value)
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| path_text(&path, "repository_roots"))
            .collect::<Result<Vec<_>, _>>()?;
        if roots.is_empty() {
            return Err(InspectionError::new(
                "invalid_repository_roots",
                "COCKPIT_REPOSITORY_ROOTS must contain at least one path",
            ));
        }
        (roots, "environment")
    } else if let Some(roots) = file.repository_roots.clone() {
        (roots, "toml")
    } else {
        (
            vec![path_text(
                &env::current_dir().map_err(|_| {
                    InspectionError::new(
                        "configuration_unavailable",
                        "Cannot resolve the current repository directory",
                    )
                })?,
                "repository_roots",
            )?],
            "default",
        )
    };
    if roots.is_empty() {
        return Err(InspectionError::new(
            "invalid_repository_roots",
            "repository_roots must contain at least one path",
        ));
    }
    if roots.len() > 1024 {
        return Err(InspectionError::new(
            "invalid_repository_roots",
            "repository_roots cannot contain more than 1024 paths",
        ));
    }
    validate_paths(&roots, "repository_roots")?;
    origins.insert("repository_roots".into(), roots_origin.into());


    let default_cache = xdg_directory("XDG_CACHE_HOME", ".cache")?.join("cockpit");
    let (cache_root, cache_origin) = choose_path(
        "COCKPIT_CACHE_ROOT",
        file.cache_root.clone(),
        &path_text(&default_cache, "cache_root")?,
    )?;
    validate_paths(&[cache_root.clone()], "cache_root")?;
    origins.insert("cache_root".into(), cache_origin.into());
    let default_state = xdg_directory("XDG_STATE_HOME", ".local/state")?.join("cockpit");

    let (worktree_root, worktree_origin) = choose_path(
        "COCKPIT_WORKTREE_ROOT",
        file.worktree_root.clone(),
        &path_text(&default_state.join("worktrees"), "worktree_root")?,
    )?;
    let (companion_root, companion_origin) = choose_path(
        "COCKPIT_COMPANION_ROOT",
        file.companion_root.clone(),
        &path_text(&default_state.join("companions"), "companion_root")?,
    )?;
    let (state_root, state_origin) = choose_path(
        "COCKPIT_STATE_ROOT",
        file.state_root.clone(),
        &path_text(&default_state.join("operations"), "state_root")?,
    )?;
    validate_paths(&[worktree_root.clone()], "worktree_root")?;
    validate_paths(&[companion_root.clone()], "companion_root")?;
    validate_paths(&[state_root.clone()], "state_root")?;
    origins.insert("worktree_root".into(), worktree_origin.into());
    origins.insert("companion_root".into(), companion_origin.into());
    origins.insert("state_root".into(), state_origin.into());
    let default_library = xdg_directory("XDG_DATA_HOME", ".local/share")?.join("cockpit/library");
    let (library_root, library_origin) = choose_path(
        "COCKPIT_LIBRARY_ROOT",
        file.library_root.clone(),
        &path_text(&default_library, "library_root")?,
    )?;
    validate_paths(&[library_root.clone()], "library_root")?;
    for (field, root) in [
        ("state_root", state_root.as_str()),
        ("companion_root", companion_root.as_str()),
        ("worktree_root", worktree_root.as_str()),
        ("cache_root", cache_root.as_str()),
    ] {
        if paths_overlap_lexically(&library_root, root) {
            return Err(InspectionError::new(
                "invalid_library_root",
                format!("library_root must not overlap {field}"),
            ));
        }
    }
    origins.insert("library_root".into(), library_origin.into());
    let default_notes = xdg_directory("XDG_DATA_HOME", ".local/share")?.join("cockpit/notes");
    let (notes_root, notes_origin) = choose_path(
        "COCKPIT_NOTES_ROOT",
        file.notes_root.clone(),
        &path_text(&default_notes, "notes_root")?,
    )
    .map_err(|error| InspectionError::new("invalid_notes_root", error.message))?;
    validate_paths(&[notes_root.clone()], "notes_root")?;
    for (field, root) in [
        ("library_root", library_root.as_str()),
        ("state_root", state_root.as_str()),
        ("companion_root", companion_root.as_str()),
        ("worktree_root", worktree_root.as_str()),
        ("cache_root", cache_root.as_str()),
    ]
    .into_iter()
    .chain(roots.iter().map(|root| ("repository_roots", root.as_str())))
    {
        if paths_overlap_lexically(&notes_root, root) {
            return Err(InspectionError::new(
                "invalid_notes_root",
                format!("notes_root must not overlap {field}"),
            ));
        }
    }
    origins.insert("notes_root".into(), notes_origin.into());

    let branch_template = file
        .branch_template
        .clone()
        .unwrap_or_else(|| "cockpit/{repo}/{task_id}".into());
    let checkout_template = file
        .checkout_template
        .clone()
        .unwrap_or_else(|| "{repo}-{task_id}".into());
    validate_template(&branch_template, false, "branch_template")?;
    validate_template(&checkout_template, true, "checkout_template")?;
    origins.insert(
        "branch_template".into(),
        if file.branch_template.is_some() {
            "toml"
        } else {
            "default"
        }
        .into(),
    );
    origins.insert(
        "checkout_template".into(),
        if file.checkout_template.is_some() {
            "toml"
        } else {
            "default"
        }
        .into(),
    );

    let limits_from_file = file.limits.is_some();
    let limits_file = file.limits.unwrap_or_default();
    let limits_origins = [
        ("catalog_depth", limits_file.catalog_depth.is_some()),
        ("catalog_entries", limits_file.catalog_entries.is_some()),
        ("git_timeout_ms", limits_file.git_timeout_ms.is_some()),
        ("git_output_bytes", limits_file.git_output_bytes.is_some()),
        (
            "operation_timeout_ms",
            limits_file.operation_timeout_ms.is_some(),
        ),
        (
            "context_preview_bytes",
            limits_file.context_preview_bytes.is_some(),
        ),
        (
            "context_preview_lines",
            limits_file.context_preview_lines.is_some(),
        ),
        (
            "context_directory_entries",
            limits_file.context_directory_entries.is_some(),
        ),
        (
            "context_tree_depth",
            limits_file.context_tree_depth.is_some(),
        ),
        ("library_folder_files", limits_file.library_folder_files.is_some()),
        ("library_folder_bytes", limits_file.library_folder_bytes.is_some()),
        ("library_file_bytes", limits_file.library_file_bytes.is_some()),
        ("library_space_pages", limits_file.library_space_pages.is_some()),
        ("library_attachment_bytes", limits_file.library_attachment_bytes.is_some()),
        ("library_item_attachment_bytes", limits_file.library_item_attachment_bytes.is_some()),
        ("library_max_items", limits_file.library_max_items.is_some()),
    ];
    let limits = ProjectLimits {
        catalog_depth: bounded_limit(
            limits_file.catalog_depth.unwrap_or(3),
            1,
            32,
            "catalog_depth",
        )?,
        catalog_entries: bounded_limit(
            limits_file.catalog_entries.unwrap_or(16_384),
            1,
            100_000,
            "catalog_entries",
        )?,
        git_timeout_ms: bounded_limit(
            limits_file.git_timeout_ms.unwrap_or(3000),
            1,
            120_000,
            "git_timeout_ms",
        )?,
        git_output_bytes: bounded_limit(
            limits_file.git_output_bytes.unwrap_or(1024 * 1024),
            1024,
            16 * 1024 * 1024,
            "git_output_bytes",
        )?,
        operation_timeout_ms: bounded_limit(
            limits_file.operation_timeout_ms.unwrap_or(30_000),
            1,
            600_000,
            "operation_timeout_ms",
        )?,
        context_preview_bytes: bounded_limit(
            limits_file.context_preview_bytes.unwrap_or(1024 * 1024),
            1024,
            8 * 1024 * 1024,
            "context_preview_bytes",
        )?,
        context_preview_lines: bounded_limit(
            limits_file.context_preview_lines.unwrap_or(5000),
            1,
            20_000,
            "context_preview_lines",
        )?,
        context_directory_entries: bounded_limit(
            limits_file.context_directory_entries.unwrap_or(1000),
            1,
            10_000,
            "context_directory_entries",
        )?,
        context_tree_depth: bounded_limit(
            limits_file.context_tree_depth.unwrap_or(32),
            1,
            64,
            "context_tree_depth",
        )?,
        library_folder_files: bounded_limit(limits_file.library_folder_files.unwrap_or(512), 1, 100_000, "library_folder_files")?,
        library_folder_bytes: bounded_limit_u64(limits_file.library_folder_bytes.unwrap_or(32 * 1024 * 1024), 1024 * 1024, 4 * 1024 * 1024 * 1024 - 1, "library_folder_bytes")?,
        library_file_bytes: bounded_limit_u64(limits_file.library_file_bytes.unwrap_or(4 * 1024 * 1024), 1024, 1024 * 1024 * 1024, "library_file_bytes")?,
        library_space_pages: bounded_limit(limits_file.library_space_pages.unwrap_or(200), 1, 20_000, "library_space_pages")?,
        library_attachment_bytes: bounded_limit_u64(limits_file.library_attachment_bytes.unwrap_or(25 * 1024 * 1024), 1024, 1024 * 1024 * 1024, "library_attachment_bytes")?,
        library_item_attachment_bytes: bounded_limit_u64(limits_file.library_item_attachment_bytes.unwrap_or(100 * 1024 * 1024), 1024 * 1024, 4 * 1024 * 1024 * 1024 - 1, "library_item_attachment_bytes")?,
        library_max_items: bounded_limit(limits_file.library_max_items.unwrap_or(20_000), 100, 1_000_000, "library_max_items")?,
    };
    for (field, from_file) in limits_origins {
        origins.insert(
            format!("limits.{field}"),
            if from_file && limits_from_file {
                "toml"
            } else {
                "default"
            }
            .into(),
        );
    }

    let providers_from_file = file.providers.is_some();
    let mut providers = file.providers.unwrap_or_default();
    if providers.len() > 128 {
        return Err(InspectionError::new(
            "invalid_providers",
            "providers cannot contain more than 128 entries",
        ));
    }
    for provider in &mut providers {
        let explicit_deployment = provider.deployment.is_some();
        validate_provider(provider)?;
        if matches!(provider.kind, ProviderKind::Jira | ProviderKind::Confluence) {
            provider.deployment = Some(resolved_deployment(provider)?);
            origins.insert(
                format!("providers.{}.deployment", provider.id),
                if explicit_deployment { "toml" } else { "default" }.into(),
            );
        }
    }
    let provider_origin = if providers_from_file {
        "toml"
    } else {
        "default"
    };
    origins.insert("providers".into(), provider_origin.into());
    for provider in &providers {
        origins.insert(
            format!("providers.{}.base_url", provider.id),
            provider_origin.into(),
        );
        origins.insert(
            format!("providers.{}.kind", provider.id),
            provider_origin.into(),
        );
        if !matches!(provider.kind, ProviderKind::Jira | ProviderKind::Confluence) {
            origins.insert(
                format!("providers.{}.executable", provider.id),
                provider_origin.into(),
            );
            origins.insert(
                format!("providers.{}.login", provider.id),
                if provider.login.is_some() { "toml" } else { "default" }.into(),
            );
        }
    }

    let orchestration_from_file = file.orchestration.is_some();
    let mut orchestration = file.orchestration.unwrap_or_default();
    if let Some(extension) = env::var_os("COCKPIT_OMP_EXTENSION") {
        orchestration.omp_extension = Some(path_text(Path::new(&extension), "omp_extension")?);
    }
    validate_orchestration(&orchestration)?;
    origins.insert("orchestration".into(), origin(orchestration_from_file, &file_origin, false));

    Ok(ProjectConfiguration {
        version: file.version.unwrap_or(CONFIG_VERSION),
        repository_roots: roots,
        worktree_root,
        cache_root,
        companion_root,
        state_root,
        library_root,
        notes_root,
        branch_template,
        checkout_template,
        providers,
        limits,
        orchestration,
        origins,
    })
}

/// Load browser launch settings and executable paths from the shared Cockpit TOML.
///
/// Environment values override `[browser]`, while command-name defaults remain
/// explicit so missing installed prerequisites can be reported at use time.
pub fn load_browser_configuration(
    config_path: Option<&Path>,
) -> Result<BrowserConfiguration, InspectionError> {
    let (file, _) = load_file_configuration(config_path)?;
    let browser = file.browser.unwrap_or_default();
    let (playwright_cli, _) = choose_path(
        "COCKPIT_PLAYWRIGHT_CLI",
        browser.playwright_cli,
        DEFAULT_PLAYWRIGHT_CLI,
    )?;
    let (default_url, _) = choose_path(
        "COCKPIT_BROWSER_DEFAULT_URL",
        browser.default_url,
        "about:blank",
    )?;
    crate::browser::validate_url(&default_url)?;
    let (chromium_executable, chromium_source) = choose_path(
        "COCKPIT_CHROMIUM_EXECUTABLE",
        browser.chromium_executable,
        "",
    )?;
    let (node_executable, node_source) =
        choose_path("COCKPIT_NODE_EXECUTABLE", browser.node_executable, "")?;
    let (browser_helper, helper_source) =
        choose_path("COCKPIT_BROWSER_HELPER", browser.browser_helper, "")?;
    let (playwright_core, playwright_core_source) =
        choose_path("COCKPIT_PLAYWRIGHT_CORE", browser.playwright_core, "")?;
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
        chromium_executable: (chromium_source != "default")
            .then(|| PathBuf::from(chromium_executable)),
        node_executable: (node_source != "default").then(|| PathBuf::from(node_executable)),
        browser_helper: (helper_source != "default").then(|| PathBuf::from(browser_helper)),
        playwright_core: (playwright_core_source != "default")
            .then(|| PathBuf::from(playwright_core)),
        feedback_retention_seconds,
        feedback_max_store_bytes,
    })
}

/// Load the OMP quota executable from `[quota]`, overridden by the environment.
pub fn load_quota_configuration(
    config_path: Option<&Path>,
) -> Result<QuotaConfiguration, InspectionError> {
    let (file, _) = load_file_configuration(config_path)?;
    let quota = file.quota.unwrap_or_default();
    let (omp, source) = choose_path("COCKPIT_OMP_EXECUTABLE", quota.omp_executable, "omp")?;
    let omp_executable = if source == "default" {
        let path = env::var_os("PATH");
        let home = env::var_os("HOME").map(PathBuf::from);
        default_omp_executable(
            path.as_deref(),
            home.as_deref(),
            &OMP_SYSTEM_DIRS.map(Path::new),
        )
    } else {
        PathBuf::from(omp)
    };
    Ok(QuotaConfiguration {
        omp_executable,
    })
}

/// Load the optional `[library_sync]` policy without changing project item limits.
pub fn load_library_sync_configuration(
    config_path: Option<&Path>,
) -> Result<LibrarySyncConfiguration, InspectionError> {
    let (file, _) = load_file_configuration(config_path)?;
    let configuration = file.library_sync.unwrap_or_default();
    configuration.validate()?;
    Ok(configuration)
}

fn default_omp_executable(
    path: Option<&OsStr>,
    home: Option<&Path>,
    system_dirs: &[&Path],
) -> PathBuf {
    let executable_name = if cfg!(windows) { "omp.exe" } else { "omp" };
    if let Some(path) = path {
        for directory in env::split_paths(path).filter(|directory| !directory.as_os_str().is_empty()) {
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

/// Load the native window presentation settings from the shared Cockpit TOML.
/// All platforms default to a 1.0 webview scale and native decorations.
pub fn load_window_configuration(
    config_path: Option<&Path>,
) -> Result<WindowConfiguration, InspectionError> {
    let (file, _) = load_file_configuration(config_path)?;
    let window = file.window.unwrap_or_default();
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
            Err(_) => return Err(InspectionError::new("config_unavailable", "Cannot inspect the default Cockpit configuration")),
        }
    };
    Ok(path)
}

fn load_file_configuration(
    config_path: Option<&Path>,
) -> Result<(TomlConfiguration, String), InspectionError> {
    let path = configuration_path(config_path)?;
    let Some(path) = path else {
        return Ok((TomlConfiguration::default(), "default".into()));
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
        .map_err(|_| InspectionError::new("config_unavailable", "Cannot inspect configuration"))?
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
    let text = String::from_utf8(bytes)
        .map_err(|_| InspectionError::new("invalid_config", "configuration is not valid UTF-8"))?;
    let value: TomlConfiguration = toml::from_str(&text).map_err(|error: toml::de::Error| {
        InspectionError::new(
            "invalid_config",
            format!("Invalid configuration TOML: {}", error.message()),
        )
    })?;
    let version = value.version.unwrap_or(CONFIG_VERSION);
    if version != CONFIG_VERSION {
        return Err(InspectionError::new(
            "unsupported_config_version",
            format!("configuration version {version} is unsupported; expected {CONFIG_VERSION}"),
        ));
    }
    Ok((value, "toml".into()))
}

fn choose_path(
    name: &str,
    file: Option<String>,
    default: &str,
) -> Result<(String, &'static str), InspectionError> {
    if let Some(value) = env::var_os(name) {
        let value = value.into_string().map_err(|_| {
            InspectionError::new(
                format!("invalid_{name}"),
                "Configured path is not valid UTF-8",
            )
        })?;
        validate_text(&value, name)?;
        return Ok((value, "environment"));
    }
    if let Some(value) = file {
        validate_text(&value, name)?;
        return Ok((value, "toml"));
    }
    Ok((default.into(), "default"))
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

fn validate_orchestration(configuration: &OrchestrationConfiguration) -> Result<(), InspectionError> {
    if let Some(path) = &configuration.omp_extension {
        validate_text(path, "omp_extension")?;
        if !Path::new(path).is_absolute() {
            return Err(InspectionError::new("invalid_omp_extension", "omp_extension must be absolute"));
        }
    }
    if let Some(model) = &configuration.model {
        validate_text(model, "orchestration_model")?;
        if model.starts_with('-') {
            return Err(InspectionError::new("invalid_orchestration_model", "model cannot be an option"));
        }
    }
    validate_orchestration_args(&configuration.extra_args)?;
    if configuration.routes.len() > 256 {
        return Err(InspectionError::new("invalid_orchestration_routes", "At most 256 routes are supported"));
    }
    for route in &configuration.routes {
        validate_text(&route.provider, "route_provider")?;
        validate_text(&route.instance, "route_instance")?;
        validate_text(&route.project_id_prefix, "route_project_id_prefix")?;
        validate_text(&route.repository_id, "route_repository_id")?;
        let url = url::Url::parse(&route.instance).map_err(|_| InspectionError::new("invalid_route_instance", "instance must be an HTTP(S) origin"))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none()
            || !url.username().is_empty() || url.password().is_some()
            || url.query().is_some() || url.fragment().is_some() || url.path() != "/"
        {
            return Err(InspectionError::new("invalid_route_instance", "instance must be an HTTP(S) origin without credentials or a path"));
        }
    }
    Ok(())
}

/// Only process options that do not load code, resume sessions, or bypass grants.
pub(crate) fn validate_orchestration_args(args: &[String]) -> Result<(), InspectionError> {
    if args.len() > 16 {
        return Err(InspectionError::new("invalid_orchestration_args", "At most 16 OMP arguments are supported"));
    }
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--no-extensions" | "--no-skills" | "--no-rules" => {}
            "--thinking" if args.get(index + 1).is_some_and(|value| matches!(value.as_str(), "off" | "minimal" | "low" | "medium" | "high")) => index += 1,
            _ => return Err(InspectionError::new("invalid_orchestration_args", "Only --no-extensions, --no-skills, --no-rules and --thinking LEVEL are supported")),
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

fn bounded_limit(value: u32, min: u32, max: u32, field: &str) -> Result<u32, InspectionError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(InspectionError::new(
            format!("invalid_{field}"),
            format!("{field} must be between {min} and {max}"),
        ))
    }
}
fn bounded_limit_u64(value: u64, min: u64, max: u64, field: &str) -> Result<u64, InspectionError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(InspectionError::new(
            format!("invalid_{field}"),
            format!("{field} must be between {min} and {max}"),
        ))
    }
}

fn paths_overlap_lexically(first: &str, second: &str) -> bool {
    let first = Path::new(first);
    let second = Path::new(second);
    first.starts_with(second) || second.starts_with(first)
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
        if provider.executable.is_some() {
            return Err(InspectionError::new(
                "invalid_provider_executable",
                "Jira and Confluence use Cockpit's HTTP client; remove executable",
            ));
        }
        if provider.login.is_some() {
            return Err(InspectionError::new(
                "invalid_provider_login",
                "Jira and Confluence use tokens stored in Cockpit; remove login",
            ));
        }
    } else {
        let executable = provider.executable.as_deref().ok_or_else(|| InspectionError::new(
            "invalid_provider_executable",
            "GitHub, GitLab and Gitea providers require executable",
        ))?;
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
    let url = url::Url::parse(&provider.base_url).map_err(|_| InspectionError::new(
        "invalid_provider_base_url",
        "provider base_url must be an absolute URL",
    ))?;
    // URL parsing canonicalizes DNS hosts to ASCII lowercase.
    Ok(if url.host_str().is_some_and(|host| host.ends_with(".atlassian.net")) {
        ProviderDeployment::Cloud
    } else {
        ProviderDeployment::DataCenter
    })
}

fn origin(from_file: bool, file_origin: &str, _secret: bool) -> String {
    if from_file {
        file_origin.into()
    } else {
        "default".into()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use cockpit_protocol::projects::OrchestrationConfiguration;

    use super::{
        default_omp_executable, load_browser_configuration, load_project_configuration,
        load_library_sync_configuration, load_quota_configuration, load_window_configuration,
        validate_orchestration, validate_orchestration_args, validate_template,
        LibrarySyncConfiguration,
    };

    fn library_sync_fixture() -> PathBuf {
        std::env::temp_dir().join(format!("cockpit-library-sync-{}.toml", uuid::Uuid::new_v4()))
    }

    #[test]
    fn library_sync_defaults_and_disabled_policy() {
        let path = library_sync_fixture();
        fs::write(&path, "version = 1\n").expect("write sync configuration");
        let expected = LibrarySyncConfiguration {
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
        };
        assert_eq!(LibrarySyncConfiguration::default(), expected);
        assert_eq!(load_library_sync_configuration(Some(&path)).expect("sync defaults"), expected);
        fs::write(&path, "version = 1\n[library_sync]\nenabled = false\n")
            .expect("write disabled configuration");
        assert_eq!(
            load_library_sync_configuration(Some(&path)).expect("disabled policy"),
            LibrarySyncConfiguration { enabled: false, ..expected },
        );
        fs::remove_file(path).expect("remove sync configuration");
    }

    #[test]
    fn library_sync_overrides_preserve_configured_item_limits() {
        let path = library_sync_fixture();
        fs::write(&path, concat!(
            "version = 1\n",
            "[limits]\nlibrary_space_pages = 1000\nlibrary_max_items = 3000\n",
            "[library_sync]\nenabled = true\ndelta_minutes = 1\n",
            "lag_allowance_minutes = 0\noverlap_minutes = 30\n",
            "inventory_hours = 48\naudit_days = 14\nrelated_hours = 72\n",
            "background_min_interval_seconds = 5\nbackground_in_flight = 4\n",
            "hourly_request_cap = 600\n",
        )).expect("write overridden configuration");
        assert_eq!(
            load_library_sync_configuration(Some(&path)).expect("sync overrides"),
            LibrarySyncConfiguration {
                enabled: true,
                delta_minutes: 1,
                lag_allowance_minutes: 0,
                overlap_minutes: 30,
                inventory_hours: 48,
                audit_days: 14,
                related_hours: 72,
                background_min_interval_seconds: 5,
                background_in_flight: 4,
                hourly_request_cap: 600,
            },
        );
        let projects = load_project_configuration(Some(&path), None).expect("project configuration");
        assert_eq!(projects.limits.library_space_pages, 1000);
        assert_eq!(projects.limits.library_max_items, 3000);
        fs::remove_file(path).expect("remove sync configuration");
    }

    #[test]
    fn library_sync_cadence_and_budget_boundaries() {
        let path = library_sync_fixture();
        for (field, minimum, maximum) in [
            ("delta_minutes", 1_u32, 10_080),
            ("lag_allowance_minutes", 0, 1440),
            ("overlap_minutes", 0, 10_080),
            ("inventory_hours", 1, 8760),
            ("audit_days", 1, 365),
            ("related_hours", 1, 8760),
            ("background_min_interval_seconds", 1, 3600),
            ("background_in_flight", 1, 32),
            ("hourly_request_cap", 1, 100_000),
        ] {
            for value in [minimum, maximum] {
                fs::write(&path, format!("version = 1\n[library_sync]\n{field} = {value}\n"))
                    .expect("write boundary configuration");
                load_library_sync_configuration(Some(&path))
                    .unwrap_or_else(|error| panic!("valid {field}={value}: {error}"));
            }
            for value in [minimum.checked_sub(1), Some(maximum + 1)].into_iter().flatten() {
                fs::write(&path, format!("version = 1\n[library_sync]\n{field} = {value}\n"))
                    .expect("write invalid boundary");
                let error = load_library_sync_configuration(Some(&path)).expect_err("out of range");
                assert_eq!(error.code, "invalid_library_sync_configuration");
                assert!(error.message.contains(field), "boundary diagnostic names {field}");
            }
        }
        fs::remove_file(path).expect("remove sync configuration");
    }

    #[test]
    fn library_sync_rejects_unknown_mistyped_and_oversized_configuration() {
        let path = library_sync_fixture();
        for policy in [
            "delta_minute = 60",
            "enabled = 'false'",
            "delta_minutes = -1",
            "delta_minutes = 4294967296",
            "delta_minutes = 1.5",
            "hourly_request_cap = '1200'",
        ] {
            fs::write(&path, format!("version = 1\n[library_sync]\n{policy}\n"))
                .expect("write malformed configuration");
            assert_eq!(
                load_library_sync_configuration(Some(&path)).expect_err("malformed sync policy").code,
                "invalid_config",
                "{policy}",
            );
        }
        fs::write(&path, "version = 2\n[library_sync]\nenabled = false\n")
            .expect("write unsupported version");
        assert_eq!(
            load_library_sync_configuration(Some(&path)).expect_err("unsupported version").code,
            "unsupported_config_version",
        );
        fs::write(&path, vec![b' '; super::MAX_CONFIG_BYTES + 1])
            .expect("write oversized configuration");
        assert_eq!(
            load_library_sync_configuration(Some(&path)).expect_err("bounded file").code,
            "config_too_large",
        );
        fs::remove_file(path).expect("remove sync configuration");
    }

    fn omp_fixture_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("cockpit-omp-config-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create OMP fixture");
        root
    }

    fn write_omp_executable(directory: &Path) -> PathBuf {
        fs::create_dir_all(directory).expect("create OMP install directory");
        let executable = directory.join(format!("omp{}", std::env::consts::EXE_SUFFIX));
        fs::write(&executable, "").expect("write OMP executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
                .expect("make OMP executable");
        }
        executable
    }

    #[test]
    fn omp_default_discovery_preserves_path_precedence() {
        let root = omp_fixture_root();
        let first = root.join("first");
        let second = root.join("second");
        let home = root.join("home");
        let system = root.join("system");
        let expected = write_omp_executable(&first);
        write_omp_executable(&second);
        write_omp_executable(&home.join(".local/bin"));
        write_omp_executable(&system);
        let path = std::env::join_paths([&first, &second]).expect("fixture PATH");
        assert_eq!(
            default_omp_executable(Some(&path), Some(&home), &[&system]),
            expected,
        );
        let path = std::env::join_paths([&second, &first]).expect("reversed fixture PATH");
        assert_eq!(
            default_omp_executable(Some(&path), Some(&home), &[&system]),
            second.join(format!("omp{}", std::env::consts::EXE_SUFFIX)),
        );
        fs::remove_dir_all(root).expect("remove OMP fixture");
    }

    #[test]
    fn omp_default_discovery_skips_missing_and_invalid_candidates() {
        let root = omp_fixture_root();
        let missing = root.join("missing");
        let directory = root.join("directory");
        fs::create_dir_all(directory.join(format!("omp{}", std::env::consts::EXE_SUFFIX)))
            .expect("create directory named OMP");
        let valid = root.join("valid");
        let expected = write_omp_executable(&valid);
        let mut directories = vec![missing, directory];
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let directory = root.join("non-executable");
            let executable = write_omp_executable(&directory);
            fs::set_permissions(executable, fs::Permissions::from_mode(0o644))
                .expect("remove execute permission");
            directories.push(directory);
        }
        directories.push(valid);
        let path = std::env::join_paths(&directories).expect("fixture PATH");
        assert_eq!(default_omp_executable(Some(&path), None, &[]), expected);
        fs::remove_dir_all(root).expect("remove OMP fixture");
    }

    #[test]
    fn omp_default_discovery_uses_local_then_bun_before_system() {
        let root = omp_fixture_root();
        let home = root.join("home");
        let system = root.join("system");
        let local = write_omp_executable(&home.join(".local/bin"));
        let bun = write_omp_executable(&home.join(".bun/bin"));
        let system_executable = write_omp_executable(&system);
        assert_eq!(
            default_omp_executable(None, Some(&home), &[&system]),
            local,
        );
        fs::remove_file(local).expect("remove local OMP executable");
        assert_eq!(
            default_omp_executable(Some(std::ffi::OsStr::new("")), Some(&home), &[&system]),
            bun,
        );
        fs::remove_file(bun).expect("remove bun OMP executable");
        assert_eq!(
            default_omp_executable(None, Some(&home), &[&system]),
            system_executable,
        );
        fs::remove_dir_all(root).expect("remove OMP fixture");
    }

    #[test]
    fn omp_default_discovery_uses_homebrew_prefixes_in_order() {
        let root = omp_fixture_root();
        let apple_silicon = root.join("opt/homebrew/bin");
        let intel = root.join("usr/local/bin");
        let linuxbrew = root.join("home/linuxbrew/.linuxbrew/bin");
        let directories = [apple_silicon.as_path(), intel.as_path(), linuxbrew.as_path()];
        let apple_executable = write_omp_executable(&apple_silicon);
        let intel_executable = write_omp_executable(&intel);
        let linux_executable = write_omp_executable(&linuxbrew);
        for expected in [apple_executable, intel_executable, linux_executable] {
            assert_eq!(default_omp_executable(None, None, &directories), expected);
            fs::remove_file(expected).expect("remove Homebrew OMP executable");
        }
        assert_eq!(
            default_omp_executable(None, None, &directories),
            PathBuf::from("omp"),
        );
        fs::remove_dir_all(root).expect("remove OMP fixture");
    }

    #[test]
    fn omp_default_discovery_without_candidates_keeps_bare_name() {
        assert_eq!(default_omp_executable(None, None, &[]), PathBuf::from("omp"));
        assert_eq!(
            default_omp_executable(
                Some(std::ffi::OsStr::new("")),
                Some(Path::new("")),
                &[],
            ),
            PathBuf::from("omp"),
        );
    }

    #[cfg(unix)]
    #[test]
    fn omp_default_discovery_accepts_symlink_installs() {
        let root = omp_fixture_root();
        let target = write_omp_executable(&root.join("target"));
        let directory = root.join("bin");
        fs::create_dir_all(&directory).expect("create symlink install directory");
        let executable = directory.join(format!("omp{}", std::env::consts::EXE_SUFFIX));
        std::os::unix::fs::symlink(target, &executable).expect("link OMP executable");
        let path = std::env::join_paths([&directory]).expect("fixture PATH");
        assert_eq!(default_omp_executable(Some(&path), None, &[]), executable);
        fs::remove_dir_all(root).expect("remove OMP fixture");
    }

    #[test]
    fn omp_configuration_discovers_only_default_and_preserves_explicit_overrides() {
        // Exercise the actual loader with isolated child environments, following
        // the browser/Notes configuration tests without global env mutations.
        if let Some(path) = std::env::var_os("COCKPIT_TEST_OMP_CONFIG") {
            let expected = std::env::var_os("COCKPIT_TEST_OMP_EXPECTED").expect("expected OMP path");
            assert_eq!(
                load_quota_configuration(Some(Path::new(&path)))
                    .expect("quota configuration")
                    .omp_executable,
                PathBuf::from(expected),
            );
            return;
        }

        let root = omp_fixture_root();
        let directory = root.join("bin");
        let discovered = write_omp_executable(&directory);
        let home = root.join("home");
        write_omp_executable(&home.join(".local/bin"));
        let path = root.join("config.toml");
        let toml_explicit = root.join("custom-toml-omp").to_string_lossy().into_owned();
        let environment_explicit = root.join("custom-env-omp").to_string_lossy().into_owned();
        let cases = [
            (None, None, discovered.to_string_lossy().into_owned()),
            (Some("omp"), None, "omp".to_owned()),
            (Some(toml_explicit.as_str()), None, toml_explicit.clone()),
            (None, Some("omp"), "omp".to_owned()),
            (None, Some(environment_explicit.as_str()), environment_explicit.clone()),
            (
                Some(toml_explicit.as_str()),
                Some("omp"),
                "omp".to_owned(),
            ),
            (
                Some("omp"),
                Some(environment_explicit.as_str()),
                environment_explicit.clone(),
            ),
        ];
        for (toml_executable, environment_executable, expected) in cases {
            let content = match toml_executable {
                Some(executable) => format!("version = 1\n[quota]\nomp_executable = {executable:?}\n"),
                None => "version = 1\n".into(),
            };
            fs::write(&path, content).expect("write quota configuration");
            let mut child = std::process::Command::new(
                std::env::current_exe().expect("current test executable"),
            );
            child.args([
                "--exact",
                "config::tests::omp_configuration_discovers_only_default_and_preserves_explicit_overrides",
                "--nocapture",
            ]);
            child.env_remove("COCKPIT_OMP_EXECUTABLE");
            child.env("PATH", std::env::join_paths([&directory]).expect("fixture PATH"));
            child.env("HOME", &home);
            child.env("COCKPIT_TEST_OMP_CONFIG", &path);
            child.env("COCKPIT_TEST_OMP_EXPECTED", expected);
            if let Some(value) = environment_executable {
                child.env("COCKPIT_OMP_EXECUTABLE", value);
            }
            let output = child.output().expect("run isolated quota configuration case");
            assert!(
                output.status.success(),
                "quota case TOML={toml_executable:?}, environment={environment_executable:?}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        fs::remove_dir_all(root).expect("remove OMP fixture");
    }

    #[test]
    fn template_accepts_only_documented_variables() {
        assert!(validate_template("{repo}/{task_id}-{slug}", true, "checkout_template").is_ok());
        assert!(validate_template("{repo}/{unknown}", true, "checkout_template").is_err());
        assert!(validate_template("../{repo}", true, "checkout_template").is_err());
        assert!(validate_template("/tmp/{repo}", true, "checkout_template").is_err());
    }

    #[test]
    fn browser_default_url_loading_and_validation() {
        // Isolate environment overrides in subprocesses, without mutating the
        // shared test process environment while other configuration tests run.
        if let Some(path) = std::env::var_os("COCKPIT_TEST_BROWSER_DEFAULT_URL_PATH") {
            let expected = std::env::var("COCKPIT_TEST_BROWSER_DEFAULT_URL_EXPECTED")
                .expect("expected URL or error");
            let result = load_browser_configuration(Some(std::path::Path::new(&path)));
            if expected == "invalid_browser_url" {
                assert_eq!(result.expect_err("unsafe default URL").code, expected);
            } else {
                assert_eq!(result.expect("browser configuration").default_url, expected);
            }
            return;
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-browser-default-{nonce}.toml"));
        let cases = [
            (None, None, "about:blank"),
            (Some("https://example.test/start"), None, "https://example.test/start"),
            (Some("http://localhost:3000/"), None, "http://localhost:3000/"),
            (Some("about:blank"), None, "about:blank"),
            (
                Some("https://example.test/toml"),
                Some("https://example.test/environment"),
                "https://example.test/environment",
            ),
            // Validate only the effective value: a safe environment override
            // can replace an invalid value in the TOML.
            (Some("file:///tmp/unsafe"), Some("about:blank"), "about:blank"),
            (Some("file:///tmp/unsafe"), None, "invalid_browser_url"),
            (Some("javascript:alert(1)"), None, "invalid_browser_url"),
            (Some("/relative"), None, "invalid_browser_url"),
            (Some("https://user:password@example.test/"), None, "invalid_browser_url"),
            (Some("https://example.test/"), Some("file:///tmp/unsafe"), "invalid_browser_url"),
            (
                None,
                Some("https://user@example.test/"),
                "invalid_browser_url",
            ),
        ];
        for (toml_url, environment_url, expected) in cases {
            let content = match toml_url {
                Some(url) => format!("version = 1\n[browser]\ndefault_url = {url:?}\n"),
                None => "version = 1\n".into(),
            };
            fs::write(&path, content).expect("write browser configuration");
            let mut child = std::process::Command::new(
                std::env::current_exe().expect("current test executable"),
            );
            child.args([
                "--exact",
                "config::tests::browser_default_url_loading_and_validation",
                "--nocapture",
            ]);
            for name in [
                "COCKPIT_BROWSER_DEFAULT_URL",
                "COCKPIT_PLAYWRIGHT_CLI",
                "COCKPIT_CHROMIUM_EXECUTABLE",
                "COCKPIT_NODE_EXECUTABLE",
                "COCKPIT_BROWSER_HELPER",
                "COCKPIT_PLAYWRIGHT_CORE",
            ] {
                child.env_remove(name);
            }
            child.env("COCKPIT_TEST_BROWSER_DEFAULT_URL_PATH", &path);
            child.env("COCKPIT_TEST_BROWSER_DEFAULT_URL_EXPECTED", expected);
            if let Some(value) = environment_url {
                child.env("COCKPIT_BROWSER_DEFAULT_URL", value);
            }
            let output = child.output().expect("run isolated configuration case");
            assert!(
                output.status.success(),
                "default URL case TOML={toml_url:?}, environment={environment_url:?}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        fs::remove_file(path).expect("remove browser configuration");
    }

    #[test]
    fn window_settings_default_to_unity_and_decorated() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-window-default-{nonce}.toml"));
        fs::write(&path, "version = 1\n").expect("write window configuration");
        let settings = load_window_configuration(Some(&path)).expect("window defaults");
        let _ = fs::remove_file(path);
        assert_eq!(settings.scale_factor, 1.0);
        assert!(settings.decorations);
    }

    #[test]
    fn window_settings_load_from_toml() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-window-{nonce}.toml"));
        fs::write(
            &path,
            "version = 1\n[window]\nscale_factor = 2.0\ndecorations = false\n",
        )
        .expect("write window configuration");
        let settings = load_window_configuration(Some(&path)).expect("window configuration");
        let _ = fs::remove_file(path);
        assert_eq!(settings.scale_factor, 2.0);
        assert!(!settings.decorations);
    }

    #[test]
    fn window_scale_factor_is_finite_and_bounded() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        for (index, value) in ["0.1", "10.1", "nan", "inf"].into_iter().enumerate() {
            let path =
                std::env::temp_dir().join(format!("cockpit-window-invalid-{nonce}-{index}.toml"));
            fs::write(
                &path,
                format!("version = 1\n[window]\nscale_factor = {value}\n"),
            )
            .expect("write invalid window configuration");
            let error = load_window_configuration(Some(&path)).expect_err("invalid scale factor");
            let _ = fs::remove_file(path);
            assert_eq!(error.code, "invalid_window_scale_factor");
        }
    }

    #[test]
    fn window_settings_reject_unsupported_configuration_versions() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-window-version-{nonce}.toml"));
        fs::write(&path, "version = 2\n[window]\nscale_factor = 1.25\n")
            .expect("write unsupported configuration");
        let error = load_window_configuration(Some(&path)).expect_err("unsupported version");
        let _ = fs::remove_file(path);
        assert_eq!(error.code, "unsupported_config_version");
    }

    #[test]
    fn versioned_toml_rejects_unknown_keys() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-config-{nonce}.toml"));
        fs::write(&path, "version = 1\nunknown = true\n").expect("write config");
        let error =
            load_project_configuration(Some(&path), None).expect_err("unknown key must fail");
        let _ = fs::remove_file(path);
        assert_eq!(error.code, "invalid_config");
    }

    #[test]
    fn provider_login_is_optional_and_validated_when_present() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-provider-{nonce}.toml"));
        fs::write(&path, "version = 1\n[[providers]]\nid = 'tea'\nkind = 'gitea'\nbase_url = 'https://forge.example'\nexecutable = 'tea'\n").expect("provider without login");
        let configured = load_project_configuration(Some(&path), None).expect("optional login");
        assert_eq!(configured.providers[0].login, None);
        assert_eq!(
            configured
                .origins
                .get("providers.tea.login")
                .map(String::as_str),
            Some("default")
        );
        fs::write(&path, "version = 1\n[[providers]]\nid = 'tea'\nkind = 'gitea'\nbase_url = 'https://forge.example'\nexecutable = 'tea'\nlogin = 'fixture'\n").expect("configured provider");
        let configured = load_project_configuration(Some(&path), None).expect("login");
        assert_eq!(configured.providers[0].login.as_deref(), Some("fixture"));
        assert_eq!(
            configured
                .origins
                .get("providers.tea.login")
                .map(String::as_str),
            Some("toml")
        );
        fs::write(&path, "version = 1\n[[providers]]\nid = 'tea'\nkind = 'gitea'\nbase_url = 'https://forge.example'\nexecutable = 'tea'\nlogin = ''\n").expect("bad provider");
        assert_eq!(
            load_project_configuration(Some(&path), None)
                .expect_err("empty login")
                .code,
            "invalid_provider_login"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn provider_kind_and_transport_configuration_are_explicit() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-provider-transport-{nonce}.toml"));
        let cases = [
            ("", "https://jira.example", "executable = 'jira'\n", "invalid_config"),
            ("jira", "https://jira.example", "executable = 'jira'\n", "invalid_provider_executable"),
            ("confluence", "https://wiki.example", "login = 'default'\n", "invalid_provider_login"),
            ("gitlab", "https://gitlab.example", "", "invalid_provider_executable"),
            ("gitea", "https://forge.example", "executable = 'tea'\ndeployment = 'cloud'\n", "invalid_provider_deployment"),
            ("confluence", "https://team.atlassian.net", "", "invalid_provider_base_url"),
            ("confluence", "https://team.atlassian.net/wiki/", "", "invalid_provider_base_url"),
            ("confluence", "https://wiki.example/confluence", "deployment = 'cloud'\n", "invalid_provider_base_url"),
        ];
        for (kind, base, extra, code) in cases {
            let kind = if kind.is_empty() { String::new() } else { format!("kind = '{kind}'\n") };
            fs::write(&path, format!("version = 1\n[[providers]]\nid = 'test'\n{kind}base_url = '{base}'\n{extra}")).expect("write provider");
            let error = load_project_configuration(Some(&path), None).expect_err("invalid provider");
            assert_eq!(error.code, code);
            if kind.is_empty() {
                assert!(error.message.contains("kind"), "missing kind error must name the field");
            }
        }
        fs::remove_file(path).expect("remove provider config");
    }

    #[test]
    fn atlassian_deployment_resolution_and_origins_are_independent_of_credentials() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-provider-deployment-{nonce}.toml"));
        let cases = [
            ("jira", "https://TEAM.ATLASSIAN.NET", "", super::ProviderDeployment::Cloud, "default"),
            ("jira", "https://jira.example/jira", "", super::ProviderDeployment::DataCenter, "default"),
            ("jira", "https://team.atlassian.net", "deployment = 'data_center'\n", super::ProviderDeployment::DataCenter, "toml"),
            ("jira", "https://jira.example/jira", "deployment = 'cloud'\n", super::ProviderDeployment::Cloud, "toml"),
            ("confluence", "https://team.atlassian.net/wiki", "", super::ProviderDeployment::Cloud, "default"),
            ("confluence", "https://wiki.example/confluence", "", super::ProviderDeployment::DataCenter, "default"),
        ];
        for (kind, base, extra, deployment, origin) in cases {
            fs::write(&path, format!("version = 1\n[[providers]]\nid = 'custom'\nkind = '{kind}'\nbase_url = '{base}'\n{extra}")).expect("write provider");
            let configuration = load_project_configuration(Some(&path), None).expect("valid provider");
            assert_eq!(configuration.providers[0].deployment, Some(deployment));
            assert_eq!(configuration.providers[0].executable, None);
            assert_eq!(configuration.origins.get("providers.custom.kind").map(String::as_str), Some("toml"));
            assert_eq!(configuration.origins.get("providers.custom.deployment").map(String::as_str), Some(origin));
            assert!(!configuration.origins.contains_key("providers.custom.executable"));
            assert!(!configuration.origins.contains_key("providers.custom.login"));
        }
        fs::remove_file(path).expect("remove provider config");
    }

    #[test]
    fn library_root_rejects_lexical_overlap_with_configured_roots() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let base = std::env::temp_dir().join(format!("cockpit-library-root-{nonce}"));
        let path = std::env::temp_dir().join(format!("cockpit-library-root-{nonce}.toml"));
        let config = |library: &str, companion: &str| {
            format!(
                "version = 1\nworktree_root = '{}'\ncompanion_root = '{}'\nstate_root = '{}'\nlibrary_root = '{}'\n",
                base.join("worktrees").display(),
                companion,
                base.join("state").display(),
                library,
            )
        };
        let inside_state = base.join("state/nested");
        fs::write(&path, config(&inside_state.to_string_lossy(), &base.join("companions").to_string_lossy()))
            .expect("write overlapping state roots");
        assert_eq!(
            load_project_configuration(Some(&path), None).expect_err("state overlap").code,
            "invalid_library_root"
        );

        let library = base.join("library");
        let companion = library.join("companions");
        fs::write(&path, config(&library.to_string_lossy(), &companion.to_string_lossy()))
            .expect("write overlapping companion roots");
        assert_eq!(
            load_project_configuration(Some(&path), None).expect_err("companion overlap").code,
            "invalid_library_root"
        );
        let _ = fs::remove_file(path);
    }
    #[test]
    fn library_root_defaults_to_xdg_data_directory() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-library-default-{nonce}.toml"));
        fs::write(&path, "version = 1\n").expect("write configuration");
        let configuration = load_project_configuration(Some(&path), None).expect("configuration defaults");
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".local/share"));
        assert_eq!(
            configuration.library_root,
            data_home.join("cockpit/library").to_string_lossy()
        );
        assert_eq!(configuration.origins.get("library_root").map(String::as_str), Some("default"));
        let _ = fs::remove_file(path);
    }
    #[test]
    fn orchestration_options_reject_code_loading_and_auto_approval() {
        for args in [
            vec!["--extension".into(), "/tmp/unreviewed.ts".into()],
            vec!["--resume".into(), "previous-session".into()],
            vec!["--yolo".into()],
            vec!["--thinking".into(), "off --yolo".into()],
        ] {
            assert_eq!(validate_orchestration_args(&args).unwrap_err().code, "invalid_orchestration_args");
        }
        assert!(validate_orchestration_args(&["--no-extensions".into(), "--thinking".into(), "off".into()]).is_ok());
        let configuration: OrchestrationConfiguration = toml::from_str(
            "model = 'tiny'\nextra_args = ['--no-extensions', '--no-skills', '--no-rules']\n[[routes]]\nprovider = 'jira'\ninstance = 'https://issues.example'\nproject_id_prefix = 'APP-'\nrepository_id = 'api'\n"
        ).unwrap();
        assert!(validate_orchestration(&configuration).is_ok());
        let mut unsafe_route = configuration;
        unsafe_route.routes[0].instance = "https://user:secret@issues.example/path".into();
        assert_eq!(validate_orchestration(&unsafe_route).unwrap_err().code, "invalid_route_instance");
    }

    #[test]
    fn notes_root_loading_precedence_validation_and_no_directory_creation() {
        // Environment-dependent cases run in separate processes, so no test
        // mutates the shared process environment.
        if let Some(path) = std::env::var_os("COCKPIT_TEST_NOTES_CONFIG") {
            let expected = std::env::var("COCKPIT_TEST_NOTES_EXPECTED").expect("expected root");
            let result = load_project_configuration(Some(std::path::Path::new(&path)), None);
            if expected == "invalid_notes_root" {
                assert_eq!(result.expect_err("invalid Notes root").code, expected);
            } else {
                let configuration = result.expect("Notes configuration");
                assert_eq!(configuration.notes_root, expected);
                let expected_origin = std::env::var("COCKPIT_TEST_NOTES_ORIGIN").expect("origin");
                assert_eq!(
                    configuration.origins.get("notes_root").map(String::as_str),
                    Some(expected_origin.as_str()),
                );
                for root in configuration.repository_roots.iter().chain([
                    &configuration.notes_root,
                    &configuration.library_root,
                    &configuration.state_root,
                    &configuration.companion_root,
                    &configuration.worktree_root,
                    &configuration.cache_root,
                ]) {
                    assert!(!std::path::Path::new(root).exists(), "loader created {root}");
                }
            }
            return;
        }

        let id = uuid::Uuid::new_v4();
        let base = std::env::temp_dir().join(format!("cockpit-notes-config-{id}"));
        let path = std::env::temp_dir().join(format!("cockpit-notes-config-{id}.toml"));
        let toml_root = base.join("toml-notes").to_string_lossy().into_owned();
        let environment_root = base.join("environment-notes").to_string_lossy().into_owned();
        let default_root = base.join("data/cockpit/notes").to_string_lossy().into_owned();
        let home_root = base.join("home/.local/share/cockpit/notes").to_string_lossy().into_owned();
        let mut cases = vec![
            (None, None, default_root, "default", true),
            (None, None, home_root, "default", false),
            (Some(toml_root.clone()), None, toml_root.clone(), "toml", true),
            (Some(toml_root), Some(environment_root.clone()), environment_root.clone(), "environment", true),
            (Some("relative".into()), Some(environment_root.clone()), environment_root, "environment", true),
            (Some(base.join("library-notes").to_string_lossy().into_owned()), None,
                base.join("library-notes").to_string_lossy().into_owned(), "toml", true),
        ];
        for invalid in [
            "relative".to_owned(),
            base.join("notes/../escape").to_string_lossy().into_owned(),
            String::new(),
            format!("/{}", "n".repeat(super::MAX_TEXT_BYTES)),
        ] {
            cases.push((Some(invalid), None, "invalid_notes_root".into(), "", true));
        }
        for field in ["library", "state", "companions", "worktrees", "cache", "repositories"] {
            for notes in [base.join(field), base.join(field).join("child"), base.clone()] {
                cases.push((Some(notes.to_string_lossy().into_owned()), None,
                    "invalid_notes_root".into(), "", true));
            }
        }
        // An environment override still must be validated against every root.
        cases.push((None, Some(base.join("repositories").to_string_lossy().into_owned()),
            "invalid_notes_root".into(), "", true));
        for (toml_notes, environment_notes, expected, expected_origin, use_xdg) in cases {
            let mut content = format!(
                "version = 1\nrepository_roots = [{:?}]\nworktree_root = {:?}\ncompanion_root = {:?}\nstate_root = {:?}\ncache_root = {:?}\nlibrary_root = {:?}\n",
                base.join("repositories").to_string_lossy(),
                base.join("worktrees").to_string_lossy(),
                base.join("companions").to_string_lossy(),
                base.join("state").to_string_lossy(),
                base.join("cache").to_string_lossy(),
                base.join("library").to_string_lossy(),
            );
            if let Some(value) = &toml_notes {
                content.push_str(&format!("notes_root = {value:?}\n"));
            }
            fs::write(&path, content).expect("write Notes configuration");
            let mut child = std::process::Command::new(
                std::env::current_exe().expect("test executable"),
            );
            child.args([
                "--exact",
                "config::tests::notes_root_loading_precedence_validation_and_no_directory_creation",
                "--nocapture",
            ]);
            for name in [
                "COCKPIT_REPOSITORY_ROOTS",
                "COCKPIT_WORKTREE_ROOT",
                "COCKPIT_COMPANION_ROOT",
                "COCKPIT_STATE_ROOT",
                "COCKPIT_CACHE_ROOT",
                "COCKPIT_LIBRARY_ROOT",
                "COCKPIT_NOTES_ROOT",
                "XDG_DATA_HOME",
            ] {
                child.env_remove(name);
            }
            child.env("HOME", base.join("home"));
            child.env("XDG_STATE_HOME", base.join("xdg-state"));
            child.env("XDG_CACHE_HOME", base.join("xdg-cache"));
            if use_xdg {
                child.env("XDG_DATA_HOME", base.join("data"));
            }
            if let Some(value) = &environment_notes {
                child.env("COCKPIT_NOTES_ROOT", value);
            }
            child.env("COCKPIT_TEST_NOTES_CONFIG", &path);
            child.env("COCKPIT_TEST_NOTES_EXPECTED", expected);
            child.env("COCKPIT_TEST_NOTES_ORIGIN", expected_origin);
            let output = child.output().expect("run isolated Notes configuration case");
            assert!(
                output.status.success(),
                "Notes configuration TOML={toml_notes:?}, environment={environment_notes:?}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        fs::remove_file(path).expect("remove Notes test configuration");
        assert!(!base.exists(), "configuration loading must not create roots");
    }
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
