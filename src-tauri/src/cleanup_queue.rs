//! Persisted candidates are evidence only. Live sessions and Trash tickets own action authority.
use crate::safety::{self, CheckedItem, Identity, Scan, Volume};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ffi::{CString, OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::Read,
    os::fd::{AsRawFd, FromRawFd},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};
const MAX_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_ENTRIES: usize = 1000;
// Where Clear keeps an unreadable record: one file, replaced by the next such Clear.
const QUARANTINE: &std::ffi::CStr = c"queue.quarantine";
const FULL: &str = "Cleanup queue supports at most 1,000 entries. Remove items first.";
mod native_path {
    use super::*;
    pub fn serialize<S: serde::Serializer>(path: &Path, s: S) -> Result<S::Ok, S::Error> {
        path.as_os_str().as_bytes().serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        let bytes = Vec::<u8>::deserialize(d)?;
        if bytes.is_empty() || bytes.len() > 32768 || bytes.contains(&0) {
            return Err(serde::de::Error::custom("Invalid native queue path"));
        }
        let path = PathBuf::from(OsString::from_vec(bytes));
        if !path.is_absolute()
            || path.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
        {
            return Err(serde::de::Error::custom("Invalid queue path"));
        }
        Ok(path)
    }
}
fn open_at(
    directory: &File,
    name: &std::ffi::CStr,
    flags: i32,
    mode: libc::mode_t,
) -> std::io::Result<File> {
    // Names are generated constants or UUID basenames, and directory stays open
    // through publication so a directory-path replacement cannot redirect writes.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn rename_at(directory: &File, from: &std::ffi::CStr, to: &std::ffi::CStr) -> std::io::Result<()> {
    if unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            from.as_ptr(),
            directory.as_raw_fd(),
            to.as_ptr(),
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
/// Whether `queue.json` must be set aside before a recovery publish replaces it: it exists, and it is not a
/// regular file or it belongs to a queue that could not be loaded (a newer version may have written it).
fn record_needs_quarantine(directory: &File, blocked: bool) -> std::io::Result<bool> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            c"queue.json".as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(false);
        }
        return Err(error);
    }
    let stat = unsafe { stat.assume_init() };
    Ok(blocked || stat.st_mode & libc::S_IFMT != libc::S_IFREG)
}
/// Removes the `<uuid>.tmp` files a crash between `publish`'s write and its rename left behind (up to 16 MB
/// each). Only regular files named `*.tmp` go: symlinks, directories and the quarantined record stay. Best effort.
fn remove_stale_temps(directory: &File) {
    // Lists the directory through the descriptor opened without following symlinks, not through its path.
    let fd = unsafe { libc::dup(directory.as_raw_fd()) };
    if fd < 0 {
        return;
    }
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        unsafe { libc::close(fd) };
        return;
    }
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        let (name, kind) = unsafe {
            (
                std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()),
                (*entry).d_type,
            )
        };
        // `unlinkat` never follows a symlink, and without AT_REMOVEDIR it leaves directories alone.
        if kind == libc::DT_REG && name.to_bytes().ends_with(b".tmp") {
            unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) };
        }
    }
    unsafe { libc::closedir(stream) };
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Guard {
    #[serde(with = "native_path")]
    pub path: PathBuf,
    pub identity: Identity,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Candidate {
    pub id: String,
    pub item: Guard,
    pub ancestors: Vec<Guard>,
    pub root: Guard,
    pub root_ancestors: Vec<Guard>,
    pub allocated_bytes: u64,
    #[serde(default)]
    pub last_error: Option<String>,
    /// The volume of the scan the entry was queued from. Missing in records written before it existed: such an
    /// entry never reads ready and is never trashed (`safety::verify` rejects an unknown volume).
    #[serde(default)]
    pub volume: Option<Volume>,
}
impl Candidate {
    pub fn checked(&self) -> CheckedItem {
        CheckedItem {
            path: self.item.path.clone(),
            identity: self.item.identity.clone(),
            ancestors: self
                .ancestors
                .iter()
                .map(|g| (g.path.clone(), g.identity.clone()))
                .collect(),
            volume: self.volume.clone(),
        }
    }
    pub fn root_checked(&self) -> CheckedItem {
        CheckedItem {
            path: self.root.path.clone(),
            identity: self.root.identity.clone(),
            ancestors: self
                .root_ancestors
                .iter()
                .map(|g| (g.path.clone(), g.identity.clone()))
                .collect(),
            // Only the item's volume is recorded: it is the root's when both were on one device (a folder scan can
            // cross a mount). Otherwise the root's volume is unknown and its checks stay strict.
            volume: (self.root.identity.device == self.item.identity.device)
                .then(|| self.volume.clone())
                .flatten(),
        }
    }
    fn valid(&self) -> bool {
        let lineage_valid = |path: &Path, guards: &[Guard]| {
            guards.len() <= 1024
                && path
                    .ancestors()
                    .skip(1)
                    .eq(guards.iter().map(|g| g.path.as_path()))
                && guards.iter().all(|g| g.identity.kind == 1)
        };
        uuid::Uuid::parse_str(&self.id).is_ok_and(|id| id.to_string() == self.id)
            && self
                .last_error
                .as_ref()
                .is_none_or(|error| error.len() <= 32768)
            && self.volume.as_ref().is_none_or(Volume::valid)
            && self.item.path != self.root.path
            && self.item.path.starts_with(&self.root.path)
            && (1..=4).contains(&self.item.identity.kind)
            && self.root.identity.kind == 1
            && lineage_valid(&self.item.path, &self.ancestors)
            && lineage_valid(&self.root.path, &self.root_ancestors)
            && self
                .ancestors
                .iter()
                .any(|g| g.path == self.root.path && g.identity == self.root.identity)
    }
}
#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    entries: Vec<Candidate>,
}
#[derive(Clone, Default)]
pub(crate) struct Queue {
    directory: Option<PathBuf>,
    pub entries: Vec<Candidate>,
    pub revision: u64,
    pub warning: Option<String>,
    pub blocked: bool,
    #[cfg(test)]
    fail_publication: bool,
}
impl Queue {
    pub fn load(directory: PathBuf) -> Self {
        let mut q = Self {
            directory: Some(directory.clone()),
            ..Self::default()
        };
        let read = (|| -> Result<Vec<Candidate>, std::io::Error> {
            // Validate the private parent too; do not traverse an attacker-replaced directory.
            let dir = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
                .open(&directory)?;
            remove_stale_temps(&dir);
            let file = open_at(&dir, c"queue.json", libc::O_RDONLY | libc::O_NONBLOCK, 0)?;
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::other("Queue record is not regular"));
            }
            let mut bytes = vec![];
            file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(std::io::Error::other("Queue record is too large"));
            }
            let record: Record = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
            if record.version != 1
                || record.entries.len() > MAX_ENTRIES
                || record.entries.iter().any(|c| !c.valid())
            {
                return Err(std::io::Error::other("Unsupported or invalid queue record"));
            }
            let mut ids = std::collections::HashSet::new();
            for (i, c) in record.entries.iter().enumerate() {
                if !ids.insert(&c.id)
                    || record.entries[..i].iter().any(|p| {
                        p.volume == c.volume
                            && (c.item.path.starts_with(&p.item.path)
                                || p.item.path.starts_with(&c.item.path))
                    })
                {
                    return Err(std::io::Error::other(
                        "Duplicate or overlapping queue records",
                    ));
                }
            }
            Ok(record.entries)
        })();
        match read {
            Ok(entries) => q.entries = entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                q.blocked = true;
                q.warning = Some(format!(
                    "Could not restore cleanup queue. Clearing the queue keeps the unreadable record as {}, replacing any earlier one. {e}",
                    directory.join("queue.quarantine").display()
                ));
            }
        }
        q
    }
    /// Whether entries can be matched at all: not when empty or unreadable.
    pub fn matchable(&self) -> bool {
        !self.entries.is_empty() && !self.blocked
    }
    pub fn require_mutable(&self) -> Result<(), String> {
        if self.blocked {
            return Err("Clear the unreadable cleanup queue before changing it.".into());
        }
        Ok(())
    }
    fn publish(&mut self, entries: Vec<Candidate>, recovery: bool) -> Result<(), String> {
        let directory = self
            .directory
            .as_ref()
            .ok_or("Cleanup queue storage is unavailable.")?;
        if entries.len() > MAX_ENTRIES || entries.iter().any(|c| !c.valid()) {
            return Err("Cleanup queue supports at most 1,000 valid entries.".into());
        }
        let bytes = serde_json::to_vec(&Record {
            version: 1,
            entries: entries.clone(),
        })
        .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("The cleanup queue is full: its paths are too long to store. Remove some items first.".into());
        }
        fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let metadata = fs::symlink_metadata(directory).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Queue directory is not private.".into());
        }
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(directory)
            .map_err(|e| e.to_string())?;
        dir.set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let temp =
            CString::new(format!("{}.tmp", uuid::Uuid::new_v4())).map_err(|e| e.to_string())?;
        let mut quarantine = None;
        let result = (|| -> Result<Option<String>, std::io::Error> {
            use std::io::Write;
            let mut file = open_at(
                &dir,
                &temp,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            dir.sync_all()?;
            if recovery && record_needs_quarantine(&dir, self.blocked)? {
                // A folder already quarantined there cannot be replaced: keep it and use a unique name instead.
                let name = match rename_at(&dir, c"queue.json", QUARANTINE) {
                    Ok(()) => QUARANTINE.to_owned(),
                    Err(_) => {
                        let name =
                            CString::new(format!("queue.quarantine-{}", uuid::Uuid::new_v4()))
                                .map_err(std::io::Error::other)?;
                        rename_at(&dir, c"queue.json", &name)?;
                        name
                    }
                };
                quarantine = Some(name);
                dir.sync_all()?;
            }
            #[cfg(test)]
            if self.fail_publication {
                return Err(std::io::Error::other("Injected publication failure"));
            }
            rename_at(&dir, &temp, c"queue.json")?;
            Ok(dir
                .sync_all()
                .err()
                .map(|e| format!("Queue saved, but durability could not be confirmed: {e}")))
        })();
        let warning = match result {
            Ok(w) => w,
            Err(e) => {
                unsafe {
                    libc::unlinkat(dir.as_raw_fd(), temp.as_ptr(), 0);
                }
                let rollback = quarantine
                    .as_ref()
                    .and_then(|name| rename_at(&dir, name, c"queue.json").err());
                let _ = dir.sync_all();
                return Err(match rollback {
                    Some(error) => format!("Could not save cleanup queue: {e}. Original record remains quarantined; restoring its name failed: {error}"),
                    None => format!("Could not save cleanup queue: {e}"),
                });
            }
        };
        self.entries = entries;
        self.revision += 1;
        self.warning = warning;
        self.blocked = false;
        Ok(())
    }
    fn queued_keys(&self) -> Keys<'_> {
        self.entries
            .iter()
            .map(|c| (&c.volume, c.item.path.as_path()))
            .collect()
    }
    /// Drops the items that are, or lie under, a queued entry: adding them changes nothing, so they need no checks.
    pub fn retain_uncovered(&self, items: &mut Vec<CheckedItem>) {
        let queued = self.queued_keys();
        items.retain(|i| !covered(&queued, i));
    }
    /// Fails as `add` would when `items` (outermost only) cannot be merged in under `MAX_ENTRIES`, from paths alone
    /// so it can run before any disk check: an item under a queued entry adds nothing, one above queued entries
    /// replaces them. Only paths on the same volume overlap: a card ejected and another mounted at the same path
    /// are different places.
    pub fn check_room(&self, items: &[CheckedItem]) -> Result<(), String> {
        self.require_mutable()?;
        let queued = self.queued_keys();
        let new: std::collections::HashSet<(&Option<Volume>, &Path)> = items
            .iter()
            .map(|i| (&i.volume, i.path.as_path()))
            .collect();
        let added = items.iter().filter(|i| !covered(&queued, i)).count();
        let replaced = self
            .entries
            .iter()
            .filter(|c| {
                c.item
                    .path
                    .ancestors()
                    .skip(1)
                    .any(|a| new.contains(&(&c.volume, a)))
            })
            .count();
        if self.entries.len() - replaced + added > MAX_ENTRIES {
            return Err(FULL.into());
        }
        Ok(())
    }
    pub fn add(&mut self, candidates: Vec<Candidate>) -> Result<(), String> {
        self.require_mutable()?;
        let mut candidates: Vec<_> = self
            .entries
            .iter()
            .cloned()
            .chain(candidates)
            .enumerate()
            .collect();
        candidates
            .sort_by(|a, b| (&a.1.volume, &a.1.item.path).cmp(&(&b.1.volume, &b.1.item.path)));
        let mut entries: Vec<(usize, Candidate)> = Vec::new();
        for c in candidates {
            // Volume then native path order groups descendants; stable sorting keeps existing duplicates.
            if entries.last().is_some_and(|p| {
                p.1.volume == c.1.volume && c.1.item.path.starts_with(&p.1.item.path)
            }) {
                continue;
            }
            entries.push(c);
        }
        if entries.len() > MAX_ENTRIES {
            return Err(FULL.into());
        }
        entries.sort_by_key(|(index, _)| *index);
        let entries: Vec<_> = entries.into_iter().map(|(_, c)| c).collect();
        if entries
            .iter()
            .map(|c| &c.id)
            .eq(self.entries.iter().map(|c| &c.id))
        {
            return Ok(());
        }
        self.publish(entries, false)
    }
    pub fn remove(&mut self, ids: &[String]) -> Result<(), String> {
        self.require_mutable()?;
        if ids.len() > MAX_ENTRIES {
            return Err("Remove at most 1,000 cleanup queue entries at a time.".into());
        }
        let ids: std::collections::HashSet<_> = ids.iter().collect();
        if ids.is_empty() {
            return Ok(());
        }
        if ids
            .iter()
            .any(|id| !self.entries.iter().any(|c| &c.id == *id))
        {
            return Err("Unknown cleanup queue entry.".into());
        }
        self.publish(
            self.entries
                .iter()
                .filter(|c| !ids.contains(&c.id))
                .cloned()
                .collect(),
            false,
        )
    }
    pub fn clear(&mut self) -> Result<(), String> {
        self.publish(vec![], true)
    }
    /// Entries under a trashed item are gone, and a failed item's own entry records the error. Both only match an
    /// entry on the item's volume.
    pub fn outcomes(
        &mut self,
        trashed: &[CheckedItem],
        failures: &[(CheckedItem, String)],
    ) -> Result<(), String> {
        let mut changed = false;
        let entries: Vec<_> = self
            .entries
            .iter()
            .filter_map(|c| {
                if trashed
                    .iter()
                    .any(|t| t.volume == c.volume && c.item.path.starts_with(&t.path))
                {
                    changed = true;
                    return None;
                }
                let mut c = c.clone();
                // Exact path: a failed folder never tried the queued entries under it.
                if let Some((_, error)) = failures
                    .iter()
                    .find(|(f, _)| f.volume == c.volume && f.path == c.item.path)
                {
                    let error: String = error.chars().take(4096).collect();
                    if c.last_error.as_ref() != Some(&error) {
                        changed = true;
                        c.last_error = Some(error);
                    }
                }
                Some(c)
            })
            .collect();
        if !changed {
            return Ok(());
        }
        self.require_mutable()?;
        self.publish(entries, false)
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EntryView {
    pub id: String,
    pub name: String,
    pub path: String,
    pub path_key: String,
    pub root_path: String,
    pub allocated_bytes: u64,
    pub status: String,
    pub reason: Option<String>,
    /// The live scan this entry is checked against, when it has one. Review is addressed to that scan.
    pub matched_scan_id: Option<String>,
    /// The matched scan holds this entry's item on the entry's own volume (not a same-path item on another disk).
    pub on_scan_volume: bool,
    pub last_error: Option<String>,
}
#[derive(Serialize)]
pub(crate) struct View {
    pub entries: Vec<EntryView>,
    pub revision: u64,
    pub warning: Option<String>,
    pub blocked: bool,
}

type Keys<'a> = std::collections::HashSet<(&'a Option<Volume>, &'a Path)>;
/// Whether `item` is, or lies under, an entry of `queued` on its own volume.
fn covered(queued: &Keys, item: &CheckedItem) -> bool {
    item.path
        .ancestors()
        .any(|a| queued.contains(&(&item.volume, a)))
}

/// The queue entries for `items`, the checked items of `node_ids` in `scan`.
pub(crate) fn candidates(
    scan: &Scan,
    items: Vec<CheckedItem>,
    node_ids: &[u32],
) -> Result<Vec<Candidate>, String> {
    let snapshot = scan.snapshot;
    let root = snapshot.node(0).ok_or("Missing scan root")?;
    let root = Guard {
        path: snapshot.root.clone(),
        identity: Identity::of(&root),
    };
    let guards = |lineage: &[(PathBuf, Identity)]| {
        lineage
            .iter()
            .map(|(path, identity)| Guard {
                path: path.clone(),
                identity: identity.clone(),
            })
            .collect::<Vec<_>>()
    };
    let ids: HashMap<_, _> = node_ids
        .iter()
        .map(|&id| Ok((snapshot.path(id)?, id)))
        .collect::<Result<_, String>>()?;
    items
        .into_iter()
        .map(|item| {
            let node = snapshot
                .node(ids[&item.path])
                .ok_or("Missing selected node")?;
            Ok(Candidate {
                id: uuid::Uuid::new_v4().to_string(),
                ancestors: guards(&item.ancestors),
                item: Guard {
                    path: item.path,
                    identity: item.identity,
                },
                root: root.clone(),
                root_ancestors: guards(scan.root_ancestors),
                allocated_bytes: node.allocated_size,
                last_error: None,
                volume: item.volume,
            })
        })
        .collect()
}

/// How one queue entry stands against the scan it is matched to.
pub(crate) struct EntryStatus {
    pub status: &'static str,
    pub reason: Option<String>,
    /// The item's node in that scan, set only when the entry is ready.
    pub node_id: Option<u32>,
    pub scan_id: String,
    /// The scan's node at the item's path is on the entry's recorded volume.
    pub on_scan_volume: bool,
}

/// Matches every queue entry to one of `scans` (all usable), then gives it a status from that scan. Entries are
/// keyed by root path and item path, with no scan id, so the match is made here every time:
/// 1. The scan whose root is the entry's own root. A disk scan wins over a folder scan of the same root, whatever
///    the order of `scans`.
/// 2. Otherwise the disk scan with the longest root that contains the item's path (never a folder scan), so an
///    item queued from a folder still counts once its whole disk has been scanned. The item must then be found
///    in that scan (and not be an excluded mount); if it is not, the entry has no status ("needs scan").
///
/// An entry with no scan has no status, which `view` shows as "needs scan". The item is verified through the
/// matched scan itself in both cases: same identity and ancestor chain as recorded, then `safety::verify_paths`
/// against that scan's root. `home`, `data` (the app's data folder) and `mounts` feed its protection rules.
pub(crate) fn statuses(
    queue: &Queue,
    scans: &[Scan],
    home: Option<&Path>,
    data: Option<&Path>,
    mounts: &[PathBuf],
) -> HashMap<String, EntryStatus> {
    let mut statuses = HashMap::new();
    if !queue.matchable() {
        return statuses;
    }
    // (entry, whether the scan's root is the entry's own root), per scan.
    let mut assigned: Vec<Vec<(&Candidate, bool)>> = vec![vec![]; scans.len()];
    for c in &queue.entries {
        let exact = scans
            .iter()
            .enumerate()
            .filter(|(_, u)| u.snapshot.root == c.root.path)
            .min_by_key(|(_, u)| !u.disk)
            .map(|(i, _)| i);
        let covering = || {
            scans
                .iter()
                .enumerate()
                .filter(|(_, u)| u.disk && c.item.path.starts_with(&u.snapshot.root))
                .max_by_key(|(_, u)| u.snapshot.root.components().count())
                .map(|(i, _)| i)
        };
        if let Some((i, exact)) = exact
            .map(|i| (i, true))
            .or_else(|| covering().map(|i| (i, false)))
        {
            assigned[i].push((c, exact));
        }
    }
    for (scan, entries) in scans.iter().zip(assigned) {
        if entries.is_empty() {
            continue;
        }
        let snapshot = scan.snapshot;
        // Native paths are matched by walking native names down from the scan root, never by building every
        // node's path. Siblings share one parent walk and one name map. Display strings are never keys.
        let walk = !snapshot.has_path_overrides();
        let matches: HashMap<&Path, u32> = if walk {
            let mut folders: HashMap<&Path, Option<HashMap<&OsStr, u32>>> = HashMap::new();
            entries
                .iter()
                .filter_map(|(c, _)| {
                    let path = c.item.path.as_path();
                    let found = match (path.parent(), path.file_name()) {
                        (Some(parent), Some(name)) if path != snapshot.root => {
                            let children = folders.entry(parent).or_insert_with(|| {
                                let mut map = HashMap::new();
                                for &child in
                                    snapshot.children(safety::node_at(snapshot, parent)?).ok()?
                                {
                                    if let Ok(name) = snapshot.native_name(child) {
                                        map.entry(name).or_insert(child);
                                    }
                                }
                                Some(map)
                            });
                            let id = *children.as_ref()?.get(name)?;
                            (snapshot.path(id).ok()? == path).then_some(id)
                        }
                        _ => safety::node_at(snapshot, path),
                    };
                    Some((path, found?))
                })
                .collect()
        } else {
            // Records with recorded path spellings are not reachable by names: match them in one traversal.
            let wanted: std::collections::HashSet<_> =
                entries.iter().map(|(c, _)| c.item.path.as_path()).collect();
            let mut matches = HashMap::new();
            for node in snapshot.iter_nodes() {
                if let Ok(path) = node.path() {
                    if let Some(&wanted) = wanted.get(path.as_path()) {
                        matches.insert(wanted, node.id);
                    }
                }
            }
            matches
        };
        let root_identity = snapshot.node(0).map(|n| Identity::of(&n));
        // The scan's root as the record has it, once a disk that was plugged in again is read under its new number.
        let root_changed = |c: &Candidate| {
            let Some(now) = &root_identity else {
                return true;
            };
            let saved = c
                .root_checked()
                .renumbered(safety::recorded_volume(scan, 0).as_ref(), now.device);
            saved.identity != *now || scan.root_ancestors != saved.ancestors
        };
        for (c, exact) in entries {
            let found = matches.get(c.item.path.as_path()).copied();
            let on_scan_volume = c.volume.is_some()
                && found.is_some_and(|id| safety::recorded_volume(scan, id) == c.volume);
            let (status, reason, node_id) = if !exact {
                // The queued root is not this scan's root, so its identity is not compared with it: it is
                // part of the item's recorded ancestor chain, which is compared below.
                // A disk scan that lacks the item, holds an excluded mount there, or holds a different
                // item or ancestor chain than the record is stale for this entry, not a verdict on it:
                // no match. A match that fails the on-disk check stays "blocked".
                match found.filter(|&id| snapshot.node(id).is_some_and(|n| !n.excluded)) {
                    None => continue,
                    Some(id) => match item_status(scan, c, id, home, data, mounts) {
                        ("changed", ..) => continue,
                        status => status,
                    },
                }
            } else if root_changed(c) {
                ("changed", Some("Original scan location or its ancestors changed. Remove this entry and deliberately add it again.".into()), None)
            } else if let Some(id) = found {
                item_status(scan, c, id, home, data, mounts)
            } else {
                (
                    "missing",
                    Some("Item is missing or was not included in this scan.".into()),
                    None,
                )
            };
            statuses.insert(
                c.id.clone(),
                EntryStatus {
                    status,
                    reason,
                    node_id,
                    scan_id: scan.id.to_string(),
                    on_scan_volume,
                },
            );
        }
    }
    statuses
}

/// Status of an entry whose item is node `id` of `scan`: the recorded volume, identity and ancestors must equal
/// what the scan holds, and the item must pass the path checks of review time. The volume is compared in memory
/// (polled by the UI: no volume read here); `safety::verify` reads it again before Trash.
fn item_status(
    scan: &Scan,
    c: &Candidate,
    id: u32,
    home: Option<&Path>,
    data: Option<&Path>,
    mounts: &[PathBuf],
) -> (&'static str, Option<String>, Option<u32>) {
    let current = match safety::checked_item(scan, id) {
        Ok(current) => current,
        Err(error) => return ("blocked", Some(error), None),
    };
    // An entry without a recorded volume (queued by an older version) fails closed here.
    if c.volume.is_none() || current.volume != c.volume {
        return ("changed", Some("This entry's volume was replaced, or the entry was queued by an older version. Remove it and deliberately add it again.".into()), None);
    }
    // The same volume plugged in again may have a new device number: the record is read under it, and the inode,
    // kind and birth time must still match. The path checks below use that record too.
    let saved = c
        .checked()
        .renumbered(current.volume.as_ref(), current.identity.device);
    if current.identity != saved.identity || current.ancestors != saved.ancestors {
        return (
            "changed",
            Some(
                "Item or its ancestors changed. Remove this entry and deliberately add it again."
                    .into(),
            ),
            None,
        );
    }
    match safety::verify_paths(&saved, &scan.snapshot.root, home, data, mounts) {
        Ok(()) => ("ready", None, Some(id)),
        Err(error) => ("blocked", Some(error), None),
    }
}

/// The nodes of scan `scan_id` that entries are ready for: exactly what a review of that scan holds.
pub(crate) fn ready_nodes(
    queue: &Queue,
    statuses: &HashMap<String, EntryStatus>,
    scan_id: &str,
) -> Vec<u32> {
    queue
        .entries
        .iter()
        .filter_map(|c| {
            statuses
                .get(&c.id)
                .filter(|m| m.scan_id == scan_id)?
                .node_id
        })
        .collect()
}

pub(crate) fn view(queue: &Queue, statuses: &HashMap<String, EntryStatus>) -> View {
    View {
        revision: queue.revision,
        warning: queue.warning.clone(),
        blocked: queue.blocked,
        entries: queue
            .entries
            .iter()
            .map(|c| {
                let matched = statuses.get(&c.id);
                EntryView {
                    id: c.id.clone(),
                    name: c
                        .item
                        .path
                        .file_name()
                        .unwrap_or(c.item.path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                    path: c.item.path.to_string_lossy().into_owned(),
                    path_key: crate::path_key(&c.item.path),
                    root_path: c.root.path.to_string_lossy().into_owned(),
                    allocated_bytes: c.allocated_bytes,
                    status: matched.map_or("needsScan", |m| m.status).into(),
                    reason: match matched {
                        Some(m) => m.reason.clone(),
                        None => Some(
                            "Complete a scan of this entry's location, or of the disk it is on, before cleanup."
                                .into(),
                        ),
                    },
                    matched_scan_id: matched.map(|m| m.scan_id.clone()),
                    on_scan_volume: matched.is_some_and(|m| m.on_scan_volume),
                    last_error: c.last_error.clone(),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(root: &Path, path: &Path) -> Candidate {
        let guard = |path: &Path| Guard {
            path: path.to_path_buf(),
            identity: Identity::read(path).unwrap(),
        };
        let ancestors = |path: &Path| {
            crate::safety::ancestors(path)
                .unwrap()
                .into_iter()
                .map(|(path, identity)| Guard { path, identity })
                .collect()
        };
        Candidate {
            id: uuid::Uuid::new_v4().to_string(),
            item: guard(path),
            ancestors: ancestors(path),
            root: guard(root),
            root_ancestors: ancestors(root),
            allocated_bytes: 4096,
            last_error: None,
            volume: Some(Volume::read(root).unwrap()),
        }
    }
    /// A queue loaded from a record that holds `entries` exactly as given: `publish` would refuse some of them.
    fn load_record(storage: &Path, entries: Vec<Candidate>) -> Queue {
        fs::create_dir_all(storage).unwrap();
        let record = Record {
            version: 1,
            entries,
        };
        fs::write(
            storage.join("queue.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        Queue::load(storage.to_path_buf())
    }
    /// The record must not load: the queue is blocked and says why, nothing may change it until it is cleared,
    /// and clearing leaves a queue that loads.
    fn assert_blocks(entries: Vec<Candidate>, why: &str) {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("queue");
        let mut q = load_record(&storage, entries);
        assert!(q.blocked, "{why}");
        assert!(q.entries.is_empty(), "{why}");
        assert!(
            q.warning
                .as_deref()
                .is_some_and(|w| w.contains("Could not restore")),
            "{why}"
        );
        assert!(q.add(vec![]).is_err(), "{why}");
        q.clear().unwrap();
        assert!(!Queue::load(storage).blocked, "{why}");
    }
    #[test]
    fn a_well_formed_record_that_breaks_a_rule_blocks_the_queue_until_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let elsewhere = fs::canonicalize(elsewhere.path()).unwrap();
        let folder = root.join("folder");
        fs::create_dir(&folder).unwrap();
        let (a, b, child) = (root.join("a"), root.join("b"), folder.join("child"));
        let outside = elsewhere.join("outside");
        for path in [&a, &b, &child, &outside] {
            fs::write(path, []).unwrap();
        }
        // The control: the same record without a fault loads, so each case below is blocked by its own fault.
        let ok = load_record(
            &dir.path().join("control"),
            vec![candidate(&root, &a), candidate(&root, &b)],
        );
        assert!(!ok.blocked, "{:?}", ok.warning);
        assert_eq!(ok.entries.len(), 2);

        // An entry whose item lies outside its recorded root.
        assert_blocks(vec![candidate(&root, &outside)], "item outside its root");
        // A volume that is no canonical UUID, in either spelling.
        for volume in [
            "not-a-uuid",
            &uuid::Uuid::new_v4().to_string().to_uppercase(),
        ] {
            let mut entry = candidate(&root, &a);
            entry.volume = Some(Volume::Uuid(volume.into()));
            assert_blocks(vec![entry], volume);
        }
        // A root that is no directory, and a root identity that is not the one its own ancestor chain holds.
        let mut entry = candidate(&root, &a);
        entry.root.identity.kind = 2;
        assert_blocks(vec![entry], "root that is not a directory");
        let mut entry = candidate(&root, &a);
        entry.root.identity.inode ^= 1;
        assert_blocks(vec![entry], "root identity unlike the recorded ancestor");
        // An item kind no node has.
        let mut entry = candidate(&root, &a);
        entry.item.identity.kind = 9;
        assert_blocks(vec![entry], "unknown item kind");
        // An ancestor chain that skips a folder.
        let mut entry = candidate(&root, &child);
        entry.ancestors.remove(0);
        assert_blocks(vec![entry], "broken ancestor chain");

        // Overlapping entries (a folder and what lies under it), either way round, and a repeated id.
        let (parent, under) = (candidate(&root, &folder), candidate(&root, &child));
        assert_blocks(vec![parent.clone(), under.clone()], "folder then child");
        assert_blocks(vec![under, parent], "child then folder");
        let mut twin = candidate(&root, &b);
        twin.id = candidate(&root, &a).id;
        let mut first = candidate(&root, &a);
        first.id = twin.id.clone();
        assert_blocks(vec![first, twin], "the same id twice");
    }
    #[test]
    fn a_removal_names_its_own_limit_one_past_it() {
        let mut q = Queue::default();
        let ids = |n: usize| vec!["missing".to_string(); n];
        // At the limit the ids are looked up (and none exists); one past it, the limit is what is refused.
        assert_eq!(
            q.remove(&ids(MAX_ENTRIES)),
            Err("Unknown cleanup queue entry.".into())
        );
        assert_eq!(
            q.remove(&ids(MAX_ENTRIES + 1)),
            Err("Remove at most 1,000 cleanup queue entries at a time.".into())
        );
    }
    #[test]
    fn a_queue_saved_before_birth_times_still_loads_and_matches() {
        fn strip_birth(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::Object(m) => {
                    m.remove("birth");
                    m.values_mut().for_each(strip_birth);
                }
                serde_json::Value::Array(a) => a.iter_mut().for_each(strip_birth),
                _ => {}
            }
        }
        // Under HOME: the system temp dir is a protected path.
        let dir = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let file = root.join("file");
        fs::write(&file, b"data").unwrap();
        let storage = root.join("queue");
        let entry = candidate(&root, &file);
        assert!(entry.item.identity.birth.is_some());
        Queue::load(storage.clone())
            .add(vec![entry.clone()])
            .unwrap();
        let record = storage.join("queue.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
        assert!(json.to_string().contains("birth"));
        strip_birth(&mut json);
        fs::write(&record, serde_json::to_vec(&json).unwrap()).unwrap();
        let queue = Queue::load(storage);
        assert!(!queue.blocked, "{:?}", queue.warning);
        assert_eq!(queue.entries.len(), 1);
        let old = &queue.entries[0];
        assert_eq!(old.item.identity.birth, None);
        assert_eq!(old.item.identity, entry.item.identity);
        assert!(safety::verify_paths(&old.checked(), &root, None, None, &[]).is_ok());
    }
    #[test]
    fn restart_preserves_native_bytes_private_modes_and_overlap_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let folder = root.join("folder");
        fs::create_dir(&folder).unwrap();
        let native = folder.join(OsString::from_vec(vec![b'f', 0xff]));
        // APFS rejects invalid UTF-8 names; exercise the lossless record codec
        // using a native path and identities captured from a disposable real item.
        let actual = folder.join("actual");
        fs::write(&actual, b"data").unwrap();
        let storage = root.join("queue");
        let mut queue = Queue::load(storage.clone());
        let mut child = candidate(&root, &actual);
        child.item.path = native.clone();
        queue.add(vec![child.clone()]).unwrap();
        queue.add(vec![child.clone()]).unwrap();
        assert_eq!(queue.entries.len(), 1);
        assert_eq!(queue.revision, 1);
        let restored = Queue::load(storage.clone());
        assert_eq!(restored.entries[0].id, child.id);
        assert_eq!(restored.entries[0].item.path, native);
        assert_eq!(
            fs::metadata(storage.join("queue.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&storage).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let parent = candidate(&root, &folder);
        queue.add(vec![parent.clone()]).unwrap();
        assert_eq!(queue.entries.len(), 1);
        assert_eq!(queue.entries[0].id, parent.id);
        queue.add(vec![child]).unwrap();
        assert_eq!(queue.entries[0].id, parent.id);
        queue.outcomes(&[parent.checked()], &[]).unwrap();
        assert!(Queue::load(storage).entries.is_empty());
    }
    #[test]
    fn records_without_a_volume_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let path = root.join("item");
        fs::write(&path, []).unwrap();
        let storage = root.join("queue");
        let mut entry = serde_json::to_value(candidate(&root, &path)).unwrap();
        entry.as_object_mut().unwrap().remove("volume");
        fs::create_dir(&storage).unwrap();
        fs::write(
            storage.join("queue.json"),
            serde_json::to_vec(&serde_json::json!({"version": 1, "entries": [entry]})).unwrap(),
        )
        .unwrap();
        let q = Queue::load(storage);
        assert!(!q.blocked);
        assert!(q.entries[0].volume.is_none());
        assert!(q.entries[0].checked().volume.is_none());
    }
    #[test]
    fn corrupt_unsupported_nonregular_and_symlink_records_require_explicit_clear() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("queue");
        fs::create_dir(&storage).unwrap();
        let path = storage.join("queue.json");
        for bytes in [
            b"bad".as_slice(),
            br#"{"version":2,"entries":[]}"#.as_slice(),
        ] {
            fs::write(&path, bytes).unwrap();
            let mut q = Queue::load(storage.clone());
            assert!(q.blocked && q.warning.is_some());
            assert!(q.add(vec![]).is_err());
            assert!(q.remove(&["missing".into()]).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            q.clear().unwrap();
            assert!(!Queue::load(storage.clone()).blocked);
        }
        fs::remove_file(&path).unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, b"untouched").unwrap();
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        let mut q = Queue::load(storage.clone());
        assert!(q.blocked);
        q.clear().unwrap();
        assert_eq!(fs::read(victim).unwrap(), b"untouched");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(Queue::load(storage).blocked);
    }
    #[test]
    fn failed_publication_keeps_previous_memory_and_record() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let path = root.join("item");
        fs::write(&path, []).unwrap();
        let storage = root.join("queue");
        let mut q = Queue::load(storage.clone());
        q.add(vec![candidate(&root, &path)]).unwrap();
        let revision = q.revision;
        let original = fs::read(storage.join("queue.json")).unwrap();
        let moved = root.join("original-storage");
        fs::rename(&storage, &moved).unwrap();
        fs::write(&storage, b"blocks directory creation").unwrap();
        assert!(q.clear().is_err());
        assert_eq!(q.entries.len(), 1);
        assert_eq!(q.revision, revision);
        assert_eq!(fs::read(moved.join("queue.json")).unwrap(), original);
    }
    #[test]
    fn batch_removal_validates_before_publishing_and_preserves_record_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let mut entries = vec![];
        for name in ["a", "b", "c"] {
            let path = root.join(name);
            fs::write(&path, []).unwrap();
            entries.push(candidate(&root, &path));
        }
        let ids: Vec<_> = entries.iter().map(|c| c.id.clone()).collect();
        let storage = root.join("queue");
        let mut q = Queue::load(storage.clone());
        q.add(entries).unwrap();
        let revision = q.revision;
        let original = fs::read(storage.join("queue.json")).unwrap();
        q.remove(&[]).unwrap();
        assert!(q.remove(&[ids[0].clone(), "missing".into()]).is_err());
        assert!(q.remove(&vec![ids[0].clone(); MAX_ENTRIES + 1]).is_err());
        q.fail_publication = true;
        assert!(q.remove(&ids[..2]).is_err());
        assert_eq!(q.entries.len(), 3);
        assert_eq!(q.revision, revision);
        assert_eq!(fs::read(storage.join("queue.json")).unwrap(), original);
        q.fail_publication = false;
        q.remove(&[ids[0].clone(), ids[1].clone(), ids[0].clone()])
            .unwrap();
        assert_eq!(q.revision, revision + 1);
        assert_eq!(q.entries.len(), 1);
        assert_eq!(q.entries[0].id, ids[2]);
        let restored = Queue::load(storage);
        assert_eq!(restored.entries.len(), 1);
        assert_eq!(restored.entries[0].id, ids[2]);
    }
    #[test]
    fn capacity_checked_after_collapsing_overlaps() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let parent = root.join("parent");
        fs::create_dir(&parent).unwrap();
        let mut entries = vec![];
        for i in 0..=MAX_ENTRIES {
            let path = parent.join(i.to_string());
            fs::write(&path, []).unwrap();
            entries.push(candidate(&root, &path));
        }
        let mut q = Queue::load(root.join("queue"));
        assert!(q.add(entries.clone()).is_err());
        assert!(q.entries.is_empty());
        entries.push(candidate(&root, &parent));
        q.add(entries).unwrap();
        assert_eq!(q.entries.len(), 1);
    }
    #[test]
    fn large_sibling_addition_is_atomic_and_parent_replacement_preserves_queue_order() {
        let dir = tempfile::tempdir().unwrap();
        let guard = |path: &str| Guard {
            path: path.into(),
            identity: Identity {
                device: 1,
                inode: 1,
                kind: 1,
                birth: None,
            },
        };
        let template = Candidate {
            id: uuid::Uuid::new_v4().to_string(),
            item: guard("/synthetic/parent"),
            ancestors: vec![guard("/synthetic"), guard("/")],
            root: guard("/synthetic"),
            root_ancestors: vec![guard("/")],
            allocated_bytes: 0,
            last_error: None,
            volume: None,
        };
        let mut existing = template.clone();
        existing.id = uuid::Uuid::new_v4().to_string();
        existing.item.path = "/synthetic/other".into();
        existing.last_error = Some("Keep this failure".into());
        let mut q = Queue::load(dir.path().join("queue"));
        q.add(vec![existing.clone()]).unwrap();
        let original = fs::read(dir.path().join("queue/queue.json")).unwrap();
        let siblings = || {
            (0..100_000)
                .map(|i| {
                    let mut child = template.clone();
                    child.id = format!("00000000-0000-4000-8000-{i:012x}");
                    child.item.path = template.item.path.join(format!("file-{i}"));
                    child.ancestors.insert(0, template.item.clone());
                    child
                })
                .collect::<Vec<_>>()
        };
        assert!(q.add(siblings()).is_err());
        assert_eq!(q.revision, 1);
        assert_eq!(q.entries[0].last_error, existing.last_error);
        assert_eq!(
            fs::read(dir.path().join("queue/queue.json")).unwrap(),
            original
        );
        let mut entries = siblings();
        let mut duplicate = existing.clone();
        duplicate.id = uuid::Uuid::new_v4().to_string();
        duplicate.last_error = None;
        entries.push(duplicate);
        entries.push(template.clone());
        q.add(entries).unwrap();
        assert_eq!(q.revision, 2);
        assert_eq!(q.entries.len(), 2);
        assert_eq!(q.entries[0].id, existing.id);
        assert_eq!(q.entries[0].last_error, existing.last_error);
        assert_eq!(q.entries[1].id, template.id);
        assert_eq!(Queue::load(dir.path().join("queue")).entries.len(), 2);
    }
    #[test]
    fn explicit_clear_quarantines_directory_record_and_preserves_contents() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("queue");
        let record = storage.join("queue.json");
        fs::create_dir_all(&record).unwrap();
        fs::write(record.join("keep"), b"preserved").unwrap();
        let mut q = Queue::load(storage.clone());
        assert!(q.blocked);
        q.clear().unwrap();
        assert!(!q.blocked);
        let restored = Queue::load(storage.clone());
        assert!(!restored.blocked && restored.entries.is_empty());
        let preserved = fs::read_dir(&storage)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("queue.quarantine")
            })
            .unwrap();
        assert_eq!(fs::read(preserved.join("keep")).unwrap(), b"preserved");
        // A later unreadable record cannot replace that folder: it is kept beside it, and Clear still works.
        fs::write(storage.join("queue.json"), br#"{"version":2,"entries":[]}"#).unwrap();
        let mut q = Queue::load(storage.clone());
        assert!(q.blocked);
        q.clear().unwrap();
        assert!(!q.blocked);
        assert_eq!(fs::read(preserved.join("keep")).unwrap(), b"preserved");
    }
    #[test]
    fn clearing_a_blocked_queue_keeps_an_unreadable_regular_record_in_quarantine() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("queue");
        fs::create_dir_all(&storage).unwrap();
        // A newer version's record: a regular file this version cannot read.
        let newer = br#"{"version":2,"entries":[]}"#;
        fs::write(storage.join("queue.json"), newer).unwrap();
        let mut q = Queue::load(storage.clone());
        assert!(q.blocked);
        q.clear().unwrap();
        assert!(!q.blocked);
        let restored = Queue::load(storage.clone());
        assert!(!restored.blocked && restored.entries.is_empty());
        let quarantined = || -> Vec<PathBuf> {
            fs::read_dir(&storage)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("queue.quarantine")
                })
                .collect()
        };
        let kept = quarantined();
        assert_eq!(kept.len(), 1);
        assert_eq!(fs::read(&kept[0]).unwrap(), newer);
        // Clearing a queue that loaded fine replaces its own record: nothing more to keep.
        let mut healthy = restored;
        healthy.clear().unwrap();
        assert_eq!(quarantined(), kept);
    }
    #[test]
    fn a_later_clear_replaces_the_quarantined_record_and_the_warning_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("queue");
        fs::create_dir_all(&storage).unwrap();
        let quarantine = storage.join("queue.quarantine");
        for newer in [
            &br#"{"version":2,"entries":[]}"#[..],
            &br#"{"version":3}"#[..],
        ] {
            fs::write(storage.join("queue.json"), newer).unwrap();
            let mut q = Queue::load(storage.clone());
            assert!(q
                .warning
                .as_deref()
                .unwrap()
                .contains(quarantine.to_str().unwrap()));
            q.clear().unwrap();
            assert_eq!(fs::read(&quarantine).unwrap(), newer);
        }
        let files = fs::read_dir(&storage).unwrap().count();
        assert_eq!(files, 2, "queue.json and one quarantine file");
    }
    #[test]
    fn failed_clear_restores_quarantined_record_before_reporting_failure() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("queue");
        let record = storage.join("queue.json");
        fs::create_dir_all(&record).unwrap();
        fs::write(record.join("keep"), b"preserved").unwrap();
        let mut q = Queue::load(storage.clone());
        q.fail_publication = true;
        assert!(q.clear().is_err());
        assert!(q.blocked && q.revision == 0);
        assert!(record.is_dir());
        assert_eq!(fs::read(record.join("keep")).unwrap(), b"preserved");
        assert_eq!(fs::read_dir(storage).unwrap().count(), 1);
    }
    #[test]
    fn a_failed_folder_does_not_mark_queued_entries_under_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let folder = root.join("folder");
        fs::create_dir(&folder).unwrap();
        let child = folder.join("child");
        fs::write(&child, []).unwrap();
        let mut q = Queue::load(root.join("queue"));
        q.add(vec![candidate(&root, &child)]).unwrap();
        let revision = q.revision;
        // The folder failed to trash; the entry under it was never tried.
        let item = |path: &Path| candidate(&root, path).checked();
        q.outcomes(&[], &[(item(&folder), "folder failed".into())])
            .unwrap();
        assert_eq!((q.revision, &q.entries[0].last_error), (revision, &None));
        q.outcomes(&[], &[(item(&child), "child failed".into())])
            .unwrap();
        assert_eq!(q.entries[0].last_error.as_deref(), Some("child failed"));
        // A trashed folder does take its descendants with it.
        q.outcomes(&[item(&folder)], &[]).unwrap();
        assert!(q.entries.is_empty());
    }
    #[test]
    fn load_removes_stale_temp_records_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let storage = dir.path().join("queue");
        fs::create_dir(&storage).unwrap();
        let stale = storage.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&stale, b"half written").unwrap();
        let quarantine = storage.join("queue.quarantine");
        fs::write(&quarantine, b"kept").unwrap();
        let victim = dir.path().join("victim.tmp");
        fs::write(&victim, b"untouched").unwrap();
        let link = storage.join("link.tmp");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        let folder = storage.join("folder.tmp");
        fs::create_dir(&folder).unwrap();
        assert!(!Queue::load(storage).blocked);
        assert!(!stale.exists());
        assert!(quarantine.exists() && folder.is_dir());
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read(victim).unwrap(), b"untouched");
    }
}
