//! Private, bounded optimization cache. Historical library entries are never touched.
use crate::library::{Library, ScanMetadata};
use disk_scan::{Identity, ScanState, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Write},
    os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Token {
    pub cursor: u64,
    pub epoch: String,
    pub full_scan_at_ms: u64,
}
#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    accounting: disk_scan::AccountingInfo,
    root: Vec<u8>,
    identity: Identity,
    token: Token,
    started_at_ms: u64,
}
pub struct Candidate {
    id: String,
    pub token: Token,
    started_at_ms: u64,
}
pub struct CachedScan {
    pub snapshot: Snapshot,
    pub token: Token,
}
/// A candidate confirmed under the cache lock, decoded without it.
pub struct Pending {
    library: Library,
    id: String,
    token: Token,
    started_at_ms: u64,
}
impl Pending {
    pub fn decode(self, root: &Path, identity: &Identity) -> Result<CachedScan, String> {
        let (snapshot, metadata) = self.library.load_cache(&self.id)?;
        if !eligible(&snapshot)
            || snapshot.root != root
            || snapshot.node(0).ok_or("Missing cache root")?.identity != *identity
            || metadata.started_at_ms != self.started_at_ms
        {
            return Err("Rescan cache does not match its root or policy".into());
        }
        Ok(CachedScan {
            snapshot,
            token: self.token,
        })
    }
}
pub struct Cache {
    root: PathBuf,
    library: Library,
    count_limit: usize,
    bytes_limit: u64,
}
pub fn eligible(s: &Snapshot) -> bool {
    s.incremental_accounting_eligible()
        && s.accounting.warnings.is_empty()
        && s.accounting.deduction_bytes == 0
        && s.state == ScanState::Completed
        && !s.stats.incomplete
        && s.stats.errors == 0
        && s.stats.excluded == 0
        && s.node_count() != 0
        && s.root != Path::new("/")
        && !s.iter_nodes().any(|n| n.excluded)
}
pub fn recent(token: &Token, started_at_ms: u64, now: u64) -> bool {
    token.full_scan_at_ms != 0
        && token.full_scan_at_ms <= started_at_ms
        && started_at_ms <= now
        && now - token.full_scan_at_ms <= 60 * 60 * 1000
}
impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self {
            library: Library::new(root.clone()),
            root,
            count_limit: 8,
            bytes_limit: 512 * 1024 * 1024,
        }
    }
    pub fn contains_storage(&self, scope: &Path) -> bool {
        self.root.starts_with(scope)
    }
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        let uuid = uuid::Uuid::parse_str(id).map_err(|_| "Invalid cache ID")?;
        if uuid.to_string() != id {
            return Err("Invalid cache ID".into());
        }
        Ok(self.root.join(format!("{id}.cursor")))
    }
    fn record(&self, id: &str) -> Result<Record, String> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.path(id)?)
            .map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file()
            || file.metadata().map_err(|e| e.to_string())?.len() > 16 * 1024
        {
            return Err("Invalid cache record".into());
        }
        let r: Record = serde_json::from_reader(BufReader::new(file.take(16 * 1024)))
            .map_err(|e| e.to_string())?;
        if r.version != 3
            || r.accounting.model != disk_scan::AccountingModel::FullClonesV1
            || !r.accounting.checks_complete
            || r.accounting.shared
            || !r.accounting.warnings.is_empty()
            || r.accounting.deduction_bytes != 0
            || r.token.cursor == 0
            || r.token.epoch.is_empty()
            || r.token.epoch.len() > 256
            || r.token.full_scan_at_ms == 0
            || r.token.full_scan_at_ms > r.started_at_ms
        {
            return Err("Unsupported cache record".into());
        }
        Ok(r)
    }
    pub fn probe(
        &self,
        root: &Path,
        identity: &Identity,
        epoch: &str,
    ) -> Result<Option<Candidate>, String> {
        self.maintain()?;
        for row in self.library.list()?.entries {
            let Ok(record) = self.record(&row.id) else {
                continue;
            };
            if record.root != root.as_os_str().as_bytes()
                || &record.identity != identity
                || record.token.epoch != epoch
                || !recent(&record.token, record.started_at_ms, crate::now_ms())
            {
                continue;
            }
            return Ok(Some(Candidate {
                id: row.id,
                token: record.token,
                started_at_ms: record.started_at_ms,
            }));
        }
        Ok(None)
    }
    /// Confirm the candidate's record is still the one whose history was replayed. Brief: call it under
    /// the cache lock, then decode the `Pending` entry without it. A published archive never changes
    /// (every save gets a new ID), so pruning can at worst remove it, and then the decode fails and
    /// the scan falls back to a full one.
    pub fn begin_load(
        &self,
        candidate: Candidate,
        root: &Path,
        identity: &Identity,
        epoch: &str,
    ) -> Result<Pending, String> {
        // Bind the tree to precisely the token whose history was replayed. Never
        // pick a newer cache after replay, including across multiple app instances.
        let record = self.record(&candidate.id)?;
        if record.root != root.as_os_str().as_bytes()
            || &record.identity != identity
            || record.token != candidate.token
            || record.started_at_ms != candidate.started_at_ms
            || record.token.epoch != epoch
            || !recent(&record.token, record.started_at_ms, crate::now_ms())
        {
            return Err("Rescan cache changed during event replay".into());
        }
        Ok(Pending {
            library: self.library.clone(),
            id: candidate.id,
            token: record.token,
            started_at_ms: record.started_at_ms,
        })
    }
    #[cfg(test)]
    pub fn load_candidate(
        &self,
        candidate: Candidate,
        root: &Path,
        identity: &Identity,
        epoch: &str,
    ) -> Result<CachedScan, String> {
        self.begin_load(candidate, root, identity, epoch)?
            .decode(root, identity)
    }
    #[cfg(test)]
    pub fn load(
        &self,
        root: &Path,
        identity: &Identity,
        epoch: &str,
    ) -> Result<Option<CachedScan>, String> {
        self.probe(root, identity, epoch)?
            .map(|candidate| self.load_candidate(candidate, root, identity, epoch))
            .transpose()
    }
    fn archive_bytes(&self, id: &str) -> Result<u64, String> {
        self.path(id)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.root.join(format!("{id}.zst")))
            .map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Invalid cache archive".into());
        }
        Ok(metadata.len())
    }
    fn maintain(&self) -> Result<(), String> {
        let entries = self.library.list()?.entries;
        let ids: std::collections::HashSet<_> = entries
            .iter()
            .filter(|row| self.record(&row.id).is_ok())
            .map(|row| row.id.as_str())
            .collect();
        for entry in fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if self.path(stem).is_err() {
                continue;
            }
            let suffix = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            if suffix == "tmp"
                || (["json", "zst", "cursor"].contains(&suffix) && !ids.contains(stem))
            {
                // UUID-named files in this optimization cache only. Never follow a symlink.
                if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                    continue;
                }
                fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
        File::open(&self.root)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        self.library.delete(id)?;
        match fs::remove_file(self.path(id)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
    fn write_record(&self, id: &str, record: &Record) -> Result<(), String> {
        let temp = self.root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        let write = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temp)
                .map_err(|e| e.to_string())?;
            serde_json::to_writer(&mut file, record).map_err(|e| e.to_string())?;
            file.flush()
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            fs::rename(&temp, self.path(id)?).map_err(|e| e.to_string())?;
            File::open(&self.root)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())
        })();
        if write.is_err() {
            let _ = fs::remove_file(temp);
        }
        write
    }
    /// Caller must prove that the freshly scanned tree is semantically unchanged.
    /// Serialized cache operations bind this refresh to the exact previous seed.
    pub fn refresh_unchanged(
        &self,
        root: &Path,
        identity: &Identity,
        previous: &Token,
        next: &Token,
    ) -> Result<bool, String> {
        if next.cursor == 0
            || next.cursor < previous.cursor
            || next.epoch != previous.epoch
            || next.full_scan_at_ms != previous.full_scan_at_ms
        {
            return Ok(false);
        }
        let Some(candidate) = self.probe(root, identity, &previous.epoch)? else {
            return Ok(false);
        };
        if candidate.token != *previous {
            return Ok(false);
        }
        // Recheck the newest published seed and its exact cursor immediately before
        // replacement. A missing seed or newer publication cannot be refreshed.
        let Some(current) = self.probe(root, identity, &previous.epoch)? else {
            return Ok(false);
        };
        if current.id != candidate.id
            || current.token != candidate.token
            || current.started_at_ms != candidate.started_at_ms
        {
            return Ok(false);
        }
        let Ok(mut record) = self.record(&candidate.id) else {
            return Ok(false);
        };
        if record.root != root.as_os_str().as_bytes()
            || record.identity != *identity
            || record.token != *previous
            || record.started_at_ms != candidate.started_at_ms
            || !recent(&record.token, record.started_at_ms, crate::now_ms())
        {
            return Ok(false);
        }
        record.token = next.clone();
        self.write_record(&candidate.id, &record)?;
        self.library
            .touch_cache(&candidate.id, record.started_at_ms)?;
        Ok(true)
    }
    pub fn save(&self, s: &Snapshot, m: &ScanMetadata, token: &Token) -> Result<(), String> {
        if !eligible(s) || token.cursor == 0 || token.epoch.is_empty() || token.epoch.len() > 256 {
            return Err("This result cannot seed incremental scans".into());
        }
        self.maintain()?;
        let row = self.library.save_cache(s, m)?;
        if let Err(e) = self.write_record(
            &row.id,
            &Record {
                version: 3,
                accounting: s.accounting.clone(),
                root: s.root.as_os_str().as_bytes().to_vec(),
                identity: s.node(0).unwrap().identity.clone(),
                token: token.clone(),
                started_at_ms: m.started_at_ms,
            },
        ) {
            let _ = self.remove(&row.id);
            return Err(e);
        }
        if row.file_bytes > self.bytes_limit {
            self.remove(&row.id)?;
            return Err("Rescan cache exceeds its storage budget".into());
        }
        let mut count = 0usize;
        let mut bytes = 0u64;
        let mut entries = self.library.list()?.entries;
        entries.sort_by_key(|entry| entry.id != row.id);
        for entry in entries {
            let actual_bytes = self.archive_bytes(&entry.id)?;
            let record = self.record(&entry.id);
            let same = record
                .as_ref()
                .is_ok_and(|r| r.root == s.root.as_os_str().as_bytes());
            let discard = record.is_err()
                || (entry.id != row.id && same)
                || count >= self.count_limit
                || bytes.saturating_add(actual_bytes) > self.bytes_limit;
            if discard {
                self.remove(&entry.id)?;
            } else {
                count += 1;
                bytes += actual_bytes;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};
    fn fixture(path: &Path) -> Snapshot {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("file"), b"abc").unwrap();
        disk_scan::scan(
            path.to_path_buf(),
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap()
    }
    fn metadata() -> ScanMetadata {
        ScanMetadata {
            started_at_ms: crate::now_ms(),
            completed_at_ms: crate::now_ms(),
            workers: 1,
            worker_mode: "auto".into(),
            worker_source: "hardware".into(),
            qos: disk_scan::ScanQos::Automatic,
        }
    }
    #[test]
    fn accounting_evidence_is_required_for_every_seed() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = fixture(&directory.path().join("source"));
        assert!(eligible(&snapshot));
        for kind in 0..5 {
            let mut rejected = snapshot.clone();
            match kind {
                0 => rejected.accounting.model = disk_scan::AccountingModel::HardLinksV1,
                1 => rejected.accounting.checks_complete = false,
                2 => rejected.accounting.shared = true,
                3 => rejected
                    .accounting
                    .warnings
                    .push("uncertain metadata".into()),
                _ => rejected.accounting.deduction_bytes = 1,
            }
            assert!(!eligible(&rejected));
        }
    }

    #[test]
    fn reload_matches_identity_epoch_and_keeps_history_separate() {
        let d = tempfile::tempdir().unwrap();
        let s = fixture(&d.path().join("source"));
        let c = Cache::new(d.path().join("cache"));
        let token = Token {
            cursor: 42,
            epoch: "boot:journal".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        c.save(&s, &metadata(), &token).unwrap();
        let fresh = Cache::new(d.path().join("cache"));
        let loaded = fresh
            .load(&s.root, &s.node(0).unwrap().identity, &token.epoch)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.snapshot.stats.entries, s.stats.entries);
        assert_eq!(loaded.token.cursor, 42);
        assert!(fresh
            .load(&s.root, &s.node(0).unwrap().identity, "changed")
            .unwrap()
            .is_none());
        let mut identity = s.node(0).unwrap().identity.clone();
        identity.inode += 1;
        assert!(fresh
            .load(&s.root, &identity, &token.epoch)
            .unwrap()
            .is_none());
        assert!(Library::new(d.path().join("manual-history"))
            .list()
            .unwrap()
            .entries
            .is_empty());
        assert!(!c.root.join("selection.json").exists());
        let row = c.library.list().unwrap().entries.remove(0);
        for suffix in ["json", "zst", "cursor"] {
            assert_eq!(
                fs::metadata(c.root.join(format!("{}.{suffix}", row.id)))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn replacement_pruning_corruption_and_budget() {
        let d = tempfile::tempdir().unwrap();
        let mut c = Cache::new(d.path().join("cache"));
        c.count_limit = 2;
        let token = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        let s = fixture(&d.path().join("a"));
        c.save(&s, &metadata(), &token).unwrap();
        c.save(&s, &metadata(), &token).unwrap();
        assert_eq!(c.library.list().unwrap().entries.len(), 1);
        for n in ["b", "c"] {
            let s = fixture(&d.path().join(n));
            c.save(&s, &metadata(), &token).unwrap();
        }
        assert_eq!(c.library.list().unwrap().entries.len(), 2);
        let s = fixture(&d.path().join("d"));
        c.bytes_limit = 1;
        assert!(c.save(&s, &metadata(), &token).is_err());
        assert_eq!(c.library.list().unwrap().entries.len(), 2);
        c.bytes_limit = 512 * 1024 * 1024;
        c.save(&s, &metadata(), &token).unwrap();
        let row = c
            .library
            .list()
            .unwrap()
            .entries
            .into_iter()
            .find(|r| r.root_path == s.root.to_string_lossy())
            .unwrap();
        fs::write(c.root.join(format!("{}.zst", row.id)), b"corrupt").unwrap();
        assert!(c
            .load(&s.root, &s.node(0).unwrap().identity, &token.epoch)
            .is_err());
    }
    // The app shares one Cache behind a mutex; scans on different disks that finish together publish through it.
    #[test]
    fn scans_publishing_together_keep_whole_entries_within_the_count_and_byte_limits() {
        let d = tempfile::tempdir().unwrap();
        let mut c = Cache::new(d.path().join("cache"));
        let sources: Vec<_> = (0..8)
            .map(|n| fixture(&d.path().join(format!("source-{n}"))))
            .collect();
        c.save(
            &sources[0],
            &metadata(),
            &Token {
                cursor: 1,
                epoch: "epoch".into(),
                full_scan_at_ms: crate::now_ms(),
            },
        )
        .unwrap();
        let one = c.library.list().unwrap().entries[0].file_bytes;
        c.count_limit = 5;
        // Room for three archives of this size, so the byte cap binds before the count cap.
        c.bytes_limit = one * 3 + one / 2;
        let limit = c.bytes_limit;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(Some(c)));
        std::thread::scope(|scope| {
            for source in &sources {
                let shared = shared.clone();
                scope.spawn(move || {
                    let token = Token {
                        cursor: 2,
                        epoch: "epoch".into(),
                        full_scan_at_ms: crate::now_ms(),
                    };
                    let guard = shared.lock().unwrap();
                    guard
                        .as_ref()
                        .unwrap()
                        .save(source, &metadata(), &token)
                        .unwrap();
                });
            }
        });
        let guard = shared.lock().unwrap();
        let c = guard.as_ref().unwrap();
        let entries = c.library.list().unwrap().entries;
        assert!(
            !entries.is_empty() && entries.len() <= 3,
            "{}",
            entries.len()
        );
        let mut bytes = 0;
        for entry in &entries {
            bytes += c.archive_bytes(&entry.id).unwrap();
            // Every survivor is a whole, loadable record for its own root.
            let source = sources
                .iter()
                .find(|s| s.root.to_string_lossy() == entry.root_path)
                .unwrap();
            let loaded = c
                .load(&source.root, &source.node(0).unwrap().identity, "epoch")
                .unwrap()
                .unwrap();
            assert_eq!(loaded.snapshot.stats.entries, source.stats.entries);
        }
        assert!(bytes <= limit);
        // Nothing half written is left behind.
        let leftovers = fs::read_dir(&c.root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }
    // Decoding an archive can take seconds, so it must not hold the lock other scans publish through.
    #[test]
    fn a_candidate_decodes_after_the_lock_is_released_and_a_pruned_one_fails_cleanly() {
        for (count_limit, pruned) in [(1, true), (8, false)] {
            let d = tempfile::tempdir().unwrap();
            let mut c = Cache::new(d.path().join("cache"));
            c.count_limit = count_limit;
            let a = fixture(&d.path().join("a"));
            let b = fixture(&d.path().join("b"));
            let identity = a.node(0).unwrap().identity.clone();
            let token = Token {
                cursor: 42,
                epoch: "epoch".into(),
                full_scan_at_ms: crate::now_ms(),
            };
            c.save(&a, &metadata(), &token).unwrap();
            let shared = std::sync::Mutex::new(Some(c));
            let pending = {
                let guard = shared.lock().unwrap();
                let cache = guard.as_ref().unwrap();
                let candidate = cache.probe(&a.root, &identity, "epoch").unwrap().unwrap();
                cache
                    .begin_load(candidate, &a.root, &identity, "epoch")
                    .unwrap()
            };
            // Another scan publishes before this one decodes: the lock is free for it.
            shared
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .save(&b, &metadata(), &token)
                .unwrap();
            match pending.decode(&a.root, &identity) {
                Ok(loaded) => {
                    assert!(!pruned);
                    assert_eq!(loaded.snapshot.stats.entries, a.stats.entries);
                    assert_eq!(loaded.token.cursor, 42);
                }
                Err(_) => assert!(pruned, "a kept entry must decode"),
            }
        }
    }
    #[test]
    fn preflight_does_not_decode_and_rejects_changed_token_before_loading() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path().join("cache"));
        let s = fixture(&d.path().join("source"));
        let token = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        c.save(&s, &metadata(), &token).unwrap();
        let candidate = c
            .probe(&s.root, &s.node(0).unwrap().identity, &token.epoch)
            .unwrap()
            .unwrap();
        fs::write(c.root.join(format!("{}.zst", candidate.id)), b"corrupt").unwrap();
        // Probe can decide whether events already require full traversal without
        // parsing even a corrupt body. Loading it still enforces all validations.
        assert!(c
            .probe(&s.root, &s.node(0).unwrap().identity, &token.epoch)
            .unwrap()
            .is_some());
        assert!(c
            .load_candidate(
                candidate,
                &s.root,
                &s.node(0).unwrap().identity,
                &token.epoch
            )
            .is_err());
        c.save(&s, &metadata(), &token).unwrap();
        let candidate = c
            .probe(&s.root, &s.node(0).unwrap().identity, &token.epoch)
            .unwrap()
            .unwrap();
        let mut record = c.record(&candidate.id).unwrap();
        record.token.cursor += 1;
        fs::write(
            c.path(&candidate.id).unwrap(),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(c
            .load_candidate(
                candidate,
                &s.root,
                &s.node(0).unwrap().identity,
                &token.epoch
            )
            .err()
            .unwrap()
            .contains("changed during"));
    }
    #[test]
    fn failed_cache_update_keeps_previous_seed() {
        let d = tempfile::tempdir().unwrap();
        let mut c = Cache::new(d.path().join("cache"));
        let s = fixture(&d.path().join("source"));
        let token = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        c.save(&s, &metadata(), &token).unwrap();
        // Force a publication failure without the retired selection write.
        c.bytes_limit = 1;
        assert!(c.save(&s, &metadata(), &token).is_err());
        let previous = c
            .load(&s.root, &s.node(0).unwrap().identity, &token.epoch)
            .unwrap()
            .unwrap();
        assert_eq!(previous.token.cursor, 42);
        assert_eq!(c.library.list().unwrap().entries.len(), 1);
    }
    #[test]
    fn unchanged_refresh_preserves_archive_binding_age_and_private_selection() {
        use std::os::unix::fs::MetadataExt;
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path().join("cache"));
        let s = fixture(&d.path().join("source"));
        let m = metadata();
        let previous = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: m.started_at_ms,
        };
        c.save(&s, &m, &previous).unwrap();
        let mut row = c.library.list().unwrap().entries.remove(0);
        // An old LRU timestamp proves refresh advances only publication age.
        row.saved_at_ms = 1;
        fs::write(
            c.root.join(format!("{}.json", row.id)),
            serde_json::to_vec(&row).unwrap(),
        )
        .unwrap();
        let archive = c.root.join(format!("{}.zst", row.id));
        let bytes = fs::read(&archive).unwrap();
        let inode = fs::metadata(&archive).unwrap().ino();
        assert!(!c.root.join("selection.json").exists());
        // Preserve a selection left by an older private-cache writer.
        fs::write(
            c.root.join("selection.json"),
            serde_json::to_vec(&Some(row.id.clone())).unwrap(),
        )
        .unwrap();
        let selection = fs::read(c.root.join("selection.json")).unwrap();
        let history = Library::new(d.path().join("history"));
        let historical = history.save(&s, &m, |_| {}).unwrap();
        let historical_bytes = fs::read(
            d.path()
                .join("history")
                .join(format!("{}.zst", historical.id)),
        )
        .unwrap();
        let historical_selection = fs::read(d.path().join("history/selection.json")).unwrap();
        let next = Token {
            cursor: 84,
            ..previous.clone()
        };
        assert!(c
            .refresh_unchanged(&s.root, &s.node(0).unwrap().identity, &previous, &next)
            .unwrap());
        assert_eq!(fs::read(&archive).unwrap(), bytes);
        assert_eq!(fs::metadata(&archive).unwrap().ino(), inode);
        assert_eq!(fs::read(c.root.join("selection.json")).unwrap(), selection);
        let loaded = c
            .load(&s.root, &s.node(0).unwrap().identity, &next.epoch)
            .unwrap()
            .unwrap();
        assert!(loaded.token == next);
        assert_eq!(loaded.token.full_scan_at_ms, previous.full_scan_at_ms);
        let (_, loaded_metadata) = c.library.load_cache(&row.id).unwrap();
        assert_eq!(loaded_metadata.started_at_ms, m.started_at_ms);
        let touched = c.library.list().unwrap().entries.remove(0);
        assert_eq!(touched.started_at_ms, row.started_at_ms);
        assert_eq!(touched.completed_at_ms, row.completed_at_ms);
        assert_eq!(touched.file_bytes, row.file_bytes);
        assert!(touched.saved_at_ms > row.saved_at_ms);
        assert_eq!(
            fs::read(
                d.path()
                    .join("history")
                    .join(format!("{}.zst", historical.id))
            )
            .unwrap(),
            historical_bytes
        );
        assert_eq!(
            fs::read(d.path().join("history/selection.json")).unwrap(),
            historical_selection
        );
        assert!(!recent(
            &next,
            m.started_at_ms,
            previous.full_scan_at_ms + 3_600_001
        ));
    }
    #[test]
    fn unchanged_refresh_rejects_missing_replaced_mismatched_and_expired_seeds() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path().join("cache"));
        let s = fixture(&d.path().join("source"));
        let m = metadata();
        let previous = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: m.started_at_ms,
        };
        let next = Token {
            cursor: 84,
            ..previous.clone()
        };
        assert!(!c
            .refresh_unchanged(&s.root, &s.node(0).unwrap().identity, &previous, &next)
            .unwrap());
        c.save(&s, &m, &previous).unwrap();
        let row = c.library.list().unwrap().entries.remove(0);
        let original = fs::read(c.path(&row.id).unwrap()).unwrap();
        for bad in [
            Token {
                cursor: 1,
                ..next.clone()
            },
            Token {
                epoch: "other".into(),
                ..next.clone()
            },
            Token {
                full_scan_at_ms: previous.full_scan_at_ms + 1,
                ..next.clone()
            },
        ] {
            assert!(!c
                .refresh_unchanged(&s.root, &s.node(0).unwrap().identity, &previous, &bad)
                .unwrap());
        }
        let wrong_previous = Token {
            cursor: 43,
            ..previous.clone()
        };
        assert!(!c
            .refresh_unchanged(
                &s.root,
                &s.node(0).unwrap().identity,
                &wrong_previous,
                &next
            )
            .unwrap());
        assert_eq!(fs::read(c.path(&row.id).unwrap()).unwrap(), original);
        // A newer tree publication is authoritative; stale memory cannot refresh it.
        c.save(&s, &m, &next).unwrap();
        assert!(!c
            .refresh_unchanged(&s.root, &s.node(0).unwrap().identity, &previous, &next)
            .unwrap());
        let row = c.library.list().unwrap().entries.remove(0);
        let mut record = c.record(&row.id).unwrap();
        record.token.full_scan_at_ms = 1;
        fs::write(
            c.path(&row.id).unwrap(),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let expired = record.token.clone();
        let next_expired = Token {
            cursor: 100,
            ..expired.clone()
        };
        assert!(!c
            .refresh_unchanged(
                &s.root,
                &s.node(0).unwrap().identity,
                &expired,
                &next_expired
            )
            .unwrap());
    }
    #[test]
    fn legacy_optimization_cache_is_discarded_without_migrating_history() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path().join("cache"));
        let s = fixture(&d.path().join("source"));
        let token = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        c.save(&s, &metadata(), &token).unwrap();
        let row = c.library.list().unwrap().entries.remove(0);
        let mut record = c.record(&row.id).unwrap();
        record.version = 1;
        fs::write(
            c.path(&row.id).unwrap(),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(c
            .load(&s.root, &s.node(0).unwrap().identity, &token.epoch)
            .unwrap()
            .is_none());
        assert!(c.library.list().unwrap().entries.is_empty());
    }
    #[test]
    fn persisted_hardlinks_retain_own_sizes_and_global_allocation_owner() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("source");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("a"), vec![7; 8192]).unwrap();
        fs::hard_link(root.join("a"), root.join("b")).unwrap();
        let s = disk_scan::scan(
            root,
            disk_scan::ScanOptions::default(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        let c = Cache::new(d.path().join("cache"));
        let token = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        assert!(eligible(&s));
        c.save(&s, &metadata(), &token).unwrap();
        let loaded = c
            .load(&s.root, &s.node(0).unwrap().identity, &token.epoch)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.snapshot.stats.allocated_bytes,
            s.stats.allocated_bytes
        );
        let links: Vec<_> = loaded
            .snapshot
            .iter_nodes()
            .filter(|n| n.hard_link)
            .collect();
        assert_eq!(links.len(), 2);
        assert!(links.iter().all(|n| n.own_allocated_size > 0));
        assert_eq!(links.iter().filter(|n| n.allocated_size == 0).count(), 1);
        assert_eq!(
            links.iter().map(|n| n.allocated_size).sum::<u64>(),
            links[0].own_allocated_size
        );
    }
    #[test]
    fn partial_results_are_rejected_and_hardlinks_are_eligible() {
        let d = tempfile::tempdir().unwrap();
        let mut s = fixture(&d.path().join("a"));
        assert!(eligible(&s));
        s.stats.incomplete = true;
        assert!(!eligible(&s));
        s.stats.incomplete = false;
        s.update_node(1, |n| n.hard_link = true).unwrap();
        assert!(eligible(&s));
        s.update_node(1, |n| n.hard_link = false).unwrap();
        s.stats.excluded = 1;
        assert!(!eligible(&s));
        s.stats.excluded = 0;
        s.state = ScanState::Cancelled;
        assert!(!eligible(&s));
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    #[test]
    fn original_full_age_is_not_reset_by_incremental_or_clock_rollback() {
        let token = Token {
            cursor: 1,
            epoch: "epoch".into(),
            full_scan_at_ms: 1000,
        };
        assert!(recent(&token, 2000, 3000));
        assert!(!recent(&token, 2000, 1500));
        assert!(!recent(&token, 500, 1000));
        assert!(!recent(&token, 3_601_001, 3_601_001));
    }
    #[test]
    fn maintenance_removes_only_owned_interrupted_cache_files() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path().join("cache"));
        c.library.list().unwrap();
        for ext in ["tmp", "zst", "json", "cursor"] {
            fs::write(
                c.root.join(format!("{}.{}", uuid::Uuid::new_v4(), ext)),
                b"orphan",
            )
            .unwrap();
        }
        fs::write(c.root.join("unrelated.txt"), b"keep").unwrap();
        c.maintain().unwrap();
        assert!(c.root.join("unrelated.txt").exists());
        assert_eq!(fs::read_dir(&c.root).unwrap().count(), 1);
    }
}
