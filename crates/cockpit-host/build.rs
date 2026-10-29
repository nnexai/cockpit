use std::{path::PathBuf, process::Command};

fn git(root: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").current_dir(root).args(args).output()
        .or_else(|_| Command::new("/usr/bin/git").current_dir(root).args(args).output()).ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .parent().unwrap().parent().unwrap().canonicalize().unwrap();
    println!("cargo:rustc-env=COCKPIT_SOURCE_ROOT={}", root.display());
    if let Some(commit) = git(&root, &["rev-parse", "HEAD"]) {
        println!("cargo:rustc-env=COCKPIT_BUILD_COMMIT={commit}");
    }
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&root, &["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
    }
    if let Some(reference) = git(&root, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(&root, &["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
    }
}
