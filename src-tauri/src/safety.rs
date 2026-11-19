//! Validation is repeated immediately before recycling. Native paths never come from an action request.
use disk_scan::{NodeKind, Snapshot};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    pub device: u64,
    pub inode: u64,
    pub kind: u8,
    /// Creation time (seconds, nanoseconds). (device, inode) alone cannot tell a file from the one that took its
    /// directory slot on FAT/exFAT. None when the platform has none, and in records written before it existed.
    #[serde(default)]
    pub birth: Option<(i64, i64)>,
}
/// Two identities are the same item when device, inode and kind match and, if both know a birth time, it matches
/// too. shortcut: a side without a birth time (a scan snapshot, a queue entry saved by an older version, a platform
/// without one) compares as before, so FAT inode reuse goes unseen for it; that makes this non-transitive, so it
/// is not `Eq`. Upgrade by recording the birth time in the scan and re-adding old queue entries.
impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        (self.device, self.inode, self.kind) == (other.device, other.inode, other.kind)
            && match (self.birth, other.birth) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}
#[cfg(target_os = "macos")]
fn birth(m: &std::fs::Metadata) -> Option<(i64, i64)> {
    use std::os::macos::fs::MetadataExt as _;
    Some((m.st_birthtime(), m.st_birthtime_nsec()))
}
#[cfg(not(target_os = "macos"))]
fn birth(_: &std::fs::Metadata) -> Option<(i64, i64)> {
    None
}
impl Identity {
    pub fn read(path: &Path) -> Result<Self, String> {
        let m = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let kind = if m.is_dir() {
            1
        } else if m.is_file() {
            2
        } else if m.file_type().is_symlink() {
            3
        } else {
            4
        };
        #[cfg(unix)]
        {
            Ok(Self {
                device: m.dev(),
                inode: m.ino(),
                kind,
                birth: birth(&m),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = kind;
            Err("Filesystem identity checks not implemented on this platform".into())
        }
    }
}

/// Which filesystem an item was on. (device, inode) alone is not enough: a removable volume reformatted and
/// mounted at the same path can reuse the device number, and FAT/exFAT file IDs come from on-disk layout.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Volume {
    Uuid(String),
    /// The filesystem reports no UUID. shortcut: only (device, inode) protects such volumes, as before.
    NoUuid,
}
impl Volume {
    pub fn read(path: &Path) -> Result<Self, String> {
        Ok(
            match crate::platform::volume_uuid(path).map_err(|e| e.to_string())? {
                Some(uuid) => Self::Uuid(uuid),
                None => Self::NoUuid,
            },
        )
    }
    pub fn valid(&self) -> bool {
        match self {
            Self::Uuid(uuid) => uuid::Uuid::parse_str(uuid).is_ok_and(|u| u.to_string() == *uuid),
            Self::NoUuid => true,
        }
    }
}

#[derive(Clone)]
pub struct CheckedItem {
    pub path: PathBuf,
    pub identity: Identity,
    pub ancestors: Vec<(PathBuf, Identity)>,
    /// The volume the identities were read on. None (unknown) never passes `verify`.
    pub volume: Option<Volume>,
}
impl CheckedItem {
    /// Records the item's birth time as the disk has it now, so `verify` refuses another file that reuses its
    /// device and inode. A scan snapshot holds none. Call it on a fresh item, before the `verify` that follows.
    pub fn pin_birth(&mut self) {
        self.identity.birth = Identity::read(&self.path).ok().and_then(|i| i.birth);
    }
}

/// One live scan as the cleanup rules see it: only what they read, with no session state. The caller decides which
/// scans may be offered (live, valid, completed, not running).
pub struct Scan<'a> {
    pub id: &'a str,
    /// A scan of a whole disk, as opposed to a folder scan.
    pub disk: bool,
    pub snapshot: &'a Snapshot,
    pub root_ancestors: &'a [(PathBuf, Identity)],
    /// The volume of each region the scan walked, keyed by the region's root node. None while not recorded.
    pub volumes: Option<&'a HashMap<u32, Volume>>,
}

/// An item's volume is its nearest region's: its own node or the closest ancestor that starts a region.
fn recorded_volume(scan: &Scan, id: u32) -> Option<Volume> {
    let volumes = scan.volumes?;
    std::iter::successors(Some(id), |&id| scan.snapshot.node(id)?.parent)
        .find_map(|id| volumes.get(&id))
        .cloned()
}

/// What the scan holds for node `id`, as the guard `verify` checks later. Excluded nodes and ancestors are refused.
pub fn checked_item(scan: &Scan, id: u32) -> Result<CheckedItem, String> {
    let snapshot = scan.snapshot;
    let n = snapshot.node(id).ok_or("Unknown node")?;
    if n.excluded {
        return Err("Excluded locations cannot be moved to Trash from this scan.".into());
    }
    let identity = Identity {
        device: n.identity.device,
        inode: n.identity.inode,
        kind: match n.identity.kind {
            NodeKind::Directory => 1,
            NodeKind::File => 2,
            NodeKind::Symlink => 3,
            NodeKind::Other => 4,
        },
        birth: None,
    };
    let mut lineage = vec![];
    let mut parent = n.parent;
    while let Some(id) = parent {
        if lineage.len() >= snapshot.node_count() {
            return Err("Invalid cleanup ancestry".into());
        }
        let p = snapshot.node(id).ok_or("Invalid cleanup ancestry")?;
        if !p.is_directory || p.excluded {
            return Err("Excluded or invalid cleanup ancestor".into());
        }
        lineage.push((
            p.path()?,
            Identity {
                device: p.identity.device,
                inode: p.identity.inode,
                kind: 1,
                birth: None,
            },
        ));
        parent = p.parent;
    }
    lineage.extend(scan.root_ancestors.iter().cloned());
    Ok(CheckedItem {
        path: n.path()?,
        identity,
        ancestors: lineage,
        volume: recorded_volume(scan, id),
    })
}

/// The node of `snapshot` at exactly `path`, found by walking child names down from the scan root.
pub fn node_at(snapshot: &Snapshot, path: &Path) -> Option<u32> {
    let mut id = 0;
    for component in path.strip_prefix(&snapshot.root).ok()?.components() {
        let std::path::Component::Normal(name) = component else {
            return None;
        };
        id = *snapshot
            .children(id)
            .ok()?
            .iter()
            .find(|&&child| snapshot.native_name(child).is_ok_and(|n| n == name))?;
    }
    (snapshot.path(id).ok()? == path).then_some(id)
}

pub fn ancestors(path: &Path) -> Result<Vec<(PathBuf, Identity)>, String> {
    path.ancestors()
        .skip(1)
        .map(|p| {
            let identity = Identity::read(p)?;
            if identity.kind != 1 {
                return Err("A path ancestor is not a directory.".into());
            }
            Ok((p.to_path_buf(), identity))
        })
        .collect()
}

pub fn protected(path: &Path, root: &Path, home: Option<&Path>, mounts: &[PathBuf]) -> bool {
    // A mount point, or a folder with a mount anywhere inside it: Trash would leave that volume mounted.
    if path == root
        || path == Path::new("/")
        || home.is_some_and(|h| path == h)
        || mounts.iter().any(|p| p.starts_with(path))
    {
        return true;
    }
    if [
        "/System", "/Library", "/bin", "/sbin", "/usr", "/etc", "/private", "/dev",
    ]
    .iter()
    .any(|p| path.starts_with(p))
    {
        return true;
    }
    if ["/Users", "/Volumes", "/Applications"]
        .iter()
        .any(|p| path == Path::new(p))
    {
        return true;
    }
    home.is_some_and(|h| path == h.join("Library"))
}

/// The check immediately before Trash: `verify_paths`, then the item's volume must still be the recorded one.
pub fn verify(
    item: &CheckedItem,
    root: &Path,
    home: Option<&Path>,
    mounts: &[PathBuf],
) -> Result<(), String> {
    verify_paths(item, root, home, mounts)?;
    match &item.volume {
        Some(volume) if Volume::read(&item.path)? == *volume => Ok(()),
        _ => Err("The volume at this location changed. Rescan before cleanup.".into()),
    }
}
/// Path, protection and identity checks without reading the volume UUID, for code polled by the UI. Its callers
/// compare the recorded volume in memory instead.
pub fn verify_paths(
    item: &CheckedItem,
    root: &Path,
    home: Option<&Path>,
    mounts: &[PathBuf],
) -> Result<(), String> {
    if !item.path.starts_with(root) || protected(&item.path, root, home, mounts) {
        return Err("This location is protected from cleanup.".into());
    }
    // Canonicalize the parent, not the item: recycling a symlink must recycle the link itself.
    let parent = item.path.parent().ok_or("Missing parent")?;
    if std::fs::canonicalize(parent).map_err(|e| e.to_string())? != parent {
        return Err("The path's ancestors now resolve elsewhere.".into());
    }
    for (path, identity) in &item.ancestors {
        if Identity::read(path)? != *identity {
            return Err("A path ancestor changed. Rescan before cleanup.".into());
        }
    }
    if Identity::read(&item.path)? != item.identity {
        return Err("The selected item changed. Rescan before cleanup.".into());
    }
    Ok(())
}

pub fn collapse(items: Vec<CheckedItem>) -> Vec<CheckedItem> {
    let mut items: Vec<_> = items.into_iter().enumerate().collect();
    items.sort_by(|a, b| a.1.path.cmp(&b.1.path));
    let mut result: Vec<(usize, CheckedItem)> = Vec::new();
    for item in items {
        // Native path order puts each parent immediately before its descendants.
        if result
            .last()
            .is_none_or(|p| !item.1.path.starts_with(&p.1.path))
        {
            result.push(item);
        }
    }
    result.sort_by_key(|(index, item)| (item.path.components().count(), *index));
    result.into_iter().map(|(_, item)| item).collect()
}

pub struct Ticket {
    pub token: String,
    pub created: Instant,
    pub items: Vec<CheckedItem>,
    pub queue_revision: Option<u64>,
}
impl Ticket {
    pub fn expired(&self) -> bool {
        self.created.elapsed() > Duration::from_secs(120)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protects_roots_and_system() {
        let home = Path::new("/Users/test");
        let root = Path::new("/Users/test/proj");
        // No mounts here: a mount below a path would protect it too and hide a deleted rule.
        let blocked = |path: &str| protected(Path::new(path), root, Some(home), &[]);
        // Each of these is refused by one rule of `protected` alone.
        assert!(blocked("/Users/test/proj"), "the scan root");
        assert!(blocked("/"), "/");
        assert!(blocked("/Users/test"), "the home folder");
        assert!(blocked("/Users/test/Library"), "the home Library");
        for system in [
            "/System", "/Library", "/bin", "/sbin", "/usr", "/etc", "/private", "/dev",
        ] {
            assert!(blocked(system), "{system}");
            assert!(blocked(&format!("{system}/thing")), "{system}/thing");
        }
        for folder in ["/Users", "/Volumes", "/Applications"] {
            assert!(blocked(folder), "{folder}");
        }
        // Ordinary paths pass: only those folders themselves are protected, not what is inside.
        for path in [
            "/Users/test/Downloads/x",
            "/Users/x/Downloads/file",
            "/Volumes/Ext/dir",
            "/Applications/Some.app",
            "/Users/test/Library/Caches",
        ] {
            assert!(!blocked(path), "{path}");
        }
        // Without a home folder, its two rules do not apply.
        assert!(!protected(Path::new("/Users/test"), root, None, &[]));
        assert!(!protected(
            Path::new("/Users/test/Library"),
            root,
            None,
            &[]
        ));
    }
    #[test]
    fn protects_mount_points_and_folders_holding_one() {
        let home = Path::new("/Users/test");
        let root = Path::new("/Users/test/proj");
        let mounts = [
            PathBuf::from("/Users/test/dev/vm/mnt"),
            PathBuf::from("/Volumes/Ext"),
        ];
        let blocked = |path: &str| protected(Path::new(path), root, Some(home), &mounts);
        assert!(blocked("/Users/test/dev/vm/mnt"), "a mount point");
        assert!(blocked("/Volumes/Ext"), "a volume's mount point");
        assert!(blocked("/Users/test/dev/vm"), "the parent of a mount point");
        assert!(
            blocked("/Users/test/dev"),
            "a folder holding a mount deeper in"
        );
        // Inside a mount, beside one, or a longer name: not protected (components, not text).
        for path in [
            "/Volumes/Ext/dir",
            "/Users/test/dev/vm/mnt/inner",
            "/Users/test/dev/vm/mn",
            "/Users/test/dev2",
        ] {
            assert!(!blocked(path), "{path}");
        }
        // The "/" mount is under no path but "/".
        let with_root_mount = [PathBuf::from("/")];
        assert!(!protected(
            Path::new("/Users/test/Downloads/x"),
            root,
            Some(home),
            &with_root_mount
        ));
    }
    #[test]
    fn collapses_overlapping_selection() {
        let d = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let p = d.path().join("a");
        std::fs::create_dir(&p).unwrap();
        let c = p.join("b");
        std::fs::write(&c, []).unwrap();
        let make = |path: PathBuf| CheckedItem {
            identity: Identity::read(&path).unwrap(),
            ancestors: ancestors(&path).unwrap(),
            path,
            volume: None,
        };
        assert_eq!(collapse(vec![make(c), make(p)]).len(), 1);
    }
    #[test]
    fn collapses_large_sibling_selection_without_losing_order_or_identity() {
        let make = |path: PathBuf, inode| CheckedItem {
            path,
            identity: Identity {
                device: 1,
                inode,
                kind: 2,
                birth: None,
            },
            ancestors: vec![],
            volume: None,
        };
        let items: Vec<_> = (0..100_000)
            .rev()
            .map(|i| make(PathBuf::from(format!("/root/file-{i}")), i))
            .collect();
        let result = collapse(items);
        assert_eq!(result.len(), 100_000);
        assert_eq!(result[0].identity.inode, 99_999);
        assert_eq!(result[99_999].identity.inode, 0);
        let result = collapse(vec![
            make("/root/a/child".into(), 1),
            make("/root/a-b".into(), 2),
            make("/root/a".into(), 3),
            make("/root/a".into(), 4),
        ]);
        assert_eq!(
            result.iter().map(|i| i.identity.inode).collect::<Vec<_>>(),
            [2, 3]
        );
    }
    #[test]
    fn rejects_replaced_file_and_ancestor() {
        let d = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let p = root.join("a");
        std::fs::write(&p, []).unwrap();
        let item = CheckedItem {
            path: p.clone(),
            identity: Identity::read(&p).unwrap(),
            ancestors: ancestors(&p).unwrap(),
            volume: Some(Volume::read(&root).unwrap()),
        };
        assert!(verify(&item, &root, None, &[]).is_ok());
        std::fs::rename(&p, root.join("old")).unwrap();
        std::fs::write(&p, []).unwrap();
        assert!(verify(&item, &root, None, &[]).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_parent_swap() {
        let d = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let p = root.join("folder");
        std::fs::create_dir(&p).unwrap();
        let f = p.join("file");
        std::fs::write(&f, []).unwrap();
        let item = CheckedItem {
            path: f.clone(),
            identity: Identity::read(&f).unwrap(),
            ancestors: ancestors(&f).unwrap(),
            volume: Some(Volume::read(&root).unwrap()),
        };
        std::fs::rename(&p, root.join("old")).unwrap();
        std::os::unix::fs::symlink(root.join("old"), &p).unwrap();
        assert!(verify(&item, &root, None, &[]).is_err());
    }
    #[test]
    fn another_birth_time_is_another_item_but_an_unknown_one_matches_as_before() {
        let id = |birth| Identity {
            device: 1,
            inode: 2,
            kind: 2,
            birth,
        };
        assert_eq!(id(Some((10, 5))), id(Some((10, 5))));
        assert_ne!(id(Some((10, 5))), id(Some((11, 5))));
        assert_ne!(id(Some((10, 5))), id(Some((10, 6))));
        assert_eq!(id(None), id(Some((10, 5))));
        assert_ne!(
            Identity {
                inode: 3,
                ..id(None)
            },
            id(None)
        );
    }
    #[test]
    fn a_record_without_a_birth_time_still_loads_and_matches() {
        let old: Identity = serde_json::from_str(r#"{"device":1,"inode":2,"kind":2}"#).unwrap();
        assert_eq!(old.birth, None);
        let new = Identity {
            birth: Some((10, 5)),
            ..old.clone()
        };
        assert_eq!(old, new);
        let saved: Identity = serde_json::from_str(&serde_json::to_string(&new).unwrap()).unwrap();
        assert_eq!(saved.birth, Some((10, 5)));
    }
    #[test]
    fn rejects_a_file_with_the_same_inode_but_another_birth_time() {
        let d = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let p = root.join("a");
        std::fs::write(&p, []).unwrap();
        let mut item = CheckedItem {
            path: p.clone(),
            identity: Identity::read(&p).unwrap(),
            ancestors: ancestors(&p).unwrap(),
            volume: None,
        };
        let (secs, nanos) = item.identity.birth.expect("macOS reports a birth time");
        assert!(verify_paths(&item, &root, None, &[]).is_ok());
        // What FAT inode reuse looks like: same device and inode, a file born at another time.
        item.identity.birth = Some((secs - 1, nanos));
        assert!(verify_paths(&item, &root, None, &[]).is_err());
        // An item with no recorded birth time is checked as before, until `pin_birth` records it.
        item.identity.birth = None;
        assert!(verify_paths(&item, &root, None, &[]).is_ok());
        item.pin_birth();
        assert_eq!(item.identity.birth, Some((secs, nanos)));
    }
    #[test]
    fn rejects_other_or_unknown_volume_before_trash() {
        let d = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let p = root.join("a");
        std::fs::write(&p, []).unwrap();
        let mut item = CheckedItem {
            path: p.clone(),
            identity: Identity::read(&p).unwrap(),
            ancestors: ancestors(&p).unwrap(),
            volume: Some(Volume::read(&root).unwrap()),
        };
        assert!(verify(&item, &root, None, &[]).is_ok());
        // Same path and identities, but recorded on another (reformatted) volume.
        item.volume = Some(Volume::Uuid(uuid::Uuid::new_v4().to_string()));
        assert!(verify_paths(&item, &root, None, &[]).is_ok());
        assert!(verify(&item, &root, None, &[]).is_err());
        item.volume = Some(Volume::NoUuid);
        assert!(verify(&item, &root, None, &[]).is_err());
        item.volume = None;
        assert!(verify(&item, &root, None, &[]).is_err());
    }
}
