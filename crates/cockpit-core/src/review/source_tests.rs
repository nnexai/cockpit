use super::super::test_support::*;
use super::*;
use cockpit_protocol::review::ReviewFileStatus;

#[test]
fn tracked_viewer_diff_runs_on_a_standard_worker_stack() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("review worker runtime");
    runtime.block_on(async {
        tokio::spawn(async {
            let fixture = service_fixture("worker-stack").await;
            std::fs::write(fixture.checkout.join("tracked.txt"), "base\nchanged\n")
                .expect("modify tracked file");
            let snapshot = fixture.snapshot(ReviewComparison::Unstaged).await;
            let file = snapshot
                .files
                .iter()
                .find(|file| file.new_path.as_deref() == Some("tracked.txt"))
                .expect("tracked change");
            let diff = fixture.file(&snapshot, &file.file_id).await;
            assert_eq!(diff.old_source.as_deref(), Some("base\n"));
            assert_eq!(diff.new_source.as_deref(), Some("base\nchanged\n"));
            std::fs::remove_dir_all(&fixture.workspace).expect("cleanup");
        })
        .await
        .expect("worker completes tracked Review");
    });
}

#[test]
fn frozen_worktree_source_preserves_physical_lf_lines_and_raw_cr_data() {
    let root = fixture("frozen-source");
    std::fs::write(root.join("note.txt"), "first\rsecond\r\nthird").expect("write source");
    let source = read_worktree_source(&root, "note.txt").expect("read source");
    assert_eq!(source.text.as_deref(), Some("first\rsecond\r\nthird"));
    assert_eq!(source.total_lines, Some(2));
    assert_eq!(
        source.hash.as_deref(),
        Some("sha256:75c542c465e6b79ba0a1a3bc586de68b17c112285d6828f82e1dbeab9fff38f0")
    );
}

#[test]
fn worktree_source_cursor_rejects_a_changed_file_between_pages() {
    let root = fixture("source-cursor");
    std::fs::write(root.join("large.txt"), vec![b'x'; MAX_FILE_BYTES + 17]).expect("write source");
    let first = read_worktree_source_page(&root, "large.txt", 0).expect("first page");
    let first_identity = first.identity.clone().expect("cursor");
    std::fs::write(root.join("large.txt"), vec![b'y'; MAX_FILE_BYTES + 17]).expect("mutate source");
    let second_identity = worktree_source_identity(&root, "large.txt")
        .transpose()
        .expect("read cursor")
        .expect("mutated cursor");
    assert_ne!(first_identity, second_identity);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn large_worktree_sources_are_available_as_bounded_continuation_pages() {
    let root = fixture("paged-source");
    let bytes = vec![b'x'; MAX_FILE_BYTES + 17];
    std::fs::write(root.join("large.txt"), &bytes).expect("write large source");
    let first = read_worktree_source_page(&root, "large.txt", 0).expect("first page");
    assert_eq!(first.text.as_ref().map(String::len), Some(MAX_FILE_BYTES));
    assert_eq!(first.total_bytes, Some((MAX_FILE_BYTES + 17) as u32));
    assert!(first.truncated);
    let second = read_worktree_source_page(&root, "large.txt", MAX_FILE_BYTES as u32)
        .expect("continuation page");
    assert_eq!(second.text.as_deref(), Some("xxxxxxxxxxxxxxxxx"));
    assert!(!second.truncated);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn real_collect_keeps_staged_and_unstaged_sources_distinct_without_git_mutation() {
    let root = fixture("real-partial");
    std::fs::write(root.join("note.txt"), "base\n").expect("write base");
    commit(&root, "base");
    std::fs::write(root.join("note.txt"), "staged\n").expect("write staged");
    git_bytes(&root, &["add", "--", "note.txt"]);
    std::fs::write(root.join("note.txt"), "worktree\n").expect("write worktree");
    let before_index = git_bytes(&root, &["ls-files", "--stage", "-z"]);
    let before_status = git_bytes(&root, &["status", "--porcelain=v1", "-z"]);
    let before_objects = git_bytes(&root, &["count-objects", "-v"]);

    let diffs = collect_real(&service(&root), &root, ReviewComparison::AllLocal, None).await;
    let staged = diffs
        .values()
        .find(|diff| diff.file.comparison == ReviewComparison::Staged)
        .expect("staged diff");
    let unstaged = diffs
        .values()
        .find(|diff| diff.file.comparison == ReviewComparison::Unstaged)
        .expect("unstaged diff");
    assert_ne!(staged.file.file_id, unstaged.file.file_id);
    assert_eq!(staged.old_source.as_deref(), Some("base\n"));
    assert_eq!(staged.new_source.as_deref(), Some("staged\n"));
    assert_eq!(unstaged.old_source.as_deref(), Some("staged\n"));
    assert_eq!(unstaged.new_source.as_deref(), Some("worktree\n"));
    assert_eq!(
        git_bytes(&root, &["ls-files", "--stage", "-z"]),
        before_index
    );
    assert_eq!(
        git_bytes(&root, &["status", "--porcelain=v1", "-z"]),
        before_status
    );
    assert_eq!(git_bytes(&root, &["count-objects", "-v"]), before_objects);
}

#[tokio::test]
async fn real_collect_freezes_branch_sources_and_hashes_despite_worktree_edits() {
    let root = fixture("real-branch");
    std::fs::write(root.join("review.txt"), "base\n").expect("write base");
    commit(&root, "base");
    std::fs::write(root.join("review.txt"), "committed review\n").expect("write reviewed commit");
    commit(&root, "reviewed");
    let service = service(&root);
    let diffs = collect_real(&service, &root, ReviewComparison::Branch, Some("HEAD~1")).await;
    let diff = diffs.values().next().expect("branch diff");
    let frozen_hash = diff.new_source_hash.clone().expect("new source hash");
    assert_eq!(diff.old_source.as_deref(), Some("base\n"));
    assert_eq!(diff.new_source.as_deref(), Some("committed review\n"));

    std::fs::write(root.join("review.txt"), "uncommitted\n").expect("edit worktree");
    let head = service
        .git_text(&root, &["rev-parse", "--verify", "HEAD"])
        .await
        .expect("head");
    let source = service
        .git_source(&root, &head, "review.txt")
        .await
        .expect("frozen head source");
    assert_eq!(source.text.as_deref(), Some("committed review\n"));
    assert_eq!(source.hash.as_deref(), Some(frozen_hash.as_str()));
}

#[cfg(unix)]
#[tokio::test]
async fn real_collect_preserves_binary_unreadable_and_bounded_untracked_rows() {
    let root = fixture("real-special");
    std::fs::write(root.join("literal.txt"), "ordinary\n").expect("write literal");
    std::fs::write(root.join("binary.dat"), b"before\0").expect("write binary");
    commit(&root, "base");
    std::fs::write(
        root.join("literal.txt"),
        "Binary files are ordinary text\nSubproject commit also ordinary text\n",
    )
    .expect("write literal phrases");
    std::fs::write(root.join("binary.dat"), b"after\0").expect("change binary");
    std::fs::File::create(root.join("large.txt"))
        .expect("large file")
        .set_len((MAX_FILE_BYTES + 1) as u64)
        .expect("size large file");
    std::os::unix::fs::symlink("/outside-review-fixture", root.join("outside-link"))
        .expect("create untracked link");

    let diffs = collect_real(&service(&root), &root, ReviewComparison::AllLocal, None).await;
    let literal = diffs
        .values()
        .find(|diff| diff.file.new_path.as_deref() == Some("literal.txt"))
        .expect("literal row");
    assert_eq!(literal.file.status, ReviewFileStatus::Modified);
    assert!(!literal.file.binary && !literal.hunks.is_empty());
    let binary = diffs
        .values()
        .find(|diff| diff.file.new_path.as_deref() == Some("binary.dat"))
        .expect("binary row");
    assert_eq!(binary.file.status, ReviewFileStatus::Binary);
    let large = diffs
        .values()
        .find(|diff| diff.file.new_path.as_deref() == Some("large.txt"))
        .expect("large row");
    assert!(large.truncated && large.new_source_truncated);
    let link = diffs
        .values()
        .find(|diff| diff.file.new_path.as_deref() == Some("outside-link"))
        .expect("link row");
    assert_eq!(link.file.status, ReviewFileStatus::Unreadable);
    assert!(!link.truncated);
    assert!(link.new_source.is_none());
    assert!(!link.diagnostics.is_empty());
}
