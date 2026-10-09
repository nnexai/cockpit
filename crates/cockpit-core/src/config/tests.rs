use cockpit_protocol::projects::OrchestrationConfiguration;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    ConfigurationFile, LibrarySyncConfiguration, default_omp_executable, validate_orchestration,
    validate_orchestration_args, validate_template,
};

fn library_sync_fixture() -> PathBuf {
    std::env::temp_dir().join(format!(
        "cockpit-library-sync-{}.toml",
        uuid::Uuid::new_v4()
    ))
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
    assert_eq!(
        ConfigurationFile::load(Some(&path))
            .and_then(|file| file.library_sync.resolve())
            .expect("sync defaults"),
        expected
    );
    fs::write(&path, "version = 1\n[library_sync]\nenabled = false\n")
        .expect("write disabled configuration");
    assert_eq!(
        ConfigurationFile::load(Some(&path))
            .and_then(|file| file.library_sync.resolve())
            .expect("disabled policy"),
        LibrarySyncConfiguration {
            enabled: false,
            ..expected
        },
    );
    fs::remove_file(path).expect("remove sync configuration");
}

#[test]
fn library_sync_overrides_preserve_configured_item_limits() {
    let path = library_sync_fixture();
    fs::write(
        &path,
        concat!(
            "version = 1\n",
            "[limits]\nlibrary_space_pages = 1000\nlibrary_max_items = 3000\n",
            "[library_sync]\nenabled = true\ndelta_minutes = 1\n",
            "lag_allowance_minutes = 0\noverlap_minutes = 30\n",
            "inventory_hours = 48\naudit_days = 14\nrelated_hours = 72\n",
            "background_min_interval_seconds = 5\nbackground_in_flight = 4\n",
            "hourly_request_cap = 600\n",
        ),
    )
    .expect("write overridden configuration");
    assert_eq!(
        ConfigurationFile::load(Some(&path))
            .and_then(|file| file.library_sync.resolve())
            .expect("sync overrides"),
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
    let projects = ConfigurationFile::load(Some(&path))
        .and_then(|file| file.project.resolve(None))
        .expect("project configuration");
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
            fs::write(
                &path,
                format!("version = 1\n[library_sync]\n{field} = {value}\n"),
            )
            .expect("write boundary configuration");
            ConfigurationFile::load(Some(&path))
                .and_then(|file| file.library_sync.resolve())
                .unwrap_or_else(|error| panic!("valid {field}={value}: {error}"));
        }
        for value in [minimum.checked_sub(1), Some(maximum + 1)]
            .into_iter()
            .flatten()
        {
            fs::write(
                &path,
                format!("version = 1\n[library_sync]\n{field} = {value}\n"),
            )
            .expect("write invalid boundary");
            let error = ConfigurationFile::load(Some(&path))
                .and_then(|file| file.library_sync.resolve())
                .expect_err("out of range");
            assert_eq!(error.code, "invalid_library_sync_configuration");
            assert!(
                error.message.contains(field),
                "boundary diagnostic names {field}"
            );
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
            ConfigurationFile::load(Some(&path))
                .and_then(|file| file.library_sync.resolve())
                .expect_err("malformed sync policy")
                .code,
            "invalid_config",
            "{policy}",
        );
    }
    fs::write(&path, "version = 2\n[library_sync]\nenabled = false\n")
        .expect("write unsupported version");
    assert_eq!(
        ConfigurationFile::load(Some(&path))
            .and_then(|file| file.library_sync.resolve())
            .expect_err("unsupported version")
            .code,
        "unsupported_config_version",
    );
    fs::write(&path, vec![b' '; super::MAX_CONFIG_BYTES + 1])
        .expect("write oversized configuration");
    assert_eq!(
        ConfigurationFile::load(Some(&path))
            .and_then(|file| file.library_sync.resolve())
            .expect_err("bounded file")
            .code,
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
    assert_eq!(default_omp_executable(None, Some(&home), &[&system]), local,);
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
    let directories = [
        apple_silicon.as_path(),
        intel.as_path(),
        linuxbrew.as_path(),
    ];
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
    assert_eq!(
        default_omp_executable(None, None, &[]),
        PathBuf::from("omp")
    );
    assert_eq!(
        default_omp_executable(Some(std::ffi::OsStr::new("")), Some(Path::new("")), &[],),
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
            ConfigurationFile::load(Some(Path::new(&path)))
                .and_then(|file| file.quota.resolve())
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
        (
            None,
            Some(environment_explicit.as_str()),
            environment_explicit.clone(),
        ),
        (Some(toml_explicit.as_str()), Some("omp"), "omp".to_owned()),
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
        let mut child =
            std::process::Command::new(std::env::current_exe().expect("current test executable"));
        child.args([
            "--exact",
            "config::tests::omp_configuration_discovers_only_default_and_preserves_explicit_overrides",
            "--nocapture",
        ]);
        child.env_remove("COCKPIT_OMP_EXECUTABLE");
        child.env(
            "PATH",
            std::env::join_paths([&directory]).expect("fixture PATH"),
        );
        child.env("HOME", &home);
        child.env("COCKPIT_TEST_OMP_CONFIG", &path);
        child.env("COCKPIT_TEST_OMP_EXPECTED", expected);
        if let Some(value) = environment_executable {
            child.env("COCKPIT_OMP_EXECUTABLE", value);
        }
        let output = child
            .output()
            .expect("run isolated quota configuration case");
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
        let result = ConfigurationFile::load(Some(std::path::Path::new(&path)))
            .and_then(|file| file.browser.resolve());
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
        (
            Some("https://example.test/start"),
            None,
            "https://example.test/start",
        ),
        (
            Some("http://localhost:3000/"),
            None,
            "http://localhost:3000/",
        ),
        (Some("about:blank"), None, "about:blank"),
        (
            Some("https://example.test/toml"),
            Some("https://example.test/environment"),
            "https://example.test/environment",
        ),
        // Validate only the effective value: a safe environment override
        // can replace an invalid value in the TOML.
        (
            Some("file:///tmp/unsafe"),
            Some("about:blank"),
            "about:blank",
        ),
        (Some("file:///tmp/unsafe"), None, "invalid_browser_url"),
        (Some("javascript:alert(1)"), None, "invalid_browser_url"),
        (Some("/relative"), None, "invalid_browser_url"),
        (
            Some("https://user:password@example.test/"),
            None,
            "invalid_browser_url",
        ),
        (
            Some("https://example.test/"),
            Some("file:///tmp/unsafe"),
            "invalid_browser_url",
        ),
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
        let mut child =
            std::process::Command::new(std::env::current_exe().expect("current test executable"));
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
    let settings = ConfigurationFile::load(Some(&path))
        .and_then(|file| file.window.resolve())
        .expect("window defaults");
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
    let settings = ConfigurationFile::load(Some(&path))
        .and_then(|file| file.window.resolve())
        .expect("window configuration");
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
        let error = ConfigurationFile::load(Some(&path))
            .and_then(|file| file.window.resolve())
            .expect_err("invalid scale factor");
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
    let error = ConfigurationFile::load(Some(&path))
        .and_then(|file| file.window.resolve())
        .expect_err("unsupported version");
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
    let error = ConfigurationFile::load(Some(&path))
        .and_then(|file| file.project.resolve(None))
        .expect_err("unknown key must fail");
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
    let configured = ConfigurationFile::load(Some(&path))
        .and_then(|file| file.project.resolve(None))
        .expect("optional login");
    assert_eq!(configured.providers[0].login, None);
    fs::write(&path, "version = 1\n[[providers]]\nid = 'tea'\nkind = 'gitea'\nbase_url = 'https://forge.example'\nexecutable = 'tea'\nlogin = 'fixture'\n").expect("configured provider");
    let configured = ConfigurationFile::load(Some(&path))
        .and_then(|file| file.project.resolve(None))
        .expect("login");
    assert_eq!(configured.providers[0].login.as_deref(), Some("fixture"));
    fs::write(&path, "version = 1\n[[providers]]\nid = 'tea'\nkind = 'gitea'\nbase_url = 'https://forge.example'\nexecutable = 'tea'\nlogin = ''\n").expect("bad provider");
    assert_eq!(
        ConfigurationFile::load(Some(&path))
            .and_then(|file| file.project.resolve(None))
            .expect_err("empty login")
            .code,
        "invalid_provider_login"
    );
    let _ = fs::remove_file(path);
}

#[test]
fn provider_kind_and_transport_configuration_are_explicit() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("cockpit-provider-transport-{nonce}.toml"));
    let cases = [
        (
            "",
            "https://jira.example",
            "executable = 'jira'\n",
            "invalid_config",
        ),
        (
            "gitlab",
            "https://gitlab.example",
            "",
            "invalid_provider_executable",
        ),
        (
            "gitea",
            "https://forge.example",
            "executable = 'tea'\ndeployment = 'cloud'\n",
            "invalid_provider_deployment",
        ),
        (
            "confluence",
            "https://team.atlassian.net",
            "",
            "invalid_provider_base_url",
        ),
        (
            "confluence",
            "https://team.atlassian.net/wiki/",
            "",
            "invalid_provider_base_url",
        ),
        (
            "confluence",
            "https://wiki.example/confluence",
            "deployment = 'cloud'\n",
            "invalid_provider_base_url",
        ),
    ];
    for (kind, base, extra, code) in cases {
        let kind = if kind.is_empty() {
            String::new()
        } else {
            format!("kind = '{kind}'\n")
        };
        fs::write(
            &path,
            format!("version = 1\n[[providers]]\nid = 'test'\n{kind}base_url = '{base}'\n{extra}"),
        )
        .expect("write provider");
        let error = ConfigurationFile::load(Some(&path))
            .and_then(|file| file.project.resolve(None))
            .expect_err("invalid provider");
        assert_eq!(error.code, code);
        if kind.is_empty() {
            assert!(
                error.message.contains("kind"),
                "missing kind error must name the field"
            );
        }
    }
    fs::remove_file(path).expect("remove provider config");
}

#[test]
fn atlassian_deployment_resolution_is_independent_of_credentials() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("cockpit-provider-deployment-{nonce}.toml"));
    let cases = [
        (
            "jira",
            "https://TEAM.ATLASSIAN.NET",
            "",
            super::ProviderDeployment::Cloud,
        ),
        (
            "jira",
            "https://jira.example/jira",
            "",
            super::ProviderDeployment::DataCenter,
        ),
        (
            "jira",
            "https://team.atlassian.net",
            "deployment = 'data_center'\n",
            super::ProviderDeployment::DataCenter,
        ),
        (
            "jira",
            "https://jira.example/jira",
            "deployment = 'cloud'\n",
            super::ProviderDeployment::Cloud,
        ),
        (
            "confluence",
            "https://team.atlassian.net/wiki",
            "",
            super::ProviderDeployment::Cloud,
        ),
        (
            "confluence",
            "https://wiki.example/confluence",
            "",
            super::ProviderDeployment::DataCenter,
        ),
    ];
    for (kind, base, extra, deployment) in cases {
        fs::write(&path, format!("version = 1\n[[providers]]\nid = 'custom'\nkind = '{kind}'\nbase_url = '{base}'\n{extra}")).expect("write provider");
        let configuration = ConfigurationFile::load(Some(&path))
            .and_then(|file| file.project.resolve(None))
            .expect("valid provider");
        assert_eq!(configuration.providers[0].deployment, Some(deployment));
        assert_eq!(configuration.providers[0].executable, None);
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
    let config = |library: &str| {
        format!(
            "version = 1\nworktree_root = '{}'\nstate_root = '{}'\nlibrary_root = '{}'\n",
            base.join("worktrees").display(),
            base.join("state").display(),
            library,
        )
    };
    let inside_state = base.join("state/nested");
    fs::write(&path, config(&inside_state.to_string_lossy()))
        .expect("write overlapping state roots");
    assert_eq!(
        ConfigurationFile::load(Some(&path))
            .and_then(|file| file.project.resolve(None))
            .expect_err("state overlap")
            .code,
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
    let configuration = ConfigurationFile::load(Some(&path))
        .and_then(|file| file.project.resolve(None))
        .expect("configuration defaults");
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".local/share")
        });
    assert_eq!(
        configuration.library_root,
        data_home.join("cockpit/library").to_string_lossy()
    );
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
        assert_eq!(
            validate_orchestration_args(&args).unwrap_err().code,
            "invalid_orchestration_args"
        );
    }
    assert!(
        validate_orchestration_args(&["--no-extensions".into(), "--thinking".into(), "off".into()])
            .is_ok()
    );
    let configuration: OrchestrationConfiguration = toml::from_str(
        "model = 'tiny'\nextra_args = ['--no-extensions', '--no-skills', '--no-rules']\n[[routes]]\nprovider = 'jira'\ninstance = 'https://issues.example'\nproject_id_prefix = 'APP-'\nrepository_id = 'api'\n"
    ).unwrap();
    assert!(validate_orchestration(&configuration).is_ok());
    let mut unsafe_route = configuration;
    unsafe_route.routes[0].instance = "https://user:secret@issues.example/path".into();
    assert_eq!(
        validate_orchestration(&unsafe_route).unwrap_err().code,
        "invalid_route_instance"
    );
}

#[test]
fn notes_root_loading_precedence_validation_and_no_directory_creation() {
    // Environment-dependent cases run in separate processes, so no test
    // mutates the shared process environment.
    if let Some(path) = std::env::var_os("COCKPIT_TEST_NOTES_CONFIG") {
        let expected = std::env::var("COCKPIT_TEST_NOTES_EXPECTED").expect("expected root");
        let result = ConfigurationFile::load(Some(std::path::Path::new(&path)))
            .and_then(|file| file.project.resolve(None));
        if expected == "invalid_notes_root" {
            assert_eq!(result.expect_err("invalid Notes root").code, expected);
        } else {
            let configuration = result.expect("Notes configuration");
            assert_eq!(configuration.notes_root, expected);
            for root in configuration.repository_roots.iter().chain([
                &configuration.notes_root,
                &configuration.library_root,
                &configuration.state_root,
                &configuration.worktree_root,
                &configuration.cache_root,
            ]) {
                assert!(
                    !std::path::Path::new(root).exists(),
                    "loader created {root}"
                );
            }
        }
        return;
    }

    let id = uuid::Uuid::new_v4();
    let base = std::env::temp_dir().join(format!("cockpit-notes-config-{id}"));
    let path = std::env::temp_dir().join(format!("cockpit-notes-config-{id}.toml"));
    let toml_root = base.join("toml-notes").to_string_lossy().into_owned();
    let environment_root = base
        .join("environment-notes")
        .to_string_lossy()
        .into_owned();
    let default_root = base
        .join("data/cockpit/notes")
        .to_string_lossy()
        .into_owned();
    let home_root = base
        .join("home/.local/share/cockpit/notes")
        .to_string_lossy()
        .into_owned();
    let mut cases = vec![
        (None, None, default_root, true),
        (None, None, home_root, false),
        (Some(toml_root.clone()), None, toml_root.clone(), true),
        (
            Some(toml_root),
            Some(environment_root.clone()),
            environment_root.clone(),
            true,
        ),
        (
            Some("relative".into()),
            Some(environment_root.clone()),
            environment_root,
            true,
        ),
        (
            Some(base.join("library-notes").to_string_lossy().into_owned()),
            None,
            base.join("library-notes").to_string_lossy().into_owned(),
            true,
        ),
    ];
    for invalid in [
        "relative".to_owned(),
        base.join("notes/../escape").to_string_lossy().into_owned(),
        String::new(),
        format!("/{}", "n".repeat(super::MAX_TEXT_BYTES)),
    ] {
        cases.push((Some(invalid), None, "invalid_notes_root".into(), true));
    }
    for field in ["library", "state", "worktrees", "cache", "repositories"] {
        for notes in [
            base.join(field),
            base.join(field).join("child"),
            base.clone(),
        ] {
            cases.push((
                Some(notes.to_string_lossy().into_owned()),
                None,
                "invalid_notes_root".into(),
                true,
            ));
        }
    }
    // An environment override still must be validated against every root.
    cases.push((
        None,
        Some(base.join("repositories").to_string_lossy().into_owned()),
        "invalid_notes_root".into(),
        true,
    ));
    for (toml_notes, environment_notes, expected, use_xdg) in cases {
        let mut content = format!(
            "version = 1\nrepository_roots = [{:?}]\nworktree_root = {:?}\nstate_root = {:?}\ncache_root = {:?}\nlibrary_root = {:?}\n",
            base.join("repositories").to_string_lossy(),
            base.join("worktrees").to_string_lossy(),
            base.join("state").to_string_lossy(),
            base.join("cache").to_string_lossy(),
            base.join("library").to_string_lossy(),
        );
        if let Some(value) = &toml_notes {
            content.push_str(&format!("notes_root = {value:?}\n"));
        }
        fs::write(&path, content).expect("write Notes configuration");
        let mut child =
            std::process::Command::new(std::env::current_exe().expect("test executable"));
        child.args([
            "--exact",
            "config::tests::notes_root_loading_precedence_validation_and_no_directory_creation",
            "--nocapture",
        ]);
        for name in [
            "COCKPIT_REPOSITORY_ROOTS",
            "COCKPIT_WORKTREE_ROOT",
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
        let output = child
            .output()
            .expect("run isolated Notes configuration case");
        assert!(
            output.status.success(),
            "Notes configuration TOML={toml_notes:?}, environment={environment_notes:?}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    fs::remove_file(path).expect("remove Notes test configuration");
    assert!(
        !base.exists(),
        "configuration loading must not create roots"
    );
}

#[test]
fn configuration_file_sections_resolve_from_one_parsed_snapshot() {
    let path = library_sync_fixture();
    fs::write(
        &path,
        concat!(
            "version = 1\n",
            "[window]\nscale_factor = 1.75\ndecorations = false\n",
            "[library_sync]\nenabled = false\ndelta_minutes = 13\n",
            "lag_allowance_minutes = 2\noverlap_minutes = 17\n",
            "inventory_hours = 31\naudit_days = 11\nrelated_hours = 43\n",
            "background_min_interval_seconds = 7\nbackground_in_flight = 3\n",
            "hourly_request_cap = 457\n",
            "[limits]\ncatalog_depth = 9\nlibrary_folder_bytes = 2147483648\n",
        ),
    )
    .expect("write snapshot configuration");
    let file = ConfigurationFile::load(Some(&path)).expect("parse shared configuration once");
    fs::write(
        &path,
        "version = 1\n[window]\nscale_factor = 3.0\ndecorations = true\n",
    )
    .expect("replace backing configuration");
    let window = file
        .window
        .resolve()
        .expect("resolve original window section");
    assert_eq!(window.scale_factor, 1.75);
    assert!(!window.decorations);
    fs::remove_file(&path).expect("remove backing configuration");
    let policy = file
        .library_sync
        .resolve()
        .expect("resolve original sync section");
    assert_eq!(
        policy,
        LibrarySyncConfiguration {
            enabled: false,
            delta_minutes: 13,
            lag_allowance_minutes: 2,
            overlap_minutes: 17,
            inventory_hours: 31,
            audit_days: 11,
            related_hours: 43,
            background_min_interval_seconds: 7,
            background_in_flight: 3,
            hourly_request_cap: 457,
        },
    );
    let project = file
        .project
        .resolve(None)
        .expect("resolve original project limits");
    assert_eq!(project.limits.catalog_depth, 9);
    assert_eq!(project.limits.library_folder_bytes, 2_147_483_648_u64);
}

#[test]
fn project_limits_defaults_and_all_sixteen_bounds_are_independent_contracts() {
    let path = library_sync_fixture();
    fs::write(&path, "version = 1\n").expect("write default limits configuration");
    let defaults = ConfigurationFile::load(Some(&path))
        .and_then(|file| file.project.resolve(None))
        .expect("default project limits")
        .limits;

    // These consumer contract literals are deliberately independent of the
    // production field table, including each field's public integer type.
    macro_rules! check_limits {
        ($kind:ty; $(($field:ident, $default:expr, $minimum:expr, $maximum:expr)),+ $(,)?) => {
            $(
                let observed: $kind = defaults.$field;
                let expected: $kind = $default;
                assert_eq!(observed, expected, "{} default", stringify!($field));
                let minimum: $kind = $minimum;
                let maximum: $kind = $maximum;
                for value in [minimum, maximum] {
                    fs::write(
                        &path,
                        format!("version = 1\n[limits]\n{} = {value}\n", stringify!($field)),
                    )
                    .expect("write accepted limit boundary");
                    let configuration = ConfigurationFile::load(Some(&path))
                        .and_then(|file| file.project.resolve(None))
                        .unwrap_or_else(|error| {
                            panic!("valid {}={value}: {error}", stringify!($field))
                        });
                    let observed: $kind = configuration.limits.$field;
                    assert_eq!(observed, value, "{} boundary", stringify!($field));
                }
                for value in [minimum - 1, maximum + 1] {
                    fs::write(
                        &path,
                        format!("version = 1\n[limits]\n{} = {value}\n", stringify!($field)),
                    )
                    .expect("write rejected limit boundary");
                    let error = ConfigurationFile::load(Some(&path))
                        .and_then(|file| file.project.resolve(None))
                        .expect_err("out-of-range project limit");
                    assert_eq!(
                        error.code,
                        concat!("invalid_", stringify!($field)),
                        "{}={value}",
                        stringify!($field),
                    );
                }
            )+
        };
    }

    check_limits!(u32;
        (catalog_depth, 3, 1, 32),
        (catalog_entries, 16_384, 1, 100_000),
        (git_timeout_ms, 3_000, 1, 120_000),
        (git_output_bytes, 1_048_576, 1_024, 16_777_216),
        (operation_timeout_ms, 30_000, 1, 600_000),
        (context_preview_bytes, 1_048_576, 1_024, 8_388_608),
        (context_preview_lines, 5_000, 1, 20_000),
        (context_directory_entries, 1_000, 1, 10_000),
        (context_tree_depth, 32, 1, 64),
        (library_folder_files, 512, 1, 100_000),
        (library_space_pages, 200, 1, 20_000),
        (library_max_items, 20_000, 100, 1_000_000),
    );
    check_limits!(u64;
        (library_folder_bytes, 33_554_432, 1_048_576, 4_294_967_295),
        (library_file_bytes, 4_194_304, 1_024, 1_073_741_824),
        (library_attachment_bytes, 26_214_400, 1_024, 1_073_741_824),
        (library_item_attachment_bytes, 104_857_600, 1_048_576, 4_294_967_295),
    );
    fs::remove_file(path).expect("remove limits configuration");
}
