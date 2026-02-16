use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;

extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}

const LOCK_EX: i32 = 2;

/// Exclusive file lock on the data directory.
/// Lock released when dropped (file handle closed).
pub struct FileLock {
    _file: File,
}

impl FileLock {
    pub fn acquire(dir: &Path) -> Result<Self, String> {
        let lockpath = dir.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&lockpath)
            .map_err(|e| format!("lock: {e}"))?;
        let ret = unsafe { flock(file.as_raw_fd(), LOCK_EX) };
        if ret != 0 {
            return Err("failed to acquire lock".into());
        }
        Ok(FileLock { _file: file })
    }
}
