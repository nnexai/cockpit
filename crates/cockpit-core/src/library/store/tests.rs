use super::*;
use crate::library::{
    asset_entry,
    tests::{assert_store_valid, asset, fixture, reopen},
};
use std::sync::{atomic::Ordering, mpsc};

fn folder(
    store: &Arc<Store>,
    origin: &Path,
    previous: Option<&LibraryIndexEntry>,
    bytes: &[u8],
) -> (Stage, LibraryIndexEntry) {
    let mut entry = previous
        .cloned()
        .unwrap_or_else(|| asset_entry(&asset(1, "folder"), None));
    if previous.is_none() {
        entry.summary.item_id = format!("folder:{}", Uuid::new_v4());
        entry.summary.kind = LibraryItemKind::FolderCopy;
        entry.summary.logical_id = entry.summary.item_id.clone();
        entry.summary.item_path = "copied-folder".into();
        entry.summary.document_path = None;
        entry.canonical_url = None;
    }
    entry.summary.revision = hash(bytes);
    entry.summary.folder = Some(LibraryFolderInfo {
        origin_path: origin.to_string_lossy().into_owned(),
        git_working_tree: false,
        files: 2,
        bytes: (bytes.len() * 2) as u64,
        skipped_symlinks: 3,
        skipped_special: 1,
        skipped_ignored: 2,
        skipped_other: 4,
    });
    let stage = store.stage().unwrap();
    // Two files make reader consistency and complete marker verification observable.
    atomic_write_bytes(&stage.dir, "first.txt", bytes).unwrap();
    atomic_write_bytes(&stage.dir, "second.txt", bytes).unwrap();
    store
        .seal(
            &stage,
            &mut entry,
            vec![
                MarkerFile {
                    path: "first.txt".into(),
                    hash: hash(bytes),
                    bytes: bytes.len() as u64,
                },
                MarkerFile {
                    path: "second.txt".into(),
                    hash: hash(bytes),
                    bytes: bytes.len() as u64,
                },
            ],
        )
        .unwrap();
    (stage, entry)
}
fn fault(store: &Store, point: &'static str) {
    *store.fault.lock().unwrap_or_else(|e| e.into_inner()) = Some(point);
}
fn assert_entry(store: &Store, expected: &LibraryIndexEntry, bytes: &[u8]) {
    assert_store_valid(store);
    let _lock = store.shared().unwrap();
    let index = store.index().unwrap();
    assert_eq!(
        serde_json::to_value(&index.items[0]).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let dir = store.item_dir(&expected.summary.item_path).unwrap();
    assert_eq!(dir.read("first.txt").unwrap(), bytes);
    assert_eq!(dir.read("second.txt").unwrap(), bytes);
    assert_eq!(store.journal.entries().unwrap().count(), 0);
}
fn refresh_folder_from_recovered_origin(store: &Arc<Store>, expected: &LibraryIndexEntry) {
    // S4's filesystem refresh consumer can reconstruct its input from the
    // complete recovered entry; no transient original request is required.
    let entry = store.index().unwrap().items.remove(0);
    assert_eq!(
        entry.summary.folder.as_ref().unwrap().origin_path,
        expected.summary.folder.as_ref().unwrap().origin_path
    );
    let origin = Path::new(&entry.summary.folder.as_ref().unwrap().origin_path);
    std::fs::write(origin, b"later folder revision").unwrap();
    let bytes = std::fs::read(origin).unwrap();
    let (stage, refreshed) = folder(store, origin, Some(&entry), &bytes);
    store
        .publish(
            stage,
            refreshed.clone(),
            Some(&entry.summary.revision),
            None,
        )
        .unwrap();
    assert_entry(store, &refreshed, &bytes);
}
#[tokio::test]
async fn first_folder_publish_crash_recovers_complete_origin_and_can_refresh() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    std::fs::write(&origin, b"first").unwrap();
    let (stage, entry) = folder(&store, &origin, None, b"first");
    let stage_name = stage.name.clone();
    fault(&store, "new_to_target");
    assert_eq!(
        store
            .publish(stage, entry.clone(), None, None)
            .unwrap_err()
            .code,
        "library_test_crash"
    );
    let reopened = reopen(&f);
    let listing = reopened.listing(None).await.unwrap();
    assert_eq!(
        listing.items[0].folder.as_ref().unwrap().origin_path,
        origin.to_string_lossy()
    );
    let recovered = reopened.open().unwrap();
    assert_entry(&recovered, &entry, b"first");
    assert!(!exists(&recovered.staging, &stage_name).unwrap());
    refresh_folder_from_recovered_origin(&recovered, &entry);
}
#[test]
fn two_rename_r1_crash_with_invalid_stage_rolls_back_old_content_and_index() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, old) = folder(&store, &origin, None, b"old");
    store.publish(stage, old.clone(), None, None).unwrap();
    let (stage, new) = folder(&store, &origin, Some(&old), b"new");
    let stage_name = stage.name.clone();
    store.force_two_rename.store(true, Ordering::SeqCst);
    fault(&store, "old_to_backup");
    assert_eq!(
        store
            .publish(stage, new, Some(&old.summary.revision), None)
            .unwrap_err()
            .code,
        "library_test_crash"
    );
    assert!(!exists(&store.root, &old.summary.item_path).unwrap());
    // D2 rolls back at R1 only when the staged replacement fails verification.
    // This deliberately supplies the rollback precondition for acceptance (2).
    let staged = store.staging.open_dir_nofollow(&stage_name).unwrap();
    atomic_write_bytes(&staged, "first.txt", b"torn stage").unwrap();
    let recovered = reopen(&f).open().unwrap();
    assert_entry(&recovered, &old, b"old");
    assert!(!exists(&recovered.staging, &stage_name).unwrap());
    assert_eq!(recovered.trash.entries().unwrap().count(), 0);
}
#[test]
fn two_rename_r1_crash_with_valid_stage_rolls_forward_per_d2() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, old) = folder(&store, &origin, None, b"old");
    store.publish(stage, old.clone(), None, None).unwrap();
    let (stage, new) = folder(&store, &origin, Some(&old), b"new");
    store.force_two_rename.store(true, Ordering::SeqCst);
    fault(&store, "old_to_backup");
    assert_eq!(
        store
            .publish(stage, new.clone(), Some(&old.summary.revision), None)
            .unwrap_err()
            .code,
        "library_test_crash"
    );
    let recovered = reopen(&f).open().unwrap();
    assert_entry(&recovered, &new, b"new");
    assert_eq!(recovered.staging.entries().unwrap().count(), 0);
    assert_eq!(recovered.trash.entries().unwrap().count(), 0);
}
#[test]
fn replacement_crash_after_target_rename_recovers_complete_entry_and_origin() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, old) = folder(&store, &origin, None, b"old");
    store.publish(stage, old.clone(), None, None).unwrap();
    let moved_origin = f.root.join("moved-origin.txt");
    let (stage, new) = folder(&store, &moved_origin, Some(&old), b"new");
    fault(&store, "new_to_target");
    assert_eq!(
        store
            .publish(stage, new.clone(), Some(&old.summary.revision), None)
            .unwrap_err()
            .code,
        "library_test_crash"
    );
    let recovered = reopen(&f).open().unwrap();
    assert_entry(&recovered, &new, b"new");
    refresh_folder_from_recovered_origin(&recovered, &new);
}
#[test]
fn crash_after_index_commit_is_idempotent_on_repeated_reopen() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, entry) = folder(&store, &origin, None, b"committed");
    fault(&store, "index_commit");
    assert_eq!(
        store
            .publish(stage, entry.clone(), None, None)
            .unwrap_err()
            .code,
        "library_test_crash"
    );
    let recovered = reopen(&f).open().unwrap();
    assert_entry(&recovered, &entry, b"committed");
    let generation = recovered.index().unwrap().generation;
    let again = reopen(&f).open().unwrap();
    assert_entry(&again, &entry, b"committed");
    assert_eq!(again.index().unwrap().generation, generation);
}
#[test]
fn host_b_open_retains_host_a_active_download_and_crash_orphan() {
    let f = fixture();
    let a = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, entry) = folder(&a, &origin, None, b"active download");
    let stage_name = stage.name.clone();
    let mut orphan = a.stage().unwrap();
    atomic_write_bytes(&orphan.dir, "orphan.bin", b"crash-left bytes").unwrap();
    let orphan_name = orphan.name.clone();
    orphan.owned = false;
    drop(orphan);
    let b = reopen(&f).open().unwrap();
    assert_eq!(
        b.staging
            .open_dir_nofollow(&stage_name)
            .unwrap()
            .read("first.txt")
            .unwrap(),
        b"active download"
    );
    assert_eq!(
        b.staging
            .open_dir_nofollow(&orphan_name)
            .unwrap()
            .read("orphan.bin")
            .unwrap(),
        b"crash-left bytes"
    );
    a.publish(stage, entry.clone(), None, None).unwrap();
    assert_entry(&b, &entry, b"active download");
    assert!(!exists(&a.staging, &stage_name).unwrap());
    let failed = a.stage().unwrap();
    let failed_name = failed.name.clone();
    drop(failed);
    assert!(!exists(&a.staging, &failed_name).unwrap());
    let reopened = reopen(&f).open().unwrap();
    assert_eq!(
        reopened
            .staging
            .open_dir_nofollow(&orphan_name)
            .unwrap()
            .read("orphan.bin")
            .unwrap(),
        b"crash-left bytes"
    );
}
#[test]
fn shared_reader_blocks_two_rename_and_never_observes_missing_or_mixed_target() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, old) = folder(&store, &origin, None, b"old");
    store.publish(stage, old.clone(), None, None).unwrap();
    let (stage, new) = folder(&store, &origin, Some(&old), b"new");
    store.force_two_rename.store(true, Ordering::SeqCst);
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let reader_store = store.clone();
    let old_for_reader = old.clone();
    let reader = std::thread::spawn(move || {
        let lock = reader_store.shared().unwrap();
        held_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        let dir = reader_store
            .item_dir(&old_for_reader.summary.item_path)
            .unwrap();
        assert!(verify_entry(&dir, &old_for_reader).is_ok());
        assert!(!exists(&dir, ".cockpit-item.json").unwrap());
        assert_eq!(dir.read("first.txt").unwrap(), b"old");
        assert_eq!(dir.read("second.txt").unwrap(), b"old");
        drop(lock);
    });
    held_rx.recv().unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let writer_store = store.clone();
    let new_for_writer = new.clone();
    let writer = std::thread::spawn(move || {
        writer_store
            .publish(stage, new_for_writer, Some(&old.summary.revision), None)
            .unwrap();
        done_tx.send(()).unwrap();
    });
    assert!(
        done_rx
            .recv_timeout(std::time::Duration::from_millis(30))
            .is_err(),
        "writer must wait for shared reader"
    );
    release_tx.send(()).unwrap();
    reader.join().unwrap();
    writer.join().unwrap();
    done_rx.recv().unwrap();
    assert_entry(&store, &new, b"new");
}
#[test]
fn publish_rechecks_edits_and_cleans_rejected_operation_stage() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, old) = folder(&store, &origin, None, b"old");
    store.publish(stage, old.clone(), None, None).unwrap();
    let (stage, new) = folder(&store, &origin, Some(&old), b"new");
    let name = stage.name.clone();
    let target = store.item_dir(&old.summary.item_path).unwrap();
    atomic_write_bytes(&target, "second.txt", b"user edited during fetch").unwrap();
    assert_eq!(
        store
            .publish(stage, new, Some(&old.summary.revision), None)
            .unwrap_err()
            .code,
        "library_conflict"
    );
    assert_eq!(
        target.read("second.txt").unwrap(),
        b"user edited during fetch"
    );
    assert!(!exists(&store.staging, &name).unwrap());
    assert_eq!(
        store.index().unwrap().items[0].summary.revision,
        old.summary.revision
    );
}
#[test]
fn oversized_publish_leaves_index_openable_and_prior_items_intact() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, prior) = folder(&store, &origin, None, b"prior");
    store.publish(stage, prior.clone(), None, None).unwrap();

    let (stage, mut oversized) = folder(&store, &origin, None, b"oversized");
    oversized.summary.item_path = "copied-folder-oversized".into();
    oversized.summary.title = "x".repeat(MAX_INDEX as usize + 1024);
    let stage_name = stage.name.clone();
    assert_eq!(
        store
            .publish(stage, oversized, None, None)
            .unwrap_err()
            .code,
        "library_full"
    );
    assert!(!exists(&store.staging, &stage_name).unwrap());

    let reopened = reopen(&f).open().unwrap();
    let index = reopened.index().unwrap();
    assert_eq!(index.items.len(), 1);
    assert_eq!(index.items[0].summary.item_id, prior.summary.item_id);
    assert!(exists(&reopened.root, &prior.summary.item_path).unwrap());
    assert!(!exists(&reopened.root, "copied-folder-oversized").unwrap());
    assert_eq!(reopened.journal.entries().unwrap().count(), 0);
}

#[test]
fn occupied_new_destination_conflicts_without_journaling_or_removing_target() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, entry) = folder(&store, &origin, None, b"new");
    let (parent, leaf) = store.create_item_parent(&entry.summary.item_path).unwrap();
    parent.create_dir(leaf).unwrap();
    let occupied = store.item_dir(&entry.summary.item_path).unwrap();
    atomic_write_bytes(&occupied, "user.txt", b"keep me").unwrap();

    assert_eq!(
        store
            .publish(stage, entry.clone(), None, None)
            .unwrap_err()
            .code,
        "library_conflict"
    );
    assert_eq!(occupied.read("user.txt").unwrap(), b"keep me");
    let reopened = reopen(&f).open().unwrap();
    assert!(reopened.index().unwrap().items.is_empty());
    assert_eq!(reopened.journal.entries().unwrap().count(), 0);
    assert_eq!(
        reopened
            .item_dir(&entry.summary.item_path)
            .unwrap()
            .read("user.txt")
            .unwrap(),
        b"keep me"
    );
}

#[test]
fn item_limit_refuses_add_without_eviction_and_remove_crash_finishes() {
    let f = fixture();
    let store = Store::open(Path::new(&f.service.configuration.library_root), 1).unwrap();
    let mut first = asset_entry(&asset(1, "one"), None);
    let stage = store.stage_asset(&mut first, &asset(1, "one")).unwrap();
    store.publish(stage, first.clone(), None, None).unwrap();
    let mut second = asset_entry(&asset(2, "two"), None);
    let stage = store.stage_asset(&mut second, &asset(2, "two")).unwrap();
    let stage_name = stage.name.clone();
    assert_eq!(
        store.publish(stage, second, None, None).unwrap_err().code,
        "library_full"
    );
    assert_eq!(
        store.index().unwrap().items[0].summary.item_id,
        first.summary.item_id
    );
    assert!(!exists(&store.staging, &stage_name).unwrap());
    assert_store_valid(&store);
    fault(&store, "remove_to_backup");
    assert_eq!(
        store
            .remove(&first.summary.item_id, &first.summary.revision)
            .unwrap_err()
            .code,
        "library_test_crash"
    );
    let recovered = reopen(&f).open().unwrap();
    assert!(recovered.index().unwrap().items.is_empty());
    assert!(!exists(&recovered.root, &first.summary.item_path).unwrap());
    assert_eq!(recovered.trash.entries().unwrap().count(), 0);
    assert_eq!(recovered.journal.entries().unwrap().count(), 0);
}
#[test]
fn unowned_files_and_child_directories_survive_replace_and_remove() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let mut old = asset_entry(&asset(1, "old"), None);
    let stage = store.stage_asset(&mut old, &asset(1, "old")).unwrap();
    store.publish(stage, old.clone(), None, None).unwrap();
    let target = store.item_dir(&old.summary.item_path).unwrap();
    atomic_write_bytes(&target, "notes.md", b"user notes").unwrap();
    target.create_dir("empty").unwrap();
    target.create_dir("nested").unwrap();
    atomic_write_bytes(
        &target.open_dir_nofollow("nested").unwrap(),
        "draft.md",
        b"user draft",
    )
    .unwrap();
    assert!(store.conflicts(&old).unwrap().is_empty());
    let mut new = asset_entry(&asset(1, "new"), Some(&old));
    let stage = store.stage_asset(&mut new, &asset(1, "new")).unwrap();
    store
        .publish(stage, new.clone(), Some(&old.summary.revision), None)
        .unwrap();
    assert_eq!(target.read("notes.md").unwrap(), b"user notes");
    assert_eq!(
        target
            .open_dir_nofollow("nested")
            .unwrap()
            .read("draft.md")
            .unwrap(),
        b"user draft"
    );
    store
        .remove(&new.summary.item_id, &new.summary.revision)
        .unwrap();
    assert_eq!(target.read("notes.md").unwrap(), b"user notes");
    assert_eq!(
        target
            .open_dir_nofollow("nested")
            .unwrap()
            .read("draft.md")
            .unwrap(),
        b"user draft"
    );
}

#[test]
fn edited_owned_document_blocks_publish_while_user_files_remain_unowned() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let mut old = asset_entry(&asset(1, "old"), None);
    let stage = store.stage_asset(&mut old, &asset(1, "old")).unwrap();
    store.publish(stage, old.clone(), None, None).unwrap();
    let target = store.item_dir(&old.summary.item_path).unwrap();
    let document = old
        .summary
        .document_path
        .as_deref()
        .unwrap()
        .strip_prefix(&format!("{}/", old.summary.item_path))
        .unwrap();
    atomic_write_bytes(&target, document, b"user edit").unwrap();
    atomic_write_bytes(&target, "notes.txt", b"user note").unwrap();
    let child = open_child(&target, "child").unwrap();
    atomic_write_bytes(&child, "child.md", b"child document").unwrap();
    assert_eq!(
        store
            .conflicts(&old)
            .unwrap()
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        vec![document]
    );
    let mut new = asset_entry(&asset(1, "old"), Some(&old));
    let stage = store.stage_asset(&mut new, &asset(1, "old")).unwrap();
    assert_eq!(
        store
            .publish(stage, new, Some(&old.summary.revision), None)
            .unwrap_err()
            .code,
        "library_conflict"
    );
    assert_eq!(target.read("notes.txt").unwrap(), b"user note");
    assert_eq!(child.read("child.md").unwrap(), b"child document");
}

#[test]
fn confirmed_edited_predecessor_recovers_at_journal_and_two_rename_r1() {
    for (point, valid_stage, forward) in [
        ("journal", true, false),
        ("old_to_backup", false, false),
        ("old_to_backup", true, true),
    ] {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, old) = folder(&store, &origin, None, b"old");
        store.publish(stage, old.clone(), None, None).unwrap();
        let target = store
            .root
            .open_dir_nofollow(&old.summary.item_path)
            .unwrap();
        atomic_write_bytes(&target, "first.txt", b"confirmed user edit").unwrap();
        atomic_write_bytes(&target, "notes.md", b"confirmed addition").unwrap();
        let confirmed = store.conflicts(&old).unwrap();
        let snapshot = inventory(&target).unwrap();
        let (stage, new) = folder(&store, &origin, Some(&old), b"new");
        let stage_name = stage.name.clone();
        store
            .force_two_rename
            .store(point == "old_to_backup", Ordering::SeqCst);
        fault(&store, point);
        assert_eq!(
            store
                .publish(
                    stage,
                    new.clone(),
                    Some(&old.summary.revision),
                    Some(&confirmed)
                )
                .unwrap_err()
                .code,
            "library_test_crash"
        );
        if !valid_stage {
            atomic_write_bytes(
                &store.staging.open_dir_nofollow(&stage_name).unwrap(),
                "first.txt",
                b"invalid stage",
            )
            .unwrap();
        }
        let recovered = reopen(&f).open().unwrap();
        if forward {
            assert_entry(&recovered, &new, b"new");
        } else {
            assert_eq!(
                serde_json::to_value(&recovered.index().unwrap().items[0]).unwrap(),
                serde_json::to_value(&old).unwrap()
            );
            assert_eq!(
                inventory(
                    &recovered
                        .root
                        .open_dir_nofollow(&old.summary.item_path)
                        .unwrap()
                )
                .unwrap(),
                snapshot
            );
        }
        assert_eq!(recovered.journal.entries().unwrap().count(), 0);
        assert_eq!(recovered.trash.entries().unwrap().count(), 0);
        assert!(!exists(&recovered.staging, &stage_name).unwrap());
    }
}

#[test]
fn recovery_sync_failure_keeps_old_index_and_journal_until_durable_retry() {
    for method in [
        Method::NewTarget,
        Method::Exchange,
        Method::TwoRename,
        Method::Remove,
    ] {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, old) = folder(&store, &origin, None, b"old");
        if method != Method::NewTarget {
            store.publish(stage, old.clone(), None, None).unwrap();
        } else {
            drop(stage);
        }
        let before = serde_json::to_value(store.index().unwrap()).unwrap();
        let new = if method == Method::Remove {
            fault(&store, "rename_unsynced");
            assert_eq!(
                store
                    .remove(&old.summary.item_id, &old.summary.revision)
                    .unwrap_err()
                    .code,
                "library_test_crash"
            );
            None
        } else {
            let previous = (method != Method::NewTarget).then_some(&old);
            let (stage, new) = folder(&store, &origin, previous, b"new");
            store
                .force_two_rename
                .store(method == Method::TwoRename, Ordering::SeqCst);
            fault(&store, "rename_unsynced");
            assert_eq!(
                store
                    .publish(
                        stage,
                        new.clone(),
                        previous.map(|e| e.summary.revision.as_str()),
                        None
                    )
                    .unwrap_err()
                    .code,
                "library_test_crash"
            );
            Some(new)
        };
        // The rename happened, but its durable boundary failed. Recovery must
        // retain both the old index and replay intent if that boundary fails again.
        fault(&store, "recovery_sync");
        assert_eq!(
            store.recover_pending().unwrap_err().code,
            "library_test_crash"
        );
        assert_eq!(
            serde_json::to_value(store.index().unwrap()).unwrap(),
            before
        );
        assert_eq!(store.journal.entries().unwrap().count(), 1);
        let recovered = reopen(&f).open().unwrap();
        if let Some(new) = new {
            assert_entry(&recovered, &new, b"new");
        } else {
            assert!(recovered.index().unwrap().items.is_empty());
            assert!(!exists(&recovered.root, &old.summary.item_path).unwrap());
        }
        assert_eq!(recovered.journal.entries().unwrap().count(), 0);
    }
}
#[test]
fn missing_tracked_file_and_added_symlink_cannot_be_removed_silently() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let origin = f.root.join("origin.txt");
    let (stage, old) = folder(&store, &origin, None, b"old");
    store.publish(stage, old.clone(), None, None).unwrap();
    let target = store
        .root
        .open_dir_nofollow(&old.summary.item_path)
        .unwrap();
    target.remove_file("first.txt").unwrap();
    let conflicts = store.conflicts(&old).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, "first.txt");
    assert_eq!(conflicts[0].current_hash, "missing");
    assert_eq!(
        store
            .remove(&old.summary.item_id, &old.summary.revision)
            .unwrap_err()
            .code,
        "library_conflict"
    );
    #[cfg(unix)]
    {
        std::fs::write(&origin, b"outside content").unwrap();
        std::os::unix::fs::symlink(
            &origin,
            store.path.join(&old.summary.item_path).join("outside"),
        )
        .unwrap();
        assert_eq!(store.conflicts(&old).unwrap_err().code, "library_conflict");
        assert_eq!(
            store
                .remove(&old.summary.item_id, &old.summary.revision)
                .unwrap_err()
                .code,
            "library_conflict"
        );
        assert_eq!(std::fs::read(&origin).unwrap(), b"outside content");
    }
    assert_eq!(target.read("second.txt").unwrap(), b"old");
}
#[test]
fn merge_publication_recovers_per_owned_entry_transition() {
    for (point, expect_new) in [
        ("journal", false),
        ("entries_backed_up", true),
        ("entry_published", true),
    ] {
        let f = fixture();
        let store = f.service.open().unwrap();
        let mut old = asset_entry(&asset(1, "old body"), None);
        let stage = store.stage_asset(&mut old, &asset(1, "old body")).unwrap();
        store.publish(stage, old.clone(), None, None).unwrap();
        let mut new = asset_entry(&asset(1, "new body"), Some(&old));
        let stage = store.stage_asset(&mut new, &asset(1, "new body")).unwrap();
        fault(&store, point);
        assert_eq!(
            store
                .publish(stage, new.clone(), Some(&old.summary.revision), None)
                .unwrap_err()
                .code,
            "library_test_crash"
        );
        let recovered = reopen(&f).open().unwrap();
        let current = recovered.index().unwrap().items.remove(0);
        if expect_new {
            assert_eq!(current.summary.revision, new.summary.revision);
            let dir = recovered.item_dir(&current.summary.item_path).unwrap();
            let name = current
                .summary
                .document_path
                .as_deref()
                .unwrap()
                .strip_prefix(&format!("{}/", current.summary.item_path))
                .unwrap();
            assert!(
                String::from_utf8(dir.read(name).unwrap())
                    .unwrap()
                    .contains("new body")
            );
        } else {
            assert_eq!(current.summary.revision, old.summary.revision);
            let dir = recovered.item_dir(&current.summary.item_path).unwrap();
            let name = current
                .summary
                .document_path
                .as_deref()
                .unwrap()
                .strip_prefix(&format!("{}/", current.summary.item_path))
                .unwrap();
            assert!(
                String::from_utf8(dir.read(name).unwrap())
                    .unwrap()
                    .contains("old body")
            );
        }
        assert_eq!(recovered.journal.entries().unwrap().count(), 0);
    }
}

#[test]
fn moving_parent_recovers_index_and_descendant_paths() {
    for (point, moved) in [("move_journal", false), ("move_renamed", true)] {
        let f = fixture();
        let store = f.service.open().unwrap();
        let mut parent_asset = asset(1, "parent body");
        parent_asset.title = "Parent".into();
        parent_asset.source.provider_id = "confluence".into();
        parent_asset.source.provider_instance = "https://acme.atlassian.net/wiki".into();
        parent_asset.source.resource_type = "page".into();
        parent_asset.source.canonical_id = "100".into();
        parent_asset.source_url = Some("https://acme.atlassian.net/wiki/pages/100".into());
        parent_asset.container = Some(crate::sources::SourceContainer {
            id: "SPACE".into(),
            label: "SPACE · Space".into(),
        });
        let mut parent = asset_entry(&parent_asset, None);
        let stage = store.stage_asset(&mut parent, &parent_asset).unwrap();
        store.publish(stage, parent.clone(), None, None).unwrap();

        let mut child_asset = asset(2, "child body");
        child_asset.title = "Child".into();
        child_asset.source.provider_id = "confluence".into();
        child_asset.source.provider_instance = "https://acme.atlassian.net/wiki".into();
        child_asset.source.resource_type = "page".into();
        child_asset.source.canonical_id = "101".into();
        child_asset.source_url = Some("https://acme.atlassian.net/wiki/pages/101".into());
        child_asset.container = parent_asset.container.clone();
        let mut child = asset_entry(&child_asset, None);
        child.summary.parent_item_id = Some(parent.summary.item_id.clone());
        child.summary.item_path = format!("{}/Child", parent.summary.item_path);
        child.summary.document_path = Some(format!("{}/Child.md", child.summary.item_path));
        let stage = store.stage_asset(&mut child, &child_asset).unwrap();
        store.publish(stage, child.clone(), None, None).unwrap();

        let mut renamed_asset = parent_asset.clone();
        renamed_asset.title = "Parent Renamed".into();
        let mut renamed = asset_entry(&renamed_asset, Some(&parent));
        let stage = store.stage_asset(&mut renamed, &renamed_asset).unwrap();
        fault(&store, point);
        assert_eq!(
            store
                .publish(stage, renamed.clone(), Some(&parent.summary.revision), None)
                .unwrap_err()
                .code,
            "library_test_crash"
        );
        let recovered = reopen(&f).open().unwrap();
        let items = recovered.index().unwrap().items;
        let saved_parent = items
            .iter()
            .find(|item| item.summary.item_id == parent.summary.item_id)
            .unwrap();
        let saved_child = items
            .iter()
            .find(|item| item.summary.item_id == child.summary.item_id)
            .unwrap();
        if moved {
            assert_eq!(saved_parent.summary.item_path, renamed.summary.item_path);
            assert_eq!(
                saved_child.summary.item_path,
                format!("{}/Child", renamed.summary.item_path)
            );
            assert_eq!(
                saved_child.summary.document_path.as_deref(),
                Some(format!("{}/Child/Child.md", renamed.summary.item_path).as_str())
            );
        } else {
            assert_eq!(saved_parent.summary.item_path, parent.summary.item_path);
            assert_eq!(saved_child.summary.item_path, child.summary.item_path);
            assert_eq!(
                saved_child.summary.document_path,
                child.summary.document_path
            );
        }
        assert_eq!(recovered.journal.entries().unwrap().count(), 0);
    }
}

fn publish_asset(
    store: &Arc<Store>,
    n: u32,
    body: &str,
    previous: Option<&LibraryIndexEntry>,
) -> LibraryIndexEntry {
    let mut entry = asset_entry(&asset(n, body), previous);
    let stage = store.stage_asset(&mut entry, &asset(n, body)).unwrap();
    if previous.is_none() {
        crate::library::refs::insert_ref(&mut entry.summary, LibraryItemRef::Manual);
    }
    store
        .publish(
            stage,
            entry.clone(),
            previous.map(|e| e.summary.revision.as_str()),
            None,
        )
        .unwrap();
    entry
}

#[test]
fn reference_added_between_snapshot_and_publish_survives_the_publish() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let first = publish_asset(&store, 1, "one", None);
    // The save prepared its entry from this snapshot ...
    let mut updated = asset_entry(&asset(1, "one changed"), Some(&first));
    let stage = store
        .stage_asset(&mut updated, &asset(1, "one changed"))
        .unwrap();
    // ... and a follow adopts the item before the publish lands.
    let follow = LibraryItemRef::Follow {
        follow_id: "follow:other".into(),
    };
    store
        .mutate_index(|index| {
            index.follows.push(LibraryFollowSummary {
                follow_id: "follow:other".into(),
                provider_id: "confluence".into(),
                provider_instance: "https://x.atlassian.net/wiki".into(),
                source: LibraryFollowSource::ConfluenceSpace {
                    space_key: "SD".into(),
                    space_name: "Software Development".into(),
                },
                include_attachments: false,
                item_count: 0,
                partial: None,
                excluded_ids: vec![],
                last_refreshed_at: None,
                state: LibraryItemState::Fresh,
                reference_depth: Some(0),
            });
            Ok(())
        })
        .unwrap();
    store
        .add_ref(&first.summary.item_id, follow.clone())
        .unwrap();
    store
        .publish(stage, updated, Some(&first.summary.revision), None)
        .unwrap();
    let saved = store.index().unwrap().items.remove(0).summary;
    assert_ne!(
        saved.revision, first.summary.revision,
        "the publish happened"
    );
    assert_eq!(saved.refs, [LibraryItemRef::Manual, follow]);
}

#[test]
fn depth_and_inclusion_written_between_snapshot_and_publish_survive_the_publish() {
    use cockpit_protocol::library::{LibraryInclusion, LibraryInclusionHolder};
    let f = fixture();
    let store = f.service.open().unwrap();
    let first = publish_asset(&store, 1, "one", None);
    let mut updated = asset_entry(&asset(1, "one changed"), Some(&first));
    let stage = store
        .stage_asset(&mut updated, &asset(1, "one changed"))
        .unwrap();
    // A depth change and an inclusion land after the save took its snapshot.
    let inclusion = LibraryInclusion {
        holder: LibraryInclusionHolder::Item {
            item_id: "source:seed".into(),
        },
        from_item_id: Some("source:seed".into()),
        from_label: "Seed".into(),
        relation: "body".into(),
        depth: 1,
    };
    store
        .mutate_index(|index| {
            let summary = &mut index.items[0].summary;
            summary.reference_depth = Some(2);
            crate::library::refs::set_inclusion(summary, inclusion.clone());
            Ok(())
        })
        .unwrap();
    store
        .publish(stage, updated, Some(&first.summary.revision), None)
        .unwrap();
    let saved = store.index().unwrap().items.remove(0).summary;
    assert_ne!(
        saved.revision, first.summary.revision,
        "the publish happened"
    );
    assert_eq!(
        (saved.reference_depth, saved.included_by),
        (Some(2), Some(vec![inclusion]))
    );
}

#[test]
fn dropped_item_lease_releases_lock_with_duplicated_descriptor_alive() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let id = "reserved-item";
    let held = store.lease(id).unwrap();
    // Model the descriptor inherited between a concurrent fork and exec.
    let inherited = held._file.try_clone().unwrap();
    assert_eq!(store.lease(id).err().unwrap().code, "library_item_busy");
    drop(held);
    let replacement = store.lease(id).unwrap();
    drop(inherited);
    assert_eq!(store.lease(id).err().unwrap().code, "library_item_busy");
    drop(replacement);
    assert!(store.lease(id).is_ok());
}

#[test]
fn dropped_library_leases_release_lock_with_duplicated_descriptor_alive() {
    let f = fixture();
    let store = f.service.open().unwrap();
    for shared in [true, false] {
        let held = if shared {
            store.shared().unwrap()
        } else {
            store.exclusive().unwrap()
        };
        let inherited = held._file.try_clone().unwrap();
        assert!(store.try_exclusive().unwrap().is_none());
        drop(held);
        let replacement = store.try_exclusive().unwrap().unwrap();
        drop(inherited);
        assert!(store.try_exclusive().unwrap().is_none());
        drop(replacement);
    }
}

#[test]
fn independent_shared_lease_survives_another_lease_drop() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let first = store.shared().unwrap();
    let second = store.shared().unwrap();
    let inherited = first._file.try_clone().unwrap();
    drop(first);
    assert!(store.try_exclusive().unwrap().is_none());
    drop(second);
    let exclusive = store.try_exclusive().unwrap().unwrap();
    drop(inherited);
    assert!(store.try_exclusive().unwrap().is_none());
    drop(exclusive);
    assert!(store.try_exclusive().unwrap().is_some());
}

#[test]
fn exclusive_try_lock_reports_contention_without_waiting() {
    let f = fixture();
    let store = f.service.open().unwrap();
    let held = store.exclusive().unwrap();
    assert!(store.try_exclusive().unwrap().is_none());
    drop(held);
    assert!(store.try_exclusive().unwrap().is_some());
}
