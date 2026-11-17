//! A bounded composition map: real directory context plus exact file-type tiles.
//! The retained frontier is small; one metadata traversal assigns files to the
//! nearest retained ancestor, without a histogram for every scanned directory.
use crate::{file_types, size, view, MapData, NodeView};
use disk_scan::{NodeKind, Snapshot};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
};

const MAX_DIRECTORIES: usize = 128;
const MAX_DEPTH: usize = 4;
const MAX_TYPES: usize = 12;
const MAX_DIRECT_FILES: usize = 8;

#[derive(Clone, Copy, Default)]
struct Bytes {
    logical: u64,
    allocated: u64,
}
impl Bytes {
    fn node(node: &disk_scan::NodeRef<'_>) -> Self {
        Self {
            logical: node.logical_size,
            allocated: node.allocated_size,
        }
    }
    fn selected(self, allocated: bool) -> u64 {
        if allocated {
            self.allocated
        } else {
            self.logical
        }
    }
    fn add(&mut self, other: Self) {
        self.logical = self.logical.saturating_add(other.logical);
        self.allocated = self.allocated.saturating_add(other.allocated);
    }
    fn subtract(self, other: Self) -> Result<Self, String> {
        Ok(Self {
            logical: self
                .logical
                .checked_sub(other.logical)
                .ok_or("File-type map exceeds recorded logical size")?,
            allocated: self
                .allocated
                .checked_sub(other.allocated)
                .ok_or("File-type map exceeds recorded on-disk size")?,
        })
    }
    fn positive(self) -> bool {
        self.logical > 0 || self.allocated > 0
    }
}
#[derive(Default)]
struct Group {
    bytes: Bytes,
    files: u64,
}
struct Bucket {
    id: u32,
    groups: BTreeMap<String, Group>,
    // Only a fixed number of real direct-file candidates are retained.
    direct: BTreeSet<(u64, Reverse<u32>)>,
}
impl Bucket {
    fn new(id: u32) -> Self {
        Self {
            id,
            groups: BTreeMap::new(),
            direct: BTreeSet::new(),
        }
    }
}
fn keep_largest(set: &mut BTreeSet<(u64, Reverse<u32>)>, entry: (u64, Reverse<u32>), cap: usize) {
    if cap == 0 {
        return;
    }
    set.insert(entry);
    if set.len() > cap {
        set.pop_first();
    }
}
fn aggregate(
    id: u32,
    parent: &NodeView,
    name: String,
    bytes: Bytes,
    extension: Option<String>,
    files: Option<u64>,
) -> NodeView {
    NodeView {
        id,
        parent_id: Some(parent.id),
        name,
        path: String::new(),
        path_key: String::new(),
        kind: "aggregate".into(),
        logical_bytes: bytes.logical,
        allocated_bytes: bytes.allocated,
        raw_allocated_bytes: 0,
        verified_clone: false,
        clone_deduction_bytes: 0,
        clone_owner: None,
        children_count: 0,
        incomplete: parent.incomplete,
        excluded: false,
        file_type: extension,
        file_count: files,
    }
}

pub(crate) fn build(
    snapshot: &Snapshot,
    scope: u32,
    allocated: bool,
    limit: usize,
    issues: &[PathBuf],
) -> Result<MapData, String> {
    let budget = limit.clamp(2, 5000);
    let root = snapshot.node(scope).ok_or("Unknown node")?;
    if root.identity.kind != NodeKind::Directory {
        return Ok(MapData {
            nodes: vec![view(&root, issues)?],
            root_id: scope,
        });
    }
    // Reserve space for types and metadata at every retained folder before
    // spending the budget on hierarchy. Depth is relative to the chosen root.
    let directory_budget = (budget / (MAX_TYPES + MAX_DIRECT_FILES + 2)).clamp(1, MAX_DIRECTORIES);
    let mut retained = vec![(scope, 0usize)];
    let mut retained_index = HashMap::from([(scope, 0usize)]);
    let mut candidates = BTreeSet::new();
    let mut inspect = 0;
    while inspect < retained.len() {
        let (id, depth) = retained[inspect];
        let node = snapshot.node(id).ok_or("Unknown map folder")?;
        if !node.excluded && depth < MAX_DEPTH {
            for &child in node.children() {
                let child = snapshot.node(child).ok_or("Unknown map child")?;
                if child.identity.kind == NodeKind::Directory
                    && !child.excluded
                    && size(&child, allocated) > 0
                {
                    candidates.insert((size(&child, allocated), Reverse(child.id), depth + 1));
                    if candidates.len() > directory_budget {
                        candidates.pop_first();
                    }
                }
            }
        }
        inspect += 1;
        if retained.len() < directory_budget {
            if let Some((_, Reverse(id), depth)) = candidates.pop_last() {
                retained_index.insert(id, retained.len());
                retained.push((id, depth));
            }
        }
    }
    let slots = (budget - retained.len()) / retained.len();
    let direct_budget = (slots.saturating_sub(2) / 2).min(MAX_DIRECT_FILES);
    let mut buckets: Vec<_> = retained.iter().map(|&(id, _)| Bucket::new(id)).collect();
    // A child-index stack is bounded by depth, even for million-entry folders.
    let mut stack = vec![(scope, 0usize, 0usize)];
    while let Some((id, next_child, bucket)) = stack.last_mut() {
        let node = snapshot.node(*id).ok_or("Unknown subtree node")?;
        if node.excluded {
            stack.pop();
            continue;
        }
        if node.identity.kind == NodeKind::Directory && *next_child < node.children().len() {
            let child = node.children()[*next_child];
            *next_child += 1;
            let child_bucket = retained_index.get(&child).copied().unwrap_or(*bucket);
            stack.push((child, 0, child_bucket));
            continue;
        }
        if node.identity.kind == NodeKind::File {
            let bucket = &mut buckets[*bucket];
            let group = bucket
                .groups
                .entry(file_types::extension(node.native_name()))
                .or_default();
            group.bytes.add(Bytes::node(&node));
            group.files += 1;
            if node.parent == Some(bucket.id) && size(&node, allocated) > 0 {
                keep_largest(
                    &mut bucket.direct,
                    (size(&node, allocated), Reverse(node.id)),
                    direct_budget,
                );
            }
        }
        stack.pop();
    }
    let mut nodes = Vec::with_capacity(budget);
    let mut synthetic_id = 0x80000000u32;
    for mut bucket in buckets {
        let node = snapshot.node(bucket.id).ok_or("Unknown map folder")?;
        let parent = view(&node, issues)?;
        let mut accounted = Bytes::default();
        for &child in node.children() {
            if retained_index.contains_key(&child) {
                accounted.add(Bytes::node(
                    &snapshot.node(child).ok_or("Unknown map folder")?,
                ));
            }
        }
        let mut direct_views = Vec::new();
        for (_, Reverse(id)) in bucket.direct.iter().rev() {
            let file = snapshot.node(*id).ok_or("Unknown map file")?;
            let bytes = Bytes::node(&file);
            let group = bucket
                .groups
                .get_mut(&file_types::extension(file.native_name()))
                .ok_or("Missing map file type")?;
            group.bytes = group.bytes.subtract(bytes)?;
            group.files -= 1;
            accounted.add(bytes);
            direct_views.push(view(&file, issues)?);
        }
        let aggregate_slots = slots - direct_views.len();
        // Always reserve one honest neutral remainder; never label omitted types
        // or directory metadata as the largest extension at tiny budgets.
        let type_slots = aggregate_slots.saturating_sub(1).min(MAX_TYPES);
        let mut groups: Vec<_> = bucket
            .groups
            .into_iter()
            .filter(|(_, group)| group.files > 0)
            .collect();
        groups.sort_by(|(left_ext, left), (right_ext, right)| {
            right
                .bytes
                .selected(allocated)
                .cmp(&left.bytes.selected(allocated))
                .then_with(|| right.bytes.logical.cmp(&left.bytes.logical))
                .then_with(|| right.bytes.allocated.cmp(&left.bytes.allocated))
                .then_with(|| left_ext.cmp(right_ext))
        });
        nodes.push(parent.clone());
        nodes.extend(direct_views);
        let omitted_types = groups.len() > type_slots;
        for (extension, group) in groups.into_iter().take(type_slots) {
            accounted.add(group.bytes);
            let label = if extension.is_empty() {
                "No extension".into()
            } else {
                format!(".{extension}")
            };
            nodes.push(aggregate(
                synthetic_id,
                &parent,
                label,
                group.bytes,
                Some(extension),
                Some(group.files),
            ));
            synthetic_id = synthetic_id
                .checked_add(1)
                .ok_or("Map aggregate ID limit exceeded")?;
        }
        let residual = Bytes::node(&node).subtract(accounted)?;
        if residual.positive() {
            nodes.push(aggregate(
                synthetic_id,
                &parent,
                if omitted_types {
                    "Other file types & metadata".into()
                } else {
                    "Folder metadata & other items".into()
                },
                residual,
                None,
                None,
            ));
            synthetic_id = synthetic_id
                .checked_add(1)
                .ok_or("Map aggregate ID limit exceeded")?;
        }
    }
    debug_assert!(nodes.len() <= budget);
    Ok(MapData {
        nodes,
        root_id: scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use disk_scan::{Identity, Node, ScanState};
    use std::collections::HashSet;

    struct Fixture {
        nodes: Vec<Node>,
    }
    impl Fixture {
        fn new() -> Self {
            Self { nodes: Vec::new() }
        }
        fn add(
            &mut self,
            parent: Option<u32>,
            name: &str,
            kind: NodeKind,
            logical: u64,
            allocated: u64,
            excluded: bool,
        ) -> u32 {
            let id = self.nodes.len() as u32;
            let path = parent.map_or_else(
                || PathBuf::from("/fixture"),
                |id| self.nodes[id as usize].path.join(name),
            );
            self.nodes.push(Node {
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
                logical_size: logical,
                allocated_size: allocated,
                own_logical_size: logical,
                own_allocated_size: allocated,
                is_directory: kind == NodeKind::Directory,
                hard_link: false,
                clone_metadata: None,
                excluded,
            });
            if let Some(parent) = parent {
                self.nodes[parent as usize].children.push(id);
            }
            id
        }
        fn finish(mut self) -> Snapshot {
            for i in (1..self.nodes.len()).rev() {
                let parent = self.nodes[i].parent.unwrap() as usize;
                let logical = self.nodes[i].logical_size;
                let allocated = self.nodes[i].allocated_size;
                self.nodes[parent].logical_size += logical;
                self.nodes[parent].allocated_size += allocated;
            }
            let mut snapshot = Snapshot::empty(PathBuf::from("/fixture"));
            for node in self.nodes {
                snapshot.append_legacy_node(node).unwrap();
            }
            snapshot.state = ScanState::Completed;
            snapshot
        }
    }
    fn bytes(node: &NodeView) -> (u64, u64) {
        (node.logical_bytes, node.allocated_bytes)
    }
    fn assert_honest(map: &MapData, snapshot: &Snapshot, limit: usize) {
        assert!(map.nodes.len() <= limit.clamp(2, 5000));
        let ids: HashSet<_> = map.nodes.iter().map(|n| n.id).collect();
        assert_eq!(ids.len(), map.nodes.len());
        assert_eq!(map.root_id, map.nodes[0].id);
        for node in &map.nodes {
            if node.id != map.root_id {
                assert!(ids.contains(&node.parent_id.unwrap()));
            }
            if node.kind == "aggregate" {
                assert!(node.id >= 0x80000000);
                assert!(node.path.is_empty());
                assert_eq!(node.file_type.is_some(), node.file_count.is_some());
            } else {
                let actual = snapshot.node(node.id).unwrap();
                assert_eq!(bytes(node), (actual.logical_size, actual.allocated_size));
                assert!(node.file_type.is_none());
                assert!(node.file_count.is_none());
            }
            if node.kind == "directory" {
                let child_totals = map
                    .nodes
                    .iter()
                    .filter(|child| child.parent_id == Some(node.id))
                    .fold((0, 0), |(logical, allocated), child| {
                        (
                            logical + child.logical_bytes,
                            allocated + child.allocated_bytes,
                        )
                    });
                assert_eq!(child_totals, bytes(node), "folder {}", node.name);
            }
        }
    }
    fn mixed_fixture() -> Snapshot {
        let mut f = Fixture::new();
        let root = f.add(None, "fixture", NodeKind::Directory, 7, 16, false);
        let nested = f.add(Some(root), "nested", NodeKind::Directory, 3, 8, false);
        f.add(Some(nested), "photo.JPG", NodeKind::File, 1000, 512, false);
        f.add(Some(nested), "second.jpg", NodeKind::File, 900, 512, false);
        let zero_link = f.add(Some(nested), "hardlink.jpg", NodeKind::File, 0, 0, false);
        f.nodes[zero_link as usize].hard_link = true;
        f.add(Some(nested), "clone.jpg", NodeKind::File, 700, 0, false);
        f.add(Some(nested), ".env", NodeKind::File, 0, 0, false);
        f.add(Some(nested), "link.jpg", NodeKind::Symlink, 50, 64, false);
        f.add(Some(nested), "socket.jpg", NodeKind::Other, 20, 32, false);
        let excluded = f.add(Some(root), "excluded", NodeKind::Directory, 4, 8, true);
        f.add(
            Some(excluded),
            "hidden.jpg",
            NodeKind::File,
            100,
            128,
            false,
        );
        f.add(Some(root), "outside.MOV", NodeKind::File, 5000, 2048, false);
        f.finish()
    }
    #[test]
    fn both_metrics_and_small_limits_preserve_accounting_and_synthetic_authority() {
        let snapshot = mixed_fixture();
        for allocated in [false, true] {
            for limit in [0, 2, 3, 4, 10, 23, 47, 100, 5000, 10000] {
                let map = build(&snapshot, 0, allocated, limit, &[]).unwrap();
                assert_honest(&map, &snapshot, limit);
                assert_eq!(
                    serde_json::to_value(&map).unwrap(),
                    serde_json::to_value(build(&snapshot, 0, allocated, limit, &[]).unwrap())
                        .unwrap()
                );
                if limit <= 2 {
                    assert_eq!(map.nodes.len(), 2);
                    assert!(map.nodes[1].file_type.is_none());
                }
                let scope = build(&snapshot, 1, allocated, limit, &[]).unwrap();
                assert_honest(&scope, &snapshot, limit);
                assert!(!scope
                    .nodes
                    .iter()
                    .any(|n| n.file_type.as_deref() == Some("mov")));
            }
        }
        // Count zero-byte and attributed-zero files without classifying links,
        // metadata, or the excluded subtree as file bytes.
        let map = build(&snapshot, 1, true, 5000, &[]).unwrap();
        let colored = map
            .nodes
            .iter()
            .filter(|n| n.file_type.is_some() || n.kind == "file")
            .fold((0, 0, 0), |(files, logical, allocated), n| {
                (
                    files + n.file_count.unwrap_or(1),
                    logical + n.logical_bytes,
                    allocated + n.allocated_bytes,
                )
            });
        assert_eq!(colored, (5, 2600, 1024));
        let jpg = map
            .nodes
            .iter()
            .find(|n| n.file_type.as_deref() == Some("jpg"))
            .unwrap();
        assert_eq!(jpg.file_count, Some(2));
        assert_eq!(bytes(jpg), (700, 0));
        assert!(map.nodes.iter().any(|n| n.file_type.as_deref() == Some("")));
        let excluded = build(&snapshot, 9, true, 5000, &[]).unwrap();
        assert_honest(&excluded, &snapshot, 5000);
        assert!(excluded.nodes.iter().all(|n| n.file_type.is_none()));
        assert!(build(&snapshot, 9999, true, 5000, &[]).is_err());
    }
    #[test]
    fn deep_broad_tree_keeps_major_context_and_colors_descendants_beyond_bfs_budget() {
        let mut f = Fixture::new();
        let root = f.add(None, "fixture", NodeKind::Directory, 0, 1, false);
        let users = f.add(Some(root), "Users", NodeKind::Directory, 0, 1, false);
        let home = f.add(Some(users), "home", NodeKind::Directory, 0, 1, false);
        let docs = f.add(Some(home), "Documents", NodeKind::Directory, 0, 1, false);
        // Existing breadth-first expansion spends all 5000 tiles on folders.
        for index in 0..6200 {
            let folder = f.add(
                Some(docs),
                &format!("project-{index}"),
                NodeKind::Directory,
                0,
                1,
                false,
            );
            let mut parent = folder;
            for depth in 0..8 {
                parent = f.add(
                    Some(parent),
                    &format!("depth-{depth}"),
                    NodeKind::Directory,
                    0,
                    1,
                    false,
                );
            }
            f.add(
                Some(parent),
                if index % 2 == 0 {
                    "movie.MOV"
                } else {
                    "archive.ZIP"
                },
                NodeKind::File,
                8192,
                4096,
                false,
            );
        }
        let snapshot = f.finish();
        for allocated in [false, true] {
            let map = build(&snapshot, root, allocated, 5000, &[]).unwrap();
            assert_honest(&map, &snapshot, 5000);
            for expected in [users, home, docs] {
                assert!(map.nodes.iter().any(|n| n.id == expected));
            }
            let root_size = size(&snapshot.node(root).unwrap(), allocated);
            let colored: u64 = map
                .nodes
                .iter()
                .filter(|n| n.file_type.is_some() || n.kind == "file")
                .map(|n| {
                    if allocated {
                        n.allocated_bytes
                    } else {
                        n.logical_bytes
                    }
                })
                .sum();
            assert!(
                colored > root_size * 99 / 100,
                "{colored} of {root_size} colored"
            );
            assert!(map
                .nodes
                .iter()
                .any(|n| n.file_type.as_deref() == Some("mov")));
            assert!(map
                .nodes
                .iter()
                .any(|n| n.file_type.as_deref() == Some("zip")));
            assert!(map.nodes.iter().filter(|n| n.kind == "directory").count() <= MAX_DIRECTORIES);
            for n in map.nodes.iter().filter(|n| n.kind == "directory") {
                let mut depth = 0;
                let mut id = n.id;
                while id != root {
                    id = snapshot.node(id).unwrap().parent.unwrap();
                    depth += 1;
                }
                assert!(depth <= MAX_DEPTH);
            }
        }
    }
    #[test]
    fn diverse_extensions_use_neutral_remainder_without_false_type_labels() {
        let mut f = Fixture::new();
        let root = f.add(None, "fixture", NodeKind::Directory, 0, 0, false);
        let nested = f.add(Some(root), "nested", NodeKind::Directory, 0, 0, false);
        for index in 0..100 {
            f.add(
                Some(nested),
                &format!("file.ext{index}"),
                NodeKind::File,
                100,
                50,
                false,
            );
        }
        let snapshot = f.finish();
        for limit in [2, 3, 5000] {
            let map = build(&snapshot, root, true, limit, &[]).unwrap();
            assert_honest(&map, &snapshot, limit);
            let neutral = map
                .nodes
                .iter()
                .filter(|n| n.kind == "aggregate" && n.file_type.is_none())
                .map(|n| n.allocated_bytes)
                .sum::<u64>();
            assert!(neutral >= 4000, "omitted types must remain neutral");
        }
    }
}
