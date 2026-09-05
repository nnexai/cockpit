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
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlLimits {
    catalog_depth: Option<u32>,
    catalog_entries: Option<u32>,
    git_timeout_ms: Option<u32>,
    git_output_bytes: Option<u32>,
    operation_timeout_ms: Option<u32>,
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

    let version = file.version.unwrap_or(CONFIG_VERSION);
    if version != CONFIG_VERSION {
        return Err(InspectionError::new(
            "unsupported_config_version",
            format!("configuration version {version} is unsupported; expected {CONFIG_VERSION}"),
        ));
    }
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
    ];
    let limits = ProjectLimits {
        catalog_depth: bounded_limit(
            limits_file.catalog_depth.unwrap_or(3),
            1,
            32,
            "catalog_depth",
        )?,
        catalog_entries: bounded_limit(
            limits_file.catalog_entries.unwrap_or(1024),
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
    }

    Ok(ProjectConfiguration {
        version,
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
    let value = toml::from_str(&text).map_err(|error: toml::de::Error| {
        InspectionError::new(
            "invalid_config",
            format!("Invalid configuration TOML: {}", error.message()),
        )
    })?;
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

    use super::{load_project_configuration, validate_template};

    #[test]
    fn template_accepts_only_documented_variables() {
        assert!(validate_template("{repo}/{task_id}-{slug}", true, "checkout_template").is_ok());
        assert!(validate_template("{repo}/{unknown}", true, "checkout_template").is_err());
        assert!(validate_template("../{repo}", true, "checkout_template").is_err());
        assert!(validate_template("/tmp/{repo}", true, "checkout_template").is_err());
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
