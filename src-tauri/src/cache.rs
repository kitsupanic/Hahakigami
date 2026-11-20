//! Private, bounded optimization cache.
use crate::cache_store::{
    atomic, open, sync_dir, unusable, verified_nonshared, ScanMetadata, Store,
};
use disk_scan::{Identity, ScanState, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    io::{BufReader, Read},
    os::unix::ffi::OsStrExt,
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
    store: Store,
    id: String,
    token: Token,
    started_at_ms: u64,
}
impl Pending {
    pub fn decode(self, root: &Path, identity: &Identity) -> Result<CachedScan, String> {
        let (snapshot, metadata) = self.store.load(&self.id)?;
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
    store: Store,
    count_limit: usize,
    bytes_limit: u64,
}
pub fn eligible(s: &Snapshot) -> bool {
    s.incremental_accounting_eligible()
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
impl Record {
    /// The check every use of a record repeats: same root, identity and epoch, and still `recent`.
    fn matches(&self, root: &Path, identity: &Identity, epoch: &str) -> bool {
        self.root == root.as_os_str().as_bytes()
            && &self.identity == identity
            && self.token.epoch == epoch
            && recent(&self.token, self.started_at_ms, crate::now_ms())
    }
}
impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self {
            store: Store::new(root.clone()),
            root,
            count_limit: 8,
            bytes_limit: 512 * 1024 * 1024,
        }
    }
    pub fn contains_storage(&self, scope: &Path) -> bool {
        self.root.starts_with(scope)
    }
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        self.store.path(id, "cursor")
    }
    fn record(&self, id: &str) -> Result<Record, String> {
        self.read_record(id).map_err(|e| e.to_string())
    }
    /// `Err` is `unusable` for a missing or invalid record; any other error is a failed read, not a verdict.
    fn read_record(&self, id: &str) -> io::Result<Record> {
        let file = open(&self.path(id).map_err(io::Error::other)?)?;
        if file.metadata()?.len() > 16 * 1024 {
            return Err(io::Error::other("Invalid cache record"));
        }
        let r: Record = serde_json::from_reader(BufReader::new(file.take(16 * 1024)))?;
        if r.version != 3
            || !verified_nonshared(&r.accounting)
            || r.token.cursor == 0
            || r.token.epoch.is_empty()
            || r.token.epoch.len() > 256
            || r.token.full_scan_at_ms == 0
            || r.token.full_scan_at_ms > r.started_at_ms
        {
            return Err(io::Error::other("Unsupported cache record"));
        }
        Ok(r)
    }
    /// The newest entry that matches, with the record it was matched on.
    fn find(
        &self,
        root: &Path,
        identity: &Identity,
        epoch: &str,
    ) -> Result<Option<(String, Record)>, String> {
        self.maintain()?;
        for row in self.store.list()? {
            if let Ok(record) = self.record(&row.id) {
                if record.matches(root, identity, epoch) {
                    return Ok(Some((row.id, record)));
                }
            }
        }
        Ok(None)
    }
    pub fn probe(
        &self,
        root: &Path,
        identity: &Identity,
        epoch: &str,
    ) -> Result<Option<Candidate>, String> {
        Ok(self
            .find(root, identity, epoch)?
            .map(|(id, record)| Candidate {
                id,
                token: record.token,
                started_at_ms: record.started_at_ms,
            }))
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
        if !record.matches(root, identity, epoch)
            || record.token != candidate.token
            || record.started_at_ms != candidate.started_at_ms
        {
            return Err("Rescan cache changed during event replay".into());
        }
        Ok(Pending {
            store: self.store.clone(),
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
        let file = open(&self.store.path(id, "zst")?).map_err(|e| e.to_string())?;
        Ok(file.metadata().map_err(|e| e.to_string())?.len())
    }
    fn maintain(&self) -> Result<(), String> {
        let entries = self.store.list()?;
        let now = crate::now_ms();
        // An entry `recent` refuses or whose record is missing or invalid can never seed a scan again: keep
        // only the others, so the sweep below deletes the rest (row, record and archive share the ID's stem).
        // A record that merely failed to read (EMFILE, EIO) is not dead: stop and delete nothing.
        let mut ids = std::collections::HashSet::new();
        for row in &entries {
            match self.read_record(&row.id) {
                Ok(r) if recent(&r.token, r.started_at_ms, now) => {
                    ids.insert(row.id.as_str());
                }
                Ok(_) => (),
                Err(e) if unusable(&e) => (),
                Err(e) => return Err(e.to_string()),
            }
        }
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
                // One undeletable leftover (EPERM, `uchg`) must not fail every later
                // pass; the next one retries it, as the prune loop in `save` does.
                let _ = fs::remove_file(&path);
            }
        }
        sync_dir(&self.root).map_err(|e| e.to_string())
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        self.store.delete(id)?;
        match fs::remove_file(self.path(id)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
    fn write_record(&self, id: &str, record: &Record) -> Result<(), String> {
        // A post-rename directory-sync warning is dropped: `maintain` syncs the directory again before every use.
        atomic(&self.root, &self.path(id)?, "", |mut file| {
            serde_json::to_writer(&mut file, record).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())
        })
        .map(drop)
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
        // The caller holds the Baselines cache lock, so nothing publishes between the match and the write.
        let Some((id, mut record)) = self.find(root, identity, &previous.epoch)? else {
            return Ok(false);
        };
        if record.token != *previous {
            return Ok(false);
        }
        record.token = next.clone();
        self.write_record(&id, &record)?;
        self.store.touch(&id, record.started_at_ms)?;
        Ok(true)
    }
    pub fn save(&self, s: &Snapshot, m: &ScanMetadata, token: &Token) -> Result<(), String> {
        if !eligible(s) || token.cursor == 0 || token.epoch.is_empty() || token.epoch.len() > 256 {
            return Err("This result cannot seed incremental scans".into());
        }
        self.maintain()?;
        let row = self.store.save(s, m)?;
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
        // The new entry is published: from here only its own failures fail the save. An old entry that cannot
        // be read or removed is skipped, and the next `save` enforces the caps on it again.
        let Ok(mut entries) = self.store.list() else {
            return Ok(());
        };
        entries.sort_by_key(|entry| entry.id != row.id);
        for entry in entries {
            let new = entry.id == row.id;
            let Ok(actual_bytes) = self.archive_bytes(&entry.id) else {
                if new {
                    return Err("Could not read the saved rescan cache".into());
                }
                continue;
            };
            let record = match self.read_record(&entry.id) {
                Ok(r) => Some(r),
                Err(e) if unusable(&e) => None,
                Err(_) => continue,
            };
            let same = record
                .as_ref()
                .is_some_and(|r| r.root == s.root.as_os_str().as_bytes());
            let discard = record.is_none()
                || (!new && same)
                || count >= self.count_limit
                || bytes.saturating_add(actual_bytes) > self.bytes_limit;
            if discard {
                if let Err(e) = self.remove(&entry.id) {
                    if new {
                        return Err(e);
                    }
                }
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
            workers: 1,
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
    fn reload_matches_identity_and_epoch() {
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
        let row = c.store.list().unwrap().remove(0);
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
        assert_eq!(c.store.list().unwrap().len(), 1);
        for n in ["b", "c"] {
            let s = fixture(&d.path().join(n));
            c.save(&s, &metadata(), &token).unwrap();
        }
        assert_eq!(c.store.list().unwrap().len(), 2);
        let s = fixture(&d.path().join("d"));
        c.bytes_limit = 1;
        assert!(c.save(&s, &metadata(), &token).is_err());
        assert_eq!(c.store.list().unwrap().len(), 2);
        c.bytes_limit = 512 * 1024 * 1024;
        c.save(&s, &metadata(), &token).unwrap();
        let id = c
            .probe(&s.root, &s.node(0).unwrap().identity, &token.epoch)
            .unwrap()
            .unwrap()
            .id;
        fs::write(c.root.join(format!("{id}.zst")), b"corrupt").unwrap();
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
        let one = c.store.list().unwrap()[0].file_bytes;
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
        let entries = c.store.list().unwrap();
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
                .find(|s| {
                    c.probe(&s.root, &s.node(0).unwrap().identity, "epoch")
                        .unwrap()
                        .is_some_and(|k| k.id == entry.id)
                })
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
        assert_eq!(c.store.list().unwrap().len(), 1);
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
        let mut row = c.store.list().unwrap().remove(0);
        // An old LRU timestamp proves refresh advances only publication age.
        row.used_at_ms = 1;
        fs::write(
            c.root.join(format!("{}.json", row.id)),
            serde_json::to_vec(&row).unwrap(),
        )
        .unwrap();
        let archive = c.root.join(format!("{}.zst", row.id));
        let bytes = fs::read(&archive).unwrap();
        let inode = fs::metadata(&archive).unwrap().ino();
        let next = Token {
            cursor: 84,
            ..previous.clone()
        };
        assert!(c
            .refresh_unchanged(&s.root, &s.node(0).unwrap().identity, &previous, &next)
            .unwrap());
        assert_eq!(fs::read(&archive).unwrap(), bytes);
        assert_eq!(fs::metadata(&archive).unwrap().ino(), inode);
        let loaded = c
            .load(&s.root, &s.node(0).unwrap().identity, &next.epoch)
            .unwrap()
            .unwrap();
        assert!(loaded.token == next);
        assert_eq!(loaded.token.full_scan_at_ms, previous.full_scan_at_ms);
        let (_, loaded_metadata) = c.store.load(&row.id).unwrap();
        assert_eq!(loaded_metadata.started_at_ms, m.started_at_ms);
        let touched = c.store.list().unwrap().remove(0);
        assert_eq!(touched.started_at_ms, row.started_at_ms);
        assert_eq!(touched.file_bytes, row.file_bytes);
        assert!(touched.used_at_ms > row.used_at_ms);
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
        let row = c.store.list().unwrap().remove(0);
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
        let row = c.store.list().unwrap().remove(0);
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
    fn legacy_optimization_cache_is_discarded() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path().join("cache"));
        let s = fixture(&d.path().join("source"));
        let token = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        c.save(&s, &metadata(), &token).unwrap();
        let row = c.store.list().unwrap().remove(0);
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
        assert!(c.store.list().unwrap().is_empty());
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
    #[test]
    fn maintenance_deletes_expired_entries_and_keeps_recent_ones() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path().join("cache"));
        let s = fixture(&d.path().join("source"));
        let token = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        c.save(&s, &metadata(), &token).unwrap();
        let kept = c.store.list().unwrap().remove(0).id;
        // A second entry for another root whose full scan is long past (expired, not corrupt).
        let other = fixture(&d.path().join("other"));
        let m = ScanMetadata {
            started_at_ms: 2000,
            ..metadata()
        };
        let old_token = Token {
            full_scan_at_ms: 1000,
            ..token.clone()
        };
        c.save(&other, &m, &old_token).unwrap();
        let expired = c
            .store
            .list()
            .unwrap()
            .into_iter()
            .find(|row| row.id != kept)
            .unwrap()
            .id;
        c.maintain().unwrap();
        assert_eq!(c.store.list().unwrap().len(), 1);
        assert!(c.record(&kept).is_ok());
        for suffix in ["json", "zst", "cursor"] {
            assert!(!c.root.join(format!("{expired}.{suffix}")).exists());
            assert!(c.root.join(format!("{kept}.{suffix}")).exists());
        }
    }
    #[test]
    fn maintenance_deletes_only_entries_that_can_never_be_read() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path().join("cache"));
        let s = fixture(&d.path().join("source"));
        let token = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        c.save(&s, &metadata(), &token).unwrap();
        let id = c.store.list().unwrap().remove(0).id;
        let cursor = c.root.join(format!("{id}.cursor"));
        // A read that fails this time is not a verdict on the entry; one that fails every time is.
        use std::io::Error;
        for transient in [
            libc::EMFILE,
            libc::ENFILE,
            libc::EIO,
            libc::EAGAIN,
            libc::EINTR,
        ] {
            assert!(!crate::cache_store::unusable(&Error::from_raw_os_error(
                transient
            )));
        }
        for dead in [libc::ENOENT, libc::ELOOP, libc::EACCES, libc::EPERM] {
            assert!(crate::cache_store::unusable(&Error::from_raw_os_error(
                dead
            )));
        }
        assert!(crate::cache_store::unusable(&Error::other(
            "not a regular file"
        )));
        // A record that parses to nothing is dead: the entry goes.
        fs::write(&cursor, b"not json").unwrap();
        c.maintain().unwrap();
        for suffix in ["json", "zst", "cursor"] {
            assert!(!c.root.join(format!("{id}.{suffix}")).exists());
        }
    }
    // An immutable old record cannot be removed (macOS `uchg`), standing in for a failed removal.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_save_succeeds_when_an_old_entry_cannot_be_removed() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache::new(d.path().join("cache"));
        let s = fixture(&d.path().join("source"));
        let token = Token {
            cursor: 42,
            epoch: "epoch".into(),
            full_scan_at_ms: crate::now_ms(),
        };
        c.save(&s, &metadata(), &token).unwrap();
        let old = c.store.list().unwrap().remove(0).id;
        let cursor = c.root.join(format!("{old}.cursor"));
        let flag = |arg: &str| {
            assert!(std::process::Command::new("chflags")
                .args([arg])
                .arg(&cursor)
                .status()
                .unwrap()
                .success())
        };
        flag("uchg");
        // Same root: the new entry replaces the old one, whose removal fails.
        let saved = c.save(&s, &metadata(), &token);
        // The undeletable leftover is still there: the next pass must not fail on it.
        let again = c.save(&s, &metadata(), &token);
        flag("nouchg");
        saved.unwrap();
        again.unwrap();
        assert_eq!(c.store.list().unwrap().len(), 1);
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
        c.store.list().unwrap();
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
