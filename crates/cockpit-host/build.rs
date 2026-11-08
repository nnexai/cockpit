use std::{fs, path::{Path, PathBuf}, process::Command};

fn git(root: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").current_dir(root).args(args).output()
        .or_else(|_| Command::new("/usr/bin/git").current_dir(root).args(args).output()).ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn collect_web_assets(root: &Path, dir: &Path, assets: &mut Vec<(String, PathBuf)>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries = fs::read_dir(dir).expect("read embedded web directory")
        .collect::<Result<Vec<_>, _>>().expect("read embedded web entry");
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().expect("read embedded web entry type");
        if kind.is_dir() {
            collect_web_assets(root, &path, assets);
        } else if kind.is_file() {
            println!("cargo:rerun-if-changed={}", path.display());
            let name = path.strip_prefix(root).unwrap().to_str()
                .expect("embedded web paths must be UTF-8").replace('\\', "/");
            assets.push((name, path));
        } else {
            panic!("embedded web assets must not contain symlinks or special files: {}", path.display());
        }
    }
}

fn generate_web_assets() {
    println!("cargo:rerun-if-env-changed=COCKPIT_EMBED_WEB_DIST");
    let mut assets = Vec::new();
    if let Some(root) = std::env::var_os("COCKPIT_EMBED_WEB_DIST") {
        let root = PathBuf::from(root);
        assert!(root.is_absolute(), "COCKPIT_EMBED_WEB_DIST must be absolute");
        collect_web_assets(&root, &root, &mut assets);
        assert!(assets.iter().any(|(name, _)| name == "index.html"),
            "embedded web assets must contain index.html");
    }
    let mut source = String::from("static EMBEDDED_WEB_ASSETS: &[(&str, &[u8])] = &[\n");
    for (name, path) in assets {
        source.push_str(&format!("    ({name:?}, include_bytes!({:?})),\n", path.to_str().expect("embedded web paths must be UTF-8")));
    }
    source.push_str("];\n");
    fs::write(PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("web_assets.rs"), source)
        .expect("write embedded web assets table");
}

fn main() {
    generate_web_assets();
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
