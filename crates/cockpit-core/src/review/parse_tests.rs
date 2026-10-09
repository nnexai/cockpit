use super::super::git::worktree_token;
use super::super::snapshot::file_id;
use super::super::test_support::*;
use super::*;

#[test]
fn fixture_preserves_partially_staged_file_as_two_distinct_anchor_scopes() {
    let root = fixture("partial");
    std::fs::write(root.join("note.txt"), "one\ntwo\n").expect("write base");
    git_bytes(&root, &["add", "--", "note.txt"]);
    git_bytes(&root, &["commit", "-m", "base"]);
    std::fs::write(root.join("note.txt"), "one\nstaged\n").expect("write staged");
    git_bytes(&root, &["add", "--", "note.txt"]);
    std::fs::write(root.join("note.txt"), "one\nstaged\nunstaged\n").expect("write unstaged");

    let staged = parse_name_status(
        &git_bytes(&root, &["diff", "--cached", "--name-status", "-z", "--"]),
        ReviewComparison::Staged,
    )
    .expect("staged status");
    let unstaged = parse_name_status(
        &git_bytes(&root, &["diff", "--name-status", "-z", "--"]),
        ReviewComparison::Unstaged,
    )
    .expect("unstaged status");
    assert_eq!(staged.len(), 1);
    assert_eq!(unstaged.len(), 1);
    assert_eq!(staged[0].new_path.as_deref(), Some("note.txt"));
    assert_eq!(unstaged[0].new_path.as_deref(), Some("note.txt"));
    assert_ne!(
        file_id(
            staged[0].comparison,
            &staged[0].old_path,
            &staged[0].new_path
        ),
        file_id(
            unstaged[0].comparison,
            &unstaged[0].old_path,
            &unstaged[0].new_path
        )
    );
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn unified_parser_keeps_canonical_old_and_new_line_numbers() {
    let change = Change {
        status: ReviewFileStatus::Modified,
        comparison: ReviewComparison::Unstaged,
        old_path: Some("old.txt".to_owned()),
        new_path: Some("new.txt".to_owned()),
        old_revision: None,
        new_revision: None,
    };
    let (hunks, binary, truncated) =
        parse_unified("@@ -2,2 +2,3 @@\n same\n-old\n+new\n+tail\n", &change, 8)
            .expect("parse hunk");
    assert!(!binary && !truncated);
    assert_eq!(hunks[0].lines[0].old_line, Some(2));
    assert_eq!(hunks[0].lines[0].new_line, Some(2));
    assert_eq!(hunks[0].lines[1].old_line, Some(3));
    assert_eq!(hunks[0].lines[1].new_line, None);
    assert_eq!(hunks[0].lines[2].old_line, None);
    assert_eq!(hunks[0].lines[2].new_line, Some(3));
    assert_eq!(hunks[0].lines[3].new_line, Some(4));
    assert_eq!(
        change_counts(&hunks, binary, truncated, ReviewFileStatus::Modified),
        (Some(2), Some(1))
    );
    assert_eq!(
        change_counts(&hunks, false, true, ReviewFileStatus::Modified),
        (None, None)
    );
    assert_eq!(
        change_counts(&hunks, true, false, ReviewFileStatus::Modified),
        (None, None)
    );
    assert_eq!(
        change_counts(&hunks, false, false, ReviewFileStatus::ModeOnly),
        (None, None)
    );
}

#[test]
fn oversized_untracked_sources_stay_bounded_and_file_ids_are_transport_safe() {
    let root = fixture("bounded-untracked");
    std::fs::write(root.join("small.txt"), "one\ntwo\n").expect("write small source");
    let (small_hunks, binary, truncated) = untracked_hunk(&root, "small.txt").unwrap();
    assert_eq!(
        change_counts(&small_hunks, binary, truncated, ReviewFileStatus::Untracked),
        (Some(2), Some(0))
    );
    let file = std::fs::File::create(root.join("huge.bin")).unwrap();
    file.set_len(8 * 1024 * 1024 * 1024).unwrap();
    assert!(worktree_token(&root, b"", b"small.txt\0huge.bin\0").is_ok());
    let (hunks, _, truncated) = untracked_hunk(&root, "huge.bin").unwrap();
    assert!(hunks.is_empty() && truncated);
    assert_eq!(
        change_counts(&hunks, false, truncated, ReviewFileStatus::Untracked),
        (None, None)
    );
    let id = file_id(
        ReviewComparison::Unstaged,
        &Some("a\nb".into()),
        &Some("-new.txt".into()),
    );
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    );
    std::fs::remove_dir_all(root).unwrap();
}
