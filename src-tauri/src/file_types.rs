//! Metadata-only extension summaries and bounded visible-node highlighting.
use disk_scan::{NodeKind, NodeRef, Snapshot};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsStr,
    path::Path,
};

/// Last suffix, including native hidden-file semantics; no leading dot.
pub(crate) fn extension(name: &OsStr) -> String {
    Path::new(name)
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}
pub(crate) fn matches(node: &NodeRef<'_>, wanted: &str) -> bool {
    !node.excluded
        && node.identity.kind == NodeKind::File
        && extension(node.native_name()) == wanted
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    extension: String,
    files: u64,
    allocated_bytes: u64,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Summary {
    items: Vec<Item>,
    files: u64,
    allocated_bytes: u64,
    pub(crate) incomplete: bool,
}
/// Walk only the chosen subtree. Excluded directories do not confer traversal authority.
pub(crate) fn visit(
    snapshot: &Snapshot,
    scope: u32,
    mut visitor: impl FnMut(NodeRef<'_>),
) -> Result<(), String> {
    if snapshot.node(scope).is_none() {
        return Err("Unknown scope node".into());
    }
    let mut pending = vec![scope];
    while let Some(id) = pending.pop() {
        let node = snapshot.node(id).ok_or("Unknown subtree node")?;
        if node.excluded {
            continue;
        }
        if node.identity.kind == NodeKind::Directory {
            pending.extend(node.children().iter().copied());
        }
        visitor(node);
    }
    Ok(())
}
pub(crate) fn summarize(snapshot: &Snapshot, scope: u32) -> Result<Summary, String> {
    let mut items = BTreeMap::<String, Item>::new();
    let mut result = Summary::default();
    visit(snapshot, scope, |node| {
        if node.identity.kind != NodeKind::File {
            return;
        }
        let key = extension(node.native_name());
        let item = items.entry(key.clone()).or_insert_with(|| Item {
            extension: key,
            ..Item::default()
        });
        item.files += 1;
        item.allocated_bytes = item.allocated_bytes.saturating_add(node.allocated_size);
        result.files += 1;
        result.allocated_bytes = result.allocated_bytes.saturating_add(node.allocated_size);
    })?;
    result.items = items.into_values().collect();
    Ok(result)
}
/// Postorder traversal propagates one match bit per active directory, never all file IDs.
pub(crate) fn highlights(
    snapshot: &Snapshot,
    scope: u32,
    wanted: &str,
    ids: &[u32],
) -> Result<Vec<u32>, String> {
    if ids.len() > 5000 {
        return Err("At most 5000 visible nodes may be highlighted".into());
    }
    if snapshot.node(scope).is_none() {
        return Err("Unknown scope node".into());
    }
    let requested: HashSet<_> = ids.iter().copied().collect();
    let wanted = wanted.to_lowercase();
    let mut found = HashSet::new();
    // (node, next child index, subtree has a match)
    let mut stack = vec![(scope, 0usize, false)];
    while let Some((id, next_child, matched)) = stack.last_mut() {
        let node = snapshot.node(*id).ok_or("Unknown subtree node")?;
        if !node.excluded
            && node.identity.kind == NodeKind::Directory
            && *next_child < node.children().len()
        {
            let child = node.children()[*next_child];
            *next_child += 1;
            stack.push((child, 0, false));
            continue;
        }
        let has_match = !node.excluded && (*matched || matches(&node, &wanted));
        if has_match && requested.contains(id) {
            found.insert(*id);
        }
        stack.pop();
        if has_match {
            if let Some(parent) = stack.last_mut() {
                parent.2 = true;
            }
        }
    }
    // Preserve request order and return actual IDs only, once.
    Ok(ids.iter().copied().filter(|id| found.remove(id)).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use disk_scan::{Identity, Node, ScanState};
    use std::path::PathBuf;

    pub(crate) fn fixture() -> Snapshot {
        let mut snapshot = Snapshot::empty(PathBuf::from("/fixture"));
        for (id, parent, name, kind, length, allocated, excluded) in [
            (0, None, "fixture", NodeKind::Directory, 999, 999, false),
            (1, Some(0), "nested", NodeKind::Directory, 777, 777, false),
            (2, Some(1), "deep", NodeKind::Directory, 777, 777, false),
            (3, Some(2), "PHOTO.JPG", NodeKind::File, 100, 32, false),
            (4, Some(1), "alias.jpg", NodeKind::File, 0, 0, false),
            (5, Some(0), "outside.jpg", NodeKind::File, 200, 64, false),
            (6, Some(1), "hidden.jpg", NodeKind::File, 900, 900, true),
            (7, Some(1), "link.jpg", NodeKind::Symlink, 800, 800, false),
            (8, Some(1), "socket.jpg", NodeKind::Other, 700, 700, false),
            (9, Some(1), ".env", NodeKind::File, 10, 0, false),
            (10, Some(1), "empty.", NodeKind::File, 0, 0, false),
            (11, Some(0), "archive.gz", NodeKind::File, 80, 16, false),
        ] {
            let path = match parent {
                None => PathBuf::from("/fixture"),
                Some(parent) => snapshot.path(parent).unwrap().join(name),
            };
            snapshot
                .append_legacy_node(Node {
                    id,
                    parent,
                    children: vec![],
                    name: name.into(),
                    path,
                    identity: Identity {
                        device: 1,
                        inode: u64::from(id),
                        kind,
                    },
                    allocated_size: allocated,
                    length,
                    own_allocated_size: allocated,
                    is_directory: kind == NodeKind::Directory,
                    hard_link: id == 4,
                    clone_metadata: None,
                    excluded,
                })
                .unwrap();
        }
        snapshot.set_children(0, &[1, 5, 11]).unwrap();
        snapshot.set_children(1, &[2, 4, 6, 7, 8, 9, 10]).unwrap();
        snapshot.set_children(2, &[3]).unwrap();
        snapshot.state = ScanState::Completed;
        snapshot
    }
    #[test]
    fn suffixes_are_exact_case_normalized_and_dotfile_aware() {
        for (name, expected) in [
            ("image.JpG", "jpg"),
            ("archive.tar.gz", "gz"),
            (".env", ""),
            (".config.JSON", "json"),
            ("trailing.", ""),
            ("no-extension", ""),
            ("..", ""),
            (".", ""),
        ] {
            assert_eq!(extension(OsStr::new(name)), expected, "{name}");
        }
    }
    #[test]
    fn accounting_counts_regular_files_and_attributed_zero_values_in_scope() {
        let snapshot = fixture();
        let summary = summarize(&snapshot, 1).unwrap();
        assert_eq!((summary.files, summary.allocated_bytes), (4, 32));
        let jpg = summary
            .items
            .iter()
            .find(|item| item.extension == "jpg")
            .unwrap();
        assert_eq!((jpg.files, jpg.allocated_bytes), (2, 32));
        let extensionless = &summary.items[0];
        assert_eq!(
            (
                extensionless.extension.as_str(),
                extensionless.files,
                extensionless.allocated_bytes
            ),
            ("", 2, 0)
        );
        let whole = summarize(&snapshot, 0).unwrap();
        assert_eq!((whole.files, whole.allocated_bytes), (6, 112));
        assert!(summarize(&snapshot, 100).is_err());
    }
    #[test]
    fn highlights_requested_ancestors_of_hidden_descendants_only_within_scope() {
        let snapshot = fixture();
        assert_eq!(
            highlights(&snapshot, 1, "JPG", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 999, 1]).unwrap(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            highlights(&snapshot, 1, "jpg", &[1, 2]).unwrap(),
            vec![1, 2]
        );
        assert_eq!(
            highlights(&snapshot, 1, "", &[1, 9, 10]).unwrap(),
            vec![1, 9, 10]
        );
        assert!(highlights(&snapshot, 1, "jpg", &vec![1; 5001]).is_err());
        assert!(highlights(&snapshot, 999, "jpg", &[]).is_err());
        assert!(highlights(&snapshot, 6, "jpg", &[6]).unwrap().is_empty());
    }
}
