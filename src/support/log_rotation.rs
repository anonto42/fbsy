//! Size-based log rotation for detached service processes.
//!
//! Each service child writes to its log file via its inherited stdout fd
//! (opened with O_APPEND in [`crate::runtime::process`]). Because the fd
//! stays open inside the child, we use **copytruncate** semantics: copy the
//! current log to a numbered backup, then truncate the live file to zero.
//! The fd's next write goes to offset 0 of the now-empty file — no fd
//! re-opening required.
//!
//! Files are numbered `.log.1` (newest) … `.log.N` (oldest). When N is
//! reached the oldest is deleted before shifting.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Cap at which a log is rotated (10 MiB).
pub const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
/// Number of rotated backup files to keep.
pub const KEEP_FILES: u32 = 5;
const CHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Spawn a background daemon thread that checks and rotates `log_path` every
/// 60 seconds. Safe to call from within a detached child process.
pub fn spawn_rotation_thread(log_path: PathBuf) {
    std::thread::Builder::new()
        .name("log-rotate".to_string())
        .spawn(move || loop {
            std::thread::sleep(CHECK_INTERVAL);
            rotate_if_needed(&log_path, MAX_LOG_BYTES, KEEP_FILES);
        })
        .ok();
}

/// Rotate `path` if it exceeds `max_bytes`, keeping up to `keep` backups.
/// No-ops silently if the file is within the limit or if any step fails.
pub fn rotate_if_needed(path: &Path, max_bytes: u64, keep: u32) {
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if size <= max_bytes {
        return;
    }

    // Shift existing backups: delete oldest, rename N-1→N, …, 1→2.
    let _ = std::fs::remove_file(backup_path(path, keep));
    for i in (1..keep).rev() {
        let from = backup_path(path, i);
        let to = backup_path(path, i + 1);
        let _ = std::fs::rename(&from, &to);
    }

    // Copy current log to .log.1.
    let _ = std::fs::copy(path, backup_path(path, 1));

    // Truncate the live file to zero. The child's append-mode fd continues
    // writing from offset 0 — no fd re-open needed.
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_len(0);
    }
}

fn backup_path(log: &Path, n: u32) -> PathBuf {
    // "bridge.log" → "bridge.log.1", "bridge.log.2", …
    let mut p = log.as_os_str().to_os_string();
    p.push(format!(".{n}"));
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_bytes(path: &Path, n: usize) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .unwrap();
        f.write_all(&vec![b'x'; n]).unwrap();
    }

    #[test]
    fn no_rotation_when_under_cap() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("svc.log");
        write_bytes(&log, 100);
        rotate_if_needed(&log, 1024, 3);
        assert!(log.exists());
        assert!(!backup_path(&log, 1).exists());
    }

    #[test]
    fn rotation_creates_backup_and_truncates_live_file() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("svc.log");
        write_bytes(&log, 200);
        rotate_if_needed(&log, 100, 3);

        // Live file truncated.
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);
        // Backup created.
        assert!(backup_path(&log, 1).exists());
        assert_eq!(std::fs::metadata(backup_path(&log, 1)).unwrap().len(), 200);
    }

    #[test]
    fn rotation_shifts_existing_backups() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("svc.log");
        // Pre-seed .log.1 and .log.2.
        write_bytes(&backup_path(&log, 1), 10);
        write_bytes(&backup_path(&log, 2), 20);
        write_bytes(&log, 200);

        rotate_if_needed(&log, 100, 3);

        // Old .1 shifted to .2, old .2 shifted to .3.
        assert_eq!(std::fs::metadata(backup_path(&log, 2)).unwrap().len(), 10);
        assert_eq!(std::fs::metadata(backup_path(&log, 3)).unwrap().len(), 20);
    }

    #[test]
    fn oldest_backup_deleted_when_keep_exceeded() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("svc.log");
        // Fill all 2 backup slots.
        write_bytes(&backup_path(&log, 1), 1);
        write_bytes(&backup_path(&log, 2), 1);
        write_bytes(&log, 200);

        rotate_if_needed(&log, 100, 2);

        assert!(!backup_path(&log, 3).exists(), "slot 3 must not exist");
        assert!(backup_path(&log, 2).exists());
    }
}
