//! Metadata-only search and filters. Session ownership bounds this cache to one snapshot.
use crate::sorting::{self, Order};
#[cfg(test)]
use disk_scan::Node;
use disk_scan::{NodeKind, NodeRef, ScanState, Snapshot};
use std::os::unix::ffi::OsStrExt;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Query {
    text: String,
    category: String,
    min: Option<u64>,
    max: Option<u64>,
    order: Order,
    search: bool,
    extension: Option<String>,
    scope_id: Option<u32>,
}
impl Query {
    #[cfg(test)]
    pub(crate) fn new(
        query: String,
        category: String,
        min_bytes: Option<u64>,
        max_bytes: Option<u64>,
    ) -> Result<Self, String> {
        Self::with_order(query, category, min_bytes, max_bytes, Order::default())
    }
    pub(crate) fn with_order(
        query: String,
        category: String,
        min_bytes: Option<u64>,
        max_bytes: Option<u64>,
        order: Order,
    ) -> Result<Self, String> {
        if !matches!(
            category.as_str(),
            "all"
                | "images"
                | "videos"
                | "audio"
                | "documents"
                | "archives"
                | "applications"
                | "other"
        ) {
            return Err("Unknown file category".into());
        }
        if min_bytes.zip(max_bytes).is_some_and(|(min, max)| min > max) {
            return Err("Minimum size must not exceed maximum size".into());
        }
        Ok(Self {
            text: query.to_lowercase(),
            category,
            min: min_bytes,
            max: max_bytes,
            order,
            search: false,
            extension: None,
            scope_id: None,
        })
    }
    pub(crate) fn search(query: String, order: Order) -> Result<Self, String> {
        let mut result = Self::with_order(query, "all".into(), None, None, order)?;
        result.search = true;
        Ok(result)
    }
    pub(crate) fn with_file_type(
        mut self,
        extension: Option<String>,
        scope_id: Option<u32>,
    ) -> Self {
        self.extension = extension.map(|value| value.to_lowercase());
        self.scope_id = scope_id;
        self
    }
    fn matches(&self, node: &NodeRef<'_>) -> bool {
        if self.search {
            return !self.text.is_empty()
                && node.display_name().to_lowercase().contains(&self.text);
        }
        if node.parent.is_none() || node.excluded {
            return false;
        }
        if self
            .extension
            .as_ref()
            .is_some_and(|extension| !crate::file_types::matches(node, extension))
        {
            return false;
        }
        let size = node.allocated_size;
        (self.text.is_empty() || node.display_name().to_lowercase().contains(&self.text))
            && self.min.is_none_or(|min| size >= min)
            && self.max.is_none_or(|max| size <= max)
            && (self.category == "all" || category(node) == Some(self.category.as_str()))
    }
}
fn category(node: &NodeRef<'_>) -> Option<&'static str> {
    let extension = std::path::Path::new(node.native_name())
        .extension()
        .map(|e| e.as_bytes());
    let is = |list: &[&[u8]]| {
        extension.is_some_and(|ext| {
            list.iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
    };
    if node.identity.kind == NodeKind::Directory {
        return if is(&[b"app"]) {
            Some("applications")
        } else {
            None
        };
    }
    if node.identity.kind != NodeKind::File {
        return None;
    }
    if is(&[
        b"jpg", b"jpeg", b"png", b"gif", b"webp", b"bmp", b"tif", b"tiff", b"heic", b"heif",
        b"avif", b"svg", b"ico", b"dng", b"raw",
    ]) {
        Some("images")
    } else if is(&[
        b"mp4", b"m4v", b"mov", b"mkv", b"avi", b"mpg", b"mpeg", b"webm", b"wmv", b"flv", b"3gp",
    ]) {
        Some("videos")
    } else if is(&[
        b"mp3", b"m4a", b"aac", b"wav", b"flac", b"ogg", b"opus", b"aif", b"aiff", b"alac",
    ]) {
        Some("audio")
    } else if is(&[
        b"pdf", b"txt", b"md", b"rtf", b"doc", b"docx", b"odt", b"xls", b"xlsx", b"ods", b"csv",
        b"ppt", b"pptx", b"odp", b"pages", b"numbers", b"key", b"html", b"htm", b"epub",
    ]) {
        Some("documents")
    } else if is(&[
        b"zip", b"rar", b"7z", b"tar", b"gz", b"bz2", b"xz", b"zst", b"tgz",
    ]) {
        Some("archives")
    } else if is(&[b"pkg", b"dmg"]) {
        Some("applications")
    } else {
        Some("other")
    }
}
struct Entry {
    query: Query,
    nodes_address: usize,
    count: usize,
    root: std::path::PathBuf,
    ids: Vec<u32>,
}
#[derive(Default)]
pub(crate) struct Cache {
    entry: Option<Entry>,
    #[cfg(test)]
    builds: usize,
}
impl Cache {
    /// Owned payload buffers only; excludes stack fields and allocator overhead.
    #[cfg(any(test, feature = "performance-benchmark"))]
    pub(crate) fn retained_buffer_storage(&self) -> disk_scan::BufferStorage {
        let Some(entry) = &self.entry else {
            return disk_scan::BufferStorage::default();
        };
        let mut storage = disk_scan::BufferStorage {
            length_bytes: entry.ids.len() * std::mem::size_of::<u32>()
                + entry.root.as_os_str().len(),
            capacity_bytes: entry.ids.capacity() * std::mem::size_of::<u32>()
                + entry.root.capacity(),
        };
        storage.length_bytes += entry.query.text.len() + entry.query.category.len();
        storage.capacity_bytes += entry.query.text.capacity() + entry.query.category.capacity();
        if let Some(extension) = &entry.query.extension {
            storage.length_bytes += extension.len();
            storage.capacity_bytes += extension.capacity();
        }
        storage
    }
    pub(crate) fn page(
        &mut self,
        snapshot: &Snapshot,
        query: &Query,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<u32>, usize), String> {
        if query.scope_id.is_some_and(|id| snapshot.node(id).is_none()) {
            return Err("Unknown scope node".into());
        }
        let address = snapshot.storage_identity();
        let completed = snapshot.state == ScanState::Completed;
        let reuse = completed
            && self.entry.as_ref().is_some_and(|e| {
                e.query == *query
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
            let mut ids = Vec::new();
            if let Some(scope) = query.scope_id {
                crate::file_types::visit(snapshot, scope, |node| {
                    if query.matches(&node) {
                        ids.push(node.id);
                    }
                })?;
            } else {
                ids.extend(
                    snapshot
                        .iter_nodes()
                        .filter(|n| query.matches(n))
                        .map(|n| n.id),
                );
            }
            sorting::sort_ids(snapshot, &mut ids, query.order)?;
            if completed {
                self.entry = Some(Entry {
                    query: query.clone(),
                    nodes_address: address,
                    count: snapshot.node_count(),
                    root: snapshot.root.clone(),
                    ids,
                });
            } else {
                return Ok(page(&ids, offset, limit));
            }
        }
        Ok(page(
            &self.entry.as_ref().expect("completed cache populated").ids,
            offset,
            limit,
        ))
    }
}
fn page(ids: &[u32], offset: usize, limit: usize) -> (Vec<u32>, usize) {
    let start = offset.min(ids.len());
    let end = start.saturating_add(limit.min(500)).min(ids.len());
    (ids[start..end].to_vec(), ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use disk_scan::Identity;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};
    fn node(id: u32, name: &str, kind: NodeKind, length: u64, allocated: u64) -> Node {
        Node {
            id,
            parent: Some(0),
            children: vec![],
            name: name.into(),
            path: PathBuf::from("/fixture").join(name),
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
        }
    }
    fn snapshot(nodes: Vec<Node>) -> Snapshot {
        let mut s = Snapshot::empty(PathBuf::from("/fixture"));
        for mut node in nodes {
            let parent = node.parent;
            node.parent = if node.id == 0 { None } else { Some(0) };
            let id = node.id;
            s.append_legacy_node(node).unwrap();
            s.update_node(id, |metadata| metadata.parent = parent)
                .unwrap();
        }
        s.state = ScanState::Completed;
        s
    }
    fn predicate_snapshot(node: &Node) -> Snapshot {
        let mut nodes = Vec::new();
        for id in 0..node.id {
            let mut filler = node.clone();
            filler.id = id;
            filler.parent = if id == 0 { None } else { Some(0) };
            nodes.push(filler);
        }
        nodes.push(node.clone());
        snapshot(nodes)
    }
    impl Query {
        fn matches_legacy(&self, node: &Node) -> bool {
            let snapshot = predicate_snapshot(node);
            self.matches(&snapshot.node(node.id).unwrap())
        }
    }
    fn category_legacy(node: &Node) -> Option<&'static str> {
        let snapshot = predicate_snapshot(node);
        category(&snapshot.node(node.id).unwrap())
    }
    fn query(category: &str, min: Option<u64>, max: Option<u64>) -> Query {
        Query::new(String::new(), category.into(), min, max).unwrap()
    }
    #[test]
    fn extension_and_scope_are_exact_and_distinct_cache_keys() {
        let snapshot = crate::file_types::tests::fixture();
        let mut cache = Cache::default();
        let base = query("all", None, None);
        let scoped = base.clone().with_file_type(Some("JPG".into()), Some(1));
        let (mut ids, total) = cache.page(&snapshot, &scoped, 0, 500).unwrap();
        ids.sort_unstable();
        assert_eq!((ids, total), (vec![3, 4], 2));
        assert_eq!(cache.builds, 1);
        cache.page(&snapshot, &scoped, 0, 500).unwrap();
        assert_eq!(cache.builds, 1);
        let whole = base.clone().with_file_type(Some("jpg".into()), Some(0));
        assert_eq!(cache.page(&snapshot, &whole, 0, 500).unwrap().1, 3);
        assert_eq!(cache.builds, 2);
        let extensionless = base.clone().with_file_type(Some("".into()), Some(1));
        let (mut ids, _) = cache.page(&snapshot, &extensionless, 0, 500).unwrap();
        ids.sort_unstable();
        assert_eq!(ids, vec![9, 10]);
        assert_eq!(cache.builds, 3);
        let partial = base.clone().with_file_type(Some("jp".into()), Some(1));
        assert_eq!(cache.page(&snapshot, &partial, 0, 500).unwrap().1, 0);
        assert_eq!(cache.builds, 4);
        let invalid = base.with_file_type(None, Some(999));
        assert!(cache.page(&snapshot, &invalid, 0, 500).is_err());
    }
    #[test]
    fn retained_buffers_include_query_payload_and_release_running_cache() {
        let mut snapshot = snapshot(vec![node(0, "needle", NodeKind::File, 1, 1)]);
        let mut cache = Cache::default();
        let text = "needle".repeat(100);
        let query = Query::search(text.clone(), Order::default()).unwrap();
        cache.page(&snapshot, &query, 0, 1).unwrap();
        let storage = cache.retained_buffer_storage();
        assert!(storage.length_bytes >= snapshot.root.as_os_str().len() + text.len() + 3);
        assert!(storage.capacity_bytes >= storage.length_bytes);
        snapshot.state = ScanState::Running;
        cache.page(&snapshot, &query, 0, 1).unwrap();
        assert_eq!(cache.retained_buffer_storage().capacity_bytes, 0);
    }
    #[test]
    fn all_extensions_native_names_and_kind_boundaries() {
        for (expected, extensions) in [
            ("images","jpg jpeg png gif webp bmp tif tiff heic heif avif svg ico dng raw"),
            ("videos","mp4 m4v mov mkv avi mpg mpeg webm wmv flv 3gp"),
            ("audio","mp3 m4a aac wav flac ogg opus aif aiff alac"),
            ("documents","pdf txt md rtf doc docx odt xls xlsx ods csv ppt pptx odp pages numbers key html htm epub"),
            ("archives","zip rar 7z tar gz bz2 xz zst tgz"), ("applications","pkg dmg")
        ] {
            for extension in extensions.split_whitespace() {
                let mut n = node(1,&format!("file.{}",extension.to_uppercase()),NodeKind::File,0,0);
                assert!(query(expected,None,None).matches_legacy(&n), "{extension}");
                n.path = PathBuf::from("/fixture").join(OsString::from_vec([vec![0xff,b'.'],extension.as_bytes().to_vec()].concat()));
                assert_eq!(category_legacy(&n),Some(expected));
                n.identity.kind = NodeKind::Symlink;
                assert_eq!(category_legacy(&n),None);
                assert!(query("all",None,None).matches_legacy(&n));
            }
        }
        for name in ["unknown.xyz", "no-extension", ".png", "regular.app"] {
            assert_eq!(
                category_legacy(&node(1, name, NodeKind::File, 0, 0)),
                Some("other")
            );
        }
        assert_eq!(
            category_legacy(&node(1, "Editor.APP", NodeKind::Directory, 0, 0)),
            Some("applications")
        );
        for kind in [NodeKind::Directory, NodeKind::Symlink, NodeKind::Other] {
            assert_eq!(category_legacy(&node(1, "ordinary.png", kind, 0, 0)), None);
        }
    }
    #[test]
    fn predicates_bounds_metrics_and_exclusions() {
        let mut n = node(1, "Holiday.PNG", NodeKind::File, 1_000_000, 4096);
        n.hard_link = true;
        let q = Query::new("HOLI".into(), "images".into(), Some(4096), Some(4096)).unwrap();
        assert!(q.matches_legacy(&n));
        assert!(!Query::new("absent".into(), "images".into(), None, None,)
            .unwrap()
            .matches_legacy(&n));
        n.allocated_size = 0;
        assert!(query("images", Some(0), Some(0)).matches_legacy(&n));
        n.excluded = true;
        assert!(!query("all", None, None).matches_legacy(&n));
        n.excluded = false;
        n.parent = None;
        assert!(!query("all", None, None).matches_legacy(&n));
        assert!(Query::new("".into(), "bad".into(), None, None).is_err());
        assert!(Query::new("".into(), "all".into(), Some(2), Some(1),).is_err());
        let folder = node(2, "Folder", NodeKind::Directory, 9999, 8888);
        assert!(query("all", Some(8888), Some(8888)).matches_legacy(&folder));
        assert!(!query("other", None, None).matches_legacy(&folder));
    }
    #[test]
    fn real_hard_links_and_symlink_classification() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("photo.png"), b"photo").unwrap();
        std::fs::hard_link(dir.path().join("photo.png"), dir.path().join("alias.png")).unwrap();
        std::os::unix::fs::symlink("photo.png", dir.path().join("link.png")).unwrap();
        let s = disk_scan::scan(
            dir.path().to_path_buf(),
            disk_scan::ScanOptions::default(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        let (_, images) = Cache::default()
            .page(&s, &query("images", None, None), 0, 500)
            .unwrap();
        assert_eq!(images, 2); // The symlink's suffix must not classify it as a regular image.
        let (_, zero_images) = Cache::default()
            .page(&s, &query("images", Some(0), Some(0)), 0, 500)
            .unwrap();
        assert_eq!(zero_images, 1); // Only one path owns the hard-link allocation.
    }

    #[test]
    fn deterministic_pages_cache_reuse_and_running_transitions() {
        let mut nodes = vec![
            node(0, "z", NodeKind::File, 2, 1),
            node(1, "b", NodeKind::File, 1, 1),
            node(2, "a", NodeKind::File, 1, 1),
            node(3, "a", NodeKind::File, 1, 1),
        ];
        nodes.extend((4..608).map(|id| node(id, &format!("x{id}"), NodeKind::File, 0, 0)));
        let mut s = snapshot(nodes);
        let q = query("all", None, None);
        let mut cache = Cache::default();
        assert_eq!(cache.page(&s, &q, 0, 2).unwrap().0, vec![2, 3]);
        assert_eq!(cache.page(&s, &q, 2, 2).unwrap().0, vec![1, 0]);
        assert_eq!(cache.builds, 1);
        let empty = cache.page(&s, &q, 0, 0).unwrap();
        assert!(empty.0.is_empty());
        assert_eq!(empty.1, s.node_count());
        assert_eq!(cache.builds, 1);
        assert_eq!(cache.page(&s, &q, 0, usize::MAX).unwrap().0.len(), 500);
        assert!(cache.page(&s, &q, usize::MAX, 500).unwrap().0.is_empty());
        assert!(cache.entry.as_ref().is_some_and(|e| e.query == q));
        s.state = ScanState::Running;
        cache.page(&s, &q, 0, 1).unwrap();
        cache.page(&s, &q, 0, 1).unwrap();
        assert_eq!(cache.builds, 3);
        assert!(cache.entry.is_none());
        s.update_node(0, |node| node.allocated_size = 0).unwrap();
        s.state = ScanState::Completed;
        assert_eq!(cache.page(&s, &q, 0, 1).unwrap().0, vec![2]);
        assert_eq!(cache.builds, 4);
        let other = snapshot(vec![node(0, "different", NodeKind::File, 1, 1)]);
        assert_eq!(cache.page(&other, &q, 0, 10).unwrap(), (vec![0], 1));
        assert_eq!(cache.builds, 5);
    }
    #[test]
    fn search_membership_and_sort_orders_share_one_cache() {
        let mut nodes = vec![
            node(0, "ROOT image2", NodeKind::Directory, 100, 1),
            node(1, "image10.png", NodeKind::File, 10, 1),
            node(2, "image2.png", NodeKind::File, 2, 10),
            node(3, "image02.png", NodeKind::File, 3, 0),
        ];
        nodes[0].parent = None;
        nodes[3].excluded = true;
        let mut s = snapshot(nodes);
        let mut cache = Cache::default();
        let filter = Query::with_order(
            "image".into(),
            "all".into(),
            None,
            None,
            Order::NameAscending,
        )
        .unwrap();
        assert_eq!(cache.page(&s, &filter, 0, 500).unwrap(), (vec![2, 1], 2));
        let search = Query::search("IMAGE".into(), Order::NameAscending).unwrap();
        assert_eq!(
            cache.page(&s, &search, 0, 500).unwrap(),
            (vec![0, 2, 3, 1], 4)
        );
        assert_eq!(cache.builds, 2);
        assert!(cache.entry.as_ref().unwrap().query.search);
        assert_eq!(cache.page(&s, &search, 1, 1).unwrap().0, vec![2]);
        assert_eq!(cache.builds, 2);
        let desc = Query::search("image".into(), Order::NameDescending).unwrap();
        assert_eq!(cache.page(&s, &desc, 0, 500).unwrap().0, vec![0, 1, 3, 2]);
        let size = Query::search("image".into(), Order::SizeDescending).unwrap();
        assert_eq!(cache.page(&s, &size, 0, 500).unwrap().0, vec![0, 2, 1, 3]);
        assert_eq!(cache.builds, 4);
        assert!(cache
            .page(
                &s,
                &Query::search("".into(), Order::default()).unwrap(),
                0,
                500
            )
            .unwrap()
            .0
            .is_empty());
        s.state = ScanState::Running;
        cache.page(&s, &search, 0, 500).unwrap();
        let mut node = s.legacy_node(2).unwrap();
        node.name = "absent".into();
        s.replace_node(2, node).unwrap();
        assert_eq!(cache.page(&s, &search, 0, 500).unwrap().1, 3);
        assert!(cache.entry.is_none());
    }

    #[test]
    #[ignore = "million-entry synthetic filter benchmark"]
    fn million_entry_profile() {
        let s = snapshot(
            (0..1_000_000)
                .map(|id| {
                    node(
                        id,
                        &format!("image{:07}.png", (u64::from(id) * 7919) % 1_000_000),
                        NodeKind::File,
                        u64::from(id),
                        u64::from(id),
                    )
                })
                .collect(),
        );
        let mut cache = Cache::default();
        for order in [Order::SizeDescending, Order::NameAscending] {
            let q = Query::with_order("".into(), "images".into(), None, None, order).unwrap();
            let start = std::time::Instant::now();
            let first = cache.page(&s, &q, 0, 500).unwrap();
            let first_us = start.elapsed().as_micros();
            let start = std::time::Instant::now();
            let second = cache.page(&s, &q, 500, 500).unwrap();
            let page_us = start.elapsed().as_micros();
            assert_eq!(first.1, 1_000_000);
            assert_eq!(second.0.len(), 500);
            eprintln!("filter_sort_profile: entries=1000000 order={order:?} first_us={first_us} cached_page_us={page_us}");
        }
        assert_eq!(cache.builds, 2);
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        assert_eq!(
            unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
            0
        );
        let rss = unsafe { usage.assume_init() }.ru_maxrss;
        eprintln!("filter_sort_profile: entries=1000000 peak_rss_native={rss}");
    }
}
