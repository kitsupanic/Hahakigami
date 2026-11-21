//! Stable metadata-only list ordering. Maps retain their independent layout ordering.
#[cfg(test)]
use disk_scan::Node;
use disk_scan::{NodeKind, NodeRef, ScanState, Snapshot};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, os::unix::ffi::OsStrExt, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Order {
    #[default]
    SizeDescending,
    SizeAscending,
    NameAscending,
    NameDescending,
}
impl Order {
    pub(crate) fn parse(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("sizeDescending") => Ok(Self::SizeDescending),
            Some("sizeAscending") => Ok(Self::SizeAscending),
            Some("nameAscending") => Ok(Self::NameAscending),
            Some("nameDescending") => Ok(Self::NameDescending),
            _ => Err("Unknown sort order".into()),
        }
    }
}
fn group(a: &NodeRef<'_>, b: &NodeRef<'_>) -> Ordering {
    (b.identity.kind == NodeKind::Directory).cmp(&(a.identity.kind == NodeKind::Directory))
}
fn tie(
    snapshot: &Snapshot,
    a: &NodeRef<'_>,
    b: &NodeRef<'_>,
    left: &mut PathBuf,
    right: &mut PathBuf,
) -> Ordering {
    // Siblings share a path prefix, so their path order is their name order. A path override
    // can break that, so with any override present every pair builds both paths.
    if a.parent == b.parent && !snapshot.has_path_overrides() {
        return a
            .native_name()
            .as_bytes()
            .cmp(b.native_name().as_bytes())
            .then(a.id.cmp(&b.id));
    }
    let (a, b) = (a.id, b.id);
    // Every selected path is checked before sorting; the immutable snapshot
    // cannot change between that pass and this comparator.
    snapshot.path_into(a, left).expect("prevalidated sort path");
    snapshot
        .path_into(b, right)
        .expect("prevalidated sort path");
    left.as_os_str()
        .as_bytes()
        .cmp(right.as_os_str().as_bytes())
        .then(a.cmp(&b))
}
/// Case folding is done before sorting; arbitrarily long ASCII numbers never overflow.
fn natural(a: &[u8], b: &[u8]) -> Ordering {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (start_a, start_b) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let (mut significant_a, mut significant_b) = (start_a, start_b);
            while significant_a < i && a[significant_a] == b'0' {
                significant_a += 1;
            }
            while significant_b < j && b[significant_b] == b'0' {
                significant_b += 1;
            }
            let order = (i - significant_a)
                .cmp(&(j - significant_b))
                .then_with(|| a[significant_a..i].cmp(&b[significant_b..j]))
                .then((i - start_a).cmp(&(j - start_b)));
            if order != Ordering::Equal {
                return order;
            }
        } else {
            let order = a[i].cmp(&b[j]);
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}
pub(crate) fn sort_ids(snapshot: &Snapshot, ids: &mut [u32], order: Order) -> Result<(), String> {
    let (mut left, mut right) = (PathBuf::new(), PathBuf::new());
    let first_parent = ids
        .first()
        .and_then(|&id| snapshot.node(id))
        .map(|n| n.parent);
    let mut one_folder = true;
    for &id in ids.iter() {
        let node = snapshot.node(id).ok_or("Invalid sort node ID")?;
        one_folder &= Some(node.parent) == first_parent;
    }
    // `tie` builds paths only across folders or with path overrides, so only then are they validated here.
    if !one_folder || snapshot.has_path_overrides() {
        for &id in ids.iter() {
            snapshot.path_into(id, &mut left)?;
        }
    }
    match order {
        Order::SizeDescending | Order::SizeAscending => ids.sort_unstable_by(|&a, &b| {
            let (a, b) = (
                snapshot.node(a).expect("checked sort id"),
                snapshot.node(b).expect("checked sort id"),
            );
            let sizes = if order == Order::SizeDescending {
                b.allocated_size.cmp(&a.allocated_size)
            } else {
                a.allocated_size.cmp(&b.allocated_size)
            };
            group(&a, &b)
                .then(sizes)
                .then_with(|| tie(snapshot, &a, &b, &mut left, &mut right))
        }),
        Order::NameAscending | Order::NameDescending => {
            // Keep keys only for the selected IDs, not for every node in the snapshot.
            let mut keyed: Vec<_> = ids
                .iter()
                .map(|&id| {
                    (
                        id,
                        // shortcut: byte order after NFC folding, so accented letters sort after "z"; upgrade to
                        // locale collation if users ask for dictionary order.
                        crate::filters::folded(
                            &snapshot.node(id).expect("checked sort id").display_name(),
                        ),
                    )
                })
                .collect();
            keyed.sort_unstable_by(|(a, ka), (b, kb)| {
                let (a, b) = (
                    snapshot.node(*a).expect("checked sort id"),
                    snapshot.node(*b).expect("checked sort id"),
                );
                let names = natural(ka.as_bytes(), kb.as_bytes());
                group(&a, &b)
                    .then(if order == Order::NameDescending {
                        names.reverse()
                    } else {
                        names
                    })
                    .then_with(|| tie(snapshot, &a, &b, &mut left, &mut right))
            });
            for (slot, (id, _)) in ids.iter_mut().zip(keyed) {
                *slot = id;
            }
        }
    }
    Ok(())
}
pub(crate) struct Entry<K> {
    pub(crate) key: K,
    nodes_address: usize,
    count: usize,
    root: PathBuf,
    pub(crate) ids: Vec<u32>,
}
/// One cached sorted ID list, rebuilt when the key or the snapshot changes. Shared by folder listings and filters.
pub(crate) struct ResultCache<K> {
    pub(crate) entry: Option<Entry<K>>,
    /// The most ids one page returns.
    pub(crate) page_cap: usize,
    #[cfg(test)]
    pub(crate) builds: usize,
}
impl<K> Default for ResultCache<K> {
    fn default() -> Self {
        Self {
            entry: None,
            page_cap: 500,
            #[cfg(test)]
            builds: 0,
        }
    }
}
pub(crate) type Cache = ResultCache<(u32, Order)>;
impl<K: PartialEq + Clone> ResultCache<K> {
    /// Owned payload buffers only (the key's via `key_storage`); excludes stack fields and allocator overhead.
    #[cfg(any(test, feature = "performance-benchmark"))]
    pub(crate) fn storage_with_key(
        &self,
        key_storage: impl FnOnce(&K) -> disk_scan::BufferStorage,
    ) -> disk_scan::BufferStorage {
        let Some(entry) = &self.entry else {
            return disk_scan::BufferStorage::default();
        };
        let key = key_storage(&entry.key);
        disk_scan::BufferStorage {
            length_bytes: entry.ids.len() * std::mem::size_of::<u32>()
                + entry.root.as_os_str().len()
                + key.length_bytes,
            capacity_bytes: entry.ids.capacity() * std::mem::size_of::<u32>()
                + entry.root.capacity()
                + key.capacity_bytes,
        }
    }
    /// A page of the list for `key`: reused when the snapshot is final and unchanged, otherwise built.
    pub(crate) fn get_or_build(
        &mut self,
        snapshot: &Snapshot,
        key: &K,
        build: impl FnOnce() -> Result<Vec<u32>, String>,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<u32>, usize), String> {
        // A cancelled or failed scan keeps its partial results unchanged, so they cache like a completed one.
        let final_state = snapshot.state != ScanState::Running;
        let address = snapshot.storage_identity();
        let reuse = final_state
            && self.entry.as_ref().is_some_and(|e| {
                e.key == *key
                    && e.nodes_address == address
                    && e.count == snapshot.node_count()
                    && e.root == snapshot.root
            });
        if !reuse {
            self.entry = None;
            #[cfg(test)]
            {
                self.builds += 1;
            }
            let ids = build()?;
            if final_state {
                self.entry = Some(Entry {
                    key: key.clone(),
                    nodes_address: address,
                    count: snapshot.node_count(),
                    root: snapshot.root.clone(),
                    ids,
                });
            } else {
                return Ok(page(&ids, offset, limit, self.page_cap));
            }
        }
        Ok(page(
            &self.entry.as_ref().expect("cache populated").ids,
            offset,
            limit,
            self.page_cap,
        ))
    }
}
impl Cache {
    #[cfg(any(test, feature = "performance-benchmark"))]
    pub(crate) fn retained_buffer_storage(&self) -> disk_scan::BufferStorage {
        self.storage_with_key(|_| disk_scan::BufferStorage::default())
    }
    pub(crate) fn page(
        &mut self,
        snapshot: &Snapshot,
        node_id: u32,
        order: Order,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<u32>, usize), String> {
        let node = snapshot.node(node_id).ok_or("Unknown node ID")?;
        self.get_or_build(
            snapshot,
            &(node_id, order),
            || {
                let mut ids = node.children().to_vec();
                if ids.iter().any(|&id| snapshot.node(id).is_none()) {
                    return Err("Invalid child node ID".into());
                }
                sort_ids(snapshot, &mut ids, order)?;
                Ok(ids)
            },
            offset,
            limit,
        )
    }
}
pub(crate) fn page(ids: &[u32], offset: usize, limit: usize, cap: usize) -> (Vec<u32>, usize) {
    let start = offset.min(ids.len());
    let end = start.saturating_add(limit.min(cap)).min(ids.len());
    (ids[start..end].to_vec(), ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use disk_scan::Identity;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    fn fixture() -> Snapshot {
        let mut s = Snapshot::empty(PathBuf::from("/fixture"));
        for (id, (name, kind, length, allocated)) in [
            ("root", NodeKind::Directory, 99, 99),
            ("Folder10", NodeKind::Directory, 1, 100),
            ("Folder2", NodeKind::Directory, 2, 0),
            ("file10", NodeKind::File, 10, 1),
            ("File2", NodeKind::File, 2, 10),
            ("file02", NodeKind::File, 2, 0),
            ("Ä", NodeKind::File, 0, 0),
            ("ä", NodeKind::File, 0, 0),
            ("�", NodeKind::File, 0, 0),
            ("�", NodeKind::File, 0, 0),
            ("foo", NodeKind::Symlink, 0, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let id = id as u32;
            let path = if id == 8 || id == 9 {
                s.root
                    .join(OsString::from_vec(vec![if id == 8 { 0xff } else { 0xfe }]))
            } else {
                s.root.join(name)
            };
            s.append_legacy_node(Node {
                id,
                parent: if id == 0 { None } else { Some(0) },
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
                hard_link: false,
                clone_metadata: None,
                excluded: false,
            })
            .unwrap();
        }
        s.set_children(0, &(1..=10).collect::<Vec<_>>()).unwrap();
        s.set_children(1, &[3, 4]).unwrap();
        s.set_children(2, &[5]).unwrap();
        s.state = ScanState::Completed;
        s
    }
    #[test]
    fn retained_buffers_track_completed_pages_and_running_release() {
        let mut snapshot = fixture();
        let mut cache = Cache::default();
        assert_eq!(cache.retained_buffer_storage().capacity_bytes, 0);
        cache.page(&snapshot, 0, Order::default(), 0, 2).unwrap();
        let storage = cache.retained_buffer_storage();
        assert!(storage.length_bytes >= 10 * 4 + snapshot.root.as_os_str().len());
        assert!(storage.capacity_bytes >= storage.length_bytes);
        cache.page(&snapshot, 0, Order::default(), 2, 2).unwrap();
        assert_eq!(
            cache.retained_buffer_storage().capacity_bytes,
            storage.capacity_bytes
        );
        snapshot.state = ScanState::Running;
        cache.page(&snapshot, 0, Order::default(), 0, 2).unwrap();
        assert_eq!(cache.retained_buffer_storage().capacity_bytes, 0);
    }
    #[test]
    fn orders_group_folders_and_preserve_raw_ties() {
        let s = fixture();
        for (order, expected) in [
            (Order::SizeDescending, vec![1, 2, 4, 3, 5, 10, 6, 7, 9, 8]),
            (Order::SizeAscending, vec![2, 1, 5, 10, 6, 7, 9, 8, 3, 4]),
            (Order::NameAscending, vec![2, 1, 4, 5, 3, 10, 6, 7, 9, 8]),
            (Order::NameDescending, vec![1, 2, 9, 8, 6, 7, 10, 3, 5, 4]),
        ] {
            let mut ids = s.node(0).unwrap().children().to_vec();
            ids.reverse();
            sort_ids(&s, &mut ids, order).unwrap();
            assert_eq!(ids, expected, "{order:?}");
        }
    }
    #[test]
    fn identical_native_paths_use_ascending_id_in_every_mode() {
        let mut s = fixture();
        let mut duplicate = s.legacy_node(4).unwrap();
        duplicate.id = 11;
        s.append_legacy_node(duplicate).unwrap();
        for order in [
            Order::SizeDescending,
            Order::SizeAscending,
            Order::NameAscending,
            Order::NameDescending,
        ] {
            let mut ids = vec![11, 4];
            sort_ids(&s, &mut ids, order).unwrap();
            assert_eq!(ids, vec![4, 11]);
        }
    }
    #[test]
    fn name_order_matches_composed_and_decomposed_spellings() {
        let mut s = fixture();
        for (id, name) in [(11, "zebra"), (12, "E\u{301}clair"), (13, "\u{c9}cran")] {
            let mut node = s.legacy_node(3).unwrap();
            node.id = id;
            node.name = name.into();
            node.path = s.root.join(name);
            s.append_legacy_node(node).unwrap();
        }
        s.set_children(0, &[11, 12, 13]).unwrap();
        let mut ids = vec![11, 13, 12];
        sort_ids(&s, &mut ids, Order::NameAscending).unwrap();
        // Both spellings sort together; before NFC folding the decomposed one sorted before "zebra".
        assert_eq!(ids, vec![11, 12, 13]);
    }
    #[test]
    fn folder_pages_cap_output_without_cloning_full_list() {
        let mut s = fixture();
        for id in 11..=610 {
            let mut node = s.legacy_node(3).unwrap();
            node.id = id;
            node.name = format!("file{id}");
            node.path = s.root.join(&node.name);
            s.append_legacy_node(node).unwrap();
            let mut children = s.node(0).unwrap().children().to_vec();
            children.push(id);
            s.set_children(0, &children).unwrap();
        }
        let mut cache = Cache::default();
        let first = cache
            .page(&s, 0, Order::NameAscending, 0, usize::MAX)
            .unwrap();
        assert_eq!((first.0.len(), first.1), (500, 610));
        let last = cache
            .page(&s, 0, Order::NameAscending, 500, usize::MAX)
            .unwrap();
        assert_eq!((last.0.len(), last.1), (110, 610));
        assert_eq!(cache.builds, 1);
        assert_eq!(cache.entry.as_ref().unwrap().ids.len(), 610);
    }

    #[test]
    fn invalid_selected_paths_fail_before_reordering() {
        let mut s = fixture();
        assert!(sort_ids(&s, &mut [1, 99], Order::NameAscending).is_err());
        let mut root = s.legacy_node(0).unwrap();
        root.path = s.root.clone();
        let mut child = s.legacy_node(1).unwrap();
        child.path = s.root.join("child");
        child.id = 1;
        child.parent = Some(0);
        let mut compact = Snapshot::empty(s.root.clone());
        compact.append_legacy_node(root).unwrap();
        compact.append_legacy_node(child).unwrap();
        compact.update_node(1, |n| n.parent = Some(1)).unwrap();
        let mut ids = [1, 0];
        assert!(sort_ids(&compact, &mut ids, Order::SizeAscending).is_err());
        assert_eq!(ids, [1, 0]);
        s.set_children(0, &[99]).unwrap();
        assert!(Cache::default()
            .page(&s, 0, Order::default(), 0, 1)
            .is_err());
    }

    #[test]
    fn natural_numbers_have_no_parse_limit_and_order_zero_padding() {
        assert_eq!(natural(b"file2", b"file10"), Ordering::Less);
        assert_eq!(natural(b"file2", b"file02"), Ordering::Less);
        assert_eq!(natural(b"file0", b"file00"), Ordering::Less);
        assert_eq!(natural(b"file0002", b"file3"), Ordering::Less);
        assert_eq!(natural(b"a1b2", b"a1b10"), Ordering::Less);
        assert_eq!(natural(b"a", b"a0"), Ordering::Less);
        let huge = format!("file{}", "9".repeat(1000));
        let larger = format!("file1{}", "0".repeat(1000));
        assert_eq!(natural(huge.as_bytes(), larger.as_bytes()), Ordering::Less);
        assert_eq!(Order::parse(None).unwrap(), Order::SizeDescending);
        for order in [
            Order::SizeDescending,
            Order::SizeAscending,
            Order::NameAscending,
            Order::NameDescending,
        ] {
            let text = serde_json::to_value(order).unwrap();
            assert_eq!(Order::parse(text.as_str()).unwrap(), order);
        }
        assert!(Order::parse(Some("unknown")).is_err());
        assert!(Order::parse(Some("")).is_err());
    }
    #[test]
    fn last_folder_cache_pages_and_running_changes() {
        let mut s = fixture();
        let mut cache = Cache::default();
        assert_eq!(
            cache.page(&s, 0, Order::NameAscending, 0, 2).unwrap(),
            (vec![2, 1], 10)
        );
        assert_eq!(
            cache.page(&s, 0, Order::NameAscending, 2, 2).unwrap(),
            (vec![4, 5], 10)
        );
        assert_eq!(cache.builds, 1);
        assert_eq!(
            cache
                .page(&s, 0, Order::NameAscending, usize::MAX, 500)
                .unwrap(),
            (vec![], 10)
        );
        assert_eq!(
            cache.page(&s, 0, Order::NameAscending, 0, 0).unwrap(),
            (vec![], 10)
        );
        cache.page(&s, 1, Order::NameAscending, 0, 500).unwrap();
        assert_eq!(cache.builds, 2);
        assert_eq!(cache.entry.as_ref().unwrap().key.0, 1);
        cache.page(&s, 0, Order::NameAscending, 0, 500).unwrap();
        assert_eq!(cache.builds, 3);
        cache.page(&s, 0, Order::SizeAscending, 0, 500).unwrap();
        assert_eq!(cache.builds, 4);
        assert!(cache.page(&s, 99, Order::default(), 0, 500).is_err());
        s.state = ScanState::Running;
        cache.page(&s, 0, Order::NameAscending, 0, 500).unwrap();
        let mut node = s.legacy_node(4).unwrap();
        node.name = "zzz".into();
        s.replace_node(4, node).unwrap();
        assert_eq!(
            cache.page(&s, 0, Order::NameAscending, 2, 1).unwrap().0,
            vec![5]
        );
        assert_eq!(cache.builds, 6);
        assert!(cache.entry.is_none());
        s.state = ScanState::Completed;
        cache.page(&s, 0, Order::NameAscending, 0, 500).unwrap();
        assert_eq!(cache.builds, 7);
        let other = fixture();
        cache.page(&other, 0, Order::NameAscending, 0, 500).unwrap();
        assert_eq!(cache.builds, 8);
    }
    /// The pre-shortcut order: size, then full native path bytes, then id.
    fn reference_size_order(snapshot: &Snapshot, ids: &[u32], order: Order) -> Vec<u32> {
        let path = |id| snapshot.path(id).unwrap();
        let mut sorted = ids.to_vec();
        sorted.sort_by(|&a, &b| {
            let (na, nb) = (snapshot.node(a).unwrap(), snapshot.node(b).unwrap());
            let sizes = if order == Order::SizeDescending {
                nb.allocated_size.cmp(&na.allocated_size)
            } else {
                na.allocated_size.cmp(&nb.allocated_size)
            };
            group(&na, &nb)
                .then(sizes)
                .then_with(|| {
                    path(a)
                        .as_os_str()
                        .as_bytes()
                        .cmp(path(b).as_os_str().as_bytes())
                })
                .then(a.cmp(&b))
        });
        sorted
    }
    #[test]
    fn size_ties_keep_full_path_order_for_siblings_and_other_folders() {
        let mut s = Snapshot::empty(PathBuf::from("/tie"));
        // (name, parent, is_directory): equal sizes, ids deliberately out of name order.
        let nodes = [
            ("root", None, true),
            ("b", Some(0), true),
            ("a", Some(0), true),
            ("z", Some(1), false),
            ("m", Some(1), false),
            ("z", Some(2), false),
            ("a", Some(2), false),
            ("a", Some(0), false),
            ("b.txt", Some(0), false),
        ];
        for (id, (name, parent, dir)) in nodes.into_iter().enumerate() {
            let id = id as u32;
            s.append_legacy_node(Node {
                id,
                parent,
                children: vec![],
                name: name.into(),
                path: parent.map_or(s.root.clone(), |p| {
                    s.legacy_node(p).unwrap().path.join(name)
                }),
                identity: Identity {
                    device: 1,
                    inode: u64::from(id),
                    kind: if dir {
                        NodeKind::Directory
                    } else {
                        NodeKind::File
                    },
                },
                allocated_size: 4096,
                length: 0,
                own_allocated_size: 4096,
                is_directory: dir,
                hard_link: false,
                clone_metadata: None,
                excluded: false,
            })
            .unwrap();
        }
        s.set_children(0, &[1, 2, 7, 8]).unwrap();
        s.set_children(1, &[3, 4]).unwrap();
        s.set_children(2, &[5, 6]).unwrap();
        let all: Vec<u32> = (1..9).collect();
        for order in [Order::SizeDescending, Order::SizeAscending] {
            for ids in [all.clone(), vec![8, 7, 2, 1], vec![4, 3], vec![6, 5]] {
                let mut sorted = ids.clone();
                sort_ids(&s, &mut sorted, order).unwrap();
                assert_eq!(sorted, reference_size_order(&s, &ids, order));
            }
        }
        // The fixture has path overrides, which must not be shortcut by name.
        let overridden = fixture();
        let ids: Vec<u32> = (1..=10).collect();
        for order in [Order::SizeDescending, Order::SizeAscending] {
            let mut sorted = ids.clone();
            sort_ids(&overridden, &mut sorted, order).unwrap();
            assert_eq!(sorted, reference_size_order(&overridden, &ids, order));
        }
    }
    #[test]
    fn cancelled_and_failed_scans_cache_like_completed_ones() {
        for state in [ScanState::Cancelled, ScanState::Failed] {
            let mut s = fixture();
            s.state = state;
            let mut cache = Cache::default();
            cache.page(&s, 0, Order::NameAscending, 0, 2).unwrap();
            cache.page(&s, 0, Order::NameAscending, 2, 2).unwrap();
            assert_eq!(cache.builds, 1);
        }
    }
}
