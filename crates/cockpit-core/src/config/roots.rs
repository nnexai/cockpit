use std::env;
use std::path::{Path, PathBuf};

use crate::InspectionError;

use super::{configured, path_text, validate_paths, xdg_directory};

pub(super) struct RawRoots {
    pub(super) repository_roots: Option<Vec<String>>,
    pub(super) cache_root: Option<String>,
    pub(super) worktree_root: Option<String>,
    pub(super) state_root: Option<String>,
    pub(super) library_root: Option<String>,
    pub(super) notes_root: Option<String>,
}

pub(super) struct Roots {
    pub(super) repository: Vec<String>,
    pub(super) cache: String,
    pub(super) worktree: String,
    pub(super) state: String,
    pub(super) library: String,
    pub(super) notes: String,
}

#[derive(Clone, Copy)]
enum Root {
    Cache,
    Worktree,
    State,
    Library,
    Notes,
}

impl RawRoots {
    fn take(&mut self, root: Root) -> Option<String> {
        match root {
            Root::Cache => self.cache_root.take(),
            Root::Worktree => self.worktree_root.take(),
            Root::State => self.state_root.take(),
            Root::Library => self.library_root.take(),
            Root::Notes => self.notes_root.take(),
        }
    }
}

struct RootSpec {
    field: &'static str,
    env: &'static str,
    default: &'static str,
    overlaps: &'static [Root],
    overlaps_repositories: bool,
    field_coded_env_errors: bool,
}

const ROOTS: [RootSpec; 5] = [
    RootSpec {
        field: "cache_root",
        env: "COCKPIT_CACHE_ROOT",
        default: "cockpit",
        overlaps: &[],
        overlaps_repositories: false,
        field_coded_env_errors: false,
    },
    RootSpec {
        field: "worktree_root",
        env: "COCKPIT_WORKTREE_ROOT",
        default: "cockpit/worktrees",
        overlaps: &[],
        overlaps_repositories: false,
        field_coded_env_errors: false,
    },
    RootSpec {
        field: "state_root",
        env: "COCKPIT_STATE_ROOT",
        default: "cockpit/operations",
        overlaps: &[],
        overlaps_repositories: false,
        field_coded_env_errors: false,
    },
    RootSpec {
        field: "library_root",
        env: "COCKPIT_LIBRARY_ROOT",
        default: "cockpit/library",
        overlaps: &[Root::State, Root::Worktree, Root::Cache],
        overlaps_repositories: false,
        field_coded_env_errors: false,
    },
    RootSpec {
        field: "notes_root",
        env: "COCKPIT_NOTES_ROOT",
        default: "cockpit/notes",
        overlaps: &[Root::Library, Root::State, Root::Worktree, Root::Cache],
        overlaps_repositories: true,
        field_coded_env_errors: true,
    },
];

const GROUPS: [(&str, &str, &[Root]); 4] = [
    ("XDG_CACHE_HOME", ".cache", &[Root::Cache]),
    (
        "XDG_STATE_HOME",
        ".local/state",
        &[Root::Worktree, Root::State],
    ),
    ("XDG_DATA_HOME", ".local/share", &[Root::Library]),
    ("XDG_DATA_HOME", ".local/share", &[Root::Notes]),
];

pub(super) fn resolve(
    mut file: RawRoots,
    invocation: Option<&[PathBuf]>,
) -> Result<Roots, InspectionError> {
    let repository = resolve_repositories(file.repository_roots.take(), invocation)?;
    let mut values: [String; 5] = std::array::from_fn(|_| String::new());
    for (variable, home_suffix, group) in GROUPS {
        resolve_group(
            &mut file,
            &mut values,
            &repository,
            variable,
            home_suffix,
            group,
        )?;
    }
    let [cache, worktree, state, library, notes] = values;
    Ok(Roots {
        repository,
        cache,
        worktree,
        state,
        library,
        notes,
    })
}

fn resolve_group(
    file: &mut RawRoots,
    values: &mut [String; 5],
    repository: &[String],
    variable: &str,
    home_suffix: &str,
    group: &[Root],
) -> Result<(), InspectionError> {
    let base = xdg_directory(variable, home_suffix)?;
    // Choose the entire group before validating any path: state text errors
    // must still precede worktree traversal errors.
    for &root in group {
        let spec = &ROOTS[root as usize];
        let default = path_text(&base.join(spec.default), spec.field)?;
        let chosen = configured(spec.env, file.take(root)).map_err(|error| {
            if spec.field_coded_env_errors {
                InspectionError::new("invalid_notes_root", error.message)
            } else {
                error
            }
        })?;
        values[root as usize] = chosen.unwrap_or(default);
    }
    for &root in group {
        validate_root(root, values, repository)?;
    }
    Ok(())
}

fn validate_root(
    root: Root,
    values: &[String; 5],
    repository: &[String],
) -> Result<(), InspectionError> {
    let spec = &ROOTS[root as usize];
    let value = &values[root as usize];
    validate_paths(std::slice::from_ref(value), spec.field)?;
    for &other in spec.overlaps {
        reject_overlap(
            value,
            &values[other as usize],
            spec.field,
            ROOTS[other as usize].field,
        )?;
    }
    if spec.overlaps_repositories {
        for other in repository {
            reject_overlap(value, other, spec.field, "repository_roots")?;
        }
    }
    Ok(())
}

fn reject_overlap(
    value: &str,
    other: &str,
    field: &str,
    other_field: &str,
) -> Result<(), InspectionError> {
    let value = Path::new(value);
    let other = Path::new(other);
    if value.starts_with(other) || other.starts_with(value) {
        return Err(InspectionError::new(
            format!("invalid_{field}"),
            format!("{field} must not overlap {other_field}"),
        ));
    }
    Ok(())
}

fn resolve_repositories(
    file: Option<Vec<String>>,
    invocation: Option<&[PathBuf]>,
) -> Result<Vec<String>, InspectionError> {
    let roots = if let Some(roots) = invocation {
        if roots.is_empty() {
            return Err(InspectionError::new(
                "invalid_repository_roots",
                "repository_roots invocation override must contain at least one path",
            ));
        }
        roots
            .iter()
            .map(|path| path_text(path, "repository_roots"))
            .collect::<Result<Vec<_>, _>>()?
    } else if let Some(value) = env::var_os("COCKPIT_REPOSITORY_ROOTS") {
        let roots = env::split_paths(&value)
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| path_text(&path, "repository_roots"))
            .collect::<Result<Vec<_>, _>>()?;
        if roots.is_empty() {
            return Err(InspectionError::new(
                "invalid_repository_roots",
                "COCKPIT_REPOSITORY_ROOTS must contain at least one path",
            ));
        }
        roots
    } else if let Some(roots) = file {
        roots
    } else {
        vec![path_text(
            &env::current_dir().map_err(|_| {
                InspectionError::new(
                    "configuration_unavailable",
                    "Cannot resolve the current repository directory",
                )
            })?,
            "repository_roots",
        )?]
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
    Ok(roots)
}
