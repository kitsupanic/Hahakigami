//! App-private historical scans. Callers serialize operations and own session authority.
use disk_scan::{
    AccountingInfo, AccountingModel, CloneMetadata, Identity, LegacyTreeBuilder, Node, ScanIssue,
    ScanQos, ScanState, ScanStats, Snapshot, WorkerStats,
};
use serde::{
    de::{SeqAccess, Visitor},
    ser::SerializeSeq,
    Deserialize, Serialize, Serializer,
};
use std::os::unix::{
    ffi::{OsStrExt, OsStringExt},
    fs::{OpenOptionsExt, PermissionsExt},
};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
#[path = "archive_codec.rs"]
mod archive_codec;
#[path = "binary_archive.rs"]
mod binary_archive;
#[path = "cache_codec.rs"]
mod cache_codec;

// Lossless fast mode for disposable caches; historical archives retain level 3.
const CACHE_COMPRESSION_LEVEL: i32 = -1;
const PARALLEL_CACHE_MIN_NODES: usize = 4096;
const MAX_NODES: usize = 8_000_000;
// Decoded bytes are streamed; this is an archive budget, not a buffer allocation.
const MAX_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedEntry {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub saved_at_ms: u64,
    pub entries: u64,
    pub logical_bytes: u64,
    pub allocated_bytes: u64,
    pub incomplete: bool,
    pub errors: u64,
    pub excluded: u64,
    pub file_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ScanMetadata {
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub workers: usize,
    pub worker_mode: String,
    pub worker_source: String,
    pub qos: ScanQos,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct List {
    pub entries: Vec<SavedEntry>,
    pub last_opened_id: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Library {
    root: PathBuf,
    node_limit: usize,
    byte_limit: u64,
    fast_record_limit: usize,
}
fn err(e: impl fmt::Display) -> String {
    e.to_string()
}
fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .and_then(|f| {
            if f.metadata()?.is_file() {
                Ok(f)
            } else {
                Err(io::Error::other("library file is not regular"))
            }
        })
}
impl Library {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            node_limit: MAX_NODES,
            byte_limit: MAX_BYTES,
            fast_record_limit: archive_codec::MAX_RECORD_BYTES,
        }
    }
    fn cache_byte_limit(&self) -> u64 {
        self.byte_limit.min(MAX_CACHE_BYTES)
    }
    fn prepare(&self) -> Result<(), String> {
        if !self.root.exists() {
            fs::create_dir_all(&self.root).map_err(err)?;
        }
        let m = fs::symlink_metadata(&self.root).map_err(err)?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err("Library directory is not a private directory".into());
        }
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700)).map_err(err)
    }
    fn path(&self, id: &str, suffix: &str) -> Result<PathBuf, String> {
        let uuid = uuid::Uuid::parse_str(id).map_err(|_| "Invalid saved scan ID")?;
        if uuid.to_string() != id {
            return Err("Invalid saved scan ID".into());
        }
        Ok(self.root.join(format!("{id}.{suffix}")))
    }
    fn json<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Result<T, String> {
        serde_json::from_reader(BufReader::new(open(path).map_err(err)?.take(1024 * 1024)))
            .map_err(err)
    }
    fn atomic(
        &self,
        path: &Path,
        write: impl FnOnce(File) -> Result<(), String>,
    ) -> Result<(), String> {
        let temp = self.root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        let f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)
            .map_err(err)?;
        let result = (|| {
            write(f)?;
            fs::rename(&temp, path).map_err(err)?;
            File::open(&self.root)
                .and_then(|d| d.sync_all())
                .map_err(err)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    fn write_json(&self, path: &Path, value: &impl Serialize) -> Result<(), String> {
        self.atomic(path, |mut f| {
            serde_json::to_writer(&mut f, value).map_err(err)?;
            f.sync_all().map_err(err)
        })
    }
    pub fn list(&self) -> Result<List, String> {
        self.prepare()?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(err)? {
            let path = entry.map_err(err)?.path();
            if path.extension().is_some_and(|s| s == "json")
                && path.file_stem().is_some_and(|s| s != "selection")
            {
                if let Ok(row) = self.json::<SavedEntry>(&path) {
                    if self.path(&row.id, "json").is_ok_and(|p| p == path)
                        && self.path(&row.id, "zst").is_ok_and(|p| open(&p).is_ok())
                    {
                        entries.push(row)
                    }
                }
            }
        }
        entries.sort_by(|a, b| {
            b.saved_at_ms
                .cmp(&a.saved_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        let last_opened_id = self
            .json::<Option<String>>(&self.root.join("selection.json"))
            .unwrap_or(None);
        Ok(List {
            entries,
            last_opened_id,
        })
    }
    pub fn save(
        &self,
        s: &Snapshot,
        m: &ScanMetadata,
        mut progress: impl FnMut(u64),
    ) -> Result<SavedEntry, String> {
        self.save_format(s, m, &mut progress, false, None)
    }
    pub fn save_cache(&self, s: &Snapshot, m: &ScanMetadata) -> Result<SavedEntry, String> {
        self.save_format(s, m, &mut |_| {}, true, None)
    }
    fn save_format(
        &self,
        s: &Snapshot,
        m: &ScanMetadata,
        progress: &mut impl FnMut(u64),
        compact: bool,
        parallel_override: Option<bool>,
    ) -> Result<SavedEntry, String> {
        self.save_format_selection(s, m, progress, compact, parallel_override, !compact)
    }
    fn save_format_selection(
        &self,
        s: &Snapshot,
        m: &ScanMetadata,
        progress: &mut impl FnMut(u64),
        compact: bool,
        parallel_override: Option<bool>,
        select: bool,
    ) -> Result<SavedEntry, String> {
        self.prepare()?;
        let eligible = parallel_cache_eligible(compact, s.node_count(), m.workers);
        let parallel = eligible && parallel_override.unwrap_or(true);
        if parallel {
            validate_header(s, self.node_limit)?;
            validate_metadata(m)?;
        } else {
            validate(s, self.node_limit)?;
            validate_metadata(m)?;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let archive = self.path(&id, "zst")?;
        self.atomic(&archive, |f| {
            validated_encoding(
                parallel,
                || validate(s, self.node_limit),
                || {
                    let mut encoder = zstd::stream::write::Encoder::new(
                        BufWriter::new(f),
                        if compact { CACHE_COMPRESSION_LEVEL } else { 3 },
                    )
                    .map_err(err)?;
                    encoder.include_checksum(true).map_err(err)?;
                    let limited = LimitedWriter {
                        inner: &mut encoder,
                        count: 0,
                        limit: if compact {
                            self.cache_byte_limit()
                        } else {
                            self.byte_limit
                        },
                        progress,
                    };
                    let mut buffered = BufWriter::with_capacity(cache_codec::WRITE_BATCH, limited);
                    if compact {
                        cache_codec::encode(&mut buffered, s, m)?;
                    } else {
                        binary_archive::encode(&mut buffered, s, m)?;
                    }
                    buffered.flush().map_err(err)?;
                    drop(buffered);
                    let mut writer = encoder.finish().map_err(err)?;
                    writer.flush().map_err(err)?;
                    writer.get_ref().sync_all().map_err(err)
                },
            )
        })?;
        let row = SavedEntry {
            id: id.clone(),
            name: s
                .root
                .file_name()
                .unwrap_or(s.root.as_os_str())
                .to_string_lossy()
                .into_owned(),
            root_path: s.root.to_string_lossy().into_owned(),
            started_at_ms: m.started_at_ms,
            completed_at_ms: m.completed_at_ms,
            saved_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(err)?
                .as_millis() as u64,
            entries: s.stats.entries,
            logical_bytes: s.stats.logical_bytes,
            allocated_bytes: s.stats.allocated_bytes,
            incomplete: s.stats.incomplete,
            errors: s.stats.errors,
            excluded: s.stats.excluded,
            file_bytes: open(&archive).map_err(err)?.metadata().map_err(err)?.len(),
        };
        if let Err(e) = self.write_json(&self.path(&id, "json")?, &row) {
            let _ = fs::remove_file(archive);
            return Err(e);
        }
        // Private caches are addressed by their cursor, not historical selection.
        if let Err(selection_error) = if !select {
            Ok(())
        } else {
            self.mark_opened(&id)
        } {
            // A failed save must not leave a library entry that retries duplicate.
            let rollback = (|| {
                let metadata_result = fs::remove_file(self.path(&id, "json")?).map_err(err);
                let archive_result = fs::remove_file(&archive).map_err(err);
                let sync_result = File::open(&self.root)
                    .and_then(|directory| directory.sync_all())
                    .map_err(err);
                metadata_result.and(archive_result).and(sync_result)
            })();
            return Err(match rollback {
                Ok(()) => selection_error,
                Err(rollback_error) => {
                    format!("{selection_error}; saved scan rollback failed: {rollback_error}")
                }
            });
        }
        Ok(row)
    }
    pub fn load(
        &self,
        id: &str,
        mut progress: impl FnMut(u64),
    ) -> Result<(Snapshot, ScanMetadata), String> {
        self.prepare()?;
        let _: SavedEntry = self.json(&self.path(id, "json")?)?;
        self.load_path(&self.path(id, "zst")?, &mut progress, None, &mut || {})
    }
    pub fn load_cancellable(
        &self,
        id: &str,
        mut progress: impl FnMut(u64),
        cancelled: &AtomicBool,
        mut validating: impl FnMut(),
    ) -> Result<(Snapshot, ScanMetadata), String> {
        check_open_cancelled(Some(cancelled))?;
        self.prepare()?;
        let _: SavedEntry = self.json(&self.path(id, "json")?)?;
        self.load_path(
            &self.path(id, "zst")?,
            &mut progress,
            Some(cancelled),
            &mut validating,
        )
    }
    fn load_path(
        &self,
        path: &Path,
        progress: &mut impl FnMut(u64),
        cancelled: Option<&AtomicBool>,
        validating: &mut impl FnMut(),
    ) -> Result<(Snapshot, ScanMetadata), String> {
        check_open_cancelled(cancelled)?;
        let mut file = BufReader::new(open(path).map_err(err)?);
        // Require a standard checksummed frame, rather than accepting an unchecked archive.
        let mut header = [0; 5];
        file.read_exact(&mut header).map_err(err)?;
        if header[..4] != [0x28, 0xb5, 0x2f, 0xfd] || header[4] & 4 == 0 {
            return Err("Saved scan has no valid checksum frame".into());
        }
        use std::io::Seek;
        file.rewind().map_err(err)?;
        // Sniff the decoded magic; a sniff failure falls through to the JSON path's own errors.
        let binary = sniff_binary_archive(&mut file).unwrap_or(false);
        file.rewind().map_err(err)?;
        let (mut s, metadata) = if binary {
            let decoded = self.decode_binary(&mut file, progress, cancelled)?;
            validating();
            check_open_cancelled(cancelled)?;
            decoded
        } else {
            let archive = match self.decode_archive(&mut file, progress, true, cancelled) {
                Ok(archive) => archive,
                Err(archive_codec::DecodeError::LegacyRequired) => {
                    // Reuse the same opened descriptor: a renamed/replaced archive
                    // cannot change which file is validated during fallback.
                    file.rewind().map_err(err)?;
                    self.decode_archive(&mut file, progress, false, cancelled)
                        .map_err(|error| match error {
                            archive_codec::DecodeError::Invalid(message) => message,
                            archive_codec::DecodeError::LegacyRequired => {
                                "Invalid legacy saved scan".into()
                            }
                        })?
                }
                Err(archive_codec::DecodeError::Invalid(message)) => return Err(message),
            };
            validating();
            check_open_cancelled(cancelled)?;
            if !matches!(archive.version, 1 | 2) {
                return Err("Unsupported saved scan version".into());
            }
            if archive.version == 2 && archive.snapshot.accounting.is_none() {
                return Err("Version-two archive requires accounting evidence".into());
            }
            check_open_cancelled(cancelled)?;
            let s = archive.snapshot.into_snapshot()?;
            if archive.version == 1
                && (s.accounting.model != AccountingModel::HardLinksV1
                    || s.iter_nodes().any(|n| n.clone_metadata().is_some()))
            {
                return Err("Version-one archives cannot contain clone accounting".into());
            }
            (s, archive.metadata)
        };
        let attributions = validated_clone_attributions(&s, self.node_limit)?;
        s.set_planned_clone_attributions(attributions);
        validate_metadata(&metadata)?;
        check_open_cancelled(cancelled)?;
        Ok((s, metadata))
    }
    /// Import historical archives only; cache wire records are never portable scans.
    pub fn import_scan(
        &self,
        source: &Path,
        mut progress: impl FnMut(u64),
    ) -> Result<SavedEntry, String> {
        let (snapshot, metadata) = self.load_path(source, &mut progress, None, &mut || {})?;
        self.save_format_selection(&snapshot, &metadata, &mut progress, false, None, false)
    }
    /// Publish a validated canonical historical archive at the selected destination.
    pub fn export_scan(
        &self,
        id: &str,
        destination: &Path,
        mut progress: impl FnMut(u64),
    ) -> Result<(), String> {
        let (snapshot, metadata) = self.load(id, &mut progress)?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or("Export destination must have a parent directory")?;
        let parent = fs::canonicalize(parent).map_err(err)?;
        if parent.starts_with(fs::canonicalize(&self.root).map_err(err)?) {
            return Err("Cannot export into the private scan library".into());
        }
        let name = destination.file_name().ok_or("Invalid export filename")?;
        let destination = parent.join(name);
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err("Export destination must be a regular file".into());
            }
            Ok(metadata) => {
                use std::os::unix::fs::MetadataExt;
                let source = open(&self.path(id, "zst")?)
                    .map_err(err)?
                    .metadata()
                    .map_err(err)?;
                if metadata.dev() == source.dev() && metadata.ino() == source.ino() {
                    return Err("Cannot overwrite the saved scan source".into());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(err(error)),
        }
        let temp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)
            .map_err(err)?;
        let result = (|| {
            let mut encoder =
                zstd::stream::write::Encoder::new(BufWriter::new(file), 3).map_err(err)?;
            encoder.include_checksum(true).map_err(err)?;
            {
                let mut writer = LimitedWriter {
                    inner: &mut encoder,
                    count: 0,
                    limit: self.byte_limit,
                    progress: &mut progress,
                };
                serde_json::to_writer(
                    &mut writer,
                    &ArchiveRef {
                        version: 2,
                        metadata: &metadata,
                        snapshot: SnapshotRef(&snapshot),
                    },
                )
                .map_err(err)?;
            }
            let mut writer = encoder.finish().map_err(err)?;
            writer.flush().map_err(err)?;
            writer.get_ref().sync_all().map_err(err)?;
            // Rename replaces the directory entry, never writes through its target.
            fs::rename(&temp, &destination).map_err(err)?;
            File::open(&parent)
                .and_then(|directory| directory.sync_all())
                .map_err(err)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
    pub fn load_cache(&self, id: &str) -> Result<(Snapshot, ScanMetadata), String> {
        self.prepare()?;
        let _: SavedEntry = self.json(&self.path(id, "json")?)?;
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
            cancelled: None,
            inner: decoder,
            count: 0,
            limit: self.cache_byte_limit(),
            progress: &mut |_| {},
        };
        let mut buffered = BufReader::with_capacity(64 * 1024, limited);
        let result = cache_codec::decode(&mut buffered, self.node_limit)?;
        let mut compressed = buffered.into_inner().inner.finish();
        let mut extra = [0];
        if compressed.read(&mut extra).map_err(err)? != 0 {
            return Err("Cache contains trailing compressed data".into());
        }
        validate(&result.0, self.node_limit)?;
        validate_metadata(&result.1)?;
        Ok(result)
    }
    fn decode_binary(
        &self,
        file: &mut BufReader<File>,
        progress: &mut impl FnMut(u64),
        cancelled: Option<&AtomicBool>,
    ) -> Result<(Snapshot, ScanMetadata), String> {
        let mut decoder = zstd::stream::read::Decoder::new(file)
            .map_err(err)?
            .single_frame();
        decoder.window_log_max(27).map_err(err)?;
        let limited = LimitedReader {
            cancelled,
            inner: decoder,
            count: 0,
            limit: self.byte_limit,
            progress,
        };
        let mut buffered = BufReader::with_capacity(256 * 1024, limited);
        let result = binary_archive::decode(&mut buffered, self.node_limit)?;
        let mut compressed = buffered.into_inner().inner.finish();
        let mut extra = [0];
        if compressed.read(&mut extra).map_err(err)? != 0 {
            return Err("Saved scan contains trailing compressed data".into());
        }
        Ok(result)
    }
    fn decode_archive(
        &self,
        file: &mut BufReader<File>,
        progress: &mut impl FnMut(u64),
        fast: bool,
        cancelled: Option<&AtomicBool>,
    ) -> Result<Archive, archive_codec::DecodeError> {
        let mut decoder = zstd::stream::read::Decoder::new(file)
            .map_err(err)?
            .single_frame();
        decoder.window_log_max(27).map_err(err)?;
        let limited = LimitedReader {
            cancelled,
            inner: archive_codec::StringLimitedReader::new(decoder, archive_codec::MAX_FIELD_BYTES),
            count: 0,
            limit: self.byte_limit,
            progress,
        };
        let mut buffered = BufReader::new(limited);
        let archive = if fast {
            archive_codec::decode_with_limit(
                &mut buffered,
                self.node_limit,
                self.fast_record_limit,
            )?
        } else {
            let mut deserializer = serde_json::Deserializer::from_reader(&mut buffered);
            let archive = Archive::deserialize(&mut deserializer).map_err(err)?;
            deserializer.end().map_err(err)?;
            archive
        };
        let mut compressed = buffered.into_inner().inner.inner.finish();
        let mut extra = [0];
        if compressed.read(&mut extra).map_err(err)? != 0 {
            return Err("Saved scan contains trailing compressed data".into());
        }
        Ok(archive)
    }
    /// Update only the private optimization-cache LRU row, never archive/selection.
    pub fn touch_cache(&self, id: &str, started_at_ms: u64) -> Result<(), String> {
        self.prepare()?;
        let path = self.path(id, "json")?;
        let mut row: SavedEntry = self.json(&path)?;
        if row.id != id || row.started_at_ms != started_at_ms {
            return Err("Cache metadata changed during cursor refresh".into());
        }
        open(&self.path(id, "zst")?).map_err(err)?;
        row.saved_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(err)?
            .as_millis() as u64;
        self.write_json(&path, &row)
    }
    pub fn mark_opened(&self, id: &str) -> Result<(), String> {
        self.prepare()?;
        let _: SavedEntry = self.json(&self.path(id, "json")?)?;
        self.write_json(&self.root.join("selection.json"), &Some(id))
    }
    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.delete_with_remove(id, |path| fs::remove_file(path))
    }
    fn delete_with_remove(
        &self,
        id: &str,
        mut remove: impl FnMut(&Path) -> io::Result<()>,
    ) -> Result<(), String> {
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
        if self.list()?.last_opened_id.as_deref() == Some(id) {
            self.write_json(&self.root.join("selection.json"), &Option::<String>::None)?;
        }
        // Persist archive removal first so an interruption never leaves a hidden archive.
        for path in [&archive, &metadata] {
            match remove(path) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(err(e)),
            }
            File::open(&self.root)
                .and_then(|f| f.sync_all())
                .map_err(err)?;
        }
        Ok(())
    }
}
fn sniff_binary_archive(file: &mut BufReader<File>) -> io::Result<bool> {
    let mut decoder = zstd::stream::read::Decoder::new(file)?.single_frame();
    decoder.window_log_max(27)?;
    let mut magic = Vec::with_capacity(8);
    decoder.take(8).read_to_end(&mut magic)?;
    Ok(magic == binary_archive::MAGIC)
}
struct LimitedWriter<W, F> {
    inner: W,
    count: u64,
    limit: u64,
    progress: F,
}
impl<W: Write, F: FnMut(u64)> Write for LimitedWriter<W, F> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if b.len() as u64 > self.limit - self.count {
            return Err(io::Error::other("Saved scan exceeds decoded byte limit"));
        }
        let n = self.inner.write(b)?;
        self.count += n as u64;
        if self.count / (1024 * 1024) != (self.count - n as u64) / (1024 * 1024) {
            (self.progress)(self.count)
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
pub const OPEN_CANCELLED: &str = "Saved scan opening cancelled.";
fn check_open_cancelled(cancelled: Option<&AtomicBool>) -> Result<(), String> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(OPEN_CANCELLED.into())
    } else {
        Ok(())
    }
}
struct LimitedReader<'a, R, F> {
    cancelled: Option<&'a AtomicBool>,
    inner: R,
    count: u64,
    limit: u64,
    progress: F,
}
impl<R: Read, F: FnMut(u64)> Read for LimitedReader<'_, R, F> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        check_open_cancelled(self.cancelled).map_err(io::Error::other)?;
        let cap = b
            .len()
            .min((self.limit - self.count).saturating_add(1) as usize);
        let n = self.inner.read(&mut b[..cap])?;
        if n as u64 > self.limit - self.count {
            return Err(io::Error::other("Saved scan exceeds decoded byte limit"));
        }
        self.count += n as u64;
        if self.count / (1024 * 1024) != (self.count - n as u64) / (1024 * 1024) {
            (self.progress)(self.count)
        }
        Ok(n)
    }
}
#[derive(Serialize)]
struct ArchiveRef<'a> {
    version: u32,
    metadata: &'a ScanMetadata,
    snapshot: SnapshotRef<'a>,
}
#[derive(Deserialize)]
struct Archive {
    version: u32,
    metadata: ScanMetadata,
    snapshot: SnapshotWire,
}
struct SnapshotRef<'a>(&'a Snapshot);
#[derive(Serialize, Deserialize)]
struct NodeWire {
    id: u32,
    parent: Option<u32>,
    #[serde(deserialize_with = "children")]
    children: Vec<u32>,
    name: String,
    #[serde(deserialize_with = "native_path")]
    path: Vec<u8>,
    identity: Identity,
    logical_size: u64,
    allocated_size: u64,
    own_logical_size: u64,
    own_allocated_size: u64,
    is_directory: bool,
    hard_link: bool,
    excluded: bool,
    #[serde(default)]
    clone_metadata: Option<CloneMetadata>,
}
#[derive(Serialize, Deserialize)]
struct IssueWire {
    #[serde(deserialize_with = "native_path")]
    path: Vec<u8>,
    message: String,
    error_kind: Option<String>,
    raw_os_error: Option<i32>,
}
#[derive(Deserialize)]
struct SnapshotWire {
    #[serde(deserialize_with = "native_path")]
    root: Vec<u8>,
    #[serde(deserialize_with = "nodes")]
    nodes: Snapshot,
    stats: ScanStats,
    state: ScanState,
    #[serde(deserialize_with = "issues")]
    errors: Vec<IssueWire>,
    #[serde(deserialize_with = "issues")]
    exclusions: Vec<IssueWire>,
    #[serde(deserialize_with = "worker_stats")]
    worker_stats: Vec<WorkerStats>,
    finalization_us: u64,
    descriptor_limit: Option<usize>,
    descriptor_peak: Option<usize>,
    #[serde(default)]
    accounting: Option<AccountingInfo>,
}
impl NodeWire {
    fn into_node(self) -> Node {
        Node {
            id: self.id,
            parent: self.parent,
            children: self.children,
            name: self.name,
            path: std::ffi::OsString::from_vec(self.path).into(),
            identity: self.identity,
            logical_size: self.logical_size,
            allocated_size: self.allocated_size,
            own_logical_size: self.own_logical_size,
            own_allocated_size: self.own_allocated_size,
            is_directory: self.is_directory,
            hard_link: self.hard_link,
            excluded: self.excluded,
            clone_metadata: self.clone_metadata,
        }
    }
}
impl IssueWire {
    fn into_issue(self) -> ScanIssue {
        ScanIssue {
            path: std::ffi::OsString::from_vec(self.path).into(),
            message: self.message,
            error_kind: self.error_kind,
            raw_os_error: self.raw_os_error,
        }
    }
}
impl SnapshotWire {
    fn into_snapshot(self) -> Result<Snapshot, String> {
        let root: PathBuf = std::ffi::OsString::from_vec(self.root).into();
        let mut snapshot = self.nodes;
        if snapshot.node_count() != 0 && snapshot.root != root {
            return Err("Invalid saved scan root".into());
        }
        snapshot.restore_legacy_root(root)?;
        snapshot.stats = self.stats;
        snapshot.state = self.state;
        snapshot.errors = self.errors.into_iter().map(IssueWire::into_issue).collect();
        snapshot.exclusions = self
            .exclusions
            .into_iter()
            .map(IssueWire::into_issue)
            .collect();
        snapshot.worker_stats = self.worker_stats;
        snapshot.finalization_us = self.finalization_us;
        snapshot.descriptor_limit = self.descriptor_limit;
        snapshot.descriptor_peak = self.descriptor_peak;
        snapshot.accounting = self.accounting.unwrap_or_default();
        Ok(snapshot)
    }
}
fn bounded_vec<'de, D, T>(d: D, limit: usize) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    bounded_vec_bytes(d, limit, usize::MAX, |_| 0)
}
fn bounded_vec_bytes<'de, D, T, F>(
    d: D,
    limit: usize,
    bytes: usize,
    weight: F,
) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
    F: Fn(&T) -> usize,
{
    struct Bounded<T, F>(usize, usize, F, std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, F: Fn(&T) -> usize> Visitor<'de> for Bounded<T, F> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("bounded saved scan field")
        }
        fn visit_seq<A: SeqAccess<'de>>(mut self, mut a: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(value) = a.next_element()? {
                if out.len() == self.0 {
                    return Err(serde::de::Error::custom(
                        "Saved scan array exceeds field limit",
                    ));
                }
                self.1 = self.1.checked_sub((self.2)(&value)).ok_or_else(|| {
                    serde::de::Error::custom("Saved scan metadata exceeds byte limit")
                })?;
                out.push(value);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(Bounded(limit, bytes, weight, std::marker::PhantomData))
}
fn children<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u32>, D::Error> {
    bounded_vec(d, MAX_NODES)
}
fn native_path<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    bounded_vec(d, archive_codec::MAX_FIELD_BYTES)
}
// Errors, exclusions and workers together retain at most the fast envelope budget.
fn issues<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<IssueWire>, D::Error> {
    bounded_vec_bytes(
        d,
        10_000,
        archive_codec::MAX_RECORD_BYTES / 4,
        |issue: &IssueWire| {
            issue.path.capacity()
                + issue.message.capacity()
                + issue.error_kind.as_ref().map_or(0, String::capacity)
        },
    )
}
fn worker_stats<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<WorkerStats>, D::Error> {
    bounded_vec_bytes(
        d,
        disk_scan::MAX_WORKERS,
        archive_codec::MAX_RECORD_BYTES / 4,
        |worker: &WorkerStats| worker.qos.capacity(),
    )
}
// Bound resident fields even when a positional/large record uses streaming serde.
fn nodes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Snapshot, D::Error> {
    nodes_with_limit(d, MAX_NODES)
}
fn nodes_with_limit<'de, D: serde::Deserializer<'de>>(
    d: D,
    limit: usize,
) -> Result<Snapshot, D::Error> {
    struct Nodes(usize);
    impl<'de> Visitor<'de> for Nodes {
        type Value = Snapshot;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("bounded nodes")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
            let mut out = LegacyTreeBuilder::new(PathBuf::new());
            let mut remaining_children = self.0.saturating_sub(1);
            loop {
                if out.snapshot().node_count() == self.0 {
                    if a.next_element::<serde::de::IgnoredAny>()?.is_some() {
                        return Err(serde::de::Error::custom("Saved scan exceeds node limit"));
                    }
                    break;
                }
                match a.next_element::<NodeWire>()? {
                    Some(n) => {
                        remaining_children = remaining_children
                            .checked_sub(n.children.len())
                            .ok_or_else(|| {
                                serde::de::Error::custom("Saved scan exceeds child reference limit")
                            })?;
                        out.append(n.into_node())
                            .map_err(serde::de::Error::custom)?;
                    }
                    None => break,
                }
            }
            Ok(out.finish())
        }
    }
    d.deserialize_seq(Nodes(limit))
}
struct NodesRef<'a>(&'a Snapshot);
impl Serialize for NodesRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.node_count()))?;
        let mut paths = self.0.path_cursor();
        for n in self.0.iter_nodes() {
            let path = paths.path(n.id).map_err(serde::ser::Error::custom)?;
            let name = n.display_name();
            #[derive(Serialize)]
            struct N<'a> {
                id: u32,
                parent: Option<u32>,
                children: &'a [u32],
                name: &'a str,
                path: &'a [u8],
                identity: &'a Identity,
                logical_size: u64,
                allocated_size: u64,
                own_logical_size: u64,
                own_allocated_size: u64,
                is_directory: bool,
                hard_link: bool,
                excluded: bool,
                clone_metadata: Option<&'a CloneMetadata>,
            }
            seq.serialize_element(&N {
                id: n.id,
                parent: n.parent,
                children: n.children(),
                name: &name,
                path: path.as_os_str().as_bytes(),
                identity: &n.identity,
                logical_size: n.logical_size,
                allocated_size: n.allocated_size,
                own_logical_size: n.own_logical_size,
                own_allocated_size: n.own_allocated_size,
                is_directory: n.is_directory,
                hard_link: n.hard_link,
                excluded: n.excluded,
                clone_metadata: n.clone_metadata(),
            })?;
        }
        seq.end()
    }
}
struct IssuesRef<'a>(&'a [ScanIssue]);
impl Serialize for IssuesRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for i in self.0 {
            #[derive(Serialize)]
            struct I<'a> {
                path: &'a [u8],
                message: &'a str,
                error_kind: &'a Option<String>,
                raw_os_error: Option<i32>,
            }
            seq.serialize_element(&I {
                path: i.path.as_os_str().as_bytes(),
                message: &i.message,
                error_kind: &i.error_kind,
                raw_os_error: i.raw_os_error,
            })?
        }
        seq.end()
    }
}
impl Serialize for SnapshotRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct View<'a> {
            root: &'a [u8],
            nodes: NodesRef<'a>,
            stats: &'a ScanStats,
            state: ScanState,
            errors: IssuesRef<'a>,
            exclusions: IssuesRef<'a>,
            worker_stats: &'a [WorkerStats],
            finalization_us: u64,
            descriptor_limit: Option<usize>,
            descriptor_peak: Option<usize>,
            accounting: &'a AccountingInfo,
        }
        let n = self.0;
        View {
            root: n.root.as_os_str().as_bytes(),
            nodes: NodesRef(n),
            stats: &n.stats,
            state: n.state,
            errors: IssuesRef(&n.errors),
            exclusions: IssuesRef(&n.exclusions),
            worker_stats: &n.worker_stats,
            finalization_us: n.finalization_us,
            descriptor_limit: n.descriptor_limit,
            descriptor_peak: n.descriptor_peak,
            accounting: &n.accounting,
        }
        .serialize(s)
    }
}
fn parallel_cache_eligible(compact: bool, nodes: usize, workers: usize) -> bool {
    compact && workers > 1 && nodes >= PARALLEL_CACHE_MIN_NODES
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

fn validate_metadata(m: &ScanMetadata) -> Result<(), String> {
    if m.completed_at_ms < m.started_at_ms || !(1..=disk_scan::MAX_WORKERS).contains(&m.workers) {
        Err("Invalid saved scan metadata".into())
    } else {
        Ok(())
    }
}
fn validate_header(s: &Snapshot, limit: usize) -> Result<(), String> {
    if s.state != ScanState::Completed {
        return Err("Only completed scans can be saved".into());
    }
    if s.node_count() == 0 || s.node_count() > limit {
        return Err("Saved scan exceeds node limit or has no root".into());
    }
    let root = s.node(0).ok_or("Missing root")?;
    if root.parent.is_some()
        || root.path()? != s.root
        || !s.root.is_absolute()
        || !root.is_directory
    {
        return Err("Invalid saved scan root".into());
    }
    Ok(())
}
fn validate(s: &Snapshot, limit: usize) -> Result<(), String> {
    validated_clone_attributions(s, limit).map(|_| ())
}
type CloneAttributions = Vec<(u32, (bool, u64))>;
/// Full validation, returning the clone attributions its allocation plan derived.
fn validated_clone_attributions(s: &Snapshot, limit: usize) -> Result<CloneAttributions, String> {
    validate_header(s, limit)?;
    if s.accounting.warnings.len() > 32 || s.accounting.warnings.iter().any(|w| w.len() > 4096) {
        return Err("Saved scan accounting warnings exceed limit".into());
    }
    let root = s.node(0).ok_or("Missing root")?;
    let mut seen = vec![false; s.node_count()];
    seen[0] = true;
    let mut directories = 0u64;
    let mut logical = vec![0u64; s.node_count()];
    let ordinary_paths = cfg!(unix) && !s.has_path_overrides();
    if ordinary_paths && s.root.as_os_str().as_bytes().contains(&0) {
        return Err("Invalid saved scan node".into());
    }
    let mut paths = s.path_cursor();
    for (i, n) in s.iter_nodes().enumerate() {
        if n.id as usize != i
            || n.is_directory != (n.identity.kind == disk_scan::NodeKind::Directory)
        {
            return Err("Invalid saved scan node".into());
        }
        directories += u64::from(n.is_directory);
        if i > 0 {
            let p = n.parent.ok_or("Disconnected saved scan node")? as usize;
            if p >= i || !s.node(p as u32).ok_or("Invalid parent")?.is_directory {
                return Err("Invalid saved scan parent/path".into());
            }
        }
        if ordinary_paths {
            // Structural induction: the root is valid and every preceding
            // parent is a directory. One exact Normal basename defines each
            // child's path and its lexical parent without rebuilding prefixes.
            // Duplicate sibling names remain accepted by the historical policy.
            if i > 0 && s.native_component(n.id)?.as_bytes().contains(&0) {
                return Err("Invalid saved scan node".into());
            }
        } else {
            // A supplied legacy spelling remains authoritative. Validate its
            // complete bytes and parent relation before accepting the archive.
            let (path, parent_path) = paths.paths(n.id)?;
            if path.as_os_str().as_bytes().contains(&0) {
                return Err("Invalid saved scan node".into());
            }
            if i > 0 && path.parent() != parent_path {
                return Err("Invalid saved scan parent/path".into());
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
                return Err("Invalid saved scan child topology".into());
            }
            seen[c] = true;
        }
        logical[i] = n.own_logical_size;
    }
    if seen.iter().any(|s| !*s) {
        return Err("Unreachable saved scan node".into());
    }
    let disk_scan::AllocationPlan {
        values: mut allocated,
        clone_deduction_bytes: deduction,
        clone_attributions,
        ..
    } = disk_scan::allocation_plan(s)?;
    if deduction != s.accounting.deduction_bytes
        || (s.iter_nodes().any(|n| n.clone_metadata().is_some()) && !s.accounting.shared)
    {
        return Err("Invalid saved scan accounting summary".into());
    }

    for i in (0..s.node_count()).rev() {
        let n = s.node(i as u32).ok_or("Invalid node")?;
        if logical[i] != n.logical_size || allocated[i] != n.allocated_size {
            return Err("Invalid saved scan size counters".into());
        }
        if let Some(p) = n.parent {
            logical[p as usize] = logical[p as usize].saturating_add(logical[i]);
            allocated[p as usize] = allocated[p as usize].saturating_add(allocated[i]);
        }
    }
    if s.stats.entries != s.node_count() as u64
        || s.stats.directories != directories
        || s.stats.logical_bytes != root.logical_size
        || s.stats.allocated_bytes != root.allocated_size
        || s.stats.errors < s.errors.len() as u64
        || s.stats.excluded < s.exclusions.len() as u64
        || s.stats.restricted_system_issues > s.stats.errors
    {
        return Err("Invalid saved scan statistics".into());
    }
    Ok(clone_attributions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};
    fn fixture() -> (tempfile::TempDir, Snapshot, ScanMetadata) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("file"), b"hello").unwrap();
        fs::hard_link(root.join("file"), root.join("hard")).unwrap();
        std::os::unix::fs::symlink("file", root.join("link")).unwrap();
        fs::write(root.join("odd\nname"), b"odd").unwrap();
        let sparse = File::create(root.join("sparse")).unwrap();
        sparse.set_len(1024 * 1024).unwrap();
        let mut s = disk_scan::scan(
            root,
            disk_scan::ScanOptions {
                workers: 1,
                backend: disk_scan::Backend::Portable,
                qos: ScanQos::Automatic,
                descriptor_limit: None,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        // APFS rejects non-UTF-8 file names; historical archives must still preserve them.
        let odd_id = s
            .iter_nodes()
            .find(|n| n.display_name() == "odd\nname")
            .unwrap()
            .id;
        let mut odd = s.legacy_node(odd_id).unwrap();
        odd.path = s
            .root
            .join(std::ffi::OsString::from_vec(vec![b'a', 0xff, b'\n']));
        s.replace_node(odd_id, odd).unwrap();
        s.stats.errors = 10001;
        s.stats.excluded = 10002;
        s.stats.restricted_system_issues = 7;
        s.stats.incomplete = true;
        s.errors.push(ScanIssue {
            path: s.path(2).unwrap(),
            message: "permission denied".into(),
            error_kind: Some("PermissionDenied".into()),
            raw_os_error: Some(libc::EACCES),
        });
        s.exclusions.push(ScanIssue {
            path: s.root.join("external"),
            message: "excluded".into(),
            error_kind: None,
            raw_os_error: None,
        });
        let m = ScanMetadata {
            started_at_ms: 10,
            completed_at_ms: 20,
            workers: 1,
            worker_mode: "manual".into(),
            worker_source: "setting".into(),
            qos: ScanQos::Automatic,
        };
        s.accounting = AccountingInfo::default();
        (dir, s, m)
    }
    #[test]
    fn historical_open_cancels_before_and_during_decode_without_mutating_selection() {
        let (dir, mut snapshot, metadata) = fixture();
        // A large wire record forces several decoder reads and a progress report.
        let id = snapshot
            .iter_nodes()
            .find(|node| !node.is_directory)
            .unwrap()
            .id;
        let mut node = snapshot.legacy_node(id).unwrap();
        node.name = "x".repeat(2 * 1024 * 1024);
        node.path = snapshot.root.join(&node.name);
        snapshot.replace_node(id, node).unwrap();
        disk_scan::aggregate(&mut snapshot);
        let library = Library::new(dir.path().join("library"));
        let entry = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            library
                .load_cancellable(&entry.id, |_| {}, &cancelled, || {})
                .unwrap_err(),
            OPEN_CANCELLED
        );
        cancelled.store(false, Ordering::Release);
        let mut reports = 0;
        let result = library.load_cancellable(
            &entry.id,
            |_| {
                reports += 1;
                cancelled.store(true, Ordering::Release);
            },
            &cancelled,
            || {},
        );
        assert!(result.is_err());
        assert!(reports > 0, "must cancel after decoding actually starts");
        assert_eq!(
            library.list().unwrap().last_opened_id,
            Some(entry.id.clone())
        );
        cancelled.store(false, Ordering::Release);
        assert!(library
            .load_cancellable(&entry.id, |_| {}, &cancelled, || {})
            .is_ok());
    }
    #[test]
    fn r3_v2_archive_rejects_duplicate_inode_with_inconsistent_hardlink_flags() {
        let (directory, original, metadata) = fixture();
        let library = Library::new(directory.path().join("r3-library"));
        let entry = library.save(&original, &metadata, |_| {}).unwrap();
        let archive_path = library.path(&entry.id, "zst").unwrap();
        let template = original
            .iter_nodes()
            .find(|n| n.identity.kind == disk_scan::NodeKind::File)
            .unwrap();
        let template = original.legacy_node(template.id).unwrap();
        for flags in [[false, false], [true, false]] {
            let mut forged = Snapshot::empty(original.root.clone());
            let mut root = original.legacy_node(0).unwrap();
            root.children = vec![1, 2];
            root.own_logical_size = 0;
            root.own_allocated_size = 0;
            root.logical_size = 200;
            root.allocated_size = 140;
            forged.append_legacy_node(root).unwrap();
            for (index, name) in ["a", "b"].into_iter().enumerate() {
                let mut node = template.clone();
                node.id = index as u32 + 1;
                node.parent = Some(0);
                node.children.clear();
                node.path = original.root.join(name);
                node.name = name.into();
                node.logical_size = 100;
                node.own_logical_size = 100;
                node.own_allocated_size = 120;
                node.allocated_size = if index == 0 { 120 } else { 20 };
                node.hard_link = flags[index];
                node.clone_metadata = Some(CloneMetadata {
                    clone_id: 77,
                    data_allocated_size: 100,
                    data_logical_size: 100,
                    reference_count: 2,
                    may_share_blocks: true,
                    verified: true,
                });
                forged.append_legacy_node(node).unwrap();
            }
            forged.state = ScanState::Completed;
            forged.stats.entries = 3;
            forged.stats.directories = 1;
            forged.stats.logical_bytes = 200;
            forged.stats.allocated_bytes = 140;
            forged.accounting.model = AccountingModel::FullClonesV1;
            forged.accounting.shared = true;
            forged.accounting.checks_complete = true;
            forged.accounting.deduction_bytes = 100;
            library
                .atomic(&archive_path, |file| {
                    let mut encoder = zstd::stream::write::Encoder::new(file, 3).map_err(err)?;
                    encoder.include_checksum(true).map_err(err)?;
                    serde_json::to_writer(
                        &mut encoder,
                        &ArchiveRef {
                            version: 2,
                            metadata: &metadata,
                            snapshot: SnapshotRef(&forged),
                        },
                    )
                    .map_err(err)?;
                    encoder.finish().map_err(err)?.sync_all().map_err(err)
                })
                .unwrap();
            assert!(
                library.load(&entry.id, |_| {}).is_err(),
                "accepted forged flags {flags:?}"
            );

            // The historical model preserves its original unflagged same-inode
            // policy; this stricter rule must not reinterpret version-one scans.
            if flags == [false, false] {
                for id in 1..=2 {
                    let mut node = forged.legacy_node(id).unwrap();
                    node.clone_metadata = None;
                    node.allocated_size = 120;
                    forged.replace_legacy_node(id, node).unwrap();
                }
                forged.update_node(0, |n| n.allocated_size = 240).unwrap();
                forged.stats.allocated_bytes = 240;
                forged.accounting = AccountingInfo::default();
                library
                    .atomic(&archive_path, |file| {
                        let mut encoder =
                            zstd::stream::write::Encoder::new(file, 3).map_err(err)?;
                        encoder.include_checksum(true).map_err(err)?;
                        serde_json::to_writer(
                            &mut encoder,
                            &ArchiveRef {
                                version: 1,
                                metadata: &metadata,
                                snapshot: SnapshotRef(&forged),
                            },
                        )
                        .map_err(err)?;
                        encoder.finish().map_err(err)?.sync_all().map_err(err)
                    })
                    .unwrap();
                let (loaded, _) = library.load(&entry.id, |_| {}).unwrap();
                assert_eq!(loaded.stats.allocated_bytes, 240);
                assert_eq!(loaded.accounting.model, AccountingModel::HardLinksV1);
            }
        }
    }

    #[test]
    fn clone_archive_preserves_hardlinks_and_resource_fork_attribution() {
        let (directory, mut snapshot, metadata) = fixture();
        let ids: Vec<_> = snapshot
            .iter_nodes()
            .filter(|n| n.hard_link || n.display_name() == "odd\nname")
            .map(|n| n.id)
            .collect();
        assert_eq!(ids.len(), 3);
        for id in ids {
            let mut node = snapshot.legacy_node(id).unwrap();
            node.clone_metadata = Some(CloneMetadata {
                clone_id: 99,
                data_allocated_size: 2048,
                data_logical_size: 1,
                reference_count: 3,
                may_share_blocks: true,
                verified: true,
            });
            snapshot.replace_legacy_node(id, node).unwrap();
        }
        snapshot.accounting.model = AccountingModel::FullClonesV1;
        snapshot.accounting.checks_complete = true;
        snapshot.accounting.shared = true;
        snapshot.accounting.deduction_bytes = 2048;
        let allocations = disk_scan::attributed_own_allocations(&snapshot).unwrap();
        for (id, allocation) in allocations.into_iter().enumerate() {
            snapshot
                .update_node(id as u32, |n| n.allocated_size = allocation)
                .unwrap();
        }
        for id in (1..snapshot.node_count() as u32).rev() {
            let node = snapshot.node(id).unwrap();
            let parent = node.parent.unwrap();
            let allocation = node.allocated_size;
            snapshot
                .update_node(parent, |n| n.allocated_size += allocation)
                .unwrap();
        }
        snapshot.stats.allocated_bytes = snapshot.node(0).unwrap().allocated_size;
        let library = Library::new(directory.path().join("clones"));
        let entry = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let (loaded, _) = library.load(&entry.id, |_| {}).unwrap();
        assert_eq!(loaded.stats.allocated_bytes, snapshot.stats.allocated_bytes);
        assert_eq!(loaded.accounting.deduction_bytes, 2048);
        assert_eq!(
            loaded
                .iter_nodes()
                .filter(|n| n.clone_metadata().is_some())
                .count(),
            3
        );
        let portable = directory.path().join("clones.hahakigamiscan");
        library.export_scan(&entry.id, &portable, |_| {}).unwrap();
        let imported = library.import_scan(&portable, |_| {}).unwrap();
        let (portable_snapshot, _) = library.load(&imported.id, |_| {}).unwrap();
        assert_eq!(
            serde_json::to_value(SnapshotRef(&loaded)).unwrap(),
            serde_json::to_value(SnapshotRef(&portable_snapshot)).unwrap()
        );
        snapshot.accounting.deduction_bytes = 0;
        assert!(library.save(&snapshot, &metadata, |_| {}).is_err());
    }

    #[test]
    fn version_two_preserves_clone_metadata_and_rejects_invalid_allocation() {
        let (d, mut snapshot, metadata) = fixture();
        let id = snapshot
            .iter_nodes()
            .find(|n| n.identity.kind == disk_scan::NodeKind::File && !n.hard_link)
            .unwrap()
            .id;
        let mut node = snapshot.legacy_node(id).unwrap();
        node.clone_metadata = Some(CloneMetadata {
            clone_id: 42,
            data_allocated_size: node.own_allocated_size,
            data_logical_size: node.own_logical_size,
            reference_count: 2,
            may_share_blocks: true,
            verified: true,
        });
        snapshot.replace_legacy_node(id, node.clone()).unwrap();
        snapshot.accounting.model = AccountingModel::FullClonesV1;
        snapshot.accounting.shared = true;
        snapshot.accounting.checks_complete = true;
        let library = Library::new(d.path().join("clone-library"));
        let entry = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let (loaded, _) = library.load(&entry.id, |_| {}).unwrap();
        assert_eq!(
            loaded.node(id).unwrap().clone_metadata(),
            node.clone_metadata.as_ref()
        );
        assert_eq!(loaded.accounting.model, AccountingModel::FullClonesV1);
        node.clone_metadata.as_mut().unwrap().data_allocated_size = node.own_allocated_size + 1;
        snapshot.replace_legacy_node(id, node).unwrap();
        assert!(library.save(&snapshot, &metadata, |_| {}).is_err());
    }

    #[test]
    fn raw_singleton_clone_roundtrips_without_attribution_or_cache_eligibility() {
        let (d, mut snapshot, metadata) = fixture();
        let id = snapshot
            .iter_nodes()
            .find(|n| n.identity.kind == disk_scan::NodeKind::File && !n.hard_link)
            .unwrap()
            .id;
        let mut node = snapshot.legacy_node(id).unwrap();
        node.clone_metadata = Some(CloneMetadata {
            clone_id: 42,
            data_allocated_size: node.own_allocated_size,
            data_logical_size: node.own_logical_size,
            reference_count: 2,
            may_share_blocks: true,
            verified: false,
        });
        snapshot.replace_legacy_node(id, node.clone()).unwrap();
        snapshot.accounting.model = AccountingModel::FullClonesV1;
        snapshot.accounting.shared = true;
        snapshot.accounting.checks_complete = true;
        assert!(!snapshot.incremental_accounting_eligible());
        let library = Library::new(d.path().join("raw-singleton-library"));
        let entry = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let (loaded, _) = library.load(&entry.id, |_| {}).unwrap();
        let actual = loaded.node(id).unwrap();
        assert_eq!(actual.clone_metadata(), node.clone_metadata.as_ref());
        assert_eq!(actual.allocated_size, node.own_allocated_size);
        assert_eq!(actual.clone_attribution(), None);
        assert_eq!(loaded.stats.allocated_bytes, snapshot.stats.allocated_bytes);
        assert_eq!(loaded.accounting.deduction_bytes, 0);
        assert!(!loaded.incremental_accounting_eligible());
    }

    #[test]
    fn archive_roundtrip_crosses_record_chunks_with_clone_attribution() {
        let (d, mut snapshot, metadata) = fixture();
        let mut root = snapshot.legacy_node(0).unwrap();
        let template = snapshot
            .iter_nodes()
            .find(|n| n.identity.kind == disk_scan::NodeKind::File)
            .map(|n| snapshot.legacy_node(n.id).unwrap())
            .unwrap();
        while snapshot.node_count() < 9000 {
            let id = snapshot.node_count() as u32;
            let mut node = template.clone();
            node.id = id;
            node.parent = Some(0);
            node.name = format!("boundary-{id}");
            node.path = snapshot.root.join(&node.name);
            node.identity.inode = 10_000_000 + u64::from(id);
            node.hard_link = false;
            node.children.clear();
            node.logical_size = 0;
            node.allocated_size = 0;
            node.own_logical_size = 0;
            node.own_allocated_size = 0;
            node.clone_metadata = None;
            if matches!(id, 4095 | 8192) {
                node.logical_size = 512;
                node.own_logical_size = 512;
                node.allocated_size = 1024;
                node.own_allocated_size = 1024;
                node.clone_metadata = Some(CloneMetadata {
                    clone_id: 42,
                    data_allocated_size: 512,
                    data_logical_size: 512,
                    reference_count: 2,
                    may_share_blocks: true,
                    verified: true,
                });
            }
            snapshot.append_legacy_node(node).unwrap();
            root.children.push(id);
        }
        snapshot.replace_legacy_node(0, root).unwrap();
        snapshot.accounting.model = AccountingModel::FullClonesV1;
        snapshot.accounting.shared = true;
        disk_scan::aggregate(&mut snapshot);
        snapshot.stats.entries = snapshot.node_count() as u64;
        assert_eq!(snapshot.accounting.deduction_bytes, 512);
        let expected = serde_json::to_value(SnapshotRef(&snapshot)).unwrap();
        let library = Library::new(d.path().join("chunk-boundaries"));
        let row = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let (loaded, _) = library.load(&row.id, |_| {}).unwrap();
        assert_eq!(
            serde_json::to_value(SnapshotRef(&loaded)).unwrap(),
            expected
        );
        assert_eq!(
            loaded.node(4095).unwrap().clone_attribution(),
            Some((true, 0))
        );
        assert_eq!(
            loaded.node(8192).unwrap().clone_attribution(),
            Some((false, 512))
        );
    }

    #[test]
    fn roundtrip_preserves_native_paths_accounting_and_diagnostics() {
        let (d, s, m) = fixture();
        let l = Library::new(d.path().join("library"));
        let row = l.save(&s, &m, |_| {}).unwrap();
        let (loaded, metadata) = l.load(&row.id, |_| {}).unwrap();
        assert_eq!(
            serde_json::to_value(SnapshotRef(&s)).unwrap(),
            serde_json::to_value(SnapshotRef(&loaded)).unwrap()
        );
        assert_eq!(metadata.completed_at_ms, 20);
        assert_eq!(l.list().unwrap().last_opened_id, Some(row.id.clone()));
        assert_eq!(
            fs::metadata(&l.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(l.path(&row.id, "zst").unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        l.delete(&row.id).unwrap();
        assert!(l.list().unwrap().entries.is_empty());
        assert!(l.list().unwrap().last_opened_id.is_none());
        assert!(l.load(&row.id, |_| {}).is_err());
    }
    #[test]
    fn delete_recovers_each_missing_half_and_is_repeatable() {
        let (d, s, m) = fixture();
        let library = Library::new(d.path().join("library"));
        for missing in ["json", "zst"] {
            let row = library.save(&s, &m, |_| {}).unwrap();
            fs::remove_file(library.path(&row.id, missing).unwrap()).unwrap();
            library.delete(&row.id).unwrap();
            library.delete(&row.id).unwrap();
            assert!(!library.path(&row.id, "json").unwrap().exists());
            assert!(!library.path(&row.id, "zst").unwrap().exists());
            assert!(library.list().unwrap().last_opened_id.is_none());
        }
        assert!(library.delete("../escape").is_err());
    }
    #[test]
    fn delete_retains_metadata_when_archive_removal_fails() {
        let (d, s, m) = fixture();
        let library = Library::new(d.path().join("library"));
        let row = library.save(&s, &m, |_| {}).unwrap();
        let archive = library.path(&row.id, "zst").unwrap();
        let result = library.delete_with_remove(&row.id, |path| {
            assert_eq!(path, archive);
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected unlink failure",
            ))
        });
        assert!(result.unwrap_err().contains("injected unlink failure"));
        assert!(library.path(&row.id, "json").unwrap().exists());
        assert!(archive.exists());
        assert_eq!(library.list().unwrap().entries.len(), 1);
        library.delete(&row.id).unwrap();
        assert!(library.list().unwrap().entries.is_empty());
    }
    #[test]
    fn delete_retries_after_metadata_removal_fails() {
        let (d, s, m) = fixture();
        let library = Library::new(d.path().join("library"));
        let row = library.save(&s, &m, |_| {}).unwrap();
        let metadata = library.path(&row.id, "json").unwrap();
        let archive = library.path(&row.id, "zst").unwrap();
        let result = library.delete_with_remove(&row.id, |path| {
            if path == metadata {
                assert!(!archive.exists());
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected metadata unlink failure",
                ))
            } else {
                fs::remove_file(path)
            }
        });
        assert!(result
            .unwrap_err()
            .contains("injected metadata unlink failure"));
        assert!(metadata.exists());
        library.delete(&row.id).unwrap();
        assert!(!metadata.exists());
    }
    #[test]
    fn portable_roundtrip_preserves_native_snapshot_and_selection() {
        let (d, s, m) = fixture();
        let library = Library::new(d.path().join("library"));
        let original = library.save(&s, &m, |_| {}).unwrap();
        let path = d.path().join("shared.hahakigamiscan");
        fs::write(&path, b"old export").unwrap();
        library.export_scan(&original.id, &path, |_| {}).unwrap();
        let imported = library.import_scan(&path, |_| {}).unwrap();
        assert_ne!(original.id, imported.id);
        let (loaded, metadata) = library.load(&imported.id, |_| {}).unwrap();
        assert_eq!(
            serde_json::to_value(SnapshotRef(&s)).unwrap(),
            serde_json::to_value(SnapshotRef(&loaded)).unwrap()
        );
        assert_eq!(metadata.completed_at_ms, m.completed_at_ms);
        let list = library.list().unwrap();
        assert_eq!(list.entries.len(), 2);
        assert_eq!(list.last_opened_id, Some(original.id));
    }
    #[test]
    fn portable_rejections_preserve_library_and_existing_destination() {
        let (d, s, m) = fixture();
        let mut library = Library::new(d.path().join("library"));
        let row = library.save(&s, &m, |_| {}).unwrap();
        let archive = library.path(&row.id, "zst").unwrap();
        let original = fs::read(&archive).unwrap();
        let path = d.path().join("shared.hahakigamiscan");
        fs::write(&path, b"keep me").unwrap();
        let mut corrupt = original.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        fs::write(&archive, &corrupt).unwrap();
        assert!(library.export_scan(&row.id, &path, |_| {}).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"keep me");
        assert!(library.import_scan(&archive, |_| {}).is_err());
        fs::write(&archive, &original).unwrap();
        assert!(library.export_scan(&row.id, &archive, |_| {}).is_err());
        let alias = d.path().join("alias");
        fs::hard_link(&archive, &alias).unwrap();
        assert!(library.export_scan(&row.id, &alias, |_| {}).is_err());
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(library.export_scan(&row.id, &link, |_| {}).is_err());
        assert!(library.export_scan(&row.id, d.path(), |_| {}).is_err());
        library.byte_limit = 1;
        assert!(library.import_scan(&archive, |_| {}).is_err());
        library.byte_limit = MAX_BYTES;
        let mut cache_snapshot = s;
        cache_snapshot.accounting.model = AccountingModel::FullClonesV1;
        cache_snapshot.accounting.checks_complete = true;
        cache_snapshot.errors.clear();
        cache_snapshot.exclusions.clear();
        let cache = library.save_cache(&cache_snapshot, &m).unwrap();
        assert!(library
            .import_scan(&library.path(&cache.id, "zst").unwrap(), |_| {})
            .is_err());
        let unsupported = d.path().join("unsupported");
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
        encoder.include_checksum(true).unwrap();
        serde_json::to_writer(
            &mut encoder,
            &ArchiveRef {
                version: 99,
                metadata: &m,
                snapshot: SnapshotRef(&cache_snapshot),
            },
        )
        .unwrap();
        fs::write(&unsupported, encoder.finish().unwrap()).unwrap();
        assert!(library
            .import_scan(&unsupported, |_| {})
            .unwrap_err()
            .contains("version"));
        let list = library.list().unwrap();
        assert_eq!(list.entries.len(), 2); // Original plus private cache fixture.
        assert_eq!(list.last_opened_id, Some(row.id));
        assert!(!fs::read_dir(d.path()).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|extension| extension == "tmp")));
    }
    #[test]
    fn corruption_truncation_and_unsupported_versions_fail() {
        let (d, s, m) = fixture();
        let l = Library::new(d.path().join("library"));
        let row = l.save(&s, &m, |_| {}).unwrap();
        let path = l.path(&row.id, "zst").unwrap();
        let original = fs::read(&path).unwrap();
        let mut corrupt = original.clone();
        let n = corrupt.len();
        corrupt[n - 1] ^= 1;
        fs::write(&path, corrupt).unwrap();
        assert!(l.load(&row.id, |_| {}).is_err());
        fs::write(&path, &original[..original.len() - 2]).unwrap();
        assert!(l.load(&row.id, |_| {}).is_err());
        l.atomic(&path, |f| {
            let mut e = zstd::stream::write::Encoder::new(f, 3).map_err(err)?;
            e.include_checksum(true).map_err(err)?;
            serde_json::to_writer(
                &mut e,
                &ArchiveRef {
                    version: 99,
                    metadata: &m,
                    snapshot: SnapshotRef(&s),
                },
            )
            .map_err(err)?;
            e.finish().map_err(err)?.sync_all().map_err(err)
        })
        .unwrap();
        assert!(l.load(&row.id, |_| {}).unwrap_err().contains("version"));
    }
    #[test]
    fn limits_topology_state_and_metadata_fail_without_publication() {
        let (d, mut s, m) = fixture();
        let mut l = Library::new(d.path().join("library"));
        l.node_limit = 2;
        assert!(l.save(&s, &m, |_| {}).is_err());
        l.node_limit = MAX_NODES;
        l.byte_limit = 30;
        assert!(l.save(&s, &m, |_| {}).is_err());
        assert!(l.list().unwrap().entries.is_empty());
        l.byte_limit = MAX_BYTES;
        let mut children = s.children(0).unwrap().to_vec();
        children.push(0);
        s.set_children(0, &children).unwrap();
        assert!(l.save(&s, &m, |_| {}).is_err());
        let mut children = s.children(0).unwrap().to_vec();
        children.pop();
        s.set_children(0, &children).unwrap();
        s.state = ScanState::Cancelled;
        assert!(l.save(&s, &m, |_| {}).is_err());
        s.state = ScanState::Completed;
        s.update_node(1, |n| n.logical_size += 1).unwrap();
        assert!(l.save(&s, &m, |_| {}).is_err());
    }
    #[test]
    fn load_limits_and_unreachable_nodes_fail() {
        let (d, mut s, m) = fixture();
        let mut l = Library::new(d.path().join("library"));
        let row = l.save(&s, &m, |_| {}).unwrap();
        l.byte_limit = 100;
        assert!(l.load(&row.id, |_| {}).is_err());
        l.byte_limit = MAX_BYTES;
        l.node_limit = 2;
        assert!(l.load(&row.id, |_| {}).is_err());
        l.node_limit = MAX_NODES;
        let mut children = s.children(0).unwrap().to_vec();
        children.pop();
        s.set_children(0, &children).unwrap();
        let path = l.path(&row.id, "zst").unwrap();
        l.atomic(&path, |f| {
            let mut e = zstd::stream::write::Encoder::new(f, 3).map_err(err)?;
            e.include_checksum(true).map_err(err)?;
            serde_json::to_writer(
                &mut e,
                &ArchiveRef {
                    version: 1,
                    metadata: &m,
                    snapshot: SnapshotRef(&s),
                },
            )
            .map_err(err)?;
            e.finish().map_err(err)?.sync_all().map_err(err)
        })
        .unwrap();
        assert!(l.load(&row.id, |_| {}).is_err());
    }
    #[test]
    fn interrupted_writes_symlinks_stale_ids_and_missing_selection() {
        let (d, s, m) = fixture();
        let l = Library::new(d.path().join("library"));
        l.prepare().unwrap();
        fs::write(l.root.join("incomplete.tmp"), b"partial").unwrap();
        assert!(l.list().unwrap().entries.is_empty());
        l.write_json(
            &l.root.join("selection.json"),
            &Some(uuid::Uuid::new_v4().to_string()),
        )
        .unwrap();
        let stale = l.list().unwrap().last_opened_id.unwrap();
        assert!(l.load(&stale, |_| {}).is_err());
        assert!(l.load("../escape", |_| {}).is_err());
        let path = l.root.join("failed.json");
        assert!(l
            .atomic(&path, |mut f| {
                f.write_all(b"partial").map_err(err)?;
                Err("simulated failure".into())
            })
            .is_err());
        assert!(!path.exists());
        let row = l.save(&s, &m, |_| {}).unwrap();
        let archive = l.path(&row.id, "zst").unwrap();
        fs::remove_file(&archive).unwrap();
        std::os::unix::fs::symlink(d.path().join("data/file"), archive).unwrap();
        assert!(l.load(&row.id, |_| {}).is_err());
        assert!(l.delete(&row.id).is_err());
        let invalid = Library::new(d.path().join("data/file"));
        assert!(invalid.save(&s, &m, |_| {}).is_err());
    }
    #[test]
    fn late_selection_failure_rolls_back_publication_and_preserves_selection() {
        let (d, s, m) = fixture();
        let l = Library::new(d.path().join("library"));
        l.prepare().unwrap();
        let selection = l.root.join("selection.json");
        fs::create_dir(&selection).unwrap();
        fs::write(selection.join("existing"), b"preserved").unwrap();
        for _ in 0..2 {
            assert!(l.save(&s, &m, |_| {}).is_err());
            assert!(l.list().unwrap().entries.is_empty());
            assert_eq!(fs::read(selection.join("existing")).unwrap(), b"preserved");
            assert_eq!(fs::read_dir(&l.root).unwrap().count(), 1);
        }
        fs::remove_dir_all(&selection).unwrap();
        let previous = Some(uuid::Uuid::new_v4().to_string());
        l.write_json(&selection, &previous).unwrap();
        assert!(l
            .atomic(&selection, |mut file| {
                file.write_all(b"partial replacement").map_err(err)?;
                Err("simulated selection write failure".into())
            })
            .is_err());
        assert_eq!(l.json::<Option<String>>(&selection).unwrap(), previous);
    }
    #[test]
    fn legacy_archive_fast_decoder_preserves_fields_and_buffer_boundaries() {
        let (_d, mut snapshot, metadata) = fixture();
        // Exercise quotes, braces and escaped backslashes across tiny buffers.
        let mut node = snapshot.legacy_node(1).unwrap();
        node.name = "quote\"{bracket]\\line\n".into();
        snapshot.replace_node(1, node).unwrap();
        let bytes = serde_json::to_vec(&ArchiveRef {
            version: 1,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        let expected: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        // Value reserialization deliberately changes all field ordering.
        let reordered = serde_json::to_vec(&expected).unwrap();
        for capacity in [1, 2, 7, 64, 8192] {
            for input in [&bytes, &reordered] {
                let mut reader = BufReader::with_capacity(capacity, input.as_slice());
                let decoded = archive_codec::decode(&mut reader, MAX_NODES).unwrap();
                let decoded_snapshot = decoded.snapshot.into_snapshot().unwrap();
                let actual = serde_json::to_value(ArchiveRef {
                    version: decoded.version,
                    metadata: &decoded.metadata,
                    snapshot: SnapshotRef(&decoded_snapshot),
                })
                .unwrap();
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn fast_decoder_rejects_malformed_records_duplicates_and_trailing_data() {
        let (_d, snapshot, metadata) = fixture();
        let original = serde_json::to_string(&ArchiveRef {
            version: 1,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        let mut malformed = vec![
            format!("{original}false"),
            original[..original.len() - 1].into(),
            original.replacen("\"nodes\":[", "\"nodes\":[],\"nodes\":[", 1),
            original.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
            original.replacen("\"children\":[", "\"children\":[,", 1),
        ];
        malformed.push(original.replacen("\"version\":1", "\"version\":01", 1));
        for input in malformed {
            let mut reader = BufReader::with_capacity(7, input.as_bytes());
            assert!(
                archive_codec::decode(&mut reader, MAX_NODES).is_err(),
                "accepted {input}"
            );
        }
        let mut wire: serde_json::Value = serde_json::from_str(&original).unwrap();
        wire["snapshot"]["nodes"][0]["children"] = serde_json::json!([]);
        let bytes = serde_json::to_vec(&wire).unwrap();
        let mut reader = BufReader::new(bytes.as_slice());
        assert!(
            matches!(archive_codec::decode(&mut reader, 2), Err(archive_codec::DecodeError::Invalid(message)) if message.contains("node limit"))
        );
    }

    #[test]
    fn archive_child_reference_budget_covers_fast_and_streaming_decoders() {
        let (_directory, snapshot, metadata) = fixture();
        let limit = snapshot.node_count();
        assert_eq!(snapshot.node(0).unwrap().children().len(), limit - 1);
        let mut wire = serde_json::to_value(ArchiveRef {
            version: 1,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        let bytes = serde_json::to_vec(&wire).unwrap();
        let archive = archive_codec::decode(&mut BufReader::new(bytes.as_slice()), limit).unwrap();
        assert_eq!(archive.snapshot.nodes.node_count(), limit);
        assert_eq!(
            archive.snapshot.nodes.node(0).unwrap().children().len(),
            limit - 1
        );

        let node_bytes = serde_json::to_vec(&wire["snapshot"]["nodes"]).unwrap();
        let decoded = nodes_with_limit(
            &mut serde_json::Deserializer::from_reader(node_bytes.as_slice()),
            limit,
        )
        .unwrap();
        assert_eq!(decoded.node_count(), limit);
        assert_eq!(decoded.node(0).unwrap().children().len(), limit - 1);

        // Each array fits alone; together they exceed a tree's N-1 edges.
        wire["snapshot"]["nodes"][1]["children"] = serde_json::json!([0]);
        let node_bytes = format!(
            "[{},{},",
            wire["snapshot"]["nodes"][0], wire["snapshot"]["nodes"][1]
        );
        // The missing tail proves rejection happens while retaining the second node.
        let bytes = format!("{{\"snapshot\":{{\"nodes\":{node_bytes}");
        assert!(matches!(
            archive_codec::decode(&mut BufReader::new(bytes.as_bytes()), limit),
            Err(archive_codec::DecodeError::Invalid(message))
                if message.contains("child reference limit")
        ));
        let error = nodes_with_limit(
            &mut serde_json::Deserializer::from_reader(node_bytes.as_bytes()),
            limit,
        )
        .unwrap_err();
        assert!(error.to_string().contains("child reference limit"));
    }

    #[test]
    fn positional_struct_archive_falls_back_and_streams_into_compact_storage() {
        let (_directory, snapshot, metadata) = fixture();
        let expected = serde_json::to_value(ArchiveRef {
            version: 1,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        fn positional(value: &serde_json::Value, fields: &[&str]) -> serde_json::Value {
            serde_json::Value::Array(fields.iter().map(|field| value[*field].clone()).collect())
        }
        let mut wire = expected.clone();
        wire["snapshot"]["nodes"] = serde_json::Value::Array(
            wire["snapshot"]["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| {
                    positional(
                        node,
                        &[
                            "id",
                            "parent",
                            "children",
                            "name",
                            "path",
                            "identity",
                            "logical_size",
                            "allocated_size",
                            "own_logical_size",
                            "own_allocated_size",
                            "is_directory",
                            "hard_link",
                            "excluded",
                        ],
                    )
                })
                .collect(),
        );
        wire["snapshot"] = positional(
            &wire["snapshot"],
            &[
                "root",
                "nodes",
                "stats",
                "state",
                "errors",
                "exclusions",
                "worker_stats",
                "finalization_us",
                "descriptor_limit",
                "descriptor_peak",
            ],
        );
        let bytes =
            serde_json::to_vec(&positional(&wire, &["version", "metadata", "snapshot"])).unwrap();
        assert!(matches!(
            archive_codec::decode(&mut BufReader::new(bytes.as_slice()), MAX_NODES),
            Err(archive_codec::DecodeError::LegacyRequired)
        ));
        let archive: Archive = serde_json::from_slice(&bytes).unwrap();
        let decoded = archive.snapshot.into_snapshot().unwrap();
        validate(&decoded, MAX_NODES).unwrap();
        assert_eq!(
            serde_json::to_value(ArchiveRef {
                version: archive.version,
                metadata: &archive.metadata,
                snapshot: SnapshotRef(&decoded)
            })
            .unwrap(),
            expected
        );
    }

    #[test]
    fn nr1p2_root_spellings_preserve_baseline_acceptance_and_all_native_bytes() {
        let (_directory, snapshot, metadata) = fixture();
        let original = serde_json::to_value(ArchiveRef {
            version: 1,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        let header: Vec<u8> = serde_json::from_value(original["snapshot"]["root"].clone()).unwrap();
        fn positional(value: &serde_json::Value, fields: &[&str]) -> serde_json::Value {
            serde_json::Value::Array(fields.iter().map(|field| value[*field].clone()).collect())
        }
        for suffix in [b"//".as_slice(), b"/./".as_slice()] {
            for variant_children in [false, true] {
                let mut wire = original.clone();
                let mut root_bytes = header.clone();
                root_bytes.extend_from_slice(suffix);
                let recorded_root = PathBuf::from(std::ffi::OsString::from_vec(root_bytes.clone()));
                let header_root = PathBuf::from(std::ffi::OsString::from_vec(header.clone()));
                let nodes = wire["snapshot"]["nodes"].as_array_mut().unwrap();
                nodes[0]["path"] = serde_json::to_value(&root_bytes).unwrap();
                for node in nodes.iter_mut().skip(1) {
                    let bytes: Vec<u8> = serde_json::from_value(node["path"].clone()).unwrap();
                    if variant_children {
                        let mut path = root_bytes.clone();
                        path.extend_from_slice(&bytes[header.len()..]);
                        node["path"] = serde_json::to_value(path).unwrap();
                    }
                    let bytes: Vec<u8> = serde_json::from_value(node["path"].clone()).unwrap();
                    let child = PathBuf::from(std::ffi::OsString::from_vec(bytes));
                    // Independent checks use the original validator's Path
                    // equality, rather than compact reconstruction or raw Eq.
                    assert_eq!(child.parent(), Some(recorded_root.as_path()));
                }
                assert_eq!(recorded_root, header_root);
                assert!(header_root.is_absolute());
                let positional_nodes = serde_json::Value::Array(
                    nodes
                        .iter()
                        .map(|node| {
                            positional(
                                node,
                                &[
                                    "id",
                                    "parent",
                                    "children",
                                    "name",
                                    "path",
                                    "identity",
                                    "logical_size",
                                    "allocated_size",
                                    "own_logical_size",
                                    "own_allocated_size",
                                    "is_directory",
                                    "hard_link",
                                    "excluded",
                                ],
                            )
                        })
                        .collect(),
                );
                let bytes = serde_json::to_vec(&wire).unwrap();
                let fast = archive_codec::decode(&mut BufReader::new(bytes.as_slice()), MAX_NODES)
                    .unwrap();
                let mut positional_wire = wire.clone();
                positional_wire["snapshot"]["nodes"] = positional_nodes;
                positional_wire["snapshot"] = positional(
                    &positional_wire["snapshot"],
                    &[
                        "root",
                        "nodes",
                        "stats",
                        "state",
                        "errors",
                        "exclusions",
                        "worker_stats",
                        "finalization_us",
                        "descriptor_limit",
                        "descriptor_peak",
                    ],
                );
                let positional_bytes = serde_json::to_vec(&positional(
                    &positional_wire,
                    &["version", "metadata", "snapshot"],
                ))
                .unwrap();
                let legacy: Archive = serde_json::from_slice(&positional_bytes).unwrap();
                for archive in [fast, legacy] {
                    let decoded = archive.snapshot.into_snapshot().unwrap();
                    validate(&decoded, MAX_NODES).unwrap();
                    assert_eq!(
                        serde_json::to_value(ArchiveRef {
                            version: 1,
                            metadata: &metadata,
                            snapshot: SnapshotRef(&decoded)
                        })
                        .unwrap(),
                        wire
                    );
                }
            }
        }
        let mut invalid = original;
        invalid["snapshot"]["root"] = serde_json::json!(b"/non-equivalent-root".to_vec());
        let bytes = serde_json::to_vec(&invalid).unwrap();
        let archive =
            archive_codec::decode(&mut BufReader::new(bytes.as_slice()), MAX_NODES).unwrap();
        assert!(archive.snapshot.into_snapshot().is_err());
        let archive: Archive = serde_json::from_slice(&bytes).unwrap();
        assert!(archive.snapshot.into_snapshot().is_err());
    }

    #[test]
    fn core_r2_history_without_ordinary_basename_remains_lossless() {
        let (_, _, metadata) = fixture();
        fn positional(value: &serde_json::Value, fields: &[&str]) -> serde_json::Value {
            serde_json::Value::Array(fields.iter().map(|field| value[*field].clone()).collect())
        }
        for (root, child) in [("/", "/.."), ("/a", "/a/..")] {
            // Independent baseline acceptance proof: the original validator
            // allowed these lexical parent relations for read-only history.
            assert_eq!(Path::new(child).parent(), Some(Path::new(root)));
            let mut snapshot = Snapshot::empty(root.into());
            for (id, path) in [(0, root), (1, child)] {
                snapshot
                    .append_legacy_node(Node {
                        id,
                        parent: (id != 0).then_some(0),
                        children: if id == 0 { vec![1] } else { vec![] },
                        name: format!("recorded label {id}"),
                        path: path.into(),
                        identity: Identity {
                            device: 1,
                            inode: u64::from(id) + 1,
                            kind: disk_scan::NodeKind::Directory,
                        },
                        logical_size: 0,
                        allocated_size: 0,
                        own_logical_size: 0,
                        own_allocated_size: 0,
                        is_directory: true,
                        hard_link: false,
                        excluded: false,
                        clone_metadata: None,
                    })
                    .unwrap();
            }
            snapshot.stats.entries = 2;
            snapshot.stats.directories = 2;
            snapshot.state = ScanState::Completed;
            validate(&snapshot, MAX_NODES).unwrap();
            let wire = serde_json::to_value(ArchiveRef {
                version: 1,
                metadata: &metadata,
                snapshot: SnapshotRef(&snapshot),
            })
            .unwrap();
            let bytes = serde_json::to_vec(&wire).unwrap();
            let fast =
                archive_codec::decode(&mut BufReader::new(bytes.as_slice()), MAX_NODES).unwrap();
            let mut positional_wire = wire.clone();
            positional_wire["snapshot"]["nodes"] = serde_json::Value::Array(
                wire["snapshot"]["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|node| {
                        positional(
                            node,
                            &[
                                "id",
                                "parent",
                                "children",
                                "name",
                                "path",
                                "identity",
                                "logical_size",
                                "allocated_size",
                                "own_logical_size",
                                "own_allocated_size",
                                "is_directory",
                                "hard_link",
                                "excluded",
                            ],
                        )
                    })
                    .collect(),
            );
            positional_wire["snapshot"] = positional(
                &positional_wire["snapshot"],
                &[
                    "root",
                    "nodes",
                    "stats",
                    "state",
                    "errors",
                    "exclusions",
                    "worker_stats",
                    "finalization_us",
                    "descriptor_limit",
                    "descriptor_peak",
                ],
            );
            let legacy: Archive = serde_json::from_value(positional(
                &positional_wire,
                &["version", "metadata", "snapshot"],
            ))
            .unwrap();
            for archive in [fast, legacy] {
                let decoded = archive.snapshot.into_snapshot().unwrap();
                validate(&decoded, MAX_NODES).unwrap();
                assert_eq!(
                    decoded.path(1).unwrap().as_os_str(),
                    Path::new(child).as_os_str()
                );
                assert_eq!(
                    serde_json::to_value(ArchiveRef {
                        version: 1,
                        metadata: &metadata,
                        snapshot: SnapshotRef(&decoded)
                    })
                    .unwrap(),
                    wire
                );
                let mut decoded = decoded;
                decoded.accounting.model = AccountingModel::FullClonesV1;
                decoded.accounting.checks_complete = true;
                let mut encoded = Vec::new();
                cache_codec::encode(&mut encoded, &decoded, &metadata).unwrap();
                let (cached, _) = cache_codec::decode(&mut encoded.as_slice(), MAX_NODES).unwrap();
                validate(&cached, MAX_NODES).unwrap();
                assert_eq!(
                    cached.path(1).unwrap().as_os_str(),
                    Path::new(child).as_os_str()
                );
            }
        }
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
        let mut logical = vec![0u64; nodes.len()];
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
            logical[i] = node.own_logical_size;
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
            if logical[i] != nodes[i].logical_size || allocated[i] != nodes[i].allocated_size {
                return false;
            }
            if let Some(parent) = nodes[i].parent {
                logical[parent as usize] = logical[parent as usize].saturating_add(logical[i]);
                allocated[parent as usize] =
                    allocated[parent as usize].saturating_add(allocated[i]);
            }
        }
        s.stats.entries == nodes.len() as u64
            && s.stats.directories == directories
            && s.stats.logical_bytes == root.logical_size
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
                    logical_size: size,
                    allocated_size: size,
                    own_logical_size: if directory { 0 } else { 4 },
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
        snapshot.stats.logical_bytes = nodes[0].logical_size;
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
        // Historical policy permits duplicate sibling names, unlike seed reuse.
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
        moved[1].logical_size = 4;
        moved[1].allocated_size = 4;
        moved[2].logical_size = 8;
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

    #[test]
    fn decoder_parent_cache_never_erases_a_supplied_wrong_parent_path() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["a", "b"] {
            fs::create_dir(directory.path().join(name)).unwrap();
            fs::write(directory.path().join(name).join("file"), b"data").unwrap();
        }
        let mut snapshot = disk_scan::scan(
            directory.path().into(),
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        validate(&snapshot, MAX_NODES).unwrap();
        let files: Vec<_> = snapshot
            .iter_nodes()
            .filter(|node| !node.is_directory)
            .map(|node| (node.id, node.parent, node.path().unwrap()))
            .collect();
        assert_eq!(files.len(), 2);
        assert_ne!(files[0].1, files[1].1);
        let target = files[1].0;
        let supplied_path = files[0].2.clone();
        let mut node = snapshot.legacy_node(target).unwrap();
        node.path = supplied_path.clone();
        snapshot.replace_node(target, node).unwrap();
        let metadata = ScanMetadata {
            started_at_ms: 1,
            completed_at_ms: 2,
            workers: 1,
            worker_mode: "auto".into(),
            worker_source: "hardware".into(),
            qos: ScanQos::Automatic,
        };
        let wire = serde_json::to_value(ArchiveRef {
            version: 1,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        let bytes = serde_json::to_vec(&wire).unwrap();
        let fast = archive_codec::decode(&mut BufReader::new(bytes.as_slice()), MAX_NODES).unwrap();
        // Both the fast parser and the positional-compatible serde visitor must
        // retain the supplied path until the independent topology validator.
        let legacy: Archive = serde_json::from_slice(&bytes).unwrap();
        for archive in [fast, legacy] {
            let decoded = archive.snapshot.into_snapshot().unwrap();
            assert_eq!(
                decoded.path(target).unwrap().as_os_str().as_bytes(),
                supplied_path.as_os_str().as_bytes()
            );
            assert!(validate(&decoded, MAX_NODES)
                .unwrap_err()
                .contains("parent/path"));
        }
        let mut encoded = Vec::new();
        cache_codec::encode(&mut encoded, &snapshot, &metadata).unwrap();
        let (decoded, _) = cache_codec::decode(&mut encoded.as_slice(), MAX_NODES).unwrap();
        assert_eq!(
            decoded.path(target).unwrap().as_os_str().as_bytes(),
            supplied_path.as_os_str().as_bytes()
        );
        assert!(validate(&decoded, MAX_NODES)
            .unwrap_err()
            .contains("parent/path"));
    }

    #[test]
    fn streaming_decoders_preserve_sparse_native_spellings_and_reject_contradictory_flags() {
        let (_directory, snapshot, metadata) = fixture();
        let mut wire = serde_json::to_value(ArchiveRef {
            version: 1,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        let nodes = wire["snapshot"]["nodes"].as_array_mut().unwrap();
        // Path equality ignores repeated separators; the archive still promises
        // exact native byte spelling, including historically accepted overrides.
        let original: Vec<u8> = serde_json::from_value(nodes[1]["path"].clone()).unwrap();
        let split = original.iter().rposition(|&b| b == b'/').unwrap();
        let mut unusual = original.clone();
        unusual.insert(split, b'/');
        nodes[1]["path"] = serde_json::to_value(&unusual).unwrap();
        nodes[1]["name"] = serde_json::json!("historical display override");
        let bytes = serde_json::to_vec(&wire).unwrap();
        let fast = archive_codec::decode(&mut BufReader::new(bytes.as_slice()), MAX_NODES).unwrap();
        let legacy: Archive = serde_json::from_slice(&bytes).unwrap();
        for archive in [fast, legacy] {
            let snapshot = archive.snapshot.into_snapshot().unwrap();
            validate(&snapshot, MAX_NODES).unwrap();
            assert_eq!(snapshot.path(1).unwrap().as_os_str().as_bytes(), unusual);
            assert_eq!(
                snapshot.display_name(1).unwrap(),
                "historical display override"
            );
            assert_eq!(
                serde_json::to_value(ArchiveRef {
                    version: 1,
                    metadata: &metadata,
                    snapshot: SnapshotRef(&snapshot)
                })
                .unwrap(),
                wire
            );
        }
        for field in ["is_directory", "hard_link"] {
            let mut bad = wire.clone();
            bad["snapshot"]["nodes"][0][field] = serde_json::json!(field == "hard_link");
            let bytes = serde_json::to_vec(&bad).unwrap();
            assert!(
                archive_codec::decode(&mut BufReader::new(bytes.as_slice()), MAX_NODES).is_err()
            );
            assert!(serde_json::from_slice::<Archive>(&bytes).is_err());
        }
    }

    #[test]
    fn archive_decoded_boundary_preserves_publication_and_selection() {
        let (directory, snapshot, metadata) = fixture();
        let mut library = Library::new(directory.path().join("library"));
        // Library saves are binary (v3); exports stay portable JSON (v2).
        let json = serde_json::to_vec(&ArchiveRef {
            version: 2,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        let mut binary = Vec::new();
        binary_archive::encode(&mut binary, &snapshot, &metadata).unwrap();
        let (json_boundary, binary_boundary) = (json.len() as u64, binary.len() as u64);
        assert!(binary_boundary < json_boundary);
        library.byte_limit = binary_boundary;
        let row = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let source = library.path(&row.id, "zst").unwrap();
        let destination = directory.path().join("export.hahakigamiscan");
        for limit in [binary_boundary, binary_boundary + 1] {
            library.byte_limit = limit;
            assert!(library.load(&row.id, |_| {}).is_ok());
        }
        for limit in [json_boundary, json_boundary + 1] {
            library.byte_limit = limit;
            library.export_scan(&row.id, &destination, |_| {}).unwrap();
            // Force the original decoder too: both paths share the decoded budget.
            for record_limit in [archive_codec::MAX_RECORD_BYTES, 32] {
                library.fast_record_limit = record_limit;
                let imported = library.import_scan(&destination, |_| {}).unwrap();
                assert!(library.load(&imported.id, |_| {}).is_ok());
            }
        }
        let selection = fs::read(library.root.join("selection.json")).unwrap();
        let entries = library.list().unwrap().entries.len();
        let original = fs::read(&source).unwrap();
        library.byte_limit = binary_boundary - 1;
        assert!(library
            .save(&snapshot, &metadata, |_| {})
            .unwrap_err()
            .contains("byte limit"));
        assert!(library
            .load(&row.id, |_| {})
            .unwrap_err()
            .contains("byte limit"));
        assert!(library.import_scan(&source, |_| {}).is_err());
        fs::write(&destination, b"keep existing export").unwrap();
        library.byte_limit = json_boundary - 1;
        assert!(library
            .export_scan(&row.id, &destination, |_| {})
            .unwrap_err()
            .contains("byte limit"));
        assert_eq!(fs::read(&destination).unwrap(), b"keep existing export");
        // The JSON decoders (fast and fallback) enforce the same budget on import.
        library.byte_limit = json_boundary + 1;
        library.export_scan(&row.id, &destination, |_| {}).unwrap();
        library.byte_limit = json_boundary - 1;
        for record_limit in [archive_codec::MAX_RECORD_BYTES, 32] {
            library.fast_record_limit = record_limit;
            assert!(library
                .import_scan(&destination, |_| {})
                .unwrap_err()
                .contains("byte limit"));
        }
        assert_eq!(fs::read(&source).unwrap(), original);
        assert_eq!(library.list().unwrap().entries.len(), entries);
        assert_eq!(
            fs::read(library.root.join("selection.json")).unwrap(),
            selection
        );
        assert!(!fs::read_dir(&library.root).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "tmp")));
    }

    // Every field a real scan can carry: native paths, issues, clone metadata with
    // shared accounting and deductions, warnings, worker rows and descriptor counters.
    fn full_fixture() -> (tempfile::TempDir, Snapshot, ScanMetadata) {
        let (directory, mut snapshot, metadata) = fixture();
        let ids: Vec<_> = snapshot
            .iter_nodes()
            .filter(|n| n.hard_link || n.display_name() == "odd\nname")
            .map(|n| n.id)
            .collect();
        for id in ids {
            let mut node = snapshot.legacy_node(id).unwrap();
            node.clone_metadata = Some(CloneMetadata {
                clone_id: 99,
                data_allocated_size: 2048,
                data_logical_size: 1,
                reference_count: 3,
                may_share_blocks: true,
                verified: true,
            });
            snapshot.replace_legacy_node(id, node).unwrap();
        }
        snapshot.accounting.model = AccountingModel::FullClonesV1;
        snapshot.accounting.checks_complete = true;
        snapshot.accounting.shared = true;
        snapshot.accounting.deduction_bytes = 2048;
        snapshot.accounting.warnings = vec!["clone check skipped".into()];
        for (id, allocation) in disk_scan::attributed_own_allocations(&snapshot)
            .unwrap()
            .into_iter()
            .enumerate()
        {
            snapshot
                .update_node(id as u32, |n| n.allocated_size = allocation)
                .unwrap();
        }
        for id in (1..snapshot.node_count() as u32).rev() {
            let node = snapshot.node(id).unwrap();
            let (parent, allocation) = (node.parent.unwrap(), node.allocated_size);
            snapshot
                .update_node(parent, |n| n.allocated_size += allocation)
                .unwrap();
        }
        snapshot.stats.allocated_bytes = snapshot.node(0).unwrap().allocated_size;
        snapshot.worker_stats = vec![WorkerStats {
            id: 3,
            entries: 6,
            cpu_us: Some(9),
            qos: "utility".into(),
            ..Default::default()
        }];
        snapshot.finalization_us = 1234;
        snapshot.descriptor_limit = Some(256);
        snapshot.descriptor_peak = Some(17);
        (directory, snapshot, metadata)
    }
    fn zstd_frame(data: &[u8]) -> Vec<u8> {
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }
    fn decoded(path: &Path) -> Vec<u8> {
        zstd::stream::decode_all(fs::read(path).unwrap().as_slice()).unwrap()
    }
    #[test]
    fn binary_archive_roundtrips_every_field_and_matches_json_v2() {
        let (directory, snapshot, metadata) = full_fixture();
        let library = Library::new(directory.path().join("library"));
        let row = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let saved = library.path(&row.id, "zst").unwrap();
        assert_eq!(&decoded(&saved)[..8], binary_archive::MAGIC);
        let (binary, binary_metadata) = library.load(&row.id, |_| {}).unwrap();
        // Attributions reused from validation match a fresh attribution pass.
        let mut refreshed = binary.clone();
        refreshed.refresh_clone_attributions().unwrap();
        let attributions = |s: &Snapshot| -> Vec<_> {
            (0..s.node_count() as u32)
                .map(|id| s.clone_attribution(id))
                .collect()
        };
        assert_eq!(attributions(&binary), attributions(&refreshed));
        assert!(attributions(&binary).iter().any(Option::is_some));
        // The portable export stays JSON v2; loading it exercises the old codec.
        let portable = directory.path().join("portable.hahakigamiscan");
        library.export_scan(&row.id, &portable, |_| {}).unwrap();
        assert_eq!(decoded(&portable)[0], b'{');
        let (json, json_metadata) = library
            .load_path(&portable, &mut |_| {}, None, &mut || {})
            .unwrap();
        let wire = |s: &Snapshot, m: &ScanMetadata| {
            serde_json::to_value(ArchiveRef {
                version: 2,
                metadata: m,
                snapshot: SnapshotRef(s),
            })
            .unwrap()
        };
        assert_eq!(wire(&binary, &binary_metadata), wire(&json, &json_metadata));
        assert_eq!(wire(&binary, &binary_metadata), wire(&snapshot, &metadata));
        // Facts the JSON view would hide or flatten.
        assert_eq!(binary.root.as_os_str(), snapshot.root.as_os_str());
        for id in 0..snapshot.node_count() as u32 {
            assert_eq!(
                binary.path(id).unwrap().as_os_str().as_bytes(),
                snapshot.path(id).unwrap().as_os_str().as_bytes()
            );
        }
        assert!(binary.iter_nodes().any(|n| binary
            .path(n.id)
            .unwrap()
            .as_os_str()
            .as_bytes()
            .contains(&0xff)));
        assert_eq!(binary.errors.len(), 1);
        assert_eq!(binary.exclusions.len(), 1);
        assert_eq!(binary.errors[0].raw_os_error, Some(libc::EACCES));
        assert_eq!(binary.accounting.warnings, ["clone check skipped"]);
        assert_eq!(binary.accounting.deduction_bytes, 2048);
        assert!(binary.accounting.shared);
        assert_eq!(
            binary
                .iter_nodes()
                .filter(|n| n.clone_metadata().is_some())
                .count(),
            3
        );
        assert_eq!(binary.worker_stats[0].cpu_us, Some(9));
        assert_eq!(
            (
                binary.finalization_us,
                binary.descriptor_limit,
                binary.descriptor_peak
            ),
            (1234, Some(256), Some(17))
        );
        // The encoding is canonical: a decoded archive re-encodes to the same bytes.
        let (mut first, mut second) = (Vec::new(), Vec::new());
        binary_archive::encode(&mut first, &snapshot, &metadata).unwrap();
        binary_archive::encode(&mut second, &binary, &binary_metadata).unwrap();
        assert_eq!(first, second);

        // Exact legacy spellings of the root (and of every child path) survive,
        // while the header root keeps its own spelling, as in JSON v2.
        let original_root = snapshot.root.as_os_str().as_bytes().to_vec();
        for suffix in [b"//".as_slice(), b"/./".as_slice()] {
            for variant_children in [false, true] {
                let mut spelled = original_root.clone();
                spelled.extend_from_slice(suffix);
                let mut compact = Snapshot::empty(snapshot.root.clone());
                let mut expected = Vec::new();
                for view in snapshot.iter_nodes() {
                    let mut node = snapshot.legacy_node(view.id).unwrap();
                    if node.id == 0 {
                        node.path = std::ffi::OsString::from_vec(spelled.clone()).into();
                    } else if variant_children {
                        let mut raw = spelled.clone();
                        raw.extend_from_slice(
                            &node.path.as_os_str().as_bytes()[original_root.len()..],
                        );
                        node.path = std::ffi::OsString::from_vec(raw).into();
                    }
                    expected.push(node.path.as_os_str().as_bytes().to_vec());
                    compact.append_legacy_node(node).unwrap();
                }
                compact.stats = snapshot.stats.clone();
                compact.accounting = snapshot.accounting.clone();
                compact.state = snapshot.state;
                assert!(compact.has_path_overrides());
                validate(&compact, MAX_NODES).unwrap();
                let mut encoded = Vec::new();
                binary_archive::encode(&mut encoded, &compact, &metadata).unwrap();
                let (again, _) =
                    binary_archive::decode(&mut encoded.as_slice(), MAX_NODES).unwrap();
                validate(&again, MAX_NODES).unwrap();
                assert!(again.has_path_overrides());
                assert_eq!(again.root.as_os_str().as_bytes(), original_root);
                for (id, path) in expected.iter().enumerate() {
                    assert_eq!(again.path(id as u32).unwrap().as_os_str().as_bytes(), path);
                }
            }
        }
    }

    #[test]
    fn binary_archive_keeps_display_labels_and_explicit_paths_on_ordinary_nodes() {
        let (_directory, snapshot, metadata) = full_fixture();
        let mut changed = snapshot.clone();
        // Display name differing from the native basename; a spelled-out path that
        // is not parent/basename; and both at once.
        for (id, label, spelling) in [
            (1u32, Some("Renamed \u{2713}"), false),
            (2, None, true),
            (3, Some(""), true),
        ] {
            let mut node = changed.legacy_node(id).unwrap();
            if let Some(label) = label {
                node.name = label.into();
            }
            if spelling {
                let mut raw = node.path.as_os_str().as_bytes().to_vec();
                let slash = raw.iter().rposition(|b| *b == b'/').unwrap();
                raw.insert(slash, b'/');
                node.path = std::ffi::OsString::from_vec(raw).into();
            }
            changed.replace_legacy_node(id, node).unwrap();
        }
        let mut encoded = Vec::new();
        binary_archive::encode(&mut encoded, &changed, &metadata).unwrap();
        let (again, _) = binary_archive::decode(&mut encoded.as_slice(), MAX_NODES).unwrap();
        validate(&again, MAX_NODES).unwrap();
        for id in 0..changed.node_count() as u32 {
            assert_eq!(
                again.path(id).unwrap().as_os_str().as_bytes(),
                changed.path(id).unwrap().as_os_str().as_bytes()
            );
            assert_eq!(
                again.node(id).unwrap().display_name(),
                changed.node(id).unwrap().display_name()
            );
        }
        let mut reencoded = Vec::new();
        binary_archive::encode(&mut reencoded, &again, &metadata).unwrap();
        assert_eq!(reencoded, encoded);
    }

    #[test]
    fn json_v2_archives_still_open_and_binary_files_import() {
        let (directory, snapshot, metadata) = full_fixture();
        let library = Library::new(directory.path().join("library"));
        let row = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let portable = directory.path().join("portable.hahakigamiscan");
        library.export_scan(&row.id, &portable, |_| {}).unwrap();
        // Importing a JSON export re-saves it as a binary library archive.
        let from_json = library.import_scan(&portable, |_| {}).unwrap();
        assert_eq!(
            &decoded(&library.path(&from_json.id, "zst").unwrap())[..8],
            binary_archive::MAGIC
        );
        // A binary file is accepted by import as well.
        let copy = directory.path().join("copy.zst");
        fs::copy(library.path(&row.id, "zst").unwrap(), &copy).unwrap();
        let from_binary = library.import_scan(&copy, |_| {}).unwrap();
        for id in [&from_json.id, &from_binary.id] {
            let (loaded, _) = library.load(id, |_| {}).unwrap();
            assert_eq!(
                serde_json::to_value(SnapshotRef(&loaded)).unwrap(),
                serde_json::to_value(SnapshotRef(&snapshot)).unwrap()
            );
        }
        // Version-one JSON (no clone accounting) is untouched by the new sniffing.
        let (directory, snapshot, metadata) = fixture();
        let library = Library::new(directory.path().join("legacy"));
        let mut bytes = serde_json::to_vec(&ArchiveRef {
            version: 1,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        let path = library.root.join("v1.zst");
        library.prepare().unwrap();
        fs::write(&path, zstd_frame(&bytes)).unwrap();
        assert!(library.import_scan(&path, |_| {}).is_ok());
        bytes.truncate(bytes.len() - 1);
        fs::write(&path, zstd_frame(&bytes)).unwrap();
        assert!(library.import_scan(&path, |_| {}).is_err());
    }

    #[test]
    fn binary_archive_rejects_truncation_flags_oversize_trailing_and_cache_magic() {
        let (directory, snapshot, metadata) = full_fixture();
        let mut bytes = Vec::new();
        binary_archive::encode(&mut bytes, &snapshot, &metadata).unwrap();
        let decode = |b: &[u8]| binary_archive::decode(&mut &*b, MAX_NODES);
        assert!(decode(&bytes).is_ok());
        for length in 0..bytes.len() {
            assert!(decode(&bytes[..length]).is_err(), "prefix {length}");
        }
        let header_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let count_at = 12 + header_len;
        let record = count_at + 4;
        let patched = |at: usize, patch: &[u8]| {
            let mut b = bytes.clone();
            b[at..at + patch.len()].copy_from_slice(patch);
            b
        };
        // Flags: unknown bit, directory-without-kind, hard-link on a directory,
        // a clone flag whose payload is absent, and a root without its explicit path.
        for flags in [0x84u8, 0x05, 0x0c, 0x64, 0x04] {
            assert!(decode(&patched(record + 56, &[flags])).is_err(), "{flags}");
        }
        // The next record is an ordinary child: its first field is the native
        // basename. A slash makes it a non-component; the explicit flag instead
        // reads the same bytes as the exact path.
        let first = u32::from_le_bytes(bytes[record + 57..record + 61].try_into().unwrap());
        let label = u32::from_le_bytes(bytes[record + 61..record + 65].try_into().unwrap());
        let children = u32::from_le_bytes(bytes[record + 65..record + 69].try_into().unwrap());
        let child = record
            + 69
            + first as usize
            + label.saturating_sub(1) as usize
            + 4 * children as usize
            + 29 * usize::from(bytes[record + 56] & 32 != 0);
        assert_eq!(bytes[child + 56] & 64, 0);
        let native_len = u32::from_le_bytes(bytes[child + 57..child + 61].try_into().unwrap());
        let mut slash = bytes.clone();
        slash[child + 69] = b'/';
        assert!(decode(&slash).is_err());
        let explicit = decode(&patched(child + 56, &[bytes[child + 56] | 64]))
            .unwrap()
            .0;
        assert_eq!(
            explicit.path(1).unwrap().as_os_str().as_bytes().len(),
            native_len as usize
        );
        assert_ne!(explicit.path(1).unwrap(), snapshot.path(1).unwrap());
        // Clone metadata is fixed binary: only the two defined flag bits exist.
        let clone_at = bytes
            .windows(16)
            .position(|w| w == [99u64.to_le_bytes(), 2048u64.to_le_bytes()].concat())
            .unwrap()
            + 28;
        for (flags, ok) in [(0u8, true), (3, true), (4, false), (0xff, false)] {
            assert_eq!(decode(&patched(clone_at, &[flags])).is_ok(), ok, "{flags}");
        }
        // Counts and lengths beyond the field, topology, node and header limits.
        for (at, value) in [
            (record + 57, u32::MAX),
            (record + 57, archive_codec::MAX_FIELD_BYTES as u32 + 1),
            (record + 61, u32::MAX),
            (record + 65, u32::MAX),
            (record + 65, snapshot.node_count() as u32),
            (count_at, 0),
            (count_at, MAX_NODES as u32 + 1),
            (8, u32::MAX),
            (8, 256 * 1024 * 1024 + 1),
        ] {
            assert!(decode(&patched(at, &value.to_le_bytes())).is_err(), "{at}");
        }
        assert!(binary_archive::decode(&mut bytes.as_slice(), 2).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode(&trailing).unwrap_err().contains("trailing decoded"));
        // Archive bytes are never a cache, nor cache bytes an archive; caches
        // additionally refuse the archive-only clone flag.
        let mut cache_magic = bytes.clone();
        cache_magic[..8].copy_from_slice(b"HAHCAC03");
        assert!(decode(&cache_magic).is_err());
        assert!(cache_codec::decode(&mut bytes.as_slice(), MAX_NODES).is_err());
        assert!(
            cache_codec::read_nodes(&mut &bytes[count_at..], MAX_NODES, 64 * 1024, false).is_err()
        );
        assert!(
            cache_codec::read_nodes(&mut &bytes[count_at..], MAX_NODES, 64 * 1024, true).is_ok()
        );

        // The same failures through a saved library file and its checksummed frame.
        let library = Library::new(directory.path().join("library"));
        let row = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let path = library.path(&row.id, "zst").unwrap();
        let valid = fs::read(&path).unwrap();
        assert!(library.load(&row.id, |_| {}).is_ok());
        for length in [0, 4, valid.len() - 1] {
            fs::write(&path, &valid[..length]).unwrap();
            assert!(library.load(&row.id, |_| {}).is_err());
        }
        let mut flipped = valid.clone();
        *flipped.last_mut().unwrap() ^= 1;
        fs::write(&path, flipped).unwrap();
        assert!(library.load(&row.id, |_| {}).is_err());
        for suffix in [vec![0], zstd_frame(b"extra")] {
            let mut extra = valid.clone();
            extra.extend(suffix);
            fs::write(&path, extra).unwrap();
            assert!(library
                .load(&row.id, |_| {})
                .unwrap_err()
                .contains("trailing compressed"));
        }
        let raw = decoded_bytes(&valid);
        for (data, expected) in [
            ([raw.clone(), vec![0]].concat(), "trailing decoded"),
            (cache_magic, ""),
            (raw[..raw.len() - 1].to_vec(), ""),
        ] {
            fs::write(&path, zstd_frame(&data)).unwrap();
            let error = library.load(&row.id, |_| {}).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        fs::write(&path, &valid).unwrap();
        let mut limited = Library::new(library.root.clone());
        limited.byte_limit = raw.len() as u64 - 1;
        assert!(limited
            .load(&row.id, |_| {})
            .unwrap_err()
            .contains("byte limit"));
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            library
                .load_cancellable(&row.id, |_| {}, &cancelled, || {})
                .unwrap_err(),
            OPEN_CANCELLED
        );
        assert!(library.load(&row.id, |_| {}).is_ok());
    }
    fn decoded_bytes(frame: &[u8]) -> Vec<u8> {
        zstd::stream::decode_all(frame).unwrap()
    }

    #[test]
    fn streamed_counters_cross_four_gib_without_large_allocations() {
        let old_boundary = MAX_CACHE_BYTES;
        assert_eq!(MAX_BYTES, 16 * 1024 * 1024 * 1024);
        let mut writes = Vec::new();
        let mut writer = LimitedWriter {
            inner: io::sink(),
            count: old_boundary - 1,
            limit: MAX_BYTES,
            progress: |count| writes.push(count),
        };
        writer.write_all(&[1, 2]).unwrap();
        assert_eq!(writer.count, old_boundary + 1);
        assert_eq!(writes, [old_boundary + 1]);
        let mut reader = LimitedReader {
            cancelled: None,
            inner: &[1, 2][..],
            count: old_boundary - 1,
            limit: MAX_BYTES,
            progress: |_| {},
        };
        reader.read_exact(&mut [0; 2]).unwrap();
        assert_eq!(reader.count, old_boundary + 1);
        let library = Library::new(PathBuf::new());
        assert_eq!(library.cache_byte_limit(), old_boundary);
        let mut restricted = library;
        restricted.byte_limit = 32;
        assert_eq!(restricted.cache_byte_limit(), 32);
        let mut writer = LimitedWriter {
            inner: io::sink(),
            count: old_boundary - 1,
            limit: old_boundary,
            progress: |_| {},
        };
        assert!(writer.write_all(&[1, 2]).is_err());
        let mut reader = LimitedReader {
            cancelled: None,
            inner: &[1, 2][..],
            count: old_boundary - 1,
            limit: old_boundary,
            progress: |_| {},
        };
        assert!(reader.read(&mut [0; 2]).is_err());
    }

    #[test]
    fn wire_array_limits_reject_before_reading_the_tail() {
        for bytes in [b"[1,2,3,4,".as_slice(), b"[255,2,3,4,".as_slice()] {
            let mut decoder = serde_json::Deserializer::from_slice(bytes);
            let error = bounded_vec::<_, u32>(&mut decoder, 3).unwrap_err();
            assert!(error.to_string().contains("field limit"));
            let mut decoder = serde_json::Deserializer::from_reader(bytes);
            let error = bounded_vec::<_, u8>(&mut decoder, 3).unwrap_err();
            assert!(error.to_string().contains("field limit"));
        }
        for reader in [false, true] {
            let bytes = b"[1,2,3]".as_slice();
            let values = if reader {
                bounded_vec::<_, u32>(&mut serde_json::Deserializer::from_reader(bytes), 3)
            } else {
                bounded_vec::<_, u32>(&mut serde_json::Deserializer::from_slice(bytes), 3)
            }
            .unwrap();
            assert_eq!(values, [1, 2, 3]);
        }
    }

    #[test]
    fn metadata_limits_cover_fast_and_positional_decoders() {
        let (_directory, snapshot, metadata) = fixture();
        let base = serde_json::to_value(ArchiveRef {
            version: 2,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        let issue = serde_json::json!({"path": [], "message": "", "error_kind": null, "raw_os_error": null});
        let worker = serde_json::to_value(WorkerStats::default()).unwrap();
        for (field, values) in [
            ("errors", vec![issue.clone(); 10_001]),
            ("exclusions", vec![issue; 10_001]),
            ("worker_stats", vec![worker; disk_scan::MAX_WORKERS + 1]),
            ("warnings", vec![serde_json::json!(""); 33]),
            ("warnings", vec![serde_json::json!("x".repeat(4097))]),
        ] {
            let mut wire = base.clone();
            if field == "warnings" {
                wire["snapshot"]["accounting"][field] = values.into();
            } else {
                wire["snapshot"][field] = values.into();
            }
            for positional in [false, true] {
                let wire = if positional {
                    serde_json::json!([wire["version"], wire["metadata"], wire["snapshot"]])
                } else {
                    wire.clone()
                };
                let bytes = serde_json::to_vec(&wire).unwrap();
                let error = if positional {
                    serde_json::from_reader::<_, Archive>(bytes.as_slice())
                        .err()
                        .unwrap()
                        .to_string()
                } else {
                    format!(
                        "{:?}",
                        archive_codec::decode(&mut BufReader::new(bytes.as_slice()), MAX_NODES)
                            .err()
                            .unwrap()
                    )
                };
                assert!(error.contains("limit"), "{field}: {error}");
            }
        }
        // Check the shared retained-byte budget without constructing large diagnostics.
        for streaming in [false, true] {
            let bytes = br#"["1234","5678","tail"#.as_slice();
            let error = if streaming {
                bounded_vec_bytes::<_, String, _>(
                    &mut serde_json::Deserializer::from_reader(bytes),
                    10,
                    7,
                    String::capacity,
                )
            } else {
                bounded_vec_bytes::<_, String, _>(
                    &mut serde_json::Deserializer::from_slice(bytes),
                    10,
                    7,
                    String::capacity,
                )
            }
            .unwrap_err();
            assert!(error.to_string().contains("metadata exceeds byte limit"));
        }
        let bytes = format!("{{\"warnings\":[{},", vec!["\"\""; 33].join(","));
        assert!(
            serde_json::from_reader::<_, AccountingInfo>(bytes.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("warnings exceed limit")
        );
    }

    #[test]
    fn archive_string_limit_covers_fast_and_positional_streaming_decoders() {
        let (_directory, snapshot, metadata) = fixture();
        let wire = serde_json::to_value(ArchiveRef {
            version: 1,
            metadata: &metadata,
            snapshot: SnapshotRef(&snapshot),
        })
        .unwrap();
        for oversized in [false, true] {
            let mut wire = wire.clone();
            wire["snapshot"]["nodes"][0]["name"] =
                serde_json::json!("x".repeat(if oversized { 257 } else { 256 }));
            for positional in [false, true] {
                let wire = if positional {
                    serde_json::json!([wire["version"], wire["metadata"], wire["snapshot"]])
                } else {
                    wire.clone()
                };
                let bytes = serde_json::to_vec(&wire).unwrap();
                // One-byte buffers also split escaped quotes and backslashes.
                let guarded = archive_codec::StringLimitedReader::new(bytes.as_slice(), 256);
                let mut reader = BufReader::with_capacity(1, guarded);
                let result = if positional {
                    let mut decoder = serde_json::Deserializer::from_reader(&mut reader);
                    Archive::deserialize(&mut decoder).map_err(|e| e.to_string())
                } else {
                    archive_codec::decode(&mut reader, MAX_NODES).map_err(|e| format!("{e:?}"))
                };
                if oversized {
                    assert!(result.err().unwrap().contains("string exceeds field limit"));
                } else {
                    assert!(result.is_ok(), "valid string at the boundary");
                }
            }
        }
        let bytes = br#"["escaped\\\"quote", "next"]"#;
        let reader = archive_codec::StringLimitedReader::new(bytes.as_slice(), 32);
        assert!(
            serde_json::from_reader::<_, Vec<String>>(BufReader::with_capacity(1, reader)).is_ok()
        );
        let bytes = b"[\"12345";
        let reader = archive_codec::StringLimitedReader::new(bytes.as_slice(), 4);
        assert!(
            serde_json::from_reader::<_, Vec<String>>(BufReader::with_capacity(1, reader))
                .unwrap_err()
                .to_string()
                .contains("field limit")
        );
    }

    #[test]
    fn oversized_fast_records_fall_back_without_changing_archive_or_limits() {
        let (directory, snapshot, metadata) = fixture();
        let mut library = Library::new(directory.path().join("library"));
        let row = library.save(&snapshot, &metadata, |_| {}).unwrap();
        let path = library.path(&row.id, "zst").unwrap();
        let archive = fs::read(&path).unwrap();
        // Exercise the same fallback as a 128 MiB node or metadata envelope,
        // without a large test allocation.
        library.fast_record_limit = 32;
        let (loaded, loaded_metadata) = library.load(&row.id, |_| {}).unwrap();
        assert_eq!(
            serde_json::to_value(SnapshotRef(&loaded)).unwrap(),
            serde_json::to_value(SnapshotRef(&snapshot)).unwrap()
        );
        assert_eq!(loaded_metadata.completed_at_ms, metadata.completed_at_ms);
        assert_eq!(fs::read(&path).unwrap(), archive);
        library.node_limit = 2;
        assert!(library.load(&row.id, |_| {}).is_err());
        library.node_limit = MAX_NODES;
        library.byte_limit = 100;
        assert!(library.load(&row.id, |_| {}).is_err());
    }

    #[test]
    fn fallback_checks_checksum_and_both_kinds_of_trailing_data() {
        let (directory, snapshot, metadata) = fixture();
        let mut library = Library::new(directory.path().join("library"));
        let row = library.save(&snapshot, &metadata, |_| {}).unwrap();
        library.fast_record_limit = 32;
        let path = library.path(&row.id, "zst").unwrap();
        let original = fs::read(&path).unwrap();
        let mut corrupt = original.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        fs::write(&path, corrupt).unwrap();
        assert!(library.load(&row.id, |_| {}).is_err());
        let mut appended = original;
        appended.extend_from_slice(b"unexpected compressed tail");
        fs::write(&path, appended).unwrap();
        assert!(library.load(&row.id, |_| {}).is_err());
        library
            .atomic(&path, |file| {
                let mut encoder = zstd::stream::write::Encoder::new(file, 3).map_err(err)?;
                encoder.include_checksum(true).map_err(err)?;
                serde_json::to_writer(
                    &mut encoder,
                    &ArchiveRef {
                        version: 1,
                        metadata: &metadata,
                        snapshot: SnapshotRef(&snapshot),
                    },
                )
                .map_err(err)?;
                encoder.write_all(b" true").map_err(err)?;
                encoder.finish().map_err(err)?.sync_all().map_err(err)
            })
            .unwrap();
        assert!(library.load(&row.id, |_| {}).is_err());
    }

    fn benchmark_load(library: &Library, id: &str, legacy: bool) -> (Snapshot, u128, u128) {
        let start = std::time::Instant::now();
        let file = BufReader::new(open(&library.path(id, "zst").unwrap()).unwrap());
        let decoder = zstd::stream::read::Decoder::new(file)
            .unwrap()
            .single_frame();
        let mut reader = BufReader::new(decoder);
        let archive = if legacy {
            let mut deserializer = serde_json::Deserializer::from_reader(&mut reader);
            let archive = Archive::deserialize(&mut deserializer).unwrap();
            deserializer.end().unwrap();
            archive
        } else {
            archive_codec::decode(&mut reader, MAX_NODES).unwrap()
        };
        let decode_us = start.elapsed().as_micros();
        let start = std::time::Instant::now();
        let snapshot = archive.snapshot.into_snapshot().unwrap();
        validate(&snapshot, MAX_NODES).unwrap();
        validate_metadata(&archive.metadata).unwrap();
        (snapshot, decode_us, start.elapsed().as_micros())
    }

    #[test]
    #[ignore = "million-entry archive benchmark; run explicitly with --ignored --nocapture"]
    fn million_entry_streaming_benchmark() {
        let (d, mut s, m) = fixture();
        let template_id = s
            .iter_nodes()
            .find(|n| n.identity.kind == disk_scan::NodeKind::File && !n.hard_link)
            .unwrap()
            .id;
        let template = s.legacy_node(template_id).unwrap();
        let mut root = s.legacy_node(0).unwrap();
        root.children = (1..1_000_000).collect();
        root.logical_size = root.own_logical_size + template.own_logical_size * 999_999;
        root.allocated_size = root.own_allocated_size + template.own_allocated_size * 999_999;
        let mut fresh = Snapshot::empty(s.root.clone());
        fresh.append_legacy_node(root).unwrap();
        for i in 1..1_000_000 {
            let mut n = template.clone();
            n.id = i;
            n.parent = Some(0);
            n.children.clear();
            n.name = format!("f{i}");
            n.path = s.root.join(&n.name);
            n.logical_size = n.own_logical_size;
            n.allocated_size = n.own_allocated_size;
            fresh.append_legacy_node(n).unwrap();
        }
        s = fresh;
        s.state = ScanState::Completed;
        s.stats.entries = s.node_count() as u64;
        s.stats.directories = 1;
        s.stats.logical_bytes = s.node(0).unwrap().logical_size;
        s.stats.allocated_bytes = s.node(0).unwrap().allocated_size;
        let l = Library::new(d.path().join("library"));
        let start = std::time::Instant::now();
        let row = l.save(&s, &m, |_| {}).unwrap();
        let save_ms = start.elapsed().as_millis();
        drop(s);
        let start = std::time::Instant::now();
        let (loaded, _) = l.load(&row.id, |_| {}).unwrap();
        assert_eq!(loaded.node_count(), 1_000_000);
        let load_ms = start.elapsed().as_millis();
        drop(loaded);
        // Set only for a fresh-process peak-RSS measurement; repeated loads
        // retain allocator arenas and must not be compared to isolated runs.
        if std::env::var_os("HAHAKIGAMI_BENCH_SINGLE_LOAD").is_none() {
            for pair in 0..3 {
                for legacy in if pair % 2 == 0 {
                    [true, false]
                } else {
                    [false, true]
                } {
                    let (loaded, decode_us, validate_us) = benchmark_load(&l, &row.id, legacy);
                    assert_eq!(loaded.node_count(), 1_000_000);
                    eprintln!("paired_load: pair={pair}, legacy={legacy}, decode_us={decode_us}, validate_us={validate_us}");
                    drop(loaded);
                }
            }
        }
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) }, 0);
        eprintln!("million entries: save_ms={save_ms}, load_ms={}, archive_bytes={}, peak_rss_native_units={}",load_ms,row.file_bytes,usage.ru_maxrss);
    }
    #[test]
    #[ignore = "read-only actual archive profile; supply HAH_PROFILE_ARCHIVE explicitly"]
    fn existing_archive_readonly_profile() {
        let path = PathBuf::from(
            std::env::var_os("HAH_PROFILE_ARCHIVE").expect("explicit archive path required"),
        );
        let library = Library::new(path.parent().unwrap().to_path_buf());
        let mut file = BufReader::new(open(&path).unwrap());
        let before = file.get_ref().metadata().unwrap();
        let mut header = [0u8; 5];
        file.read_exact(&mut header).unwrap();
        assert_eq!(header[..4], [0x28, 0xb5, 0x2f, 0xfd]);
        assert_ne!(header[4] & 4, 0);
        use std::io::Seek;
        for legacy in [true, false] {
            file.rewind().unwrap();
            let start = std::time::Instant::now();
            let archive = library
                .decode_archive(&mut file, &mut |_| {}, !legacy, None)
                .unwrap();
            assert_eq!(archive.version, 2);
            let decode_ms = start.elapsed().as_millis();
            let snapshot = archive.snapshot.into_snapshot().unwrap();
            let validate_start = std::time::Instant::now();
            validate(&snapshot, library.node_limit).unwrap();
            validate_metadata(&archive.metadata).unwrap();
            eprintln!("actual_readonly_load: legacy={legacy}, entries={}, decode_ms={decode_ms}, validate_ms={}, total_ms={}",snapshot.node_count(),validate_start.elapsed().as_millis(),start.elapsed().as_millis());
            drop(snapshot);
        }
        let after = file.get_ref().metadata().unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    }
    #[test]
    #[ignore = "read-only actual archive conversion profile; supply HAH_PROFILE_ARCHIVE explicitly"]
    fn existing_archive_v3_conversion_profile() {
        use std::io::Seek;
        use std::time::Instant;
        let path = PathBuf::from(
            std::env::var_os("HAH_PROFILE_ARCHIVE").expect("explicit archive path required"),
        );
        let source = Library::new(path.parent().unwrap().to_path_buf());
        let mut file = BufReader::new(open(&path).unwrap());
        let before = file.get_ref().metadata().unwrap();
        // JSON v2 baseline, split like the production load: decode / validate / refresh.
        let start = Instant::now();
        let archive = source
            .decode_archive(&mut file, &mut |_| {}, true, None)
            .unwrap();
        assert_eq!(archive.version, 2);
        let mut snapshot = archive.snapshot.into_snapshot().unwrap();
        let metadata = archive.metadata;
        let v2_decode_ms = start.elapsed().as_millis();
        let step = Instant::now();
        validate(&snapshot, source.node_limit).unwrap();
        validate_metadata(&metadata).unwrap();
        let v2_validate_ms = step.elapsed().as_millis();
        let step = Instant::now();
        snapshot.refresh_clone_attributions().unwrap();
        let v2_refresh_ms = step.elapsed().as_millis();
        let v2_total_ms = start.elapsed().as_millis();
        let entries = snapshot.node_count();
        // Write only into a fresh temporary library, never beside the source.
        let directory = tempfile::tempdir().unwrap();
        let target = Library::new(directory.path().join("v3-library"));
        let save = Instant::now();
        let row = target
            .save_format_selection(&snapshot, &metadata, &mut |_| {}, false, None, false)
            .unwrap();
        let save_ms = save.elapsed().as_millis();
        let v3_path = target.path(&row.id, "zst").unwrap();
        let (v2_bytes, v3_bytes) = (before.len(), fs::metadata(&v3_path).unwrap().len());
        let expected = serde_json::to_value(SnapshotRef(&snapshot)).unwrap();
        drop(snapshot);
        // Same phases on the v3 copy, then the production entry point end to end.
        let mut v3_file = BufReader::new(open(&v3_path).unwrap());
        assert!(sniff_binary_archive(&mut v3_file).unwrap());
        v3_file.rewind().unwrap();
        let start = Instant::now();
        let (mut loaded, loaded_metadata) = target
            .decode_binary(&mut v3_file, &mut |_| {}, None)
            .unwrap();
        let v3_decode_ms = start.elapsed().as_millis();
        let step = Instant::now();
        validate(&loaded, target.node_limit).unwrap();
        validate_metadata(&loaded_metadata).unwrap();
        let v3_validate_ms = step.elapsed().as_millis();
        let step = Instant::now();
        loaded.refresh_clone_attributions().unwrap();
        let v3_refresh_ms = step.elapsed().as_millis();
        let v3_total_ms = start.elapsed().as_millis();
        assert_eq!(
            serde_json::to_value(SnapshotRef(&loaded)).unwrap(),
            expected
        );
        drop(loaded);
        let start = Instant::now();
        let (loaded, _) = target
            .load_path(&v3_path, &mut |_| {}, None, &mut || {})
            .unwrap();
        let load_path_ms = start.elapsed().as_millis();
        assert_eq!(loaded.node_count(), entries);
        eprintln!("v2_json: entries={entries}, decode_ms={v2_decode_ms}, validate_ms={v2_validate_ms}, refresh_ms={v2_refresh_ms}, total_ms={v2_total_ms}, compressed_bytes={v2_bytes}");
        eprintln!("v3_binary: entries={entries}, decode_ms={v3_decode_ms}, validate_ms={v3_validate_ms}, refresh_ms={v3_refresh_ms}, total_ms={v3_total_ms}, compressed_bytes={v3_bytes}, save_ms={save_ms}, load_path_ms={load_path_ms}");
        let after = open(&path).unwrap().metadata().unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    }
    #[test]
    #[ignore = "repeated v3 load for sampling; supply HAH_PROFILE_ARCHIVE and HAH_PROFILE_V3_LIBRARY"]
    fn v3_archive_repeated_load_profile() {
        let source = PathBuf::from(std::env::var_os("HAH_PROFILE_ARCHIVE").unwrap());
        let library = Library::new(std::env::var_os("HAH_PROFILE_V3_LIBRARY").unwrap().into());
        // Import once (reads the source read-only) so later runs only load v3.
        let id = match library.list().unwrap().entries.first() {
            Some(entry) => entry.id.clone(),
            None => library.import_scan(&source, |_| {}).unwrap().id,
        };
        for round in 0..3 {
            let start = std::time::Instant::now();
            let (snapshot, _) = library.load(&id, |_| {}).unwrap();
            eprintln!(
                "v3_load round={round} entries={} ms={}",
                snapshot.node_count(),
                start.elapsed().as_millis()
            );
        }
    }
}

#[cfg(test)]
mod cache_format_tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};
    fn fixture() -> (tempfile::TempDir, Library, Snapshot, ScanMetadata) {
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
            completed_at_ms: 2,
            workers: 1,
            worker_mode: "auto".into(),
            worker_source: "hardware".into(),
            qos: ScanQos::Automatic,
        };
        let l = Library::new(d.path().join("cache"));
        (d, l, s, m)
    }
    fn compressed(data: &[u8]) -> Vec<u8> {
        let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        encoder.include_checksum(true).unwrap();
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }
    #[test]
    fn parallel_cache_gate_preserves_single_worker_small_and_historical_paths() {
        assert!(!parallel_cache_eligible(true, PARALLEL_CACHE_MIN_NODES, 1));
        assert!(!parallel_cache_eligible(
            true,
            PARALLEL_CACHE_MIN_NODES - 1,
            4
        ));
        assert!(!parallel_cache_eligible(false, PARALLEL_CACHE_MIN_NODES, 4));
        assert!(parallel_cache_eligible(true, PARALLEL_CACHE_MIN_NODES, 2));
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
        assert!(l.save_cache(&invalid, &m).is_err());
        assert_eq!(fs::read_dir(&l.root).unwrap().count(), 0);
        let mut limited = Library::new(l.root.clone());
        limited.byte_limit = 32;
        assert!(limited
            .save_cache(&s, &m)
            .unwrap_err()
            .contains("byte limit"));
        assert_eq!(fs::read_dir(&l.root).unwrap().count(), 0);
        let saved = l.save_cache(&s, &m).unwrap();
        assert_eq!(
            l.load_cache(&saved.id).unwrap().0.node_count(),
            s.node_count()
        );
        assert!(!l.root.join("selection.json").exists());
    }
    #[test]
    fn parallel_validation_failure_cannot_publish_even_after_encoding() {
        let (_d, l, _s, _m) = fixture();
        l.prepare().unwrap();
        let archive = l.root.join("invalid.zst");
        let encoded = std::sync::atomic::AtomicBool::new(false);
        let result = l.atomic(&archive, |mut file| {
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
    #[ignore = "readonly real-cache durable save probe; explicit environment paths required"]
    fn cache_publication_overlap_probe() {
        let source = PathBuf::from(std::env::var("HAH_CACHE_SAVE_PROBE_SOURCE").unwrap());
        let output = PathBuf::from(std::env::var("HAH_CACHE_SAVE_PROBE_OUTPUT").unwrap());
        assert!(!output.exists());
        fs::create_dir_all(&output).unwrap();
        let library = Library::new(source);
        let row = library.list().unwrap().entries.remove(0);
        let (snapshot, metadata) = library.load_cache(&row.id).unwrap();
        assert!(snapshot.node_count() >= PARALLEL_CACHE_MIN_NODES && metadata.workers > 1);
        let mut rows = Vec::new();
        let mut reference = None;
        for pair in 0..6 {
            for parallel in if pair % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let destination = Library::new(output.join(format!("save-{pair}-{parallel}")));
                let start = std::time::Instant::now();
                let saved = destination
                    .save_format(&snapshot, &metadata, &mut |_| {}, true, Some(parallel))
                    .unwrap();
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.;
                let bytes = fs::read(destination.path(&saved.id, "zst").unwrap()).unwrap();
                if let Some(expected) = &reference {
                    assert_eq!(&bytes, expected);
                } else {
                    reference = Some(bytes);
                }
                let (loaded, _) = destination.load_cache(&saved.id).unwrap();
                assert_eq!(loaded.node_count(), snapshot.node_count());
                assert_eq!(loaded.stats.logical_bytes, snapshot.stats.logical_bytes);
                rows.push(serde_json::json!({"pair":pair,"parallel":parallel,"elapsed_ms":elapsed_ms,"bytes":saved.file_bytes}));
                eprintln!("publication_probe: pair={pair} parallel={parallel} ms={elapsed_ms}");
            }
        }
        fs::write(
            output.join("report.json"),
            serde_json::to_vec_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn nonregular_cache_archives_are_rejected_without_blocking() {
        let (_d, l, s, m) = fixture();
        let row = l.save_cache(&s, &m).unwrap();
        let path = l.path(&row.id, "zst").unwrap();
        fs::remove_file(&path).unwrap();
        let native = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(native.as_ptr(), 0o600) }, 0);
        assert!(l.load_cache(&row.id).unwrap_err().contains("not regular"));
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(l.load_cache(&row.id).unwrap_err().contains("not regular"));
    }
    #[test]
    fn cache_checksum_truncation_and_trailing_data_are_rejected() {
        let (_d, l, s, m) = fixture();
        let row = l.save_cache(&s, &m).unwrap();
        let path = l.path(&row.id, "zst").unwrap();
        let valid = fs::read(&path).unwrap();
        let (loaded, _) = l.load_cache(&row.id).unwrap();
        assert_eq!(loaded.node_count(), s.node_count());
        for len in [0, 4, valid.len() - 1] {
            fs::write(&path, &valid[..len]).unwrap();
            assert!(l.load_cache(&row.id).is_err());
        }
        let mut bad = valid.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        fs::write(&path, bad).unwrap();
        assert!(l.load_cache(&row.id).is_err());
        for suffix in [vec![0], compressed(b"extra")] {
            let mut bad = valid.clone();
            bad.extend(suffix);
            fs::write(&path, bad).unwrap();
            assert!(l
                .load_cache(&row.id)
                .unwrap_err()
                .contains("trailing compressed"));
        }
        let mut decoded = zstd::stream::decode_all(valid.as_slice()).unwrap();
        decoded.push(0);
        fs::write(&path, compressed(&decoded)).unwrap();
        assert!(l
            .load_cache(&row.id)
            .unwrap_err()
            .contains("trailing decoded"));
        fs::write(&path, &valid).unwrap();
        let mut limited = Library::new(l.root.clone());
        limited.byte_limit = 32;
        assert!(limited.load_cache(&row.id).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("missing", &path).unwrap();
        assert!(l.load_cache(&row.id).is_err());
    }
    #[test]
    fn compact_final_tail_enforces_byte_limit_without_publication() {
        let (_d, l, s, m) = fixture();
        let mut encoded = Vec::new();
        cache_codec::encode(&mut encoded, &s, &m).unwrap();
        let mut limited = Library::new(l.root.clone());
        limited.byte_limit = encoded.len() as u64 - 1;
        assert!(limited
            .save_cache(&s, &m)
            .unwrap_err()
            .contains("byte limit"));
        assert_eq!(fs::read_dir(&l.root).unwrap().count(), 0);
        limited.byte_limit += 1;
        let row = limited.save_cache(&s, &m).unwrap();
        assert!(limited.load_cache(&row.id).is_ok());
    }
    #[test]
    fn compact_cache_does_not_publish_historical_selection() {
        let (_d, l, s, m) = fixture();
        let cache = l.save_cache(&s, &m).unwrap();
        assert!(l.list().unwrap().last_opened_id.is_none());
        let history = l.save(&s, &m, |_| {}).unwrap();
        assert_eq!(l.list().unwrap().last_opened_id, Some(history.id.clone()));
        let before = std::fs::read(l.root.join("selection.json")).unwrap();
        l.save_cache(&s, &m).unwrap();
        assert_eq!(
            std::fs::read(l.root.join("selection.json")).unwrap(),
            before
        );
        assert!(l.load_cache(&cache.id).is_ok());
    }
    #[test]
    fn compact_cache_and_historical_formats_remain_distinct_and_validated() {
        let (_d, l, s, m) = fixture();
        let cache = l.save_cache(&s, &m).unwrap();
        let history = l.save(&s, &m, |_| {}).unwrap();
        assert!(l.load_cache(&cache.id).is_ok());
        assert!(l.load(&history.id, |_| {}).is_ok());
        assert!(l.load(&cache.id, |_| {}).is_err());
        assert!(l.load_cache(&history.id).is_err());
        assert_eq!(
            &zstd::stream::decode_all(
                fs::read(l.path(&history.id, "zst").unwrap())
                    .unwrap()
                    .as_slice()
            )
            .unwrap()[..8],
            binary_archive::MAGIC
        );
        let mut invalid = s.clone();
        invalid.update_node(1, |n| n.parent = None).unwrap();
        assert!(l.save_cache(&invalid, &m).is_err());
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
        assert!(l.load_cache(&cache.id).is_err());
    }
}
