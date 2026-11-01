use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentContext {
    pub(super) native_session_id: String,
    pub(super) session_title: Option<String>,
    pub(super) working_description: Option<String>,
    pub(super) latest_turn_timestamp: Option<i64>,
}

const PREFIX_BYTES: usize = 64 * 1024;
const TAIL_BYTES: usize = 128 * 1024;
const TEXT_BYTES: usize = 512;

fn bounded_text(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > TEXT_BYTES || value.chars().any(char::is_control) {
        None
    } else {
        Some(value.to_owned())
    }
}

// Only newline-terminated records can be distinguished from an in-progress write.
fn complete_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split_inclusive(|byte| *byte == b'\n')
        .filter_map(|line| line.strip_suffix(b"\n"))
}

fn parse_context(prefix: &[u8], tail: &[u8], tail_starts_mid_file: bool) -> Option<AgentContext> {
    let mut lines = complete_lines(prefix);
    let first: serde_json::Value = serde_json::from_slice(lines.next()?).ok()?;
    let (session, session_title) = if first.get("type")?.as_str()? == "title" {
        let title = first.get("title").and_then(|v| v.as_str()).and_then(bounded_text);
        (serde_json::from_slice::<serde_json::Value>(lines.next()?).ok()?, title)
    } else {
        (first, None)
    };
    if session.get("type")?.as_str()? != "session" {
        return None;
    }
    let id = session.get("id")?.as_str()?;
    if id.is_empty() || id.len() > TEXT_BYTES || id.chars().any(char::is_control) {
        return None;
    }
    let session_title = session_title.or_else(|| session.get("title").and_then(|v| v.as_str()).and_then(bounded_text));

    // If the window begins in a record, discard that record even if its suffix
    // happens to look like JSON. Starts outside the window are never inferred.
    let tail = if tail_starts_mid_file {
        match tail.iter().position(|byte| *byte == b'\n') {
            Some(index) => &tail[index + 1..],
            None => &[],
        }
    } else {
        tail
    };
    let mut outstanding: Vec<(String, Option<String>)> = Vec::new();
    let mut latest_turn_timestamp = None;
    for line in complete_lines(tail) {
        let record: serde_json::Value = match serde_json::from_slice(line) {
            Ok(record) => record,
            // A damaged record could have been a completion; do not report an
            // earlier start as definitely outstanding after it.
            Err(_) => { outstanding.clear(); latest_turn_timestamp = None; continue; }
        };
        if record.get("type").and_then(|v| v.as_str()) == Some("custom")
            && record.get("customType").and_then(|v| v.as_str()) == Some("tool_execution_start")
        {
            latest_turn_timestamp = None;
            if let Some(data) = record.get("data") {
                if let Some(id) = data.get("toolCallId").and_then(|v| v.as_str()).and_then(bounded_text) {
                    let intent = data.get("intent").and_then(|v| v.as_str()).and_then(bounded_text);
                    outstanding.retain(|(previous, _)| previous != &id);
                    outstanding.push((id, intent));
                }
            }
        } else if record.get("type").and_then(|v| v.as_str()) == Some("message") {
            if let Some(message) = record.get("message") {
                let role = message.get("role").and_then(|v| v.as_str());
                if matches!(role, Some("user" | "assistant" | "toolResult")) {
                    latest_turn_timestamp = if role == Some("assistant")
                        && message.get("stopReason").and_then(|v| v.as_str()) == Some("stop")
                    {
                        record.get("timestamp").and_then(|v| v.as_str())
                            .and_then(|value| time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok())
                            .map(|timestamp| timestamp.unix_timestamp())
                    } else {
                        None
                    };
                }
                if message.get("role").and_then(|v| v.as_str()) == Some("toolResult") {
                    if let Some(id) = message.get("toolCallId").and_then(|v| v.as_str()).and_then(bounded_text) {
                        outstanding.retain(|(previous, _)| previous != &id);
                    } else {
                        outstanding.clear();
                    }
                } else if message.get("role").and_then(|v| v.as_str()) == Some("user")
                    || (message.get("role").and_then(|v| v.as_str()) == Some("assistant")
                        && matches!(message.get("stopReason").and_then(|v| v.as_str()), Some("stop" | "aborted" | "error")))
                {
                    outstanding.clear();
                }
            }
        }
    }
    if !tail.is_empty() && tail.last() != Some(&b'\n') {
        latest_turn_timestamp = None;
    }
    Some(AgentContext {
        native_session_id: id.to_owned(),
        session_title,
        working_description: outstanding.pop().and_then(|(_, intent)| intent),
        latest_turn_timestamp,
    })
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        collections::VecDeque,
        fs::{File, Metadata, OpenOptions},
        io::{Read, Seek, SeekFrom},
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        sync::{LazyLock, Mutex},
    };

    const CACHE_SIZE: usize = 128;

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Identity {
        dev: u64,
        ino: u64,
        len: u64,
        mtime: i64,
        mtime_nsec: i64,
        ctime: i64,
        ctime_nsec: i64,
    }

    impl Identity {
        fn of(metadata: &Metadata) -> Self {
            Self {
                dev: metadata.dev(), ino: metadata.ino(), len: metadata.len(),
                mtime: metadata.mtime(), mtime_nsec: metadata.mtime_nsec(),
                ctime: metadata.ctime(), ctime_nsec: metadata.ctime_nsec(),
            }
        }
    }

    struct Entry {
        path: String,
        identity: Identity,
        context: AgentContext,
    }

    static CACHE: LazyLock<Mutex<VecDeque<Entry>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));

    fn stable(file: &File, path: &Path, identity: Identity) -> bool {
        let Ok(opened) = file.metadata() else { return false };
        let Ok(leaf) = std::fs::symlink_metadata(path) else { return false };
        opened.is_file() && leaf.is_file()
            && Identity::of(&opened) == identity && Identity::of(&leaf) == identity
    }


    fn read_window(file: &mut File, offset: u64, len: usize) -> Option<Vec<u8>> {
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut bytes = vec![0; len];
        file.read_exact(&mut bytes).ok()?;
        Some(bytes)
    }

    pub(super) fn read(path: &str) -> Option<AgentContext> {
        read_with_hook(path, || {})
    }

    // The hook makes the metadata race check deterministic in unit tests.
    fn read_with_hook(path: &str, after_read: impl FnOnce()) -> Option<AgentContext> {
        let path_ref = Path::new(path);
        if !path_ref.is_absolute() || path.chars().any(char::is_control) {
            return None;
        }
        let leaf = std::fs::symlink_metadata(path_ref).ok()?;
        if !leaf.is_file() { return None; }
        let mut file = OpenOptions::new().read(true)
            .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
            .open(path_ref).ok()?;
        let opened = file.metadata().ok()?;
        if !opened.is_file() || opened.dev() != leaf.dev() || opened.ino() != leaf.ino() {
            return None;
        }
        let identity = Identity::of(&opened);
        let cache = &*CACHE;
        let cached = cache.lock().ok().and_then(|mut entries| {
            let index = entries.iter().position(|entry| entry.path == path && entry.identity == identity)?;
            let entry = entries.remove(index)?;
            let context = entry.context.clone();
            entries.push_back(entry);
            Some(context)
        });
        if let Some(context) = cached {
            after_read();
            return stable(&file, path_ref, identity).then_some(context);
        }
        let prefix_len = identity.len.min(PREFIX_BYTES as u64) as usize;
        let tail_len = identity.len.min(TAIL_BYTES as u64) as usize;
        let prefix = read_window(&mut file, 0, prefix_len)?;
        let tail_offset = identity.len - tail_len as u64;
        let tail = read_window(&mut file, tail_offset, tail_len)?;
        after_read();
        if !stable(&file, path_ref, identity) { return None; }
        let context = parse_context(&prefix, &tail, tail_offset != 0)?;
        if let Ok(mut entries) = cache.lock() {
            entries.retain(|entry| entry.path != path);
            if entries.len() >= CACHE_SIZE { entries.pop_front(); }
            entries.push_back(Entry { path: path.to_owned(), identity, context: context.clone() });
        }
        Some(context)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{os::unix::fs::symlink, sync::atomic::{AtomicU64, Ordering}};

        static NEXT: AtomicU64 = AtomicU64::new(0);
        struct Fixture(std::path::PathBuf);
        impl Fixture {
            fn new(contents: &[u8]) -> Self {
                let path = std::env::temp_dir().join(format!("cockpit-omp-context-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
                std::fs::write(&path, contents).unwrap();
                Self(path)
            }
            fn path(&self) -> &str { self.0.to_str().unwrap() }
        }
        impl Drop for Fixture {
            fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
        }
        const HEADER: &str = "{\"type\":\"title\",\"title\":\" First \"}\n{\"type\":\"session\",\"id\":\"opaque id\"}\n";
        fn start(id: &str, intent: &str) -> String {
            format!("{{\"type\":\"custom\",\"customType\":\"tool_execution_start\",\"data\":{{\"toolCallId\":{id:?},\"intent\":{intent:?}}}}}\n")
        }
        fn result(id: &str) -> String {
            format!("{{\"type\":\"message\",\"message\":{{\"role\":\"toolResult\",\"toolCallId\":{id:?}}}}}\n")
        }

        #[test]
        fn reads_identity_title_and_outstanding_calls() {
            let data = format!("{HEADER}{}{}{}", start("a", " first "), start("b", "second"), result("b"));
            let f = Fixture::new(data.as_bytes());
            let context = read(f.path()).unwrap();
            assert_eq!(context.native_session_id, "opaque id");
            assert_eq!(context.session_title.as_deref(), Some("First"));
            assert_eq!(context.working_description.as_deref(), Some("first"));
            std::fs::write(&f.0, format!("{data}{}", result("a"))).unwrap();
            assert_eq!(read(f.path()).unwrap().working_description, None);
        }

        #[test]
        fn accepts_session_without_title_and_clears_finished_turns() {
            let header = "{\"type\":\"session\",\"id\":\"id\"}\n";
            for ending in [
                "{\"type\":\"message\",\"message\":{\"role\":\"user\"}}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"stop\"}}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"aborted\"}}\n",
            ] {
                let f = Fixture::new(format!("{header}{}{ending}", start("a", "old")).as_bytes());
                let context = read(f.path()).unwrap();
                assert_eq!(context.session_title, None);
                assert_eq!(context.working_description, None);
            }
            let f = Fixture::new(format!("{header}{}{{\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"stopReason\":\"toolUse\"}}}}\n", start("a", "active")).as_bytes());
            assert_eq!(read(f.path()).unwrap().working_description.as_deref(), Some("active"));
        }

        #[test]
        fn session_title_fallback_preserves_first_line_precedence() {
            for (prefix, expected) in [
                ("", Some("fallback")),
                ("{\"type\":\"title\",\"title\":\"  \"}\n", Some("fallback")),
                ("{\"type\":\"title\",\"title\":\" current \"}\n", Some("current")),
            ] {
                let f = Fixture::new(format!("{prefix}{{\"type\":\"session\",\"id\":\"id\",\"title\":\" fallback \"}}\n").as_bytes());
                assert_eq!(read(f.path()).unwrap().session_title.as_deref(), expected);
            }
        }

        #[test]
        fn final_turn_timestamp_requires_complete_latest_terminal_assistant() {
            let final_message = "{\"type\":\"message\",\"timestamp\":\"2026-01-01T00:00:00.123Z\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"stop\"}}\n";
            let f = Fixture::new(format!("{HEADER}{final_message}").as_bytes());
            assert_eq!(read(f.path()).unwrap().latest_turn_timestamp, Some(1767225600));
            for later in [
                "{\"type\":\"message\",\"message\":{\"role\":\"user\"}}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"toolResult\",\"toolCallId\":\"a\"}}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"toolUse\"}}\n",
                "{\"type\":\"message\"",
                "corrupt\n",
            ] {
                std::fs::write(&f.0, format!("{HEADER}{final_message}{later}")).unwrap();
                assert_eq!(read(f.path()).unwrap().latest_turn_timestamp, None);
            }
            std::fs::write(&f.0, format!("{HEADER}{{\"type\":\"message\",\"timestamp\":\"invalid\",\"message\":{{\"role\":\"assistant\",\"stopReason\":\"stop\"}}}}\n")).unwrap();
            assert_eq!(read(f.path()).unwrap().latest_turn_timestamp, None);
        }

        #[test]
        fn rejects_invalid_headers_and_sanitizes_text() {
            for header in ["{}\n{}\n".to_owned(), "{\"type\":\"title\"}\n{\"type\":\"session\",\"id\":\"bad\\n\"}\n".to_owned(), format!("{{\"type\":\"title\"}}\n{{\"type\":\"session\",\"id\":\"{}\"}}\n", "a".repeat(513)), HEADER.trim_end().to_owned()] {
                let f = Fixture::new(header.as_bytes());
                assert_eq!(read(f.path()), None);
            }
            let f = Fixture::new(format!("{{\"type\":\"title\",\"title\":\"bad\\ntext\"}}\n{{\"type\":\"session\",\"id\":\"id\"}}\n{}", start("a", &"x".repeat(513))).as_bytes());
            let context = read(f.path()).unwrap();
            assert_eq!(context.session_title, None);
            assert_eq!(context.working_description, None);
        }

        #[test]
        fn ignores_partial_records_and_invalidates_uncertain_starts() {
            let partial = start("b", "partial");
            let data = format!("{HEADER}{}not-json\n{}{}", start("a", "old"), start("c", "new"), partial.trim_end());
            let f = Fixture::new(data.as_bytes());
            assert_eq!(read(f.path()).unwrap().working_description.as_deref(), Some("new"));
            let data = format!("{HEADER}{}not-json\n", start("a", "old"));
            std::fs::write(&f.0, data).unwrap();
            assert_eq!(read(f.path()).unwrap().working_description, None);
            let data = format!("{HEADER}{}{}{}", start("a", "outside"), "x".repeat(TAIL_BYTES), result("a"));
            std::fs::write(&f.0, data).unwrap();
            assert_eq!(read(f.path()).unwrap().working_description, None);
        }

        #[test]
        fn rejects_mutation_and_symlink_and_refreshes_cache() {
            let f = Fixture::new(HEADER.as_bytes());
            assert_eq!(read(f.path()).unwrap().session_title.as_deref(), Some("First"));
            let updated = HEADER.replace("First", "Other");
            std::fs::write(&f.0, &updated).unwrap();
            assert_eq!(read(f.path()).unwrap().session_title.as_deref(), Some("Other"));
            assert_eq!(read_with_hook(f.path(), || std::fs::write(&f.0, HEADER).unwrap()), None);
            let link = f.0.with_extension("link");
            symlink(&f.0, &link).unwrap();
            assert_eq!(read(link.to_str().unwrap()), None);
            std::fs::remove_file(link).unwrap();
            assert_eq!(read("relative"), None);
            assert_eq!(read("/tmp/bad\npath"), None);
        }
    }
}

pub(super) fn read_omp_context(path: &str) -> Option<AgentContext> {
    #[cfg(unix)]
    { unix::read(path) }
    #[cfg(not(unix))]
    { let _ = path; None }
}
