//! Independent metadata scans and their in-memory results for the CLI and local MCP server.
use crate::{filters, platform, settings, sorting};
use disk_scan::{Progress, ScanState, Snapshot};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
};

#[derive(Default)]
struct Session {
    current: Option<Arc<RwLock<Snapshot>>>,
    active: bool,
    /// True from the start of a scan until its store is installed; `current` is still the previous scan then.
    starting: bool,
    /// Counts installed scans, so a node ID from an earlier scan can be told apart from the current one.
    scan_id: u64,
    cancel: Arc<AtomicBool>,
    progress: Option<Progress>,
    error: Option<String>,
    /// Replaced with the session, so a new scan never sees another scan's cached order or matches.
    caches: Arc<Mutex<Caches>>,
}

#[derive(Default)]
struct Caches {
    sorts: sorting::Cache,
    filters: filters::Cache,
}

pub(crate) struct Service {
    session: Mutex<Session>,
}

impl Service {
    pub(crate) fn new() -> Self {
        Self {
            session: Mutex::new(Session::default()),
        }
    }

    pub(crate) fn scan(&self, path: PathBuf) -> Result<Value, String> {
        self.scan_with(path, Arc::new(AtomicBool::new(false)))
    }

    /// `scan` with the caller's cancel flag, so a caller can stop exactly this scan, even before it starts.
    pub(crate) fn scan_with(
        &self,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value, String> {
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
            // Active before the path is resolved, so a cancel during setup is kept: the scanner starts cancelled.
            session.active = true;
            session.starting = true;
            session.cancel = cancel.clone();
        }
        // Clears `active` on every exit, including a scanner panic or an early `?`, so later scans are not refused.
        let _active = ActiveGuard(&self.session);
        let resolved = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        platform::ensure_local_volume(&resolved)?;
        let expected = disk_scan::platform_identity(&resolved).map_err(|e| e.to_string())?;
        let root = disk_scan::ScanRoot::open(resolved, expected).map_err(|e| e.to_string())?;
        let scope = if platform::is_volume_root(&root) {
            settings::AutoScope::Volume
        } else {
            settings::AutoScope::Folder
        };
        let (workers, _, _) = settings::ScanSettings::default()
            .resolve_scan_workers(scope, platform::scheduling_conditions());
        let store = Arc::new(RwLock::new(Snapshot::empty(root.path().to_owned())));
        {
            let mut session = self
                .session
                .lock()
                .map_err(|_| "Automation session lock poisoned")?;
            *session = Session {
                current: Some(store.clone()),
                active: true,
                scan_id: session.scan_id + 1,
                cancel: cancel.clone(),
                ..Session::default()
            };
        }
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
        let mut session = self
            .session
            .lock()
            .map_err(|_| "Automation session lock poisoned")?;
        session.active = false;
        match result {
            Ok(()) => {
                let mut value = summary(&snapshot);
                value["scanId"] = json!(session.scan_id);
                Ok(value)
            }
            Err(error) => {
                let error = error.to_string();
                session.error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub(crate) fn status(&self) -> Result<Value, String> {
        let (store, active, starting, scan_id, progress, error) = {
            let session = self
                .session
                .lock()
                .map_err(|_| "Automation session lock poisoned")?;
            (
                session.current.clone(),
                session.active,
                session.starting,
                session.scan_id,
                session.progress.clone(),
                session.error.clone(),
            )
        };
        // `store` is still the previous scan's, or none for a session's first scan; neither describes the new one.
        if starting {
            return Ok(json!({"state": "starting", "running": true, "error": null}));
        }
        let Some(store) = store else {
            return Ok(json!({"state": "idle", "running": false}));
        };
        // Final aggregation holds the write lock; cancellation and progress stay responsive.
        let mut value = match store.try_read() {
            Ok(snapshot) => summary(&snapshot),
            Err(std::sync::TryLockError::WouldBlock) => {
                json!({"state": "running", "progress": progress})
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err("Automation snapshot lock poisoned".into())
            }
        };
        value["scanId"] = json!(scan_id);
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
        Ok(json!({"cancelRequested": session.active}))
    }

    /// Runs `operation` on the session's latest scan, also passing that scan's caches.
    /// A `scan_id` from an earlier scan is refused; the result carries the current one.
    fn with_caches(
        &self,
        scan_id: Option<u64>,
        operation: impl FnOnce(&Snapshot, &mut Caches) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let (store, caches, current) = {
            let session = self
                .session
                .lock()
                .map_err(|_| "Automation session lock poisoned")?;
            (
                session
                    .current
                    .clone()
                    .ok_or("No scan yet; start a scan first.")?,
                session.caches.clone(),
                session.scan_id,
            )
        };
        if scan_id.is_some_and(|id| id != current) {
            return Err("This node ID belongs to an earlier scan. Call summary again.".into());
        }
        let snapshot = store
            .read()
            .map_err(|_| "Automation snapshot lock poisoned")?;
        let mut caches = caches
            .lock()
            .map_err(|_| "Automation cache lock poisoned")?;
        let mut value = operation(&snapshot, &mut caches)?;
        value["scanId"] = json!(current);
        Ok(value)
    }

    fn with_snapshot(
        &self,
        scan_id: Option<u64>,
        operation: impl FnOnce(&Snapshot) -> Result<Value, String>,
    ) -> Result<Value, String> {
        self.with_caches(scan_id, |snapshot, _| operation(snapshot))
    }

    pub(crate) fn summary(&self) -> Result<Value, String> {
        self.with_snapshot(None, |snapshot| Ok(summary(snapshot)))
    }

    pub(crate) fn node(&self, node_id: u32, scan_id: Option<u64>) -> Result<Value, String> {
        self.with_snapshot(scan_id, |snapshot| {
            let paths = issue_paths(snapshot);
            let mut value = serde_json::to_value(crate::view(
                &snapshot.node(node_id).ok_or("Unknown node ID")?,
                &paths,
            )?)
            .map_err(|e| e.to_string())?;
            value["coverage"] = coverage(snapshot);
            Ok(value)
        })
    }

    pub(crate) fn children(
        &self,
        node_id: u32,
        scan_id: Option<u64>,
        offset: usize,
        limit: usize,
    ) -> Result<Value, String> {
        validate_page(limit)?;
        self.with_caches(scan_id, |snapshot, caches| {
            if snapshot
                .node(node_id)
                .is_some_and(|node| !node.is_directory)
            {
                return Err("Not a directory".into());
            }
            let (ids, total) = page_of(offset, limit, |offset, limit| {
                let order = sorting::Order::SizeDescending;
                caches.sorts.page(snapshot, node_id, order, offset, limit)
            })?;
            node_page(snapshot, &ids, total, offset, limit)
        })
    }

    pub(crate) fn search(
        &self,
        query: &str,
        scan_id: Option<u64>,
        offset: usize,
        limit: usize,
    ) -> Result<Value, String> {
        validate_page(limit)?;
        // Match the trimmed text, as the GUI does, so padding never changes what is found.
        let query = query.trim();
        if query.is_empty() || query.len() > 1024 {
            return Err("Filename query must contain 1 to 1024 bytes.".into());
        }
        let query = filters::Query::search(query.into(), sorting::Order::SizeDescending)?;
        self.with_caches(scan_id, |snapshot, caches| {
            let (ids, total) = page_of(offset, limit, |offset, limit| {
                caches.filters.page(snapshot, &query, offset, limit)
            })?;
            node_page(snapshot, &ids, total, offset, limit)
        })
    }

    pub(crate) fn issues(&self, offset: usize, limit: usize) -> Result<Value, String> {
        validate_page(limit)?;
        self.with_snapshot(None, |snapshot| {
            let total = snapshot.errors.len().saturating_add(snapshot.exclusions.len());
            let items: Vec<_> = snapshot.errors.iter().map(|issue| ("error", issue))
                .chain(snapshot.exclusions.iter().map(|issue| ("exclusion", issue)))
                .skip(offset).take(limit).map(|(kind, issue)| json!({
                    "kind": kind, "path": issue.path.to_string_lossy(), "pathKey": crate::path_key(&issue.path),
                    "message": issue.message, "errorKind": issue.error_kind, "rawOsError": issue.raw_os_error,
                })).collect();
            Ok(json!({"items": items, "total": total, "offset": offset, "limit": limit, "coverage": coverage(snapshot)}))
        })
    }
}

struct ActiveGuard<'a>(&'a Mutex<Session>);
impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        let mut session = self.0.lock().unwrap_or_else(|e| e.into_inner());
        session.active = false;
        session.starting = false;
    }
}

fn validate_page(limit: usize) -> Result<(), String> {
    if (1..=1000).contains(&limit) {
        Ok(())
    } else {
        Err("Limit must be between 1 and 1000.".into())
    }
}

/// The caches serve at most 500 ids per call, so a larger `limit` takes a second call.
fn page_of(
    offset: usize,
    limit: usize,
    mut fetch: impl FnMut(usize, usize) -> Result<(Vec<u32>, usize), String>,
) -> Result<(Vec<u32>, usize), String> {
    let (mut ids, total) = fetch(offset, limit.min(500))?;
    if limit > 500 {
        ids.extend(fetch(offset.saturating_add(500), limit - 500)?.0);
    }
    Ok((ids, total))
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

fn summary(snapshot: &Snapshot) -> Value {
    json!({"state": state(snapshot), "rootPath": snapshot.root.to_string_lossy(), "pathKey": crate::path_key(&snapshot.root),
        "rootId": snapshot.node(0).map(|node| node.id), "allocatedBytes": snapshot.stats.allocated_bytes,
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
        json!({"items": items, "total": total, "offset": offset, "limit": limit, "coverage": coverage(snapshot)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_flag_clears_when_a_scan_panics() {
        let service = Service::new();
        service.session.lock().unwrap().active = true;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _active = ActiveGuard(&service.session);
            panic!("scanner failure");
        }));
        assert!(result.is_err());
        assert!(!service.session.lock().unwrap().active);
    }

    #[test]
    fn a_scan_that_fails_to_start_clears_active_and_keeps_the_previous_results() {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let service = Service::new();
        service.scan(root.clone()).unwrap();
        assert!(service.scan(root.join("missing")).is_err());
        assert!(!service.session.lock().unwrap().active);
        assert_eq!(service.summary().unwrap()["state"], "completed");
        service.scan(root).unwrap();
    }

    #[test]
    fn children_pages_stay_ordered_and_a_new_scan_drops_the_caches() {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("small"), vec![1; 10]).unwrap();
        std::fs::write(root.join("large"), vec![1; 100_000]).unwrap();
        let service = Service::new();
        let names = |page: Value| -> Vec<String> {
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["name"].as_str().unwrap().to_owned())
                .collect()
        };
        service.scan(root.clone()).unwrap();
        assert_eq!(names(service.children(0, None, 0, 1).unwrap()), ["large"]);
        assert_eq!(names(service.children(0, None, 1, 1).unwrap()), ["small"]);
        let first = service.session.lock().unwrap().caches.clone();
        std::fs::write(root.join("largest"), vec![1; 1_000_000]).unwrap();
        service.scan(root.clone()).unwrap();
        assert_eq!(names(service.children(0, None, 0, 1).unwrap()), ["largest"]);
        assert!(
            !Arc::ptr_eq(&first, &service.session.lock().unwrap().caches),
            "a new scan starts with empty caches"
        );
    }

    #[test]
    fn node_ids_from_an_earlier_scan_are_refused() {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let service = Service::new();
        let first = service.scan(root.clone()).unwrap()["scanId"].clone();
        assert_eq!(service.status().unwrap()["scanId"], first);
        let id = first.as_u64();
        assert_eq!(service.node(0, id).unwrap()["scanId"], first);
        // A failed start keeps the previous scan, so its counter stays valid.
        assert!(service.scan(root.join("missing")).is_err());
        assert!(service.node(0, id).is_ok());
        let second = service.scan(root).unwrap()["scanId"].as_u64();
        assert_ne!(second, id);
        for result in [
            service.node(0, id),
            service.children(0, id, 0, 1),
            service.search("x", id, 0, 1),
        ] {
            assert!(result.unwrap_err().contains("earlier scan"));
        }
        assert!(service.node(0, second).is_ok());
        assert!(service.node(0, None).is_ok());
    }

    #[test]
    fn children_of_a_file_is_an_error_and_unknown_ids_keep_theirs() {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("file"), [1; 10]).unwrap();
        let service = Service::new();
        service.scan(root).unwrap();
        let file = service.search("file", None, 0, 1).unwrap()["items"][0]["id"]
            .as_u64()
            .unwrap() as u32;
        assert_eq!(
            service.children(file, None, 0, 1).unwrap_err(),
            "Not a directory"
        );
        assert_ne!(
            service.children(u32::MAX, None, 0, 1).unwrap_err(),
            "Not a directory"
        );
    }

    #[test]
    fn search_matches_the_trimmed_query() {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("needle.txt"), [1; 10]).unwrap();
        let service = Service::new();
        service.scan(root).unwrap();
        assert_eq!(
            service.search("  needle ", None, 0, 10).unwrap()["total"],
            1
        );
    }

    #[test]
    fn status_reports_starting_without_the_previous_scans_totals() {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let service = Service::new();
        let start = |service: &Service| {
            let mut session = service.session.lock().unwrap();
            session.active = true;
            session.starting = true;
        };
        // A session's first scan, with no previous results, is starting too, not idle.
        start(&service);
        assert_eq!(service.status().unwrap()["state"], "starting");
        drop(ActiveGuard(&service.session));
        service.scan(root).unwrap();
        start(&service);
        let status = service.status().unwrap();
        assert_eq!(status["state"], "starting");
        assert_eq!(status["running"], true);
        assert!(status.get("allocatedBytes").is_none());
        // A start that fails clears the state and the previous results are served again.
        drop(ActiveGuard(&service.session));
        assert_eq!(service.status().unwrap()["state"], "completed");
    }

    #[test]
    fn scan_follows_a_symlinked_root() {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("file"), [1; 10]).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let summary = Service::new().scan(link).unwrap();
        assert_eq!(summary["state"], "completed");
        assert_eq!(
            summary["rootPath"],
            std::fs::canonicalize(&target)
                .unwrap()
                .to_string_lossy()
                .as_ref()
        );
    }
}
