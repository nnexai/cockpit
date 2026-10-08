use std::{
    fs,
    os::unix::fs::{MetadataExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use serde_json::Value;

const NOTES: &str = "cockpit-cli-notes";
const ORCHESTRATION: &str = "cockpit-cli-orchestration";

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-skills-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let project = root.join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir(&project).unwrap();
        Self {
            root,
            home,
            project,
        }
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cockpit"));
        command
            .arg("skills")
            .args(arguments)
            .current_dir(&self.project)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env_remove("COCKPIT_CONFIG")
            .env_remove("COCKPIT_CONFIG_PATH")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn cli(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().unwrap()
    }

    fn project_skill(&self, name: &str) -> PathBuf {
        self.project
            .join(".agents/skills")
            .join(name)
            .join("SKILL.md")
    }

    fn home_skill(&self, name: &str) -> PathBuf {
        self.home.join(".agent/skills").join(name).join("SKILL.md")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn response(output: Output, exit: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn result<'a>(response: &'a Value, name: &str) -> &'a Value {
    response["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["skill"] == name)
        .unwrap()
}

fn assert_no_temporary_files(folder: &Path) {
    let mut names: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names, [std::ffi::OsString::from("SKILL.md")]);
}

#[test]
fn list_and_unknown_show_have_no_installation_effects() {
    let fixture = Fixture::new();
    let list = response(fixture.cli(&["list"]), 0);
    let names: Vec<_> = list["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skill| skill["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, [NOTES, ORCHESTRATION]);
    let error = response(fixture.cli(&["show", "not-bundled"]), 2);
    assert_eq!(error["error"]["code"], "skills_usage");
    assert!(!fixture.home.join(".agent").exists());
    assert!(!fixture.project.join(".agents").exists());
}

#[test]
fn symlinked_home_base_is_allowed_but_descendants_stay_below_it() {
    let fixture = Fixture::new();
    let alias = fixture.root.join("home-alias");
    symlink(&fixture.home, &alias).unwrap();
    let output = response(
        fixture
            .command(&["install", "--home", NOTES])
            .env("HOME", &alias)
            .output()
            .unwrap(),
        0,
    );
    assert_eq!(
        output["destination"],
        alias.join(".agent/skills").to_str().unwrap()
    );
    assert_eq!(
        result(&output, NOTES)["path"],
        alias
            .join(".agent/skills")
            .join(NOTES)
            .join("SKILL.md")
            .to_str()
            .unwrap()
    );
    assert!(fixture.home_skill(NOTES).is_file());
    assert!(!fixture.home.join(".agents").exists());
}

#[test]
fn file_in_place_of_skill_directory_is_preserved_and_other_skill_installs() {
    let fixture = Fixture::new();
    let directory = fixture.project_skill(NOTES).parent().unwrap().to_path_buf();
    fs::create_dir_all(directory.parent().unwrap()).unwrap();
    fs::write(&directory, "not a skill directory").unwrap();
    let output = response(fixture.cli(&["install", "--project", "--replace"]), 11);
    assert_eq!(result(&output, NOTES)["status"], "refused");
    assert_eq!(result(&output, ORCHESTRATION)["status"], "installed");
    assert_eq!(
        fs::read_to_string(directory).unwrap(),
        "not a skill directory"
    );
}

#[test]
fn installs_to_exact_home_and_project_targets() {
    let fixture = Fixture::new();
    let home = response(fixture.cli(&["install", "--home"]), 0);
    assert_eq!(
        home["destination"],
        fixture.home.join(".agent/skills").to_str().unwrap()
    );
    for name in [NOTES, ORCHESTRATION] {
        let entry = result(&home, name);
        assert_eq!(entry["status"], "installed");
        assert_eq!(entry["path"], fixture.home_skill(name).to_str().unwrap());
        assert!(fixture.home_skill(name).is_file());
    }
    assert!(!fixture.home.join(".agents").exists());
    assert!(!fixture.project.join(".agents").exists());

    let project = response(fixture.cli(&["install", "--project", NOTES]), 0);
    assert_eq!(
        project["destination"],
        fixture.project.join(".agents/skills").to_str().unwrap()
    );
    assert_eq!(
        result(&project, NOTES)["path"],
        fixture.project_skill(NOTES).to_str().unwrap()
    );
    assert_eq!(project["results"].as_array().unwrap().len(), 1);
    assert!(fixture.project_skill(NOTES).is_file());
    assert!(!fixture.project_skill(ORCHESTRATION).exists());
    assert!(!fixture.project.join(".agent").exists());
}

#[test]
fn identical_install_preserves_inode_and_mtime() {
    let fixture = Fixture::new();
    response(fixture.cli(&["install", "--project"]), 0);
    let before: Vec<_> = [NOTES, ORCHESTRATION]
        .map(|name| fs::metadata(fixture.project_skill(name)).unwrap())
        .into();
    let repeated = response(fixture.cli(&["install", "--project", "--replace"]), 0);
    for (name, before) in [NOTES, ORCHESTRATION].into_iter().zip(before) {
        assert_eq!(result(&repeated, name)["status"], "unchanged");
        let after = fs::metadata(fixture.project_skill(name)).unwrap();
        assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
        assert_eq!(
            (before.mtime(), before.mtime_nsec()),
            (after.mtime(), after.mtime_nsec())
        );
        assert_no_temporary_files(fixture.project_skill(name).parent().unwrap());
    }
}

#[test]
fn conflict_preserves_bytes_and_replace_only_changes_requested_file() {
    let fixture = Fixture::new();
    response(fixture.cli(&["install", "--project"]), 0);
    let target = fixture.project_skill(NOTES);
    let bundled = fs::read(&target).unwrap();
    fs::write(&target, b"user-edited skill\n").unwrap();
    let before = fs::metadata(&target).unwrap();
    let sibling = target.parent().unwrap().join("user-data");
    fs::write(&sibling, "keep me").unwrap();
    let conflict = response(fixture.cli(&["install", "--project"]), 9);
    assert_eq!(result(&conflict, NOTES)["status"], "conflict");
    assert_eq!(result(&conflict, NOTES)["code"], "skills_conflict");
    assert_eq!(result(&conflict, ORCHESTRATION)["status"], "unchanged");
    assert_eq!(fs::read(&target).unwrap(), b"user-edited skill\n");
    let after = fs::metadata(&target).unwrap();
    assert_eq!(
        (before.ino(), before.mtime(), before.mtime_nsec()),
        (after.ino(), after.mtime(), after.mtime_nsec())
    );

    let replaced = response(
        fixture.cli(&["install", "--project", "--replace", NOTES]),
        0,
    );
    assert_eq!(result(&replaced, NOTES)["status"], "replaced");
    assert_eq!(fs::read(target).unwrap(), bundled);
    assert_eq!(fs::read_to_string(sibling).unwrap(), "keep me");
}

#[test]
fn conflict_does_not_stop_other_requested_installs() {
    let fixture = Fixture::new();
    let target = fixture.project_skill(NOTES);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "preserve edited skill").unwrap();
    let output = response(fixture.cli(&["install", "--project"]), 9);
    assert_eq!(result(&output, NOTES)["status"], "conflict");
    assert_eq!(result(&output, ORCHESTRATION)["status"], "installed");
    assert_eq!(fs::read_to_string(target).unwrap(), "preserve edited skill");
    assert!(fixture.project_skill(ORCHESTRATION).is_file());
}

#[test]
fn symlinked_final_is_refused_even_with_replace_and_other_skill_installs() {
    let fixture = Fixture::new();
    let outside = fixture.root.join("outside");
    fs::write(&outside, "outside bytes").unwrap();
    let target = fixture.project_skill(NOTES);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    symlink(&outside, &target).unwrap();
    let output = response(fixture.cli(&["install", "--project", "--replace"]), 11);
    assert_eq!(result(&output, NOTES)["status"], "refused");
    assert_eq!(result(&output, NOTES)["code"], "skills_unsafe_path");
    assert_eq!(result(&output, ORCHESTRATION)["status"], "installed");
    assert_eq!(fs::read_to_string(outside).unwrap(), "outside bytes");
    assert!(
        fs::symlink_metadata(target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn nonregular_final_and_unsafe_skill_directory_are_independent_refusals() {
    let fixture = Fixture::new();
    let notes = fixture.project_skill(NOTES);
    fs::create_dir_all(&notes).unwrap();
    let outside = fixture.root.join("outside-folder");
    fs::create_dir(&outside).unwrap();
    symlink(
        &outside,
        fixture.project_skill(ORCHESTRATION).parent().unwrap(),
    )
    .unwrap();
    let output = response(fixture.cli(&["install", "--project", "--replace"]), 11);
    for name in [NOTES, ORCHESTRATION] {
        assert_eq!(result(&output, name)["status"], "refused");
    }
    assert!(notes.is_dir());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn dangling_symlink_and_fifo_final_are_refused_without_blocking() {
    let fixture = Fixture::new();
    let notes = fixture.project_skill(NOTES);
    let orchestration = fixture.project_skill(ORCHESTRATION);
    fs::create_dir_all(notes.parent().unwrap()).unwrap();
    fs::create_dir_all(orchestration.parent().unwrap()).unwrap();
    symlink(fixture.root.join("missing"), &notes).unwrap();
    nix::unistd::mkfifo(
        &orchestration,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    let output = response(fixture.cli(&["install", "--project", "--replace"]), 11);
    for name in [NOTES, ORCHESTRATION] {
        assert_eq!(result(&output, name)["status"], "refused");
    }
    assert!(!fixture.root.join("missing").exists());
}

#[test]
fn unsafe_root_components_fail_before_installation() {
    for component in [".agents", ".agents/skills"] {
        let fixture = Fixture::new();
        let outside = fixture.root.join("outside");
        fs::create_dir(&outside).unwrap();
        let link = fixture.project.join(component);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&outside, link).unwrap();
        let error = response(fixture.cli(&["install", "--project"]), 11);
        assert_eq!(error["error"]["code"], "skills_unsafe_path");
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
    let fixture = Fixture::new();
    fs::write(fixture.project.join(".agents"), "not a directory").unwrap();
    let error = response(fixture.cli(&["install", "--project"]), 11);
    assert_eq!(error["error"]["code"], "skills_unsafe_path");
    assert_eq!(
        fs::read_to_string(fixture.project.join(".agents")).unwrap(),
        "not a directory"
    );
}

#[test]
fn validates_all_names_and_destination_before_effects() {
    let fixture = Fixture::new();
    for arguments in [
        vec!["install"],
        vec!["install", "--home", "--project"],
        vec!["install", "--project", NOTES, "not-bundled"],
        vec!["install", "--project", "../escape"],
    ] {
        let error = response(fixture.cli(&arguments), 2);
        assert_eq!(error["error"]["code"], "skills_usage");
        assert!(!fixture.home.join(".agent").exists());
        assert!(!fixture.project.join(".agents").exists());
    }
    for home in [None, Some("relative"), Some("")] {
        let mut command = fixture.command(&["install", "--home"]);
        match home {
            Some(home) => {
                command.env("HOME", home);
            }
            None => {
                command.env_remove("HOME");
            }
        }
        let error = response(command.output().unwrap(), 2);
        assert_eq!(error["error"]["code"], "skills_home_unavailable");
        assert!(!fixture.home.join(".agent").exists());
        assert!(!fixture.project.join(".agents").exists());
    }
}

#[test]
fn concurrent_installers_publish_once_and_leave_no_temps() {
    let fixture = Fixture::new();
    let children: Vec<_> = (0..8)
        .map(|_| {
            fixture
                .command(&["install", "--project", NOTES])
                .spawn()
                .unwrap()
        })
        .collect();
    let mut installed = 0;
    for child in children {
        let output = response(child.wait_with_output().unwrap(), 0);
        match result(&output, NOTES)["status"].as_str().unwrap() {
            "installed" => installed += 1,
            "unchanged" => {}
            status => panic!("Unexpected status: {status}"),
        }
    }
    assert_eq!(installed, 1);
    assert_no_temporary_files(fixture.project_skill(NOTES).parent().unwrap());

    fs::write(fixture.project_skill(NOTES), "old version").unwrap();
    let children: Vec<_> = (0..8)
        .map(|_| {
            fixture
                .command(&["install", "--project", "--replace", NOTES])
                .spawn()
                .unwrap()
        })
        .collect();
    let mut replaced = 0;
    for child in children {
        let output = response(child.wait_with_output().unwrap(), 0);
        match result(&output, NOTES)["status"].as_str().unwrap() {
            "replaced" => replaced += 1,
            "unchanged" => {}
            status => panic!("Unexpected status: {status}"),
        }
    }
    assert_eq!(replaced, 1);
    assert_no_temporary_files(fixture.project_skill(NOTES).parent().unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn swapped_skill_directory_cannot_retarget_a_waiting_installer() {
    use fs2::FileExt;
    use std::time::{Duration, Instant};

    let fixture = Fixture::new();
    let target = fixture.project_skill(NOTES);
    let folder = target.parent().unwrap();
    fs::create_dir_all(folder).unwrap();
    fs::write(&target, "original directory bytes").unwrap();
    let held = fs::File::open(folder).unwrap();
    held.lock_exclusive().unwrap();
    let mut child = fixture
        .command(&["install", "--project", "--replace", NOTES])
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let fds = fs::read_dir(format!("/proc/{}/fd", child.id())).unwrap();
        if fds
            .filter_map(Result::ok)
            .any(|entry| fs::read_link(entry.path()).is_ok_and(|path| path == folder))
        {
            break;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "Installer exited before reaching the directory lock"
        );
        assert!(
            Instant::now() < deadline,
            "Installer did not open the skill directory"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let moved = fixture.root.join("moved-skill");
    fs::rename(folder, &moved).unwrap();
    fs::create_dir(folder).unwrap();
    fs::write(&target, "replacement directory bytes").unwrap();
    FileExt::unlock(&held).unwrap();
    let output = response(child.wait_with_output().unwrap(), 11);
    assert_eq!(result(&output, NOTES)["status"], "refused");
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "replacement directory bytes"
    );
    assert_eq!(
        fs::read_to_string(moved.join("SKILL.md")).unwrap(),
        "original directory bytes"
    );
    assert_no_temporary_files(&moved);
    assert_no_temporary_files(folder);
}
