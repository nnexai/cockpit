use super::super::test_support::*;
use super::*;

#[test]
fn worktree_token_changes_when_an_untracked_file_bytes_change() {
    let root = fixture("untracked-token");
    std::fs::write(root.join("draft.txt"), "first").expect("write first source");
    let first = worktree_token(&root, b"", b"draft.txt\0").expect("first token");
    std::fs::write(root.join("draft.txt"), "other").expect("write second source");
    let second = worktree_token(&root, b"", b"draft.txt\0").expect("second token");
    assert_ne!(first, second);
}

#[test]
fn worktree_token_covers_untracked_inventory_beyond_the_old_row_limit() {
    let root = fixture("untracked-inventory");
    let paths = (0..257)
        .map(|index| format!("missing-{index}.txt"))
        .collect::<Vec<_>>()
        .join("\0");
    let paths = format!("{paths}\0");
    assert!(worktree_token(&root, b"", paths.as_bytes()).is_ok());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn git_source_pages_stream_large_blob_sides() {
    let root = fixture("git-paged-source");
    let mut bytes = vec![b'x'; MAX_FILE_BYTES];
    bytes.extend_from_slice(b"continuation-tail");
    std::fs::write(root.join("large.txt"), bytes).expect("write source");
    commit(&root, "large source");
    let head = String::from_utf8(git_bytes(&root, &["rev-parse", "HEAD"]))
        .expect("head text")
        .trim()
        .to_owned();
    let service = service(&root);
    let first = service
        .git_source_page(&root, &head, "large.txt", 0)
        .await
        .expect("first Git page");
    assert_eq!(first.text.as_deref().map(str::len), Some(MAX_FILE_BYTES));
    assert!(first.truncated);
    assert_eq!(first.total_bytes, Some((MAX_FILE_BYTES + 17) as u32));
    let second = service
        .git_source_page(&root, &head, "large.txt", MAX_FILE_BYTES as u32)
        .await
        .expect("second Git page");
    assert_eq!(second.text.as_deref(), Some("continuation-tail"));
    assert!(!second.truncated);
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn git_source_pages_resume_at_utf8_boundary() {
    let root = fixture("git-paged-unicode");
    let mut bytes = vec![b'x'; MAX_FILE_BYTES - 1];
    bytes.extend_from_slice("é-tail".as_bytes());
    std::fs::write(root.join("unicode.txt"), bytes).expect("write source");
    commit(&root, "unicode source");
    let head = String::from_utf8(git_bytes(&root, &["rev-parse", "HEAD"]))
        .expect("head text")
        .trim()
        .to_owned();
    let service = service(&root);
    let first = service
        .git_source_page(&root, &head, "unicode.txt", 0)
        .await
        .expect("first Git page");
    assert_eq!(
        first.text.as_ref().map(String::len),
        Some(MAX_FILE_BYTES - 1)
    );
    assert!(first.truncated);
    let second = service
        .git_source_page(&root, &head, "unicode.txt", (MAX_FILE_BYTES - 1) as u32)
        .await
        .expect("continuation Git page");
    assert_eq!(second.text.as_deref(), Some("é-tail"));
    assert!(!second.truncated);
    std::fs::remove_dir_all(root).expect("cleanup");
}
