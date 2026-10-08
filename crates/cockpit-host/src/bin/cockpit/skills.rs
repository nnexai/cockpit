use std::{
    fs::{File, Metadata, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use clap::{ArgGroup, Args, Subcommand};
use cockpit_core::InspectionError;
use fs2::FileExt;
use nix::{
    errno::Errno,
    fcntl::{AtFlags, OFlag, openat, renameat},
    sys::stat::{Mode, fstatat, mkdirat},
    unistd::{Uid, UnlinkatFlags, linkat, unlinkat},
};
use serde::Serialize;
use serde_json::json;

pub(super) const SKILLS_HELP: &str =
    "Bundled SKILL.md guides for using this CLI, embedded in the binary.
list prints {\"skills\": [{\"name\", \"description\"}]}; show NAME prints one
SKILL.md. install copies skills (all, or the NAMEs given) to exactly one of:
  --home      $HOME/.agent/skills/<name>/SKILL.md
  --project   ./.agents/skills/<name>/SKILL.md (current directory)
Existing identical files are left untouched (unchanged). Different existing
files are never overwritten unless --replace is given (conflict, exit 9).
Symlinked, non-directory or foreign-owned destination components are refused
(exit 11). Each skill is reported separately in {\"destination\", \"results\"};
on exit 20 rerun install without --replace to see what was written.
Concurrent installers lock each skill directory. Other same-uid writers can
race the final check and rename; this is not filesystem-wide compare-and-swap.
Nothing else is configured: no agent settings or other skill folders change.

Examples:
  cockpit-cli skills list
  cockpit-cli skills show cockpit-cli-notes
  cockpit-cli skills install --home
  cockpit-cli skills install --project cockpit-cli-orchestration";

#[derive(Debug, Args)]
pub(super) struct SkillsArgs {
    #[command(subcommand)]
    command: SkillsCommand,
}

#[derive(Debug, Subcommand)]
enum SkillsCommand {
    /// List the names and descriptions of bundled skills without installing.
    List,
    /// Print a bundled SKILL.md without writing to the filesystem.
    Show {
        /// Bundled skill name from skills list.
        name: String,
    },
    /// Install selected skills (all by default) into one explicit destination.
    #[command(group(ArgGroup::new("destination").required(true).args(["home", "project"])), after_long_help = SKILLS_HELP)]
    Install {
        /// Install under exactly $HOME/.agent/skills; HOME must be absolute.
        #[arg(long)]
        home: bool,
        /// Install under exactly ./.agents/skills in the current directory.
        #[arg(long)]
        project: bool,
        /// Explicitly replace different existing regular files; never follow links.
        #[arg(long)]
        replace: bool,
        /// Bundled skill names from skills list; omission selects all.
        names: Vec<String>,
    },
}

pub(super) struct BundledSkill {
    pub name: &'static str,
    pub content: &'static str,
}

pub(super) const BUNDLED: &[BundledSkill] = &[
    BundledSkill {
        name: "cockpit-cli-notes",
        content: include_str!(
            "../../../../../integrations/agent-skills/cockpit-cli-notes/SKILL.md"
        ),
    },
    BundledSkill {
        name: "cockpit-cli-orchestration",
        content: include_str!(
            "../../../../../integrations/agent-skills/cockpit-cli-orchestration/SKILL.md"
        ),
    },
];

fn skill(name: &str) -> Result<&'static BundledSkill, InspectionError> {
    BUNDLED
        .iter()
        .find(|skill| skill.name == name)
        .ok_or_else(|| {
            InspectionError::new("skills_usage", format!("Unknown bundled skill: {name}"))
        })
}

fn description(content: &'static str) -> &'static str {
    content
        .strip_prefix("---\n")
        .and_then(|body| body.split_once("\n---"))
        .and_then(|(header, _)| {
            header
                .lines()
                .find_map(|line| line.strip_prefix("description:"))
        })
        .map(str::trim)
        .unwrap_or("")
}

fn unsafe_path(path: &Path) -> InspectionError {
    InspectionError::new(
        "skills_unsafe_path",
        format!(
            "Destination must be a real owned directory or regular file: {}",
            path.display()
        ),
    )
}

fn unknown(error: impl std::fmt::Display) -> InspectionError {
    InspectionError::new(
        "skills_outcome_unknown",
        format!(
            "{error}; rerun `skills install` without --replace to inspect; identical content reports unchanged"
        ),
    )
}

fn conflict(message: impl Into<String>) -> InspectionError {
    InspectionError::new("skills_conflict", message)
}

fn same_inode(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn same_snapshot(left: &Metadata, right: &Metadata) -> bool {
    same_inode(left, right)
        && left.len() == right.len()
        && left.mode() == right.mode()
        && left.uid() == right.uid()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

fn safe_entry(
    parent: &File,
    name: &str,
    path: &Path,
    expected: nix::libc::mode_t,
) -> Result<bool, InspectionError> {
    let metadata = match fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(metadata) => metadata,
        Err(Errno::ENOENT) => return Ok(false),
        Err(Errno::ELOOP | Errno::ENOTDIR) => return Err(unsafe_path(path)),
        Err(error) => return Err(unknown(error)),
    };
    if metadata.st_mode & nix::libc::S_IFMT != expected
        || metadata.st_uid != Uid::current().as_raw()
    {
        return Err(unsafe_path(path));
    }
    Ok(true)
}

fn open_directory(parent: &File, name: &str, path: &Path) -> Result<Option<File>, InspectionError> {
    if !safe_entry(parent, name, path, nix::libc::S_IFDIR)? {
        return Ok(None);
    }
    match openat(
        parent,
        name,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => {
            let file = File::from(fd);
            let metadata = file.metadata().map_err(unknown)?;
            if !metadata.is_dir() || metadata.uid() != Uid::current().as_raw() {
                return Err(unsafe_path(path));
            }
            Ok(Some(file))
        }
        Err(Errno::ENOENT) => Ok(None),
        Err(Errno::ELOOP | Errno::ENOTDIR) => Err(unsafe_path(path)),
        Err(error) => Err(unknown(error)),
    }
}

// Keep each parent open: replacing a path cannot retarget fd-relative writes.
struct Destination {
    base: PathBuf,
    names: [&'static str; 2],
    directories: Vec<File>,
}

impl Destination {
    fn inspect(base: PathBuf, home: bool) -> Result<Self, InspectionError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_CLOEXEC)
            .open(&base)
            .map_err(unknown)?;
        let mut destination = Self {
            base,
            names: [if home { ".agent" } else { ".agents" }, "skills"],
            directories: vec![file],
        };
        for name in destination.names {
            let path = destination.path().join(name);
            match open_directory(destination.directories.last().unwrap(), name, &path)? {
                Some(file) => destination.directories.push(file),
                None => break,
            }
        }
        destination.verify()?;
        Ok(destination)
    }

    fn path(&self) -> PathBuf {
        self.names[..self.directories.len() - 1]
            .iter()
            .fold(self.base.clone(), |path, name| path.join(name))
    }

    fn verify(&self) -> Result<(), InspectionError> {
        let current = File::open(&self.base)
            .map_err(unknown)?
            .metadata()
            .map_err(unknown)?;
        if !same_inode(&current, &self.directories[0].metadata().map_err(unknown)?) {
            return Err(unsafe_path(&self.base));
        }
        let mut path = self.base.clone();
        for (index, name) in self.names[..self.directories.len() - 1].iter().enumerate() {
            path.push(name);
            verify_directory(
                &self.directories[index],
                name,
                &self.directories[index + 1],
                &path,
            )?;
        }
        Ok(())
    }

    fn create(&mut self) -> Result<(), InspectionError> {
        while self.directories.len() < 3 {
            self.verify()?;
            let name = self.names[self.directories.len() - 1];
            let parent = self.directories.last().unwrap();
            let path = self.path().join(name);
            match mkdirat(parent, name, Mode::from_bits_truncate(0o777)) {
                Ok(()) => parent.sync_all().map_err(unknown)?,
                Err(Errno::EEXIST) => {}
                Err(error) => return Err(unknown(error)),
            }
            let file = open_directory(parent, name, &path)?.ok_or_else(|| unsafe_path(&path))?;
            self.directories.push(file);
        }
        self.verify()
    }

    fn directory(&self) -> &File {
        self.directories.last().unwrap()
    }
}

fn verify_directory(
    parent: &File,
    name: &str,
    directory: &File,
    path: &Path,
) -> Result<(), InspectionError> {
    let metadata = directory.metadata().map_err(unknown)?;
    let current =
        fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW).map_err(|_| unsafe_path(path))?;
    if current.st_dev as u64 != metadata.dev()
        || current.st_ino as u64 != metadata.ino()
        || current.st_mode & nix::libc::S_IFMT != nix::libc::S_IFDIR
        || current.st_uid != Uid::current().as_raw()
    {
        return Err(unsafe_path(path));
    }
    Ok(())
}

struct DirectoryLock<'a>(&'a File);
impl Drop for DirectoryLock<'_> {
    fn drop(&mut self) {
        let _ = FileExt::unlock(self.0);
    }
}

struct Existing {
    metadata: Metadata,
    bytes: Vec<u8>,
}

fn existing(
    directory: &File,
    path: &Path,
    length: usize,
) -> Result<Option<Existing>, InspectionError> {
    if !safe_entry(directory, "SKILL.md", path, nix::libc::S_IFREG)? {
        return Ok(None);
    }
    let file = match openat(
        directory,
        "SKILL.md",
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(Errno::ENOENT) => return Ok(None),
        Err(Errno::ELOOP | Errno::ENOTDIR) => return Err(unsafe_path(path)),
        Err(error) => return Err(unknown(error)),
    };
    let metadata = file.metadata().map_err(unknown)?;
    if !metadata.is_file() || metadata.uid() != Uid::current().as_raw() {
        return Err(unsafe_path(path));
    }
    let mut bytes = Vec::new();
    (&file)
        .take(length as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(unknown)?;
    if !same_snapshot(&metadata, &file.metadata().map_err(unknown)?) {
        return Err(conflict(
            "SKILL.md changed while being read; inspect before retrying",
        ));
    }
    Ok(Some(Existing { metadata, bytes }))
}

struct Temporary<'a> {
    directory: &'a File,
    name: String,
    file: File,
    present: bool,
}

impl Temporary<'_> {
    fn verify(&self) -> Result<(), InspectionError> {
        let metadata = self.file.metadata().map_err(unknown)?;
        let current = fstatat(
            self.directory,
            self.name.as_str(),
            AtFlags::AT_SYMLINK_NOFOLLOW,
        )
        .map_err(unknown)?;
        if current.st_dev as u64 != metadata.dev()
            || current.st_ino as u64 != metadata.ino()
            || current.st_mode & nix::libc::S_IFMT != nix::libc::S_IFREG
        {
            return Err(unknown("Temporary file identity changed"));
        }
        Ok(())
    }

    fn remove(&mut self) -> Result<(), InspectionError> {
        if self.present {
            self.verify()?;
            unlinkat(
                self.directory,
                self.name.as_str(),
                UnlinkatFlags::NoRemoveDir,
            )
            .map_err(unknown)?;
            self.present = false;
        }
        Ok(())
    }
}
impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

#[derive(Serialize)]
struct SkillResult {
    skill: &'static str,
    path: PathBuf,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

fn install_skill(
    destination: &Destination,
    skill: &BundledSkill,
    replace: bool,
) -> Result<&'static str, InspectionError> {
    destination.verify()?;
    let path = destination.path().join(skill.name);
    let parent = destination.directory();
    match mkdirat(parent, skill.name, Mode::from_bits_truncate(0o777)) {
        Ok(()) => parent.sync_all().map_err(unknown)?,
        Err(Errno::EEXIST) => {}
        Err(error) => return Err(unknown(error)),
    }
    let directory = open_directory(parent, skill.name, &path)?.ok_or_else(|| unsafe_path(&path))?;
    directory.lock_exclusive().map_err(unknown)?;
    let _lock = DirectoryLock(&directory);
    let verify = || {
        destination.verify()?;
        verify_directory(parent, skill.name, &directory, &path)
    };
    verify()?;
    let final_path = path.join("SKILL.md");
    let content = skill.content.as_bytes();
    let original = existing(&directory, &final_path, content.len())?;
    if let Some(original) = &original {
        if original.bytes == content {
            let current = existing(&directory, &final_path, content.len())?.ok_or_else(|| {
                conflict("SKILL.md disappeared while comparing; inspect before retrying")
            })?;
            if !same_snapshot(&original.metadata, &current.metadata) || current.bytes != content {
                return Err(conflict(
                    "SKILL.md changed while comparing; inspect before retrying",
                ));
            }
            verify()?;
            return Ok("unchanged");
        }
        if !replace {
            return Err(conflict(
                "Existing SKILL.md differs; use --replace only to explicitly overwrite it",
            ));
        }
    }
    verify()?;
    let name = format!(".{}.SKILL.md.{}.tmp", skill.name, uuid::Uuid::new_v4());
    let file = File::from(
        openat(
            &directory,
            name.as_str(),
            OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::from_bits_truncate(0o644),
        )
        .map_err(unknown)?,
    );
    let mut temporary = Temporary {
        directory: &directory,
        name,
        file,
        present: true,
    };
    temporary.file.write_all(content).map_err(unknown)?;
    temporary.file.sync_all().map_err(unknown)?;
    verify()?;
    temporary.verify()?;
    let status = if let Some(original) = original {
        let current = existing(&directory, &final_path, content.len())?.ok_or_else(|| {
            conflict("SKILL.md disappeared before replacement; inspect before retrying")
        })?;
        if !same_snapshot(&original.metadata, &current.metadata) || original.bytes != current.bytes
        {
            return Err(conflict(
                "SKILL.md changed before replacement; inspect before retrying",
            ));
        }
        // Advisory locking serializes installers, not external same-uid editors.
        verify()?;
        renameat(&directory, temporary.name.as_str(), &directory, "SKILL.md").map_err(unknown)?;
        temporary.present = false;
        "replaced"
    } else {
        match linkat(
            &directory,
            temporary.name.as_str(),
            &directory,
            "SKILL.md",
            AtFlags::empty(),
        ) {
            Ok(()) => "installed",
            Err(Errno::EEXIST) => {
                let current = existing(&directory, &final_path, content.len())?;
                if current.is_some_and(|current| current.bytes == content) {
                    "unchanged"
                } else {
                    return Err(conflict(
                        "SKILL.md appeared during installation and differs; it was not replaced",
                    ));
                }
            }
            Err(error) => return Err(unknown(error)),
        }
    };
    temporary.remove()?;
    directory.sync_all().map_err(unknown)?;
    verify().map_err(|error| unknown(error.message))?;
    Ok(status)
}

fn exit_code(code: &str) -> u8 {
    match code {
        "skills_usage" | "skills_home_unavailable" => 2,
        "skills_conflict" => 9,
        "skills_unsafe_path" => 11,
        _ => 20,
    }
}

fn write_json(value: &impl Serialize) -> std::io::Result<()> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, value).map_err(std::io::Error::other)?;
    output.write_all(b"\n")
}

pub(super) fn print_error(error: InspectionError) -> u8 {
    eprintln!("error: {}: {}", error.code, error.message);
    let exit = exit_code(&error.code);
    if let Err(error) =
        write_json(&json!({"error": {"code": error.code, "message": error.message}}))
    {
        eprintln!("error: skills_outcome_unknown: Cannot write the skills response: {error}");
        return 20;
    }
    exit
}

fn execute(args: SkillsArgs) -> Result<u8, InspectionError> {
    match args.command {
        SkillsCommand::List => {
            let skills: Vec<_> = BUNDLED
                .iter()
                .map(|skill| json!({"name": skill.name, "description": description(skill.content)}))
                .collect();
            write_json(&json!({"skills": skills})).map_err(unknown)?;
            Ok(0)
        }
        SkillsCommand::Show { name } => {
            std::io::stdout()
                .lock()
                .write_all(skill(&name)?.content.as_bytes())
                .map_err(unknown)?;
            Ok(0)
        }
        SkillsCommand::Install {
            home,
            project: _,
            replace,
            names,
        } => {
            let mut selected = Vec::new();
            if names.is_empty() {
                selected.extend(BUNDLED);
            } else {
                for name in names {
                    let skill = skill(&name)?;
                    if !selected
                        .iter()
                        .any(|selected: &&BundledSkill| selected.name == skill.name)
                    {
                        selected.push(skill);
                    }
                }
            }
            let base = if home {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .ok_or_else(|| {
                        InspectionError::new(
                            "skills_home_unavailable",
                            "HOME must be set to an absolute directory",
                        )
                    })?
            } else {
                std::env::current_dir().map_err(unknown)?
            };
            if base.to_str().is_none() {
                return Err(InspectionError::new(
                    "skills_usage",
                    "Destination must be UTF-8 so installed paths can be reported as JSON",
                ));
            }
            let mut destination = Destination::inspect(base, home)?;
            destination.create()?;
            let mut exit = 0;
            let results: Vec<_> = selected
                .into_iter()
                .map(|skill| {
                    let path = destination.path().join(skill.name).join("SKILL.md");
                    match install_skill(&destination, skill, replace) {
                        Ok(status) => SkillResult {
                            skill: skill.name,
                            path,
                            status,
                            code: None,
                            message: None,
                        },
                        Err(error) => {
                            let code = exit_code(&error.code);
                            exit = exit.max(code);
                            SkillResult {
                                skill: skill.name,
                                path,
                                status: match code {
                                    9 => "conflict",
                                    11 => "refused",
                                    _ => "failed",
                                },
                                code: Some(error.code),
                                message: Some(error.message),
                            }
                        }
                    }
                })
                .collect();
            write_json(&json!({"destination": destination.path(), "results": results}))
                .map_err(unknown)?;
            Ok(exit)
        }
    }
}

pub(super) async fn run(args: SkillsArgs) -> u8 {
    match execute(args) {
        Ok(exit) => exit,
        Err(error) => print_error(error),
    }
}
