//! Revision authority for a single native preview, independent of Objective-C objects.
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    pub open: bool,
    pub scan_id: Option<String>,
    pub node_id: Option<u32>,
    pub revision: u64,
}
#[derive(Default)]
struct RegistryState {
    status: Status,
    current: bool,
}
#[derive(Default)]
pub(crate) struct Registry {
    state: Mutex<RegistryState>,
    native_attempted: AtomicBool,
}
impl Registry {
    #[cfg(target_os = "macos")]
    pub(super) fn mark_native_attempted(&self) {
        self.native_attempted.store(true, Ordering::Release);
    }
    fn native_attempted(&self) -> bool {
        self.native_attempted.load(Ordering::Acquire)
    }
    pub(crate) fn begin(&self) -> Result<u64, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Quick Look state is unavailable")?;
        state.status.revision = state
            .status
            .revision
            .checked_add(1)
            .filter(|revision| *revision < u64::MAX)
            .ok_or("Quick Look revision limit reached")?;
        state.current = true;
        Ok(state.status.revision)
    }
    pub(crate) fn is_current(&self, revision: u64) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.current && state.status.revision == revision)
    }
    pub(crate) fn status(&self) -> Status {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .clone()
    }
    pub(crate) fn invalidate(&self) -> Status {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let revision = state.status.revision.saturating_add(1);
        state.current = false;
        state.status = Status {
            revision,
            ..Status::default()
        };
        state.status.clone()
    }
    pub(crate) fn publish(
        &self,
        revision: u64,
        scan_id: String,
        node_id: u32,
        open: bool,
    ) -> Result<Status, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Quick Look state is unavailable")?;
        if !state.current || state.status.revision != revision {
            return Err("Quick Look request is stale".into());
        }
        let revision = if open {
            revision
        } else {
            revision
                .checked_add(1)
                .ok_or("Quick Look revision limit reached")?
        };
        state.current = open;
        state.status = Status {
            open,
            scan_id: open.then_some(scan_id),
            node_id: open.then_some(node_id),
            revision,
        };
        Ok(state.status.clone())
    }
}
pub(crate) type Check = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;
#[cfg(target_os = "macos")]
#[path = "quick_look_native.rs"]
mod native;

#[cfg(target_os = "macos")]
async fn on_main<R: Runtime, T: Send + 'static>(
    app: &AppHandle<R>,
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = sender.send(task());
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        receiver
            .recv()
            .map_err(|_| "Quick Look main-thread operation was interrupted".to_string())?
    })
    .await
    .map_err(|e| e.to_string())?
}
#[cfg(target_os = "macos")]
async fn pause() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(50))
    })
    .await
    .map_err(|e| e.to_string())
}
#[cfg(target_os = "macos")]
async fn await_dismissal<R: Runtime>(
    app: &AppHandle<R>,
    registry: &Arc<Registry>,
    revision: u64,
) -> Result<(), String> {
    for _ in 0..100 {
        let registry = registry.clone();
        if on_main(app, move || native::close(&registry, revision)).await? {
            return Ok(());
        }
        pause().await?;
    }
    Err("Quick Look has not finished closing; try closing the preview again".into())
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn present<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    registry: Arc<Registry>,
    revision: u64,
    scan_id: String,
    node_id: u32,
    path: PathBuf,
    check: Check,
    refresh: Check,
) -> Result<Status, String> {
    #[cfg(target_os = "macos")]
    {
        for _ in 0..100 {
            let validate = refresh.clone();
            let refreshed = tauri::async_runtime::spawn_blocking(move || validate())
                .await
                .map_err(|e| e.to_string())
                .and_then(|result| result);
            if let Err(error) = refreshed {
                if let Ok(status) = registry.publish(revision, scan_id.clone(), node_id, false) {
                    let _ = app.emit("quick-look-status", &status);
                    if registry.native_attempted() {
                        await_dismissal(&app, &registry, status.revision).await?;
                    }
                }
                return Err(error);
            }
            let emit_app = app.clone();
            let window = window.clone();
            let request_registry = registry.clone();
            let scan_id = scan_id.clone();
            let path = path.clone();
            let check = check.clone();
            let result = on_main(&app, move || {
                let emit: native::Emit = Arc::new(move |status| {
                    let _ = emit_app.emit("quick-look-status", status);
                });
                let result =
                    window
                        .ns_window()
                        .map_err(|e| e.to_string())
                        .and_then(|window| unsafe {
                            native::present(
                                window,
                                request_registry.clone(),
                                revision,
                                scan_id.clone(),
                                node_id,
                                path,
                                check,
                                emit.clone(),
                            )
                        });
                if result.is_err() && request_registry.is_current(revision) {
                    if let Ok(status) = request_registry.publish(revision, scan_id, node_id, false)
                    {
                        emit(status);
                    }
                }
                result
            })
            .await?;
            if let Some(status) = result {
                if !status.open {
                    await_dismissal(&app, &registry, revision).await?;
                }
                return Ok(status);
            }
            pause().await?;
        }
        if let Ok(status) = registry.publish(revision, scan_id, node_id, false) {
            let _ = app.emit("quick-look-status", status);
        }
        Err("Quick Look is still closing a previous preview; try again".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (
            app, window, registry, revision, scan_id, node_id, path, check, refresh,
        );
        Err("Quick Look is only available on macOS".into())
    }
}
pub(crate) async fn close<R: Runtime>(
    app: AppHandle<R>,
    registry: Arc<Registry>,
) -> Result<Status, String> {
    // Revoke pending requests before the first await. A later close cannot let an
    // earlier cleanup proceed until the native panel has actually disappeared.
    let status = registry.invalidate();
    let _ = app.emit("quick-look-status", &status);
    if app.get_webview_window("main").is_none() && !registry.native_attempted() {
        return Ok(status);
    }
    #[cfg(target_os = "macos")]
    {
        await_dismissal(&app, &registry, status.revision).await?;
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_publications_cannot_reopen_closed_or_newer_preview() {
        let registry = Registry::default();
        let first = registry.begin().unwrap();
        assert!(
            registry
                .publish(first, "first".into(), 1, true)
                .unwrap()
                .open
        );
        let closed = registry.invalidate();
        assert!(!closed.open);
        assert!(closed.revision > first);
        assert!(registry.publish(first, "first".into(), 1, true).is_err());
        let next = registry.begin().unwrap();
        registry.publish(next, "next".into(), 2, true).unwrap();
        assert!(registry.publish(first, "first".into(), 1, false).is_err());
        assert_eq!(registry.status().node_id, Some(2));
    }
    #[test]
    fn overlapping_threads_only_latest_request_can_publish() {
        let registry = Arc::new(Registry::default());
        let first = registry.begin().unwrap();
        let worker = registry.clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let wait = barrier.clone();
        let thread = std::thread::spawn(move || {
            wait.wait();
            worker.publish(first, "stale".into(), 1, true)
        });
        let next = registry.begin().unwrap();
        barrier.wait();
        assert!(thread.join().unwrap().is_err());
        assert!(registry.is_current(next));
        registry.publish(next, "latest".into(), 2, true).unwrap();
        assert_eq!(registry.invalidate().scan_id, None);
    }
    #[test]
    fn native_close_advances_status_without_revoking_a_newer_request() {
        let registry = Registry::default();
        let opened = registry.begin().unwrap();
        registry.publish(opened, "scan".into(), 1, true).unwrap();
        let closed = registry.publish(opened, "scan".into(), 1, false).unwrap();
        assert!(closed.revision > opened);
        assert!(!closed.open);
        let latest = registry.begin().unwrap();
        assert!(registry.publish(opened, "scan".into(), 1, false).is_err());
        assert!(registry.is_current(latest));
        registry.publish(latest, "new".into(), 2, true).unwrap();
        assert_eq!(registry.status().node_id, Some(2));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_attempt_proof_is_never_reset_by_status_changes() {
        let registry = Registry::default();
        assert!(!registry.native_attempted());
        registry.mark_native_attempted();
        registry.invalidate();
        let rev = registry.begin().unwrap();
        registry.publish(rev, "scan".into(), 1, false).unwrap();
        assert!(registry.native_attempted());
    }
    #[test]
    fn native_close_revokes_same_revision_publication() {
        let registry = Registry::default();
        let revision = registry.begin().unwrap();
        registry.publish(revision, "scan".into(), 1, true).unwrap();
        registry.publish(revision, "scan".into(), 1, false).unwrap();
        assert!(!registry.is_current(revision));
        assert!(registry.publish(revision, "scan".into(), 1, true).is_err());
        assert!(!registry.status().open);
    }
    #[test]
    fn revision_overflow_never_reuses_old_authority() {
        let registry = Registry::default();
        registry.state.lock().unwrap().status.revision = u64::MAX;
        assert!(registry.begin().is_err());
        assert!(!registry.invalidate().open);
        assert_eq!(registry.status().revision, u64::MAX);
        assert!(!registry.is_current(u64::MAX));
        assert!(registry.publish(u64::MAX, "stale".into(), 1, true).is_err());
    }
}
