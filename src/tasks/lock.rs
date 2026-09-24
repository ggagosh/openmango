//! One OpenMango process at a time starts scheduled runs: the app while it's open, or the
//! background runner while it's closed. Whoever holds the lock file is that process.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

/// Held for as long as this process starts scheduled runs. The system lets go of it when the
/// process ends, however it ends.
#[derive(Debug)]
pub struct SchedulerLock(#[allow(dead_code)] File);

impl SchedulerLock {
    /// Takes the lock in `dir`, or `None` when another OpenMango process holds it.
    pub fn try_take(dir: &Path) -> std::io::Result<Option<Self>> {
        std::fs::create_dir_all(dir)?;
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(dir.join("tasks.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self(file))),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_holder_at_a_time() {
        let directory = tempfile::tempdir().unwrap();
        let first = SchedulerLock::try_take(directory.path()).unwrap();
        assert!(first.is_some());
        assert!(SchedulerLock::try_take(directory.path()).unwrap().is_none(), "held");
        drop(first);
        assert!(SchedulerLock::try_take(directory.path()).unwrap().is_some(), "let go");
    }
}
