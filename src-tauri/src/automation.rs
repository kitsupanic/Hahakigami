//! Independent metadata scans and retained results for the CLI and local MCP server.
use crate::{filters, instance, library, platform, settings, sorting};
use disk_scan::{Progress, ScanState, Snapshot};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
struct Session {
    current: Option<Arc<RwLock<Snapshot>>>,
    scan_id: Option<String>,
    active: bool,
    cancel: Arc<AtomicBool>,
    progress: Option<Progress>,
    error: Option<String>,
}

pub(crate) struct Service {
    data_dir: PathBuf,
    session: Mutex<Session>,
    library_gate: Mutex<()>,
}

impl Service {
    pub(crate) fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            session: Mutex::new(Session::default()),
            library_gate: Mutex::new(()),
        }
    }

    fn library<T>(
        &self,
        operation: impl FnOnce(&library::Library) -> Result<T, String>,
    ) -> Result<T, String> {
        let _gate = self
            .library_gate
            .lock()
            .map_err(|_| "Automation library lock poisoned")?;
        let _instance = instance::acquire(&self.data_dir).map_err(|e| e.to_string())?;
        operation(&library::Library::new(self.data_dir.join("saved-scans")))
    }

    pub(crate) fn scan(&self, path: PathBuf) -> Result<Value, String> {
        let expected = disk_scan::platform_identity(&path).map_err(|e| e.to_string())?;
        let resolved = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        let root = disk_scan::ScanRoot::open(resolved, expected).map_err(|e| e.to_string())?;
        let scope = if platform::is_volume_root(&root) {
            settings::AutoScope::Volume
        } else {
            settings::AutoScope::Folder
        };
        let (workers, _, source) = settings::ScanSettings::default()
            .resolve_scan_workers(scope, platform::scheduling_conditions());
        let store = Arc::new(RwLock::new(Snapshot::empty(root.path().to_owned())));
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut session = self
                .session
                .lock()
                .map_err(|_| "Automation session lock poisoned")?;
            if session.active {
                return Err(
                    "A scan is already running; cancel it and wait for it to finish.".into(),
                );
            }
            *session = Session {
                current: Some(store.clone()),
                active: true,
                cancel: cancel.clone(),
                ..Session::default()
            };
        }
        let started = now_ms();
        let result = disk_scan::scan_live_authorized(
            root,
            disk_scan::ScanOptions {
                workers,
                ..Default::default()
            },
            cancel,
            store.clone(),
            |progress| {
                if let Ok(mut session) = self.session.lock() {
                    session.progress = Some(progress);
                }
            },
        );
        if result.is_err() {
            let mut snapshot = store
                .write()
                .map_err(|_| "Automation snapshot lock poisoned")?;
            snapshot.state = ScanState::Failed;
            snapshot.stats.incomplete = true;
        }
        let snapshot = store
            .read()
            .map_err(|_| "Automation snapshot lock poisoned")?;
        let saved = match result {
            Err(error) => Err(error.to_string()),
            Ok(()) if snapshot.state == ScanState::Completed && crate::SAVED_SCANS_ENABLED => self.library(|library| {
                library.save(&snapshot, &library::ScanMetadata {
                    started_at_ms: started,
                    completed_at_ms: now_ms(),
                    workers,
                    worker_mode: "auto".into(),
                    worker_source: source.into(),
                    qos: disk_scan::ScanQos::Automatic,
                }, |_| {}).map(|entry| Some(entry.id))
            }).map_err(|error| format!("Scan completed but could not be saved: {error}. Results remain available as current.")),
            Ok(()) => Ok(None),
        };
        let mut session = self
            .session
            .lock()
            .map_err(|_| "Automation session lock poisoned")?;
        session.active = false;
        match saved {
            Ok(id) => {
                session.scan_id = id;
                Ok(summary(&snapshot, session.scan_id.as_deref()))
            }
            Err(error) => {
                session.error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub(crate) fn status(&self) -> Result<Value, String> {
        let (store, id, active, progress, error) = {
            let session = self
                .session
                .lock()
                .map_err(|_| "Automation session lock poisoned")?;
            (
                session.current.clone(),
                session.scan_id.clone(),
                session.active,
                session.progress.clone(),
                session.error.clone(),
            )
        };
        let Some(store) = store else {
            return Ok(json!({"state": "idle", "running": false, "scanId": null}));
        };
        // Final aggregation holds the write lock; cancellation and progress stay responsive.
        let mut value = match store.try_read() {
            Ok(snapshot) => summary(&snapshot, id.as_deref()),
            Err(std::sync::TryLockError::WouldBlock) => {
                json!({"state": "running", "scanId": id, "progress": progress})
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err("Automation snapshot lock poisoned".into())
            }
        };
        value["running"] = json!(active);
        value["error"] = json!(error);
        Ok(value)
    }

    pub(crate) fn cancel(&self) -> Result<Value, String> {
        let session = self
            .session
            .lock()
            .map_err(|_| "Automation session lock poisoned")?;
        if session.active {
            session.cancel.store(true, Ordering::Relaxed);
        }
        Ok(json!({"cancelRequested": session.active, "scanId": session.scan_id}))
    }

    pub(crate) fn list(&self, offset: usize, limit: usize) -> Result<Value, String> {
        crate::require_saved_scans()?;
        validate_page(limit)?;
        self.library(|library| {
            let list = library.list()?;
            let total = list.entries.len();
            Ok(json!({"items": list.entries.into_iter().skip(offset).take(limit).collect::<Vec<_>>(), "total": total, "offset": offset, "limit": limit}))
        })
    }

    fn with_snapshot(
        &self,
        id: &str,
        operation: impl FnOnce(&Snapshot, Option<&str>) -> Result<Value, String>,
    ) -> Result<Value, String> {
        if id != "current" {
            crate::require_saved_scans()?;
            if uuid::Uuid::parse_str(id).is_err() {
                return Err("Invalid saved scan ID".into());
            }
        }
        let current = {
            let session = self
                .session
                .lock()
                .map_err(|_| "Automation session lock poisoned")?;
            if id == "current" || session.scan_id.as_deref() == Some(id) {
                session
                    .current
                    .clone()
                    .map(|store| (store, session.scan_id.clone()))
            } else {
                None
            }
        };
        if let Some((store, saved_id)) = current {
            let snapshot = store
                .read()
                .map_err(|_| "Automation snapshot lock poisoned")?;
            return operation(&snapshot, saved_id.as_deref());
        }
        if id == "current" {
            return Err("No current scan; start a scan or use a saved scan ID.".into());
        }
        let (snapshot, _) = self.library(|library| library.load(id, |_| {}))?;
        operation(&snapshot, Some(id))
    }

    pub(crate) fn summary(&self, scan_id: &str) -> Result<Value, String> {
        self.with_snapshot(scan_id, |snapshot, id| Ok(summary(snapshot, id)))
    }

    pub(crate) fn node(&self, scan_id: &str, node_id: u32) -> Result<Value, String> {
        self.with_snapshot(scan_id, |snapshot, id| {
            let paths = issue_paths(snapshot);
            let mut value = serde_json::to_value(crate::view(
                &snapshot.node(node_id).ok_or("Unknown node ID")?,
                &paths,
            )?)
            .map_err(|e| e.to_string())?;
            value["scanId"] = json!(id);
            value["coverage"] = coverage(snapshot);
            Ok(value)
        })
    }

    pub(crate) fn children(
        &self,
        scan_id: &str,
        node_id: u32,
        offset: usize,
        limit: usize,
    ) -> Result<Value, String> {
        validate_page(limit)?;
        self.with_snapshot(scan_id, |snapshot, id| {
            let mut ids = snapshot.children(node_id)?.to_vec();
            sorting::sort_ids(snapshot, &mut ids, sorting::Order::SizeDescending, true)?;
            node_page(
                snapshot,
                id,
                &ids[offset.min(ids.len())..ids.len().min(offset.saturating_add(limit))],
                ids.len(),
                offset,
                limit,
            )
        })
    }

    pub(crate) fn search(
        &self,
        scan_id: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Value, String> {
        validate_page(limit)?;
        if query.trim().is_empty() || query.len() > 1024 {
            return Err("Filename query must contain 1 to 1024 bytes.".into());
        }
        let query = filters::Query::search(
            query.into(),
            "allocated".into(),
            sorting::Order::SizeDescending,
        )?;
        self.with_snapshot(scan_id, |snapshot, id| {
            let mut cache = filters::Cache::default();
            let (mut ids, total) = cache.page(snapshot, &query, offset, limit.min(500))?;
            if limit > 500 {
                let (more, _) =
                    cache.page(snapshot, &query, offset.saturating_add(500), limit - 500)?;
                ids.extend(more);
            }
            node_page(snapshot, id, &ids, total, offset, limit)
        })
    }

    pub(crate) fn issues(
        &self,
        scan_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Value, String> {
        validate_page(limit)?;
        self.with_snapshot(scan_id, |snapshot, id| {
            let total = snapshot.errors.len().saturating_add(snapshot.exclusions.len());
            let items: Vec<_> = snapshot.errors.iter().map(|issue| ("error", issue))
                .chain(snapshot.exclusions.iter().map(|issue| ("exclusion", issue)))
                .skip(offset).take(limit).map(|(kind, issue)| json!({
                    "kind": kind, "path": issue.path.to_string_lossy(), "pathKey": crate::path_key(&issue.path),
                    "message": issue.message, "errorKind": issue.error_kind, "rawOsError": issue.raw_os_error,
                })).collect();
            Ok(json!({"items": items, "total": total, "offset": offset, "limit": limit, "scanId": id, "coverage": coverage(snapshot)}))
        })
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn validate_page(limit: usize) -> Result<(), String> {
    if (1..=1000).contains(&limit) {
        Ok(())
    } else {
        Err("Limit must be between 1 and 1000.".into())
    }
}

fn state(snapshot: &Snapshot) -> &'static str {
    match snapshot.state {
        ScanState::Running => "running",
        ScanState::Completed => "completed",
        ScanState::Cancelled => "cancelled",
        ScanState::Failed => "failed",
    }
}

fn coverage(snapshot: &Snapshot) -> Value {
    json!({"state": state(snapshot), "incomplete": snapshot.stats.incomplete, "errors": snapshot.stats.errors,
        "excluded": snapshot.stats.excluded, "errorsStored": snapshot.errors.len(), "exclusionsStored": snapshot.exclusions.len(),
        "restrictedSystemIssues": snapshot.stats.restricted_system_issues, "accounting": snapshot.accounting})
}

fn summary(snapshot: &Snapshot, scan_id: Option<&str>) -> Value {
    json!({"scanId": scan_id, "state": state(snapshot), "rootPath": snapshot.root.to_string_lossy(), "pathKey": crate::path_key(&snapshot.root),
        "rootId": snapshot.node(0).map(|node| node.id), "logicalBytes": snapshot.stats.logical_bytes, "allocatedBytes": snapshot.stats.allocated_bytes,
        "entries": snapshot.stats.entries, "errors": snapshot.stats.errors, "excluded": snapshot.stats.excluded, "incomplete": snapshot.stats.incomplete,
        "stats": snapshot.stats, "accounting": snapshot.accounting, "coverage": coverage(snapshot)})
}

fn issue_paths(snapshot: &Snapshot) -> Vec<PathBuf> {
    let mut paths: Vec<_> = snapshot
        .errors
        .iter()
        .map(|issue| issue.path.clone())
        .collect();
    paths.sort_unstable();
    paths
}

fn node_page(
    snapshot: &Snapshot,
    scan_id: Option<&str>,
    ids: &[u32],
    total: usize,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    let paths = issue_paths(snapshot);
    let items = ids
        .iter()
        .map(|&id| crate::view(&snapshot.node(id).ok_or("Unknown node ID")?, &paths))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        json!({"items": items, "total": total, "offset": offset, "limit": limit, "scanId": scan_id, "coverage": coverage(snapshot)}),
    )
}
