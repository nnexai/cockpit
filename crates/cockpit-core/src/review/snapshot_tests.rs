use super::super::parse::parse_numstat;
use super::super::test_support::*;
use super::*;
use crate::repositories::RepositoryCatalog;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn inventory_includes_line_counts_before_opening_files() {
    let root = fixture("inventory-counts");
    std::fs::write(root.join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
    git_bytes(&root, &["add", "tracked.txt"]);
    git_bytes(&root, &["commit", "-m", "count baseline"]);
    std::fs::write(root.join("tracked.txt"), "one\nreplacement\nthree\nfour\n").unwrap();
    std::fs::write(root.join("new.txt"), "new\nfile\n").unwrap();
    let service = service(&root);
    let revisions = service.revision_tokens(&root).await.unwrap();
    let (files, _, _, _, _) = service
        .collect(
            &root,
            &ReviewSnapshotRequest {
                binding_id: "binding".into(),
                repository_id: "repository".into(),
                comparison: ReviewComparison::AllLocal,
                base_ref: None,
            },
            &revisions,
            None,
        )
        .await
        .unwrap();
    let tracked = files
        .iter()
        .find(|file| file.new_path.as_deref() == Some("tracked.txt"))
        .unwrap();
    assert_eq!((tracked.additions, tracked.deletions), (Some(2), Some(1)));
    let new = files
        .iter()
        .find(|file| file.new_path.as_deref() == Some("new.txt"))
        .unwrap();
    assert_eq!((new.additions, new.deletions), (Some(2), Some(0)));
    let renamed = parse_numstat(b"2\t1\t\0old\tname\0new\nname\0-\t-\tbinary\0").unwrap();
    assert_eq!(renamed["new\nname"].additions, Some(2));
    assert!(renamed["binary"].binary);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn resolves_an_unconfigured_pane_checkout_by_its_opaque_identity() {
    let root = fixture("unconfigured-checkout");
    let pane_cwd = root.join("src");
    std::fs::create_dir(&pane_cwd).expect("pane directory");
    let configured = configuration(&root);
    let repository_id = RepositoryCatalog::new(configured.clone())
        .list()
        .await
        .expect("catalog listing")
        .repositories
        .into_iter()
        .next()
        .expect("fixture checkout")
        .repository_id;
    let mut unconfigured = configured;
    unconfigured.repository_roots.clear();
    let service = service_with_configuration(unconfigured);

    let resolved = service
        .resolve_checkout(
            &Some(pane_cwd.to_string_lossy().into_owned()),
            &None,
            &repository_id,
        )
        .await
        .expect("unconfigured checkout resolves from pane cwd");

    assert_eq!(resolved.repository_id, repository_id);
    assert_eq!(resolved.checkout_path, root.to_string_lossy());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn review_remains_authorized_after_source_closes_and_rejects_retired_binding() {
    let fixture = service_fixture("viewer-authorization").await;
    fixture.adapter.source_closed.store(true, Ordering::Relaxed);
    let snapshot = fixture.snapshot(ReviewComparison::AllLocal).await;
    assert_eq!(snapshot.viewer_id, fixture.viewer_id);
    let evidence = fixture
        .service
        .comment_evidence(FIXTURE_SESSION, &fixture.viewer_id, &fixture.binding_id)
        .await
        .expect("pinned source comments remain authorized");
    assert_eq!(
        evidence.source_id,
        checkout_source_id(&fixture.checkout).expect("checkout identity")
    );
    fixture
        .viewers
        .release(FIXTURE_SESSION, &fixture.viewer_id)
        .await
        .expect("release");
    assert_eq!(
        fixture
            .service
            .snapshot(
                FIXTURE_SESSION,
                &fixture.viewer_id,
                &fixture.request(ReviewComparison::AllLocal)
            )
            .await
            .expect_err("released viewer cannot read")
            .code,
        "viewer_not_found"
    );
    std::fs::remove_dir_all(fixture.workspace).expect("cleanup");
}

#[tokio::test]
async fn review_rejects_a_replaced_root_and_a_changed_binding() {
    let fixture = service_fixture("root-authorization").await;
    let error = fixture
        .service
        .snapshot(
            FIXTURE_SESSION,
            &fixture.viewer_id,
            &ReviewSnapshotRequest {
                binding_id: "retired".to_owned(),
                ..fixture.request(ReviewComparison::AllLocal)
            },
        )
        .await
        .expect_err("binding cannot be guessed or reused");
    assert_eq!(error.code, "context_stale_binding");
    std::fs::rename(&fixture.checkout, fixture.workspace.join("old-repo")).expect("replace root");
    fixture_at(&fixture.checkout);
    let error = fixture
        .service
        .snapshot(
            FIXTURE_SESSION,
            &fixture.viewer_id,
            &fixture.request(ReviewComparison::AllLocal),
        )
        .await
        .expect_err("replaced directory inode cannot inherit access");
    assert_eq!(error.code, "context_root_not_authorized");
    std::fs::remove_dir_all(fixture.workspace).expect("cleanup");
}

#[tokio::test]
async fn regression_snapshot_rebuilds_an_untracked_file_rewritten_in_place_with_same_size_and_mtime()
 {
    let fixture = service_fixture("untracked-same-metadata").await;
    let path = fixture.checkout.join("draft.txt");
    std::fs::write(&path, "BEFORE\n").expect("write untracked before");

    let first = fixture.snapshot(ReviewComparison::AllLocal).await;
    let first_diff = fixture
        .file(&first, &untracked_file_id(&first, "draft.txt"))
        .await;
    let first_text = diff_text(&first_diff);
    assert!(first_text.contains("BEFORE"), "{first_text}");
    assert!(!first_text.contains("AFTER!"), "{first_text}");

    let before = std::fs::metadata(&path).expect("metadata before rewrite");
    let original_mtime = before.modified().expect("mtime before rewrite");
    std::fs::write(&path, "AFTER!\n").expect("rewrite untracked in place");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open rewritten file")
        .set_modified(original_mtime)
        .expect("restore original mtime");
    let after = std::fs::metadata(&path).expect("metadata after rewrite");
    assert_eq!(after.len(), before.len(), "same byte length");
    assert_eq!(after.modified().expect("mtime after"), original_mtime);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(after.ino(), before.ino(), "same inode");
    }

    let second = fixture.snapshot(ReviewComparison::AllLocal).await;
    assert_ne!(second.review_id, first.review_id);
    let second_diff = fixture
        .file(&second, &untracked_file_id(&second, "draft.txt"))
        .await;
    let second_text = diff_text(&second_diff);
    assert!(second_text.contains("AFTER!"), "{second_text}");
    assert!(!second_text.contains("BEFORE"), "{second_text}");
    std::fs::remove_dir_all(&fixture.workspace).expect("cleanup");
}

#[tokio::test]
async fn regression_snapshot_reuses_an_unchanged_tracked_only_review() {
    let fixture = service_fixture("tracked-cache-reuse").await;
    std::fs::write(fixture.checkout.join("tracked.txt"), "base\nchanged\n")
        .expect("modify tracked file");

    let first = fixture.snapshot(ReviewComparison::AllLocal).await;
    assert!(
        first
            .files
            .iter()
            .any(|file| file.new_path.as_deref() == Some("tracked.txt"))
    );
    assert!(
        first
            .files
            .iter()
            .all(|file| file.status != ReviewFileStatus::Untracked)
    );
    let second = fixture.snapshot(ReviewComparison::AllLocal).await;
    assert_eq!(second.review_id, first.review_id);
    assert_eq!(second.worktree_revision, first.worktree_revision);

    std::fs::write(
        fixture.checkout.join("tracked.txt"),
        "base\nchanged again\n",
    )
    .expect("modify tracked file again");
    let third = fixture.snapshot(ReviewComparison::AllLocal).await;
    assert_ne!(third.review_id, first.review_id);
    std::fs::remove_dir_all(&fixture.workspace).expect("cleanup");
}

#[tokio::test]
async fn discovers_a_linked_worktree_from_a_subdirectory_with_its_catalog_identity() {
    let workspace =
        std::env::temp_dir().join(format!("cockpit-review-worktrees-{}", Uuid::new_v4()));
    let primary = workspace.join("primary");
    fixture_at(&primary);
    std::fs::write(primary.join("base.txt"), "base\n").expect("write base");
    commit(&primary, "base");
    let linked = workspace.join("linked");
    let linked_text = linked.to_string_lossy().into_owned();
    git_bytes(
        &primary,
        &["worktree", "add", "-b", "linked-review", &linked_text],
    );
    let pane_cwd = linked.join("src");
    std::fs::create_dir(&pane_cwd).expect("linked pane directory");

    let mut configuration = configuration(&workspace);
    configuration.repository_roots = vec![workspace.to_string_lossy().into_owned()];
    let catalog = RepositoryCatalog::new(configuration);
    let listed = catalog.list().await.expect("catalog listing");
    let linked_candidate = listed
        .repositories
        .into_iter()
        .find(|candidate| candidate.checkout_path == linked_text)
        .expect("linked worktree in catalog");
    let discovered = catalog
        .discover_checkout(&pane_cwd)
        .await
        .expect("linked worktree from pane cwd");

    assert_eq!(discovered.repository_id, linked_candidate.repository_id);
    assert_eq!(discovered.checkout_path, linked_text);
    assert!(discovered.is_linked_worktree);
    std::fs::remove_dir_all(workspace).expect("cleanup");
}

#[tokio::test]
async fn collect_keeps_an_untracked_inventory_beyond_the_old_row_limit() {
    let root = fixture("untracked-collection");
    for index in 0..257 {
        std::fs::write(root.join(format!("file-{index}.txt")), "change\n")
            .expect("write untracked file");
    }
    let service = service(&root);
    let request = ReviewSnapshotRequest {
        binding_id: "test-binding".to_owned(),
        repository_id: "test-repository".to_owned(),
        comparison: ReviewComparison::Untracked,
        base_ref: None,
    };
    let revisions = service
        .revision_tokens(&root)
        .await
        .expect("revision tokens");
    let (files, changes, diagnostics, truncated, _) = service
        .collect(&root, &request, &revisions, None)
        .await
        .expect("review inventory");
    assert_eq!(files.len(), 257);
    assert_eq!(changes.len(), 257);
    assert!(diagnostics.is_empty());
    assert!(!truncated);
    std::fs::remove_dir_all(root).expect("cleanup");
}
