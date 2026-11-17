//! One writer per app-data directory; the operating system releases the lock on exit.
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io,
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

pub fn acquire(directory: &Path) -> io::Result<File> {
    fs::create_dir_all(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::other("App-data directory is not a directory"));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join("instance.lock"))?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("App-instance lock is not a regular file"));
    }
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "Hahakigami is already running with this app-data directory. Quit that instance first.",
        ),
        TryLockError::Error(error) => error,
    })?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_second_process_and_releases_on_exit() {
        const DIRECTORY: &str = "HAHAKIGAMI_TEST_INSTANCE_DIRECTORY";
        if let Some(directory) = std::env::var_os(DIRECTORY) {
            let _lock = acquire(Path::new(&directory)).unwrap();
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("data");
        let child = || {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "instance::tests::blocks_second_process_and_releases_on_exit",
                    "--nocapture",
                ])
                .env(DIRECTORY, &root)
                .output()
                .unwrap()
        };
        let lock = acquire(&root).unwrap();
        assert_eq!(
            acquire(&root).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        let blocked = child();
        assert!(!blocked.status.success());
        assert!(String::from_utf8_lossy(&blocked.stderr).contains("already running"));
        let _independent = acquire(&directory.path().join("independent")).unwrap();
        drop(lock);
        let exited = child();
        assert!(exited.status.success(), "{exited:?}");
        let lock = acquire(&root).unwrap();
        lock.unlock().unwrap();
        let _replacement = acquire(&root).unwrap();
    }

    #[test]
    fn rejects_redirected_and_nonregular_lock_files() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        fs::write(&outside, b"unchanged").unwrap();
        let root = directory.path().join("data");
        fs::create_dir(&root).unwrap();
        let path = root.join("instance.lock");
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(acquire(&root).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"unchanged");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(acquire(&root).is_err());
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        assert!(acquire(&alias).is_err());
    }
}
