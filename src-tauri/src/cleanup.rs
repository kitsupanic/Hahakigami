//! Moving items to the Trash: the review that issues a ticket, and the commit that spends it.
//!
//! Commands stay thin: they take the `Activity`, close Quick Look and move this off the async workers. Everything
//! between finding the scan and reporting the outcome is here: holding the scan, the ticket, `safety` verification,
//! the Trash itself (behind the `trash` seam), `Queue::outcomes` and invalidating the cleaned scan.
use crate::{
    cleanup_queue::{self, Queue},
    issue_index, platform, safety,
    slots::{Need, Session, Slot},
    view, NodeView,
};
use disk_scan::Snapshot;
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
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

/// What a review covers.
pub(crate) enum Pick<'a> {
    /// Nodes the person selected in this scan.
    Nodes(&'a [u32]),
    /// The queue entries ready for this scan, matched against every live scan like the view.
    Queue(&'a Mutex<Queue>, &'a [(Slot, Arc<Session>)]),
}

/// The production Trash adapter.
pub(crate) fn native_trash(item: &safety::CheckedItem) -> Result<(), String> {
    platform::trash(&item.path, item.identity.kind == 1)
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
            let statuses = statuses(&queue, scans);
            let ids = cleanup_queue::ready_nodes(&queue, &statuses, &s.id);
            if ids.is_empty() {
                return Err("No queued items are ready in the current live scan.".into());
            }
            let snapshot = held.completed_snapshot()?;
            let review = issue(s, &snapshot, &ids, Some(queue.revision))?;
            // The ticket re-read each birth time from disk; it must still match the one saved when the item was
            // queued, or a file recreated in the same slot after `statuses` would pass.
            let mut ticket = s.ticket.lock().unwrap();
            let replaced = ticket.as_ref().is_some_and(|t| {
                t.items.iter().any(|item| {
                    queue
                        .entries
                        .iter()
                        .any(|c| c.item.path == item.path && c.item.identity != item.identity)
                })
            });
            if replaced {
                *ticket = None;
                return Err("The selected item changed. Rescan before cleanup.".into());
            }
            Ok(review)
        }
    }
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
    let items = selection(s, snapshot, ids, None)?;
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
        created: Instant::now(),
        items,
        queue_revision,
    });
    Ok(Review {
        token,
        items: views,
    })
}

/// Spends the ticket `token` on `s`: verifies every item on disk, moves each one with `trash`, records the
/// outcomes in the queue and invalidates `s` if anything moved. `queued` says which review the caller expects.
pub(crate) fn commit(
    s: &Session,
    queue: &Mutex<Queue>,
    token: &str,
    queued: bool,
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
    let mut queue = queue.lock().unwrap();
    if queued != ticket.queue_revision.is_some()
        || ticket
            .queue_revision
            .is_some_and(|revision| revision != queue.revision || queue.blocked)
    {
        return Err("Cleanup queue changed. Review ready items again.".into());
    }
    let snapshot = held.completed_snapshot()?;
    let (mut result, successful, failures) = execute(&ticket.items, &snapshot.root, trash)?;
    if result.trashed > 0 {
        s.invalidate();
    }
    result.persistence_warning = queue
        .outcomes(&successful, &failures)
        .err()
        .or_else(|| queue.warning.clone());
    Ok(result)
}
type Execution = (TrashResult, Vec<PathBuf>, Vec<(PathBuf, String)>);
fn execute(
    items: &[safety::CheckedItem],
    root: &Path,
    mut trash: impl FnMut(&safety::CheckedItem) -> Result<(), String>,
) -> Result<Execution, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mounts = platform::mounted_paths();
    // Verify all first, so a stale ticket moves nothing; then again just before each item's move.
    for item in items {
        safety::verify(item, root, home.as_deref(), &mounts)?;
    }
    let mut result = TrashResult {
        trashed: 0,
        failures: vec![],
        persistence_warning: None,
        outcomes: vec![],
    };
    let mut successful = vec![];
    let mut failures = vec![];
    for (review_index, item) in items.iter().enumerate() {
        match safety::verify(item, root, home.as_deref(), &mounts).and_then(|_| trash(item)) {
            Ok(()) => {
                result.trashed += 1;
                successful.push(item.path.clone());
                result.outcomes.push(TrashOutcome {
                    review_index,
                    status: TrashOutcomeStatus::Trashed,
                    message: None,
                });
            }
            Err(error) => {
                failures.push((item.path.clone(), error.clone()));
                result.outcomes.push(TrashOutcome {
                    review_index,
                    status: TrashOutcomeStatus::Failed,
                    message: Some(error.clone()),
                });
                result.failures.push(error);
            }
        }
    }
    Ok((result, successful, failures))
}

/// Adds nodes of the held scan `s` to the queue.
pub(crate) fn add(s: &Session, queue: &Mutex<Queue>, node_ids: &[u32]) -> Result<(), String> {
    let held = s.hold(Need::LiveCompleted)?;
    // Queue before store. Queue edits need the activity gate, so this copy stays current while `held`.
    let queued = queue.lock().unwrap().clone();
    let snapshot = held.completed_snapshot()?;
    let items = selection(s, &snapshot, node_ids, Some(&queued))?;
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
    let view = cleanup_queue::view(&copy, &statuses(&copy, scans));
    // An edit that landed meanwhile makes the view stale: compute it once more from the newer queue.
    let newer = {
        let queue = queue.lock().unwrap();
        (queue.revision != copy.revision).then(|| queue.clone())
    };
    match newer {
        None => view,
        Some(copy) => cleanup_queue::view(&copy, &statuses(&copy, scans)),
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
/// does, so a huge selection is refused without a single disk read.
pub(crate) fn selection(
    s: &Session,
    snapshot: &Snapshot,
    node_ids: &[u32],
    queued: Option<&Queue>,
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
        queued.check_room(&items)?;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mounts = platform::mounted_paths();
    for item in &mut items {
        // Read before `verify`, so a file replaced in between fails it.
        item.pin_birth();
        safety::verify(item, &snapshot.root, home.as_deref(), &mounts)?;
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
fn statuses(
    queue: &Queue,
    scans: &[(Slot, Arc<Session>)],
) -> HashMap<String, cleanup_queue::EntryStatus> {
    if !queue.matchable() {
        return HashMap::new();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mounts = platform::mounted_paths();
    with_usable_scans(scans, |scans| {
        cleanup_queue::statuses(queue, scans, home.as_deref(), &mounts)
    })
}
