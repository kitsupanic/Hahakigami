mod access_setup;
mod automation;
mod budget;
mod cache;
mod cleanup;
mod cleanup_queue;
pub mod cli;
mod comparison;
mod events;
mod explorer_preferences;
mod file_type_map;
mod file_types;
mod filters;
mod instance;
mod library;
mod mcp;
#[cfg(feature = "performance-benchmark")]
pub mod performance;
mod platform;
mod preview_validation;
mod quick_look;
mod safety;
mod settings;
mod slots;
mod sorting;
use cleanup::{Review, TrashResult};
use disk_scan::{NodeKind, NodeRef, ScanState, Snapshot};
use serde::{Deserialize, Serialize};
use slots::{Need, Request, Session, Slot};
use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, PoisonError, RwLock,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{ipc::Channel, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// Master switch for the Saved scans feature. Flip to `true` and un-skip the tests marked with this name.
pub(crate) const SAVED_SCANS_ENABLED: bool = false;
pub(crate) const SAVED_SCANS_DISABLED: &str = "Saved scans are turned off in this version.";

pub(crate) fn require_saved_scans() -> Result<(), String> {
    if SAVED_SCANS_ENABLED {
        Ok(())
    } else {
        Err(SAVED_SCANS_DISABLED.into())
    }
}

#[tauri::command]
fn saved_scans_enabled() -> bool {
    SAVED_SCANS_ENABLED
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeView {
    id: u32,
    parent_id: Option<u32>,
    name: String,
    path: String,
    path_key: String,
    kind: String,
    logical_bytes: u64,
    allocated_bytes: u64,
    raw_allocated_bytes: u64,
    verified_clone: bool,
    clone_deduction_bytes: u64,
    clone_owner: Option<bool>,
    children_count: usize,
    incomplete: bool,
    excluded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_count: Option<u64>,
}
fn path_key(path: &Path) -> String {
    use std::fmt::Write;
    let mut key = String::with_capacity(path.as_os_str().len());
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.!~*'()".contains(&byte) {
            key.push(char::from(byte));
        } else {
            write!(&mut key, "%{byte:02X}").unwrap();
        }
    }
    key
}
fn view(node: &NodeRef<'_>, issues: &[PathBuf]) -> Result<NodeView, String> {
    let path = node.path()?;
    let path_key = path_key(&path);
    let incomplete = issues
        .get(issues.partition_point(|p| p < &path))
        .is_some_and(|p| p.starts_with(&path));
    let path = path
        .into_os_string()
        .into_string()
        .unwrap_or_else(|native| native.to_string_lossy().into_owned());
    Ok(NodeView {
        id: node.id,
        parent_id: node.parent,
        name: node.display_name().into_owned(),
        path,
        path_key,
        kind: match node.identity.kind {
            NodeKind::Directory => "directory",
            NodeKind::File => "file",
            NodeKind::Symlink => "symlink",
            NodeKind::Other => "other",
        }
        .into(),
        logical_bytes: node.logical_size,
        allocated_bytes: node.allocated_size,
        raw_allocated_bytes: node.own_allocated_size,
        verified_clone: node.clone_metadata().is_some_and(|metadata| {
            metadata.verified && metadata.clone_id != 0 && metadata.reference_count > 1
        }),
        clone_deduction_bytes: node
            .clone_attribution()
            .map_or(0, |(_, deduction)| deduction),
        clone_owner: node.clone_attribution().map(|(owner, _)| owner),
        children_count: node.children().len(),
        incomplete,
        excluded: node.excluded,
        file_type: None,
        file_count: None,
    })
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    scan_id: String,
    root_id: u32,
    state: String,
    entries: u64,
    logical_bytes: u64,
    allocated_bytes: u64,
    accounting_model: disk_scan::AccountingModel,
    accounting_warnings: Vec<String>,
    elapsed_ms: u64,
    errors: u64,
    excluded: u64,
    incomplete: bool,
    restricted_system_issues: u64,
    errors_stored: usize,
    exclusions_stored: usize,
    message: Option<String>,
    workers: usize,
    worker_mode: settings::WorkerMode,
    worker_source: String,
    qos: disk_scan::ScanQos,
    worker_stats: Vec<disk_scan::WorkerStats>,
    finalization_us: u64,
    descriptor_limit: Option<usize>,
    descriptor_peak: Option<usize>,
    origin: SessionOrigin,
    saved_id: Option<String>,
    started_at_ms: u64,
    completed_at_ms: Option<u64>,
    scan_mode: String,
    scan_phase: String,
    walk_ms: u64,
    reused_entries: u64,
    rescanned_directories: u64,
    fallback_reason: Option<String>,
    cache_warning: Option<String>,
}
#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SessionOrigin {
    Live,
    Saved,
}
#[derive(Clone)]
struct ScanDetails {
    actual_workers: Option<usize>,
    mode: String,
    phase: String,
    reused_entries: u64,
    rescanned_directories: u64,
    fallback_reason: Option<String>,
    cache_warning: Option<String>,
    #[cfg(feature = "performance-benchmark")]
    phase_timings_us: std::collections::BTreeMap<String, u64>,
    #[cfg(feature = "performance-benchmark")]
    reuse_storage: Option<disk_scan::ReuseStorageStats>,
    #[cfg(feature = "performance-benchmark")]
    map_sort_scratch: Option<MapSortScratch>,
}
impl Default for ScanDetails {
    fn default() -> Self {
        Self {
            actual_workers: None,
            mode: "full".into(),
            phase: "scanning".into(),
            reused_entries: 0,
            rescanned_directories: 0,
            fallback_reason: None,
            cache_warning: None,
            #[cfg(feature = "performance-benchmark")]
            phase_timings_us: Default::default(),
            #[cfg(feature = "performance-benchmark")]
            reuse_storage: None,
            #[cfg(feature = "performance-benchmark")]
            map_sort_scratch: None,
        }
    }
}
fn session_running(s: &Session) -> bool {
    s.finishing.load(Ordering::Acquire) || s.store.read().unwrap().state == ScanState::Running
}
impl Session {
    fn actual_workers(&self) -> usize {
        self.scan_details
            .lock()
            .unwrap()
            .actual_workers
            .unwrap_or(self.workers)
    }
    fn summary(&self) -> Summary {
        let s = self.store.read().unwrap();
        if s.state != ScanState::Running {
            let _ = self.completed_at_ms.compare_exchange(
                0,
                now_ms(),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        let details = self.scan_details.lock().unwrap().clone();
        Summary {
            scan_mode: details.mode,
            // A finished walk holds the complete tree with raw sizes while
            // clone sharing is verified; only "scanning" is refined.
            scan_phase: if details.phase == "scanning"
                && s.state == ScanState::Running
                && s.stats.walk_ms > 0
            {
                "verifying".into()
            } else {
                details.phase
            },
            walk_ms: s.stats.walk_ms,
            reused_entries: details.reused_entries,
            rescanned_directories: details.rescanned_directories,
            fallback_reason: details.fallback_reason,
            cache_warning: details.cache_warning,
            origin: self.origin,
            saved_id: self.saved_id.clone(),
            started_at_ms: self.started_at_ms,
            completed_at_ms: match self.completed_at_ms.load(Ordering::Acquire) {
                0 => None,
                n => Some(n),
            },
            scan_id: self.id.clone(),
            root_id: 0,
            state: if self.finishing.load(Ordering::Acquire) {
                "running"
            } else {
                match s.state {
                    ScanState::Running => "running",
                    ScanState::Completed => "completed",
                    ScanState::Cancelled => "cancelled",
                    ScanState::Failed => "failed",
                }
            }
            .into(),
            entries: s.stats.entries,
            logical_bytes: s.stats.logical_bytes,
            allocated_bytes: s.stats.allocated_bytes,
            accounting_model: s.accounting.model,
            accounting_warnings: s.accounting.warnings.clone(),
            elapsed_ms: s.stats.elapsed_ms,
            errors: s.stats.errors,
            excluded: s.stats.excluded,
            incomplete: s.stats.incomplete,
            restricted_system_issues: s.stats.restricted_system_issues,
            errors_stored: s.errors.len(),
            exclusions_stored: s.exclusions.len(),
            message: s
                .errors
                .iter()
                .find(|issue| !disk_scan::is_system_restriction(issue))
                .map(|e| format!("{}: {}", e.path.display(), e.message)),
            workers: details.actual_workers.unwrap_or(self.workers),
            worker_mode: self.worker_mode,
            worker_source: self.worker_source.clone(),
            qos: self.qos,
            worker_stats: s.worker_stats.clone(),
            finalization_us: s.finalization_us,
            descriptor_limit: s.descriptor_limit,
            descriptor_peak: s.descriptor_peak,
        }
    }
}
fn issue_index<'a>(
    session: &'a Session,
    snapshot: &Snapshot,
) -> std::sync::MutexGuard<'a, (u64, Vec<PathBuf>)> {
    let mut index = session.issues.lock().unwrap();
    if index.0 != snapshot.stats.errors || index.1.len() != snapshot.errors.len() {
        index.0 = snapshot.stats.errors;
        index.1 = snapshot.errors.iter().map(|e| e.path.clone()).collect();
        index.1.sort_unstable();
    }
    index
}

#[derive(Default)]
struct AppState {
    scan_settings: Arc<Mutex<settings::ScanSettings>>,
    explorer_preferences: Arc<Mutex<explorer_preferences::Preferences>>,
    sort_requests: Arc<AtomicU64>,
    preview: Arc<quick_look::Registry>,
    access_setup: Arc<Mutex<access_setup::AccessSetup>>,
    targets: Mutex<HashMap<String, (platform::Target, PathBuf)>>,
    target_guards: Mutex<HashMap<String, safety::CheckedItem>>,
    // The live scans, the open saved scan and the scan budget; commands reach scans only through it.
    scans: Arc<slots::Slots>,
    library: Arc<Mutex<Option<library::Library>>>,
    cleanup_queue: Arc<Mutex<cleanup_queue::Queue>>,
    rescan_cache: Arc<Mutex<Option<cache::Cache>>>,
    activity: Arc<ActivityGate>,
    saved_open: Arc<Mutex<Option<Arc<SavedOpen>>>>,
}
fn register(
    state: &AppState,
    target: platform::Target,
    path: PathBuf,
) -> Result<platform::Target, String> {
    let guard = safety::CheckedItem {
        identity: safety::Identity::read(&path)?,
        ancestors: safety::ancestors(&path)?,
        path: path.clone(),
        volume: None,
    };
    if guard.identity.kind != 1 {
        return Err("Choose a directory, not a symbolic link.".into());
    }
    Ok(retain_target(state, target, guard))
}
/// Records `target` with the `guard` its scan start validates; the path scanned is the guard's.
fn retain_target(
    state: &AppState,
    target: platform::Target,
    guard: safety::CheckedItem,
) -> platform::Target {
    let path = guard.path.clone();
    state
        .target_guards
        .lock()
        .unwrap()
        .insert(target.id.clone(), guard);
    state
        .targets
        .lock()
        .unwrap()
        .insert(target.id.clone(), (target.clone(), path));
    target
}

#[tauri::command]
async fn discover_targets(app: tauri::AppHandle) -> Result<Vec<platform::Target>, String> {
    #[cfg(feature = "performance-benchmark")]
    if let Some(config) = performance::app_config() {
        return Ok(vec![register(
            &app.state::<AppState>(),
            platform::target(&config.root, "Benchmark location".into()),
            config.root.clone(),
        )?]);
    }
    // statvfs/lstat can stall on a dead network mount; keep it off the async workers.
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        platform::discover()
            .into_iter()
            .map(|(t, p)| register(&state, t, p))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}
/// Paths `discover_targets` would list as disks. Reads the mount table only: no capacity probe, no registration.
#[tauri::command]
async fn mounted_volumes() -> Vec<String> {
    platform::volume_paths()
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}
#[tauri::command]
async fn choose_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<platform::Target>, String> {
    let chosen =
        tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
            .await
            .map_err(|e| e.to_string())?;
    match chosen {
        None => Ok(None),
        Some(file) => {
            let path = file.into_path().map_err(|e| e.to_string())?;
            let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned();
            Ok(Some(register(&state, platform::target(&path, name), path)?))
        }
    }
}
fn validate_target(guard: &safety::CheckedItem) -> Result<(), String> {
    if safety::Identity::read(&guard.path)? != guard.identity
        || std::fs::canonicalize(&guard.path).map_err(|e| e.to_string())? != guard.path
    {
        return Err("The chosen folder changed. Choose it again before scanning.".into());
    }
    for (path, identity) in &guard.ancestors {
        if safety::Identity::read(path)? != *identity {
            return Err("A scan target ancestor changed. Choose the folder again.".into());
        }
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
// Owned guard can cross the blocking-task boundary. All session mutations reserve
// it before acquiring other locks, so library operations cannot race cleanup.
// Exclusive holders (library, cleanup, queue edits, restart) refuse each other and every scan start.
// Scan starts only share it, so scans on different slots can begin at the same time.
#[derive(Default)]
struct ActivityGate {
    // -1: one exclusive holder; n >= 0: n scan starts in flight.
    state: AtomicIsize,
}
impl ActivityGate {
    fn busy(&self) -> bool {
        self.state.load(Ordering::Acquire) != 0
    }
}
struct Activity {
    gate: Arc<ActivityGate>,
    shared: bool,
}
impl Activity {
    const BUSY: &'static str = "Another operation is in progress. Wait for it to finish.";
    fn begin(gate: &Arc<ActivityGate>) -> Result<Self, String> {
        gate.state
            .compare_exchange(0, -1, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Self::BUSY.to_string())?;
        Ok(Self {
            gate: gate.clone(),
            shared: false,
        })
    }
    fn begin_shared(gate: &Arc<ActivityGate>) -> Result<Self, String> {
        gate.state
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n >= 0).then_some(n + 1)
            })
            .map_err(|_| Self::BUSY.to_string())?;
        Ok(Self {
            gate: gate.clone(),
            shared: true,
        })
    }
}
impl Drop for Activity {
    fn drop(&mut self) {
        if self.shared {
            self.gate.state.fetch_sub(1, Ordering::Release);
        } else {
            self.gate.state.store(0, Ordering::Release);
        }
    }
}
// The commit mutex orders cancellation against selection persistence and session replacement.
struct SavedOpen {
    id: String,
    cancelled: AtomicBool,
    started: AtomicBool,
    committed: Mutex<bool>,
}
struct SavedOpenGuard(Arc<Mutex<Option<Arc<SavedOpen>>>>, String);
impl Drop for SavedOpenGuard {
    fn drop(&mut self) {
        let mut slot = self.0.lock().unwrap();
        if slot.as_ref().is_some_and(|job| job.id == self.1) {
            *slot = None;
        }
    }
}
#[tauri::command]
fn begin_saved_open(state: State<'_, AppState>) -> Result<String, String> {
    require_saved_scans()?;
    let mut slot = state.saved_open.lock().unwrap();
    if slot
        .as_ref()
        .is_some_and(|job| job.started.load(Ordering::Acquire))
        || state.activity.busy()
    {
        return Err("Another operation is in progress. Wait for it to finish.".into());
    }
    // A lost handshake response must not reserve application activity forever.
    // Unstarted tokens can be replaced; claiming and replacement share this lock.
    let id = uuid::Uuid::new_v4().to_string();
    *slot = Some(Arc::new(SavedOpen {
        id: id.clone(),
        cancelled: AtomicBool::new(false),
        started: AtomicBool::new(false),
        committed: Mutex::new(false),
    }));
    Ok(id)
}
#[tauri::command]
fn cancel_saved_open(state: State<'_, AppState>, request_id: String) -> bool {
    let job = state.saved_open.lock().unwrap().clone();
    let Some(job) = job.filter(|job| job.id == request_id) else {
        return false;
    };
    let committed = job.committed.lock().unwrap();
    if *committed {
        return false;
    }
    job.cancelled.store(true, Ordering::Release);
    true
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryProgress {
    operation: &'static str,
    phase: &'static str,
    bytes: u64,
}
fn progress(
    channel: &Channel<LibraryProgress>,
    operation: &'static str,
    phase: &'static str,
    bytes: u64,
) {
    let _ = channel.send(LibraryProgress {
        operation,
        phase,
        bytes,
    });
}
#[tauri::command]
async fn list_saved_scans(state: State<'_, AppState>) -> Result<library::List, String> {
    require_saved_scans()?;
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("Saved scan library is unavailable.")?
            .list()
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn save_scan(
    state: State<'_, AppState>,
    scan_id: String,
    on_progress: Channel<LibraryProgress>,
) -> Result<library::SavedEntry, String> {
    require_saved_scans()?;
    let activity = Activity::begin(&state.activity)?;
    let s = state.scans.find(&scan_id)?;
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = activity;
        let held = s.hold(Need::LiveCompleted)?;
        let summary = s.summary();
        let snapshot = held.completed_snapshot()?;
        let metadata = library::ScanMetadata {
            started_at_ms: s.started_at_ms,
            completed_at_ms: summary
                .completed_at_ms
                .ok_or("Scan completion time is unavailable.")?,
            workers: s.actual_workers(),
            worker_mode: match s.worker_mode {
                settings::WorkerMode::Auto => "auto",
                settings::WorkerMode::Manual => "manual",
            }
            .into(),
            worker_source: s.worker_source.clone(),
            qos: s.qos,
        };
        progress(&on_progress, "save", "writing", 0);
        let guard = library.lock().unwrap();
        let library = guard.as_ref().ok_or("Saved scan library is unavailable.")?;
        let mut last_progress = Instant::now();
        let entry = library.save(&snapshot, &metadata, |bytes| {
            if last_progress.elapsed() >= std::time::Duration::from_millis(100) {
                progress(&on_progress, "save", "writing", bytes);
                last_progress = Instant::now();
            }
        })?;
        progress(&on_progress, "save", "done", 0);
        Ok(entry)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn open_saved_scan<R: tauri::Runtime>(
    state: State<'_, AppState>,
    id: String,
    request_id: String,
    on_progress: Channel<LibraryProgress>,
    app: tauri::AppHandle<R>,
) -> Result<Summary, String> {
    require_saved_scans()?;
    let job = {
        let slot = state.saved_open.lock().unwrap();
        let job = slot
            .as_ref()
            .filter(|job| job.id == request_id)
            .cloned()
            .ok_or("Saved scan opening request is no longer active.")?;
        job.started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Saved scan opening request has already started.")?;
        job
    };
    let request_guard = SavedOpenGuard(state.saved_open.clone(), request_id);
    if job.cancelled.load(Ordering::Acquire) {
        return Err(library::OPEN_CANCELLED.into());
    }
    let activity = Activity::begin(&state.activity)?;
    quick_look::close(app.clone(), state.preview.clone()).await?;
    state
        .scans
        .require_idle("Cancel the running scan before opening saved results.")?;
    let library = state.library.clone();
    let scans = state.scans.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _request_guard = request_guard;
        let _activity = activity;
        let guard = library.lock().unwrap();
        let library = guard.as_ref().ok_or("Saved scan library is unavailable.")?;
        progress(&on_progress, "open", "reading", 0);
        let mut last_progress = Instant::now();
        let (snapshot, metadata) = library
            .load_cancellable(
                &id,
                |bytes| {
                    if last_progress.elapsed() >= std::time::Duration::from_millis(100) {
                        progress(&on_progress, "open", "reading", bytes);
                        last_progress = Instant::now();
                    }
                },
                &job.cancelled,
                || progress(&on_progress, "open", "validating", 0),
            )
            .map_err(|error| {
                if job.cancelled.load(Ordering::Acquire) {
                    library::OPEN_CANCELLED.to_string()
                } else {
                    error
                }
            })?;
        let s = Arc::new(Session::saved(&id, snapshot, metadata));
        // Startup selection must commit before replacement, keeping the existing
        // session available on every load/persistence failure.
        let mut committed = job.committed.lock().unwrap();
        if job.cancelled.load(Ordering::Acquire) {
            return Err(library::OPEN_CANCELLED.into());
        }
        library.mark_opened(&id)?;
        let summary = s.summary();
        scans.open_saved(s);
        *committed = true;
        drop(committed);
        progress(&on_progress, "open", "done", 0);
        Ok(summary)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn delete_saved_scan(state: State<'_, AppState>, id: String) -> Result<(), String> {
    require_saved_scans()?;
    let activity = Activity::begin(&state.activity)?;
    let library = state.library.clone();
    let scans = state.scans.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = activity;
        library
            .lock()
            .unwrap()
            .as_ref()
            .ok_or("Saved scan library is unavailable.")?
            .delete(&id)?;
        // A deleted saved scan must not stay open.
        scans.close_saved(&id);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

// Sharing never grants authority over the paths recorded inside an archive.
// Only the native dialog supplies an external source/destination path.
fn portable_activity(state: &AppState) -> Result<Activity, String> {
    state.scans.idle_exclusive(
        &state.activity,
        "Cancel the running scan before importing or exporting results.",
    )
}

#[tauri::command]
async fn import_saved_scan<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    on_progress: Channel<LibraryProgress>,
) -> Result<Option<library::SavedEntry>, String> {
    require_saved_scans()?;
    let activity = portable_activity(&state)?;
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = activity;
        let Some(file) = app
            .dialog()
            .file()
            .add_filter("Hahakigami scan", &["hahakigamiscan"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let path = file.into_path().map_err(|e| e.to_string())?;
        let guard = library.lock().unwrap();
        let library = guard.as_ref().ok_or("Saved scan library is unavailable.")?;
        progress(&on_progress, "import", "reading", 0);
        let mut last_progress = Instant::now();
        let entry = library.import_scan(&path, |bytes| {
            if last_progress.elapsed() >= std::time::Duration::from_millis(100) {
                progress(&on_progress, "import", "reading", bytes);
                last_progress = Instant::now();
            }
        })?;
        progress(&on_progress, "import", "done", 0);
        Ok(Some(entry))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn export_saved_scan<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    id: String,
    on_progress: Channel<LibraryProgress>,
) -> Result<bool, String> {
    require_saved_scans()?;
    let activity = portable_activity(&state)?;
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = activity;
        // Derive the suggested name from the local library, never a frontend path.
        let filename = {
            let guard = library.lock().unwrap();
            let library = guard.as_ref().ok_or("Saved scan library is unavailable.")?;
            let entry = library
                .list()?
                .entries
                .into_iter()
                .find(|entry| entry.id == id)
                .ok_or("Saved scan is unavailable.")?;
            format!("scan-{}.hahakigamiscan", entry.id)
        };
        let Some(file) = app
            .dialog()
            .file()
            .add_filter("Hahakigami scan", &["hahakigamiscan"])
            .set_file_name(filename)
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let path = file.into_path().map_err(|e| e.to_string())?;
        let guard = library.lock().unwrap();
        let library = guard.as_ref().ok_or("Saved scan library is unavailable.")?;
        progress(&on_progress, "export", "writing", 0);
        let mut last_progress = Instant::now();
        library.export_scan(&id, &path, |bytes| {
            if last_progress.elapsed() >= std::time::Duration::from_millis(100) {
                progress(&on_progress, "export", "writing", bytes);
                last_progress = Instant::now();
            }
        })?;
        progress(&on_progress, "export", "done", 0);
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn compare_saved_scans(
    state: State<'_, AppState>,
    before_id: String,
    after_id: String,
    on_progress: Channel<LibraryProgress>,
) -> Result<comparison::Comparison, String> {
    require_saved_scans()?;
    if before_id == after_id {
        return Err("Choose two different saved scans to compare.".into());
    }
    let activity = state.scans.idle_exclusive(
        &state.activity,
        "Cancel the running scan before comparing saved results.",
    )?;
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = activity;
        let guard = library.lock().unwrap();
        let library = guard.as_ref().ok_or("Saved scan library is unavailable.")?;
        // Keep at most one decoded archive alongside the displayed session.
        // The first tree is reduced to a path/metadata index before loading the second.
        let baseline = {
            progress(&on_progress, "compare", "readingBefore", 0);
            let mut last_progress = Instant::now();
            let (snapshot, _) = library.load(&before_id, |bytes| {
                if last_progress.elapsed() >= std::time::Duration::from_millis(100) {
                    progress(&on_progress, "compare", "readingBefore", bytes);
                    last_progress = Instant::now();
                }
            })?;
            comparison::Baseline::from_snapshot(&snapshot)?
        };
        progress(&on_progress, "compare", "readingAfter", 0);
        let mut last_progress = Instant::now();
        let (snapshot, _) = library.load(&after_id, |bytes| {
            if last_progress.elapsed() >= std::time::Duration::from_millis(100) {
                progress(&on_progress, "compare", "readingAfter", bytes);
                last_progress = Instant::now();
            }
        })?;
        progress(&on_progress, "compare", "comparing", 0);
        let result = baseline.compare(&snapshot, before_id, after_id)?;
        // Comparison never replaces a session, creates file authority, or changes startup selection.
        progress(&on_progress, "compare", "done", 0);
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
// Compares an in-memory live scan with a saved scan; nothing is written to the library.
#[tauri::command]
async fn compare_current_scan(
    state: State<'_, AppState>,
    scan_id: String,
    saved_id: String,
    current_after: bool,
    on_progress: Channel<LibraryProgress>,
) -> Result<comparison::Comparison, String> {
    require_saved_scans()?;
    // Saved-scan comparison cannot overlap any running scan, not only the compared one.
    let activity = state.scans.idle_exclusive(
        &state.activity,
        "Cancel the running scan before comparing saved results.",
    )?;
    let s = state.scans.find(&scan_id)?;
    let library = state.library.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = activity;
        let held = s.hold(Need::LiveCompleted)?;
        let current = held.completed_snapshot()?;
        let guard = library.lock().unwrap();
        let library = guard.as_ref().ok_or("Saved scan library is unavailable.")?;
        let load_saved = |phase: &'static str| {
            progress(&on_progress, "compare", phase, 0);
            let mut last_progress = Instant::now();
            library
                .load(&saved_id, |bytes| {
                    if last_progress.elapsed() >= std::time::Duration::from_millis(100) {
                        progress(&on_progress, "compare", phase, bytes);
                        last_progress = Instant::now();
                    }
                })
                .map(|(snapshot, _)| snapshot)
        };
        // Keep at most one decoded archive alongside the displayed session, as for saved scans.
        let result = if current_after {
            let baseline = comparison::Baseline::from_snapshot(&load_saved("readingBefore")?)?;
            progress(&on_progress, "compare", "readingAfter", 0);
            progress(&on_progress, "compare", "comparing", 0);
            baseline.compare(&current, saved_id.clone(), scan_id)?
        } else {
            progress(&on_progress, "compare", "readingBefore", 0);
            let baseline = comparison::Baseline::from_snapshot(&current)?;
            let after = load_saved("readingAfter")?;
            progress(&on_progress, "compare", "comparing", 0);
            baseline.compare(&after, scan_id, saved_id.clone())?
        };
        progress(&on_progress, "compare", "done", 0);
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanStart {
    scan_id: String,
    target: platform::Target,
}
#[tauri::command]
async fn rescan_saved_scan<R: tauri::Runtime>(
    state: State<'_, AppState>,
    scan_id: String,
    on_event: Channel<Summary>,
    app: tauri::AppHandle<R>,
) -> Result<ScanStart, String> {
    require_saved_scans()?;
    let _activity = Activity::begin(&state.activity)?;
    quick_look::close(app.clone(), state.preview.clone()).await?;
    state.access_setup.lock().unwrap().require_scan_allowed()?;
    let s = state.scans.find(&scan_id)?;
    if s.origin != SessionOrigin::Saved {
        return Err("Choose a saved scan to rescan its location.".into());
    }
    let path = s.store.read().unwrap().root.clone();
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();
    // Historical identities never authorize current filesystem access. Register
    // the location afresh and let the normal held-root scanner validate it.
    let target = register(&state, platform::target(&path, name), path)?;
    let scan_id = start_scan_impl(&state, Some(target.id.clone()), None, on_event)?;
    Ok(ScanStart { scan_id, target })
}
#[tauri::command]
async fn start_scan<R: tauri::Runtime>(
    state: State<'_, AppState>,
    target_id: Option<String>,
    path: Option<String>,
    on_event: Channel<Summary>,
    app: tauri::AppHandle<R>,
) -> Result<String, String> {
    // Starts share the gate, so scans on different slots can begin at the same time.
    let _activity = Activity::begin_shared(&state.activity)?;
    quick_look::close(app.clone(), state.preview.clone()).await?;
    start_scan_impl(&state, target_id, path, on_event)
}
#[cfg(target_os = "macos")]
fn local_volume(root: &disk_scan::ScanRoot) -> Option<PathBuf> {
    use std::os::{
        fd::{AsFd, AsRawFd},
        unix::ffi::OsStrExt,
    };
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatfs(root.as_fd().as_raw_fd(), &mut stat) } != 0
        || stat.f_flags & libc::MNT_LOCAL as u32 == 0
    {
        return None;
    }
    let length = stat.f_mntonname.iter().position(|byte| *byte == 0)?;
    let bytes =
        unsafe { std::slice::from_raw_parts(stat.f_mntonname.as_ptr().cast::<u8>(), length) };
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
    path.is_absolute().then_some(path)
}
#[cfg(not(target_os = "macos"))]
fn local_volume(_: &disk_scan::ScanRoot) -> Option<PathBuf> {
    None
}
#[cfg(test)]
fn refresh_disk_baseline(
    load: impl FnOnce() -> Result<cache::CachedScan, String>,
    replay: impl FnOnce(u64) -> Result<events::Changes, String>,
    epoch_valid: impl FnOnce() -> bool,
) -> Result<(cache::CachedScan, events::Changes), String> {
    let cached = load()?;
    let dirty_paths = replay(cached.token.cursor)?.resolve(&cached.snapshot)?;
    if !epoch_valid() {
        return Err("Filesystem event journal changed while loading the cache".into());
    }
    Ok((cached, dirty_paths))
}
// Benchmark-only spans; normal builds evaluate the expression without timers or storage.
macro_rules! scan_span {
    ($session:expr, $phase:literal, $operation:expr) => {{
        #[cfg(feature = "performance-benchmark")]
        let began = Instant::now();
        let result = $operation;
        #[cfg(feature = "performance-benchmark")]
        performance::record($session, $phase, began);
        result
    }};
}
struct PreparedRescan {
    snapshot: Snapshot,
    stats: disk_scan::ReuseStats,
}
fn choose_prepared(
    changes: &events::Changes,
    prepared: Option<PreparedRescan>,
) -> Option<PreparedRescan> {
    // Call only after resolving external inode events against the baseline.
    if changes.paths.is_empty() && changes.directories.is_empty() {
        prepared
    } else {
        None
    }
}
// Cached hard-link scopes reconcile once, after final alias resolution. Even an
// external directory event can dirty every link group, making speculation wasteful.
// Other scopes keep private reconstruction while the live journal finishes; dirty
// history discards it before reconciliation against the original validated baseline.
fn prepare_rescan(
    s: &Session,
    root: &disk_scan::ScanRoot,
    cached: cache::CachedScan,
) -> (cache::CachedScan, Option<PreparedRescan>) {
    if !cached.snapshot.incremental_accounting_eligible()
        || cached.snapshot.iter_nodes().any(|node| node.hard_link)
    {
        return (cached, None);
    }
    let Ok(authority) = root.reopen() else {
        return (cached, None);
    };
    let store = Arc::new(RwLock::new(Snapshot::empty(cached.snapshot.root.clone())));
    let cancel = Arc::new(AtomicBool::new(s.cancel.load(Ordering::Relaxed)));
    let result = scan_span!(
        s,
        "speculative_reconcile",
        disk_scan::scan_live_incremental_authorized(
            authority,
            disk_scan::ScanOptions {
                workers: s.workers,
                qos: s.qos,
                descriptor_limit: s.descriptor_limit,
                ..Default::default()
            },
            cancel.clone(),
            store.clone(),
            |_| {
                if s.cancel.load(Ordering::Relaxed) {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            disk_scan::IncrementalSeed {
                snapshot: cached.snapshot.clone(),
                dirty_paths: vec![],
                dirty_directories: vec![]
            },
        )
    );
    let prepared = result
        .ok()
        .filter(|stats| stats.fallback_reason.is_none())
        .and_then(|stats| {
            let snapshot = Arc::try_unwrap(store).ok()?.into_inner().ok()?;
            (snapshot.state == ScanState::Completed
                && !snapshot.stats.incomplete
                && snapshot.incremental_accounting_eligible()
                && !s.cancel.load(Ordering::Relaxed))
            .then_some(PreparedRescan { snapshot, stats })
        });
    (cached, prepared)
}
// A scan thread that panicked while holding the cache lock poisons it; the cache's files stay
// consistent (temp file, then rename; leftovers are swept), so other scans keep using it.
fn lock_cache(cache: &Mutex<Option<cache::Cache>>) -> MutexGuard<'_, Option<cache::Cache>> {
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}
fn mark_failed(snapshot: &mut Snapshot, message: String) {
    if snapshot.state != ScanState::Failed {
        snapshot.state = ScanState::Failed;
        snapshot.stats.incomplete = true;
        snapshot.stats.errors += 1;
        let path = snapshot.root.clone();
        snapshot.errors.push(disk_scan::ScanIssue {
            error_kind: None,
            raw_os_error: None,
            path,
            message,
        });
    }
}
// A panic in a scan thread would leave its session running forever and every "no scan at all" gate
// closed. Ends it instead: Failed if it had not finished (a finished one keeps its results).
fn end_panicked_scan(s: &Session, channel: &impl Progress) {
    // The panic may have poisoned these; the session is ending and they hold plain data.
    s.store.clear_poison();
    s.scan_details.clear_poison();
    let finished = {
        let mut snapshot = s.store.write().unwrap();
        let running = snapshot.state == ScanState::Running;
        if running {
            mark_failed(&mut snapshot, "The scan stopped unexpectedly.".into());
        }
        !running
    };
    {
        let mut details = s.scan_details.lock().unwrap();
        details.phase = "done".into();
        if finished {
            details.cache_warning.get_or_insert(
                "Could not keep the rescan cache: the scan stopped unexpectedly.".into(),
            );
        }
    }
    s.finishing.store(false, Ordering::Release);
    channel.report(s.summary());
}
/// The volume of each region a scan walked, read once as the scan ends (never from code the UI polls), keyed by
/// the region's root node. A region starts at the scan root, at a device change (a mount) and at a firmlink: a scan
/// of "/" spans the System and Data volumes under one `st_dev`, so devices alone would give Data items the System
/// volume. Excluded nodes are skipped, so no mount the scan did not enter is touched; a region whose volume cannot
/// be read is left out, and its items then fail closed against the enclosing region's volume.
fn scan_volumes(snapshot: &Snapshot) -> HashMap<u32, safety::Volume> {
    volume_regions(snapshot, &disk_scan::firmlink_paths())
        .into_iter()
        .filter_map(|id| {
            let path = snapshot.path(id).ok()?;
            Some((
                id,
                safety::Volume::read(platform::VolumeOf::Path(&path)).ok()?,
            ))
        })
        .collect()
}
fn volume_regions(snapshot: &Snapshot, firmlinks: &[PathBuf]) -> Vec<u32> {
    let mut regions: Vec<u32> = snapshot
        .iter_nodes()
        .filter(|node| {
            !node.excluded
                && node.parent.is_none_or(|parent| {
                    snapshot
                        .node(parent)
                        .is_some_and(|p| p.identity.device != node.identity.device)
                })
        })
        .map(|node| node.id)
        .collect();
    regions.extend(
        firmlinks
            .iter()
            .filter_map(|path| safety::node_at(snapshot, path))
            .filter(|&id| snapshot.node(id).is_some_and(|node| !node.excluded)),
    );
    regions
}
/// Where a scan run reports its summary: on every progress step and once more, with phase "done", as it ends.
/// `report` returns false once nobody listens any more, which cancels the scan.
trait Progress: Clone + Send + Sync + 'static {
    fn report(&self, summary: Summary) -> bool;
}
/// The app's adapter: the frontend channel of the start.
impl Progress for Channel<Summary> {
    fn report(&self, summary: Summary) -> bool {
        self.send(summary).is_ok()
    }
}
fn run_scan(
    s: Arc<Session>,
    root: disk_scan::ScanRoot,
    previous: Option<cache::CachedScan>,
    cache: Arc<Mutex<Option<cache::Cache>>>,
    channel: impl Progress,
    grant: budget::Grant,
) {
    let pipeline_started = Instant::now();
    let configured = lock_cache(&cache)
        .as_ref()
        .is_some_and(|c| !c.contains_storage(root.path()));
    let volume = local_volume(&root);
    let epoch = if configured && volume.is_some() && root.path() != std::path::Path::new("/") {
        events::boot_session()
            .zip(events::journal_identity(root.path()))
            .map(|(boot, journal)| format!("{boot}:{journal}"))
    } else {
        None
    };
    // Start cursor precedes replay and enumeration. Events during either operation
    // remain in the next scan's replay, rather than advancing past unseen writes.
    let token = epoch.as_ref().and_then(|epoch| {
        events::current_id()
            .filter(|id| *id != 0)
            .map(|cursor| cache::Token {
                epoch: epoch.clone(),
                cursor,
                full_scan_at_ms: s.started_at_ms,
            })
    });
    channel.report(s.summary());
    let mut fallback = None;
    let mut seed = None;
    let mut full_scan_at_ms = s.started_at_ms;
    let mut prior_cache_token = None;
    let mut unchanged_metadata = false;
    let mut prepared_rescan = None;
    if let Some(token) = &token {
        enum Baseline {
            Memory(Box<cache::CachedScan>),
            Disk(cache::Candidate),
        }
        let cached = match previous {
            Some(previous)
                if previous.token.epoch == token.epoch
                    && cache::eligible(&previous.snapshot)
                    && cache::recent(&previous.token, s.started_at_ms, now_ms())
                    && previous
                        .snapshot
                        .node(0)
                        .is_some_and(|n| &n.identity == root.identity()) =>
            {
                Some(Baseline::Memory(Box::new(previous)))
            }
            _ => match scan_span!(
                &s,
                "cache_probe",
                lock_cache(&cache).as_ref().unwrap().probe(
                    root.path(),
                    root.identity(),
                    &token.epoch,
                )
            ) {
                Ok(result) => result.map(Baseline::Disk),
                Err(e) => {
                    fallback = Some(format!("Rescan cache unavailable: {e}"));
                    None
                }
            },
        };
        if let Some(cached) = cached {
            let cursor = match &cached {
                Baseline::Memory(cached) => cached.token.cursor,
                Baseline::Disk(candidate) => candidate.token.cursor,
            };
            let loaded = scan_span!(
                &s,
                "journal_replay",
                events::replay_with_load_on_volume(
                    root.path(),
                    volume.as_ref().unwrap(),
                    cursor,
                    &s.cancel,
                    || {
                        let cached = match cached {
                            Baseline::Memory(cached) => *cached,
                            Baseline::Disk(candidate) => {
                                // Only the record check holds the cache lock. The archive decodes
                                // without it, so another scan can publish meanwhile.
                                let pending = lock_cache(&cache).as_ref().unwrap().begin_load(
                                    candidate,
                                    root.path(),
                                    root.identity(),
                                    &token.epoch,
                                )?;
                                scan_span!(
                                    &s,
                                    "cache_decode_validate",
                                    pending.decode(root.path(), root.identity())
                                )?
                            }
                        };
                        if !cache::eligible(&cached.snapshot) {
                            return Err(
                                "Clone sharing or uncertain accounting requires a full scan".into(),
                            );
                        }
                        Ok(prepare_rescan(&s, &root, cached))
                    }
                )
            )
            .and_then(|((cached, prepared), changes)| {
                let changes =
                    scan_span!(&s, "alias_resolution", changes.resolve(&cached.snapshot))?;
                Ok((cached, prepared, changes))
            });
            match loaded {
                Ok((cached, prepared, changes))
                    if events::boot_session()
                        .zip(events::journal_identity(root.path()))
                        .is_some_and(|(boot, journal)| {
                            format!("{boot}:{journal}") == token.epoch
                        }) =>
                {
                    full_scan_at_ms = cached.token.full_scan_at_ms;
                    prior_cache_token = Some(cached.token);
                    if !s.cancel.load(Ordering::Acquire) {
                        prepared_rescan = choose_prepared(&changes, prepared);
                    }
                    seed = Some(disk_scan::IncrementalSeed {
                        snapshot: cached.snapshot,
                        dirty_paths: changes.paths,
                        dirty_directories: changes.directories,
                    });
                }
                Ok(_) => fallback = Some("Filesystem event journal changed".into()),
                Err(error) => fallback = Some(format!("Rescan cache unavailable: {error}")),
            }
        } else if fallback.is_none() {
            fallback = Some("No compatible recent clean scan is cached".into());
        }
    } else {
        fallback =
            Some("This location requires a full scan or event history is unavailable".into());
    }
    {
        let mut details = s.scan_details.lock().unwrap();
        details.phase = "scanning".into();
        details.mode = if seed.is_some() {
            "incremental"
        } else {
            "full"
        }
        .into();
        details.fallback_reason = fallback;
    }
    let options = disk_scan::ScanOptions {
        workers: s.workers,
        qos: s.qos,
        descriptor_limit: s.descriptor_limit,
        ..Default::default()
    };
    let callback = |_| {
        if !channel.report(s.summary()) {
            s.cancel.store(true, Ordering::Relaxed);
        }
    };
    #[cfg(feature = "performance-benchmark")]
    performance::record(&s, "cache_checks_total", pipeline_started);
    #[cfg(feature = "performance-benchmark")]
    let scanning_started = Instant::now();
    let result = if let Some(prepared) =
        prepared_rescan.filter(|_| !s.cancel.load(Ordering::Acquire))
    {
        drop(seed);
        *s.store.write().unwrap() = prepared.snapshot;
        callback(disk_scan::Progress {
            entries: 0,
            directories: 0,
            errors: 0,
            excluded: 0,
            elapsed_ms: 0,
        });
        Ok(prepared.stats)
    } else if let Some(seed) = seed {
        disk_scan::scan_live_incremental_authorized_with_validation_workers(
            root,
            options,
            s.cancel.clone(),
            s.store.clone(),
            callback,
            seed,
            s.cached_validation_workers,
            |workers| s.scan_details.lock().unwrap().actual_workers = Some(workers),
        )
    } else {
        disk_scan::scan_live_authorized(root, options, s.cancel.clone(), s.store.clone(), callback)
            .map(|_| disk_scan::ReuseStats {
                rescanned_directories: s
                    .store
                    .read()
                    .unwrap()
                    .worker_stats
                    .iter()
                    .map(|row| row.directories)
                    .sum(),
                ..Default::default()
            })
    };
    #[cfg(feature = "performance-benchmark")]
    performance::record(&s, "scan_and_aggregation", scanning_started);
    match result {
        Ok(stats) => {
            unchanged_metadata = stats.unchanged_metadata;
            let mut details = s.scan_details.lock().unwrap();
            details.reused_entries = stats.reused_entries;
            #[cfg(feature = "performance-benchmark")]
            {
                details.reuse_storage = stats.storage;
            }
            details.rescanned_directories = stats.rescanned_directories;
            if let Some(reason) = stats.fallback_reason {
                details.mode = "full".into();
                details.fallback_reason = Some(reason);
                full_scan_at_ms = s.started_at_ms;
            }
        }
        Err(e) => mark_failed(&mut s.store.write().unwrap(), e.to_string()),
    }
    if let Some(mut token) = token {
        let snapshot = s.store.read().unwrap();
        if cache::eligible(&snapshot) && !s.cancel.load(Ordering::Acquire) {
            token.full_scan_at_ms = full_scan_at_ms;
            s.scan_details.lock().unwrap().phase = "caching".into();
            channel.report(s.summary());
            let completed_at_ms = now_ms();
            s.completed_at_ms.store(completed_at_ms, Ordering::Release);
            let metadata = library::ScanMetadata {
                started_at_ms: s.started_at_ms,
                completed_at_ms,
                workers: s.actual_workers(),
                worker_mode: "auto".into(),
                worker_source: s.worker_source.clone(),
                qos: s.qos,
            };
            #[cfg(feature = "performance-benchmark")]
            let caching_started = Instant::now();
            let publication = (|| {
                let cache = lock_cache(&cache);
                let cache = cache.as_ref().unwrap();
                if unchanged_metadata {
                    if let Some(previous) = prior_cache_token.as_ref() {
                        if cache.refresh_unchanged(
                            &snapshot.root,
                            &snapshot.node(0).ok_or("Missing root")?.identity,
                            previous,
                            &token,
                        )? {
                            return Ok(());
                        }
                    }
                }
                cache.save(&snapshot, &metadata, &token)
            })();
            if let Err(error) = publication {
                s.scan_details.lock().unwrap().cache_warning =
                    Some(format!("Could not keep the rescan cache: {error}"));
            }
            #[cfg(feature = "performance-benchmark")]
            performance::record(&s, "cache_publication", caching_started);
            *s.cache_token.lock().unwrap() = Some(token);
        }
    }
    {
        let snapshot = s.store.read().unwrap();
        if snapshot.accounting.shared || !snapshot.accounting.checks_complete {
            s.scan_details.lock().unwrap().fallback_reason = Some(
                "Clone sharing or uncertain accounting requires a full scan; incremental reuse is disabled for these results.".into()
            );
        }
    }
    {
        let snapshot = s.store.read().unwrap();
        if snapshot.state == ScanState::Completed {
            let _ = s.volumes.set(scan_volumes(&snapshot));
        }
    }
    s.store.write().unwrap().stats.elapsed_ms = pipeline_started.elapsed().as_millis() as u64;
    s.scan_details.lock().unwrap().phase = "done".into();
    // Give the share back before the scan reads as ended, so a start on the next event finds it free.
    drop(grant);
    s.finishing.store(false, Ordering::Release);
    channel.report(s.summary());
}
fn start_scan_impl(
    state: &AppState,
    target_id: Option<String>,
    path: Option<String>,
    on_event: impl Progress,
) -> Result<String, String> {
    // Hold through session registration, so restart cannot race a scan.
    let access = state.access_setup.lock().unwrap();
    access.require_scan_allowed()?;
    let scheduling_conditions = platform::scheduling_conditions();
    let scan_settings = state
        .scan_settings
        .lock()
        .unwrap()
        .automatic_info_for_conditions(settings::AutoScope::Volume, scheduling_conditions);
    let qos = scan_settings.qos;
    let (resolved_id, root, disk) = {
        let targets = state.targets.lock().unwrap();
        if let Some(id) = target_id {
            let (target, root) = targets
                .get(&id)
                .ok_or("Choose a registered folder or volume first.")?;
            (id, root.clone(), target.volume)
        } else if let Some(path) = path {
            let matches: Vec<_> = targets
                .iter()
                .filter(|(_, (target, _))| target.path == path)
                .collect();
            if matches.len() != 1 {
                return Err("Choose the target by its registered ID.".into());
            }
            (
                matches[0].0.clone(),
                matches[0].1 .1.clone(),
                matches[0].1 .0.volume,
            )
        } else {
            return Err("Choose a registered folder or volume first.".into());
        }
    };
    let (expected, root_ancestors) = {
        let guards = state.target_guards.lock().unwrap();
        let guard = guards
            .get(&resolved_id)
            .ok_or("Choose the scan target again.")?;
        validate_target(guard)?;
        (
            disk_scan::Identity {
                device: guard.identity.device,
                inode: guard.identity.inode,
                kind: NodeKind::Directory,
            },
            guard.ancestors.clone(),
        )
    };
    // Bind the selected object to a held no-follow descriptor BEFORE the worker
    // handoff. A later rename can never authorize a different root object.
    let authorized_root =
        disk_scan::ScanRoot::open(root.clone(), expected).map_err(|e| e.to_string())?;
    let auto_scope = if platform::is_volume_root(&authorized_root) {
        settings::AutoScope::Volume
    } else {
        settings::AutoScope::Folder
    };
    let (workers, cached_validation_workers, worker_source) = state
        .scan_settings
        .lock()
        .unwrap()
        .resolve_scan_workers(auto_scope, scheduling_conditions);
    // Only this scan's own slot is replaced and used as the rescan baseline; other disks keep theirs.
    let key = if disk {
        Slot::Disk(root.clone())
    } else {
        Slot::Folder
    };
    let slots::Started {
        session: s,
        previous,
        grant,
    } = state.scans.replace(
        key,
        Request {
            root,
            root_ancestors,
            workers,
            cached_validation_workers,
            worker_mode: scan_settings.worker_mode,
            worker_source: worker_source.into(),
            qos,
        },
    )?;
    let cache = state.rescan_cache.clone();
    let id = s.id.clone();
    std::thread::spawn(move || {
        let (session, events) = (s.clone(), on_event.clone());
        let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_scan(s, authorized_root, previous, cache, on_event, grant)
        }));
        // The grant went back as the closure unwound.
        if ended.is_err() {
            end_panicked_scan(&session, &events);
        }
    });
    Ok(id)
}
#[tauri::command]
fn cancel_scan(state: State<'_, AppState>, scan_id: String) -> Result<(), String> {
    let s = state.scans.find(&scan_id)?;
    if s.scan_details.lock().unwrap().phase == "caching" {
        return Err("The scan is finishing its rescan cache. Wait for completion.".into());
    }
    s.cancel.store(true, Ordering::Relaxed);
    Ok(())
}
/// Drops a live scan whose disk was ejected: stops it if it is running, then frees its slot.
/// A scan that has not ended yet keeps counting as running until it does.
#[tauri::command]
fn discard_scan(state: State<'_, AppState>, scan_id: String) {
    state.scans.discard(&scan_id);
}
#[tauri::command]
async fn scan_summary(state: State<'_, AppState>, scan_id: String) -> Result<Summary, String> {
    Ok(state.scans.find(&scan_id)?.summary())
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum IssueKind {
    Errors,
    Restricted,
    Other,
    Excluded,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IssueView {
    category: &'static str,
    path: String,
    message: String,
    error_kind: Option<String>,
    raw_os_error: Option<i32>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IssuePage {
    scan_id: String,
    // Details belong to the scan root; paths are display-only, never action inputs.
    node_id: u32,
    kind: IssueKind,
    total: u64,
    retained_total: usize,
    offset: usize,
    items: Vec<IssueView>,
}
#[tauri::command]
async fn get_scan_issues(
    state: State<'_, AppState>,
    scan_id: String,
    kind: IssueKind,
    offset: usize,
    limit: usize,
) -> Result<IssuePage, String> {
    if !(1..=100).contains(&limit) {
        return Err("Request between 1 and 100 scan details.".into());
    }
    let s = state.scans.find(&scan_id)?;
    let snapshot = s.store.read().unwrap();
    let (details, total) = match kind {
        IssueKind::Errors => (&snapshot.errors, snapshot.stats.errors),
        IssueKind::Restricted => (&snapshot.errors, snapshot.stats.restricted_system_issues),
        IssueKind::Other => (
            &snapshot.errors,
            snapshot
                .stats
                .errors
                .saturating_sub(snapshot.stats.restricted_system_issues),
        ),
        IssueKind::Excluded => (&snapshot.exclusions, snapshot.stats.excluded),
    };
    let included = |issue: &disk_scan::ScanIssue| match kind {
        IssueKind::Restricted => disk_scan::is_system_restriction(issue),
        IssueKind::Other => !disk_scan::is_system_restriction(issue),
        _ => true,
    };
    let retained_total = match kind {
        IssueKind::Restricted | IssueKind::Other => {
            details.iter().filter(|issue| included(issue)).count()
        }
        _ => details.len(),
    };
    Ok(IssuePage {
        scan_id,
        node_id: 0,
        kind,
        total,
        retained_total,
        offset,
        items: details
            .iter()
            .filter(|issue| included(issue))
            .skip(offset)
            .take(limit)
            .map(|issue| IssueView {
                category: if kind == IssueKind::Excluded {
                    "excluded"
                } else if disk_scan::is_system_restriction(issue) {
                    "restrictedSystem"
                } else {
                    "other"
                },
                path: issue.path.to_string_lossy().into_owned(),
                message: issue.message.clone(),
                error_kind: issue.error_kind.clone(),
                raw_os_error: issue.raw_os_error,
            })
            .collect(),
    })
}

#[tauri::command]
async fn get_node(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u32,
) -> Result<NodeView, String> {
    let s = state.scans.find(&scan_id)?;
    let snapshot = s.store.read().unwrap();
    let issues = issue_index(&s, &snapshot);
    let n = snapshot.node(node_id).ok_or("Node not yet available")?;
    view(&n, &issues.1)
}
#[derive(Serialize)]
struct Page {
    items: Vec<NodeView>,
    total: usize,
}
fn allocated(metric: &str) -> Result<bool, String> {
    match metric {
        "allocated" => Ok(true),
        "logical" => Ok(false),
        _ => Err("Unknown size metric".into()),
    }
}
fn size(n: &NodeRef<'_>, allocated: bool) -> u64 {
    if allocated {
        n.allocated_size
    } else {
        n.logical_size
    }
}
struct MapSortKey<'a> {
    selected_size: u64,
    display_name: Cow<'a, str>,
}
#[cfg(any(test, feature = "performance-benchmark"))]
#[derive(Clone, Copy, Default, Serialize)]
struct MapSortScratch {
    key_vector: disk_scan::BufferStorage,
    owned_lossy_names: disk_scan::BufferStorage,
    temporary_ids: disk_scan::BufferStorage,
}
#[cfg(any(test, feature = "performance-benchmark"))]
impl MapSortScratch {
    fn from_keys(keys: &Vec<MapSortKey<'_>>) -> Self {
        let mut owned_lossy_names = disk_scan::BufferStorage::default();
        for key in keys {
            if let Cow::Owned(name) = &key.display_name {
                owned_lossy_names.length_bytes += name.len();
                owned_lossy_names.capacity_bytes += name.capacity();
            }
        }
        Self {
            key_vector: disk_scan::BufferStorage {
                length_bytes: keys.len() * std::mem::size_of::<MapSortKey<'_>>(),
                capacity_bytes: keys.capacity() * std::mem::size_of::<MapSortKey<'_>>(),
            },
            owned_lossy_names,
            temporary_ids: disk_scan::BufferStorage::default(),
        }
    }
    fn capacity_bytes(self) -> usize {
        self.key_vector.capacity_bytes
            + self.owned_lossy_names.capacity_bytes
            + self.temporary_ids.capacity_bytes
    }
}
struct MapChildOrder {
    ids: Vec<u32>,
    #[cfg(any(test, feature = "performance-benchmark"))]
    scratch: MapSortScratch,
}
fn sort_all_map_children(
    snapshot: &Snapshot,
    children: &[u32],
    allocated: bool,
) -> Result<MapChildOrder, String> {
    let mut keys = Vec::with_capacity(children.len());
    let mut positions = Vec::with_capacity(children.len());
    for (position, &id) in children.iter().enumerate() {
        let node = snapshot.node(id).ok_or("Unknown map child")?;
        positions.push(u32::try_from(position).map_err(|_| "Too many map children")?);
        keys.push(MapSortKey {
            selected_size: size(&node, allocated),
            display_name: snapshot.display_name(id)?,
        });
    }
    #[cfg(any(test, feature = "performance-benchmark"))]
    let scratch = MapSortScratch::from_keys(&keys);
    // Keep the original u32 sorting representation and comparison outcomes.
    // Position indirection preserves the old unstable permutation for equal
    // keys, including repeated IDs; no ID or directory-kind tie is added.
    positions.sort_unstable_by(|&a, &b| {
        keys[b as usize]
            .selected_size
            .cmp(&keys[a as usize].selected_size)
            .then_with(|| {
                keys[a as usize]
                    .display_name
                    .cmp(&keys[b as usize].display_name)
            })
    });
    for position in &mut positions {
        *position = children[*position as usize];
    }
    // Borrowed keys and any owned lossy strings drop here; caches retain IDs.
    Ok(MapChildOrder {
        ids: positions,
        #[cfg(any(test, feature = "performance-benchmark"))]
        scratch,
    })
}
// Map queries omit zero-sized children. Keep those entries out of retained caches
// and usually out of sort scratch as well. Unstable sorting can permute equal
// positive keys differently when zeros are removed, so ties use the original
// full ordering before filtering. This preserves historical aggregate cutoffs.
fn sort_map_children(
    snapshot: &Snapshot,
    children: &[u32],
    allocated: bool,
) -> Result<MapChildOrder, String> {
    let mut positive_count = 0;
    for &id in children {
        let node = snapshot.node(id).ok_or("Unknown map child")?;
        positive_count += usize::from(size(&node, allocated) > 0);
    }
    if positive_count == children.len() {
        return sort_all_map_children(snapshot, children, allocated);
    }
    let mut keys = Vec::with_capacity(positive_count);
    let mut ids = Vec::with_capacity(positive_count);
    for &id in children {
        let node = snapshot.node(id).ok_or("Unknown map child")?;
        let selected_size = size(&node, allocated);
        if selected_size > 0 {
            ids.push(id);
            keys.push(MapSortKey {
                selected_size,
                display_name: snapshot.display_name(id)?,
            });
        }
    }
    let mut positions = Vec::with_capacity(positive_count);
    for position in 0..positive_count {
        positions.push(u32::try_from(position).map_err(|_| "Too many map children")?);
    }
    let compare = |&a: &u32, &b: &u32| {
        let (a, b) = (a as usize, b as usize);
        keys[b]
            .selected_size
            .cmp(&keys[a].selected_size)
            .then_with(|| keys[a].display_name.cmp(&keys[b].display_name))
    };
    positions.sort_unstable_by(compare);
    #[cfg(any(test, feature = "performance-benchmark"))]
    let scratch = MapSortScratch {
        temporary_ids: disk_scan::BufferStorage {
            length_bytes: ids.len() * std::mem::size_of::<u32>(),
            capacity_bytes: ids.capacity() * std::mem::size_of::<u32>(),
        },
        ..MapSortScratch::from_keys(&keys)
    };
    if positions
        .windows(2)
        .any(|pair| compare(&pair[0], &pair[1]).is_eq())
    {
        // Drop sparse temporary buffers before allocating legacy tie scratch.
        drop(positions);
        drop(keys);
        drop(ids);
        let mut order = sort_all_map_children(snapshot, children, allocated)?;
        order
            .ids
            .retain(|&id| size(&snapshot.node(id).unwrap(), allocated) > 0);
        // A filtered Vec otherwise retains the full directory's capacity.
        order.ids.shrink_to_fit();
        #[cfg(any(test, feature = "performance-benchmark"))]
        if scratch.capacity_bytes() > order.scratch.capacity_bytes() {
            order.scratch = scratch;
        }
        return Ok(order);
    }
    for position in &mut positions {
        *position = ids[*position as usize];
    }
    Ok(MapChildOrder {
        ids: positions,
        #[cfg(any(test, feature = "performance-benchmark"))]
        scratch,
    })
}
fn sorted_children(
    s: &Session,
    snapshot: &Snapshot,
    id: u32,
    allocated: bool,
) -> Result<Vec<u32>, String> {
    let n = snapshot.node(id).ok_or("Unknown node")?;
    let key = (id, allocated);
    let mut cache = s.sorts.lock().unwrap();
    if snapshot.state != ScanState::Running {
        if let Some(ids) = cache.get(&key) {
            return Ok(ids.clone());
        }
    }
    let order = sort_map_children(snapshot, n.children(), allocated)?;
    let ids = order.ids;
    if snapshot.state != ScanState::Running {
        cache.insert(key, ids.clone());
    }
    drop(cache);
    #[cfg(feature = "performance-benchmark")]
    {
        let mut details = s.scan_details.lock().unwrap();
        // One actual sort sample with the largest combined requested capacity;
        // category maxima are not summed across unrelated/concurrent queries.
        if details
            .map_sort_scratch
            .is_none_or(|sample| order.scratch.capacity_bytes() > sample.capacity_bytes())
        {
            details.map_sort_scratch = Some(order.scratch);
        }
    }

    Ok(ids)
}
async fn list_page(
    s: Arc<Session>,
    select: impl FnOnce(&Session, &Snapshot) -> Result<(Vec<u32>, usize), String> + Send + 'static,
) -> Result<Page, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = s.store.read().unwrap();
        s.check_valid()?;
        let (ids, total) = select(&s, &snapshot)?;
        let issues = issue_index(&s, &snapshot);
        let page = Page {
            total,
            items: ids
                .into_iter()
                .map(|id| view(&snapshot.node(id).ok_or("Unknown node")?, &issues.1))
                .collect::<Result<Vec<_>, String>>()?,
        };
        s.check_valid()?;
        Ok(page)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn get_children(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u32,
    offset: usize,
    limit: usize,
    metric: String,
    sort: Option<String>,
) -> Result<Page, String> {
    let s = state.scans.find(&scan_id)?;
    let allocated = allocated(&metric)?;
    let order = sorting::Order::parse(sort.as_deref())?;
    list_page(s, move |s, snapshot| {
        s.list_folder
            .lock()
            .unwrap()
            .page(snapshot, node_id, order, allocated, offset, limit)
    })
    .await
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MapData {
    nodes: Vec<NodeView>,
    root_id: u32,
}
#[tauri::command]
async fn get_map(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u32,
    metric: String,
    limit: usize,
    color_mode: Option<String>,
) -> Result<MapData, String> {
    let s = state.scans.find(&scan_id)?;
    match color_mode.as_deref() {
        None | Some("folder") => {}
        Some("fileType") => {
            let a = allocated(&metric)?;
            return tauri::async_runtime::spawn_blocking(move || {
                let snapshot = s.store.read().unwrap();
                s.check_valid()?;
                let issues = issue_index(&s, &snapshot);
                let result = file_type_map::build(&snapshot, node_id, a, limit, &issues.1)?;
                s.check_valid()?;
                Ok(result)
            })
            .await
            .map_err(|e| e.to_string())?;
        }
        _ => return Err("Unknown map color mode".into()),
    }
    let snapshot = s.store.read().unwrap();
    let issues = issue_index(&s, &snapshot);
    let a = allocated(&metric)?;
    let root = snapshot.node(node_id).ok_or("Unknown node")?;
    let budget = limit.clamp(2, 5000);
    let mut nodes = vec![view(&root, &issues.1)?];
    let mut queue = VecDeque::from([node_id]);
    let mut aggregate_id = 0x80000000u32;
    while let Some(id) = queue.pop_front() {
        if nodes.len() >= budget {
            break;
        }
        let positive = sorted_children(&s, &snapshot, id, a)?;
        let remaining = budget - nodes.len();
        let take = if positive.len() > remaining {
            remaining.saturating_sub(1)
        } else {
            positive.len()
        };
        for child in positive.iter().take(take) {
            let n = snapshot.node(*child).ok_or("Unknown node")?;
            nodes.push(view(&n, &issues.1)?);
            if n.is_directory {
                queue.push_back(*child);
            }
        }
        if take < positive.len() {
            let (logical, allocated) =
                positive.iter().skip(take).fold((0u64, 0u64), |(l, a), i| {
                    let n = snapshot.node(*i).unwrap();
                    (
                        l.saturating_add(n.logical_size),
                        a.saturating_add(n.allocated_size),
                    )
                });
            nodes.push(NodeView {
                id: aggregate_id,
                parent_id: Some(id),
                name: format!("{} smaller items", positive.len() - take),
                path: String::new(),
                path_key: String::new(),
                kind: "aggregate".into(),
                logical_bytes: logical,
                allocated_bytes: allocated,
                raw_allocated_bytes: 0,
                verified_clone: false,
                clone_deduction_bytes: 0,
                clone_owner: None,
                children_count: 0,
                incomplete: false,
                excluded: false,
                file_type: None,
                file_count: None,
            });
            aggregate_id += 1;
        }
    }
    Ok(MapData {
        nodes,
        root_id: node_id,
    })
}
#[tauri::command]
async fn search_nodes(
    state: State<'_, AppState>,
    scan_id: String,
    query: String,
    offset: usize,
    limit: usize,
    metric: String,
    sort: Option<String>,
) -> Result<Page, String> {
    let s = state.scans.find(&scan_id)?;
    let order = sorting::Order::parse(sort.as_deref())?;
    let query = filters::Query::search(query, metric, order)?;
    list_page(s, move |s, snapshot| {
        s.filters
            .lock()
            .unwrap()
            .page(snapshot, &query, offset, limit)
    })
    .await
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn filter_nodes(
    state: State<'_, AppState>,
    scan_id: String,
    query: String,
    category: String,
    min_bytes: Option<u64>,
    max_bytes: Option<u64>,
    metric: String,
    offset: usize,
    limit: usize,
    sort: Option<String>,
    extension: Option<String>,
    scope_id: Option<u32>,
) -> Result<Page, String> {
    let s = state.scans.find(&scan_id)?;
    let order = sorting::Order::parse(sort.as_deref())?;
    let query = filters::Query::with_order(query, category, min_bytes, max_bytes, metric, order)?
        .with_file_type(extension, scope_id);
    list_page(s, move |s, snapshot| {
        s.filters
            .lock()
            .unwrap()
            .page(snapshot, &query, offset, limit)
    })
    .await
}
#[tauri::command]
async fn get_file_types(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u32,
) -> Result<file_types::Summary, String> {
    let s = state.scans.find(&scan_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = s.store.read().unwrap();
        s.check_valid()?;
        let mut result = file_types::summarize(&snapshot, node_id)?;
        let path = snapshot.node(node_id).ok_or("Unknown scope node")?.path()?;
        let issues = issue_index(&s, &snapshot);
        result.incomplete = issues
            .1
            .get(issues.1.partition_point(|p| p < &path))
            .is_some_and(|p| p.starts_with(&path));
        s.check_valid()?;
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn get_file_type_highlights(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u32,
    extension: String,
    node_ids: Vec<u32>,
) -> Result<Vec<u32>, String> {
    if node_ids.len() > 5000 {
        return Err("At most 5000 visible nodes may be highlighted".into());
    }
    let s = state.scans.find(&scan_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let snapshot = s.store.read().unwrap();
        s.check_valid()?;
        let result = file_types::highlights(&snapshot, node_id, &extension, &node_ids)?;
        s.check_valid()?;
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn get_explorer_preferences(
    state: State<'_, AppState>,
) -> Result<explorer_preferences::View, String> {
    Ok(state.explorer_preferences.lock().unwrap().view())
}
#[tauri::command]
async fn set_explorer_sort(
    state: State<'_, AppState>,
    sort: String,
) -> Result<explorer_preferences::View, String> {
    let order = sorting::Order::parse(Some(&sort))?;
    // Reserve ordering before dispatch; blocking jobs may run out of order.
    let mut prior = state.sort_requests.load(Ordering::Acquire);
    let request = loop {
        let next = prior
            .checked_add(1)
            .ok_or("Sort request limit exceeded. Restart the app.")?;
        match state.sort_requests.compare_exchange_weak(
            prior,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break next,
            Err(current) => prior = current,
        }
    };
    let preferences = state.explorer_preferences.clone();
    let requests = state.sort_requests.clone();
    tauri::async_runtime::spawn_blocking(move || {
        save_sort_request(&preferences, &requests, request, order)
    })
    .await
    .map_err(|e| e.to_string())?
}
fn save_sort_request(
    preferences: &Mutex<explorer_preferences::Preferences>,
    requests: &AtomicU64,
    request: u64,
    order: sorting::Order,
) -> Result<explorer_preferences::View, String> {
    let mut preferences = preferences.lock().unwrap();
    if request != requests.load(Ordering::Acquire) {
        return Err("A newer sort choice superseded this request.".into());
    }
    preferences.save(order)
}
fn preview_item(s: &Session, node_id: u32) -> Result<preview_validation::Item, String> {
    let snapshot = s.peek_completed()?;
    preview_validation::Item::from_snapshot(&snapshot, node_id, &s.root_ancestors)
}
#[tauri::command]
async fn quick_look_node<R: tauri::Runtime>(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u32,
    app: tauri::AppHandle<R>,
) -> Result<quick_look::Status, String> {
    let activity = Activity::begin(&state.activity)?;
    let s = state.scans.find(&scan_id)?;
    let registry = state.preview.clone();
    let current = registry.status();
    if current.open
        && current.scan_id.as_deref() == Some(&scan_id)
        && current.node_id == Some(node_id)
    {
        return quick_look::close(app, registry).await;
    }
    let item = preview_item(&s, node_id)?;
    let revision = registry.begin()?;
    let result = async {
        let scans = state.scans.clone();
        let work_s = s.clone();
        let work_registry = registry.clone();
        let path = item.path.clone();
        let (check, refresh) = tauri::async_runtime::spawn_blocking(
            move || -> Result<(quick_look::Check, quick_look::Check), String> {
                let held = work_s.hold(Need::Live)?;
                let held_file = Arc::new(item.open_checked()?);
                // Final filesystem validation runs immediately before dispatch on
                // this blocking worker. Main-thread callbacks check authority only;
                // the documented path-URL race includes the main-queue handoff.
                let _fresh = item.open_checked()?;
                drop(held);
                let weak_session = Arc::downgrade(&work_s);
                let weak_scans = Arc::downgrade(&scans);
                let check: quick_look::Check = Arc::new(move || {
                    if !work_registry.is_current(revision) {
                        return Err("Quick Look request was dismissed.".into());
                    }
                    let current_session = weak_session
                        .upgrade()
                        .ok_or("Preview session was released.")?;
                    let scans = weak_scans.upgrade().ok_or("Preview host was released.")?;
                    if !scans.is_live(&current_session) {
                        return Err("Preview session changed. Select a current file.".into());
                    }
                    drop(current_session.peek_completed()?);
                    let _keep_inode_alive = &held_file;
                    Ok(())
                });
                check()?;
                let authority = check.clone();
                let refresh: quick_look::Check = Arc::new(move || {
                    authority()?;
                    // Repeat identity validation after deliberate native waits,
                    // on a blocking worker immediately before each dispatch.
                    let _fresh = item.open_checked()?;
                    Ok(())
                });
                Ok((check, refresh))
            },
        )
        .await
        .map_err(|e| e.to_string())??;
        let window = app
            .get_webview_window("main")
            .ok_or("The main preview window is unavailable.")?;
        quick_look::present(
            app.clone(),
            window,
            registry.clone(),
            revision,
            scan_id,
            node_id,
            path,
            check,
            refresh,
        )
        .await
    }
    .await;
    if result.is_err() && registry.is_current(revision) {
        // A failed current request cannot leave old controller authority live.
        quick_look::close(app, registry).await?;
    }
    drop(activity);
    result
}
#[tauri::command]
async fn close_quick_look<R: tauri::Runtime>(
    state: State<'_, AppState>,
    app: tauri::AppHandle<R>,
) -> Result<quick_look::Status, String> {
    quick_look::close(app, state.preview.clone()).await
}
#[tauri::command]
async fn reveal_node(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u32,
) -> Result<(), String> {
    let _activity = Activity::begin(&state.activity)?;
    let s = state.scans.find(&scan_id)?;
    let _held = s.hold(Need::Live)?;
    let snapshot = s.store.read().unwrap();
    let n = snapshot.node(node_id).ok_or("Unknown node")?;
    platform::reveal(&n.path()?)
}
#[tauri::command]
async fn prepare_trash(
    state: State<'_, AppState>,
    scan_id: String,
    node_ids: Vec<u32>,
) -> Result<Review, String> {
    let _activity = Activity::begin(&state.activity)?;
    let s = state.scans.find(&scan_id)?;
    cleanup::review(&s, cleanup::Pick::Nodes(&node_ids))
}
#[tauri::command]
async fn commit_trash<R: tauri::Runtime>(
    state: State<'_, AppState>,
    scan_id: String,
    token: String,
    app: tauri::AppHandle<R>,
) -> Result<TrashResult, String> {
    commit_cleanup(state, scan_id, token, app, false).await
}
async fn commit_cleanup<R: tauri::Runtime>(
    state: State<'_, AppState>,
    scan_id: String,
    token: String,
    app: tauri::AppHandle<R>,
    queued: bool,
) -> Result<TrashResult, String> {
    let activity = Activity::begin(&state.activity)?;
    quick_look::close(app.clone(), state.preview.clone()).await?;
    let s = state.scans.find(&scan_id)?;
    let access_setup = state.access_setup.clone();
    let queue = state.cleanup_queue.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _activity = activity;
        // Restart waits for an in-flight native cleanup.
        let _access = access_setup.lock().unwrap();
        cleanup::commit(&s, &queue, &token, queued, cleanup::native_trash)
    })
    .await
    .map_err(|e| e.to_string())?
}
async fn queue_response(state: &AppState) -> Result<cleanup_queue::View, String> {
    let sessions = state.scans.sessions();
    let queue = state.cleanup_queue.clone();
    tauri::async_runtime::spawn_blocking(move || cleanup::queue_view(&queue, &sessions))
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn list_cleanup_queue(state: State<'_, AppState>) -> Result<cleanup_queue::View, String> {
    queue_response(&state).await
}
#[tauri::command]
async fn add_cleanup_queue(
    state: State<'_, AppState>,
    scan_id: String,
    node_ids: Vec<u32>,
) -> Result<cleanup_queue::View, String> {
    let _activity = Activity::begin(&state.activity)?;
    if node_ids.len() > 100_000 {
        return Err("Add at most 100,000 selection references at a time.".into());
    }
    let s = state.scans.find(&scan_id)?;
    let queue = state.cleanup_queue.clone();
    tauri::async_runtime::spawn_blocking(move || cleanup::add(&s, &queue, &node_ids))
        .await
        .map_err(|e| e.to_string())??;
    queue_response(&state).await
}
#[tauri::command]
async fn remove_cleanup_queue(
    state: State<'_, AppState>,
    entry_ids: Vec<String>,
) -> Result<cleanup_queue::View, String> {
    let _activity = Activity::begin(&state.activity)?;
    let queue = state.cleanup_queue.clone();
    tauri::async_runtime::spawn_blocking(move || queue.lock().unwrap().remove(&entry_ids))
        .await
        .map_err(|e| e.to_string())??;
    queue_response(&state).await
}
#[tauri::command]
async fn clear_cleanup_queue(state: State<'_, AppState>) -> Result<cleanup_queue::View, String> {
    let _activity = Activity::begin(&state.activity)?;
    let queue = state.cleanup_queue.clone();
    tauri::async_runtime::spawn_blocking(move || queue.lock().unwrap().clear())
        .await
        .map_err(|e| e.to_string())??;
    queue_response(&state).await
}
#[tauri::command]
async fn cleanup_queue_target(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<platform::Target, String> {
    let _activity = Activity::begin(&state.activity)?;
    let root = {
        let q = state.cleanup_queue.lock().unwrap();
        q.require_mutable()?;
        q.entries
            .iter()
            .find(|c| c.id == entry_id)
            .ok_or("Unknown cleanup queue entry.")?
            .root_checked()
    };
    validate_target(&root)?;
    let name = root
        .path
        .file_name()
        .unwrap_or(root.path.as_os_str())
        .to_string_lossy()
        .into_owned();
    // Retain the recorded guard. register() rereads authority and could accept a replacement
    // occurring between validation and registration; start_scan validates this original guard.
    // Flag a disk root like the Disks list does, so its scan uses the disk's own slot.
    let mut target = platform::target(&root.path, name);
    target.volume = platform::is_disk_path(&root.path);
    Ok(retain_target(&state, target, root))
}
#[tauri::command]
async fn prepare_cleanup_queue(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<Review, String> {
    let _activity = Activity::begin(&state.activity)?;
    let s = state.scans.find(&scan_id)?;
    let scans = state.scans.sessions();
    cleanup::review(&s, cleanup::Pick::Queue(&state.cleanup_queue, &scans))
}
#[tauri::command]
async fn commit_cleanup_queue<R: tauri::Runtime>(
    state: State<'_, AppState>,
    scan_id: String,
    token: String,
    app: tauri::AppHandle<R>,
) -> Result<TrashResult, String> {
    commit_cleanup(state, scan_id, token, app, true).await
}
#[tauri::command]
fn open_full_disk_access() -> Result<(), String> {
    platform::full_disk_access()
}
// Async so waiting on the lock held across a native cleanup never blocks the main thread.
#[tauri::command]
async fn full_disk_access_status(
    state: State<'_, AppState>,
) -> Result<access_setup::SetupInfo, String> {
    Ok(state.access_setup.lock().unwrap().info())
}
#[tauri::command]
async fn scan_without_full_disk_access(
    state: State<'_, AppState>,
) -> Result<access_setup::SetupInfo, String> {
    Ok(state.access_setup.lock().unwrap().allow_scan_anyway())
}
fn setup_idle(state: &AppState) -> Result<(), String> {
    state
        .scans
        .require_idle("Wait for the current scan to stop before restarting.")
}
#[tauri::command]
async fn restart_app<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _activity = Activity::begin(&state.activity)?;
    {
        let _access = state.access_setup.lock().unwrap();
        setup_idle(&state)?;
    }
    quick_look::close(app.clone(), state.preview.clone()).await?;
    // Restart launches the replacement before this process exits.
    if let Some(lock) = app.try_state::<std::fs::File>() {
        lock.unlock().map_err(|error| error.to_string())?;
    }
    app.restart()
}
pub fn run() {
    // Before AppState reads the limit: scans split what this process may hold open.
    budget::raise_descriptor_limit();
    #[cfg(feature = "performance-benchmark")]
    let benchmark_config = performance::read_config().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(move |app| {
            #[cfg(feature = "performance-benchmark")]
            let data = benchmark_config.state_dir.clone();
            #[cfg(not(feature = "performance-benchmark"))]
            let data = app.path().app_data_dir()?;
            app.manage(instance::acquire(&data)?);
            #[cfg(feature = "performance-benchmark")]
            {
                performance::install(app, benchmark_config).map_err(std::io::Error::other)?;
                Ok(())
            }
            #[cfg(not(feature = "performance-benchmark"))]
            {
            let config = app.path().app_config_dir()?;
            *app.state::<AppState>().explorer_preferences.lock().unwrap() =
                explorer_preferences::Preferences::load(&config);
            *app.state::<AppState>().rescan_cache.lock().unwrap() = Some(cache::Cache::new(
                data.join("rescan-cache"),
            ));
            *app.state::<AppState>().cleanup_queue.lock().unwrap() = cleanup_queue::Queue::load(data.join("cleanup-queue"));
            *app.state::<AppState>().library.lock().unwrap() = Some(library::Library::new(
                data.join("saved-scans"),
            ));
            *app.state::<AppState>().access_setup.lock().unwrap() =
                access_setup::AccessSetup::new(&std::env::current_exe()?);
            *app.state::<AppState>().scan_settings.lock().unwrap() =
                settings::ScanSettings::load(&config);
            Ok(())
            }
        })
        .on_page_load(|webview, payload| {
            #[cfg(feature = "performance-benchmark")]
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                if let Some(config) = performance::app_config() {
                    let data = serde_json::json!({"root":config.root,"mode":config.mode,"interactions":config.interactions});
                    let _ = webview.eval(format!(r#"window.__PERFORMANCE_BENCHMARK__ = {data}; window.dispatchEvent(new Event("performance-config"));"#));
                }
            }
            #[cfg(not(feature = "performance-benchmark"))]
            let _ = (webview, payload);
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            saved_scans_enabled,
            list_saved_scans,
            import_saved_scan,
            export_saved_scan,
            save_scan,
            begin_saved_open,
            cancel_saved_open,
            open_saved_scan,
            delete_saved_scan,
            compare_saved_scans,
            compare_current_scan,
            rescan_saved_scan,
            discover_targets,
            mounted_volumes,
            choose_folder,
            start_scan,
            cancel_scan,
            discard_scan,
            scan_summary,
            get_scan_issues,
            get_node,
            get_children,
            get_map,
            search_nodes,
            filter_nodes,
            get_file_types,
            get_file_type_highlights,
            get_explorer_preferences,
            set_explorer_sort,
            quick_look_node,
            close_quick_look,
            reveal_node,
            list_cleanup_queue,
            add_cleanup_queue,
            remove_cleanup_queue,
            clear_cleanup_queue,
            cleanup_queue_target,
            prepare_cleanup_queue,
            commit_cleanup_queue,
            prepare_trash,
            commit_trash,
            open_full_disk_access,
            full_disk_access_status,
            scan_without_full_disk_access,
            restart_app
        ])
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                window.state::<AppState>().preview.invalidate();
            }
        })
        .run(tauri::generate_context!())
        .expect("launch Hahakigami");
}

#[cfg(test)]
mod tests;
