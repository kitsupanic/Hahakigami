//! Where scans live, and the one way a command gets one.
//!
//! `Slots` owns the live scans (one per `Slot`, plus the discarded ones still winding down) and the scan budget. A command finds its scan with `Slots::find`, holds it with `Session::hold` before it
//! changes state or touches files, and reads a completed snapshot through `Held::completed_snapshot`. It never
//! locks a slot, reads `Session::valid` or checks `finishing` and `Completed` itself, so every command refuses
//! the same way for the same reason. Lock order: activity gate, access setup, `operation`, cleanup queue, snapshot
//! store. `Held` reads the store only on `completed_snapshot`, so holding a scan never reverses it.
use crate::{
    baseline, budget, filters, now_ms, safety, session_running, settings, sorting, ScanDetails,
};
use disk_scan::{ScanState, Snapshot};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard,
    },
};

/// The results changed under a scan: a cleanup trashed something in it, or it was replaced or discarded.
pub(crate) const RESULTS_CHANGED: &str = "Results changed after cleanup. Rescan this location.";
/// The scan is still running, finishing, cancelled or failed: nothing that needs its results can go ahead.
pub(crate) const NOT_COMPLETED: &str = "Wait for a completed scan first.";
const GONE: &str = "This scan is no longer active.";

/// Where a live scan is kept: one slot per disk (keyed by its root path), and one shared by every folder scan.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Slot {
    Disk(PathBuf),
    Folder,
}

pub(crate) struct Session {
    pub(crate) id: String,
    pub(crate) started_at_ms: u64,
    pub(crate) completed_at_ms: AtomicU64,
    pub(crate) finishing: AtomicBool,
    pub(crate) scan_details: Mutex<ScanDetails>,
    pub(crate) cache_token: Mutex<Option<baseline::Token>>,
    pub(crate) store: Arc<RwLock<Snapshot>>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) root_ancestors: Vec<(PathBuf, safety::Identity)>,
    /// The volume of each region the scan walked, keyed by the region's root node; set when a live scan completes
    /// (`scan_volumes`); an item with no recorded region fails cleanup closed.
    pub(crate) volumes: std::sync::OnceLock<HashMap<u32, safety::Volume>>,
    pub(crate) ticket: Mutex<Option<safety::Ticket>>,
    /// False once the results no longer match the disk or the scan was replaced. Goes false only through
    /// `invalidate`, and is read through `is_valid` and `check_valid`.
    valid: AtomicBool,
    /// Orders what changes a scan: cleanup, Quick Look and its own replacement. Taken by
    /// `hold`, or by `Slots::replace` for the scan it replaces.
    operation: Mutex<()>,
    pub(crate) sorts: Mutex<HashMap<(u32, bool), Vec<u32>>>,
    pub(crate) filters: Mutex<filters::Cache>,
    pub(crate) list_folder: Mutex<sorting::Cache>,
    pub(crate) issues: Mutex<(u64, Vec<PathBuf>)>,
    pub(crate) workers: usize,
    pub(crate) cached_validation_workers: usize,
    // The retained-descriptor share of this scan's budget grant; None for a scan that never ran here.
    pub(crate) descriptor_limit: Option<usize>,
    pub(crate) worker_mode: settings::WorkerMode,
    pub(crate) worker_source: String,
    pub(crate) qos: disk_scan::ScanQos,
}

/// What a scan start asks of its session: where it walks and with how many workers.
pub(crate) struct Request {
    pub(crate) root: PathBuf,
    pub(crate) root_ancestors: Vec<(PathBuf, safety::Identity)>,
    pub(crate) workers: usize,
    pub(crate) cached_validation_workers: usize,
    pub(crate) worker_mode: settings::WorkerMode,
    pub(crate) worker_source: String,
    pub(crate) qos: disk_scan::ScanQos,
}

/// What a command needs a held scan to be.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Need {
    /// A live scan whose results are still valid.
    Live,
    /// `Live`, and the scan has ended: the caller then reads `Held::completed_snapshot`.
    LiveCompleted,
}

/// A scan whose `operation` lock the command holds, so cleanup and replacement cannot interleave with it.
pub(crate) struct Held<'a> {
    s: &'a Session,
    _operation: MutexGuard<'a, ()>,
}

impl Held<'_> {
    /// The snapshot, if the scan completed. It is read-locked only for as long as the guard lives, and only
    /// here, never when the scan was held: the cleanup queue locks before the store.
    pub(crate) fn completed_snapshot(&self) -> Result<RwLockReadGuard<'_, Snapshot>, String> {
        self.s.completed()
    }
}

impl Session {
    /// A scan that was just asked for: empty and running, until its thread ends it.
    pub(crate) fn live(request: Request, descriptor_limit: Option<usize>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            started_at_ms: now_ms(),
            completed_at_ms: AtomicU64::new(0),
            finishing: AtomicBool::new(true),
            scan_details: Mutex::new(ScanDetails {
                phase: "checking".into(),
                ..Default::default()
            }),
            cache_token: Mutex::new(None),
            store: Arc::new(RwLock::new(Snapshot::empty(request.root))),
            cancel: Arc::new(AtomicBool::new(false)),
            root_ancestors: request.root_ancestors,
            volumes: Default::default(),
            ticket: Mutex::new(None),
            valid: AtomicBool::new(true),
            operation: Mutex::new(()),
            sorts: Mutex::new(HashMap::new()),
            filters: Mutex::new(filters::Cache::default()),
            list_folder: Mutex::new(sorting::Cache::default()),
            issues: Mutex::new((0, vec![])),
            workers: request.workers,
            cached_validation_workers: request.cached_validation_workers,
            descriptor_limit,
            worker_mode: request.worker_mode,
            worker_source: request.worker_source,
            qos: request.qos,
        }
    }
    pub(crate) fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }
    /// `Ok` while the results still match what was scanned. A read that ran across an invalidation checks again
    /// before it returns.
    pub(crate) fn check_valid(&self) -> Result<(), String> {
        if self.is_valid() {
            Ok(())
        } else {
            Err(RESULTS_CHANGED.into())
        }
    }
    /// The only way `valid` goes false: the scan was replaced or discarded, or a cleanup trashed something in it.
    pub(crate) fn invalidate(&self) {
        self.valid.store(false, Ordering::Release);
    }
    /// Takes the scan's `operation` lock and checks it is still usable for `need`. It does not read the store.
    pub(crate) fn hold(&self, need: Need) -> Result<Held<'_>, String> {
        let operation = self.operation.lock().unwrap();
        self.check_valid()?;
        if need == Need::LiveCompleted && self.finishing.load(Ordering::Acquire) {
            return Err(NOT_COMPLETED.into());
        }
        Ok(Held {
            s: self,
            _operation: operation,
        })
    }
    /// `hold(Need::LiveCompleted)` then `completed_snapshot`, without the `operation` lock: for callbacks that must
    /// not wait on a cleanup, like Quick Look's authority check.
    pub(crate) fn peek_completed(&self) -> Result<RwLockReadGuard<'_, Snapshot>, String> {
        self.check_valid()?;
        self.completed()
    }
    fn completed(&self) -> Result<RwLockReadGuard<'_, Snapshot>, String> {
        let snapshot = self.store.read().unwrap();
        if self.finishing.load(Ordering::Acquire) || snapshot.state != ScanState::Completed {
            return Err(NOT_COMPLETED.into());
        }
        Ok(snapshot)
    }
    /// Moves the results out as the next scan's rescan baseline, when they can be one: a live scan of the same
    /// root with a cache token and clean results. The scan is left empty.
    fn take_baseline(&self, root: &Path) -> Option<baseline::Taken> {
        let token = self.cache_token.lock().unwrap().clone()?;
        let mut snapshot = self.store.write().unwrap();
        if snapshot.root != root || !baseline::eligible(&snapshot) {
            return None;
        }
        Some(baseline::Taken {
            snapshot: std::mem::replace(&mut *snapshot, Snapshot::empty(root.to_path_buf())),
            token,
        })
    }
}

/// The live scans, at most one per slot.
/// `draining` holds discarded scans that are still winding down. They are not findable and they do not stop
/// other scans starting, but they keep their budget and still count as running for `any_running`, so
/// first-run setup waits for a walk that may be stuck on an ejected mount.
#[derive(Default)]
struct LiveSlots {
    slots: HashMap<Slot, Arc<Session>>,
    draining: Vec<Arc<Session>>,
}

impl LiveSlots {
    fn find(&self, id: &str) -> Option<Arc<Session>> {
        self.slots.values().find(|s| s.id == id).cloned()
    }
    /// Whether any scan runs, a draining one included: restart cannot overlap a scan at all. Drained scans
    /// that ended are released here.
    fn any_running(&mut self) -> bool {
        self.draining.retain(|s| session_running(s));
        !self.draining.is_empty() || self.slots.values().any(|s| session_running(s))
    }
    /// Whether this slot's own scan runs: the only thing that stops a scan starting there.
    fn slot_running(&self, key: &Slot) -> bool {
        self.slots.get(key).is_some_and(|s| session_running(s))
    }
}

/// A scan start that got its slot: the new session, the old scan's results when they can seed an incremental
/// rescan, and the scan's share of the budget, which the scan thread gives back when it ends.
pub(crate) struct Started {
    pub(crate) session: Arc<Session>,
    pub(crate) previous: Option<baseline::Taken>,
    pub(crate) grant: budget::Grant,
}

#[derive(Default)]
pub(crate) struct Slots {
    live: Mutex<LiveSlots>,
    // Workers and descriptors shared by every running scan; see budget.rs.
    budget: budget::Budget,
}

impl Slots {
    /// The live scan with this id, if its results are still valid.
    /// Takes no lock a scan holds, so read-only queries use it.
    pub(crate) fn find(&self, id: &str) -> Result<Arc<Session>, String> {
        let s = self.live.lock().unwrap().find(id).ok_or(GONE)?;
        s.check_valid()?;
        Ok(s)
    }
    /// Every live scan, for matching cleanup-queue entries against the scan of their own root.
    /// Disk slots come first, so a disk's scan wins over a folder scan of the same root.
    pub(crate) fn sessions(&self) -> Vec<(Slot, Arc<Session>)> {
        let live = self.live.lock().unwrap();
        let mut slots: Vec<_> = live
            .slots
            .iter()
            .map(|(k, s)| (k.clone(), s.clone()))
            .collect();
        slots.sort_by_key(|(key, _)| matches!(key, Slot::Folder));
        slots
    }
    /// Whether `s` still sits in a live slot: not replaced, not discarded.
    pub(crate) fn is_live(&self, s: &Arc<Session>) -> bool {
        self.live
            .lock()
            .unwrap()
            .slots
            .values()
            .any(|slot| Arc::ptr_eq(slot, s))
    }
    /// For what cannot overlap any scan, a draining one included: refuses with `reason` while one runs.
    pub(crate) fn require_idle(&self, reason: &str) -> Result<(), String> {
        if self.live.lock().unwrap().any_running() {
            return Err(reason.into());
        }
        Ok(())
    }
    /// Puts a new scan in `key`'s slot. Refuses, changing nothing, while that slot's own scan runs or the
    /// budget has nothing left. Otherwise takes the budget grant, invalidates the old scan, takes its results as
    /// the rescan baseline and installs the new session. Other slots are never touched.
    pub(crate) fn replace(&self, key: Slot, mut request: Request) -> Result<Started, String> {
        let mut live = self.live.lock().unwrap();
        let old = live.slots.get(&key).cloned();
        let _operation = old.as_ref().map(|old| old.operation.lock().unwrap());
        // Scans on other slots run side by side; only a scan already running in this slot stops the start.
        if live.slot_running(&key) {
            return Err(match key {
                Slot::Disk(_) => "This disk is already being scanned. Cancel that scan or wait for it to finish first.",
                Slot::Folder => "A folder is already being scanned. Cancel that scan or wait for it to finish first.",
            }
            .into());
        }
        // Take a share of the process-wide budget before anything is replaced. The scan thread returns it.
        let grant = self
            .budget
            .grant(request.workers.max(request.cached_validation_workers))?;
        let share = grant.share();
        request.workers = request.workers.min(share.workers);
        request.cached_validation_workers = request.cached_validation_workers.min(share.workers);
        if let Some(old) = &old {
            old.invalidate();
        }
        let previous = old
            .as_ref()
            .and_then(|old| old.take_baseline(&request.root));
        let session = Arc::new(Session::live(request, Some(share.descriptors)));
        live.slots.insert(key, session.clone());
        Ok(Started {
            session,
            previous,
            grant,
        })
    }
    /// Drops a live scan whose disk was ejected: stops it if it is running, then frees its slot.
    /// A scan that has not ended yet keeps counting as running until it does. An unknown id is a no-op.
    pub(crate) fn discard(&self, id: &str) {
        let mut live = self.live.lock().unwrap();
        let key = live
            .slots
            .iter()
            .find(|(_, s)| s.id == id)
            .map(|(key, _)| key.clone());
        let Some(s) = key.and_then(|key| live.slots.remove(&key)) else {
            return;
        };
        s.cancel.store(true, Ordering::Relaxed);
        s.invalidate();
        if session_running(&s) {
            live.draining.push(s);
        }
    }
}

#[cfg(test)]
impl Slots {
    pub(crate) fn with_budget(budget: budget::Budget) -> Self {
        Self {
            budget,
            ..Default::default()
        }
    }
    pub(crate) fn budget(&self) -> &budget::Budget {
        &self.budget
    }
    /// Puts a ready-made session in a slot, as if a scan had ended there.
    pub(crate) fn install(&self, key: Slot, s: Arc<Session>) {
        self.live.lock().unwrap().slots.insert(key, s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::baseline::Token;

    fn slots() -> Slots {
        Slots::with_budget(budget::Budget::new(budget::Share {
            workers: 4,
            descriptors: 16,
        }))
    }
    /// Room for several scans at once.
    fn roomy() -> Slots {
        Slots::with_budget(budget::Budget::new(budget::Share {
            workers: 8,
            descriptors: 128,
        }))
    }
    fn request(root: &Path) -> Request {
        Request {
            root: root.to_path_buf(),
            root_ancestors: vec![],
            workers: 4,
            cached_validation_workers: 4,
            worker_mode: settings::WorkerMode::Manual,
            worker_source: "manual".into(),
            qos: disk_scan::ScanQos::Automatic,
        }
    }
    fn dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("file"), b"abc").unwrap();
        (dir, root)
    }
    fn scan_of(root: &Path) -> Snapshot {
        disk_scan::scan(
            root.to_path_buf(),
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap()
    }
    /// A scan that has ended with these results.
    fn ended(root: &Path, snapshot: Snapshot) -> Arc<Session> {
        let mut s = Session::live(request(root), None);
        s.finishing = AtomicBool::new(false);
        s.store = Arc::new(RwLock::new(snapshot));
        Arc::new(s)
    }
    fn in_state(root: &Path, state: ScanState) -> Arc<Session> {
        let mut snapshot = Snapshot::empty(root.to_path_buf());
        snapshot.state = state;
        ended(root, snapshot)
    }
    fn token() -> Token {
        Token {
            cursor: 1,
            epoch: "epoch".into(),
            full_scan_at_ms: now_ms(),
        }
    }
    fn disk(root: &Path) -> Slot {
        Slot::Disk(root.to_path_buf())
    }
    fn end(s: &Session, state: ScanState) {
        s.store.write().unwrap().state = state;
        s.finishing.store(false, Ordering::Release);
    }

    #[test]
    fn replace_installs_a_running_scan_with_its_share_of_the_budget() {
        let slots = slots();
        let total = slots.budget.total();
        let started = slots
            .replace(disk(Path::new("/a")), request(Path::new("/a")))
            .unwrap();
        let s = &started.session;
        assert!(session_running(s));
        assert!(s.is_valid());
        assert!(Arc::ptr_eq(&slots.find(&s.id).unwrap(), s));
        assert!(slots.is_live(s));
        // Capped to what the grant gave; the scan thread owns the grant, so the budget is short until it drops.
        assert_eq!(s.workers, started.grant.share().workers);
        assert_eq!(s.descriptor_limit, Some(started.grant.share().descriptors));
        assert!(slots.budget.free().descriptors < total.descriptors);
        assert!(started.previous.is_none());
        drop(started.grant);
        assert_eq!(slots.budget.free(), total);
    }

    #[test]
    fn replace_refuses_while_its_own_slot_runs_and_changes_nothing() {
        let slots = roomy();
        let (a, b) = (Path::new("/a"), Path::new("/b"));
        let first = slots.replace(disk(a), request(a)).unwrap();
        let other = slots.replace(disk(b), request(b)).unwrap();
        let free = slots.budget.free();
        // A disk that is scanning refuses, and a folder scan refuses in its own words.
        assert_eq!(
            slots.replace(disk(a), request(a)).err().unwrap(),
            "This disk is already being scanned. Cancel that scan or wait for it to finish first."
        );
        let folder = slots.replace(Slot::Folder, request(a)).unwrap();
        assert_eq!(
            slots.replace(Slot::Folder, request(a)).err().unwrap(),
            "A folder is already being scanned. Cancel that scan or wait for it to finish first."
        );
        // Neither refusal invalidated, replaced or took a share from anything.
        assert!(first.session.is_valid() && other.session.is_valid() && folder.session.is_valid());
        assert!(Arc::ptr_eq(
            &slots.find(&first.session.id).unwrap(),
            &first.session
        ));
        assert!(Arc::ptr_eq(
            &slots.find(&other.session.id).unwrap(),
            &other.session
        ));
        assert_eq!(slots.sessions().len(), 3);
        assert_eq!(
            slots.budget.free().descriptors,
            free.descriptors - folder.grant.share().descriptors
        );
        // Once the disk's scan ended it can be replaced; the other disk is not touched by that.
        end(&first.session, ScanState::Cancelled);
        let second = slots.replace(disk(a), request(a)).unwrap();
        assert!(!first.session.is_valid());
        assert!(slots.find(&first.session.id).is_err());
        assert!(Arc::ptr_eq(
            &slots.find(&second.session.id).unwrap(),
            &second.session
        ));
        assert!(other.session.is_valid());
        assert!(slots.is_live(&other.session));
    }

    #[test]
    fn a_start_the_budget_cannot_serve_changes_nothing() {
        let slots = slots();
        let (_dir, root) = dir();
        let (a, b) = (Path::new("/a"), Path::new("/b"));
        let holders = [
            slots.replace(disk(a), request(a)).unwrap(),
            slots.replace(disk(b), request(b)).unwrap(),
        ];
        assert_eq!(slots.budget.free().descriptors, 0);
        let old = ended(&root, scan_of(&root));
        *old.cache_token.lock().unwrap() = Some(token());
        slots.install(disk(&root), old.clone());
        let error = slots.replace(disk(&root), request(&root)).err().unwrap();
        assert!(error.contains("every file"), "{error}");
        // The old scan is still there, valid and whole: its results were not taken as a baseline.
        assert!(old.is_valid());
        assert!(Arc::ptr_eq(&slots.find(&old.id).unwrap(), &old));
        assert!(old.store.read().unwrap().node_count() > 0);
        drop(holders);
    }

    #[test]
    fn replace_invalidates_the_old_scan_and_passes_its_results_on_only_when_they_can_seed_a_rescan()
    {
        let (_dir, root) = dir();
        let (_other_dir, other_root) = dir();
        // A finished scan of `root_of`, with the results edited by `edit` and a cache token if `token`.
        let old = |root_of: &Path, edit: fn(&mut Snapshot), token: bool| {
            let mut snapshot = scan_of(root_of);
            edit(&mut snapshot);
            let s = ended(root_of, snapshot);
            *s.cache_token.lock().unwrap() = token.then(self::token);
            s
        };
        let cases = [
            (
                "a clean scan of the same root with a cache token",
                old(&root, |_| {}, true),
                true,
            ),
            ("no cache token", old(&root, |_| {}, false), false),
            ("another root", old(&other_root, |_| {}, true), false),
            (
                "a cancelled scan",
                old(&root, |s| s.state = ScanState::Cancelled, true),
                false,
            ),
            (
                "a scan with issues",
                old(&root, |s| s.stats.errors = 1, true),
                false,
            ),
        ];
        for (name, old, baseline) in cases {
            let slots = slots();
            let seeded = old.store.read().unwrap().node_count();
            slots.install(disk(&root), old.clone());
            let started = slots.replace(disk(&root), request(&root)).unwrap();
            assert!(!old.is_valid(), "{name}");
            assert!(slots.find(&old.id).is_err(), "{name}");
            assert_eq!(started.previous.is_some(), baseline, "{name}");
            match started.previous {
                // The results moved out, leaving the old scan empty.
                Some(previous) => {
                    assert_eq!(previous.snapshot.node_count(), seeded, "{name}");
                    assert_eq!(old.store.read().unwrap().node_count(), 0, "{name}");
                }
                None => assert_eq!(old.store.read().unwrap().node_count(), seeded, "{name}"),
            }
        }
    }

    #[test]
    fn a_replace_in_one_slot_leaves_the_other_slots_scans_whole() {
        let slots = slots();
        let (_a, a_root) = dir();
        let (_b, b_root) = dir();
        let b = ended(&b_root, scan_of(&b_root));
        *b.cache_token.lock().unwrap() = Some(token());
        let folder = ended(&a_root, scan_of(&a_root));
        slots.install(disk(&b_root), b.clone());
        slots.install(Slot::Folder, folder.clone());
        let a = ended(&a_root, scan_of(&a_root));
        slots.install(disk(&a_root), a.clone());
        slots.replace(disk(&a_root), request(&a_root)).unwrap();
        assert!(!a.is_valid());
        for untouched in [&b, &folder] {
            assert!(untouched.is_valid());
            assert!(slots.is_live(untouched));
            assert!(untouched.store.read().unwrap().node_count() > 0);
        }
        assert!(b.cache_token.lock().unwrap().is_some());
    }

    #[test]
    fn discard_drains_a_running_scan_and_the_idle_check_stays_closed_until_it_ends() {
        let slots = slots();
        let (a, b) = (Path::new("/a"), Path::new("/b"));
        let ejected = slots.replace(disk(a), request(a)).unwrap();
        let other = slots.replace(disk(b), request(b)).unwrap();
        let id = ejected.session.id.clone();
        slots.discard(&id);
        // Gone for every lookup, cancelled and invalid, and its slot is free.
        assert_eq!(slots.find(&id).err().unwrap(), GONE);
        assert!(ejected.session.cancel.load(Ordering::Relaxed));
        assert!(!ejected.session.is_valid());
        assert!(!slots.is_live(&ejected.session));
        assert_eq!(slots.sessions().len(), 1);
        // Another disk's scan is not touched.
        assert!(other.session.is_valid());
        assert!(slots.find(&other.session.id).is_ok());
        // It still counts as running, and keeps its share of the budget, until the walk ends.
        end(&other.session, ScanState::Completed);
        assert_eq!(slots.require_idle("busy").err().as_deref(), Some("busy"));
        let held = slots.budget.free();
        end(&ejected.session, ScanState::Cancelled);
        assert!(slots.require_idle("busy").is_ok());
        assert!(slots.live.lock().unwrap().draining.is_empty());
        drop(ejected.grant);
        assert!(slots.budget.free().descriptors > held.descriptors);
        // An unknown or already discarded id does nothing.
        slots.discard(&id);
        slots.discard("missing");
        assert_eq!(slots.sessions().len(), 1);
    }

    #[test]
    fn discarding_a_finished_scan_frees_its_slot_without_closing_the_idle_check() {
        let slots = slots();
        let (_dir, root) = dir();
        let done = ended(&root, scan_of(&root));
        slots.install(disk(&root), done.clone());
        slots.discard(&done.id);
        assert!(!done.is_valid());
        assert!(slots.sessions().is_empty());
        assert!(slots.live.lock().unwrap().draining.is_empty());
        assert!(slots.require_idle("busy").is_ok());
    }

    #[test]
    fn hold_refuses_invalid_and_unfinished_scans() {
        let (_dir, root) = dir();
        let live = ended(&root, scan_of(&root));
        for need in [Need::Live, Need::LiveCompleted] {
            assert!(live.hold(need).is_ok());
        }
        // A running scan can be held, but not for what needs it completed.
        let running = Session::live(request(&root), None);
        assert!(running.hold(Need::Live).is_ok());
        assert_eq!(
            running.hold(Need::LiveCompleted).err().unwrap(),
            NOT_COMPLETED
        );
        // An invalidated scan refuses everything, and keeps refusing.
        live.invalidate();
        for need in [Need::Live, Need::LiveCompleted] {
            assert_eq!(live.hold(need).err().unwrap(), RESULTS_CHANGED);
        }
        assert_eq!(live.peek_completed().err().unwrap(), RESULTS_CHANGED);
        assert!(live.check_valid().is_err());
    }

    #[test]
    fn a_held_scan_blocks_other_holders_but_does_not_read_the_store() {
        let (_dir, root) = dir();
        let s = ended(&root, scan_of(&root));
        let held = s.hold(Need::LiveCompleted).unwrap();
        // The operation lock is taken; the store is not, so the queue can still lock before it.
        assert!(s.operation.try_lock().is_err());
        assert!(s.store.try_write().is_ok());
        let snapshot = held.completed_snapshot().unwrap();
        assert!(snapshot.node_count() > 0);
        assert!(s.store.try_write().is_err());
        drop(snapshot);
        drop(held);
        assert!(s.operation.try_lock().is_ok());
        // `peek_completed` takes no operation lock.
        let held = s.hold(Need::Live).unwrap();
        assert!(s.peek_completed().is_ok());
        drop(held);
    }

    #[test]
    fn completed_snapshot_refuses_every_state_but_completed() {
        let (_dir, root) = dir();
        for state in [ScanState::Running, ScanState::Cancelled, ScanState::Failed] {
            let s = in_state(&root, state);
            let held = s.hold(Need::LiveCompleted).unwrap();
            assert_eq!(held.completed_snapshot().err().unwrap(), NOT_COMPLETED);
            assert_eq!(s.peek_completed().err().unwrap(), NOT_COMPLETED);
        }
        // A scan whose walk ended but is still finishing (cache write) is not complete yet.
        let s = in_state(&root, ScanState::Completed);
        s.finishing.store(true, Ordering::Release);
        assert_eq!(s.hold(Need::LiveCompleted).err().unwrap(), NOT_COMPLETED);
        assert_eq!(s.peek_completed().err().unwrap(), NOT_COMPLETED);
        s.finishing.store(false, Ordering::Release);
        assert!(s
            .hold(Need::LiveCompleted)
            .unwrap()
            .completed_snapshot()
            .is_ok());
    }

    #[test]
    fn find_refuses_unknown_and_invalidated_scans() {
        let slots = slots();
        let (_dir, root) = dir();
        let live = ended(&root, scan_of(&root));
        slots.install(Slot::Folder, live.clone());
        assert_eq!(slots.find("missing").err().unwrap(), GONE);
        assert!(Arc::ptr_eq(&slots.find(&live.id).unwrap(), &live));
        // An invalidated scan that is still in its slot says why.
        live.invalidate();
        assert_eq!(slots.find(&live.id).err().unwrap(), RESULTS_CHANGED);
    }

    #[test]
    fn sessions_lists_disk_slots_before_the_folder_slot() {
        let slots = slots();
        let (_dir, root) = dir();
        for key in [Slot::Folder, disk(Path::new("/a")), disk(Path::new("/b"))] {
            slots.install(key, in_state(&root, ScanState::Completed));
        }
        let keys: Vec<_> = slots.sessions().into_iter().map(|(key, _)| key).collect();
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[2], Slot::Folder);
    }
}
