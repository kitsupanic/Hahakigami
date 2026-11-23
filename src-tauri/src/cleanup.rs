//! Moving items to the Trash: the review that issues a ticket, and the commit that spends it.
//!
//! Commands stay thin: they take the `Activity`, close Quick Look and move this off the async workers. Everything
//! between finding the scan and reporting the outcome is here: holding the scan, the ticket, `safety` verification,
//! the Trash itself (behind the `trash` seam), `Queue::outcomes` and invalidating the cleaned scan.
use crate::{
    cleanup_queue::{self, Queue},
    events, issue_index, platform, safety,
    slots::{Need, Session, Slot},
    view, NodeView,
};
use disk_scan::Snapshot;
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};

#[derive(Serialize)]
pub(crate) struct Review {
    pub(crate) token: String,
    pub(crate) items: Vec<NodeView>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrashResult {
    pub(crate) trashed: usize,
    pub(crate) failures: Vec<String>,
    pub(crate) persistence_warning: Option<String>,
    pub(crate) outcomes: Vec<TrashOutcome>,
    /// The scan was invalidated: something moved, so its results no longer match the disk.
    pub(crate) invalidated: bool,
    /// Ids of the other live scans a moved item overlaps, invalidated too: the frontend drops their slots.
    pub(crate) stale_scan_ids: Vec<String>,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrashOutcome {
    // Both review paths build Review.items in ticket.items order.
    // This index associates a receipt with that review without exposing path authority.
    pub(crate) review_index: usize,
    pub(crate) status: TrashOutcomeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) message: Option<String>,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TrashOutcomeStatus {
    Trashed,
    Failed,
}

const MOUNTS_UNREADABLE: &str = "Could not read the mounted volumes. Try again.";
/// Each error of `confirm_trashed` starts with one of these: the Trash call moved something, maybe not the reviewed item.
const WENT_TO_TRASH: &str = "The item went to the Trash";
const DIFFERENT_ITEM: &str = "A different item than the reviewed one went to the Trash";

/// The app's own data folder, set once at startup: the protection rules keep it out of the Trash.
static APP_DATA: OnceLock<PathBuf> = OnceLock::new();
pub(crate) fn set_app_data(dir: &Path) {
    // Canonical like scan paths. Set at startup, so a repeat (nothing calls twice) keeps the first.
    let _ = APP_DATA.set(std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()));
}

/// What the protection rules read from this machine: the home folder, the app data folder and the mount points.
/// Passed in, so tests can give a folder a mount.
pub(crate) struct Host {
    pub(crate) home: Option<PathBuf>,
    pub(crate) data: Option<PathBuf>,
    pub(crate) mounts: Vec<PathBuf>,
}
impl Host {
    pub(crate) fn read() -> Result<Self, String> {
        Ok(Self {
            // Canonical like scan paths, so a symlinked or off-/Users home is still matched.
            home: platform::home_dir().map(|h| std::fs::canonicalize(&h).unwrap_or(h)),
            data: APP_DATA.get().cloned(),
            mounts: platform::mounted_paths().ok_or(MOUNTS_UNREADABLE)?,
        })
    }
}

/// What a review covers.
pub(crate) enum Pick<'a> {
    /// Nodes the person selected in this scan.
    Nodes(&'a [u32]),
    /// The queue entries ready for this scan, matched against every live scan like the view.
    Queue(&'a Mutex<Queue>, &'a [(Slot, Arc<Session>)]),
}

/// The production Trash adapter. The Trash call resolves the path again, so a parent swapped for a symlink after
/// `verify` would send another item: what landed in the Trash must be the reviewed item.
pub(crate) fn native_trash(item: &safety::CheckedItem) -> Result<(), String> {
    let receipt = platform::trash(&item.path, item.identity.kind == 1)?;
    confirm_trashed(item, receipt.as_deref())
}
/// Checks, by lstat, that what sits at the Trash `receipt` is `item`. On a mismatch it stays in the Trash to put back.
fn confirm_trashed(item: &safety::CheckedItem, receipt: Option<&Path>) -> Result<(), String> {
    let receipt = receipt.ok_or_else(|| {
        format!("{WENT_TO_TRASH}, but macOS did not say where, so it could not be checked. Look in the Trash.")
    })?;
    match safety::Identity::read(receipt) {
        Ok(moved)
            if same_trashed_item(
                &moved,
                &item.identity,
                receipt.parent().is_some_and(platform::is_fat_like)
                    && std::fs::symlink_metadata(receipt).is_ok_and(|m| is_empty_file(&m))
                    && renamed_by_trash(&item.path, receipt),
            ) =>
        {
            Ok(())
        }
        Ok(_) => Err(format!(
            "{DIFFERENT_ITEM}, because a folder changed during cleanup. Open the Trash and choose Put Back: {}",
            receipt.display()
        )),
        Err(e) => Err(format!(
            "{WENT_TO_TRASH}, but could not be checked there: {e}"
        )),
    }
}

fn is_empty_file(m: &std::fs::Metadata) -> bool {
    m.is_file() && m.len() == 0
}
/// Whether `receipt` is named like `reviewed`, allowing the number the Trash adds on a name clash: `a.txt` becomes
/// `a 2.txt`, and `a` becomes `a 2`. FAT birth times are coarse, so two empty files made together share one.
fn renamed_by_trash(reviewed: &Path, receipt: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let (Some(a), Some(b)) = (reviewed.file_name(), receipt.file_name()) else {
        return false;
    };
    if a == b {
        return true;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    // A leading dot is part of the name, not an extension.
    let dot = a.iter().rposition(|&c| c == b'.').filter(|&i| i > 0);
    let (stem, ext) = a.split_at(dot.unwrap_or(a.len()));
    b.strip_prefix(stem)
        .and_then(|rest| rest.strip_suffix(ext))
        .and_then(|rest| rest.strip_prefix(b" "))
        .is_some_and(|n| !n.is_empty() && n.iter().all(u8::is_ascii_digit))
}

/// Strict identity, except for an empty file on a FAT-like volume: with no clusters its file id comes from where its
/// directory entry sits, so the move gives it a new inode. Then a birth time on both sides is required instead.
fn same_trashed_item(
    moved: &safety::Identity,
    reviewed: &safety::Identity,
    empty_on_fat: bool,
) -> bool {
    moved == reviewed
        || empty_on_fat
            && (moved.device, moved.kind) == (reviewed.device, reviewed.kind)
            && moved.birth.is_some()
            && moved.birth == reviewed.birth
}

/// Holds `s`, checks `pick`, and replaces the scan's ticket with one for exactly the reviewed items.
pub(crate) fn review(s: &Session, pick: Pick) -> Result<Review, String> {
    let held = s.hold(Need::LiveCompleted)?;
    match pick {
        Pick::Nodes(ids) => {
            let snapshot = held.completed_snapshot()?;
            if ids.is_empty() || ids.len() > 1000 {
                return Err("Select between 1 and 1,000 items.".into());
            }
            issue(s, &snapshot, ids, None)
        }
        Pick::Queue(queue, scans) => {
            // Lock order: operation (held), queue, store. The disk is checked on a copy, never under the queue lock:
            // a hung share must not stall every queue command. The ticket records the copy's revision, and `commit`
            // refuses it if the queue moved on.
            let queue = {
                let queue = queue.lock().unwrap();
                queue.require_mutable()?;
                queue.clone()
            };
            let statuses = statuses(&queue, scans)?;
            let ids = cleanup_queue::ready_nodes(&queue, &statuses, &s.id);
            if ids.is_empty() {
                return Err("No queued items are ready in the current live scan.".into());
            }
            let snapshot = held.completed_snapshot()?;
            let review = issue(s, &snapshot, &ids, Some(queue.revision))?;
            // The ticket re-read each birth time from disk; it must still match the one saved when the item was
            // queued, or a file recreated in the same slot after `statuses` would pass.
            let mut ticket = s.ticket.lock().unwrap();
            let replaced = ticket
                .as_ref()
                .is_some_and(|t| replaced_since_queued(&queue, &t.items));
            if replaced {
                *ticket = None;
                return Err("The selected item changed. Rescan before cleanup.".into());
            }
            Ok(review)
        }
    }
}
/// Whether an entry of `queue` is the reviewed item's path and volume but another file: one that reuses its device
/// and inode (FAT) has another birth time. An entry on another volume is another place. A disk plugged in again can
/// have a new device number (`CheckedItem::renumbered`): the same UUID, inode, kind and birth time are still the item.
fn replaced_since_queued(queue: &Queue, items: &[safety::CheckedItem]) -> bool {
    items.iter().any(|item| {
        queue.entries.iter().any(|c| {
            c.item.path == item.path
                && c.volume == item.volume
                && c.checked()
                    .renumbered(item.volume.as_ref(), item.identity.device)
                    .identity
                    != item.identity
        })
    })
}
fn issue(
    s: &Session,
    snapshot: &Snapshot,
    ids: &[u32],
    queue_revision: Option<u64>,
) -> Result<Review, String> {
    let selected: HashMap<PathBuf, u32> = ids
        .iter()
        .map(|&id| Ok((snapshot.path(id)?, id)))
        .collect::<Result<_, String>>()?;
    let items = selection(s, snapshot, ids, None, &Host::read()?)?;
    let issues = issue_index(s, snapshot);
    let views = items
        .iter()
        .map(|i| {
            view(
                &snapshot.node(selected[&i.path]).ok_or("Unknown node")?,
                &issues.1,
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    let token = uuid::Uuid::new_v4().to_string();
    *s.ticket.lock().unwrap() = Some(safety::Ticket {
        token: token.clone(),
        created: SystemTime::now(),
        items,
        queue_revision,
    });
    Ok(Review {
        token,
        items: views,
    })
}

/// Spends the ticket `token` on `s`: verifies every item on disk, moves each one with `trash`, records the
/// outcomes in the queue and invalidates `s` if anything moved, and with it each of the `others` (the live scans)
/// that a reviewed item overlaps. `queued` says which review the caller expects.
pub(crate) fn commit(
    s: &Session,
    queue: &Mutex<Queue>,
    token: &str,
    queued: bool,
    others: &[Arc<Session>],
    trash: impl FnMut(&safety::CheckedItem) -> Result<(), String>,
) -> Result<TrashResult, String> {
    commit_on(s, queue, token, queued, others, Host::read, trash)
}
/// A refusal the frontend tells apart from "could not be confirmed": the commit stopped before the first Trash call.
pub(crate) fn nothing_moved(error: String) -> String {
    format!("Nothing was moved: {error}")
}
/// `commit` with the home folder and mounts from `host`, read once the ticket is spent and the disk checks begin.
/// Every error it returns comes before the first Trash call (`execute` ends in `Ok`, failures inside it are
/// outcomes), so each starts with `nothing_moved`'s prefix, once, here.
pub(crate) fn commit_on(
    s: &Session,
    queue: &Mutex<Queue>,
    token: &str,
    queued: bool,
    others: &[Arc<Session>],
    host: impl FnOnce() -> Result<Host, String>,
    trash: impl FnMut(&safety::CheckedItem) -> Result<(), String>,
) -> Result<TrashResult, String> {
    spend(s, queue, token, queued, others, host, trash).map_err(nothing_moved)
}
fn spend(
    s: &Session,
    queue: &Mutex<Queue>,
    token: &str,
    queued: bool,
    others: &[Arc<Session>],
    host: impl FnOnce() -> Result<Host, String>,
    trash: impl FnMut(&safety::CheckedItem) -> Result<(), String>,
) -> Result<TrashResult, String> {
    let held = s.hold(Need::LiveCompleted)?;
    const EXPIRED: &str = "Cleanup review expired. Review the selection again.";
    let ticket = {
        let mut slot = s.ticket.lock().unwrap();
        // A wrong token must not spend or clear the ticket it does not match.
        if slot
            .as_ref()
            .ok_or("Review the selection before cleanup.")?
            .token
            != token
        {
            return Err(EXPIRED.into());
        }
        slot.take().ok_or(EXPIRED)?
    };
    if ticket.expired() {
        return Err(EXPIRED.into());
    }
    // The queue lock is not held while the disk is checked and the Trash called: a hung volume must not stall every
    // queue command. Queue edits need the activity gate, which the caller holds, so it cannot change meanwhile.
    {
        let queue = queue.lock().unwrap();
        if queued != ticket.queue_revision.is_some()
            || ticket
                .queue_revision
                .is_some_and(|revision| revision != queue.revision || queue.blocked)
        {
            return Err("Cleanup queue changed. Review ready items again.".into());
        }
    }
    // Only the root is needed, so the store is not held across the Trash calls either.
    let root = held.completed_snapshot()?.root.clone();
    let (mut result, successful, failures, moved_unconfirmed) = execute(
        &ticket.items,
        &root,
        &host()?,
        platform::mounted_paths,
        trash,
    )?;
    // A failed Trash step can still have moved something (an unconfirmed receipt): the scan no longer matches.
    if result.trashed > 0 || moved_unconfirmed {
        s.invalidate();
        result.invalidated = true;
        // Every reviewed path counts, moved or not: over-invalidating is safe, a stale scan is not.
        result.stale_scan_ids = invalidate_overlapping(s, others, &ticket.items, moved_unconfirmed);
    }
    let mut queue = queue.lock().unwrap();
    let revision = queue.revision;
    result.persistence_warning = queue.outcomes(&successful, &failures).err().or_else(|| {
        // The queue's warning is this save's only when the save happened (an unchanged queue keeps an old one).
        (queue.revision != revision)
            .then(|| queue.warning.clone())
            .flatten()
    });
    Ok(result)
}
/// Invalidates the completed, valid scans among `others` that a reviewed item overlaps (the scan holds the item, or
/// the scan's root lies in a moved folder) and returns their ids. A path prefix alone is not enough: the `/` scan
/// never walks into `/Volumes/Backup` or another mount, so it holds no item trashed there. After an
/// `unconfirmed_move` (an item left its place with no confirmed receipt) another item than the reviewed ones may
/// have gone to the Trash, from anywhere: every one of them is stale.
// shortcut: a scan still running, finishing or with a busy store is skipped, so it keeps results that miss the
// move until the user rescans it. Upgrade by invalidating it too once a running scan can be told apart.
fn invalidate_overlapping(
    s: &Session,
    others: &[Arc<Session>],
    items: &[safety::CheckedItem],
    unconfirmed_move: bool,
) -> Vec<String> {
    let mut stale = vec![];
    for other in others.iter().filter(|o| o.id != s.id) {
        let Some(snapshot) = other.try_peek_completed() else {
            continue;
        };
        if unconfirmed_move || items.iter().any(|i| overlaps(&snapshot, &i.path)) {
            other.invalidate();
            stale.push(other.id.clone());
        }
    }
    stale
}
/// Whether the scan holds `path` or its root lies inside it. macOS spells one place two ways (`/Users/me` and
/// `/System/Volumes/Data/Users/me`, `/var` and `/private/var`), so both are compared in the journal's spelling and
/// the item is looked up again under the scan's own spelling.
fn overlaps(snapshot: &Snapshot, path: &Path) -> bool {
    let (root, item) = (events::namespace(&snapshot.root), events::namespace(path));
    snapshot.root.starts_with(path)
        || root.starts_with(&item)
        || safety::node_at(snapshot, path).is_some()
        || item
            .strip_prefix(&root)
            .is_ok_and(|rest| safety::node_at(snapshot, &snapshot.root.join(rest)).is_some())
}
type Execution = (
    TrashResult,
    Vec<safety::CheckedItem>,
    Vec<(safety::CheckedItem, String)>,
    bool,
);
fn execute(
    items: &[safety::CheckedItem],
    root: &Path,
    host: &Host,
    mut mounts_now: impl FnMut() -> Option<Vec<PathBuf>>,
    mut trash: impl FnMut(&safety::CheckedItem) -> Result<(), String>,
) -> Result<Execution, String> {
    let (home, data, mounts) = (host.home.as_deref(), host.data.as_deref(), &host.mounts);
    // Verify all first, so a stale ticket moves nothing; then again just before each item's move, with the mounts
    // read anew: a disk mounted mid-batch inside a later item's folder must stop that item.
    for item in items {
        safety::verify(item, root, home, data, mounts)
            .map_err(|e| format!("{}: {e}", item.path.display()))?;
    }
    let mut result = TrashResult {
        trashed: 0,
        failures: vec![],
        persistence_warning: None,
        outcomes: vec![],
        invalidated: false,
        stale_scan_ids: vec![],
    };
    let mut successful = vec![];
    let mut failures = vec![];
    let mut moved_unconfirmed = false;
    for (review_index, item) in items.iter().enumerate() {
        let step = mounts_now()
            .ok_or_else(|| MOUNTS_UNREADABLE.to_string())
            .and_then(|now| safety::verify(item, root, home, data, &now))
            .and_then(|_| {
                trash(item).inspect_err(|e| {
                    // The failed step moved something: another item went to the Trash, or the reviewed item is no
                    // longer where it was.
                    moved_unconfirmed |= [WENT_TO_TRASH, DIFFERENT_ITEM]
                        .iter()
                        .any(|moved| e.starts_with(moved))
                        || safety::Identity::read(&item.path).as_ref() != Ok(&item.identity);
                })
            });
        match step {
            Ok(()) => {
                result.trashed += 1;
                successful.push(item.clone());
                result.outcomes.push(TrashOutcome {
                    review_index,
                    status: TrashOutcomeStatus::Trashed,
                    message: None,
                });
            }
            Err(error) => {
                failures.push((item.clone(), error.clone()));
                result.outcomes.push(TrashOutcome {
                    review_index,
                    status: TrashOutcomeStatus::Failed,
                    message: Some(error.clone()),
                });
                result.failures.push(error);
            }
        }
    }
    Ok((result, successful, failures, moved_unconfirmed))
}

/// Adds nodes of the held scan `s` to the queue.
pub(crate) fn add(s: &Session, queue: &Mutex<Queue>, node_ids: &[u32]) -> Result<(), String> {
    let held = s.hold(Need::LiveCompleted)?;
    // Queue before store. Queue edits need the activity gate, so this copy stays current while `held`.
    let queued = queue.lock().unwrap().clone();
    let snapshot = held.completed_snapshot()?;
    let items = selection(s, &snapshot, node_ids, Some(&queued), &Host::read()?)?;
    let candidates = cleanup_queue::candidates(&scan_view(s, &snapshot, false), items, node_ids)?;
    // Release the store before the queue: the lock order is operation, queue, store.
    drop(snapshot);
    queue.lock().unwrap().add(candidates)
}

/// The queue as the UI shows it, each entry matched against `scans`.
pub(crate) fn queue_view(
    queue: &Mutex<Queue>,
    scans: &[(Slot, Arc<Session>)],
) -> cleanup_queue::View {
    // The disk is checked on a copy, never under the queue lock: a hung share must not stall every queue command.
    let copy = queue.lock().unwrap().clone();
    let view = view_of(&copy, scans);
    // An edit that landed meanwhile makes the view stale: compute it once more from the newer queue.
    let newer = {
        let queue = queue.lock().unwrap();
        (queue.revision != copy.revision).then(|| queue.clone())
    };
    match newer {
        None => view,
        Some(copy) => view_of(&copy, scans),
    }
}
fn view_of(queue: &Queue, scans: &[(Slot, Arc<Session>)]) -> cleanup_queue::View {
    match statuses(queue, scans) {
        Ok(statuses) => cleanup_queue::view(queue, &statuses),
        Err(e) => {
            let mut view = cleanup_queue::view(queue, &HashMap::new());
            view.warning.get_or_insert(e);
            view
        }
    }
}

/// `s` and its `snapshot` as the cleanup rules see them.
pub(crate) fn scan_view<'a>(
    s: &'a Session,
    snapshot: &'a Snapshot,
    disk: bool,
) -> safety::Scan<'a> {
    safety::Scan {
        id: &s.id,
        disk,
        snapshot,
        root_ancestors: &s.root_ancestors,
        volumes: s.volumes.get(),
    }
}
/// The selected nodes of `s` as checked items, collapsed to the outermost and then verified against the scan root.
/// Only the outermost items touch the disk, and when `queued` is given they must fit its room before any of them
/// does, so a huge selection is refused without a single disk read. Items already queued (or under a queued entry)
/// are left out of the result and unchecked: adding them changes nothing.
pub(crate) fn selection(
    s: &Session,
    snapshot: &Snapshot,
    node_ids: &[u32],
    queued: Option<&Queue>,
    host: &Host,
) -> Result<Vec<safety::CheckedItem>, String> {
    if node_ids.is_empty() {
        return Err("Select at least one item.".into());
    }
    let scan = scan_view(s, snapshot, false);
    let items = node_ids
        .iter()
        .map(|&id| safety::checked_item(&scan, id))
        .collect::<Result<Vec<_>, _>>()?;
    let mut items = safety::collapse(items);
    if let Some(queued) = queued {
        queued.retain_uncovered(&mut items);
        queued.check_room(&items)?;
    }
    for item in &mut items {
        // Read before `verify`, so a file replaced in between fails it.
        item.pin_birth();
        safety::verify(
            item,
            &snapshot.root,
            host.home.as_deref(),
            host.data.as_deref(),
            &host.mounts,
        )
        .map_err(|e| format!("{}: {e}", item.path.display()))?;
    }
    Ok(items)
}
/// Runs `f` with the scans that can vouch for queue entries: Live, valid, not running and Completed. Their
/// snapshots stay read-locked while `f` runs.
fn with_usable_scans<R>(scans: &[(Slot, Arc<Session>)], f: impl FnOnce(&[safety::Scan]) -> R) -> R {
    let held: Vec<_> = scans
        .iter()
        .filter_map(|(slot, s)| {
            let snapshot = s.peek_completed().ok()?;
            Some((s, matches!(slot, Slot::Disk(_)), snapshot))
        })
        .collect();
    let usable: Vec<_> = held
        .iter()
        .map(|(s, disk, snapshot)| scan_view(s, snapshot, *disk))
        .collect();
    f(&usable)
}
/// Unreadable mount table: an error, never "nothing ready", so the UI says why instead of asking for a rescan.
fn statuses(
    queue: &Queue,
    scans: &[(Slot, Arc<Session>)],
) -> Result<HashMap<String, cleanup_queue::EntryStatus>, String> {
    if !queue.matchable() {
        return Ok(HashMap::new());
    }
    let host = Host::read()?;
    Ok(with_usable_scans(scans, |scans| {
        cleanup_queue::statuses(
            queue,
            scans,
            host.home.as_deref(),
            host.data.as_deref(),
            &host.mounts,
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        tests::{install, install_in, snapshot, Recording},
        AppState,
    };

    const PROTECTED: &str = "This location is protected from cleanup.";
    const QUEUE_CHANGED: &str = "Cleanup queue changed. Review ready items again.";
    fn host(home: Option<PathBuf>, mounts: Vec<PathBuf>) -> impl FnOnce() -> Result<Host, String> {
        move || {
            Ok(Host {
                home,
                data: None,
                mounts,
            })
        }
    }

    fn home_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        (dir, root)
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn a_scan_overlaps_an_item_spelled_the_other_way() {
        // `/var/...` and `/private/var/...` are one place, like `/Users/me` and `/System/Volumes/Data/Users/me`.
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        assert!(root.starts_with("/private/var"), "{root:?}");
        let other_spelling = |p: &Path| Path::new("/").join(p.strip_prefix("/private").unwrap());
        std::fs::create_dir(root.join("dir")).unwrap();
        std::fs::write(root.join("dir/a"), b"a").unwrap();
        let disk = snapshot(root.clone());
        let folder = snapshot(root.join("dir"));
        assert!(overlaps(&disk, &other_spelling(&root.join("dir/a"))));
        assert!(!overlaps(&disk, &other_spelling(&root.join("dir/missing"))));
        assert!(overlaps(&folder, &other_spelling(&root.join("dir/a"))));
        assert!(overlaps(&folder, &other_spelling(&root)));
        assert!(!overlaps(&folder, &other_spelling(&root.join("elsewhere"))));
    }
    /// A completed scan of `root` with a review of everything directly in it.
    fn reviewed(state: &AppState, root: &Path) -> (Arc<Session>, String) {
        let scan = snapshot(root.to_path_buf());
        let ids: Vec<_> = scan
            .iter_nodes()
            .filter(|n| n.parent == Some(0))
            .map(|n| n.id)
            .collect();
        let s = install(state, scan);
        let token = review(&s, Pick::Nodes(&ids)).unwrap().token;
        (s, token)
    }

    #[test]
    fn only_the_reviewed_item_passes_the_receipt_check() {
        let (_dir, root) = home_dir();
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let item = safety::CheckedItem {
            path: a.clone(),
            identity: safety::Identity::read(&a).unwrap(),
            ancestors: vec![],
            volume: None,
        };
        // The Trash renames the item: same device, inode, kind and birth time.
        let moved = root.join("moved");
        std::fs::rename(&a, &moved).unwrap();
        assert_eq!(confirm_trashed(&item, Some(&moved)), Ok(()));
        // Another file at the receipt: a swapped parent sent the wrong item.
        let error = confirm_trashed(&item, Some(&b)).unwrap_err();
        assert!(
            error.contains("different item") && error.contains("Put Back"),
            "{error}"
        );
        // A link to the item is not the item: the receipt is read without following it.
        let link = root.join("link");
        std::os::unix::fs::symlink(&moved, &link).unwrap();
        assert!(confirm_trashed(&item, Some(&link)).is_err());
        // Nothing at the receipt, or no receipt: it cannot be vouched for.
        assert!(confirm_trashed(&item, Some(&root.join("gone"))).is_err());
        assert!(confirm_trashed(&item, None).is_err());
    }

    #[test]
    fn a_new_inode_passes_the_receipt_check_only_for_an_empty_file_on_fat() {
        let reviewed = safety::Identity {
            device: 1,
            inode: 10,
            kind: 2,
            birth: Some((5, 6)),
        };
        let moved = safety::Identity {
            inode: 11,
            ..reviewed.clone()
        };
        assert!(!same_trashed_item(&moved, &reviewed, false));
        assert!(same_trashed_item(&moved, &reviewed, true));
        let other_birth = safety::Identity {
            birth: Some((7, 6)),
            ..moved.clone()
        };
        assert!(!same_trashed_item(&other_birth, &reviewed, true));
        let no_birth = safety::Identity {
            birth: None,
            ..moved.clone()
        };
        assert!(!same_trashed_item(&no_birth, &reviewed, true));
        let other_kind = safety::Identity { kind: 1, ..moved };
        assert!(!same_trashed_item(&other_kind, &reviewed, true));
    }

    #[test]
    fn only_an_empty_file_counts_for_the_fat_receipt_rule() {
        let (_dir, root) = home_dir();
        let (empty, full) = (root.join("empty"), root.join("full"));
        std::fs::write(&empty, b"").unwrap();
        std::fs::write(&full, b"x").unwrap();
        let meta = |p: &Path| std::fs::symlink_metadata(p).unwrap();
        assert!(is_empty_file(&meta(&empty)));
        assert!(!is_empty_file(&meta(&full)));
        assert!(!is_empty_file(&meta(&root)));
    }

    #[test]
    fn the_receipt_name_is_the_reviewed_name_or_the_trash_collision_rename() {
        let named = |a: &str, b: &str| renamed_by_trash(Path::new(a), Path::new(b));
        assert!(named("/v/a.txt", "/t/a.txt"));
        assert!(named("/v/a.txt", "/t/a 2.txt"));
        assert!(named("/v/a.txt", "/t/a 13.txt"));
        assert!(named("/v/a", "/t/a 2"));
        assert!(named("/v/.a", "/t/.a 2"));
        assert!(!named("/v/a.txt", "/t/b.txt"));
        assert!(!named("/v/a.txt", "/t/a 2.md"));
        assert!(!named("/v/a.txt", "/t/a .txt"));
        assert!(!named("/v/a.txt", "/t/a x.txt"));
        assert!(!named("/v/a", "/t/ab"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Moves only a disposable fixture to native Trash and restores it; run explicitly on macOS"]
    fn the_native_trash_receipt_is_the_reviewed_item() {
        let (_dir, root) = home_dir();
        let file = root.join("hahakigami-receipt.txt");
        std::fs::write(&file, b"disposable receipt fixture").unwrap();
        let item = safety::CheckedItem {
            path: file.clone(),
            identity: safety::Identity::read(&file).unwrap(),
            ancestors: vec![],
            volume: None,
        };
        let receipt = platform::trash(&file, false).unwrap().expect("a receipt");
        let confirmed = confirm_trashed(&item, Some(&receipt));
        std::fs::rename(&receipt, &file).expect("restore the disposable fixture");
        assert_eq!(confirmed, Ok(()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Needs hdiutil to mount a disposable FAT disk image and moves one empty file on it to native Trash; run explicitly on macOS"]
    fn an_empty_file_on_a_fat_volume_passes_the_receipt_check() {
        use std::process::Command;
        // Detaches in every case, a failed assert included.
        struct Mounted(PathBuf);
        impl Drop for Mounted {
            fn drop(&mut self) {
                let _ = Command::new("hdiutil")
                    .args(["detach", "-force"])
                    .arg(&self.0)
                    .output();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("fat.dmg");
        let created = Command::new("hdiutil")
            .args([
                "create", "-size", "8m", "-fs", "MS-DOS", "-volname", "HKTEST",
            ])
            .arg(&image)
            .output()
            .unwrap();
        assert!(created.status.success(), "{created:?}");
        // A private mount point: an existing /Volumes/HKTEST must never be written to or detached.
        let volume = dir.path().join("mount");
        std::fs::create_dir(&volume).unwrap();
        let attached = Command::new("hdiutil")
            .args(["attach", "-nobrowse", "-mountpoint"])
            .arg(&volume)
            .arg(&image)
            .output()
            .unwrap();
        assert!(attached.status.success(), "{attached:?}");
        let _mounted = Mounted(volume.clone());
        let file = volume.join("empty");
        std::fs::write(&file, b"").unwrap();
        let item = safety::CheckedItem {
            path: file.clone(),
            identity: safety::Identity::read(&file).unwrap(),
            ancestors: vec![],
            volume: None,
        };
        assert_eq!(native_trash(&item), Ok(()));
    }

    #[test]
    fn an_item_that_fails_the_receipt_check_is_a_failure_not_a_trashed_item() {
        let (_dir, root) = home_dir();
        std::fs::write(root.join("a"), b"a").unwrap();
        // What a swapped parent would have sent to the Trash instead.
        let (_other_dir, other_root) = home_dir();
        let other = other_root.join("a");
        std::fs::write(&other, b"o").unwrap();
        let state = AppState::default();
        let (s, token) = reviewed(&state, &root);
        let queue = Mutex::new(Queue::default());
        let result = commit(&s, &queue, &token, false, &[], |item| {
            // The folder changed under the move: the reviewed item left its path, another one was trashed.
            std::fs::rename(&item.path, other_root.join("moved")).unwrap();
            confirm_trashed(item, Some(&other))
        })
        .unwrap();
        assert_eq!(result.trashed, 0);
        assert_eq!(result.failures.len(), 1);
        assert!(result.failures[0].contains("different item"));
        assert_eq!(result.outcomes[0].status, TrashOutcomeStatus::Failed);
        // Something did move, so the scan no longer matches the disk.
        assert!(!s.is_valid());
        assert!(result.invalidated);
    }

    #[test]
    fn another_item_in_the_trash_invalidates_every_scan_even_when_the_reviewed_one_stayed() {
        let (_dir, root) = home_dir();
        std::fs::write(root.join("a"), b"a").unwrap();
        let (_other_dir, other_root) = home_dir();
        let other = other_root.join("a");
        std::fs::write(&other, b"o").unwrap();
        let state = AppState::default();
        let (s, token) = reviewed(&state, &root);
        let elsewhere = install_in(
            &state,
            Slot::Folder,
            "elsewhere",
            snapshot(other_root.clone()),
        );
        let queue = Mutex::new(Queue::default());
        // A parent swapped for a symlink and put back: the reviewed item is still in place, another one moved.
        let result = commit(
            &s,
            &queue,
            &token,
            false,
            std::slice::from_ref(&elsewhere),
            |item| confirm_trashed(item, Some(&other)),
        )
        .unwrap();
        assert_eq!(result.trashed, 0);
        assert!(result.invalidated && !s.is_valid());
        assert_eq!(result.stale_scan_ids, std::slice::from_ref(&elsewhere.id));
        assert!(!elsewhere.is_valid());
    }

    #[test]
    fn a_plain_trash_error_after_the_item_moved_invalidates_every_scan() {
        let (_dir, root) = home_dir();
        std::fs::write(root.join("a"), b"a").unwrap();
        let (_other_dir, other_root) = home_dir();
        let state = AppState::default();
        let (s, token) = reviewed(&state, &root);
        let elsewhere = install_in(&state, Slot::Folder, "elsewhere", snapshot(other_root));
        let queue = Mutex::new(Queue::default());
        // The item left its path, but the error is a native one with no receipt prefix: only the identity shows it.
        let result = commit(
            &s,
            &queue,
            &token,
            false,
            std::slice::from_ref(&elsewhere),
            |item| {
                std::fs::rename(&item.path, item.path.with_extension("moved")).unwrap();
                Err("The operation couldn't be completed.".into())
            },
        )
        .unwrap();
        assert_eq!(result.trashed, 0);
        assert!(result.invalidated && !s.is_valid());
        assert_eq!(result.stale_scan_ids, std::slice::from_ref(&elsewhere.id));
        assert!(!elsewhere.is_valid());
    }

    #[test]
    fn a_trash_call_without_a_receipt_invalidates_the_scan_even_when_the_reviewed_item_stayed() {
        let (_dir, root) = home_dir();
        std::fs::write(root.join("a"), b"a").unwrap();
        let state = AppState::default();
        let (s, token) = reviewed(&state, &root);
        let queue = Mutex::new(Queue::default());
        // Something went to the Trash, but macOS did not say what.
        let result = commit(&s, &queue, &token, false, &[], |item| {
            confirm_trashed(item, None)
        })
        .unwrap();
        assert_eq!(result.trashed, 0);
        assert!(result.invalidated && !s.is_valid());
    }

    #[test]
    fn commit_checks_the_disk_and_trashes_without_the_queue_lock() {
        let (_dir, root) = home_dir();
        std::fs::write(root.join("a"), b"a").unwrap();
        let state = AppState::default();
        let (s, token) = reviewed(&state, &root);
        let queue = Mutex::new(Queue::default());
        // Parked inside the Trash call, commit is past its queue check and its disk checks: the queue must be free.
        let (reached, wait_reached) = std::sync::mpsc::channel();
        let (go, wait_go) = std::sync::mpsc::channel::<()>();
        let (free, result) = std::thread::scope(|scope| {
            let work = scope.spawn(|| {
                commit(&s, &queue, &token, false, &[], move |_| {
                    reached.send(()).unwrap();
                    wait_go.recv().unwrap();
                    Ok(())
                })
            });
            wait_reached.recv().unwrap();
            let free = queue.try_lock().is_ok();
            go.send(()).unwrap();
            (free, work.join().unwrap())
        });
        assert!(free, "commit held the queue lock while checking the disk");
        let result = result.unwrap();
        assert_eq!(result.trashed, 1);
        assert!(result.invalidated);
    }

    #[test]
    fn an_old_queue_warning_is_not_reported_after_a_trash_that_left_the_queue_alone() {
        let (_dir, root) = home_dir();
        std::fs::write(root.join("a"), b"a").unwrap();
        let state = AppState::default();
        let (s, token) = reviewed(&state, &root);
        let queue = Mutex::new(Queue::default());
        queue.lock().unwrap().warning =
            Some("Queue saved, but durability could not be confirmed".into());
        let result = commit(&s, &queue, &token, false, &[], |_| Ok(())).unwrap();
        assert_eq!(result.trashed, 1);
        assert!(result.persistence_warning.is_none());
    }

    #[test]
    fn an_excluded_node_and_what_lies_under_it_are_refused_for_review_and_for_the_queue() {
        let (_dir, root) = home_dir();
        std::fs::create_dir(root.join("mount")).unwrap();
        std::fs::write(root.join("mount/inner"), b"x").unwrap();
        std::fs::write(root.join("plain"), b"p").unwrap();
        let mut scan = snapshot(root.clone());
        let id = |scan: &Snapshot, name: &str| {
            scan.iter_nodes()
                .find(|n| n.path().unwrap() == root.join(name))
                .unwrap()
                .id
        };
        let (mount, inner, plain) = (
            id(&scan, "mount"),
            id(&scan, "mount/inner"),
            id(&scan, "plain"),
        );
        scan.update_node(mount, |n| n.excluded = true).unwrap();
        let s = install(&AppState::default(), scan);
        let queue = Mutex::new(Queue::default());
        // The node itself, and a child that is not excluded but sits under an excluded folder.
        for (id, error) in [
            (
                mount,
                "Excluded locations cannot be moved to Trash from this scan.",
            ),
            (inner, "Excluded or invalid cleanup ancestor"),
        ] {
            assert_eq!(review(&s, Pick::Nodes(&[id])).err().as_deref(), Some(error));
            assert_eq!(add(&s, &queue, &[id]).err().as_deref(), Some(error));
        }
        assert!(s.ticket.lock().unwrap().is_none());
        assert!(queue.lock().unwrap().entries.is_empty());
        // An ordinary item of the same scan is reviewed.
        assert!(review(&s, Pick::Nodes(&[plain])).is_ok());
    }

    #[test]
    fn a_folder_that_gains_a_mount_or_is_home_after_review_is_refused_at_commit_with_nothing_trashed(
    ) {
        let (_dir, root) = home_dir();
        let folder = root.join("folder");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(root.join("plain"), b"p").unwrap();
        let state = AppState::default();
        let queue = Mutex::new(Queue::default());
        for (case, home, mounts) in [
            ("a mount under it", None, vec![folder.join("mnt")]),
            ("a mount at it", None, vec![folder.clone()]),
            ("it is the home folder", Some(folder.clone()), vec![]),
        ] {
            let (s, token) = reviewed(&state, &root);
            let mut trash = Recording::ok();
            let refused = commit_on(&s, &queue, &token, false, &[], host(home, mounts), |i| {
                trash.trash(i)
            });
            assert_eq!(
                refused.err(),
                Some(format!(
                    "Nothing was moved: {}: {PROTECTED}",
                    folder.display()
                )),
                "{case}"
            );
            // The whole batch is refused up front, the plain file beside the folder too.
            assert!(trash.calls.is_empty(), "{case}");
            assert!(s.is_valid(), "{case}");
        }
        // With neither, the same review goes through.
        let (s, token) = reviewed(&state, &root);
        let mut trash = Recording::ok();
        let result = commit_on(&s, &queue, &token, false, &[], host(None, vec![]), |i| {
            trash.trash(i)
        })
        .unwrap();
        assert_eq!((result.trashed, trash.calls.len()), (2, 2));
    }

    #[test]
    fn a_mount_that_appears_between_items_stops_the_later_item() {
        let (_dir, root) = home_dir();
        std::fs::create_dir(root.join("folder")).unwrap();
        std::fs::write(root.join("plain"), b"p").unwrap();
        let state = AppState::default();
        let (s, _) = reviewed(&state, &root);
        let items = s.ticket.lock().unwrap().take().unwrap().items;
        let host = Host {
            home: None,
            data: None,
            mounts: vec![],
        };
        // The second read, before the second item, finds a disk mounted inside that item.
        let second = items[1].path.join("mnt");
        let mut reads = 0;
        let mut trash = Recording::ok();
        let (result, _, _, _) = execute(
            &items,
            &root,
            &host,
            || {
                reads += 1;
                Some(if reads > 1 {
                    vec![second.clone()]
                } else {
                    vec![]
                })
            },
            |i| trash.trash(i),
        )
        .unwrap();
        assert_eq!(
            (result.trashed, trash.calls),
            (1, vec![items[0].path.clone()])
        );
        assert_eq!(result.failures, [PROTECTED]);
    }

    #[test]
    fn an_unreadable_mount_table_before_an_item_fails_it_and_moves_nothing_more() {
        let (_dir, root) = home_dir();
        std::fs::create_dir(root.join("folder")).unwrap();
        std::fs::write(root.join("plain"), b"p").unwrap();
        let state = AppState::default();
        let (s, _) = reviewed(&state, &root);
        let items = s.ticket.lock().unwrap().take().unwrap().items;
        let host = Host {
            home: None,
            data: None,
            mounts: vec![],
        };
        let mut reads = 0;
        let mut trash = Recording::ok();
        let (result, _, _, _) = execute(
            &items,
            &root,
            &host,
            || {
                reads += 1;
                (reads == 1).then(Vec::new)
            },
            |i| trash.trash(i),
        )
        .unwrap();
        assert_eq!(
            (result.trashed, trash.calls),
            (1, vec![items[0].path.clone()])
        );
        assert_eq!(result.failures, [MOUNTS_UNREADABLE]);
    }

    #[test]
    fn a_review_refuses_a_folder_that_holds_a_mount_or_is_home_or_app_data() {
        let (_dir, root) = home_dir();
        let folder = root.join("folder");
        std::fs::create_dir(&folder).unwrap();
        let state = AppState::default();
        let scan = snapshot(root.clone());
        let id = scan
            .iter_nodes()
            .find(|n| n.path().unwrap() == folder)
            .unwrap()
            .id;
        let s = install(&state, scan);
        let snapshot = s.store.read().unwrap();
        let select = |host: Host| {
            selection(&s, &snapshot, &[id], None, &host)
                .err()
                .as_deref()
                .map(str::to_owned)
        };
        let none = || Host {
            home: None,
            data: None,
            mounts: vec![],
        };
        assert_eq!(select(none()), None);
        let mount = Host {
            mounts: vec![folder.join("mnt")],
            ..none()
        };
        let named = format!("{}: {PROTECTED}", folder.display());
        assert_eq!(select(mount).as_deref(), Some(named.as_str()));
        let home = Host {
            home: Some(folder.clone()),
            ..none()
        };
        assert_eq!(select(home).as_deref(), Some(named.as_str()));
        // The app's own data folder, or a folder holding it: trashing it would free the instance lock.
        let data = Host {
            data: Some(folder.join("data")),
            ..none()
        };
        assert_eq!(select(data).as_deref(), Some(named.as_str()));
    }

    /// A queued file `a` of a completed scan, with the queue and the scans to review it against.
    fn queued_file(
        state: &AppState,
        root: &Path,
    ) -> (Arc<Session>, tempfile::TempDir, Mutex<Queue>) {
        std::fs::write(root.join("a"), b"a").unwrap();
        let storage = tempfile::tempdir().unwrap();
        let queue = Mutex::new(Queue::load(storage.path().join("queue")));
        let scan = snapshot(root.to_path_buf());
        let ids: Vec<_> = scan
            .iter_nodes()
            .filter(|n| n.parent == Some(0))
            .map(|n| n.id)
            .collect();
        let s = install(state, scan);
        add(&s, &queue, &ids).unwrap();
        (s, storage, queue)
    }

    #[test]
    fn a_queued_file_replaced_by_one_reusing_its_device_and_inode_counts_as_replaced() {
        let (_dir, root) = home_dir();
        let state = AppState::default();
        let (s, _storage, queue) = queued_file(&state, &root);
        let scans = state.scans.sessions();
        review(&s, Pick::Queue(&queue, &scans)).unwrap();
        let items = s.ticket.lock().unwrap().as_ref().unwrap().items.clone();
        assert!(!replaced_since_queued(&queue.lock().unwrap(), &items));
        // The disk's birth time no longer matches the one saved when the file was queued. Review's own status check
        // catches it first unless the file is swapped after that check, so the guard is tested on its own.
        queue.lock().unwrap().entries[0].item.identity.birth = Some((1, 0));
        assert!(replaced_since_queued(&queue.lock().unwrap(), &items));
    }

    #[test]
    fn a_same_path_entry_on_another_volume_does_not_refuse_review() {
        let (_dir, root) = home_dir();
        let state = AppState::default();
        let (s, _storage, queue) = queued_file(&state, &root);
        let scans = state.scans.sessions();
        {
            let mut queue = queue.lock().unwrap();
            let mut other = queue.entries[0].clone();
            other.id = uuid::Uuid::new_v4().to_string();
            other.volume = Some(safety::Volume::Uuid(uuid::Uuid::new_v4().to_string()));
            other.item.identity.birth = Some((1, 0));
            queue.entries.push(other);
        }
        assert!(review(&s, Pick::Queue(&queue, &scans)).is_ok());
        assert!(s.ticket.lock().unwrap().is_some());
    }

    #[test]
    fn a_ticket_commits_only_through_its_own_endpoint_and_not_while_the_queue_is_blocked() {
        let (_dir, root) = home_dir();
        std::fs::write(root.join("a"), b"a").unwrap();
        let state = AppState::default();
        let storage = tempfile::tempdir().unwrap();
        let queue = Mutex::new(Queue::load(storage.path().join("queue")));
        let scan = snapshot(root.clone());
        let ids: Vec<_> = scan
            .iter_nodes()
            .filter(|n| n.parent == Some(0))
            .map(|n| n.id)
            .collect();
        let s = install(&state, scan);
        add(&s, &queue, &ids).unwrap();
        let scans = state.scans.sessions();
        let mut trash = Recording::ok();
        // A ticket of the direct review, spent on the queue's endpoint.
        let token = review(&s, Pick::Nodes(&ids)).unwrap().token;
        let refused = commit(&s, &queue, &token, true, &[], |i| trash.trash(i));
        assert_eq!(
            refused.err(),
            Some(format!("Nothing was moved: {QUEUE_CHANGED}"))
        );
        // A ticket of the queue's review, spent on the direct endpoint.
        let token = review(&s, Pick::Queue(&queue, &scans)).unwrap().token;
        let refused = commit(&s, &queue, &token, false, &[], |i| trash.trash(i));
        assert_eq!(
            refused.err(),
            Some(format!("Nothing was moved: {QUEUE_CHANGED}"))
        );
        // A queue that turned unreadable after its review.
        let token = review(&s, Pick::Queue(&queue, &scans)).unwrap().token;
        queue.lock().unwrap().blocked = true;
        let refused = commit(&s, &queue, &token, true, &[], |i| trash.trash(i));
        assert_eq!(
            refused.err(),
            Some(format!("Nothing was moved: {QUEUE_CHANGED}"))
        );
        queue.lock().unwrap().blocked = false;
        assert!(trash.calls.is_empty());
        assert!(s.is_valid());
        // Its own endpoint, with the queue as reviewed, moves it.
        let token = review(&s, Pick::Queue(&queue, &scans)).unwrap().token;
        let result = commit(&s, &queue, &token, true, &[], |i| trash.trash(i)).unwrap();
        assert_eq!((result.trashed, trash.calls.len()), (1, 1));
    }
}
