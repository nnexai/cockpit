use super::*;
use crate::library::tests::{add, fixture, finished, Fixture};

async fn save(f: &Fixture, source: &Path) -> LibraryItemSummary {
    let mut request = add(1);
    request.input = source.to_string_lossy().into_owned();
    request.label = Some("Local notes".into());
    let operation = finished(&f.service, f.service.start_add(request).await.unwrap()).await;
    assert!(matches!(operation.phases[0].state, LibraryPhaseState::Done | LibraryPhaseState::Partial), "{operation:?}");
    f.service.listing(None).await.unwrap().items.into_iter().next().unwrap()
}
async fn refresh(f: &Fixture, item: &LibraryItemSummary) -> LibraryOperation {
    finished(&f.service, f.service.start_refresh(LibraryRefreshRequest::Items {
        item_ids: vec![item.item_id.clone()],
    }).await.unwrap()).await
}
fn git(path: &Path, args: &[&str]) {
    let output = std::process::Command::new("git").arg("-C").arg(path).args(args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[cfg(unix)]
#[tokio::test]
async fn plain_folder_excludes_links_special_files_git_metadata_and_native_binaries() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = fixture();
    let source = f.root.join("plain");
    std::fs::create_dir_all(source.join("nested/.git")).unwrap();
    std::fs::write(source.join("nested/notes.md"), b"notes").unwrap();
    std::fs::write(source.join("nested/.git/config"), b"private").unwrap();
    symlink(source.join("nested/notes.md"), source.join("link")).unwrap();
    nix::unistd::mkfifo(&source.join("pipe"), nix::sys::stat::Mode::S_IRUSR).unwrap();
    std::fs::write(source.join("hard-original"), b"hardlinked").unwrap();
    std::fs::hard_link(source.join("hard-original"), source.join("hard-copy")).unwrap();
    std::fs::write(source.join("binary"), b"\x7fELFexecutable").unwrap();
    std::fs::set_permissions(source.join("binary"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let item = save(&f, &source).await;
    let info = item.folder.as_ref().unwrap();
    assert_eq!((info.files, info.skipped_symlinks, info.skipped_special, info.skipped_ignored, info.skipped_other), (1, 1, 1, 1, 3));
    let copied = Path::new(&f.service.configuration.library_root).join(&item.item_path);
    assert_eq!(std::fs::read(copied.join("nested/notes.md")).unwrap(), b"notes");
    assert!(!copied.join("link").exists());
    assert!(!copied.join("pipe").exists());
    assert!(!copied.join("nested/.git").exists());
    assert_eq!(item.document_path, Some(format!("{}/nested/notes.md", item.item_path)));
}

#[tokio::test]
async fn git_root_copies_dirty_working_bytes_and_nonignored_untracked_without_gitlinks() {
    let f = fixture();
    let source = f.root.join("git");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-q"]);
    std::fs::write(source.join("tracked.txt"), b"index").unwrap();
    std::fs::write(source.join(".gitignore"), b"ignored.txt\n").unwrap();
    git(&source, &["add", "."]);
    std::fs::write(source.join("tracked.txt"), b"dirty").unwrap();
    std::fs::write(source.join("new.txt"), b"new").unwrap();
    std::fs::write(source.join("ignored.txt"), b"ignored").unwrap();
    git(&source, &["update-index", "--add", "--cacheinfo", "160000,1111111111111111111111111111111111111111,submodule"]);
    std::fs::create_dir(source.join("submodule")).unwrap();
    std::fs::write(source.join("submodule/private.txt"), b"not traversed").unwrap();
    let item = save(&f, &source).await;
    let info = item.folder.as_ref().unwrap();
    assert!(info.git_working_tree);
    assert_eq!(info.files, 3);
    assert_eq!(info.skipped_ignored, 1);
    assert_eq!(info.skipped_other, 1);
    let copied = Path::new(&f.service.configuration.library_root).join(item.item_path);
    assert_eq!(std::fs::read(copied.join("tracked.txt")).unwrap(), b"dirty");
    assert_eq!(std::fs::read(copied.join("new.txt")).unwrap(), b"new");
    assert!(!copied.join("ignored.txt").exists());
    assert!(!copied.join("submodule").exists());
}

#[tokio::test]
async fn source_changes_are_isolated_until_explicit_refresh_and_library_edits_require_cas() {
    let f = fixture();
    let source = f.root.join("plain");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), b"old").unwrap();
    std::fs::write(source.join("b.txt"), b"removed later").unwrap();
    let item = save(&f, &source).await;
    let copied = Path::new(&f.service.configuration.library_root).join(&item.item_path);
    std::fs::write(source.join("a.txt"), b"new").unwrap();
    std::fs::remove_file(source.join("b.txt")).unwrap();
    std::fs::create_dir(source.join("nested")).unwrap();
    std::fs::write(source.join("nested/c.txt"), b"added").unwrap();
    assert_eq!(std::fs::read(copied.join("a.txt")).unwrap(), b"old");
    assert_eq!(f.service.listing(None).await.unwrap().items[0].revision, item.revision);
    refresh(&f, &item).await;
    assert_eq!(std::fs::read(copied.join("a.txt")).unwrap(), b"new");
    assert!(!copied.join("b.txt").exists());
    assert_eq!(std::fs::read(copied.join("nested/c.txt")).unwrap(), b"added");
    std::fs::write(copied.join("a.txt"), b"edited in Library").unwrap();
    std::fs::write(source.join("a.txt"), b"newest").unwrap();
    refresh(&f, &item).await;
    let conflict = f.service.listing(None).await.unwrap().items.remove(0);
    assert_eq!(conflict.state, LibraryItemState::Conflict);
    assert_eq!(std::fs::read(copied.join("a.txt")).unwrap(), b"edited in Library");
    let stale = conflict.conflict.clone();
    std::fs::write(copied.join("a.txt"), b"edited again").unwrap();
    assert_eq!(f.service.start_replace(LibraryReplaceRequest { item_id: item.item_id.clone(), confirmed: stale }).await.unwrap_err().code, "library_conflict");
    refresh(&f, &item).await;
    let conflict = f.service.listing(None).await.unwrap().items.remove(0);
    let operation = f.service.start_replace(LibraryReplaceRequest { item_id: item.item_id.clone(), confirmed: conflict.conflict }).await.unwrap();
    finished(&f.service, operation).await;
    assert_eq!(std::fs::read(copied.join("a.txt")).unwrap(), b"newest");
}

#[tokio::test]
async fn owned_roots_ancestors_descendants_and_symlink_roots_are_refused() {
    let f = fixture();
    for owned in [&f.service.configuration.library_root, &f.service.configuration.companion_root,
        &f.service.configuration.state_root, &f.service.configuration.worktree_root] {
        std::fs::create_dir_all(Path::new(owned).join("child")).unwrap();
        for path in [PathBuf::from(owned), Path::new(owned).join("child"), f.root.clone()] {
            assert_eq!(f.service.resolve(LibraryResolveRequest { input: path.to_string_lossy().into_owned(), provider_id: None }).await.unwrap_err().code, "library_folder_refused");
        }
    }
    #[cfg(unix)] {
        let source = f.root.join("allowed");
        std::fs::create_dir(&source).unwrap();
        std::os::unix::fs::symlink(&source, f.root.join("alias")).unwrap();
        assert_eq!(f.service.resolve(LibraryResolveRequest { input: f.root.join("alias").to_string_lossy().into_owned(), provider_id: None }).await.unwrap_err().code, "library_folder_unavailable");
    }
}

#[tokio::test]
async fn six_hundred_files_produce_sorted_partial_512_of_600() {
    let f = fixture();
    let source = f.root.join("many");
    std::fs::create_dir(&source).unwrap();
    for n in (0..600).rev() { std::fs::write(source.join(format!("{n:04}.txt")), n.to_string()).unwrap(); }
    let item = save(&f, &source).await;
    assert_eq!(item.state, LibraryItemState::Partial);
    let partial = item.partial.unwrap();
    assert_eq!((partial.have, partial.total), (512, Some(600)));
    let copied = Path::new(&f.service.configuration.library_root).join(&item.item_path);
    assert_eq!(std::fs::read(copied.join("0511.txt")).unwrap(), b"511");
    assert!(!copied.join("0512.txt").exists());
    let store = f.service.open().unwrap();
    let entry = f.service.entry(&store, &item.item_id).unwrap().unwrap();
    assert_eq!(entry.inventory.iter().filter(|f| f.path.ends_with(".txt")).count(), 512);
    assert!(store.conflicts(&entry).unwrap().is_empty());
}

#[tokio::test]
async fn byte_limits_keep_a_sorted_prefix_and_report_partial() {
    let mut f = fixture();
    f.service.configuration.limits.library_folder_bytes = 5;
    let source = f.root.join("bytes");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a.txt"), b"1234").unwrap();
    std::fs::write(source.join("b.txt"), b"56").unwrap();
    std::fs::write(source.join("c.txt"), b"7").unwrap();
    let item = save(&f, &source).await;
    assert_eq!((item.partial.as_ref().unwrap().have, item.partial.as_ref().unwrap().total), (1, Some(3)));
    let copied = Path::new(&f.service.configuration.library_root).join(&item.item_path);
    assert_eq!(std::fs::read(copied.join("a.txt")).unwrap(), b"1234");
    assert!(!copied.join("b.txt").exists());
    assert!(!copied.join("c.txt").exists());
}

#[tokio::test]
async fn nested_folder_publication_recovers_origin_and_refreshes_after_rename_crash() {
    let f = fixture();
    let source = f.root.join("recover");
    std::fs::create_dir_all(source.join("nested/deep")).unwrap();
    std::fs::write(source.join("nested/deep/file.txt"), b"before").unwrap();
    let store = f.service.open().unwrap();
    *store.fault.lock().unwrap_or_else(|e| e.into_inner()) = Some("new_to_target");
    let mut request = add(1);
    request.input = source.to_string_lossy().into_owned();
    finished(&f.service, f.service.start_add(request).await.unwrap()).await;
    let reopened = crate::library::tests::reopen(&f);
    let item = reopened.listing(None).await.unwrap().items.remove(0);
    assert!(item.item_path.starts_with("folders/"));
    assert_eq!(item.folder.as_ref().unwrap().origin_path, source.to_string_lossy());
    let copied = Path::new(&f.service.configuration.library_root).join(&item.item_path);
    assert_eq!(std::fs::read(copied.join("nested/deep/file.txt")).unwrap(), b"before");
    std::fs::write(source.join("nested/deep/file.txt"), b"after").unwrap();
    let operation = reopened.start_refresh(LibraryRefreshRequest::Items { item_ids: vec![item.item_id] }).await.unwrap();
    finished(&reopened, operation).await;
    assert_eq!(std::fs::read(copied.join("nested/deep/file.txt")).unwrap(), b"after");
}
