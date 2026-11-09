use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, params};

const MAX_RECAP_BYTES: usize = 2048;

pub(super) fn database_path() -> Option<PathBuf> {
    // Non-default profile and config-directory layouts need their own resolved
    // agent directory; do not silently read another profile's history.
    let explicit = std::env::var_os("PI_CODING_AGENT_DIR").filter(|v| !v.is_empty());
    let dir = if let Some(dir) = explicit {
        PathBuf::from(dir)
    } else {
        for key in ["OMP_PROFILE", "PI_PROFILE"] {
            if std::env::var_os(key).is_some_and(|v| !v.is_empty() && v != "default") {
                return None;
            }
        }
        if std::env::var_os("PI_CONFIG_DIR").is_some_and(|v| !v.is_empty() && v != ".omp") {
            return None;
        }
        let home = PathBuf::from(std::env::var_os("HOME")?);
        // A migrated XDG store has a different location; avoid guessing it.
        if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            if PathBuf::from(data).join("omp").exists() {
                return None;
            }
        }
        home.join(".omp").join("agent")
    };
    if !dir.is_absolute() {
        return None;
    }
    Some(dir.join("history.db"))
}

pub(super) fn read_recap(session_id: &str) -> Option<(String, i64)> {
    read_recap_from(&database_path()?, session_id)
}

fn read_recap_from(path: &Path, session_id: &str) -> Option<(String, i64)> {
    if session_id.is_empty() || session_id.len() > 512 || session_id.chars().any(char::is_control) {
        return None;
    }
    let leaf = fs::symlink_metadata(path).ok()?;
    if !leaf.is_file() {
        return None;
    }

    // SQLite read-only WAL access requires the existing WAL and shared-memory
    // files. Refuse to let the reader create them if they are missing.
    let mut header = [0; 20];
    let mut file = open_regular(path)?;
    file.read_exact(&mut header).ok()?;
    if &header[..16] != b"SQLite format 3\0" {
        return None;
    }
    if header[18] == 2 || header[19] == 2 {
        for suffix in ["-wal", "-shm"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            if !fs::symlink_metadata(PathBuf::from(name)).ok()?.is_file() {
                return None;
            }
        }
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE;
    let connection = Connection::open_with_flags(path, flags).ok()?;
    connection.busy_timeout(Duration::ZERO).ok()?;
    // Sorting must not create SQLite temporary files for an unindexed history.
    connection.execute_batch("PRAGMA temp_store = MEMORY;").ok()?;
    let (recap, created_at): (Option<String>, i64) = connection.query_row(
        "SELECT CASE WHEN length(CAST(recap AS BLOB)) <= ?2 THEN recap END, created_at \
         FROM session_recaps WHERE session_id = ?1 COLLATE BINARY \
         ORDER BY id DESC LIMIT 1",
        params![session_id, MAX_RECAP_BYTES as i64],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).ok()?;
    if created_at < 0 { return None; }
    let recap = recap?;
    let text = recap.trim();
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    Some((text.to_owned(), created_at))
}

#[cfg(unix)]
fn open_regular(path: &Path) -> Option<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let before = fs::symlink_metadata(path).ok()?;
    if !before.is_file() { return None; }
    let file = fs::OpenOptions::new().read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path).ok()?;
    let opened = file.metadata().ok()?;
    if !opened.is_file() || before.dev() != opened.dev() || before.ino() != opened.ino() {
        return None;
    }
    Some(file)
}

#[cfg(not(unix))]
fn open_regular(path: &Path) -> Option<File> {
    if !fs::symlink_metadata(path).ok()?.is_file() { return None; }
    File::open(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn latest_exact_and_bounded() {
        let dir = std::env::temp_dir().join(format!("cockpit-recap-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("history.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE session_recaps (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, cwd TEXT NOT NULL, recap TEXT NOT NULL, created_at INTEGER NOT NULL);").unwrap();
        db.execute("INSERT INTO session_recaps VALUES (1, 'session', '/', 'old', 100)", []).unwrap();
        db.execute("INSERT INTO session_recaps VALUES (2, 'session', '/', 'new', 200)", []).unwrap();
        db.execute("INSERT INTO session_recaps VALUES (3, 'other', '/', 'wrong', 300)", []).unwrap();
        assert_eq!(read_recap_from(&path, "session"), Some(("new".to_owned(), 200)));
        assert_eq!(read_recap_from(&path, "SESSION"), None);
        assert_eq!(read_recap_from(&path, "' OR 1=1 --"), None);
        db.execute("INSERT INTO session_recaps VALUES (4, 'session', '/', ?1, 400)", ["x".repeat(MAX_RECAP_BYTES + 1)]).unwrap();
        assert_eq!(read_recap_from(&path, "session"), None);
        db.execute("INSERT INTO session_recaps VALUES (5, 'session', '/', 'line1' || char(10) || 'line2', 500)", []).unwrap();
        assert_eq!(read_recap_from(&path, "session"), None);
        drop(db);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_uncheckpointed_wal() {
        let dir = std::env::temp_dir().join(format!("cockpit-recap-wal-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("history.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE session_recaps (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, cwd TEXT NOT NULL, recap TEXT NOT NULL, created_at INTEGER NOT NULL); INSERT INTO session_recaps VALUES (1, 'session', '/', 'in WAL', 123);").unwrap();
        assert_eq!(read_recap_from(&path, "session"), Some(("in WAL".to_owned(), 123)));
        drop(db);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_database_is_not_created() {
        let path = std::env::temp_dir().join(format!("cockpit-no-recap-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        assert_eq!(read_recap_from(&path, "session"), None);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_database() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("cockpit-recap-link-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&dir).unwrap();
        let real = dir.join("real.db");
        drop(Connection::open(&real).unwrap());
        let link = dir.join("history.db");
        symlink(real, &link).unwrap();
        assert_eq!(read_recap_from(&link, "session"), None);
        fs::remove_dir_all(dir).unwrap();
    }
}
