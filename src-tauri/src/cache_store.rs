//! Files of the private rescan cache: per entry a compact scan archive (`.zst`) and a row (`.json`).
//! Callers serialize operations and own what a cached scan may be used for.
use disk_scan::{AccountingInfo, ScanState, Snapshot};
use serde::{Deserialize, Serialize};
use std::os::unix::{
    ffi::OsStrExt,
    fs::{OpenOptionsExt, PermissionsExt},
};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

// Lossless fast mode for disposable caches.
const CACHE_COMPRESSION_LEVEL: i32 = -1;
const PARALLEL_CACHE_MIN_NODES: usize = 4096;
const MAX_NODES: usize = 8_000_000;
// Decoded bytes are streamed; this is a budget, not a buffer allocation.
const MAX_CACHE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// The row stored beside each archive; `Cache` lists, orders and prunes entries by it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Row {
    pub id: String,
    pub started_at_ms: u64,
    // Last write or refresh: the recency order.
    pub used_at_ms: u64,
    pub file_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ScanMetadata {
    pub started_at_ms: u64,
    pub workers: usize,
}
#[derive(Clone)]
pub(crate) struct Store {
    root: PathBuf,
    node_limit: usize,
    byte_limit: u64,
}
pub(crate) fn err(e: impl fmt::Display) -> String {
    e.to_string()
}
pub(crate) fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .and_then(|f| {
            if f.metadata()?.is_file() {
                Ok(f)
            } else {
                Err(io::Error::other("not a regular file"))
            }
        })
}
/// Whether a failed read means the file can never be used rather than that the read failed this time. Only the
/// listed errors pass on their own; any other (missing, a symlink, malformed, EACCES) would refuse it on every read,
/// so a caller may discard it.
pub(crate) fn unusable(e: &io::Error) -> bool {
    !matches!(
        e.raw_os_error(),
        Some(libc::EMFILE | libc::ENFILE | libc::EIO | libc::EAGAIN | libc::EINTR | libc::ENOMEM)
    )
}
/// Persist directory entries (renames, removals) to disk.
pub(crate) fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path).and_then(|d| d.sync_all())
}
/// Writes `path` through a private temp file `<prefix><uuid>.tmp` in `dir` and renames it into place.
/// `Ok(Some(warning))`: the file is published but the directory sync after the rename failed.
pub(crate) fn atomic(
    dir: &Path,
    path: &Path,
    prefix: &str,
    write: impl FnOnce(File) -> Result<(), String>,
) -> Result<Option<String>, String> {
    let temp = dir.join(format!("{prefix}{}.tmp", uuid::Uuid::new_v4()));
    let f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temp)
        .map_err(err)?;
    let result = write(f).and_then(|()| fs::rename(&temp, path).map_err(err));
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result?;
    Ok(sync_dir(dir).err().map(err))
}
impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            node_limit: MAX_NODES,
            byte_limit: MAX_CACHE_BYTES,
        }
    }
    fn prepare(&self) -> Result<(), String> {
        if !self.root.exists() {
            fs::create_dir_all(&self.root).map_err(err)?;
        }
        let m = fs::symlink_metadata(&self.root).map_err(err)?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err("Cache directory is not a private directory".into());
        }
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700)).map_err(err)
    }
    pub(crate) fn path(&self, id: &str, suffix: &str) -> Result<PathBuf, String> {
        let uuid = uuid::Uuid::parse_str(id).map_err(|_| "Invalid cache ID")?;
        if uuid.to_string() != id {
            return Err("Invalid cache ID".into());
        }
        Ok(self.root.join(format!("{id}.{suffix}")))
    }
    // A parse error becomes an `io::Error` without an OS code (`unusable`); a failed read keeps its own.
    fn json<T: serde::de::DeserializeOwned>(&self, path: &Path) -> io::Result<T> {
        Ok(serde_json::from_reader(BufReader::new(
            open(path)?.take(1024 * 1024),
        ))?)
    }
    // The sync-failure warning is dropped: `Cache::maintain` syncs the directory again before every use.
    fn write_json(&self, path: &Path, value: &impl Serialize) -> Result<(), String> {
        atomic(&self.root, path, "", |mut f| {
            serde_json::to_writer(&mut f, value).map_err(err)?;
            f.sync_all().map_err(err)
        })
        .map(drop)
    }
    /// The entries with a readable row and archive, newest first. A file that cannot be read this time
    /// (not one that is missing or malformed) fails the listing instead of dropping its entry.
    pub fn list(&self) -> Result<Vec<Row>, String> {
        self.prepare()?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(err)? {
            let path = entry.map_err(err)?.path();
            if path.extension().is_some_and(|s| s == "json") {
                let row = match self.json::<Row>(&path) {
                    Ok(row) => row,
                    Err(e) if unusable(&e) => continue,
                    Err(e) => return Err(err(e)),
                };
                if !self.path(&row.id, "json").is_ok_and(|p| p == path) {
                    continue;
                }
                let Ok(archive) = self.path(&row.id, "zst") else {
                    continue;
                };
                match open(&archive) {
                    Ok(_) => entries.push(row),
                    Err(e) if unusable(&e) => (),
                    Err(e) => return Err(err(e)),
                }
            }
        }
        entries.sort_by(|a, b| {
            b.used_at_ms
                .cmp(&a.used_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(entries)
    }
    pub fn save(&self, s: &Snapshot, m: &ScanMetadata) -> Result<Row, String> {
        self.prepare()?;
        let parallel = parallel_cache_eligible(s.node_count(), m.workers);
        if parallel {
            validate_header(s, self.node_limit)?;
        } else {
            validate(s, self.node_limit)?;
        }
        validate_metadata(m)?;
        let id = uuid::Uuid::new_v4().to_string();
        let archive = self.path(&id, "zst")?;
        atomic(&self.root, &archive, "", |f| {
            validated_encoding(
                parallel,
                || validate(s, self.node_limit),
                || {
                    let mut encoder = zstd::stream::write::Encoder::new(
                        BufWriter::new(f),
                        CACHE_COMPRESSION_LEVEL,
                    )
                    .map_err(err)?;
                    encoder.include_checksum(true).map_err(err)?;
                    let limited = LimitedWriter {
                        inner: &mut encoder,
                        count: 0,
                        limit: self.byte_limit,
                    };
                    let mut buffered =
                        BufWriter::with_capacity(crate::cache_codec::WRITE_BATCH, limited);
                    crate::cache_codec::encode(&mut buffered, s, m)?;
                    buffered.flush().map_err(err)?;
                    drop(buffered);
                    let mut writer = encoder.finish().map_err(err)?;
                    writer.flush().map_err(err)?;
                    writer.get_ref().sync_all().map_err(err)
                },
            )
        })?; // Warning dropped as in `write_json`.
        let row = Row {
            id: id.clone(),
            started_at_ms: m.started_at_ms,
            used_at_ms: crate::now_ms(),
            file_bytes: open(&archive).map_err(err)?.metadata().map_err(err)?.len(),
        };
        if let Err(e) = self.write_json(&self.path(&id, "json")?, &row) {
            let _ = fs::remove_file(archive);
            return Err(e);
        }
        Ok(row)
    }
    pub fn load(&self, id: &str) -> Result<(Snapshot, ScanMetadata), String> {
        self.prepare()?;
        let _: Row = self.json(&self.path(id, "json")?).map_err(err)?;
        let mut file = BufReader::new(open(&self.path(id, "zst")?).map_err(err)?);
        let mut header = [0; 5];
        file.read_exact(&mut header).map_err(err)?;
        if header[..4] != [0x28, 0xb5, 0x2f, 0xfd] || header[4] & 4 == 0 {
            return Err("Cache has no valid checksum frame".into());
        }
        use std::io::Seek;
        file.rewind().map_err(err)?;
        let mut decoder = zstd::stream::read::Decoder::new(&mut file)
            .map_err(err)?
            .single_frame();
        decoder.window_log_max(27).map_err(err)?;
        let limited = LimitedReader {
            inner: decoder,
            count: 0,
            limit: self.byte_limit,
        };
        let mut buffered = BufReader::with_capacity(64 * 1024, limited);
        let result = crate::cache_codec::decode(&mut buffered, self.node_limit)?;
        let mut compressed = buffered.into_inner().inner.finish();
        let mut extra = [0];
        if compressed.read(&mut extra).map_err(err)? != 0 {
            return Err("Cache contains trailing compressed data".into());
        }
        validate(&result.0, self.node_limit)?;
        validate_metadata(&result.1)?;
        Ok(result)
    }
    /// Refreshes only the row's recency.
    pub fn touch(&self, id: &str, started_at_ms: u64) -> Result<(), String> {
        self.prepare()?;
        let path = self.path(id, "json")?;
        let mut row: Row = self.json(&path).map_err(err)?;
        if row.id != id || row.started_at_ms != started_at_ms {
            return Err("Cache metadata changed during cursor refresh".into());
        }
        open(&self.path(id, "zst")?).map_err(err)?;
        row.used_at_ms = crate::now_ms();
        self.write_json(&path, &row)
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.prepare()?;
        let metadata = self.path(id, "json")?;
        let archive = self.path(id, "zst")?;
        for path in [&metadata, &archive] {
            match open(path) {
                Ok(_) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(err(e)),
            }
        }
        // Persist archive removal first so an interruption never leaves a hidden archive.
        for path in [&archive, &metadata] {
            match fs::remove_file(path) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(err(e)),
            }
            sync_dir(&self.root).map_err(err)?;
        }
        Ok(())
    }
}
struct LimitedWriter<W> {
    inner: W,
    count: u64,
    limit: u64,
}
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if b.len() as u64 > self.limit - self.count {
            return Err(io::Error::other("Cache exceeds decoded byte limit"));
        }
        let n = self.inner.write(b)?;
        self.count += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
struct LimitedReader<R> {
    inner: R,
    count: u64,
    limit: u64,
}
impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let cap = b
            .len()
            .min((self.limit - self.count).saturating_add(1) as usize);
        let n = self.inner.read(&mut b[..cap])?;
        if n as u64 > self.limit - self.count {
            return Err(io::Error::other("Cache exceeds decoded byte limit"));
        }
        self.count += n as u64;
        Ok(n)
    }
}
fn parallel_cache_eligible(nodes: usize, workers: usize) -> bool {
    workers > 1 && nodes >= PARALLEL_CACHE_MIN_NODES
}

// The atomic writer may rename only after BOTH operations succeed. Even encoding
// failure joins the validator; a panic is an error rather than a published cache.
fn validated_encoding(
    parallel: bool,
    validation: impl Fn() -> Result<(), String> + Sync,
    encoding: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if !parallel {
        return encoding(); // Caller already validated the sequential path.
    }
    std::thread::scope(|scope| {
        match std::thread::Builder::new()
            .name("cache-validation".into())
            .spawn_scoped(scope, &validation)
        {
            Ok(worker) => {
                let encoded = encoding();
                worker
                    .join()
                    .map_err(|_| "Cache validation thread panicked".to_string())??;
                encoded
            }
            Err(_) => sequential_validated_encoding(&validation, encoding),
        }
    })
}

fn sequential_validated_encoding(
    validation: impl FnOnce() -> Result<(), String>,
    encoding: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    validation()?;
    encoding()
}

/// A full-clone accounting whose checks completed and found nothing shared or deducted.
pub(crate) fn verified_nonshared(a: &AccountingInfo) -> bool {
    a.model == disk_scan::AccountingModel::FullClonesV1
        && a.checks_complete
        && !a.shared
        && a.warnings.is_empty()
        && a.deduction_bytes == 0
}
fn validate_metadata(m: &ScanMetadata) -> Result<(), String> {
    if !(1..=disk_scan::MAX_WORKERS).contains(&m.workers) {
        Err("Invalid cache metadata".into())
    } else {
        Ok(())
    }
}
fn validate_header(s: &Snapshot, limit: usize) -> Result<(), String> {
    if s.state != ScanState::Completed {
        return Err("Only completed scans can be cached".into());
    }
    if s.node_count() == 0 || s.node_count() > limit {
        return Err("Cache exceeds node limit or has no root".into());
    }
    let root = s.node(0).ok_or("Missing root")?;
    if root.parent.is_some()
        || root.path()? != s.root
        || !s.root.is_absolute()
        || !root.is_directory
    {
        return Err("Invalid cache root".into());
    }
    Ok(())
}
pub(crate) fn validate(s: &Snapshot, limit: usize) -> Result<(), String> {
    validate_header(s, limit)?;
    if s.accounting.warnings.len() > 32 || s.accounting.warnings.iter().any(|w| w.len() > 4096) {
        return Err("Cache accounting warnings exceed limit".into());
    }
    let root = s.node(0).ok_or("Missing root")?;
    let mut seen = vec![false; s.node_count()];
    seen[0] = true;
    let mut directories = 0u64;
    let ordinary_paths = cfg!(unix) && !s.has_path_overrides();
    if ordinary_paths && s.root.as_os_str().as_bytes().contains(&0) {
        return Err("Invalid cache node".into());
    }
    let mut paths = s.path_cursor();
    for (i, n) in s.iter_nodes().enumerate() {
        if n.id as usize != i
            || n.is_directory != (n.identity.kind == disk_scan::NodeKind::Directory)
        {
            return Err("Invalid cache node".into());
        }
        directories += u64::from(n.is_directory);
        if i > 0 {
            let p = n.parent.ok_or("Disconnected cache node")? as usize;
            if p >= i || !s.node(p as u32).ok_or("Invalid parent")?.is_directory {
                return Err("Invalid cache parent/path".into());
            }
        }
        if ordinary_paths {
            // Structural induction: the root is valid and every preceding
            // parent is a directory. One exact Normal basename defines each
            // child's path and its lexical parent without rebuilding prefixes.
            // Duplicate sibling names remain accepted.
            if i > 0 && s.native_component(n.id)?.as_bytes().contains(&0) {
                return Err("Invalid cache node".into());
            }
        } else {
            // A supplied legacy spelling remains authoritative. Validate its
            // complete bytes and parent relation before accepting the archive.
            let (path, parent_path) = paths.paths(n.id)?;
            if path.as_os_str().as_bytes().contains(&0) {
                return Err("Invalid cache node".into());
            }
            if i > 0 && path.parent() != parent_path {
                return Err("Invalid cache parent/path".into());
            }
        }
        for &child in n.children() {
            let c = child as usize;
            if !n.is_directory
                || c <= i
                || c >= s.node_count()
                || seen[c]
                || s.node(c as u32).ok_or("Invalid child")?.parent != Some(n.id)
            {
                return Err("Invalid cache child topology".into());
            }
            seen[c] = true;
        }
    }
    if seen.iter().any(|s| !*s) {
        return Err("Unreachable cache node".into());
    }
    let disk_scan::AllocationPlan {
        values: mut allocated,
        clone_deduction_bytes: deduction,
        ..
    } = disk_scan::allocation_plan(s)?;
    if deduction != s.accounting.deduction_bytes
        || (s.iter_nodes().any(|n| n.clone_metadata().is_some()) && !s.accounting.shared)
    {
        return Err("Invalid cache accounting summary".into());
    }

    for i in (0..s.node_count()).rev() {
        let n = s.node(i as u32).ok_or("Invalid node")?;
        if allocated[i] != n.allocated_size {
            return Err("Invalid cache size counters".into());
        }
        if let Some(p) = n.parent {
            allocated[p as usize] = allocated[p as usize].saturating_add(allocated[i]);
        }
    }
    if s.stats.entries != s.node_count() as u64
        || s.stats.directories != directories
        || s.stats.allocated_bytes != root.allocated_size
        || s.stats.errors < s.errors.len() as u64
        || s.stats.excluded < s.exclusions.len() as u64
        || s.stats.restricted_system_issues > s.stats.errors
    {
        return Err("Invalid cache statistics".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use disk_scan::{Identity, Node};
    use std::os::unix::ffi::OsStringExt;
    use std::sync::{atomic::AtomicBool, Arc};
    fn fixture() -> (tempfile::TempDir, Store, Snapshot, ScanMetadata) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("source");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("file"), b"abc").unwrap();
        let s = disk_scan::scan(
            root,
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        let m = ScanMetadata {
            started_at_ms: 1,
            workers: 1,
        };
        let l = Store::new(d.path().join("cache"));
        (d, l, s, m)
    }
    fn compressed(data: &[u8]) -> Vec<u8> {
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }
    #[test]
    fn parallel_cache_gate_preserves_single_worker_and_small_paths() {
        assert!(!parallel_cache_eligible(PARALLEL_CACHE_MIN_NODES, 1));
        assert!(!parallel_cache_eligible(PARALLEL_CACHE_MIN_NODES - 1, 4));
        assert!(parallel_cache_eligible(PARALLEL_CACHE_MIN_NODES, 2));
    }
    #[test]
    fn failed_spawn_fallback_validates_before_encoding() {
        let encoded = std::cell::Cell::new(false);
        assert_eq!(
            sequential_validated_encoding(
                || Err("invalid topology".into()),
                || {
                    encoded.set(true);
                    Ok(())
                }
            )
            .unwrap_err(),
            "invalid topology"
        );
        assert!(!encoded.get());
        sequential_validated_encoding(
            || Ok(()),
            || {
                encoded.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(encoded.get());
    }
    #[test]
    fn parallel_production_cache_rejects_invalid_snapshot_and_encoding_limit() {
        let (_d, l, small, mut m) = fixture();
        for index in 0..PARALLEL_CACHE_MIN_NODES {
            fs::write(small.root.join(format!("file-{index}")), b"a").unwrap();
        }
        let s = disk_scan::scan(
            small.root,
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        m.workers = 4;
        let mut invalid = s.clone();
        invalid.update_node(1, |n| n.parent = None).unwrap();
        assert!(l.save(&invalid, &m).is_err());
        assert_eq!(fs::read_dir(&l.root).unwrap().count(), 0);
        let mut limited = Store::new(l.root.clone());
        limited.byte_limit = 32;
        assert!(limited.save(&s, &m).unwrap_err().contains("byte limit"));
        assert_eq!(fs::read_dir(&l.root).unwrap().count(), 0);
        let row = l.save(&s, &m).unwrap();
        assert_eq!(l.load(&row.id).unwrap().0.node_count(), s.node_count());
    }
    #[test]
    fn parallel_validation_failure_cannot_publish_even_after_encoding() {
        let (_d, l, _s, _m) = fixture();
        l.prepare().unwrap();
        let archive = l.root.join("invalid.zst");
        let encoded = std::sync::atomic::AtomicBool::new(false);
        let result = atomic(&l.root, &archive, "", |mut file| {
            validated_encoding(
                true,
                || {
                    while !encoded.load(std::sync::atomic::Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    Err("late invalid topology".into())
                },
                || {
                    file.write_all(b"encoded successfully").map_err(err)?;
                    encoded.store(true, std::sync::atomic::Ordering::Release);
                    Ok(())
                },
            )
        });
        assert_eq!(result.unwrap_err(), "late invalid topology");
        assert_eq!(fs::read_dir(&l.root).unwrap().count(), 0);
    }
    #[test]
    fn parallel_encoding_failure_joins_and_validation_error_wins() {
        let joined = std::sync::atomic::AtomicBool::new(false);
        let result = validated_encoding(
            true,
            || {
                joined.store(true, std::sync::atomic::Ordering::Release);
                Err("invalid topology".into())
            },
            || Err("byte limit".into()),
        );
        assert_eq!(result.unwrap_err(), "invalid topology");
        assert!(joined.load(std::sync::atomic::Ordering::Acquire));
        assert!(
            validated_encoding(true, || panic!("validator failure"), || Ok(()))
                .unwrap_err()
                .contains("panicked")
        );
    }
    #[test]
    fn nonregular_cache_archives_are_rejected_without_blocking() {
        let (_d, l, s, m) = fixture();
        let row = l.save(&s, &m).unwrap();
        let path = l.path(&row.id, "zst").unwrap();
        fs::remove_file(&path).unwrap();
        let native = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(native.as_ptr(), 0o600) }, 0);
        assert!(l.load(&row.id).unwrap_err().contains("not a regular file"));
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(l.load(&row.id).unwrap_err().contains("not a regular file"));
    }
    #[test]
    fn cache_checksum_truncation_and_trailing_data_are_rejected() {
        let (_d, l, s, m) = fixture();
        let row = l.save(&s, &m).unwrap();
        let path = l.path(&row.id, "zst").unwrap();
        let valid = fs::read(&path).unwrap();
        let (loaded, _) = l.load(&row.id).unwrap();
        assert_eq!(loaded.node_count(), s.node_count());
        for len in [0, 4, valid.len() - 1] {
            fs::write(&path, &valid[..len]).unwrap();
            assert!(l.load(&row.id).is_err());
        }
        let mut bad = valid.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        fs::write(&path, bad).unwrap();
        assert!(l.load(&row.id).is_err());
        for suffix in [vec![0], compressed(b"extra")] {
            let mut bad = valid.clone();
            bad.extend(suffix);
            fs::write(&path, bad).unwrap();
            assert!(l.load(&row.id).unwrap_err().contains("trailing compressed"));
        }
        let mut decoded = zstd::stream::decode_all(valid.as_slice()).unwrap();
        decoded.push(0);
        fs::write(&path, compressed(&decoded)).unwrap();
        assert!(l.load(&row.id).unwrap_err().contains("trailing decoded"));
        fs::write(&path, &valid).unwrap();
        let mut limited = Store::new(l.root.clone());
        limited.byte_limit = 32;
        assert!(limited.load(&row.id).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("missing", &path).unwrap();
        assert!(l.load(&row.id).is_err());
    }
    #[test]
    fn compact_final_tail_enforces_byte_limit_without_publication() {
        let (_d, l, s, m) = fixture();
        let mut encoded = Vec::new();
        crate::cache_codec::encode(&mut encoded, &s, &m).unwrap();
        let mut limited = Store::new(l.root.clone());
        limited.byte_limit = encoded.len() as u64 - 1;
        assert!(limited.save(&s, &m).unwrap_err().contains("byte limit"));
        assert_eq!(fs::read_dir(&l.root).unwrap().count(), 0);
        limited.byte_limit += 1;
        let row = limited.save(&s, &m).unwrap();
        assert!(limited.load(&row.id).is_ok());
    }
    #[test]
    fn invalid_snapshots_and_tampered_records_are_rejected() {
        let (_d, l, s, m) = fixture();
        let cache = l.save(&s, &m).unwrap();
        assert!(l.load(&cache.id).is_ok());
        let mut invalid = s.clone();
        invalid.update_node(1, |n| n.parent = None).unwrap();
        assert!(l.save(&invalid, &m).is_err());
        let mut decoded = zstd::stream::decode_all(
            fs::read(l.path(&cache.id, "zst").unwrap())
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        let header_len = u32::from_le_bytes(decoded[8..12].try_into().unwrap()) as usize;
        let record = 12 + header_len + 4;
        decoded[record..record + 4].copy_from_slice(&9u32.to_le_bytes());
        fs::write(l.path(&cache.id, "zst").unwrap(), compressed(&decoded)).unwrap();
        assert!(l.load(&cache.id).is_err());
    }
    #[test]
    fn delete_tolerates_missing_halves_and_is_repeatable() {
        let (_d, l, s, m) = fixture();
        let row = l.save(&s, &m).unwrap();
        fs::remove_file(l.path(&row.id, "zst").unwrap()).unwrap();
        l.delete(&row.id).unwrap();
        assert!(l.list().unwrap().is_empty());
        l.delete(&row.id).unwrap();
        assert!(l.delete("not-an-id").is_err());
    }
    #[test]
    fn rows_written_with_the_old_fields_still_load() {
        let (_d, l, s, m) = fixture();
        let row = l.save(&s, &m).unwrap();
        let json = format!(
            r#"{{"id":"{}","name":"n","rootPath":"/r","startedAtMs":{},"completedAtMs":2,"usedAtMs":{},"entries":3,"allocatedBytes":4,"incomplete":false,"errors":0,"excluded":0,"fileBytes":{}}}"#,
            row.id, row.started_at_ms, row.used_at_ms, row.file_bytes
        );
        fs::write(l.path(&row.id, "json").unwrap(), json).unwrap();
        assert_eq!(l.list().unwrap()[0].id, row.id);
        assert!(l.load(&row.id).is_ok());
    }
    #[test]
    fn rows_are_listed_newest_first() {
        let (_d, l, s, m) = fixture();
        let first = l.save(&s, &m).unwrap();
        let second = l.save(&s, &m).unwrap();
        let json = fs::read_to_string(l.path(&first.id, "json").unwrap()).unwrap();
        assert!(json.contains("\"usedAtMs\""));
        let mut row = first.clone();
        row.used_at_ms = second.used_at_ms + 1;
        l.write_json(&l.path(&first.id, "json").unwrap(), &row)
            .unwrap();
        let ids: Vec<_> = l.list().unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, [first.id, second.id]);
    }

    // Independent original full-path validator: no compact accessors or path
    // reconstruction are used for its node/path/topology/accounting decisions.
    fn original_full_path_accepts(s: &Snapshot, nodes: &[Node]) -> bool {
        if s.state != ScanState::Completed || nodes.is_empty() || nodes.len() > MAX_NODES {
            return false;
        }
        let root = &nodes[0];
        if root.parent.is_some()
            || root.path != s.root
            || !s.root.is_absolute()
            || !root.is_directory
        {
            return false;
        }
        let mut seen = vec![false; nodes.len()];
        seen[0] = true;
        let mut allocated = vec![0u64; nodes.len()];
        let mut directories = 0u64;
        let mut owners = std::collections::HashMap::new();
        for (i, node) in nodes.iter().enumerate().filter(|(_, node)| node.hard_link) {
            let owner = owners
                .entry((node.identity.device, node.identity.inode))
                .or_insert(i);
            if node.path < nodes[*owner].path {
                *owner = i;
            }
        }
        for (i, node) in nodes.iter().enumerate() {
            if node.id as usize != i
                || node.is_directory != (node.identity.kind == disk_scan::NodeKind::Directory)
                || node.path.as_os_str().as_bytes().contains(&0)
            {
                return false;
            }
            directories += u64::from(node.is_directory);
            if i > 0 {
                let Some(parent) = node.parent.map(|parent| parent as usize) else {
                    return false;
                };
                if parent >= i
                    || !nodes[parent].is_directory
                    || node.path.parent() != Some(nodes[parent].path.as_path())
                {
                    return false;
                }
            }
            for &child in &node.children {
                let child = child as usize;
                if !node.is_directory
                    || child <= i
                    || child >= nodes.len()
                    || seen[child]
                    || nodes[child].parent != Some(node.id)
                {
                    return false;
                }
                seen[child] = true;
            }
            allocated[i] = if node.hard_link
                && owners.get(&(node.identity.device, node.identity.inode)) != Some(&i)
            {
                0
            } else {
                node.own_allocated_size
            };
        }
        if seen.iter().any(|seen| !seen) {
            return false;
        }
        for i in (0..nodes.len()).rev() {
            if allocated[i] != nodes[i].allocated_size {
                return false;
            }
            if let Some(parent) = nodes[i].parent {
                allocated[parent as usize] =
                    allocated[parent as usize].saturating_add(allocated[i]);
            }
        }
        s.stats.entries == nodes.len() as u64
            && s.stats.directories == directories
            && s.stats.allocated_bytes == root.allocated_size
            && s.stats.errors >= s.errors.len() as u64
            && s.stats.excluded >= s.exclusions.len() as u64
            && s.stats.restricted_system_issues <= s.stats.errors
    }

    fn ordinary_oracle_nodes(root: &Path) -> Vec<Node> {
        let topology = [
            (None, "", vec![1, 2], 12),
            (Some(0), "a", vec![3, 4], 8),
            (Some(0), "b", vec![5], 4),
            (Some(1), "a/file", vec![], 4),
            (Some(1), "a/other", vec![], 4),
            (Some(2), "b/file", vec![], 4),
        ];
        topology
            .into_iter()
            .enumerate()
            .map(|(id, (parent, relative, children, size))| {
                let directory = id < 3;
                let path = if id == 0 {
                    root.to_path_buf()
                } else {
                    root.join(relative)
                };
                Node {
                    id: id as u32,
                    parent,
                    children,
                    name: path.file_name().unwrap().to_string_lossy().into_owned(),
                    path,
                    identity: Identity {
                        device: 1,
                        inode: id as u64 + 1,
                        kind: if directory {
                            disk_scan::NodeKind::Directory
                        } else {
                            disk_scan::NodeKind::File
                        },
                    },
                    allocated_size: size,
                    length: if directory { 0 } else { 4 },
                    own_allocated_size: if directory { 0 } else { 4 },
                    is_directory: directory,
                    hard_link: false,
                    excluded: false,
                    clone_metadata: None,
                }
            })
            .collect()
    }
    fn pack_oracle_nodes(root: &Path, nodes: &[Node]) -> Snapshot {
        let mut snapshot = Snapshot::empty(root.into());
        for node in nodes {
            snapshot.append_legacy_node(node.clone()).unwrap();
        }
        snapshot.state = ScanState::Completed;
        snapshot.stats.entries = nodes.len() as u64;
        snapshot.stats.directories = 3;
        snapshot.stats.allocated_bytes = nodes[0].allocated_size;
        snapshot
    }
    fn assert_original_validation_parity(snapshot: &Snapshot, original: &[Node], expected: bool) {
        assert_eq!(original_full_path_accepts(snapshot, original), expected);
        assert_eq!(validate(snapshot, MAX_NODES).is_ok(), expected);
    }

    #[test]
    fn ordinary_validation_matches_original_full_path_policy_and_failures() {
        let root = Path::new("/scope");
        let baseline = ordinary_oracle_nodes(root);
        let snapshot = pack_oracle_nodes(root, &baseline);
        assert!(!snapshot.has_path_overrides());
        assert_original_validation_parity(&snapshot, &baseline, true);
        for name in [b"native-\xff".as_slice(), b"native-\0".as_slice()] {
            let mut nodes = baseline.clone();
            nodes[3].path = root
                .join("a")
                .join(std::ffi::OsString::from_vec(name.to_vec()));
            nodes[3].name = nodes[3]
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let snapshot = pack_oracle_nodes(root, &nodes);
            assert!(!snapshot.has_path_overrides());
            assert_original_validation_parity(&snapshot, &nodes, !name.contains(&0));
        }
        // Validation permits duplicate sibling names, unlike seed reuse.
        let mut duplicate = baseline.clone();
        duplicate[4].path = duplicate[3].path.clone();
        duplicate[4].name = duplicate[3].name.clone();
        let snapshot = pack_oracle_nodes(root, &duplicate);
        assert!(!snapshot.has_path_overrides());
        assert_original_validation_parity(&snapshot, &duplicate, true);
        for parent in [None, Some(3), Some(5)] {
            let mut nodes = baseline.clone();
            nodes[3].parent = parent;
            let mut snapshot = pack_oracle_nodes(root, &baseline);
            snapshot
                .update_node(3, |node| node.parent = parent)
                .unwrap();
            assert_original_validation_parity(&snapshot, &nodes, false);
        }
        let mut nodes = baseline.clone();
        nodes[5].parent = Some(3); // Earlier file IDs are not directory parents.
        let mut snapshot = pack_oracle_nodes(root, &baseline);
        snapshot
            .update_node(5, |node| node.parent = Some(3))
            .unwrap();
        assert_original_validation_parity(&snapshot, &nodes, false);
        let mut collision = baseline.clone();
        for (id, name) in [
            (3, b"native-\xff".as_slice()),
            (4, b"native-\xfe".as_slice()),
        ] {
            collision[id].path = root
                .join("a")
                .join(std::ffi::OsString::from_vec(name.to_vec()));
            collision[id].name = collision[id]
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
        }
        assert_eq!(collision[3].name, collision[4].name);
        assert_ne!(collision[3].path, collision[4].path);
        let snapshot = pack_oracle_nodes(root, &collision);
        assert!(!snapshot.has_path_overrides());
        assert_original_validation_parity(&snapshot, &collision, true);
        // A changed parent requires reciprocal children and recomputed totals.
        let mut moved = baseline.clone();
        moved[4].parent = Some(2);
        moved[4].path = root.join("b/other");
        let snapshot = pack_oracle_nodes(root, &moved);
        assert_original_validation_parity(&snapshot, &moved, false);
        moved[1].children = vec![3];
        moved[2].children = vec![4, 5];
        moved[1].allocated_size = 4;
        moved[2].allocated_size = 8;
        let snapshot = pack_oracle_nodes(root, &moved);
        assert_original_validation_parity(&snapshot, &moved, true);
        let mut wrong_wire = baseline.clone();
        wrong_wire[4].path = root.join("b/other");
        let snapshot = pack_oracle_nodes(root, &wrong_wire);
        assert!(snapshot.has_path_overrides());
        assert_original_validation_parity(&snapshot, &wrong_wire, false);
        for suffix in ["//", "/./"] {
            let mut nodes = baseline.clone();
            nodes[0].path = PathBuf::from(format!("/scope{suffix}"));
            let snapshot = pack_oracle_nodes(root, &nodes);
            assert!(snapshot.has_path_overrides());
            assert_original_validation_parity(&snapshot, &nodes, true);
        }
        let mut links = baseline.clone();
        links[3].hard_link = true;
        links[5].hard_link = true;
        links[5].identity = links[3].identity.clone();
        links[5].allocated_size = 0;
        links[2].allocated_size = 0;
        links[0].allocated_size = 8;
        let snapshot = pack_oracle_nodes(root, &links);
        assert_original_validation_parity(&snapshot, &links, true);
        // Effective root bytes, not the obsolete stored root basename, govern
        // NUL validation after a public root change on an ordinary tree.
        let bad_root = PathBuf::from(std::ffi::OsString::from_vec(b"/scope\0".to_vec()));
        let bad_nodes = ordinary_oracle_nodes(&bad_root);
        let mut snapshot = pack_oracle_nodes(&bad_root, &bad_nodes);
        assert_original_validation_parity(&snapshot, &bad_nodes, false);
        snapshot.root = root.into();
        assert!(!snapshot.has_path_overrides());
        assert!(snapshot.native_name(0).unwrap().as_bytes().contains(&0));
        assert_original_validation_parity(&snapshot, &baseline, true);
        let mut snapshot = pack_oracle_nodes(root, &baseline);
        snapshot.root = bad_root;
        assert_original_validation_parity(&snapshot, &bad_nodes, false);
    }
}
