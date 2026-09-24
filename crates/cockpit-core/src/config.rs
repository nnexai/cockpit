use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};
use serde::Deserialize;

use crate::InspectionError;

const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_TEXT_BYTES: usize = 4096;
const CONFIG_VERSION: u32 = 1;
const DEFAULT_WINDOW_SCALE_FACTOR: f64 = 1.0;
const DEFAULT_WINDOW_DECORATIONS: bool = true;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowConfiguration {
    pub scale_factor: f64,
    pub decorations: bool,
}

/// Paths for the installed browser tooling that Cockpit is allowed to invoke.
///
/// The CLI is resolved from the owner's environment. Its normal browser selection
/// is preserved unless an executable override is configured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserConfiguration {
    pub playwright_cli: PathBuf,
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
    branch_template: Option<String>,
    checkout_template: Option<String>,
    providers: Option<Vec<ProjectProvider>>,
    limits: Option<TomlLimits>,
    window: Option<TomlWindow>,
    browser: Option<TomlBrowser>,
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
    chromium_executable: Option<String>,
    node_executable: Option<String>,
    browser_helper: Option<String>,
    playwright_core: Option<String>,
    feedback_retention_seconds: Option<u64>,
    feedback_max_store_bytes: Option<u64>,
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
    let providers = file.providers.unwrap_or_default();
    if providers.len() > 128 {
        return Err(InspectionError::new(
            "invalid_providers",
            "providers cannot contain more than 128 entries",
        ));
    }
    for provider in &providers {
        validate_provider(provider)?;
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
            format!("providers.{}.executable", provider.id),
            provider_origin.into(),
        );
        origins.insert(
            format!("providers.{}.login", provider.id),
            if provider.login.is_some() {
                "toml"
            } else {
                "default"
            }
            .into(),
        );
    }

    Ok(ProjectConfiguration {
        version: file.version.unwrap_or(CONFIG_VERSION),
        repository_roots: roots,
        worktree_root,
        companion_root,
        state_root,
        branch_template,
        checkout_template,
        providers,
        limits,
        origins,
    })
}

/// Load browser executable paths from the shared Cockpit TOML.
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

fn load_file_configuration(
    config_path: Option<&Path>,
) -> Result<(TomlConfiguration, String), InspectionError> {
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
    validate_text(&provider.executable, "provider_executable")?;
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{load_project_configuration, load_window_configuration, validate_template};

    #[test]
    fn template_accepts_only_documented_variables() {
        assert!(validate_template("{repo}/{task_id}-{slug}", true, "checkout_template").is_ok());
        assert!(validate_template("{repo}/{unknown}", true, "checkout_template").is_err());
        assert!(validate_template("../{repo}", true, "checkout_template").is_err());
        assert!(validate_template("/tmp/{repo}", true, "checkout_template").is_err());
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
        fs::write(&path, "version = 1\n[[providers]]\nid = 'tea'\nbase_url = 'https://forge.example'\nexecutable = 'tea'\n").expect("legacy provider");
        let legacy = load_project_configuration(Some(&path), None).expect("optional login");
        assert_eq!(legacy.providers[0].login, None);
        assert_eq!(
            legacy
                .origins
                .get("providers.tea.login")
                .map(String::as_str),
            Some("default")
        );
        fs::write(&path, "version = 1\n[[providers]]\nid = 'tea'\nbase_url = 'https://forge.example'\nexecutable = 'tea'\nlogin = 'fixture'\n").expect("configured provider");
        let configured = load_project_configuration(Some(&path), None).expect("login");
        assert_eq!(configured.providers[0].login.as_deref(), Some("fixture"));
        assert_eq!(
            configured
                .origins
                .get("providers.tea.login")
                .map(String::as_str),
            Some("toml")
        );
        fs::write(&path, "version = 1\n[[providers]]\nid = 'tea'\nbase_url = 'https://forge.example'\nexecutable = 'tea'\nlogin = ''\n").expect("bad provider");
        assert_eq!(
            load_project_configuration(Some(&path), None)
                .expect_err("empty login")
                .code,
            "invalid_provider_login"
        );
        let _ = fs::remove_file(path);
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
