//! Metadata-only, bounded concurrent disk scanner. Native paths never leave this crate implicitly.
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex, RwLock,
    },
    time::Instant,
};
mod bulk;
mod tree;
pub use tree::{LegacyTreeBuilder, NodeMetadata, NodePathCursor, NodeRef};
mod incremental;
pub use incremental::{
    scan_live_incremental_authorized, scan_live_incremental_authorized_with_validation_workers,
    IncrementalSeed, ReuseStats,
};
mod restriction;
mod retry;
pub use firmlinks::public_paths as firmlink_paths;
pub use restriction::{is_system_restriction, VOLUME_METADATA_NAMES};
mod diagnostics;
pub use diagnostics::WorkerStats;
mod storage;
pub use storage::{storage_stats, BufferStorage, NodeCounts, StorageStats};
#[cfg(unix)]
mod descriptor;
mod firmlinks;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Backend {
    Auto,
    Portable,
    Bulk,
}
/// A requested worker-thread QoS class, never a CPU affinity or P/E-core assignment.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ScanQos {
    #[default]
    Automatic,
    UserInitiated,
    Background,
}
#[derive(Clone, Copy, Debug)]
pub struct ScanOptions {
    pub workers: usize,
    pub backend: Backend,
    pub qos: ScanQos,
    /// Retained parent descriptors this scan may hold. `None` takes `default_descriptor_limit()`.
    pub descriptor_limit: Option<usize>,
}
/// Hard cap shared with desktop settings and benchmark worker reporting.
pub const MAX_WORKERS: usize = 16;
/// Concurrent child-directory opens allowed during enumeration. On an M4 Pro,
/// `/System/Library` (about three entries per directory) walks 40% faster at
/// three than unbounded at seven workers; large-directory trees are unchanged.
#[cfg(unix)]
const DIRECTORY_OPEN_PERMITS: usize = 3;
impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            workers: 8,
            backend: Backend::Auto,
            qos: ScanQos::Automatic,
            descriptor_limit: None,
        }
    }
}
/// The retained-descriptor limit of an unconstrained scan. Scans that run side by side split this total.
#[cfg(unix)]
pub fn default_descriptor_limit() -> usize {
    descriptor::Budget::default_limit()
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeKind {
    Directory,
    File,
    Symlink,
    Other,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub device: u64,
    pub inode: u64,
    pub kind: NodeKind,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccountingModel {
    #[default]
    HardLinksV1,
    FullClonesV1,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountingInfo {
    pub model: AccountingModel,
    #[serde(deserialize_with = "accounting_warnings")]
    pub warnings: Vec<String>,
    pub checks_complete: bool,
    pub shared: bool,
    pub deduction_bytes: u64,
    pub metadata_queries: u64,
    pub revalidation_queries: u64,
    pub metadata_us: u64,
    pub revalidation_us: u64,
    /// Attempts to walk from the held root to a parent, including failed walks.
    pub revalidation_parent_walks: u64,
    /// Actual final rich bulk syscall attempts, including failures and EOF.
    pub revalidation_bulk_queries: u64,
    /// Per-file fallback query attempts during final validation.
    pub revalidation_direct_queries: u64,
    pub revalidation_workers: u32,
    /// Validation worker wall time; revalidation_us sums worker activity.
    pub revalidation_wall_us: u64,
}
impl AccountingInfo {
    /// The one way to record an accounting uncertainty: it also makes the scan unfit as a
    /// rescan baseline. The cap matches what `accounting_warnings` accepts on decode.
    pub(crate) fn warn(&mut self, message: String) {
        self.checks_complete = false;
        if self.warnings.len() < 32 {
            self.warnings.push(message);
        }
    }
    /// A full-clone accounting whose checks completed and found nothing shared or deducted.
    pub fn verified_nonshared(&self) -> bool {
        self.model == AccountingModel::FullClonesV1
            && self.checks_complete
            && !self.shared
            && self.warnings.is_empty()
            && self.deduction_bytes == 0
    }
}
fn accounting_warnings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    struct Warnings;
    impl<'de> serde::de::Visitor<'de> for Warnings {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("at most 32 accounting warnings of at most 4096 bytes each")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut a: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(value) = a.next_element::<String>()? {
                if out.len() == 32 || value.len() > 4096 {
                    return Err(serde::de::Error::custom("Accounting warnings exceed limit"));
                }
                out.push(value);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(Warnings)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloneMetadata {
    pub clone_id: u64,
    pub data_allocated_size: u64,
    pub data_length: u64,
    pub reference_count: u32,
    pub may_share_blocks: bool,
    pub verified: bool,
}
mod clones;
#[cfg(test)]
pub(crate) use clones::hardlink_allocations;
pub use clones::{allocation_plan, AllocationPlan};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: u32,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub name: String,
    pub path: PathBuf,
    pub identity: Identity,
    pub allocated_size: u64,
    /// A file's byte count: only for clone verification and rescan change detection, never summed or shown.
    pub length: u64,
    pub own_allocated_size: u64,
    pub is_directory: bool,
    pub hard_link: bool,
    #[serde(default)]
    pub excluded: bool,
    #[serde(default)]
    pub clone_metadata: Option<CloneMetadata>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScanState {
    Running,
    Completed,
    Cancelled,
    Failed,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanStats {
    pub entries: u64,
    pub directories: u64,
    pub allocated_bytes: u64,
    pub errors: u64,
    #[serde(default)]
    pub excluded: u64,
    #[serde(default)]
    pub restricted_system_issues: u64,
    pub elapsed_ms: u64,
    pub incomplete: bool,
    /// Milliseconds until every entry was found, or 0 before that. From then on
    /// the tree is complete with raw sizes while clone sharing is verified;
    /// verified shared sizes are final only once the scan completes.
    #[serde(default)]
    pub walk_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanIssue {
    pub path: PathBuf,
    pub message: String,
    #[serde(default)]
    pub error_kind: Option<String>,
    #[serde(default)]
    pub raw_os_error: Option<i32>,
}
/// Diagnostics for the end of enumeration: idle workers waiting on the last
/// listings, and which listings were slowest. Never written to caches.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TailStats {
    /// Microseconds from scan start until all enumeration workers joined.
    pub walk_end_us: u64,
    /// Sum of each worker's final wait before it exited (idle tail).
    pub tail_idle_us: u64,
    /// Slowest directory listings: (start_us, duration_us, entries, worker).
    pub slow_listings: Vec<(u64, u64, u64, usize)>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub accounting: AccountingInfo,
    pub root: PathBuf,
    pub(crate) tree: tree::Tree,
    pub stats: ScanStats,
    pub state: ScanState,
    pub errors: Vec<ScanIssue>,
    pub exclusions: Vec<ScanIssue>,
    pub worker_stats: Vec<WorkerStats>,
    pub finalization_us: u64,
    pub descriptor_limit: Option<usize>,
    pub descriptor_peak: Option<usize>,
    pub enumeration_tail: Option<TailStats>,
}
impl Snapshot {
    pub fn worker_directories(&self) -> u64 {
        self.worker_stats.iter().map(|s| s.directories).sum()
    }
    pub fn clone_attribution(&self, id: u32) -> Option<(bool, u64)> {
        tree::lookup(&self.tree.clone_attributions, id).copied()
    }
    pub fn incremental_accounting_eligible(&self) -> bool {
        self.accounting.verified_nonshared() && self.tree.clones.is_empty()
    }
    pub fn empty(root: PathBuf) -> Self {
        Self {
            accounting: AccountingInfo::default(),
            root,
            tree: tree::Tree::default(),
            stats: ScanStats::default(),
            state: ScanState::Running,
            errors: vec![],
            exclusions: vec![],
            worker_stats: vec![],
            finalization_us: 0,
            descriptor_limit: None,
            descriptor_peak: None,
            enumeration_tail: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Progress {
    pub entries: u64,
    pub directories: u64,
    pub errors: u64,
    #[serde(default)]
    pub excluded: u64,
    pub elapsed_ms: u64,
}
impl Progress {
    pub(crate) fn of(stats: &ScanStats) -> Self {
        Self {
            entries: stats.entries,
            directories: stats.directories,
            errors: stats.errors,
            excluded: stats.excluded,
            elapsed_ms: stats.elapsed_ms,
        }
    }
}
pub type ScanError = std::io::Error;
/// IDs with the high bit set are reserved for synthetic map aggregation nodes.
pub const MAX_NODES: usize = 0x8000_0000;
/// A validated scan authority. Owns the original directory independently of its path namespace.
#[derive(Debug)]
pub struct ScanRoot {
    path: PathBuf,
    expected: Identity,
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
}
impl ScanRoot {
    /// Validate a previously registered identity without resolving or canonicalizing another target.
    pub fn open(path: PathBuf, expected: Identity) -> std::io::Result<Self> {
        if expected.kind != NodeKind::Directory {
            return Err(std::io::Error::other("scan root must be a directory"));
        }
        #[cfg(unix)]
        {
            let fd = descriptor::open(&path, &expected)?;
            Ok(Self { path, expected, fd })
        }
        #[cfg(not(unix))]
        {
            let _ = (path, expected);
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "authorized descriptor scanning is not supported on this platform",
            ))
        }
    }
    pub fn reopen(&self) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            let fd = descriptor::reopen(&self.fd, &self.expected)?;
            Ok(Self {
                path: self.path.clone(),
                expected: self.expected.clone(),
                fd,
            })
        }
        #[cfg(not(unix))]
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "descriptor reopen unsupported",
            ))
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn identity(&self) -> &Identity {
        &self.expected
    }
}
#[cfg(unix)]
impl std::os::fd::AsFd for ScanRoot {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        std::os::fd::AsFd::as_fd(&self.fd)
    }
}
fn kind(m: &fs::Metadata) -> NodeKind {
    if m.is_dir() {
        NodeKind::Directory
    } else if m.is_file() {
        NodeKind::File
    } else if m.file_type().is_symlink() {
        NodeKind::Symlink
    } else {
        NodeKind::Other
    }
}
pub fn platform_identity(path: &Path) -> std::io::Result<Identity> {
    let m = fs::symlink_metadata(path)?;
    Ok(identity(&m))
}
fn identity(m: &fs::Metadata) -> Identity {
    #[cfg(unix)]
    {
        Identity {
            device: m.dev(),
            inode: m.ino(),
            kind: kind(m),
        }
    }
    #[cfg(not(unix))]
    {
        Identity {
            device: 0,
            inode: 0,
            kind: kind(m),
        }
    }
}
mod entry_path;
use entry_path::EntryPath;
#[derive(Debug)]
pub(crate) struct Entry {
    pub clone_checked: bool,
    // Absolute metadata path, or a validated single basename from bulk enumeration.
    // Relative names are scoped to the held listing directory before path policy/errors.
    pub path: EntryPath,
    pub device: u64,
    pub inode: u64,
    pub kind: NodeKind,
    pub length: u64,
    pub allocated: u64,
    pub hard_link: bool,
}
impl Entry {
    fn path_into(&self, parent: &Path, scratch: &mut PathBuf) {
        if self.path.is_absolute() {
            scratch.clear();
            scratch.push(&self.path);
            return;
        }
        #[cfg(unix)]
        tree::join_native_component_into(parent, self.path.as_os_str(), scratch)
            .expect("path length");
        #[cfg(not(unix))]
        {
            scratch.clear();
            scratch.push(parent);
            scratch.push(&self.path);
        }
    }
    fn identity(&self) -> Identity {
        Identity {
            device: self.device,
            inode: self.inode,
            kind: self.kind,
        }
    }
}
/// Scanner-only metadata, with implicit display labels derived from native bytes.
/// Legacy wire Nodes keep their independent owned labels for compatibility.
struct ScannedNode {
    id: u32,
    path: EntryPath,
    metadata: NodeMetadata,
}
#[cfg(not(unix))]
fn entry(path: PathBuf) -> std::io::Result<Entry> {
    let m = fs::symlink_metadata(&path)?;
    let id = identity(&m);
    Ok(Entry {
        clone_checked: false,
        path: path.into(),
        device: id.device,
        inode: id.inode,
        kind: id.kind,
        hard_link: false,
        length: if m.is_dir() { 0 } else { m.len() },
        allocated: if m.is_dir() { 0 } else { m.len() },
    })
}
// Listing errors need their known child path: an io::Result<Entry> alone loses
// it on failure. Keep the native cause intact and allocate context only on Err.
#[derive(Debug)]
struct EntryMetadataError {
    path: PathBuf,
    error: std::io::Error,
}
impl std::fmt::Display for EntryMetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for EntryMetadataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
fn entry_error_path(error: &std::io::Error) -> Option<&Path> {
    error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<EntryMetadataError>())
        .map(|context| context.path.as_path())
}
fn native_entry_error(error: &std::io::Error) -> &std::io::Error {
    error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<EntryMetadataError>())
        .map_or(error, |context| &context.error)
}

fn entry_result(
    result: std::io::Result<Entry>,
    path: impl FnOnce() -> PathBuf,
) -> std::io::Result<Entry> {
    result.map_err(|error| {
        std::io::Error::new(
            error.kind(),
            EntryMetadataError {
                path: path(),
                error,
            },
        )
    })
}

#[cfg(not(unix))]
fn portable(path: &Path, cancel: &AtomicBool) -> std::io::Result<Vec<std::io::Result<Entry>>> {
    Ok(fs::read_dir(path)?
        .take_while(|_| !cancel.load(Ordering::Relaxed))
        .map(|e| {
            e.and_then(|e| {
                let path = e.path();
                entry_result(entry(path.clone()), || path)
            })
        })
        .collect())
}
#[cfg(any(test, not(unix)))]
fn enumerate(
    path: &Path,
    backend: Backend,
    cancel: &AtomicBool,
    expected: &Identity,
) -> std::io::Result<Vec<std::io::Result<Entry>>> {
    #[cfg(unix)]
    {
        let fd = descriptor::open(path, expected)?;
        enumerate_opened(path, backend, cancel, &fd)
    }
    #[cfg(not(unix))]
    {
        let _ = (backend, expected);
        portable(path, cancel)
    }
}
#[cfg(all(unix, test))]
fn enumerate_opened(
    path: &Path,
    backend: Backend,
    cancel: &AtomicBool,
    fd: &std::os::fd::OwnedFd,
) -> std::io::Result<Vec<std::io::Result<Entry>>> {
    enumerate_opened_with_accounting(
        path,
        backend,
        cancel,
        fd,
        &descriptor::identity_fd(fd)?,
        &mut Vec::new(),
        &mut std::collections::VecDeque::new(),
        &mut Vec::new(),
    )
}
#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn enumerate_opened_with_accounting(
    path: &Path,
    backend: Backend,
    cancel: &AtomicBool,
    fd: &std::os::fd::OwnedFd,
    expected: &Identity,
    accounting_warnings: &mut Vec<String>,
    clone_observations: &mut std::collections::VecDeque<(usize, CloneMetadata)>,
    bulk_buffer: &mut Vec<u8>,
) -> std::io::Result<Vec<std::io::Result<Entry>>> {
    let inherited_warnings = accounting_warnings.len();
    retry::interrupted(Some(cancel), |attempt| {
        let fresh = if attempt == 0 {
            None
        } else {
            Some(descriptor::reopen(fd, expected)?)
        };
        // A failed attempt's warnings must not outlive a successful retry.
        accounting_warnings.truncate(inherited_warnings);
        clone_observations.clear();
        enumerate_once(
            path,
            backend,
            cancel,
            fresh.as_ref().unwrap_or(fd),
            accounting_warnings,
            clone_observations,
            bulk_buffer,
        )
        .and_then(retry::complete_listing)
    })
}
#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn enumerate_once(
    path: &Path,
    backend: Backend,
    cancel: &AtomicBool,
    fd: &std::os::fd::OwnedFd,
    accounting_warnings: &mut Vec<String>,
    clone_observations: &mut std::collections::VecDeque<(usize, CloneMetadata)>,
    bulk_buffer: &mut Vec<u8>,
) -> std::io::Result<Vec<std::io::Result<Entry>>> {
    if backend != Backend::Portable {
        #[cfg(target_os = "macos")]
        {
            match bulk::enumerate_with_accounting(
                path,
                cancel,
                fd,
                accounting_warnings,
                clone_observations,
                bulk_buffer,
            ) {
                Ok(v) => return Ok(v),
                Err(e) if backend == Backend::Bulk => return Err(e),
                Err(e) if falls_back_to_readdir(&e) => {}
                Err(e) => return Err(e),
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (accounting_warnings, clone_observations, bulk_buffer);
    descriptor::enumerate(fd, path, cancel)
}
/// The volume cannot do bulk listing, or returned a record without an attribute we asked for (some third-party
/// filesystems omit one per entry), so the portable listing takes over. It rewinds and lists the folder again.
// shortcut: a listing that already holds a failed entry (one that vanished or was refused) keeps the malformed
// record as an issue and is not relisted (`retry::terminal_listing`). Upgrade if such volumes show partial folders.
#[cfg(target_os = "macos")]
fn falls_back_to_readdir(error: &std::io::Error) -> bool {
    bulk::is_unsupported(error)
        || error.raw_os_error() == Some(libc::ENOSYS)
        || error.kind() == std::io::ErrorKind::InvalidData
}
/// Reopens a directory by walking recorded child names down from the held root. A renamed root
/// or ancestor cannot hide the subtree, which an absolute-path walk from `/` would.
#[cfg(unix)]
fn open_from_root(
    store: &RwLock<Snapshot>,
    id: u32,
    root: &std::os::fd::OwnedFd,
    gate: &descriptor::OpenGate,
) -> std::io::Result<std::os::fd::OwnedFd> {
    let (names, root_identity) = descriptor::path_names(&store.read().unwrap(), id)?;
    descriptor::open_names(root, &root_identity, &names, Some(gate))
}
struct Work {
    id: u32,
    /// The listing that queued this folder came from the saved cache, not from the disk.
    parent_reused: bool,
    #[cfg(unix)]
    parent: Option<Arc<descriptor::Lease>>,
    #[cfg(unix)]
    name: Option<std::io::Result<descriptor::ChildName>>,
}

// Full scans retain their original LIFO queue. Incremental scans can bring
// expensive dirty branches forward while cheap cached validation runs alongside.
#[derive(Default)]
struct WorkQueue {
    neutral: VecDeque<Work>,
    urgent: BinaryHeap<PrioritizedWork>,
    sequence: u64,
}
struct PrioritizedWork {
    cost: u64,
    sequence: u64,
    work: Work,
}
impl PartialEq for PrioritizedWork {
    fn eq(&self, other: &Self) -> bool {
        (self.cost, self.sequence) == (other.cost, other.sequence)
    }
}
impl Eq for PrioritizedWork {}
impl PartialOrd for PrioritizedWork {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PrioritizedWork {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.cost, self.sequence).cmp(&(other.cost, other.sequence))
    }
}
impl WorkQueue {
    fn clear(&mut self) {
        self.neutral.clear();
        self.urgent.clear();
    }
    fn push(&mut self, work: Work, cost: u64) {
        if cost == 0 {
            self.neutral.push_back(work);
        } else {
            self.sequence = self.sequence.wrapping_add(1);
            self.urgent.push(PrioritizedWork {
                cost,
                sequence: self.sequence,
                work,
            });
        }
    }
    fn pop(&mut self) -> Option<Work> {
        self.urgent
            .pop()
            .map(|item| item.work)
            .or_else(|| self.neutral.pop_back())
    }
}
type SharedQueue = (Mutex<(WorkQueue, usize)>, Condvar);
/// Blocks for the next directory; `None` once cancelled or nothing is queued or in flight.
fn next_work(queue: &SharedQueue, cancel: &AtomicBool) -> Option<Work> {
    let (lock, cv) = queue;
    let mut q = lock.lock().unwrap();
    loop {
        if cancel.load(Ordering::Relaxed) || q.1 == 0 {
            return None;
        }
        if let Some(work) = q.0.pop() {
            return Some(work);
        }
        q = cv.wait(q).unwrap();
    }
}
/// A panicking worker never settles its in-flight count, so idle peers would wait
/// forever and the thread scope could not join to re-raise the panic.
struct PanicWake<'a> {
    queue: &'a SharedQueue,
    cancel: &'a AtomicBool,
}
impl Drop for PanicWake<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.cancel.store(true, Ordering::Relaxed);
            // Notifying under the lock cannot slip between a waiter's check and its wait.
            let _q = self.queue.0.lock().unwrap_or_else(|e| e.into_inner());
            self.queue.1.notify_all();
        }
    }
}
fn traversal_allowed(
    path: &Path,
    root: &Path,
    entry_device: u64,
    root_device: u64,
    data_device: Option<u64>,
) -> bool {
    if root == Path::new("/") {
        !path.starts_with("/Volumes")
            && (entry_device == root_device || Some(entry_device) == data_device)
    } else {
        entry_device == root_device
    }
}
/// Child metadata paths are complete values, so reserve their final length before pushing.
/// `PathBuf::join` first clones the parent and may double its allocation on the child append.
#[cfg(unix)]
fn child_path(parent: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let mut path = PathBuf::new();
    tree::join_native_component_into(parent, name, &mut path).expect("path length");
    path
}
fn scanned_node(e: Entry, id: u32, parent: Option<u32>) -> ScannedNode {
    let identity = e.identity();
    let is_directory = e.kind == NodeKind::Directory;
    ScannedNode {
        id,
        path: e.path,
        metadata: NodeMetadata {
            parent,
            identity,
            allocated_size: e.allocated,
            length: e.length,
            own_allocated_size: e.allocated,
            is_directory,
            hard_link: e.hard_link,
            excluded: false,
        },
    }
}
/// A fixture the host cannot build skips locally, but CI must fail rather than go green unasserted.
#[cfg(all(test, target_os = "macos"))]
fn skip_unsupported_fixture(reason: &str) {
    assert!(
        std::env::var_os("CI").is_none(),
        "unsupported fixture on CI: {reason}"
    );
    eprintln!("skipping: {reason}");
}
#[cfg(test)]
fn node(e: Entry, id: u32, parent: Option<u32>) -> Node {
    let identity = e.identity();
    Node {
        clone_metadata: None,
        id,
        parent,
        children: vec![],
        name: e
            .path
            .file_name()
            .unwrap_or(e.path.as_os_str())
            .to_string_lossy()
            .into(),
        path: e.path.to_path_buf(),
        is_directory: e.kind == NodeKind::Directory,
        hard_link: e.hard_link,
        excluded: false,
        identity,
        allocated_size: e.allocated,
        length: e.length,
        own_allocated_size: e.allocated,
    }
}
fn issue(s: &mut Snapshot, path: PathBuf, error: impl ToString) {
    s.stats.errors += 1;
    s.stats.incomplete = true;
    if s.errors.len() < 10000 {
        s.errors.push(ScanIssue {
            path,
            message: error.to_string(),
            error_kind: None,
            raw_os_error: None,
        });
    }
}
fn exclusion(s: &mut Snapshot, path: PathBuf) {
    s.stats.excluded += 1;
    if s.exclusions.len() < 10000 {
        s.exclusions.push(ScanIssue {
            path,
            message: "Outside scan scope; mounted volumes are scanned separately.".into(),
            error_kind: None,
            raw_os_error: None,
        });
    }
}
fn record_node_scope(s: &mut Snapshot, path: &Path, excluded: &mut bool, permitted: bool) {
    *excluded = !permitted;
    if *excluded {
        exclusion(s, path.to_path_buf());
    }
}
fn io_issue(s: &mut Snapshot, path: PathBuf, error: std::io::Error) {
    let path = entry_error_path(&error).map_or(path, Path::to_path_buf);
    let error = native_entry_error(&error);
    if restriction::metadata(&path, error.kind(), error.raw_os_error()) {
        s.stats.restricted_system_issues += 1;
    }
    let kind = format!("{:?}", error.kind());
    let raw = error.raw_os_error();
    let previous = s.errors.len();
    issue(s, path, error);
    if s.errors.len() > previous {
        let last = s.errors.last_mut().unwrap();
        last.error_kind = Some(kind);
        last.raw_os_error = raw;
    }
}
/// An entry that vanished (or was replaced by a file) between listing and stat/open is simply
/// gone, not a failure: its deletion is a journal event, and the next rescan re-reads its parent.
fn vanished(error: &std::io::Error) -> bool {
    matches!(
        native_entry_error(error).raw_os_error(),
        Some(libc::ENOENT | libc::ENOTDIR)
    )
}
/// An evicted (dataless) iCloud folder: workers disable materialization, so listing it fails with
/// EDEADLK instead of downloading. It holds no bytes on disk, so an empty listing is complete.
pub(crate) fn dataless(error: &std::io::Error) -> bool {
    native_entry_error(error).raw_os_error() == Some(libc::EDEADLK)
}
#[cfg(unix)]
fn replaced(error: &std::io::Error) -> bool {
    descriptor::is_replaced(error)
}
#[cfg(not(unix))]
fn replaced(_: &std::io::Error) -> bool {
    false
}
pub fn scan(
    root: PathBuf,
    options: ScanOptions,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(Progress) + Send + Sync,
) -> Result<Snapshot, ScanError> {
    let store = Arc::new(RwLock::new(Snapshot::empty(root.clone())));
    scan_live(root, options, cancel, store.clone(), progress)?;
    Ok(Arc::try_unwrap(store)
        .expect("scanner retained snapshot store")
        .into_inner()
        .unwrap())
}
pub fn scan_live(
    root: PathBuf,
    options: ScanOptions,
    cancel: Arc<AtomicBool>,
    store: Arc<RwLock<Snapshot>>,
    progress: impl Fn(Progress) + Send + Sync,
) -> Result<(), ScanError> {
    // Resolve the explicit input once, retaining its pre-resolution identity through anchoring.
    let expected = platform_identity(&root)?;
    if expected.kind != NodeKind::Directory {
        return Err(std::io::Error::other(
            "scan root must be a directory, not a symbolic link",
        ));
    }
    let resolved = fs::canonicalize(&root)?;
    let authority = ScanRoot::open(resolved, expected)?;
    scan_live_authorized(authority, options, cancel, store, progress)
}
/// Scan an already authorized, held directory; never resolve or restat the initial root path.
pub fn scan_live_authorized(
    root: ScanRoot,
    options: ScanOptions,
    cancel: Arc<AtomicBool>,
    store: Arc<RwLock<Snapshot>>,
    progress: impl Fn(Progress) + Send + Sync,
) -> Result<(), ScanError> {
    scan_live_internal_reuse(root, options, cancel, store, progress, None)
}
#[allow(clippy::too_many_arguments)]
fn scan_live_internal_reuse(
    root: ScanRoot,
    options: ScanOptions,
    cancel: Arc<AtomicBool>,
    store: Arc<RwLock<Snapshot>>,
    progress: impl Fn(Progress) + Send + Sync,
    reuse: Option<&Mutex<incremental::Reuse>>,
) -> Result<(), ScanError> {
    let ScanRoot {
        path: root,
        expected,
        #[cfg(unix)]
        fd,
    } = root;
    let initial = Entry {
        clone_checked: false,
        path: root.clone().into(),
        device: expected.device,
        inode: expected.inode,
        kind: expected.kind,
        length: 0,
        allocated: 0,
        hard_link: false,
    };
    let device = initial.device;
    let data_device = if root == Path::new("/") {
        platform_identity(Path::new("/System/Volumes/Data"))
            .ok()
            .map(|i| i.device)
    } else {
        None
    };
    #[cfg(unix)]
    let budget = descriptor::Budget::with_limit(
        options
            .descriptor_limit
            .unwrap_or_else(descriptor::Budget::default_limit),
    );
    #[cfg(unix)]
    let root_parent = budget.retain(fd).parent();
    let firmlinks = firmlinks::Policy::load(&root);
    #[cfg(unix)]
    let clone_capabilities = Mutex::new(HashMap::new());
    let allocated_seen = Mutex::new(HashSet::new());
    let visited = Mutex::new(HashSet::from([(initial.device, initial.inode)]));
    {
        let mut s = store.write().unwrap();
        *s = Snapshot::empty(root.clone());
        s.accounting.model = AccountingModel::FullClonesV1;
        s.accounting.checks_complete = true;
        let n = scanned_node(initial, 0, None);
        let inserted = s.append_scanned_node(&n);
        drop(n);
        inserted.map_err(std::io::Error::other)?;
        s.stats.entries = 1;
        s.stats.directories = 1;
        #[cfg(unix)]
        {
            s.descriptor_limit = Some(budget.limit());
        }
        s.worker_stats = (1..=options.workers.clamp(1, MAX_WORKERS))
            .map(|id| WorkerStats {
                id,
                ..WorkerStats::default()
            })
            .collect();
    }
    let started = Instant::now();
    let priorities = reuse.map(|r| r.lock().unwrap().priorities.clone());
    let mut ready = WorkQueue::default();
    ready.push(
        Work {
            id: 0,
            parent_reused: false,
            #[cfg(unix)]
            parent: root_parent.clone(),
            #[cfg(unix)]
            name: None,
        },
        0,
    );
    let queue = Arc::new((Mutex::new((ready, 1usize)), Condvar::new()));
    // Milliseconds after `started` of the last progress report; the worker that swaps it reports.
    let tick = AtomicU64::new(0);
    // Highest `entries` delivered so far: a reporter that raced past the throttle with a staler
    // snapshot must not deliver a lower count after a higher one.
    let delivered = AtomicU64::new(0);
    let tail = Mutex::new(TailStats::default());
    #[cfg(unix)]
    let open_gate = descriptor::OpenGate::new(
        DIRECTORY_OPEN_PERMITS,
        options.workers.clamp(1, MAX_WORKERS),
    );
    #[cfg(unix)]
    let root_fd = &root_parent.as_ref().expect("retained root").fd;
    std::thread::scope(|scope| {
        for worker_id in 1..=options.workers.clamp(1, MAX_WORKERS) {
            let queue = queue.clone();
            let store = store.clone();
            let cancel = cancel.clone();
            let progress = &progress;
            let tick = &tick;
            let delivered = &delivered;
            let tail = &tail;
            let root = &root;
            let visited = &visited;
            let allocated_seen = &allocated_seen;
            #[cfg(unix)]
            let clone_capabilities = &clone_capabilities;
            let firmlinks = &firmlinks;
            let priorities = priorities.as_deref();
            #[cfg(unix)]
            let budget = &budget;
            #[cfg(unix)]
            let open_gate = &open_gate;
            scope.spawn(move || {
                let _wake = PanicWake {
                    queue: &queue,
                    cancel: &cancel,
                };
                let qos_error = diagnostics::apply_qos(options.qos);
                diagnostics::disable_dataless_materialization();
                let mut telemetry = diagnostics::Tracker::new(worker_id);
                telemetry.stats.qos_error = qos_error;
                #[cfg(unix)]
                let mut bulk_buffer = Vec::new();
                let mut slow: Vec<(u64, u64, u64, usize)> = Vec::new();
                let mut last_wait_us;
                loop {
                    let waiting = Instant::now();
                    let work = next_work(&queue, &cancel);
                    last_wait_us = diagnostics::micros(waiting.elapsed());
                    telemetry.stats.queue_wait_us =
                        telemetry.stats.queue_wait_us.saturating_add(last_wait_us);
                    let Some(work) = work else {
                        break;
                    };
                    let busy = Instant::now();
                    let entries_at_start = telemetry.stats.entries;
                    telemetry.stats.directories += 1;
                    let id = work.id;
                    let store_wait = Instant::now();
                    let (path, expected) = {
                        let s = store.read().unwrap();
                        telemetry.stats.store_wait_us = telemetry
                            .stats
                            .store_wait_us
                            .saturating_add(diagnostics::micros(store_wait.elapsed()));
                        (
                            s.path(id).expect("scanner topology"),
                            s.node(id).unwrap().identity,
                        )
                    };
                    let metadata = Instant::now();
                    #[cfg(unix)]
                    let cached = reuse
                        .and_then(|r| r.lock().unwrap().take_listing(&expected))
                        .and_then(|listing| listing.verified(&path, &expected));
                    #[cfg(unix)]
                    let retain = |fd| {
                        if cached
                            .as_ref()
                            .is_some_and(|listing| !listing.has_directories)
                        {
                            descriptor::Opened::Temporary(fd)
                        } else {
                            budget.retain(fd)
                        }
                    };
                    #[cfg(unix)]
                    let opened = match (&work.parent, &work.name) {
                        (Some(parent), Some(Ok(name))) => {
                            telemetry.stats.relative_opens += 1;
                            descriptor::open_child_name_gated(
                                &parent.fd, name, &expected, open_gate,
                            )
                            .map(retain)
                        }
                        (Some(parent), None) if id == 0 => {
                            Ok(descriptor::Opened::Retained(parent.clone()))
                        }
                        (_, Some(Err(error))) => {
                            Err(std::io::Error::new(error.kind(), error.to_string()))
                        }
                        (_, None) if id != 0 => Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "missing child name",
                        )),
                        _ => {
                            telemetry.stats.absolute_reopens += 1;
                            open_from_root(&store, id, root_fd, open_gate).map(retain)
                        }
                    };
                    #[cfg(unix)]
                    let parent_lease = opened.as_ref().ok().and_then(descriptor::Opened::parent);
                    #[cfg(unix)]
                    let mut clone_listing_warnings = Vec::new();
                    #[cfg(unix)]
                    let mut clone_observations = std::collections::VecDeque::new();
                    // A cached listing means the journal reported nothing here, so a vanish on
                    // open is a missed event (stale cache), not a benign race.
                    #[cfg(unix)]
                    let fresh = cached.is_none();
                    #[cfg(not(unix))]
                    let fresh = true;
                    #[cfg(unix)]
                    let listing = match &opened {
                        Ok(opened) => match cached {
                            Some(listing) if !cancel.load(Ordering::Relaxed) => {
                                let mut entries = listing.materialize();
                                if let Some(reuse) = reuse {
                                    reuse.lock().unwrap().mark_reused(entries.len());
                                }
                                clones::vouch_reused(
                                    &mut entries,
                                    opened.fd(),
                                    clone_capabilities,
                                    &cancel,
                                    &mut clone_observations,
                                    &mut bulk_buffer,
                                    &mut clone_listing_warnings,
                                );
                                Ok(entries)
                            }
                            _ => enumerate_opened_with_accounting(
                                &path,
                                options.backend,
                                &cancel,
                                opened.fd(),
                                &expected,
                                &mut clone_listing_warnings,
                                &mut clone_observations,
                                &mut bulk_buffer,
                            ),
                        },
                        Err(e) => Err(e.raw_os_error().map_or_else(
                            || std::io::Error::new(e.kind(), e.to_string()),
                            std::io::Error::from_raw_os_error,
                        )),
                    };
                    #[cfg(not(unix))]
                    let listing = enumerate(&path, options.backend, &cancel, &expected);
                    // Incremental root comparison retains its complete-path contract.
                    // Normal scans keep bulk basenames compact until publication.
                    let mut listing = listing;
                    if id == 0 {
                        if let Some(reuse) = reuse {
                            if let Ok(entries) = &mut listing {
                                for e in entries.iter_mut().filter_map(|e| e.as_mut().ok()) {
                                    if !e.path.is_absolute() {
                                        e.path = path.join(&e.path).into();
                                    }
                                }
                            }
                            reuse.lock().unwrap().observe_root(&listing);
                        }
                    }
                    telemetry.stats.metadata_us = telemetry
                        .stats
                        .metadata_us
                        .saturating_add(diagnostics::micros(metadata.elapsed()));
                    #[cfg(unix)]
                    let mut clone_probes = match (&listing, &opened) {
                        (Ok(entries), Ok(directory)) => Some(clones::prepare(
                            entries,
                            directory.fd(),
                            clone_capabilities,
                            &cancel,
                            clone_observations,
                            // Any listing warning means a rich read failed and files were
                            // marked checked without a reply, so they are no device evidence.
                            clone_listing_warnings.is_empty(),
                        )),
                        _ => None,
                    };
                    let mut pending = vec![];
                    {
                        let store_wait = Instant::now();
                        let mut s = store.write().unwrap();
                        telemetry.stats.store_wait_us = telemetry
                            .stats
                            .store_wait_us
                            .saturating_add(diagnostics::micros(store_wait.elapsed()));
                        #[cfg(unix)]
                        accounting_listing_warnings(&mut s, clone_listing_warnings);
                        #[cfg(unix)]
                        if let Some(batch) = &clone_probes {
                            s.accounting.checks_complete &= !batch.cancelled;
                            s.accounting.metadata_queries += batch.queries;
                            s.accounting.metadata_us =
                                s.accounting.metadata_us.saturating_add(batch.micros);
                        }
                        let commit = Instant::now();
                        let entries_before = s.stats.entries;
                        let errors_before = s.stats.errors;
                        let mut allocated_seen = allocated_seen.lock().unwrap();
                        let mut visited = visited.lock().unwrap();
                        // Only this listing can append while the store is held, so its
                        // published children form a dense range, even after filtering.
                        let first_child = s.node_count();
                        let mut added_allocated = 0u64;
                        match listing {
                            Err(e) if dataless(&e) => {}
                            // The root is held by descriptor; only a child directory can vanish
                            // or be replaced by a new one (not part of what was enumerated).
                            Err(e) if id != 0 && (vanished(&e) || replaced(&e)) => {
                                // Not a race when the cache said the folder was there, and a cached
                                // parent listed it: rescan fully. A parent read from disk can lose it.
                                if let Some(reuse) = reuse.filter(|_| !fresh && work.parent_reused)
                                {
                                    reuse.lock().unwrap().stale = true;
                                }
                            }
                            // `retry::interrupted` reports cancellation as EINTR; that is not a scan error.
                            Err(e)
                                if e.kind() == std::io::ErrorKind::Interrupted
                                    && cancel.load(Ordering::Relaxed) => {}
                            Err(e) => io_issue(&mut s, path, e),
                            Ok(entries) => {
                                let check_firmlinks = firmlinks.may_have_alias_children(&path);
                                let mut entry_path = PathBuf::new();
                                for (entry_index, item) in entries.into_iter().enumerate() {
                                    #[cfg(unix)]
                                    let clone_probe = clone_probes
                                        .as_mut()
                                        .and_then(|batch| batch.take_at(entry_index));
                                    #[cfg(not(unix))]
                                    let _ = entry_index;
                                    if cancel.load(Ordering::Relaxed) {
                                        break;
                                    }
                                    match item {
                                        Err(e) if vanished(&e) || dataless(&e) => {}
                                        Err(e) => io_issue(&mut s, path.clone(), e),
                                        Ok(e) => {
                                            // Only verified alias parents need full paths for file
                                            // filtering. Directories always need paths for traversal.
                                            if check_firmlinks || e.kind == NodeKind::Directory {
                                                e.path_into(&path, &mut entry_path);
                                            }
                                            if e.hard_link {
                                                if let Some(reuse) = reuse {
                                                    reuse.lock().unwrap().observe_entry(&e);
                                                }
                                            }
                                            if check_firmlinks
                                                && firmlinks.is_mirror(&entry_path, &e.identity())
                                            {
                                                continue;
                                            }
                                            let permitted = e.kind == NodeKind::Directory
                                                && traversal_allowed(
                                                    &entry_path,
                                                    root,
                                                    e.device,
                                                    device,
                                                    data_device,
                                                );
                                            if s.node_count() >= MAX_NODES {
                                                e.path_into(&path, &mut entry_path);
                                                issue(
                                                    &mut s,
                                                    entry_path.clone(),
                                                    "node capacity exceeded",
                                                );
                                                cancel.store(true, Ordering::Relaxed);
                                                break;
                                            }
                                            let nid = s.node_count() as u32;
                                            let mut n = scanned_node(e, nid, Some(id));
                                            if n.metadata.is_directory {
                                                record_node_scope(
                                                    &mut s,
                                                    &entry_path,
                                                    &mut n.metadata.excluded,
                                                    permitted,
                                                );
                                            }
                                            if n.metadata.hard_link
                                                && !allocated_seen.insert((
                                                    n.metadata.identity.device,
                                                    n.metadata.identity.inode,
                                                ))
                                            {
                                                n.metadata.allocated_size = 0;
                                            }
                                            s.stats.entries += 1;
                                            if n.metadata.is_directory {
                                                s.stats.directories += 1;
                                            }
                                            let a = n.metadata.allocated_size;
                                            let child_path = permitted.then(|| entry_path.clone());
                                            let inserted = s.append_scanned_node(&n);
                                            drop(n);
                                            inserted.expect("scanner compact insertion");
                                            #[cfg(unix)]
                                            if let Some(probe) = clone_probe {
                                                clones::commit(&mut s, nid, probe);
                                            }
                                            added_allocated = added_allocated.saturating_add(a);
                                            if permitted
                                                && visited.insert((
                                                    s.node(nid).unwrap().identity.device,
                                                    s.node(nid).unwrap().identity.inode,
                                                ))
                                            {
                                                let n = s.node(nid).unwrap();
                                                let cost = priorities.map_or(0, |hints| {
                                                    hints.cost(
                                                        child_path.as_deref().unwrap(),
                                                        &n.identity,
                                                    )
                                                });
                                                pending.push((
                                                    Work {
                                                        id: nid,
                                                        parent_reused: !fresh,
                                                        #[cfg(unix)]
                                                        parent: parent_lease.clone(),
                                                        #[cfg(unix)]
                                                        name: child_path
                                                            .as_deref()
                                                            .unwrap()
                                                            .file_name()
                                                            .map(descriptor::ChildName::new),
                                                    },
                                                    cost,
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        s.set_scanned_children(id, first_child)
                            .expect("scanner child batch");
                        // Allocated values already include hardlink attribution.
                        // Saturating sums can be combined once without exposing a
                        // partial batch.
                        s.stats.allocated_bytes =
                            s.stats.allocated_bytes.saturating_add(added_allocated);
                        let mut ancestor = Some(id);
                        while let Some(i) = ancestor {
                            ancestor = s.node(i).unwrap().parent;
                            s.update_node(i, |n| {
                                n.allocated_size = n.allocated_size.saturating_add(added_allocated);
                            })
                            .expect("scanner ancestor");
                        }
                        s.stats.elapsed_ms = started.elapsed().as_millis() as u64;
                        telemetry.stats.entries = telemetry
                            .stats
                            .entries
                            .saturating_add(s.stats.entries - entries_before);
                        telemetry.stats.errors = telemetry
                            .stats
                            .errors
                            .saturating_add(s.stats.errors - errors_before);
                        telemetry.stats.commit_us = telemetry
                            .stats
                            .commit_us
                            .saturating_add(diagnostics::micros(commit.elapsed()));
                        let row = &mut s.worker_stats[worker_id - 1];
                        telemetry.snapshot_into(row);
                        row.busy_us = row
                            .busy_us
                            .saturating_add(diagnostics::micros(busy.elapsed()));
                    }
                    {
                        let (lock, cv) = &*queue;
                        let mut q = lock.lock().unwrap();
                        q.1 = q.1 - 1 + pending.len();
                        for (work, cost) in pending {
                            q.0.push(work, cost);
                        }
                        cv.notify_all();
                    }
                    let reporting = Instant::now();
                    let mut progress_store_wait_us = 0u64;
                    let now = started.elapsed().as_millis() as u64;
                    let last = tick.load(Ordering::Relaxed);
                    if now.saturating_sub(last) >= 100
                        && tick
                            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                            .is_ok()
                    {
                        let event = {
                            let store_wait = Instant::now();
                            let s = store.read().unwrap();
                            progress_store_wait_us = diagnostics::micros(store_wait.elapsed());
                            telemetry.stats.store_wait_us = telemetry
                                .stats
                                .store_wait_us
                                .saturating_add(progress_store_wait_us);
                            Progress::of(&s.stats)
                        };
                        if delivered.fetch_max(event.entries, Ordering::Relaxed) <= event.entries {
                            progress(event);
                        }
                    }
                    telemetry.stats.progress_us = telemetry.stats.progress_us.saturating_add(
                        diagnostics::micros(reporting.elapsed())
                            .saturating_sub(progress_store_wait_us),
                    );
                    let listing_us = diagnostics::micros(busy.elapsed());
                    telemetry.stats.busy_us = telemetry.stats.busy_us.saturating_add(listing_us);
                    record_slow_listing(
                        &mut slow,
                        (
                            diagnostics::micros(busy.duration_since(started)),
                            listing_us,
                            telemetry.stats.entries - entries_at_start,
                            worker_id,
                        ),
                    );
                }
                {
                    let mut tail = tail.lock().unwrap();
                    tail.tail_idle_us = tail.tail_idle_us.saturating_add(last_wait_us);
                    for listing in slow {
                        record_slow_listing(&mut tail.slow_listings, listing);
                    }
                }
                // Publish idle/cancelled workers too; no callback or CPU query while waiting for this guard.
                let publication = Instant::now();
                let store_wait = Instant::now();
                let mut s = store.write().unwrap();
                telemetry.stats.store_wait_us = telemetry
                    .stats
                    .store_wait_us
                    .saturating_add(diagnostics::micros(store_wait.elapsed()));
                telemetry.stats.busy_us = telemetry
                    .stats
                    .busy_us
                    .saturating_add(diagnostics::micros(publication.elapsed()));
                telemetry.finish();
                s.worker_stats[worker_id - 1] = telemetry.stats;
            });
        }
    });
    queue.0.lock().unwrap().0.clear();
    let mut tail = tail.into_inner().unwrap();
    tail.walk_end_us = diagnostics::micros(started.elapsed());
    // Enumeration is joined: neither identity set participates in final clone
    // validation or stable owner election. Release their storage before those
    // phases allocate their own grouping buffers, including after cancellation.
    drop(visited);
    drop(allocated_seen);
    // Every entry is now in the tree with raw, hard-link-attributed sizes.
    // Publish that before clone validation, which only lowers shared sizes.
    if !cancel.load(Ordering::Relaxed) {
        let event = {
            let mut s = store.write().unwrap();
            s.stats.elapsed_ms = started.elapsed().as_millis() as u64;
            s.stats.walk_ms = s.stats.elapsed_ms.max(1);
            Progress::of(&s.stats)
        };
        progress(event);
    }
    let finalizing = Instant::now();
    #[cfg(unix)]
    let validation = {
        let s = store.read().unwrap();
        clones::validate(&s, root_fd, &cancel, options.qos)
    };
    #[cfg(not(unix))]
    let validation = clones::validate_unverified(&store.read().unwrap(), &cancel);
    let mut s = store.write().unwrap();
    validation.apply(&mut s);
    #[cfg(unix)]
    {
        s.descriptor_peak = Some(budget.peak());
    }
    aggregate(&mut s);
    s.state = if cancel.load(Ordering::Relaxed) {
        s.stats.incomplete = true;
        ScanState::Cancelled
    } else {
        ScanState::Completed
    };
    s.stats.elapsed_ms = started.elapsed().as_millis() as u64;
    let event = Progress::of(&s.stats);
    s.finalization_us = diagnostics::micros(finalizing.elapsed());
    s.enumeration_tail = Some(tail);
    drop(s);
    progress(event);
    Ok(())
}
/// Keep the slowest few listings, by duration, for tail diagnostics.
fn record_slow_listing(slow: &mut Vec<(u64, u64, u64, usize)>, listing: (u64, u64, u64, usize)) {
    const KEEP: usize = 8;
    if slow.len() < KEEP {
        slow.push(listing);
    } else if let Some(min) = slow.iter_mut().min_by_key(|l| l.1) {
        if listing.1 > min.1 {
            *min = listing;
        }
    }
}
#[cfg(unix)]
fn accounting_listing_warnings(s: &mut Snapshot, warnings: Vec<String>) {
    for warning in warnings {
        s.accounting.warn(warning);
    }
}
pub fn aggregate(s: &mut Snapshot) {
    let plan = match clones::internal_allocation_plan(s) {
        Ok(plan) => plan,
        Err(error) => {
            s.accounting.warn(error);
            s.tree.clones.clear();
            clones::internal_allocation_plan(s).expect("scanner hardlinks")
        }
    };
    s.accounting.deduction_bytes = plan.clone_deduction_bytes;
    plan.apply_to(s);
    s.tree.clone_attributions = plan.clone_attributions;
    s.aggregate_sizes_to_parents().unwrap();
    if let Some(root) = s.node(0) {
        s.stats.allocated_bytes = root.allocated_size;
    }
    s.tree.names.shrink_to_fit();
    s.tree.children.shrink_to_fit();
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn bulk_listing_falls_back_for_unsupported_and_malformed_records_only() {
        let falls = |code| super::falls_back_to_readdir(&std::io::Error::from_raw_os_error(code));
        for code in [libc::ENOTSUP, libc::EOPNOTSUPP, libc::EINVAL, libc::ENOSYS] {
            assert!(falls(code), "errno {code}");
        }
        for code in [libc::EIO, libc::EACCES] {
            assert!(!falls(code), "errno {code}");
        }
        // A record missing an attribute: listing the folder plainly beats losing all of it.
        assert!(super::falls_back_to_readdir(&std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "malformed getattrlistbulk record"
        )));
    }
    #[test]
    fn listing_entries_keep_clone_observations_sparse() {
        assert!(std::mem::size_of::<super::Entry>() <= 64);
        assert!(std::mem::size_of::<std::io::Result<super::Entry>>() <= 64);
    }
    #[test]
    fn compact_entry_preserves_full_identity_and_flags() {
        for device in [0, u64::MAX] {
            for inode in [0, u64::MAX] {
                for kind in [
                    NodeKind::File,
                    NodeKind::Directory,
                    NodeKind::Symlink,
                    NodeKind::Other,
                ] {
                    for hard_link in [false, true] {
                        for clone_checked in [false, true] {
                            let entry = Entry {
                                device,
                                inode,
                                kind,
                                hard_link,
                                clone_checked,
                                path: "/fixture".into(),
                                length: u64::MAX,
                                allocated: u64::MAX,
                            };
                            assert_eq!(
                                entry.identity(),
                                Identity {
                                    device,
                                    inode,
                                    kind
                                }
                            );
                            assert_eq!(
                                (entry.hard_link, entry.clone_checked),
                                (hard_link, clone_checked)
                            );
                            assert_eq!((entry.length, entry.allocated), (u64::MAX, u64::MAX));
                        }
                    }
                }
            }
        }
    }
    fn queued_work(id: u32) -> Work {
        Work {
            id,
            parent_reused: false,
            #[cfg(unix)]
            parent: None,
            #[cfg(unix)]
            name: None,
        }
    }
    #[test]
    fn panicking_worker_wakes_idle_workers_so_the_scope_reraises() {
        let queue: Arc<SharedQueue> =
            Arc::new((Mutex::new((WorkQueue::default(), 1)), Condvar::new()));
        let cancel = AtomicBool::new(false);
        let (done, finished) = std::sync::mpsc::channel();
        // Detached, so a regression times out instead of hanging the suite.
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(|| {
                std::thread::scope(|scope| {
                    for _ in 0..3 {
                        scope.spawn(|| assert!(next_work(&queue, &cancel).is_none()));
                    }
                    scope.spawn(|| {
                        let _wake = PanicWake {
                            queue: &queue,
                            cancel: &cancel,
                        };
                        // Let the idle workers park first so the wakeup is what frees them.
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        panic!("worker failed mid-directory");
                    });
                });
            });
            done.send(outcome.is_err()).unwrap();
        });
        assert_eq!(
            finished.recv_timeout(std::time::Duration::from_secs(10)),
            Ok(true)
        );
    }
    #[test]
    fn full_queue_keeps_original_lifo_order() {
        let mut queue = WorkQueue::default();
        for id in 0..5 {
            queue.push(queued_work(id), 0);
        }
        for id in (0..5).rev() {
            assert_eq!(queue.pop().unwrap().id, id);
        }
        assert!(queue.pop().is_none());
    }
    #[test]
    fn slow_listing_diagnostics_keep_the_longest_listings() {
        let mut slow = vec![];
        for duration in [5, 1, 9, 3, 7, 2, 8, 4, 6, 10, 0] {
            record_slow_listing(&mut slow, (0, duration, duration * 100, 1));
        }
        let mut durations: Vec<_> = slow.iter().map(|l| l.1).collect();
        durations.sort_unstable();
        assert_eq!(durations, vec![3, 4, 5, 6, 7, 8, 9, 10]);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn complete_raw_tree_is_published_before_clone_verification() {
        let dir = std::env::temp_dir().join(format!("walk-publish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/original"), vec![7u8; 256 * 1024]).unwrap();
        // APFS full clones share every data block; verification deducts one copy.
        for copy in ["a/clone", "a/b/clone"] {
            let status = std::process::Command::new("/bin/cp")
                .arg("-c")
                .arg(dir.join("a/original"))
                .arg(dir.join(copy))
                .status()
                .unwrap();
            assert!(status.success());
        }
        let store = Arc::new(RwLock::new(Snapshot::empty(dir.clone())));
        let provisional = Mutex::new(None);
        scan_live(
            dir.clone(),
            ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            store.clone(),
            |_| {
                let s = store.read().unwrap();
                if s.stats.walk_ms > 0 && provisional.lock().unwrap().is_none() {
                    *provisional.lock().unwrap() = Some((
                        s.state,
                        s.stats.entries,
                        s.stats.walk_ms,
                        s.node(0).unwrap().allocated_size,
                    ));
                }
            },
        )
        .unwrap();
        let s = store.read().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let (state, entries, walk_ms, raw_root) =
            provisional.into_inner().unwrap().expect("walk-end event");
        assert_eq!(state, ScanState::Running);
        assert_eq!(entries, s.stats.entries);
        assert_eq!(s.state, ScanState::Completed);
        assert!(walk_ms <= s.stats.elapsed_ms.max(1));
        let verified_root = s.node(0).unwrap().allocated_size;
        assert!(s.accounting.checks_complete);
        // Raw sizes count every clone copy; verification deducts the shared ones.
        assert!(s.accounting.shared);
        assert!(verified_root < raw_root);
    }
    #[test]
    fn cancelled_scan_publishes_no_walk_end() {
        let dir = std::env::temp_dir().join(format!("walk-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        let s = scan(
            dir.clone(),
            ScanOptions::default(),
            Arc::new(AtomicBool::new(true)),
            |_| {},
        )
        .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(s.state, ScanState::Cancelled);
        assert_eq!(s.stats.walk_ms, 0);
    }
    #[test]
    fn scan_reports_enumeration_tail() {
        let dir = std::env::temp_dir().join(format!("tail-diagnostics-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/file"), b"x").unwrap();
        let s = scan(
            dir.clone(),
            ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let tail = s.enumeration_tail.as_ref().expect("tail diagnostics");
        assert!(tail.walk_end_us > 0);
        assert!(!tail.slow_listings.is_empty() && tail.slow_listings.len() <= 8);
    }
    #[test]
    fn expensive_dirty_branches_precede_existing_cached_work() {
        let mut queue = WorkQueue::default();
        queue.push(queued_work(1), 0);
        queue.push(queued_work(2), 100);
        queue.push(queued_work(3), 10);
        queue.push(queued_work(4), 0);
        assert_eq!(queue.pop().unwrap().id, 2);
        // A newly discovered expensive descendant competes globally, even
        // against older urgent work and neutral jobs already in the queue.
        queue.push(queued_work(5), 90);
        for id in [5, 3, 4, 1] {
            assert_eq!(queue.pop().unwrap().id, id);
        }
        assert!(queue.pop().is_none());
    }
    #[cfg(unix)]
    #[test]
    fn bulk_basename_scopes_native_paths_without_retaining_parent_prefix() {
        use std::os::unix::ffi::OsStrExt;
        let parent = Path::new("/fixture/long-parent-prefix");
        let name = std::ffi::OsStr::from_bytes(b"native-\xff\nname");
        let mut e = Entry {
            clone_checked: false,
            path: EntryPath::from_native(name),
            device: 1,
            inode: 2,
            kind: NodeKind::File,
            length: 7,
            allocated: 4096,
            hard_link: false,
        };
        let mut scratch = PathBuf::with_capacity(1024);
        let allocation = scratch.as_os_str().as_bytes().as_ptr();
        e.path_into(parent, &mut scratch);
        assert_eq!(
            scratch.as_os_str().as_bytes(),
            parent.join(name).as_os_str().as_bytes()
        );
        assert_eq!(scratch.as_os_str().as_bytes().as_ptr(), allocation);
        assert_eq!(e.path.as_os_str(), name);
        // Publication retains the name and derives the full path from topology.
        let mut s = Snapshot::empty(parent.to_path_buf());
        let root = Entry {
            path: parent.to_path_buf().into(),
            kind: NodeKind::Directory,
            inode: 1,
            clone_checked: false,
            device: 1,
            length: 0,
            allocated: 0,
            hard_link: false,
        };
        s.append_scanned_node(&scanned_node(root, 0, None)).unwrap();
        // Absolute descriptor paths still bypass the contextual parent.
        e.path = parent.join(name).into();
        e.path_into(Path::new("/other"), &mut scratch);
        assert_eq!(scratch, parent.join(name));
        e.path = EntryPath::from_native(name);
        s.append_scanned_node(&scanned_node(e, 1, Some(0))).unwrap();
        assert_eq!(
            s.path(1).unwrap().as_os_str().as_bytes(),
            parent.join(name).as_os_str().as_bytes()
        );
    }
    #[cfg(unix)]
    #[test]
    fn child_path_keeps_native_join_semantics_without_append_growth() {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::OsStr::from_bytes(b"native-\xff\nname");
        for parent in ["", "/", "/fixture", "/fixture/", "/日本語"] {
            let parent = Path::new(parent);
            let path = child_path(parent, name);
            assert_eq!(path, parent.join(name));
            assert_eq!(
                path.as_os_str().as_bytes(),
                parent.join(name).as_os_str().as_bytes()
            );
            assert_eq!(path.capacity(), path.as_os_str().len());
        }
    }
    use super::*;
    fn fixture() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "hahakigami-{}-{}-{}",
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&p).unwrap();
        fs::canonicalize(p).unwrap()
    }
    #[cfg(unix)]
    #[test]
    fn listing_accounting_warnings_preserve_raw_coverage_including_empty_results() {
        for with_file in [false, true] {
            let p = fixture();
            if with_file {
                fs::write(p.join("file"), b"owned raw fixture").unwrap();
            }
            let mut s = scan(
                p.clone(),
                ScanOptions::default(),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let before = serde_json::to_value(&s.stats).unwrap();
            let raw: Vec<_> = s
                .iter_nodes()
                .map(|n| (n.id, n.length, n.own_allocated_size))
                .collect();
            accounting_listing_warnings(
                &mut s,
                vec!["malformed rich accounting; validated baseline retained".into()],
            );
            aggregate(&mut s);
            assert_eq!(serde_json::to_value(&s.stats).unwrap(), before);
            assert_eq!(
                s.iter_nodes()
                    .map(|n| (n.id, n.length, n.own_allocated_size))
                    .collect::<Vec<_>>(),
                raw
            );
            assert!(!s.accounting.checks_complete);
            assert!(!s.incremental_accounting_eligible());
            assert_eq!(s.accounting.warnings.len(), 1);
            s.accounting
                .warnings
                .resize(32, "earlier uncertainty".into());
            accounting_listing_warnings(&mut s, vec!["later uncertainty".into()]);
            assert_eq!(s.accounting.warnings.len(), 32);
            assert!(!s.accounting.checks_complete);
            fs::remove_dir_all(p).unwrap();
        }
    }
    #[test]
    fn tree_hardlinks_symlinks_and_bulk_agree() {
        let p = fixture();
        fs::create_dir(p.join("d")).unwrap();
        fs::write(p.join("d/a"), vec![1; 8192]).unwrap();
        fs::hard_link(p.join("d/a"), p.join("b")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&p, p.join("loop")).unwrap();
        let a = scan(
            p.clone(),
            ScanOptions {
                workers: 4,
                backend: Backend::Portable,
                qos: ScanQos::Automatic,
                descriptor_limit: None,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        assert_eq!(a.state, ScanState::Completed);
        #[cfg(unix)]
        assert_eq!(
            a.iter_nodes()
                .filter(|n| n.identity.kind == NodeKind::File)
                .map(|n| n.allocated_size)
                .sum::<u64>(),
            fs::metadata(p.join("b")).unwrap().blocks() * 512
        );
        assert!(a.node_count() < 10);
        #[cfg(target_os = "macos")]
        {
            let b = scan(
                p.clone(),
                ScanOptions {
                    workers: 2,
                    backend: Backend::Bulk,
                    qos: ScanQos::Automatic,
                    descriptor_limit: None,
                },
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            assert_eq!(a.node_count(), b.node_count());
            assert_eq!(a.stats.allocated_bytes, b.stats.allocated_bytes);
        }
        fs::remove_dir_all(p).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn resource_fork_lengths_match_across_backends() {
        let p = fixture();
        fs::write(p.join("both"), vec![1; 4096]).unwrap();
        fs::write(p.join("both/..namedfork/rsrc"), vec![2; 8192]).unwrap();
        fs::write(p.join("resource-only"), b"").unwrap();
        fs::write(p.join("resource-only/..namedfork/rsrc"), vec![3; 512]).unwrap();
        fs::write(p.join("data-only"), vec![4; 1024]).unwrap();
        fs::write(p.join("empty"), b"").unwrap();
        std::os::unix::fs::symlink("both", p.join("link")).unwrap();
        let index = |backend| {
            let s = scan(
                p.clone(),
                ScanOptions {
                    backend,
                    ..ScanOptions::default()
                },
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            assert_eq!(s.state, ScanState::Completed);
            assert_eq!(s.stats.errors, 0);
            for (name, length) in [
                ("both", 12288),
                ("resource-only", 512),
                ("data-only", 1024),
                ("empty", 0),
            ] {
                let n = s.iter_nodes().find(|n| n.native_name() == name).unwrap();
                assert_eq!(n.length, length, "{name}");
            }
            let nodes = s
                .iter_nodes()
                .map(|n| {
                    (
                        n.path().unwrap(),
                        (n.identity, n.length, n.own_allocated_size),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>();
            (s.stats.allocated_bytes, nodes)
        };
        assert_eq!(index(Backend::Bulk), index(Backend::Portable));
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn cancellation_retains_root() {
        let p = fixture();
        let s = scan(
            p.clone(),
            ScanOptions::default(),
            Arc::new(AtomicBool::new(true)),
            |_| {},
        )
        .unwrap();
        assert_eq!(s.state, ScanState::Cancelled);
        assert_eq!(s.node_count(), 1);
        assert!(s.stats.incomplete);
        fs::remove_dir_all(p).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn cancelling_mid_scan_reports_no_interrupted_errors() {
        let p = fixture();
        for i in 0..64 {
            fs::create_dir_all(p.join(format!("d{i}/a/b"))).unwrap();
        }
        for i in 0..200u64 {
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let canceller = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_micros(i * 10));
                flag.store(true, Ordering::Relaxed);
            });
            let s = scan(p.clone(), ScanOptions::default(), cancel, |_| {}).unwrap();
            canceller.join().unwrap();
            assert!(s.errors.is_empty(), "{:?}", s.errors);
        }
        fs::remove_dir_all(p).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn bulk_large_directory_matches_every_portable_entry() {
        let p = fixture();
        // Long names force many native batches even in this bounded fixture.
        for i in 0..3000 {
            fs::write(p.join(format!("{i:04}-{}", "long-name".repeat(10))), b"").unwrap();
        }
        fs::write(p.join("日本語\n🌿"), b"odd").unwrap();
        std::os::unix::fs::symlink(".", p.join("loop")).unwrap();
        fs::create_dir(p.join("nested")).unwrap();
        fs::create_dir(p.join("empty")).unwrap();
        let index = |backend| {
            let s = scan(
                p.clone(),
                ScanOptions {
                    workers: 8,
                    backend,
                    qos: ScanQos::Automatic,
                    descriptor_limit: None,
                },
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            assert_eq!(s.state, ScanState::Completed);
            assert_eq!(s.stats.errors, 0);
            assert_eq!(s.stats.entries, 3005);
            s.iter_nodes()
                .map(|n| {
                    (
                        n.path().unwrap(),
                        (n.identity, n.length, n.own_allocated_size, n.hard_link),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        assert_eq!(index(Backend::Bulk), index(Backend::Portable));
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn rejects_symlink_root() {
        let p = fixture();
        #[cfg(unix)]
        {
            let link = p.with_extension("link");
            std::os::unix::fs::symlink(&p, &link).unwrap();
            assert!(scan(
                link.clone(),
                ScanOptions::default(),
                Arc::new(AtomicBool::new(false)),
                |_| {}
            )
            .is_err());
            fs::remove_file(link).unwrap();
        }
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn unusual_names_and_worker_counts() {
        let p = fixture();
        for i in 0..30 {
            fs::create_dir(p.join(format!("d{i}"))).unwrap();
            fs::write(p.join(format!("d{i}/space\n雪")), b"abc").unwrap();
        }
        for workers in [1, 2, 4, 8, 12, MAX_WORKERS] {
            for backend in [Backend::Portable, Backend::Auto] {
                let s = scan(
                    p.clone(),
                    ScanOptions {
                        workers,
                        backend,
                        qos: ScanQos::Automatic,
                        descriptor_limit: None,
                    },
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .unwrap();
                assert_eq!(s.stats.entries, 61);
                assert_eq!(s.state, ScanState::Completed);
                assert_eq!(s.stats.errors, 0);
            }
        }
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn callback_can_read_and_write_live_store() {
        let p = fixture();
        let store = Arc::new(RwLock::new(Snapshot::empty(p.clone())));
        let observed = store.clone();
        scan_live(
            p.clone(),
            ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            store,
            move |_| {
                assert_eq!(observed.read().unwrap().stats.entries, 1);
                let _guard = observed.write().unwrap();
            },
        )
        .unwrap();
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn unreadable_directory_is_incomplete() {
        use std::os::unix::fs::PermissionsExt;
        let p = fixture();
        let d = p.join("blocked");
        fs::create_dir(&d).unwrap();
        fs::set_permissions(&d, fs::Permissions::from_mode(0o0)).unwrap();
        let s = scan(
            p.clone(),
            ScanOptions {
                workers: 2,
                backend: Backend::Portable,
                qos: ScanQos::Automatic,
                descriptor_limit: None,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(s.stats.incomplete);
            assert_eq!(s.stats.errors, 1);
            assert_eq!(s.state, ScanState::Completed);
        }
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn unsearchable_directory_is_no_accounting_gap() {
        use std::os::unix::fs::PermissionsExt;
        let p = fixture();
        let d = p.join("unsearchable");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("file"), b"abc").unwrap();
        // Readable but not searchable: the clone-aware listing and the plain one fail alike.
        fs::set_permissions(&d, fs::Permissions::from_mode(0o644)).unwrap();
        let s = scan(
            p.clone(),
            ScanOptions {
                workers: 2,
                backend: Backend::Bulk,
                qos: ScanQos::Automatic,
                descriptor_limit: None,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(s.stats.incomplete);
            assert!(
                s.accounting.warnings.is_empty(),
                "{:?}",
                s.accounting.warnings
            );
            assert!(s.accounting.checks_complete);
        }
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn dataless_listing_failure_is_recognised() {
        assert!(dataless(&std::io::Error::from_raw_os_error(libc::EDEADLK)));
        assert!(!dataless(&std::io::Error::from_raw_os_error(libc::EACCES)));
    }
    #[cfg(unix)]
    #[test]
    fn disappeared_entries_are_gone_not_errors_but_a_vanished_root_is() {
        let p = fixture();
        let identity = platform_identity(&p).unwrap();
        // Child metadata, open and listing failures of a vanished entry all classify as gone.
        let fd = descriptor::open(&p, &identity).unwrap();
        let name = std::ffi::OsStr::new("missing");
        let missing = entry_result(descriptor::entry_at(&fd, &p, name), || p.join(name));
        assert!(vanished(&missing.unwrap_err()));
        assert!(vanished(&std::io::Error::from_raw_os_error(libc::ENOTDIR)));
        assert!(!vanished(&std::io::Error::from_raw_os_error(libc::EACCES)));
        fs::create_dir_all(p.join("gone/inner")).unwrap();
        fs::write(p.join("gone/inner/file"), b"x").unwrap();
        fs::create_dir(p.join("keep")).unwrap();
        fs::write(p.join("keep/file"), b"x").unwrap();
        let seed = scan(
            p.clone(),
            ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        fs::remove_dir_all(p.join("gone/inner")).unwrap();
        let rescan = |seed: Snapshot, dirty_paths| {
            let store = Arc::new(RwLock::new(Snapshot::empty(p.clone())));
            let stats = scan_live_incremental_authorized(
                ScanRoot::open(p.clone(), identity.clone()).unwrap(),
                ScanOptions::default(),
                Arc::new(AtomicBool::new(false)),
                store.clone(),
                |_| {},
                IncrementalSeed {
                    snapshot: Arc::new(seed),
                    dirty_paths,
                    dirty_directories: vec![],
                    aliases_resolved: false,
                },
            )
            .unwrap();
            (Arc::try_unwrap(store).unwrap().into_inner().unwrap(), stats)
        };
        // A cached listing means the journal reported nothing there: finding that directory
        // missing is a missed event (stale cache), so the rescan retries in full and finds it gone.
        let (stale, stats) = rescan(seed.clone(), vec![]);
        assert!(stats.fallback_reason.unwrap().contains("journal event"));
        assert!(stale.stats.errors == 0 && !stale.stats.incomplete);
        assert_eq!(stale.stats.entries, 4);
        // Its deletion event dirties the parent, so the result is a valid seed that reuses the rest.
        let (reseeded, stats) = rescan(seed, vec![p.join("gone/inner")]);
        assert_eq!(stats.fallback_reason, None);
        assert!(stats.reused_entries > 0);
        assert_eq!(reseeded.stats.entries, 4);
        assert_eq!(reseeded.stats.errors, 0);
        // A vanished scan root is still a failure.
        fs::remove_dir_all(&p).unwrap();
        assert!(enumerate(&p, Backend::Portable, &AtomicBool::new(false), &identity).is_err());
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(scan(p, ScanOptions::default(), cancel, |_| {}).is_err());
    }
    #[test]
    fn mount_boundaries_and_startup_data_policy() {
        assert!(traversal_allowed(
            Path::new("/Users/me"),
            Path::new("/"),
            2,
            1,
            Some(2)
        ));
        assert!(!traversal_allowed(
            Path::new("/mnt/other"),
            Path::new("/"),
            3,
            1,
            Some(2)
        ));
        assert!(!traversal_allowed(
            Path::new("/Volumes/external"),
            Path::new("/"),
            1,
            1,
            Some(2)
        ));
        assert!(!traversal_allowed(
            Path::new("/selected/mount"),
            Path::new("/selected"),
            2,
            1,
            None
        ));
    }
    #[test]
    #[cfg(unix)]
    fn queued_directory_and_ancestor_swaps_never_follow_links() {
        use std::os::unix::fs::symlink;
        for backend in [Backend::Portable, Backend::Auto, Backend::Bulk] {
            let p = fixture();
            let outside = fixture();
            fs::write(outside.join("outside-marker"), b"x").unwrap();
            fs::create_dir_all(p.join("ancestor/queued")).unwrap();
            let path = p.join("ancestor/queued");
            let expected = platform_identity(&path).unwrap();
            assert!(
                enumerate(&path, backend, &AtomicBool::new(false), &expected)
                    .unwrap()
                    .is_empty()
            );
            fs::rename(&path, p.join("original")).unwrap();
            symlink(&outside, &path).unwrap();
            assert!(enumerate(&path, backend, &AtomicBool::new(false), &expected).is_err());
            fs::remove_file(&path).unwrap();
            fs::rename(p.join("original"), &path).unwrap();
            fs::rename(p.join("ancestor"), p.join("old-ancestor")).unwrap();
            symlink(&outside, p.join("ancestor")).unwrap();
            assert!(enumerate(&path, backend, &AtomicBool::new(false), &expected).is_err());
            fs::remove_file(p.join("ancestor")).unwrap();
            fs::rename(p.join("old-ancestor"), p.join("ancestor")).unwrap();
            fs::rename(&path, p.join("original")).unwrap();
            fs::create_dir(&path).unwrap();
            assert!(enumerate(&path, backend, &AtomicBool::new(false), &expected).is_err());
            let mut wrong_device = platform_identity(&path).unwrap();
            wrong_device.device = wrong_device.device.wrapping_add(1);
            assert!(enumerate(&path, backend, &AtomicBool::new(false), &wrong_device).is_err());
            fs::remove_dir_all(p).unwrap();
            fs::remove_dir_all(outside).unwrap();
        }
    }
    #[test]
    #[cfg(unix)]
    fn validated_descriptor_survives_namespace_replacement() {
        let p = fixture();
        let outside = fixture();
        fs::create_dir(p.join("queued")).unwrap();
        fs::write(p.join("queued/original-marker"), b"x").unwrap();
        fs::write(outside.join("outside-marker"), b"x").unwrap();
        let path = p.join("queued");
        let fd = descriptor::open(&path, &platform_identity(&path).unwrap()).unwrap();
        fs::rename(&path, p.join("renamed")).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        let entries = descriptor::enumerate(&fd, &path, &AtomicBool::new(false)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].as_ref().unwrap().path.file_name().unwrap(),
            "original-marker"
        );
        fs::remove_dir_all(p).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn running_allocated_totals_deduplicate_hardlinks() {
        let p = fixture();
        fs::write(p.join("original"), vec![1; 8192]).unwrap();
        let allocated = fs::metadata(p.join("original")).unwrap().blocks() * 512;
        for i in 0..200 {
            let d = p.join(format!("dir{i}"));
            fs::create_dir(&d).unwrap();
            fs::hard_link(p.join("original"), d.join("a")).unwrap();
            fs::hard_link(p.join("original"), d.join("b")).unwrap();
        }
        let store = Arc::new(RwLock::new(Snapshot::empty(p.clone())));
        let observed = AtomicBool::new(false);
        // The walk reports progress with no store lock held; the post-walk event
        // always arrives while still Running, so this observation is deterministic.
        scan_live(
            p.clone(),
            ScanOptions {
                workers: 2,
                backend: Backend::Portable,
                qos: ScanQos::Automatic,
                descriptor_limit: None,
            },
            Arc::new(AtomicBool::new(false)),
            store.clone(),
            |_| {
                let s = store.read().unwrap();
                if s.state == ScanState::Running
                    && s.iter_nodes()
                        .filter(|n| n.identity.kind == NodeKind::File)
                        .count()
                        > 2
                {
                    observed.store(true, Ordering::Relaxed);
                    assert_eq!(s.stats.allocated_bytes, allocated);
                    assert_eq!(s.node(0).unwrap().allocated_size, allocated);
                }
            },
        )
        .unwrap();
        assert_eq!(store.read().unwrap().state, ScanState::Completed);
        let observed = observed.into_inner();
        assert!(observed, "fixture must exercise a Running result");
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn native_startup_alias_identities_match_descriptor_metadata() {
        let root = Path::new("/");
        let entries = enumerate(
            root,
            Backend::Bulk,
            &AtomicBool::new(false),
            &platform_identity(root).unwrap(),
        )
        .unwrap();
        for name in ["Users", "Applications", "System"] {
            let e = entries
                .iter()
                .filter_map(|e| e.as_ref().ok())
                .find(|e| e.path.file_name() == Some(std::ffi::OsStr::new(name)))
                .unwrap();
            assert_eq!(e.identity(), platform_identity(&e.path).unwrap());
            descriptor::open(&e.path, &e.identity()).unwrap();
        }
        let data = Path::new("/System/Volumes/Data");
        descriptor::open(data, &platform_identity(data).unwrap()).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn authorized_root_survives_real_directory_and_symlink_replacements() {
        for backend in [Backend::Auto, Backend::Portable, Backend::Bulk] {
            for symlink in [false, true] {
                let p = fixture();
                let outside = fixture();
                let target = p.join("root");
                fs::create_dir_all(target.join("child")).unwrap();
                fs::write(target.join("child/original-marker"), b"original").unwrap();
                fs::write(outside.join("outside-marker"), b"outside").unwrap();
                let expected = platform_identity(&target).unwrap();
                let authority = ScanRoot::open(target.clone(), expected.clone()).unwrap();
                assert_eq!(authority.path(), target);
                assert_eq!(authority.identity(), &expected);
                fs::rename(&target, p.join("renamed")).unwrap();
                if symlink {
                    std::os::unix::fs::symlink(&outside, &target).unwrap();
                } else {
                    fs::create_dir(&target).unwrap();
                    fs::write(target.join("replacement-marker"), b"replacement").unwrap();
                }
                let store = Arc::new(RwLock::new(Snapshot::empty(target.clone())));
                scan_live_authorized(
                    authority,
                    ScanOptions {
                        workers: 2,
                        backend,
                        qos: ScanQos::Automatic,
                        descriptor_limit: None,
                    },
                    Arc::new(AtomicBool::new(false)),
                    store.clone(),
                    |_| {},
                )
                .unwrap();
                let s = store.read().unwrap();
                assert_eq!(s.state, ScanState::Completed);
                assert!(!s.stats.incomplete);
                assert_eq!(s.node_count(), 3);
                assert!(s
                    .iter_nodes()
                    .any(|n| n.display_name() == "original-marker"));
                assert!(!s.iter_nodes().any(|n| n.display_name() == "outside-marker"
                    || n.display_name() == "replacement-marker"));
                assert_eq!(s.node(0).unwrap().identity, expected);
                drop(s);
                assert!(ScanRoot::open(target, expected).is_err());
                fs::remove_dir_all(p).unwrap();
                fs::remove_dir_all(outside).unwrap();
            }
        }
    }
    #[test]
    #[cfg(unix)]
    fn exhausted_descriptor_budget_still_reaches_subtrees_of_a_renamed_root() {
        let p = fixture();
        let target = p.join("root");
        fs::create_dir_all(target.join("a/b/c")).unwrap();
        fs::write(target.join("a/b/c/f"), b"x").unwrap();
        let authority =
            ScanRoot::open(target.clone(), platform_identity(&target).unwrap()).unwrap();
        fs::rename(&target, p.join("renamed")).unwrap();
        let store = Arc::new(RwLock::new(Snapshot::empty(target)));
        // One descriptor is the held root, so every deeper directory is reopened without a parent.
        scan_live_authorized(
            authority,
            ScanOptions {
                workers: 2,
                descriptor_limit: Some(1),
                ..ScanOptions::default()
            },
            Arc::new(AtomicBool::new(false)),
            store.clone(),
            |_| {},
        )
        .unwrap();
        let s = store.read().unwrap();
        assert_eq!(s.state, ScanState::Completed);
        assert!(!s.stats.incomplete);
        assert_eq!(s.stats.errors, 0);
        assert_eq!(s.node_count(), 5);
        assert!(s.iter_nodes().any(|n| n.display_name() == "f"));
        assert!(
            s.worker_stats
                .iter()
                .map(|w| w.absolute_reopens)
                .sum::<u64>()
                >= 2
        );
        drop(s);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn authorized_root_survives_ancestor_swap_and_rejects_stale_initial_authority() {
        let p = fixture();
        let outside = fixture();
        let ancestor = p.join("ancestor");
        let target = ancestor.join("root");
        fs::create_dir_all(target.join("child")).unwrap();
        fs::write(target.join("child/original-marker"), b"original").unwrap();
        fs::create_dir(outside.join("root")).unwrap();
        fs::write(outside.join("root/outside-marker"), b"outside").unwrap();
        let expected = platform_identity(&target).unwrap();
        let authority = ScanRoot::open(target.clone(), expected.clone()).unwrap();
        fs::rename(&ancestor, p.join("renamed-ancestor")).unwrap();
        std::os::unix::fs::symlink(&outside, &ancestor).unwrap();
        assert!(ScanRoot::open(target.clone(), expected.clone()).is_err());
        let store = Arc::new(RwLock::new(Snapshot::empty(target)));
        scan_live_authorized(
            authority,
            ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            store.clone(),
            |_| {},
        )
        .unwrap();
        let s = store.read().unwrap();
        assert_eq!(s.node_count(), 3);
        assert_eq!(s.node(0).unwrap().identity, expected);
        assert!(s
            .iter_nodes()
            .any(|n| n.display_name() == "original-marker"));
        assert!(!s.iter_nodes().any(|n| n.display_name() == "outside-marker"));
        drop(s);
        fs::remove_dir_all(p).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
    fn assert_worker_accounting(s: &Snapshot, workers: usize) {
        assert_eq!(s.worker_stats.len(), workers);
        assert_eq!(
            s.worker_stats.iter().map(|w| w.entries).sum::<u64>(),
            s.stats.entries - 1
        );
        assert_eq!(
            s.worker_stats.iter().map(|w| w.errors).sum::<u64>(),
            s.stats.errors
        );
        for (i, w) in s.worker_stats.iter().enumerate() {
            assert_eq!(w.id, i + 1);
            assert!(!w.qos.is_empty());
            assert!(w.relative_opens + w.absolute_reopens <= w.directories);
            assert!(w.busy_us <= w.elapsed_us);
            assert!(w.queue_wait_us <= w.elapsed_us);
            assert!(
                w.metadata_us
                    .saturating_add(w.commit_us)
                    .saturating_add(w.progress_us)
                    .saturating_add(w.store_wait_us)
                    <= w.busy_us.saturating_add(100)
            );
        }
    }
    #[test]
    fn worker_diagnostics_count_entries_tasks_idle_and_cancelled_workers() {
        let p = fixture();
        for workers in [1, 12, 16] {
            let s = scan(
                p.clone(),
                ScanOptions {
                    workers,
                    backend: Backend::Portable,
                    qos: ScanQos::Automatic,
                    descriptor_limit: None,
                },
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            assert_worker_accounting(&s, workers);
            assert_eq!(s.worker_stats.iter().map(|w| w.directories).sum::<u64>(), 1);
            assert!(s.worker_stats.iter().filter(|w| w.directories == 0).count() >= workers - 1);
            let cancelled = scan(
                p.clone(),
                ScanOptions {
                    workers,
                    backend: Backend::Portable,
                    qos: ScanQos::Automatic,
                    descriptor_limit: None,
                },
                Arc::new(AtomicBool::new(true)),
                |_| {},
            )
            .unwrap();
            assert_worker_accounting(&cancelled, workers);
            assert_eq!(
                cancelled
                    .worker_stats
                    .iter()
                    .map(|w| w.directories)
                    .sum::<u64>(),
                0
            );
        }
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn worker_diagnostics_cover_skewed_directories_and_error_totals() {
        let p = fixture();
        fs::create_dir(p.join("large")).unwrap();
        for i in 0..300 {
            fs::write(p.join(format!("large/f{i}")), b"x").unwrap();
        }
        for i in 0..20 {
            fs::create_dir(p.join(format!("small{i}"))).unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::create_dir(p.join("blocked")).unwrap();
            fs::set_permissions(p.join("blocked"), fs::Permissions::from_mode(0o0)).unwrap();
        }
        for workers in [1, 12, 16] {
            let s = scan(
                p.clone(),
                ScanOptions {
                    workers,
                    backend: Backend::Portable,
                    qos: ScanQos::Automatic,
                    descriptor_limit: None,
                },
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            assert_worker_accounting(&s, workers);
            assert!(s.worker_stats.iter().map(|w| w.directories).sum::<u64>() >= 22);
            #[cfg(unix)]
            if unsafe { libc::geteuid() } != 0 {
                assert_eq!(s.stats.errors, 1);
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(p.join("blocked"), fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn descriptor_diagnostics_count_open_attempts_and_budget_bounds() {
        let p = fixture();
        fs::create_dir(p.join("child")).unwrap();
        fs::create_dir(p.join("blocked")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p.join("blocked"), fs::Permissions::from_mode(0o0)).unwrap();
        let s = scan(
            p.clone(),
            ScanOptions {
                workers: 4,
                backend: Backend::Portable,
                qos: ScanQos::Automatic,
                descriptor_limit: None,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        assert_worker_accounting(&s, 4);
        let attempted = s.worker_stats.iter().map(|w| w.directories).sum::<u64>();
        let opened = s
            .worker_stats
            .iter()
            .map(|w| w.relative_opens + w.absolute_reopens)
            .sum::<u64>();
        assert_eq!(attempted, 3);
        assert_eq!(opened + 1, attempted);
        assert!(s.descriptor_limit.unwrap() > 0);
        assert!(s.descriptor_peak.unwrap() >= 1);
        assert!(s.descriptor_peak.unwrap() <= s.descriptor_limit.unwrap());
        fs::set_permissions(p.join("blocked"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn a_narrowed_descriptor_limit_bounds_the_scan_and_the_default_is_the_ceiling() {
        let p = fixture();
        for name in ["a", "b", "c"] {
            fs::create_dir_all(p.join(name).join("deep")).unwrap();
        }
        let scan_with = |limit| {
            scan(
                p.clone(),
                ScanOptions {
                    workers: 4,
                    descriptor_limit: limit,
                    ..ScanOptions::default()
                },
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap()
        };
        let narrowed = scan_with(Some(2));
        assert_eq!(narrowed.descriptor_limit, Some(2));
        assert!(narrowed.descriptor_peak.unwrap() <= 2);
        assert_eq!(narrowed.stats.directories, 7);
        let unconstrained = scan_with(None);
        assert_eq!(
            unconstrained.descriptor_limit,
            Some(default_descriptor_limit())
        );
        assert_eq!(unconstrained.stats.entries, narrowed.stats.entries);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn io_issue_preserves_error_kind_code_and_original_accounting() {
        let mut snapshot = Snapshot::empty(PathBuf::from("/fixture"));
        io_issue(
            &mut snapshot,
            PathBuf::from("/fixture/missing"),
            std::io::Error::from_raw_os_error(libc::ENOENT),
        );
        assert_eq!(snapshot.stats.errors, 1);
        assert!(snapshot.stats.incomplete);
        assert_eq!(snapshot.errors[0].error_kind.as_deref(), Some("NotFound"));
        assert_eq!(snapshot.errors[0].raw_os_error, Some(libc::ENOENT));
        let legacy: ScanIssue =
            serde_json::from_str(r#"{"path":"/fixture","message":"old issue"}"#).unwrap();
        assert!(legacy.error_kind.is_none());
        assert!(legacy.raw_os_error.is_none());
    }
    #[test]
    fn entry_failure_paths_survive_listing_and_keep_native_causes() {
        let root = PathBuf::from("/fixture");
        let mut snapshot = Snapshot::empty(root.clone());
        let failures = [libc::EPERM, libc::EACCES, libc::ENOENT];
        let listing = failures
            .iter()
            .enumerate()
            .map(|(i, code)| {
                entry_result(Err(std::io::Error::from_raw_os_error(*code)), || {
                    root.join(format!("child-{i}"))
                })
            })
            .collect();
        let listing = retry::complete_listing(listing).unwrap();
        for item in listing {
            io_issue(&mut snapshot, root.clone(), item.unwrap_err());
        }
        assert_eq!(snapshot.stats.errors, 3);
        assert!(snapshot.stats.incomplete);
        for (i, code) in failures.iter().enumerate() {
            assert_eq!(snapshot.errors[i].path, root.join(format!("child-{i}")));
            assert_eq!(snapshot.errors[i].raw_os_error, Some(*code));
            assert_eq!(
                snapshot.errors[i].message,
                std::io::Error::from_raw_os_error(*code).to_string()
            );
        }
        // A terminal directory-read failure still belongs to the directory.
        io_issue(
            &mut snapshot,
            root.clone(),
            std::io::Error::from_raw_os_error(libc::EIO),
        );
        assert_eq!(snapshot.errors[3].path, root);
        assert_eq!(snapshot.stats.errors, 4);
        let interrupted = entry_result(Err(std::io::Error::from_raw_os_error(libc::EINTR)), || {
            PathBuf::from("/fixture/child")
        });
        assert_eq!(
            retry::complete_listing(vec![interrupted])
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EINTR)
        );
    }
    #[cfg(unix)]
    #[test]
    fn failed_native_child_metadata_reports_unusual_filename_not_parent() {
        use std::ffi::OsStr;
        let root = fixture();
        let fd = descriptor::open(&root, &platform_identity(&root).unwrap()).unwrap();
        let name = OsStr::new("missing \" 日本語\nchild");
        let result = entry_result(descriptor::entry_at(&fd, &root, name), || root.join(name));
        let mut snapshot = Snapshot::empty(root.clone());
        io_issue(&mut snapshot, root.clone(), result.unwrap_err());
        assert_eq!(snapshot.errors[0].path, root.join(name));
        assert_eq!(snapshot.errors[0].raw_os_error, Some(libc::ENOENT));
        assert_eq!(snapshot.errors[0].error_kind.as_deref(), Some("NotFound"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn successful_metadata_does_not_allocate_error_context() {
        let result = entry_result(
            Ok(Entry {
                clone_checked: false,
                path: PathBuf::from("/fixture/file").into(),
                device: 1,
                inode: 1,
                kind: NodeKind::File,
                hard_link: false,
                length: 1,
                allocated: 1,
            }),
            || panic!("error context must be lazy"),
        );
        assert_eq!(result.unwrap().length, 1);
    }
    #[test]
    fn expected_exclusions_remain_complete_and_keep_separate_retention_counts() {
        let mut snapshot = Snapshot::empty(PathBuf::from("/fixture"));
        snapshot.state = ScanState::Completed;
        for i in 0..10007 {
            exclusion(&mut snapshot, PathBuf::from(format!("/fixture/mount{i}")));
        }
        assert_eq!(snapshot.stats.excluded, 10007);
        assert_eq!(snapshot.exclusions.len(), 10000);
        assert_eq!(snapshot.stats.errors, 0);
        assert!(snapshot.errors.is_empty());
        assert!(!snapshot.stats.incomplete);
        assert_eq!(snapshot.state, ScanState::Completed);
        for _ in 0..10003 {
            io_issue(
                &mut snapshot,
                PathBuf::from("/fixture/missing"),
                std::io::Error::from_raw_os_error(libc::ENOENT),
            );
        }
        assert_eq!(snapshot.stats.errors, 10003);
        assert_eq!(snapshot.errors.len(), 10000);
        assert_eq!(snapshot.exclusions.len(), 10000);
        assert!(snapshot.stats.incomplete);
    }
    #[test]
    fn legacy_stats_and_progress_default_exclusion_fields() {
        let mut json = serde_json::to_value(ScanStats::default()).unwrap();
        json.as_object_mut().unwrap().remove("excluded");
        let old: ScanStats = serde_json::from_value(json).unwrap();
        assert_eq!(old.excluded, 0);
        let progress: Progress =
            serde_json::from_str(r#"{"entries":1,"directories":1,"errors":0,"elapsed_ms":0}"#)
                .unwrap();
        assert_eq!(progress.excluded, 0);
    }
    #[test]
    fn node_exclusion_truth_survives_detail_cap_and_legacy_node_json() {
        let mut snapshot = Snapshot::empty(PathBuf::from("/fixture"));
        for i in 0..10000 {
            exclusion(&mut snapshot, PathBuf::from(format!("/fixture/mount{i}")));
        }
        let path = PathBuf::from("/fixture/beyond-cap");
        let entry = Entry {
            clone_checked: false,
            path: path.clone().into(),
            device: 2,
            inode: 7,
            kind: NodeKind::Directory,
            length: 0,
            allocated: 0,
            hard_link: false,
        };
        let mut n = node(entry, 0, None);
        assert!(!n.excluded);
        let permitted = traversal_allowed(&path, Path::new("/fixture"), n.identity.device, 1, None);
        record_node_scope(&mut snapshot, &n.path, &mut n.excluded, permitted);
        assert!(n.excluded);
        assert_eq!(snapshot.exclusions.len(), 10000);
        assert!(!snapshot.exclusions.iter().any(|issue| issue.path == path));
        let mut json = serde_json::to_value(&n).unwrap();
        json.as_object_mut().unwrap().remove("excluded");
        let old: Node = serde_json::from_value(json).unwrap();
        assert!(!old.excluded);
    }
    #[test]
    fn restricted_counter_preserves_all_errors_beyond_detail_cap_and_legacy_defaults() {
        let mut snapshot = Snapshot::empty(PathBuf::from("/fixture"));
        for _ in 0..10005 {
            io_issue(
                &mut snapshot,
                PathBuf::from("/System/Library/synthetic-service"),
                std::io::Error::from_raw_os_error(libc::EPERM),
            );
        }
        assert_eq!(snapshot.stats.errors, 10005);
        assert_eq!(snapshot.errors.len(), 10000);
        assert!(snapshot.stats.incomplete);
        #[cfg(target_os = "macos")]
        assert_eq!(snapshot.stats.restricted_system_issues, 10005);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(snapshot.stats.restricted_system_issues, 0);
        let before = snapshot.stats.restricted_system_issues;
        io_issue(
            &mut snapshot,
            PathBuf::from("/System/Library/synthetic-service"),
            std::io::Error::from_raw_os_error(libc::EIO),
        );
        assert_eq!(snapshot.stats.restricted_system_issues, before);
        let mut json = serde_json::to_value(ScanStats::default()).unwrap();
        json.as_object_mut()
            .unwrap()
            .remove("restricted_system_issues");
        assert_eq!(
            serde_json::from_value::<ScanStats>(json)
                .unwrap()
                .restricted_system_issues,
            0
        );
    }
}
