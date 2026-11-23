//! Read-only identity and readability checks. This never applies Trash protection rules.
use crate::safety::{CheckedItem, Identity};
use disk_scan::{NodeKind, NodeRef, Snapshot};
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, PathBuf},
};
#[derive(Clone)]
pub(crate) struct Item {
    pub path: PathBuf,
    expected: CheckedItem,
    root: PathBuf,
    /// Shown for Reveal in Finder rather than previewed: same checks, wording that fits the action.
    reveal: bool,
}
/// The preview wording, or the reveal wording when `reveal`.
fn say(reveal: bool, preview: &str, shown: &str) -> String {
    if reveal { shown } else { preview }.into()
}
impl Item {
    pub fn from_snapshot(
        snapshot: &Snapshot,
        node_id: u32,
        root_ancestors: &[(PathBuf, Identity)],
    ) -> Result<Self, String> {
        let node = snapshot.node(node_id).ok_or("Unknown preview node")?;
        if node.identity.kind != NodeKind::File || node.parent.is_none() || node.excluded {
            return Err("Select one regular, non-excluded file to preview.".into());
        }
        Self::checked(snapshot, node_id, &node, root_ancestors, false)
    }
    /// The same ancestry and identity checks for showing a node in Finder: any kind, the scan root and an excluded
    /// node included, since nothing is opened or moved.
    pub fn for_reveal(
        snapshot: &Snapshot,
        node_id: u32,
        root_ancestors: &[(PathBuf, Identity)],
    ) -> Result<Self, String> {
        let node = snapshot.node(node_id).ok_or("Unknown node")?;
        Self::checked(snapshot, node_id, &node, root_ancestors, true)
    }
    fn checked(
        snapshot: &Snapshot,
        node_id: u32,
        node: &NodeRef,
        root_ancestors: &[(PathBuf, Identity)],
        reveal: bool,
    ) -> Result<Self, String> {
        let path = node.path()?;
        if !snapshot.root.is_absolute()
            || !path.starts_with(&snapshot.root)
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(say(
                reveal,
                "Preview path is outside the scan location.",
                "This item is outside the scan location.",
            ));
        }
        let mut ancestors =
            crate::safety::lineage(snapshot, node_id, if reveal { "reveal" } else { "preview" })?;
        ancestors.extend_from_slice(root_ancestors);
        // The recorded lineage must account for every lexical ancestor. This
        // rejects symlink substitutions without rejecting macOS firmlink paths.
        for path in path.ancestors().skip(1) {
            if !ancestors.iter().any(|(recorded, _)| recorded == path) {
                return Err(say(
                    reveal,
                    "Preview ancestry is not fully registered. Rescan this location.",
                    "This item's folders are not fully registered. Rescan to reveal it.",
                ));
            }
        }
        Ok(Self {
            path: path.clone(),
            root: snapshot.root.clone(),
            expected: CheckedItem {
                path: path.clone(),
                identity: Identity::of(node),
                ancestors,
                volume: None,
            },
            reveal,
        })
    }
    pub fn open_checked(&self) -> Result<File, String> {
        self.verify_paths()?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)
            .map_err(|e| format!("Cannot read the selected file: {e}"))?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file()
            || metadata.dev() != self.expected.identity.device
            || metadata.ino() != self.expected.identity.inode
        {
            return Err("The selected file changed. Rescan before previewing.".into());
        }
        self.verify_paths()?;
        Ok(file)
    }
    /// Whether the item and its ancestors on disk are still the ones the scan recorded.
    pub fn verify_paths(&self) -> Result<(), String> {
        if !self.path.starts_with(&self.root) {
            return Err(say(
                self.reveal,
                "Preview path is outside the scan location.",
                "This item is outside the scan location.",
            ));
        }
        if !crate::safety::ancestors_unchanged(&self.expected.ancestors)? {
            return Err(say(
                self.reveal,
                "A preview ancestor changed. Rescan this location.",
                "A folder above this item moved or changed since the scan. Rescan to reveal it.",
            ));
        }
        if Identity::read(&self.path)? != self.expected.identity {
            return Err(say(
                self.reveal,
                "The selected file changed. Rescan before previewing.",
                "This item moved or changed since the scan. Rescan to reveal it.",
            ));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{atomic::AtomicBool, Arc},
    };
    fn fixture() -> (tempfile::TempDir, Snapshot, Vec<(PathBuf, Identity)>) {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::write(root.join("space ' 😀\n.txt"), b"preview").unwrap();
        let snapshot = disk_scan::scan(
            root.clone(),
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        let ancestors = crate::safety::ancestors(&root).unwrap();
        (dir, snapshot, ancestors)
    }
    #[test]
    fn readable_native_path_and_missing_replaced_symlink_fifo_rejected() {
        let (_dir, snapshot, ancestors) = fixture();
        let item = Item::from_snapshot(&snapshot, 1, &ancestors).unwrap();
        assert!(item.path.to_string_lossy().contains("😀"));
        let held = item.open_checked().unwrap();
        let old = item.path.with_extension("held");
        fs::rename(&item.path, &old).unwrap();
        assert!(item.open_checked().is_err());
        fs::write(&item.path, b"replacement").unwrap();
        assert!(item.open_checked().is_err());
        fs::remove_file(&item.path).unwrap();
        std::os::unix::fs::symlink(&old, &item.path).unwrap();
        assert!(item.open_checked().is_err());
        fs::remove_file(&item.path).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(item.path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(item.open_checked().is_err());
        assert!(held.metadata().unwrap().is_file());
    }
    #[test]
    fn incomplete_results_allow_reads_and_ancestry_is_required() {
        let (_dir, mut snapshot, ancestors) = fixture();
        snapshot.stats.incomplete = true;
        snapshot.stats.errors = 1;
        assert!(Item::from_snapshot(&snapshot, 1, &ancestors)
            .unwrap()
            .open_checked()
            .is_ok());
        assert!(Item::from_snapshot(&snapshot, 1, &[]).is_err());
        assert!(Item::from_snapshot(&snapshot, 0, &ancestors).is_err());
        for kind in [NodeKind::Directory, NodeKind::Symlink, NodeKind::Other] {
            snapshot.update_node(1, |n| n.identity.kind = kind).unwrap();
            assert!(Item::from_snapshot(&snapshot, 1, &ancestors).is_err());
        }
        snapshot
            .update_node(1, |n| n.identity.kind = NodeKind::File)
            .unwrap();
        snapshot.update_node(1, |n| n.excluded = true).unwrap();
        assert!(Item::from_snapshot(&snapshot, 1, &ancestors).is_err());
        assert!(Item::from_snapshot(&snapshot, u32::MAX, &ancestors).is_err());
    }
    #[test]
    fn ancestor_replacement_and_permission_failure() {
        let (_dir, snapshot, ancestors) = fixture();
        let item = Item::from_snapshot(&snapshot, 1, &ancestors).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&item.path, fs::Permissions::from_mode(0o0)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert!(item.open_checked().is_err());
        }
        fs::set_permissions(&item.path, fs::Permissions::from_mode(0o600)).unwrap();
        let moved = snapshot.root.with_extension("moved");
        fs::rename(&snapshot.root, &moved).unwrap();
        fs::create_dir(&snapshot.root).unwrap();
        assert!(item.open_checked().is_err());
        fs::remove_dir(&snapshot.root).unwrap();
        fs::rename(&moved, &snapshot.root).unwrap();
    }
}
