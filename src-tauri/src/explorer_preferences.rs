//! Small, independent explorer preferences; never changes scan settings.
use crate::cache_store::sync_dir;
use crate::sorting::Order;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct View {
    pub sort: Order,
    pub warning: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    sort: Order,
}
#[derive(Default)]
pub(crate) struct Preferences {
    record: Option<PathBuf>,
    sort: Order,
    warning: Option<String>,
}
/// Removes the `explorer-<uuid>.tmp` files a crash between `save`'s write and its rename left behind. Only regular
/// files go (a symlink or directory of that name stays), and nothing else in the shared config directory. Best effort.
fn remove_stale_temps(config: &Path) {
    let Ok(entries) = fs::read_dir(config) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `DirEntry::file_type` does not follow symlinks.
        if name.starts_with("explorer-")
            && name.ends_with(".tmp")
            && entry.file_type().is_ok_and(|kind| kind.is_file())
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}
impl Preferences {
    pub fn load(config: &Path) -> Self {
        remove_stale_temps(config);
        let record = config.join("explorer-preferences.json");
        let read = (|| -> Result<Order, std::io::Error> {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&record)?;
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::other("Preference file is not regular"));
            }
            let mut bytes = Vec::new();
            file.take(4097).read_to_end(&mut bytes)?;
            if bytes.len() > 4096 {
                return Err(std::io::Error::other("Preference file is too large"));
            }
            let value: Record = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
            if value.version != 1 {
                return Err(std::io::Error::other("Unsupported preference version"));
            }
            Ok(value.sort)
        })();
        match read {
            Ok(sort) => Self {
                record: Some(record),
                sort,
                warning: None,
            },
            Err(error) => Self {
                record: Some(record),
                sort: Order::default(),
                warning: (error.kind() != std::io::ErrorKind::NotFound).then(|| {
                    format!("Could not restore sort preference; using largest first. {error}")
                }),
            },
        }
    }
    pub fn view(&self) -> View {
        View {
            sort: self.sort,
            warning: self.warning.clone(),
        }
    }
    pub fn save(&mut self, sort: Order) -> Result<View, String> {
        let record = self
            .record
            .as_ref()
            .ok_or("Explorer preferences are unavailable.")?;
        let directory = record
            .parent()
            .ok_or("Preference directory is unavailable.")?;
        fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let temp = directory.join(format!("explorer-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<(), std::io::Error> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temp)?;
            serde_json::to_writer(&mut file, &Record { version: 1, sort })
                .map_err(std::io::Error::other)?;
            file.sync_all()?;
            // Sync the directory before publication as well; a failure leaves the
            // previous preference untouched. Atomic rename is the commit point.
            sync_dir(directory)?;
            fs::rename(&temp, record)?;
            // Publication succeeded. A directory-sync failure is a durability
            // warning rather than a failed write which could falsely promise rollback.
            self.warning = sync_dir(directory).err().map(|e| {
                format!("Sort preference saved, but durability could not be confirmed: {e}")
            });
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temp);
            return Err(error.to_string());
        }
        self.sort = sort;
        Ok(self.view())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn defaults_restore_and_preserve_other_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let workers = dir.path().join("scan-settings.json");
        fs::write(&workers, b"existing worker settings").unwrap();
        let mut prefs = Preferences::load(dir.path());
        assert_eq!(prefs.view().sort, Order::default());
        assert!(prefs.view().warning.is_none());
        let chosen = Order::parse(Some("nameAscending")).unwrap();
        prefs.save(chosen).unwrap();
        assert_eq!(Preferences::load(dir.path()).view().sort, chosen);
        assert_eq!(fs::read(workers).unwrap(), b"existing worker settings");
        assert_eq!(
            fs::metadata(dir.path().join("explorer-preferences.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn invalid_records_and_symlinks_are_recoverable() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("explorer-preferences.json");
        for data in [
            b"bad".to_vec(),
            br#"{"version":2,"sort":"sizeDescending"}"#.to_vec(),
            br#"{"version":1,"sort":"unknown"}"#.to_vec(),
            vec![b' '; 4097],
        ] {
            fs::write(&record, data).unwrap();
            let prefs = Preferences::load(dir.path()).view();
            assert_eq!(prefs.sort, Order::default());
            assert!(prefs.warning.is_some());
        }
        fs::remove_file(&record).unwrap();
        let other = dir.path().join("other");
        fs::write(&other, br#"{"version":1,"sort":"nameAscending"}"#).unwrap();
        std::os::unix::fs::symlink(&other, &record).unwrap();
        assert!(Preferences::load(dir.path()).view().warning.is_some());
        fs::remove_file(&record).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let fifo = std::ffi::CString::new(record.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let view = Preferences::load(dir.path()).view();
        assert_eq!(view.sort, Order::default());
        assert!(view.warning.is_some());
    }
    #[test]
    fn load_removes_stale_temp_files_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir
            .path()
            .join(format!("explorer-{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&stale, b"half written").unwrap();
        let kept = [
            ("scan-settings.tmp", b"worker temp".to_vec()),
            ("explorer-notes.txt", b"notes".to_vec()),
        ];
        for (name, bytes) in &kept {
            fs::write(dir.path().join(name), bytes).unwrap();
        }
        let directory = dir.path().join("explorer-dir.tmp");
        fs::create_dir(&directory).unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, b"untouched").unwrap();
        let link = dir.path().join("explorer-link.tmp");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        let chosen = Order::parse(Some("nameAscending")).unwrap();
        Preferences::load(dir.path()).save(chosen).unwrap();
        assert!(!stale.exists());
        for (name, bytes) in &kept {
            assert_eq!(fs::read(dir.path().join(name)).unwrap(), *bytes);
        }
        assert!(directory.is_dir());
        assert!(link.is_symlink());
        assert_eq!(fs::read(&victim).unwrap(), b"untouched");
        // Loading again, with the record in place, keeps it.
        assert_eq!(Preferences::load(dir.path()).view().sort, chosen);
    }
    #[test]
    fn failed_save_keeps_previous_value_and_retry_commits() {
        let dir = tempfile::tempdir().unwrap();
        let mut prefs = Preferences::load(dir.path());
        let chosen = Order::parse(Some("nameAscending")).unwrap();
        prefs.save(chosen).unwrap();
        let record = dir.path().join("explorer-preferences.json");
        let bytes = fs::read(&record).unwrap();
        // A directory at the commit destination deterministically rejects rename,
        // including when tests run as root. Retain the old file separately.
        let held = dir.path().join("held.json");
        fs::rename(&record, &held).unwrap();
        fs::create_dir(&record).unwrap();
        let next = Order::parse(Some("nameDescending")).unwrap();
        assert!(prefs.save(next).is_err());
        assert_eq!(prefs.view().sort, chosen);
        assert_eq!(fs::read(&held).unwrap(), bytes);
        assert!(!fs::read_dir(dir.path()).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "tmp")));
        fs::remove_dir(&record).unwrap();
        fs::rename(&held, &record).unwrap();
        prefs.save(next).unwrap();
        assert_eq!(Preferences::load(dir.path()).view().sort, next);
    }
}
