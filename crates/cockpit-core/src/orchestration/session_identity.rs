use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
};

use crate::InspectionError;

const SESSION_HEADER_BYTES: usize = 64 * 1024;

/// Read the first OMP session header from a bounded journal prefix.
///
/// The header ID is opaque: neither its spelling nor the journal filename
/// establishes an identity beyond the printable, nonempty 512-byte contract.
pub fn read_omp_session_header(input: impl Read) -> Result<String, InspectionError> {
    #[derive(serde::Deserialize)]
    struct Header {
        #[serde(rename = "type")]
        kind: String,
        id: Option<String>,
    }
    let mut input = BufReader::new(input.take(SESSION_HEADER_BYTES as u64));
    let mut line = Vec::new();
    let mut total = 0;
    loop {
        line.clear();
        let count = input.read_until(b'\n', &mut line)
            .map_err(|_| InspectionError::new("session_header_invalid", "Cannot read session header"))?;
        if count == 0 { break; }
        total += count;
        if total == SESSION_HEADER_BYTES && line.last() != Some(&b'\n') {
            return Err(InspectionError::new("session_header_invalid", "No session header within 64 KiB"));
        }
        if line.iter().all(u8::is_ascii_whitespace) { continue; }
        let header: Header = serde_json::from_slice(&line)
            .map_err(|_| InspectionError::new("session_header_invalid", "Malformed session header prefix"))?;
        if header.kind != "session" { continue; }
        let id = header.id.ok_or_else(|| InspectionError::new("session_header_invalid", "Session header has no ID"))?;
        if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
            return Err(InspectionError::new("conversation_invalid", "Conversation identity must be printable, nonempty and at most 512 bytes"));
        }
        return Ok(id);
    }
    Err(InspectionError::new("session_header_invalid", "Session file contains no session header"))
}

/// Read an OMP journal without following a leaf symlink or opening a FIFO in
/// blocking mode. The opened regular file must match the inspected leaf.
pub fn read_omp_session_file(path: &Path) -> Result<String, InspectionError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

        let leaf = std::fs::symlink_metadata(path)
            .map_err(|_| InspectionError::new("session_header_invalid", "Cannot inspect session file"))?;
        if !leaf.is_file() {
            return Err(InspectionError::new("session_header_invalid", "Session file must be a nonsymlink regular file"));
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| InspectionError::new("session_header_invalid", "Cannot open session file"))?;
        let opened = file.metadata()
            .map_err(|_| InspectionError::new("session_header_invalid", "Cannot inspect opened session file"))?;
        if !opened.is_file() || opened.dev() != leaf.dev() || opened.ino() != leaf.ino() {
            return Err(InspectionError::new("session_header_invalid", "Session file changed while opening"));
        }
        read_omp_session_header(file)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(InspectionError::new("session_header_invalid", "Safe session file inspection is unsupported on this platform"))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn header_discovery_accepts_title_prefix_and_stops_before_transcript() {
        let prefix = b"{\"type\":\"title\",\"title\":\"PRIVATE\"}\n{\"type\":\"session\",\"version\":3,\"id\":\"actual opaque ID\"}\nnot a transcript parser";
        assert_eq!(read_omp_session_header(Cursor::new(prefix)).unwrap(), "actual opaque ID");
    }

    #[test]
    fn header_discovery_rejects_malformed_missing_and_over_limit_prefixes() {
        for prefix in [b"not json\n".as_slice(), b"{\"type\":\"title\"}\n", b"{\"type\":\"session\"}\n",
            b"{\"type\":\"session\",\"id\":\"\"}\n", b"{\"type\":\"session\",\"id\":\"bad\\nID\"}\n"] {
            assert!(read_omp_session_header(Cursor::new(prefix)).is_err());
        }
        let header = b"{\"type\":\"session\",\"id\":\"edge\"}\n";
        let mut within = vec![b' '; SESSION_HEADER_BYTES - header.len()];
        within.extend_from_slice(header);
        assert_eq!(read_omp_session_header(Cursor::new(within)).unwrap(), "edge");
        let mut outside = vec![b' '; SESSION_HEADER_BYTES];
        outside.extend_from_slice(header);
        assert_eq!(read_omp_session_header(Cursor::new(outside)).unwrap_err().code, "session_header_invalid");
    }

    #[test]
    fn header_limit_requires_a_complete_line_at_the_boundary() {
        let header = b"{\"type\":\"session\",\"id\":\"edge\"}";
        let mut exact_without_newline = vec![b' '; SESSION_HEADER_BYTES - header.len()];
        exact_without_newline.extend_from_slice(header);
        assert_eq!(read_omp_session_header(Cursor::new(exact_without_newline)).unwrap_err().code,
            "session_header_invalid");
        let mut one_byte_outside = vec![b' '; SESSION_HEADER_BYTES + 1 - header.len() - 1];
        one_byte_outside.extend_from_slice(header);
        one_byte_outside.push(b'\n');
        assert_eq!(read_omp_session_header(Cursor::new(one_byte_outside)).unwrap_err().code,
            "session_header_invalid");
        assert_eq!(read_omp_session_header(Cursor::new(header)).unwrap(), "edge");
    }

    #[test]
    fn header_id_is_opaque_untrimmed_and_bounded_by_utf8_bytes() {
        for id in [" actual opaque/ID ".to_owned(), "-leading-dash".to_owned(), "é".repeat(256)] {
            let header = serde_json::to_vec(&serde_json::json!({"type":"session", "id":id})).unwrap();
            assert_eq!(read_omp_session_header(Cursor::new(header)).unwrap(), id);
        }
        for id in [String::new(), "a".repeat(513), "é".repeat(257), "bad\0ID".into(), "bad\u{7f}ID".into()] {
            let header = serde_json::to_vec(&serde_json::json!({"type":"session", "id":id})).unwrap();
            assert_eq!(read_omp_session_header(Cursor::new(header)).unwrap_err().code, "conversation_invalid");
        }
    }

    #[test]
    fn malformed_missing_and_read_failures_keep_header_error_code() {
        for prefix in [b"".as_slice(), b" \n\t", b"PRIVATE malformed\n", b"{\"type\":\"session\",\"id\":null}\n",
            b"{\"type\":\"session\",\"id\":7}\n", b"{\"type\":\"session\",\"id\":\"\xff\"}\n"] {
            let error = read_omp_session_header(Cursor::new(prefix)).unwrap_err();
            assert_eq!(error.code, "session_header_invalid");
            assert!(!error.message.contains("PRIVATE"));
        }
        struct FailingRead;
        impl Read for FailingRead {
            fn read(&mut self, _bytes: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("PRIVATE read failure"))
            }
        }
        let error = read_omp_session_header(FailingRead).unwrap_err();
        assert_eq!(error.code, "session_header_invalid");
        assert!(!error.message.contains("PRIVATE"));
    }

    #[test]
    fn first_session_header_is_authoritative_and_large_body_is_ignored() {
        let mut journal = b"\n{\"type\":\"session\",\"id\":\"first\"}\n{\"type\":\"session\",\"id\":\"second\"}\n".to_vec();
        journal.resize(SESSION_HEADER_BYTES * 4, b'x');
        assert_eq!(read_omp_session_header(Cursor::new(journal)).unwrap(), "first");
        let invalid_first = b"{\"type\":\"session\",\"id\":\"\"}\n{\"type\":\"session\",\"id\":\"second\"}\n";
        assert_eq!(read_omp_session_header(Cursor::new(invalid_first)).unwrap_err().code, "conversation_invalid");
    }

    #[cfg(unix)]
    struct TestDirectory(std::path::PathBuf);

    #[cfg(unix)]
    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cockpit-session-header-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    #[cfg(unix)]
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_reader_uses_header_not_filename_and_ignores_large_body() {
        let root = TestDirectory::new();
        let file = root.0.join("misleading-id.jsonl");
        let mut journal = b"{\"type\":\"session\",\"id\":\"actual opaque ID\"}\n".to_vec();
        journal.resize(SESSION_HEADER_BYTES * 4, b'x');
        std::fs::write(&file, journal).unwrap();
        assert_eq!(read_omp_session_file(&file).unwrap(), "actual opaque ID");
    }

    #[cfg(unix)]
    #[test]
    fn file_reader_rejects_absent_directory_and_leaf_symlink() {
        let root = TestDirectory::new();
        let file = root.0.join("valid.jsonl");
        std::fs::write(&file, b"{\"type\":\"session\",\"id\":\"actual\"}\n").unwrap();
        let link = root.0.join("link.jsonl");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        let dangling = root.0.join("dangling.jsonl");
        std::os::unix::fs::symlink(root.0.join("absent"), &dangling).unwrap();
        for path in [&root.0, &root.0.join("absent"), &link, &dangling] {
            assert_eq!(read_omp_session_file(path).unwrap_err().code, "session_header_invalid");
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "bounded FIFO test child"]
    fn fifo_reader_child() {
        let path = std::env::var_os("COCKPIT_TEST_SESSION_FIFO").unwrap();
        assert_eq!(read_omp_session_file(Path::new(&path)).unwrap_err().code, "session_header_invalid");
    }

    #[cfg(unix)]
    #[test]
    fn file_reader_rejects_fifo_without_a_writer_in_bounded_child() {
        use std::{process::{Command, Stdio}, time::{Duration, Instant}};

        let root = TestDirectory::new();
        let fifo = root.0.join("session.fifo");
        nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "orchestration::session_identity::tests::fifo_reader_child", "--ignored", "--nocapture"])
            .env("COCKPIT_TEST_SESSION_FIFO", &fifo)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "FIFO reader child failed: {status}");
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("FIFO reader blocked without a writer");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
