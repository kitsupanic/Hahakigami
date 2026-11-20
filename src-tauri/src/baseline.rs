//! The Rescan baseline (`GLOSSARY.md`): whether a scan starts from an earlier one, and keeping its
//! result for the next. `run_scan` reaches the rescan cache and the filesystem event journal only
//! through here: `Baselines::acquire` before the scan, `Ticket::publish` after it.
use crate::{cache, cache_store, events, now_ms};
use disk_scan::{ScanState, Snapshot};
#[cfg(feature = "performance-benchmark")]
use std::time::Instant;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, PoisonError, RwLock,
    },
};

/// What `slots.rs` takes from the finished scan of the same root, and what `eligible` judges.
pub(crate) use crate::cache::{eligible, CachedScan as Taken, Token};

/// A root's event history: the journal's epoch (boot session and journal identity) and the mount
/// the journal is replayed on.
#[derive(Clone, Debug)]
pub(crate) struct History {
    pub(crate) epoch: String,
    pub(crate) volume: PathBuf,
}

/// The filesystem event journal, as the baseline needs it.
pub(crate) trait Journal: Sync {
    /// This root's event history, if it has one: only a root on a local volume whose boot session and
    /// journal identity are both known.
    fn history(&self, root: &disk_scan::ScanRoot) -> Option<History>;
    /// The journal cursor now.
    fn cursor(&self) -> Option<u64>;
    /// Replays the changes since `since` while `load` runs. `load` gets a flag the journal sets
    /// once the replay has failed, so it can stop work whose result is no longer used.
    fn replay<T: Send>(
        &self,
        root: &Path,
        history: &History,
        since: u64,
        cancel: &AtomicBool,
        load: impl FnOnce(&AtomicBool) -> Result<T, String> + Send,
    ) -> Result<(T, events::Changes), String>;
}

/// The app's journal: FSEvents, through `events.rs`.
pub(crate) struct Fsevents;

impl Journal for Fsevents {
    fn history(&self, root: &disk_scan::ScanRoot) -> Option<History> {
        let volume = local_volume(root)?;
        let epoch = events::boot_session()
            .zip(events::journal_identity(root.path()))
            .map(|(boot, journal)| format!("{boot}:{journal}"))?;
        Some(History { epoch, volume })
    }
    fn cursor(&self) -> Option<u64> {
        events::current_id()
    }
    fn replay<T: Send>(
        &self,
        root: &Path,
        history: &History,
        since: u64,
        cancel: &AtomicBool,
        load: impl FnOnce(&AtomicBool) -> Result<T, String> + Send,
    ) -> Result<(T, events::Changes), String> {
        events::replay_with_abortable_load_on_volume(root, &history.volume, since, cancel, load)
    }
}

#[cfg(target_os = "macos")]
fn local_volume(root: &disk_scan::ScanRoot) -> Option<PathBuf> {
    use std::os::fd::{AsFd, AsRawFd};
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstatfs(root.as_fd().as_raw_fd(), &mut stat) } != 0 {
        return None;
    }
    let (path, local) = crate::platform::mount_of(&stat);
    (local && path.is_absolute()).then_some(path)
}
#[cfg(not(target_os = "macos"))]
fn local_volume(_: &disk_scan::ScanRoot) -> Option<PathBuf> {
    None
}

/// Benchmark builds time each phase through this hook; normal builds have no timers at all.
#[cfg(feature = "performance-benchmark")]
pub(crate) type Timing<'a> = &'a (dyn Fn(&'static str, Instant) + Sync);

// Benchmark-only spans; normal builds evaluate the expression without timers or storage.
macro_rules! span {
    ($timing:expr, $phase:literal, $operation:expr) => {{
        #[cfg(feature = "performance-benchmark")]
        let began = Instant::now();
        let result = $operation;
        #[cfg(feature = "performance-benchmark")]
        ($timing)($phase, began);
        result
    }};
}

/// The rescan cache, behind the one lock that every probe, record check, refresh and save takes.
#[derive(Default)]
pub(crate) struct Baselines {
    cache: Mutex<Option<cache::Cache>>,
}

/// One scan's question: what, if anything, to start from.
pub(crate) struct Request<'a> {
    pub(crate) root: &'a disk_scan::ScanRoot,
    /// The finished scan of the same root that `slots.rs` took, kept in memory.
    pub(crate) previous: Option<Taken>,
    pub(crate) started_at_ms: u64,
    pub(crate) cancel: &'a AtomicBool,
    /// The scan's options, which the speculative rescan uses too.
    pub(crate) options: disk_scan::ScanOptions,
    #[cfg(feature = "performance-benchmark")]
    pub(crate) timing: Timing<'a>,
}

/// How the scan starts.
#[allow(clippy::large_enum_variant)] // Built and moved once per scan; the seed is now an `Arc`, so only `Prepared` is big.
pub(crate) enum Start {
    /// The speculative rescan already is the finished result. `seed` stays because a scan cancelled
    /// before it uses the result falls back to reconciling from the baseline.
    Prepared {
        snapshot: Snapshot,
        stats: disk_scan::ReuseStats,
        seed: Box<disk_scan::IncrementalSeed>,
    },
    /// Scan incrementally from the baseline.
    Seed(disk_scan::IncrementalSeed),
    /// Measure everything. The reason is shown as the scan's fallback.
    Full(String),
}

/// Held by a scan whose root has event history, from `acquire` to the end of the scan.
pub(crate) struct Ticket<'a> {
    cache: &'a Baselines,
    /// This scan's start cursor, with the baseline's `full_scan_at_ms` when it started from one.
    token: Token,
    /// The baseline's token, if the scan started from one.
    prior: Option<Token>,
    started_at_ms: u64,
}

/// What the scan itself reported that publishing needs. `Default` is a scan that failed.
#[derive(Default)]
pub(crate) struct Scanned {
    /// The scan could not reuse the baseline and measured everything.
    pub(crate) fell_back: bool,
    /// The scan proved the tree equals the baseline's.
    pub(crate) unchanged_metadata: bool,
}

/// What publishing leaves for the scan's session.
pub(crate) struct Published {
    pub(crate) token: Token,
    /// Set when the disk write failed; the token is still good as an in-memory baseline.
    pub(crate) warning: Option<String>,
}

enum Source {
    Memory(Box<Taken>),
    Disk(cache::Candidate),
}

struct PreparedRescan {
    snapshot: Snapshot,
    stats: disk_scan::ReuseStats,
}

impl Baselines {
    pub(crate) fn configure(&self, dir: PathBuf) {
        *self.lock() = Some(cache::Cache::new(dir));
    }
    // A scan thread that panicked while holding the cache lock poisons it; the cache's files stay
    // consistent (temp file, then rename; leftovers are swept), so other scans keep using it.
    fn lock(&self) -> MutexGuard<'_, Option<cache::Cache>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// A scan thread that panics while it holds the cache lock poisons it.
    #[cfg(test)]
    pub(crate) fn poison(&self) {
        std::thread::scope(|scope| {
            let panicked = scope
                .spawn(|| {
                    let _guard = self.cache.lock().unwrap();
                    panic!("scan thread failed while publishing");
                })
                .join();
            assert!(panicked.is_err());
        });
        assert!(self.cache.is_poisoned());
    }
    /// Before the scan: chooses what it starts from. The ticket, when the root has event history,
    /// goes back to `publish` after the scan. `started` runs once the cursor is taken and before the
    /// baseline is chosen, on every path.
    pub(crate) fn acquire<'a, J: Journal>(
        &'a self,
        journal: &J,
        request: Request<'_>,
        started: impl FnOnce(),
    ) -> (Start, Option<Ticket<'a>>) {
        let root = request.root;
        let started_at_ms = request.started_at_ms;
        let configured = self
            .lock()
            .as_ref()
            .is_some_and(|c| !c.contains_storage(root.path()));
        let history = if configured && root.path() != Path::new("/") {
            journal.history(root)
        } else {
            None
        };
        // Start cursor precedes replay and enumeration. Events during either operation
        // remain in the next scan's replay, rather than advancing past unseen writes.
        let start = history.and_then(|history| {
            journal.cursor().filter(|id| *id != 0).map(|cursor| {
                let token = Token {
                    epoch: history.epoch.clone(),
                    cursor,
                    full_scan_at_ms: started_at_ms,
                };
                (token, history)
            })
        });
        started();
        let Some((token, history)) = start else {
            return (
                Start::Full(
                    "This location requires a full scan or event history is unavailable".into(),
                ),
                None,
            );
        };
        let (start, prior) = self.choose(journal, request, &token, &history);
        let mut token = token;
        if let Some(prior) = &prior {
            token.full_scan_at_ms = prior.full_scan_at_ms;
        }
        (
            start,
            Some(Ticket {
                cache: self,
                token,
                prior,
                started_at_ms,
            }),
        )
    }
    fn choose<J: Journal>(
        &self,
        journal: &J,
        request: Request<'_>,
        token: &Token,
        history: &History,
    ) -> (Start, Option<Token>) {
        let Request {
            root,
            previous,
            started_at_ms,
            cancel,
            options,
            #[cfg(feature = "performance-benchmark")]
            timing,
        } = request;
        let mut fallback = None;
        let source = match previous {
            Some(previous)
                if previous.token.epoch == token.epoch
                    && eligible(&previous.snapshot)
                    && cache::recent(&previous.token, started_at_ms, now_ms())
                    && previous
                        .snapshot
                        .node(0)
                        .is_some_and(|n| &n.identity == root.identity()) =>
            {
                Some(Source::Memory(Box::new(previous)))
            }
            _ => match span!(
                timing,
                "cache_probe",
                self.lock()
                    .as_ref()
                    .unwrap()
                    .probe(root.path(), root.identity(), &token.epoch)
            ) {
                Ok(result) => result.map(Source::Disk),
                Err(e) => {
                    fallback = Some(format!("Rescan cache unavailable: {e}"));
                    None
                }
            },
        };
        let Some(source) = source else {
            let reason =
                fallback.unwrap_or_else(|| "No compatible recent clean scan is cached".into());
            return (Start::Full(reason), None);
        };
        let since = match &source {
            Source::Memory(cached) => cached.token.cursor,
            Source::Disk(candidate) => candidate.token.cursor,
        };
        let loaded = span!(
            timing,
            "journal_replay",
            journal.replay(root.path(), history, since, cancel, |abort| {
                let cached = match source {
                    Source::Memory(cached) => *cached,
                    Source::Disk(candidate) => {
                        // Only the record check holds the cache lock. The archive decodes
                        // without it, so another scan can publish meanwhile.
                        let pending = self.lock().as_ref().unwrap().begin_load(
                            candidate,
                            root.path(),
                            root.identity(),
                            &token.epoch,
                        )?;
                        span!(
                            timing,
                            "cache_decode_validate",
                            pending.decode(root.path(), root.identity())
                        )?
                    }
                };
                if !eligible(&cached.snapshot) {
                    return Err("Clone sharing or uncertain accounting requires a full scan".into());
                }
                // Shared, not copied: the speculative rescan and the seed both read this tree.
                let snapshot = Arc::new(cached.snapshot);
                let prepared = prepare_rescan(
                    root,
                    cancel,
                    abort,
                    options,
                    #[cfg(feature = "performance-benchmark")]
                    timing,
                    &snapshot,
                );
                Ok((snapshot, cached.token, prepared))
            })
        )
        .and_then(|((snapshot, prior, prepared), changes)| {
            let changes = span!(timing, "alias_resolution", changes.resolve(&snapshot))?;
            Ok((snapshot, prior, prepared, changes))
        });
        match loaded {
            Ok((snapshot, prior, prepared, changes))
                if journal
                    .history(root)
                    .is_some_and(|now| now.epoch == token.epoch) =>
            {
                let prepared = if cancel.load(Ordering::Acquire) {
                    None
                } else {
                    choose_prepared(&changes, prepared)
                };
                let seed = disk_scan::IncrementalSeed {
                    snapshot,
                    dirty_paths: changes.paths,
                    dirty_directories: changes.directories,
                };
                let start = match prepared {
                    Some(PreparedRescan { snapshot, stats }) => Start::Prepared {
                        snapshot,
                        stats,
                        seed: Box::new(seed),
                    },
                    None => Start::Seed(seed),
                };
                (start, Some(prior))
            }
            Ok(_) => (Start::Full("Filesystem event journal changed".into()), None),
            Err(error) => (
                Start::Full(format!("Rescan cache unavailable: {error}")),
                None,
            ),
        }
    }
}

impl Ticket<'_> {
    /// After the scan: publishes a clean, uncancelled result for the next scan to start from, and
    /// returns its token; `None` when nothing is published. `about_to_write` runs once, only when it
    /// will publish, before the cache lock is taken; it returns `None` to decline (the scan was cancelled
    /// meanwhile). Dropping the ticket publishes nothing.
    pub(crate) fn publish(
        self,
        snapshot: &Snapshot,
        cancel: &AtomicBool,
        scan: Scanned,
        about_to_write: impl FnOnce() -> Option<cache_store::ScanMetadata>,
        #[cfg(feature = "performance-benchmark")] timing: Timing<'_>,
    ) -> Option<Published> {
        if !eligible(snapshot) || cancel.load(Ordering::Acquire) {
            return None;
        }
        let mut token = self.token;
        if scan.fell_back {
            token.full_scan_at_ms = self.started_at_ms;
        }
        let metadata = about_to_write()?;
        let publication = span!(
            timing,
            "cache_publication",
            (|| {
                let cache = self.cache.lock();
                let cache = cache.as_ref().unwrap();
                if scan.unchanged_metadata {
                    if let Some(previous) = self.prior.as_ref() {
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
                cache.save(snapshot, &metadata, &token)
            })()
        );
        Some(Published {
            token,
            warning: publication
                .err()
                .map(|error: String| format!("Could not keep the rescan cache: {error}")),
        })
    }
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
// `abort` is set when the journal replay failed: the walk is then pure waste, because the
// scan runs in full anyway, so it stops like a cancelled scan and prepares nothing.
fn prepare_rescan(
    root: &disk_scan::ScanRoot,
    scan_cancel: &AtomicBool,
    abort: &AtomicBool,
    options: disk_scan::ScanOptions,
    #[cfg(feature = "performance-benchmark")] timing: Timing<'_>,
    cached: &Arc<Snapshot>,
) -> Option<PreparedRescan> {
    if !cached.incremental_accounting_eligible() || cached.iter_nodes().any(|node| node.hard_link) {
        return None;
    }
    let authority = root.reopen().ok()?;
    let store = Arc::new(RwLock::new(Snapshot::empty(cached.root.clone())));
    let stopped = || scan_cancel.load(Ordering::Relaxed) || abort.load(Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(stopped()));
    let result = span!(
        timing,
        "speculative_reconcile",
        disk_scan::scan_live_incremental_authorized(
            authority,
            options,
            cancel.clone(),
            store.clone(),
            |_| {
                if stopped() {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            disk_scan::IncrementalSeed {
                snapshot: cached.clone(),
                dirty_paths: vec![],
                dirty_directories: vec![]
            },
        )
    );
    result
        .ok()
        .filter(|stats| stats.fallback_reason.is_none())
        .and_then(|stats| {
            let snapshot = Arc::try_unwrap(store).ok()?.into_inner().ok()?;
            (snapshot.state == ScanState::Completed
                && !snapshot.stats.incomplete
                && snapshot.incremental_accounting_eligible()
                && !stopped())
            .then_some(PreparedRescan { snapshot, stats })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{install, snapshot};
    use std::{collections::BTreeMap, fs, sync::atomic::AtomicUsize};

    const EPOCH: &str = "boot:journal";
    const NO_HISTORY: &str = "This location requires a full scan or event history is unavailable";
    const NOT_CACHED: &str = "No compatible recent clean scan is cached";
    const UNAVAILABLE: &str = "Rescan cache unavailable: ";

    #[cfg(feature = "performance-benchmark")]
    static NO_TIMING: fn(&'static str, Instant) = |_, _| {};

    fn history(epoch: &str) -> Option<History> {
        Some(History {
            epoch: epoch.into(),
            volume: PathBuf::from("/"),
        })
    }

    /// A scripted journal: what `history` answers on each call (the last answer repeats), the cursor, what
    /// `replay` reports, and an action that runs once the load is over, as a write during decoding would.
    struct Fake {
        histories: Mutex<Vec<Option<History>>>,
        history_calls: AtomicUsize,
        cursor: Mutex<Option<u64>>,
        changes: Mutex<Option<Result<events::Changes, String>>>,
        during: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        abort_load: AtomicBool,
        since: Mutex<Vec<u64>>,
        log: Mutex<Vec<&'static str>>,
    }
    impl Fake {
        fn new(cursor: u64) -> Self {
            Self {
                histories: Mutex::new(vec![history(EPOCH)]),
                history_calls: AtomicUsize::new(0),
                cursor: Mutex::new(Some(cursor)),
                changes: Mutex::default(),
                during: Mutex::default(),
                abort_load: AtomicBool::new(false),
                since: Mutex::default(),
                log: Mutex::default(),
            }
        }
        fn log(&self, entry: &'static str) {
            self.log.lock().unwrap().push(entry);
        }
        /// Starts the script over: the first call answers the first entry.
        fn script_histories(&self, histories: Vec<Option<History>>) {
            *self.histories.lock().unwrap() = histories;
            self.history_calls.store(0, Ordering::SeqCst);
        }
        fn set_cursor(&self, cursor: Option<u64>) {
            *self.cursor.lock().unwrap() = cursor;
        }
        fn report(&self, changes: Result<events::Changes, String>) {
            *self.changes.lock().unwrap() = Some(changes);
        }
        fn during_load(&self, action: impl FnOnce() + Send + 'static) {
            *self.during.lock().unwrap() = Some(Box::new(action));
        }
        /// The journal tells the loader to stop, as it does when the replay fails.
        fn abort_loader(&self) {
            self.abort_load.store(true, Ordering::SeqCst);
        }
        fn replayed_since(&self) -> Vec<u64> {
            self.since.lock().unwrap().clone()
        }
    }
    impl Journal for Fake {
        fn history(&self, _: &disk_scan::ScanRoot) -> Option<History> {
            self.log("history");
            let call = self.history_calls.fetch_add(1, Ordering::SeqCst);
            let histories = self.histories.lock().unwrap();
            histories[call.min(histories.len() - 1)].clone()
        }
        fn cursor(&self) -> Option<u64> {
            self.log("cursor");
            *self.cursor.lock().unwrap()
        }
        fn replay<T: Send>(
            &self,
            _: &Path,
            _: &History,
            since: u64,
            _: &AtomicBool,
            load: impl FnOnce(&AtomicBool) -> Result<T, String> + Send,
        ) -> Result<(T, events::Changes), String> {
            self.log("replay");
            self.since.lock().unwrap().push(since);
            // The real journal loads on another thread while its stream starts.
            let abort = AtomicBool::new(self.abort_load.load(Ordering::SeqCst));
            let loaded = std::thread::scope(|scope| scope.spawn(|| load(&abort)).join().unwrap())?;
            if let Some(action) = self.during.lock().unwrap().take() {
                action();
            }
            let changes = self
                .changes
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Ok(events::Changes::default()))?;
            Ok((loaded, changes))
        }
    }

    /// A scan's side of the baseline protocol, against a real cache directory and temp trees.
    struct Rig {
        dir: tempfile::TempDir,
        baselines: Baselines,
        journal: Fake,
        cancel: Arc<AtomicBool>,
        #[cfg(feature = "performance-benchmark")]
        phases: Mutex<Vec<&'static str>>,
    }
    impl Rig {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let baselines = Baselines::default();
            baselines.configure(dir.path().join("cache"));
            Self {
                dir,
                baselines,
                journal: Fake::new(10),
                cancel: Arc::new(AtomicBool::new(false)),
                #[cfg(feature = "performance-benchmark")]
                phases: Mutex::default(),
            }
        }
        /// A small canonical tree, scanned.
        fn tree(&self) -> Snapshot {
            let root = self.dir.path().join("tree");
            fs::create_dir_all(root.join("a")).unwrap();
            fs::create_dir_all(root.join("b/deep")).unwrap();
            fs::write(root.join("a/one"), b"first").unwrap();
            fs::write(root.join("b/deep/two"), b"second file, a little longer").unwrap();
            snapshot(fs::canonicalize(root).unwrap())
        }
        /// Every cache operation fails from here on: its directory is a regular file.
        fn break_cache(&self) {
            let file = self.dir.path().join("cache-file");
            fs::write(&file, b"not a directory").unwrap();
            self.baselines.configure(file);
        }
        /// The archives in the cache directory, by name.
        fn archives(&self) -> Vec<String> {
            let mut names: Vec<_> = fs::read_dir(self.dir.path().join("cache"))
                .map(|entries| {
                    entries
                        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                        .filter(|name| name.ends_with(".zst"))
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }
        /// Asks for the baseline, and checks that `started` ran exactly once.
        fn acquire(
            &self,
            root: &disk_scan::ScanRoot,
            previous: Option<Taken>,
            started_at_ms: u64,
        ) -> (Start, Option<Ticket<'_>>) {
            let started = AtomicUsize::new(0);
            #[cfg(feature = "performance-benchmark")]
            let timing = |phase: &'static str, _: Instant| self.phases.lock().unwrap().push(phase);
            let outcome = self.baselines.acquire(
                &self.journal,
                Request {
                    root,
                    previous,
                    started_at_ms,
                    cancel: &self.cancel,
                    options: disk_scan::ScanOptions::default(),
                    #[cfg(feature = "performance-benchmark")]
                    timing: &timing,
                },
                || {
                    started.fetch_add(1, Ordering::SeqCst);
                    self.journal.log("started");
                },
            );
            assert_eq!(started.load(Ordering::SeqCst), 1, "started runs once");
            outcome
        }
        /// Publishes; also returns how many times `about_to_write` ran.
        fn publish(
            &self,
            ticket: Ticket<'_>,
            snapshot: &Snapshot,
            cancelled: bool,
            scan: Scanned,
            started_at_ms: u64,
        ) -> (Option<Published>, usize) {
            self.publish_unless(ticket, snapshot, cancelled, false, scan, started_at_ms)
        }
        /// `declined` makes `about_to_write` decline, as a cancel that arrives after the first check does.
        fn publish_unless(
            &self,
            ticket: Ticket<'_>,
            snapshot: &Snapshot,
            cancelled: bool,
            declined: bool,
            scan: Scanned,
            started_at_ms: u64,
        ) -> (Option<Published>, usize) {
            let written = AtomicUsize::new(0);
            #[cfg(feature = "performance-benchmark")]
            let timing = |phase: &'static str, _: Instant| self.phases.lock().unwrap().push(phase);
            let published = ticket.publish(
                snapshot,
                &AtomicBool::new(cancelled),
                scan,
                || {
                    written.fetch_add(1, Ordering::SeqCst);
                    (!declined).then(|| metadata(started_at_ms))
                },
                #[cfg(feature = "performance-benchmark")]
                &timing,
            );
            (published, written.load(Ordering::SeqCst))
        }
        /// A first scan of the tree, published to the cache as of `started_at_ms`.
        fn publish_baseline(&self, snapshot: &Snapshot, started_at_ms: u64) -> Published {
            let (start, ticket) = self.acquire(&open(snapshot), None, started_at_ms);
            assert_eq!(full(start), NOT_CACHED);
            let (published, written) = self.publish(
                ticket.unwrap(),
                snapshot,
                false,
                Scanned::default(),
                started_at_ms,
            );
            assert_eq!(written, 1);
            published.unwrap()
        }
    }

    fn open(snapshot: &Snapshot) -> disk_scan::ScanRoot {
        disk_scan::ScanRoot::open(
            snapshot.root.clone(),
            snapshot.node(0).unwrap().identity.clone(),
        )
        .unwrap()
    }
    fn metadata(started_at_ms: u64) -> cache_store::ScanMetadata {
        cache_store::ScanMetadata {
            started_at_ms,
            workers: 1,
        }
    }
    fn token(cursor: u64, full_scan_at_ms: u64) -> Token {
        Token {
            epoch: EPOCH.into(),
            cursor,
            full_scan_at_ms,
        }
    }
    fn ago(ms: u64) -> u64 {
        now_ms() - ms
    }
    fn full(start: Start) -> String {
        match start {
            Start::Full(reason) => reason,
            Start::Seed(_) => panic!("expected a full scan, got a seed"),
            Start::Prepared { .. } => panic!("expected a full scan, got a prepared result"),
        }
    }
    fn seed(start: Start) -> disk_scan::IncrementalSeed {
        match start {
            Start::Seed(seed) => seed,
            Start::Full(reason) => panic!("expected a seed, got a full scan: {reason}"),
            Start::Prepared { .. } => panic!("expected a seed, got a prepared result"),
        }
    }
    fn prepared(start: Start) -> (Snapshot, disk_scan::ReuseStats) {
        match start {
            Start::Prepared {
                snapshot, stats, ..
            } => (snapshot, stats),
            Start::Full(reason) => panic!("expected a prepared result, got a full scan: {reason}"),
            Start::Seed(_) => panic!("expected a prepared result, got a seed"),
        }
    }
    /// What the scan compares: every node's identity, sizes and flags, by path.
    fn signature(s: &Snapshot) -> BTreeMap<PathBuf, (disk_scan::Identity, [u64; 3], [bool; 2])> {
        s.iter_nodes()
            .map(|n| {
                (
                    n.path().unwrap(),
                    (
                        n.identity.clone(),
                        [n.length, n.own_allocated_size, n.allocated_size],
                        [n.hard_link, n.excluded],
                    ),
                )
            })
            .collect()
    }
    /// Scans from the seed, as `run_scan` does.
    fn reconcile(
        root: &disk_scan::ScanRoot,
        seed: disk_scan::IncrementalSeed,
    ) -> (Snapshot, disk_scan::ReuseStats) {
        let store = Arc::new(RwLock::new(Snapshot::empty(root.path().to_path_buf())));
        let stats = disk_scan::scan_live_incremental_authorized(
            root.reopen().unwrap(),
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            store.clone(),
            |_| {},
            seed,
        )
        .unwrap();
        assert!(stats.fallback_reason.is_none());
        let snapshot = Arc::try_unwrap(store).ok().unwrap().into_inner().unwrap();
        (snapshot, stats)
    }
    fn dirty(path: PathBuf) -> events::Changes {
        events::Changes {
            paths: vec![path],
            ..Default::default()
        }
    }

    #[test]
    fn a_root_without_event_history_is_scanned_in_full_without_a_ticket() {
        let mut rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        let t = ago(1000);
        // No journal for the root.
        rig.journal.script_histories(vec![None]);
        let (start, ticket) = rig.acquire(&root, None, t);
        assert_eq!(full(start), NO_HISTORY);
        assert!(ticket.is_none());
        // No cursor, or a zero one.
        rig.journal.script_histories(vec![history(EPOCH)]);
        for cursor in [None, Some(0)] {
            rig.journal.set_cursor(cursor);
            let (start, ticket) = rig.acquire(&root, None, t);
            assert_eq!(full(start), NO_HISTORY);
            assert!(ticket.is_none());
        }
        // A cache inside the scanned tree would hold the tree's own files.
        rig.journal.set_cursor(Some(10));
        rig.baselines.configure(tree.root.join(".cache"));
        let (start, ticket) = rig.acquire(&root, None, t);
        assert_eq!(full(start), NO_HISTORY);
        assert!(ticket.is_none());
        // No cache at all: the journal is not even asked.
        rig.baselines = Baselines::default();
        rig.journal.log.lock().unwrap().clear();
        let (start, ticket) = rig.acquire(&root, None, t);
        assert_eq!(full(start), NO_HISTORY);
        assert!(ticket.is_none());
        assert_eq!(*rig.journal.log.lock().unwrap(), ["started"]);
    }

    #[test]
    fn the_cursor_is_taken_before_the_baseline_is_chosen_and_the_epoch_checked_again_after() {
        let rig = Rig::new();
        let tree = rig.tree();
        let t = ago(1000);
        let previous = Taken {
            snapshot: tree.clone(),
            token: token(7, t),
        };
        let (start, ticket) = rig.acquire(&open(&tree), Some(previous), t);
        prepared(start);
        assert!(ticket.is_some());
        assert_eq!(
            *rig.journal.log.lock().unwrap(),
            ["history", "cursor", "started", "replay", "history"]
        );
    }

    #[test]
    fn an_in_memory_baseline_is_reused_without_touching_the_disk_cache() {
        let rig = Rig::new();
        let tree = rig.tree();
        // Anything that reached the disk cache would fail.
        rig.break_cache();
        let (t0, t1) = (ago(1000), ago(500));
        let previous = Taken {
            snapshot: tree.clone(),
            token: token(7, t0),
        };
        let (start, ticket) = rig.acquire(&open(&tree), Some(previous), t1);
        let (snapshot, stats) = prepared(start);
        assert!(stats.fallback_reason.is_none());
        assert_eq!(signature(&snapshot), signature(&tree));
        // The replay starts where the baseline's history ended.
        assert_eq!(rig.journal.replayed_since(), [7]);
        // The new scan's token keeps the baseline's full-scan time and takes the fresh cursor.
        let (published, _) = rig.publish(ticket.unwrap(), &tree, false, Scanned::default(), t1);
        let token = published.unwrap().token;
        assert_eq!((token.cursor, token.full_scan_at_ms), (10, t0));
    }

    #[test]
    fn an_in_memory_baseline_that_does_not_fit_falls_through_to_the_cache() {
        let rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        let fits = || Taken {
            snapshot: tree.clone(),
            token: token(7, ago(1000)),
        };
        let mut other_epoch = fits();
        other_epoch.token.epoch = "other:journal".into();
        let mut stale = fits();
        stale.token.full_scan_at_ms = ago(2 * 60 * 60 * 1000);
        let mut unclean = fits();
        unclean.snapshot.state = ScanState::Failed;
        // A scan of another folder has another root identity.
        let other = rig.dir.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("file"), b"x").unwrap();
        let other_root = Taken {
            snapshot: snapshot(fs::canonicalize(other).unwrap()),
            token: token(7, ago(1000)),
        };
        for previous in [other_epoch, stale, unclean, other_root] {
            let (start, ticket) = rig.acquire(&root, Some(previous), ago(500));
            assert_eq!(full(start), NOT_CACHED);
            assert!(ticket.is_some());
        }
        assert!(rig.journal.replayed_since().is_empty());
    }

    #[test]
    fn a_published_scan_is_the_next_scans_baseline_on_disk() {
        let rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        let (t0, t1) = (ago(20_000), ago(10_000));
        let first = rig.publish_baseline(&tree, t0);
        assert_eq!(
            (
                first.token.cursor,
                first.token.full_scan_at_ms,
                first.warning
            ),
            (10, t0, None)
        );
        assert_eq!(rig.archives().len(), 1);
        // The next scan holds nothing in memory.
        rig.journal.set_cursor(Some(20));
        let (start, ticket) = rig.acquire(&root, None, t1);
        let (snapshot, stats) = prepared(start);
        assert!(stats.fallback_reason.is_none());
        assert_eq!(signature(&snapshot), signature(&tree));
        assert_eq!(rig.journal.replayed_since(), [10]);
        let (published, _) = rig.publish(ticket.unwrap(), &tree, false, Scanned::default(), t1);
        let second = published.unwrap();
        assert_eq!(
            (second.token.cursor, second.token.full_scan_at_ms),
            (20, t0)
        );
        // A scan of the same root replaces the earlier entry.
        assert_eq!(rig.archives().len(), 1);
    }

    #[test]
    fn a_file_that_changes_while_the_cache_loads_is_in_the_seed() {
        let rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        rig.publish_baseline(&tree, ago(10_000));
        let file = tree.root.join("a/one");
        let longer = b"longer data changed during cache decoding";
        rig.journal.during_load({
            let file = file.clone();
            move || fs::write(file, longer).unwrap()
        });
        rig.journal.report(Ok(dirty(file.clone())));
        rig.journal.set_cursor(Some(20));
        let (start, _) = rig.acquire(&root, None, ago(5000));
        // The journal is asked from the cached cursor, and the speculative result is not used.
        assert_eq!(rig.journal.replayed_since(), [10]);
        let seed = seed(start);
        assert_eq!(seed.dirty_paths, std::slice::from_ref(&file));
        let (reconciled, _) = reconcile(&root, seed);
        let fresh = snapshot(tree.root.clone());
        assert_eq!(signature(&reconciled), signature(&fresh));
        let size = |s: &Snapshot| {
            s.iter_nodes()
                .find(|n| n.path().unwrap() == file)
                .unwrap()
                .length
        };
        assert_eq!(size(&reconciled), longer.len() as u64);
    }

    #[test]
    fn a_failed_replay_is_a_full_scan() {
        let rig = Rig::new();
        let tree = rig.tree();
        rig.publish_baseline(&tree, ago(10_000));
        rig.journal.report(Err("dropped history".into()));
        let (start, ticket) = rig.acquire(&open(&tree), None, ago(5000));
        assert_eq!(full(start), "Rescan cache unavailable: dropped history");
        assert!(ticket.is_some());
    }

    #[test]
    fn a_journal_that_changed_while_loading_is_a_full_scan_that_starts_its_own_history() {
        let rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        let t1 = ago(5000);
        rig.publish_baseline(&tree, ago(10_000));
        // Another journal, or none, when the epoch is checked after the replay.
        for after in [history("other:journal"), None] {
            rig.journal.script_histories(vec![history(EPOCH), after]);
            let (start, ticket) = rig.acquire(&root, None, t1);
            assert_eq!(full(start), "Filesystem event journal changed");
            assert!(ticket.is_some());
        }
        rig.journal
            .script_histories(vec![history(EPOCH), history("other:journal")]);
        let (start, ticket) = rig.acquire(&root, None, t1);
        full(start);
        let (published, _) = rig.publish(ticket.unwrap(), &tree, false, Scanned::default(), t1);
        assert_eq!(published.unwrap().token.full_scan_at_ms, t1);
    }

    #[test]
    fn a_corrupt_archive_is_a_full_scan() {
        let rig = Rig::new();
        let tree = rig.tree();
        rig.publish_baseline(&tree, ago(10_000));
        let archive = rig.archives().remove(0);
        fs::write(rig.dir.path().join("cache").join(archive), b"corrupt").unwrap();
        let (start, ticket) = rig.acquire(&open(&tree), None, ago(5000));
        assert!(full(start).starts_with(UNAVAILABLE));
        assert!(ticket.is_some());
    }

    #[test]
    fn a_cache_that_cannot_be_read_or_written_still_gives_the_scan_its_token() {
        let rig = Rig::new();
        let tree = rig.tree();
        rig.break_cache();
        let t = ago(1000);
        let (start, ticket) = rig.acquire(&open(&tree), None, t);
        assert!(full(start).starts_with(UNAVAILABLE));
        let (published, written) =
            rig.publish(ticket.unwrap(), &tree, false, Scanned::default(), t);
        let published = published.unwrap();
        assert_eq!(written, 1);
        assert!(published
            .warning
            .unwrap()
            .starts_with("Could not keep the rescan cache: "));
        // The in-memory baseline does not need the disk record.
        assert_eq!(
            (published.token.cursor, published.token.full_scan_at_ms),
            (10, t)
        );
    }

    #[test]
    fn a_speculative_result_is_used_only_when_the_journal_reports_nothing() {
        let rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        let file = tree.root.join("a/one");
        let t = ago(1000);
        let previous = || {
            Some(Taken {
                snapshot: tree.clone(),
                token: token(7, t),
            })
        };
        // Nothing happened.
        let (start, _) = rig.acquire(&root, previous(), t);
        prepared(start);
        // A path.
        rig.journal.report(Ok(dirty(file.clone())));
        let (start, _) = rig.acquire(&root, previous(), t);
        assert_eq!(seed(start).dirty_paths, std::slice::from_ref(&file));
        // A listing.
        rig.journal.report(Ok(events::Changes {
            directories: vec![tree.root.clone()],
            ..Default::default()
        }));
        let (start, _) = rig.acquire(&root, previous(), t);
        assert_eq!(
            seed(start).dirty_directories,
            std::slice::from_ref(&tree.root)
        );
        // An event on an inode of the tree, resolved to its path.
        let inode = tree
            .iter_nodes()
            .find(|n| n.path().unwrap() == file)
            .unwrap()
            .identity
            .inode;
        rig.journal.report(Ok(events::Changes {
            shared_inodes: std::collections::HashSet::from([inode]),
            ..Default::default()
        }));
        let (start, _) = rig.acquire(&root, previous(), t);
        assert_eq!(seed(start).dirty_paths, [file]);
    }

    #[test]
    fn a_baseline_with_hard_links_is_never_prepared_even_when_the_journal_reports_nothing() {
        let rig = Rig::new();
        let scope = rig.dir.path().join("scope");
        fs::create_dir_all(scope.join("linked")).unwrap();
        fs::write(scope.join("linked/a"), b"old").unwrap();
        fs::hard_link(scope.join("linked/a"), scope.join("linked/b")).unwrap();
        let baseline = snapshot(fs::canonicalize(scope).unwrap());
        let root = open(&baseline);
        let t = ago(1000);
        let previous = Taken {
            snapshot: baseline,
            token: token(1, t),
        };
        let (start, _) = rig.acquire(&root, Some(previous), t);
        assert!(seed(start).dirty_paths.is_empty());
    }

    #[test]
    fn a_baseline_with_hard_links_is_never_prepared_and_sees_a_late_alias_write() {
        let rig = Rig::new();
        let scope = rig.dir.path().join("scope");
        fs::create_dir_all(scope.join("linked")).unwrap();
        fs::create_dir_all(scope.join("clean/deep")).unwrap();
        fs::write(scope.join("linked/a"), b"old").unwrap();
        fs::hard_link(scope.join("linked/a"), scope.join("linked/b")).unwrap();
        fs::write(scope.join("clean/deep/kept"), b"unchanged").unwrap();
        let external = rig.dir.path().join("external");
        fs::hard_link(scope.join("linked/a"), &external).unwrap();
        let baseline = snapshot(fs::canonicalize(scope).unwrap());
        let root = open(&baseline);
        let t = ago(1000);
        // The write through the outside alias lands after the baseline loaded, before the replay ends.
        rig.journal
            .during_load(move || fs::write(external, vec![3; 32768]).unwrap());
        rig.journal.report(Ok(events::Changes {
            uncertain_aliases: true,
            ..Default::default()
        }));
        let previous = Taken {
            snapshot: baseline.clone(),
            token: token(1, t),
        };
        let (start, ticket) = rig.acquire(&root, Some(previous), t);
        let seed = seed(start);
        assert_eq!(seed.dirty_paths.len(), 2);
        assert_eq!(
            seed.snapshot.stats.allocated_bytes,
            baseline.stats.allocated_bytes
        );
        #[cfg(feature = "performance-benchmark")]
        assert!(!rig
            .phases
            .lock()
            .unwrap()
            .contains(&"speculative_reconcile"));
        let (reconciled, stats) = reconcile(&root, seed);
        assert!(stats.reused_entries > 0);
        let fresh = snapshot(baseline.root.clone());
        assert_eq!(signature(&reconciled), signature(&fresh));
        assert_eq!(
            reconciled.stats.allocated_bytes,
            fresh.stats.allocated_bytes
        );
        // The baseline's token is carried to the next one.
        let (published, _) = rig.publish(ticket.unwrap(), &fresh, false, Scanned::default(), t);
        assert_eq!(published.unwrap().token.full_scan_at_ms, t);
    }

    #[test]
    fn a_cancelled_scan_never_uses_a_speculative_result() {
        let rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        let t = ago(1000);
        let previous = || {
            Some(Taken {
                snapshot: tree.clone(),
                token: token(7, t),
            })
        };
        // Cancelled before the speculative rescan starts.
        rig.cancel.store(true, Ordering::Relaxed);
        let (start, _) = rig.acquire(&root, previous(), t);
        let first = seed(start);
        assert_eq!(
            first.snapshot.stats.allocated_bytes,
            tree.stats.allocated_bytes
        );
        // Cancelled after it finished, while the journal replay was still running.
        rig.cancel.store(false, Ordering::Relaxed);
        rig.journal.during_load({
            let cancel = rig.cancel.clone();
            move || cancel.store(true, Ordering::Relaxed)
        });
        let (start, _) = rig.acquire(&root, previous(), t);
        seed(start);
    }

    #[test]
    fn a_loader_told_to_stop_prepares_nothing() {
        let rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        let t = ago(1000);
        let previous = || {
            Some(Taken {
                snapshot: tree.clone(),
                token: token(7, t),
            })
        };
        let (start, _) = rig.acquire(&root, previous(), t);
        prepared(start);
        // The journal reports a failed replay: the walk stops and yields no prepared result.
        rig.journal.abort_loader();
        let (start, _) = rig.acquire(&root, previous(), t);
        seed(start);
    }

    #[test]
    fn a_scan_that_failed_was_cancelled_or_is_not_clean_publishes_nothing() {
        let rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        let t = ago(1000);
        let mut failed = tree.clone();
        failed.state = ScanState::Failed;
        let mut incomplete = tree.clone();
        incomplete.stats.incomplete = true;
        let mut errors = tree.clone();
        errors.stats.errors = 1;
        for (snapshot, cancelled) in [
            (&failed, false),
            (&incomplete, false),
            (&errors, false),
            (&tree, true),
        ] {
            let (_, ticket) = rig.acquire(&root, None, t);
            let (published, written) =
                rig.publish(ticket.unwrap(), snapshot, cancelled, Scanned::default(), t);
            assert!(published.is_none());
            assert_eq!(written, 0, "about_to_write only runs when it will publish");
        }
        // A ticket that is dropped publishes nothing either.
        drop(rig.acquire(&root, None, t));
        assert!(rig.archives().is_empty());
    }

    #[test]
    fn a_scan_cancelled_as_it_is_about_to_write_publishes_nothing() {
        let rig = Rig::new();
        let tree = rig.tree();
        let (_, ticket) = rig.acquire(&open(&tree), None, ago(1000));
        let (published, asked) = rig.publish_unless(
            ticket.unwrap(),
            &tree,
            false,
            true,
            Scanned::default(),
            ago(1000),
        );
        assert!(published.is_none());
        assert_eq!(asked, 1);
        assert!(rig.archives().is_empty());
    }

    #[test]
    fn publishing_refreshes_the_record_when_nothing_changed_and_saves_otherwise() {
        let rig = Rig::new();
        let tree = rig.tree();
        let root = open(&tree);
        let (t0, t1, t2, t3) = (ago(40_000), ago(30_000), ago(20_000), ago(10_000));
        let unchanged = || Scanned {
            unchanged_metadata: true,
            ..Default::default()
        };
        // With no baseline there is nothing to refresh, so even an unchanged scan is saved.
        let (_, ticket) = rig.acquire(&root, None, t0);
        let (published, _) = rig.publish(ticket.unwrap(), &tree, false, unchanged(), t0);
        assert_eq!(published.unwrap().token.cursor, 10);
        let [first] = rig.archives().try_into().unwrap();
        // Unchanged from that baseline: its record is refreshed, no archive is written.
        rig.journal.set_cursor(Some(20));
        let (start, ticket) = rig.acquire(&root, None, t1);
        prepared(start);
        let (published, _) = rig.publish(ticket.unwrap(), &tree, false, unchanged(), t1);
        assert_eq!(published.unwrap().token.cursor, 20);
        assert_eq!(rig.archives(), std::slice::from_ref(&first));
        // The refreshed record has the new cursor, so the next replay starts there.
        rig.journal.set_cursor(Some(30));
        let (start, ticket) = rig.acquire(&root, None, t2);
        prepared(start);
        assert_eq!(rig.journal.replayed_since(), [10, 20]);
        // Not proven unchanged: saved, and the archive of the same root is replaced.
        let (published, _) = rig.publish(ticket.unwrap(), &tree, false, Scanned::default(), t2);
        assert_eq!(published.unwrap().token.cursor, 30);
        let [second] = rig.archives().try_into().unwrap();
        assert_ne!(second, first);
        // Unchanged, but the scan measured everything again: the baseline's full-scan time no longer
        // holds, so its record cannot be refreshed, and the new token starts at this scan.
        rig.journal.set_cursor(Some(40));
        let (start, ticket) = rig.acquire(&root, None, t3);
        prepared(start);
        let fell_back = Scanned {
            fell_back: true,
            unchanged_metadata: true,
        };
        let (published, _) = rig.publish(ticket.unwrap(), &tree, false, fell_back, t3);
        assert_eq!(published.unwrap().token.full_scan_at_ms, t3);
        let [third] = rig.archives().try_into().unwrap();
        assert_ne!(third, second);
    }

    #[test]
    fn a_poisoned_cache_lock_is_tolerated() {
        let rig = Rig::new();
        let tree = rig.tree();
        rig.baselines.poison();
        let t = ago(1000);
        rig.publish_baseline(&tree, t);
        assert_eq!(rig.archives().len(), 1);
        let (start, ticket) = rig.acquire(&open(&tree), None, ago(500));
        prepared(start);
        assert!(ticket.is_some());
    }

    #[cfg(feature = "performance-benchmark")]
    #[test]
    fn each_phase_is_timed_when_it_ends() {
        let rig = Rig::new();
        let tree = rig.tree();
        rig.publish_baseline(&tree, ago(10_000));
        rig.phases.lock().unwrap().clear();
        let (start, ticket) = rig.acquire(&open(&tree), None, ago(5000));
        prepared(start);
        rig.publish(ticket.unwrap(), &tree, false, Scanned::default(), ago(5000));
        assert_eq!(
            *rig.phases.lock().unwrap(),
            [
                "cache_probe",
                "cache_decode_validate",
                "speculative_reconcile",
                "journal_replay",
                "alias_resolution",
                "cache_publication"
            ]
        );
    }

    #[test]
    #[ignore = "opt-in native live-journal rejection of a late speculative change"]
    fn native_speculative_late_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("clean")).unwrap();
        let file = dir.path().join("clean/file");
        std::fs::write(&file, b"old").unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let file = root.join("clean/file");
        let baseline = snapshot(root.clone());
        let state = crate::AppState::default();
        let s = install(&state, baseline.clone());
        let authority =
            disk_scan::ScanRoot::open(root.clone(), baseline.node(0).unwrap().identity.clone())
                .unwrap();
        let volume = local_volume(&authority).expect("local native fixture volume");
        let since = events::current_id().expect("native FSEvents cursor");
        let cached = Arc::new(baseline.clone());
        let (prepared, changes) = events::replay_with_abortable_load_on_volume(
            &root,
            &volume,
            since,
            &s.cancel,
            |abort| {
                let prepared = prepare_rescan(
                    &authority,
                    &s.cancel,
                    abort,
                    disk_scan::ScanOptions::default(),
                    #[cfg(feature = "performance-benchmark")]
                    &NO_TIMING,
                    &cached,
                );
                let candidate = prepared.as_ref().expect("clean speculative reconstruction");
                assert_eq!(
                    candidate
                        .snapshot
                        .iter_nodes()
                        .find(|n| n.path().unwrap() == file)
                        .unwrap()
                        .length,
                    3
                );
                assert!(candidate.stats.reused_entries > 0);
                assert_eq!(
                    s.store
                        .read()
                        .unwrap()
                        .iter_nodes()
                        .find(|n| n.path().unwrap() == file)
                        .unwrap()
                        .length,
                    3
                );
                // The live event stream must include this write after private construction.
                std::fs::write(&file, b"larger content written after private preparation").unwrap();
                Ok(prepared)
            },
        )
        .unwrap();
        let changes = changes.resolve(&cached).unwrap();
        assert!(
            !changes.paths.is_empty() || !changes.directories.is_empty(),
            "late write must invalidate preparation: {changes:?}"
        );
        assert!(choose_prepared(&changes, prepared).is_none());
        // The rejected private tree was never published. Reconcile the original seed.
        assert_eq!(
            s.store
                .read()
                .unwrap()
                .iter_nodes()
                .find(|n| n.path().unwrap() == file)
                .unwrap()
                .length,
            3
        );
        let store = Arc::new(RwLock::new(Snapshot::empty(root.clone())));
        let reuse = disk_scan::scan_live_incremental_authorized(
            authority.reopen().unwrap(),
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            store.clone(),
            |_| {},
            disk_scan::IncrementalSeed {
                snapshot: cached,
                dirty_paths: changes.paths,
                dirty_directories: changes.directories,
            },
        )
        .unwrap();
        assert!(reuse.fallback_reason.is_none());
        let signature = |s: &Snapshot| -> std::collections::BTreeMap<_, _> {
            s.iter_nodes()
                .map(|n| {
                    (
                        n.path().unwrap(),
                        (
                            n.identity.clone(),
                            n.length,
                            n.own_allocated_size,
                            n.allocated_size,
                            n.excluded,
                        ),
                    )
                })
                .collect()
        };
        let actual = store.read().unwrap();
        let fresh = snapshot(root);
        assert_eq!(signature(&actual), signature(&fresh));
        assert_eq!(actual.stats.entries, fresh.stats.entries);
        assert_eq!(actual.stats.errors, 0);
    }
}
