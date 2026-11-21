//! Conservative metadata reuse. Cached paths are data, never traversal authority.
use super::*;

pub struct IncrementalSeed {
    /// Shared so a caller that keeps the baseline does not deep-copy the tree.
    pub snapshot: Arc<Snapshot>,
    pub dirty_paths: Vec<PathBuf>,
    /// Listings to reread without invalidating otherwise unchanged descendants.
    pub dirty_directories: Vec<PathBuf>,
    /// `dirty_paths` already holds every cached alias of each changed inode, so a changed path with no
    /// cached node (a new file) cannot hide a changed alias. When false, such a path makes every cached
    /// hard link dirty.
    pub aliases_resolved: bool,
}
#[derive(Debug, Default, Serialize)]
pub struct ReuseStats {
    pub reused_entries: u64,
    pub rescanned_directories: u64,
    pub fallback_reason: Option<String>,
    /// Exact metadata equivalence: no dirty history, all descendant listings
    /// reused, and the freshly enumerated root listing and totals match.
    pub unchanged_metadata: bool,
}
pub(crate) struct CachedListing {
    seed: Arc<Snapshot>,
    id: u32,
    // Derived from the same owned entries during staging, never an independent hint.
    pub(crate) has_directories: bool,
}
/// Scheduling hints. Keyed by inode, not path: a path per directory is most of the footprint of a
/// million-directory rescan. Inodes are unique here because aliased directories are left out.
pub(crate) struct Priorities {
    seed: Arc<Snapshot>,
    /// (device, inode) -> (seed node id, cost)
    costs: HashMap<(u64, u64), (u32, u64)>,
}
impl Priorities {
    /// The seed path is rebuilt only on a hit; a hint needs the exact identity and path.
    pub(crate) fn cost(&self, path: &Path, identity: &Identity) -> u64 {
        match self.costs.get(&(identity.device, identity.inode)) {
            Some(&(id, cost))
                if self.seed.node(id).is_some_and(|n| n.identity == *identity)
                    && self.seed.path(id).is_ok_and(|seed_path| seed_path == path) =>
            {
                cost
            }
            _ => 0,
        }
    }
}
pub(crate) struct Reuse {
    /// (device, inode) -> (seed node id, has directories); see `Priorities` for why not a path.
    listings: HashMap<(u64, u64), (u32, bool)>,
    seed: Arc<Snapshot>,
    pub(crate) priorities: Arc<Priorities>,
    stats: ReuseStats,
    reused_directories: u64,
    pub(crate) new_hardlink: bool,
    /// A folder served from cache no longer exists: the journal missed an event.
    pub(crate) stale: bool,
    known_hardlinks: HashSet<(u64, u64)>,
    root_listing: Vec<u32>,
    root_stats: ScanStats,
    root_own_sizes: (u64, u64),
    unchanged_possible: bool,
}
impl Reuse {
    fn new(seed: IncrementalSeed) -> Self {
        let unchanged_possible = seed.dirty_paths.is_empty()
            && seed.dirty_directories.is_empty()
            && seed
                .snapshot
                .iter_nodes()
                .all(|n| n.display_name() == n.native_name().to_string_lossy());
        let root_own_sizes = seed
            .snapshot
            .node(0)
            .map(|n| (n.length, n.own_allocated_size))
            .unwrap_or_default();
        let root_stats = seed.snapshot.stats.clone();
        let root_listing = seed.snapshot.children(0).unwrap_or_default().to_vec();
        // A changed alias (or an invalidated ancestor) invalidates every cached
        // alias of that inode. Listings only retain own sizes; aggregation then
        // elects the current lexicographic allocation owner from the new tree.
        let known_hardlinks: HashSet<_> = seed
            .snapshot
            .iter_nodes()
            .filter(|n| n.hard_link)
            .map(|n| (n.identity.device, n.identity.inode))
            .collect();
        #[derive(Default)]
        struct DirtyMark {
            subtree: bool,
            listing: bool,
            direct_listing: bool,
            requested: bool,
            node: Option<u32>,
        }
        // Keep Path's normalized equality for journal events, but translate all
        // event categories in one lookup per node. Unknown requested paths still
        // trigger the conservative all-known-hardlink-groups refresh.
        let mut marks: HashMap<PathBuf, DirtyMark> = HashMap::new();
        marks.entry(seed.snapshot.root.clone()).or_default().listing = true;
        for (paths, direct_listing) in [(&seed.dirty_paths, false), (&seed.dirty_directories, true)]
        {
            for path in paths {
                let mark = marks.entry(path.clone()).or_default();
                mark.requested = true;
                if direct_listing {
                    mark.direct_listing = true;
                } else {
                    mark.subtree = true;
                }
                for ancestor in path.ancestors() {
                    if !ancestor.starts_with(&seed.snapshot.root) {
                        break;
                    }
                    let mark = marks.entry(ancestor.to_path_buf()).or_default();
                    if mark.listing {
                        break;
                    }
                    mark.listing = true;
                }
            }
        }
        // valid_seed establishes dense ids and parent-before-child topology.
        // Propagate subtree state once, instead of hashing every path ancestor
        // for every cached node. Direct listing invalidation touches children,
        // but must not invalidate all otherwise clean descendants.
        let mut subtree_dirty = vec![false; seed.snapshot.node_count()];
        let mut listing_dirty = vec![false; seed.snapshot.node_count()];
        let mut direct_listing_dirty = vec![false; seed.snapshot.node_count()];
        let mut touched = HashSet::new();
        let mut path_cursor = seed.snapshot.path_cursor();
        for node in seed.snapshot.iter_nodes() {
            let path = path_cursor.path(node.id).expect("validated seed path");
            let mark = marks.get_mut(path);
            let index = node.id as usize;
            subtree_dirty[index] = node.parent.is_some_and(|p| subtree_dirty[p as usize])
                || mark.as_ref().is_some_and(|m| m.subtree);
            listing_dirty[index] = node.is_directory && mark.as_ref().is_some_and(|m| m.listing);
            direct_listing_dirty[index] =
                node.is_directory && mark.as_ref().is_some_and(|m| m.direct_listing);
            if let Some(mark) = mark {
                mark.node = Some(node.id);
            }
            // Only known hard-link inodes are ever looked up in `touched`.
            if (subtree_dirty[index]
                || node
                    .parent
                    .is_some_and(|p| direct_listing_dirty[p as usize]))
                && known_hardlinks.contains(&(node.identity.device, node.identity.inode))
            {
                touched.insert((node.identity.device, node.identity.inode));
            }
        }
        let mut ambiguous_alias_history =
            !seed.aliases_resolved && marks.values().any(|m| m.requested && m.node.is_none());
        // An event path with an unmatched parent, or an unmatched last name under a cached parent, is either new
        // or a cached entry spelled differently (case-insensitive or Unicode-equivalent names), whose cached
        // listing would hide the change. C is the first unmatched component below the nearest cached ancestor
        // M. A new directory has no cached listing and M's is dirty already. A child of M that C matches
        // case-insensitively, or as an NFC/NFD twin, is dirtied whole. The loop below spreads either to
        // descendants, whose hard links `touched` never sees, so dirtying any child makes every cached
        // alias dirty, resolved aliases or not.
        let nfd = icu_normalizer::DecomposingNormalizerBorrowed::new_nfd();
        let fold = |name: &std::ffi::OsStr| match name.to_str() {
            Some(name) if name.is_ascii() => name.to_ascii_lowercase().into_bytes(),
            // Per character, without context: `str::to_lowercase` turns a final Σ into ς but leaves a typed σ,
            // and keeps ϐ apart from β, while APFS folds each pair together. Over-matching only dirties more.
            Some(name) => nfd
                .normalize(name)
                .chars()
                .flat_map(char::to_uppercase)
                .flat_map(char::to_lowercase)
                // Again, so ẞ (lowercased to ß) folds like ß: ß → SS → ss.
                .flat_map(char::to_uppercase)
                .flat_map(char::to_lowercase)
                .collect::<String>()
                .into_bytes(),
            None => name.as_encoded_bytes().to_ascii_lowercase(),
        };
        let mut children_by_fold: HashMap<u32, HashMap<Vec<u8>, Vec<u32>>> = HashMap::new();
        for (path, mark) in marks.iter().filter(|(_, m)| m.requested) {
            let parent = path
                .parent()
                .and_then(|parent| marks.get(parent))
                .map(|m| m.node);
            let parent_unmatched = parent == Some(None);
            // Cached parent, unmatched last component: C is that component, matchable only by a case-insensitive or NFC/NFD twin.
            let last_unmatched = mark.node.is_none() && parent.is_some_and(|node| node.is_some());
            let cached = path
                .ancestors()
                .skip(1)
                .find_map(|a| Some((a, marks.get(a)?.node?)));
            let Some((ancestor, id)) = cached.filter(|_| parent_unmatched || last_unmatched) else {
                continue;
            };
            let widen = [id];
            let dirty: &[u32] = match path.strip_prefix(ancestor).unwrap().components().next() {
                Some(std::path::Component::Normal(name)) => {
                    let index = children_by_fold.entry(id).or_insert_with(|| {
                        let mut index: HashMap<_, Vec<u32>> = HashMap::new();
                        for child in seed.snapshot.node(id).map_or(&[][..], |n| n.children()) {
                            if let Some(child_node) = seed.snapshot.node(*child) {
                                index
                                    .entry(fold(child_node.native_name()))
                                    .or_default()
                                    .push(*child);
                            }
                        }
                        index
                    });
                    match index.get(&fold(name)) {
                        Some(variants) => variants,
                        None => &[],
                    }
                }
                _ => &widen,
            };
            for &node in dirty {
                subtree_dirty[node as usize] = true;
                ambiguous_alias_history = true;
            }
        }
        drop(marks);
        for node in seed.snapshot.iter_nodes() {
            let index = node.id as usize;
            let inode = (node.identity.device, node.identity.inode);
            subtree_dirty[index] |= node.parent.is_some_and(|p| subtree_dirty[p as usize])
                || (known_hardlinks.contains(&inode)
                    && (ambiguous_alias_history || touched.contains(&inode)));
            listing_dirty[index] |= subtree_dirty[index];
        }
        // Alias invalidation also dirties ancestor listings. Reverse topology
        // propagation avoids cloning and hashing each alias's ancestor paths.
        for node in seed.snapshot.iter_nodes().rev() {
            if listing_dirty[node.id as usize] {
                if let Some(parent) = node.parent {
                    listing_dirty[parent as usize] = true;
                }
            }
        }
        // Scheduling hints describe only validated dirty listings and their
        // ancestors. They never grant reuse or replace a fresh identity check.
        let mut costs = vec![0u64; seed.snapshot.node_count()];
        for node in seed.snapshot.iter_nodes().rev() {
            let index = node.id as usize;
            if node.is_directory && listing_dirty[index] {
                costs[index] = costs[index].saturating_add(node.children().len() as u64 + 1);
            }
            if let Some(parent) = node.parent {
                costs[parent as usize] = costs[parent as usize].saturating_add(costs[index]);
            }
        }
        // A directory hard link gives one inode several nodes, but the scan lists
        // only the first to commit, so a rescan can flip which copy has the
        // children. Neither copy's cached listing may be reused.
        let mut seen_directories = HashSet::new();
        let aliased_directories: HashSet<_> = seed
            .snapshot
            .iter_nodes()
            .filter(|n| n.is_directory)
            .map(|n| (n.identity.device, n.identity.inode))
            .filter(|inode| !seen_directories.insert(*inode))
            .collect();
        let priorities = Arc::new(Priorities {
            seed: seed.snapshot.clone(),
            costs: seed
                .snapshot
                .iter_nodes()
                .filter(|node| {
                    node.is_directory
                        && costs[node.id as usize] > 0
                        && !aliased_directories
                            .contains(&(node.identity.device, node.identity.inode))
                })
                .map(|node| {
                    (
                        (node.identity.device, node.identity.inode),
                        (node.id, costs[node.id as usize]),
                    )
                })
                .collect(),
        });
        drop(costs);
        let listings: HashMap<(u64, u64), (u32, bool)> = seed
            .snapshot
            .iter_nodes()
            .filter(|n| {
                n.is_directory
                    && !listing_dirty[n.id as usize]
                    && !aliased_directories.contains(&(n.identity.device, n.identity.inode))
            })
            .map(|n| {
                (
                    (n.identity.device, n.identity.inode),
                    (
                        n.id,
                        n.children()
                            .iter()
                            .any(|id| seed.snapshot.node(*id).is_some_and(|n| n.is_directory)),
                    ),
                )
            })
            .collect();
        let seed = seed.snapshot;
        Self {
            priorities,
            listings,
            seed,
            stats: ReuseStats::default(),
            reused_directories: 0,
            new_hardlink: false,
            stale: false,
            known_hardlinks,
            root_listing,
            root_stats,
            root_own_sizes,
            unchanged_possible,
        }
    }
    pub(crate) fn observe_entry(&mut self, entry: &Entry) {
        if entry.hard_link && !self.known_hardlinks.contains(&(entry.device, entry.inode)) {
            self.new_hardlink = true;
        }
    }
    pub(crate) fn observe_root(&mut self, listing: &std::io::Result<Vec<std::io::Result<Entry>>>) {
        if !self.unchanged_possible {
            return;
        }
        let index: HashMap<_, _> = self
            .root_listing
            .iter()
            .map(|id| (self.seed.native_name(*id).unwrap(), *id))
            .collect();
        self.unchanged_possible = listing.as_ref().is_ok_and(|entries| {
            let mut seen = HashSet::new();
            let unchanged = entries.len() == self.root_listing.len()
                && index.len() == self.root_listing.len()
                && entries.iter().all(|e| {
                    e.as_ref().is_ok_and(|e| {
                        e.path.parent() == Some(self.seed.root())
                            && seen.insert(e.path.as_ref())
                            && e.path
                                .file_name()
                                .and_then(|name| index.get(name))
                                .is_some_and(|id| {
                                    let n = self.seed.node(*id).unwrap();
                                    n.identity == e.identity()
                                        && n.length == e.length
                                        && n.own_allocated_size == e.allocated
                                        && n.hard_link == e.hard_link
                                })
                    })
                });
            unchanged
        });
    }
    /// Removes the candidate under the shared lock; it must pass `CachedListing::verified` before use.
    /// A candidate that fails stays removed, as before: the directory is simply read fresh.
    pub(crate) fn take_listing(&mut self, expected: &Identity) -> Option<CachedListing> {
        let (id, has_directories) = self.listings.remove(&(expected.device, expected.inode))?;
        Some(CachedListing {
            seed: self.seed.clone(),
            id,
            has_directories,
        })
    }
    #[cfg(test)]
    fn take_checked(&mut self, path: &Path, expected: &Identity) -> Option<CachedListing> {
        self.take_listing(expected)?.verified(path, expected)
    }
    pub(crate) fn mark_reused(&mut self, entries: usize) {
        self.reused_directories += 1;
        self.stats.reused_entries += entries as u64;
    }
}
fn valid_seed(snapshot: &Snapshot) -> bool {
    if snapshot.node_count() >= MAX_NODES || snapshot.stats.entries != snapshot.node_count() as u64
    {
        return false;
    }
    let mut links = vec![false; snapshot.node_count()];
    // Sibling names are checked one folder at a time, so this set is bounded
    // by the widest folder, not by the node count.
    let mut names = HashSet::new();
    let ordinary_paths = cfg!(unix) && !snapshot.has_path_overrides();
    let mut path_cursor = snapshot.path_cursor();
    let child_name = |id| {
        if ordinary_paths {
            snapshot.native_component(id)
        } else {
            snapshot.native_name(id)
        }
    };
    for (index, n) in snapshot.iter_nodes().enumerate() {
        if n.is_directory != (n.identity.kind == NodeKind::Directory) {
            return false;
        }
        if index == 0 {
            if n.parent.is_some() {
                return false;
            }
            // With no override the root's effective native path is snapshot.root;
            // its original stored display basename is intentionally irrelevant.
            if !ordinary_paths {
                let Ok(path) = path_cursor.path(n.id) else {
                    return false;
                };
                if path != snapshot.root {
                    return false;
                }
            }
        } else {
            let Some(parent) = n.parent.filter(|p| (*p as usize) < index) else {
                return false;
            };
            let Ok(name) = child_name(n.id) else {
                return false;
            };
            if !snapshot.node(parent).unwrap().is_directory {
                return false;
            }
            if !ordinary_paths {
                let Ok((path, parent_path)) = path_cursor.paths(n.id) else {
                    return false;
                };
                if path.file_name() != Some(name) || path.parent() != parent_path {
                    return false;
                }
            }
            // Ordinary children are constructed by joining one exact Normal
            // component to their already checked parent. Parent-before-child
            // induction proves these same file_name and path-parent relations.
        }
        let Ok(children) = snapshot.children(n.id) else {
            return false;
        };
        names.clear();
        for &child in children {
            if child as usize <= index
                || child as usize >= snapshot.node_count()
                || links[child as usize]
                || snapshot.node(child).unwrap().parent != Some(n.id)
            {
                return false;
            }
            links[child as usize] = true;
            let Ok(name) = child_name(child) else {
                return false;
            };
            if !names.insert(name) {
                return false;
            }
        }
    }
    links.iter().skip(1).all(|linked| *linked)
}
const MIN_VALIDATION_LISTINGS: usize = 4096;

fn validation_worker_count(base: usize, requested: usize, eligible_listings: usize) -> usize {
    let base = base.clamp(1, MAX_WORKERS);
    if cfg!(unix) && eligible_listings >= MIN_VALIDATION_LISTINGS {
        base.max(requested.clamp(1, MAX_WORKERS))
    } else {
        base
    }
}

/// Reuse clean metadata only after opening each directory with current held-descriptor authority.
/// A newly observed hard-link inode discards the candidate and retries against the same held root.
pub fn scan_live_incremental_authorized(
    root: ScanRoot,
    options: ScanOptions,
    cancel: Arc<AtomicBool>,
    store: Arc<RwLock<Snapshot>>,
    progress: impl Fn(Progress) + Send + Sync,
    seed: IncrementalSeed,
) -> Result<ReuseStats, ScanError> {
    scan_live_incremental_authorized_with_validation_workers(
        root,
        options,
        cancel,
        store,
        progress,
        seed,
        options.workers,
        |_| {},
    )
}

/// Allow a bounded validation pool for large eligible cached directory sets.
/// Reports the actual pool before each attempt, including any full-scan retry.
#[allow(clippy::too_many_arguments)]
pub fn scan_live_incremental_authorized_with_validation_workers(
    root: ScanRoot,
    options: ScanOptions,
    cancel: Arc<AtomicBool>,
    store: Arc<RwLock<Snapshot>>,
    progress: impl Fn(Progress) + Send + Sync,
    seed: IncrementalSeed,
    validation_workers: usize,
    on_workers: impl Fn(usize) + Send + Sync,
) -> Result<ReuseStats, ScanError> {
    let base_workers = options.workers.clamp(1, MAX_WORKERS);
    let fallback = if !seed.snapshot.incremental_accounting_eligible() {
        Some("cached accounting is shared or unresolved")
    } else if !valid_seed(&seed.snapshot)
        || seed
            .dirty_paths
            .iter()
            .chain(&seed.dirty_directories)
            .any(|p| !p.starts_with(&seed.snapshot.root))
    {
        Some("cached topology or event scope is invalid")
    } else if seed.snapshot.root != root.path
        || seed
            .snapshot
            .node(0)
            .is_none_or(|n| n.identity != root.expected)
    {
        Some("cached root identity changed")
    } else if root.path == Path::new("/") {
        Some("startup disk requires a full scan")
    } else if seed.snapshot.state != ScanState::Completed
        || seed.snapshot.stats.incomplete
        || seed.snapshot.stats.errors != 0
        || seed.snapshot.stats.excluded != 0
        || !seed.snapshot.errors.is_empty()
        || !seed.snapshot.exclusions.is_empty()
        || seed.snapshot.iter_nodes().any(|n| n.excluded)
    {
        Some("cached scan has incomplete coverage or exclusions")
    } else {
        None
    };
    if let Some(reason) = fallback {
        drop(seed);
        on_workers(base_workers);
        scan_live_authorized(root, options, cancel, store.clone(), progress)?;
        return Ok(ReuseStats {
            rescanned_directories: store.read().unwrap().worker_directories(),
            fallback_reason: Some(reason.into()),
            ..Default::default()
        });
    }
    let retry_root = root.reopen()?;
    let root_path = root.path.clone();
    let candidate = Arc::new(RwLock::new(Snapshot::empty(root_path.clone())));
    *store.write().unwrap() = Snapshot::empty(root_path.clone());
    // Candidate topology remains private, including the Completed callback of an
    // attempt that may be discarded. Only final publication exposes stable IDs.
    let report = |event: Progress| {
        let stats = candidate.read().unwrap().stats.clone();
        store.write().unwrap().stats = stats;
        progress(event);
    };
    let reuse = Reuse::new(seed);
    let selected_workers =
        validation_worker_count(base_workers, validation_workers, reuse.listings.len());
    let validation_options = ScanOptions {
        workers: selected_workers,
        ..options
    };
    let reuse = Mutex::new(reuse);
    on_workers(selected_workers);
    let mut outcome = scan_live_internal_reuse(
        root,
        validation_options,
        cancel.clone(),
        candidate.clone(),
        report,
        Some(&reuse),
    );
    let mut reuse = reuse.into_inner().unwrap();
    let stats = if outcome.is_ok()
        && (reuse.new_hardlink
            || reuse.stale
            || !candidate.read().unwrap().incremental_accounting_eligible())
        && !cancel.load(Ordering::Relaxed)
    {
        let reason = if reuse.new_hardlink {
            "new hard link requires global allocation reconciliation"
        } else if reuse.stale {
            "a reused folder changed without a journal event"
        } else {
            "sharing or unresolved accounting requires a full scan"
        };
        drop(reuse);
        on_workers(base_workers);
        outcome = scan_live_authorized(retry_root, options, cancel, candidate.clone(), report);
        ReuseStats {
            rescanned_directories: candidate.read().unwrap().worker_directories(),
            fallback_reason: Some(reason.into()),
            ..Default::default()
        }
    } else {
        let final_stats = candidate.read().unwrap();
        reuse.stats.unchanged_metadata = outcome.is_ok()
            && !cancel.load(Ordering::Relaxed)
            && final_stats.state == ScanState::Completed
            && !final_stats.stats.incomplete
            && final_stats.stats.errors == 0
            && final_stats.stats.excluded == 0
            && final_stats.incremental_accounting_eligible()
            && reuse.unchanged_possible
            && (
                final_stats.node(0).unwrap().length,
                final_stats.node(0).unwrap().own_allocated_size,
            ) == reuse.root_own_sizes
            && reuse.reused_directories + 1 == reuse.root_stats.directories
            && final_stats.stats.entries == reuse.root_stats.entries
            && final_stats.stats.directories == reuse.root_stats.directories
            && final_stats.stats.allocated_bytes == reuse.root_stats.allocated_bytes;
        drop(final_stats);
        reuse.stats.rescanned_directories = candidate
            .read()
            .unwrap()
            .worker_directories()
            .saturating_sub(reuse.reused_directories);
        reuse.stats
    };
    let mut final_snapshot = Arc::try_unwrap(candidate)
        .expect("scanner retained candidate store")
        .into_inner()
        .unwrap();
    if let Err(error) = &outcome {
        final_snapshot.state = ScanState::Failed;
        issue(&mut final_snapshot, root_path, error);
    }
    let event = Progress::of(&final_snapshot.stats);
    *store.write().unwrap() = final_snapshot;
    progress(event);
    outcome.map(|()| stats)
}

impl CachedListing {
    /// The seed path walk is allocating, so it runs here, after the `Reuse` lock is released.
    pub(crate) fn verified(self, path: &Path, expected: &Identity) -> Option<Self> {
        (self.seed.node(self.id)?.identity == *expected && self.seed.path(self.id).ok()? == path)
            .then_some(self)
    }
    /// Entries carry bare basenames like a fresh bulk listing; the worker joins the listing directory.
    pub(crate) fn materialize(self) -> Vec<std::io::Result<Entry>> {
        self.seed
            .children(self.id)
            .unwrap()
            .iter()
            .map(|id| {
                let n = self.seed.node(*id).unwrap();
                self.seed
                    .native_name(*id)
                    .map(|name| Entry {
                        clone_checked: false,
                        path: EntryPath::from_native(name),
                        device: n.identity.device,
                        inode: n.identity.inode,
                        kind: n.identity.kind,
                        length: n.length,
                        allocated: n.own_allocated_size,
                        hard_link: n.hard_link,
                    })
                    .map_err(std::io::Error::other)
            })
            .collect()
    }
}
#[cfg(all(test, unix))]
mod tests {
    fn edit_nodes(snapshot: &mut Snapshot, edit: impl FnOnce(&mut Vec<Node>, PathBuf)) {
        let mut nodes: Vec<_> = (0..snapshot.node_count())
            .map(|id| snapshot.legacy_node(id as u32).unwrap())
            .collect();
        edit(&mut nodes, snapshot.root.clone());
        snapshot.tree = crate::tree::Tree::default();
        for n in nodes {
            snapshot.append_legacy_node(n).unwrap();
        }
    }

    use super::*;
    use std::os::unix::fs::symlink;
    fn listing_paths(reuse: &Reuse) -> HashMap<PathBuf, u32> {
        let path = |id: &u32| reuse.seed.path(*id).unwrap();
        reuse
            .listings
            .values()
            .map(|(id, _)| (path(id), *id))
            .collect()
    }
    fn priority_costs(reuse: &Reuse) -> HashMap<PathBuf, u64> {
        let path = |id: &u32| reuse.seed.path(*id).unwrap();
        reuse
            .priorities
            .costs
            .values()
            .map(|(id, cost)| (path(id), *cost))
            .collect()
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "disk-incremental-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("clean/deep")).unwrap();
            fs::create_dir_all(path.join("dirty/nested")).unwrap();
            fs::write(path.join("clean/deep/kept"), b"unchanged").unwrap();
            fs::write(path.join("dirty/nested/grow"), b"small").unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
        fn full(&self) -> Snapshot {
            scan(
                self.0.clone(),
                ScanOptions::default(),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap()
        }
        fn incremental(&self, seed: Snapshot, dirty_paths: Vec<PathBuf>) -> (Snapshot, ReuseStats) {
            self.incremental_with_listings(seed, dirty_paths, vec![])
        }
        fn incremental_with_listings(
            &self,
            seed: Snapshot,
            dirty_paths: Vec<PathBuf>,
            dirty_directories: Vec<PathBuf>,
        ) -> (Snapshot, ReuseStats) {
            self.incremental_resolving(seed, dirty_paths, dirty_directories, false)
        }
        fn incremental_resolving(
            &self,
            seed: Snapshot,
            dirty_paths: Vec<PathBuf>,
            dirty_directories: Vec<PathBuf>,
            aliases_resolved: bool,
        ) -> (Snapshot, ReuseStats) {
            let authority =
                ScanRoot::open(self.0.clone(), platform_identity(&self.0).unwrap()).unwrap();
            let store = Arc::new(RwLock::new(Snapshot::empty(self.0.clone())));
            let stats = scan_live_incremental_authorized(
                authority,
                ScanOptions::default(),
                Arc::new(AtomicBool::new(false)),
                store.clone(),
                |_| {},
                IncrementalSeed {
                    dirty_directories,
                    snapshot: Arc::new(seed),
                    dirty_paths,
                    aliases_resolved,
                },
            )
            .unwrap();
            (Arc::try_unwrap(store).unwrap().into_inner().unwrap(), stats)
        }
    }
    #[test]
    fn dirty_costs_reach_ancestors_without_prioritizing_clean_siblings() {
        let f = Fixture::new();
        let snapshot = f.full();
        let dirty = f.0.join("dirty/nested");
        let own_cost = snapshot
            .iter_nodes()
            .map(|n| snapshot.legacy_node(n.id).unwrap())
            .find(|n| n.path == dirty)
            .unwrap()
            .children
            .len() as u64
            + 1;
        let reuse = Reuse::new(IncrementalSeed {
            snapshot: Arc::new(snapshot),
            dirty_paths: vec![],
            dirty_directories: vec![dirty.clone()],
            aliases_resolved: false,
        });
        let costs = priority_costs(&reuse);
        let cost = |path: &Path| costs[path];
        assert_eq!(cost(&dirty), own_cost);
        assert!(cost(&f.0.join("dirty")) > own_cost);
        assert!(cost(&f.0) > cost(&f.0.join("dirty")));
        assert!(!costs.contains_key(&f.0.join("clean")));
        assert!(!costs.contains_key(&f.0.join("clean/deep")));
        let listings = listing_paths(&reuse);
        assert!(listings.contains_key(&f.0.join("clean/deep")));
        assert!(!listings.contains_key(&dirty));
        // A hint needs the identity and the seed path: the same inode elsewhere, or another inode, gets none.
        let identity = reuse
            .seed
            .node(listings[&f.0.join("clean/deep")])
            .unwrap()
            .identity;
        assert_eq!(reuse.priorities.cost(&f.0.join("clean/deep"), &identity), 0);
        let dirty_identity = platform_identity(&dirty).unwrap();
        assert_eq!(reuse.priorities.cost(&dirty, &dirty_identity), own_cost);
        assert_eq!(
            reuse.priorities.cost(&f.0.join("moved"), &dirty_identity),
            0
        );
        let mut replacement = dirty_identity.clone();
        replacement.inode += 1;
        assert_eq!(reuse.priorities.cost(&dirty, &replacement), 0);
    }
    #[test]
    fn event_path_with_an_unmatched_parent_dirties_only_what_it_may_spell() {
        let f = Fixture::new();
        for dir in ["build/out", "build/other", "build/caf\u{e9}"] {
            fs::create_dir_all(f.0.join(dir)).unwrap();
            fs::write(f.0.join(dir).join("kept"), b"cached").unwrap();
        }
        let snapshot = Arc::new(f.full());
        // Which of these keep a reusable cached listing: build, out, other, an NFC cafe, clean/deep.
        let reusable = |paths: &[&str], directories: &[&str]| {
            let join = |names: &[&str]| names.iter().map(|n| f.0.join(n)).collect();
            let reuse = Reuse::new(IncrementalSeed {
                snapshot: snapshot.clone(),
                dirty_paths: join(paths),
                dirty_directories: join(directories),
                aliases_resolved: false,
            });
            [
                "build",
                "build/out",
                "build/other",
                "build/caf\u{e9}",
                "clean/deep",
            ]
            .map(|d| listing_paths(&reuse).contains_key(&f.0.join(d)))
        };
        // Exact spelling, a new file, and a new directory with a file dirty only the listings above them.
        assert_eq!(
            reusable(&["build/out/kept"], &[]),
            [false, false, true, true, true]
        );
        assert_eq!(
            reusable(&["build/new.bin"], &[]),
            [false, true, true, true, true]
        );
        assert_eq!(
            reusable(&["build/new/sub/a"], &[]),
            [false, true, true, true, true]
        );
        // `OUT` has no node but matches `out` case-insensitively: only that child is dirtied whole.
        assert_eq!(
            reusable(&["build/OUT/new.bin"], &[]),
            [false, false, true, true, true]
        );
        assert_eq!(
            reusable(&[], &["build/OUT/deep/er"]),
            [false, false, true, true, true]
        );
        assert_eq!(
            reusable(&["build/CAF\u{c9}/new.bin"], &[]),
            [false, true, true, false, true]
        );
        // The same twin as the last component under a cached parent, as a path or a directory.
        assert_eq!(
            reusable(&["build/OUT"], &[]),
            [false, false, true, true, true]
        );
        assert_eq!(
            reusable(&[], &["build/OUT"]),
            [false, false, true, true, true]
        );
        assert_eq!(
            reusable(&["build/CAF\u{c9}"], &[]),
            [false, true, true, false, true]
        );
        // A new last component, even non-ASCII, matches no twin and dirties no sibling.
        assert_eq!(
            reusable(&["build/new"], &[]),
            [false, true, true, true, true]
        );
        assert_eq!(
            reusable(&["build/\u{65b0}"], &[]),
            [false, true, true, true, true]
        );
        assert_eq!(
            reusable(&["Build/out/new.bin"], &[]),
            [false, false, false, false, true]
        );
        // An NFD spelling of the cached NFC child dirties it; a non-ASCII name no child matches is new.
        assert_eq!(
            reusable(&["build/cafe\u{301}/new.bin"], &[]),
            [false, true, true, false, true]
        );
        assert_eq!(
            reusable(&["build/\u{65b0}/a"], &[]),
            [false, true, true, true, true]
        );
        // A new directory is walked fresh, never taken from a cached listing.
        fs::create_dir_all(f.0.join("build/new/sub")).unwrap();
        fs::write(f.0.join("build/new/sub/a"), b"fresh").unwrap();
        let (actual, stats) = f.incremental((*snapshot).clone(), vec![f.0.join("build/new/sub/a")]);
        assert_eq!(stats.fallback_reason, None);
        assert!(stats.reused_entries > 0);
        compare(&actual, &f.full());
    }
    #[test]
    fn an_nfc_event_dirties_only_the_nfd_cached_child_it_spells() {
        let f = Fixture::new();
        for dir in [
            "build/cafe\u{301}",
            "build/\u{65b0}",
            "build/\u{39f}\u{394}\u{39f}\u{3a3}",
            "build/beta_\u{3d0}",
            "build/stra\u{df}e",
        ] {
            fs::create_dir_all(f.0.join(dir)).unwrap();
            fs::write(f.0.join(dir).join("kept"), b"cached").unwrap();
        }
        // NFC for NFD, a typed σ for a final Σ, β for its variant ϐ, and ẞ for ß: each spells the cached folder.
        let reuse = Reuse::new(IncrementalSeed {
            snapshot: Arc::new(f.full()),
            dirty_paths: vec![
                f.0.join("build/caf\u{e9}/new.bin"),
                f.0.join("build/\u{3bf}\u{3b4}\u{3bf}\u{3c3}/new.bin"),
                f.0.join("build/beta_\u{3b2}/new.bin"),
                f.0.join("build/STRA\u{1e9e}E/new.bin"),
            ],
            dirty_directories: vec![],
            aliases_resolved: false,
        });
        let cached = listing_paths(&reuse);
        for dirty in [
            "build/cafe\u{301}",
            "build/\u{39f}\u{394}\u{39f}\u{3a3}",
            "build/beta_\u{3d0}",
            "build/stra\u{df}e",
        ] {
            assert!(!cached.contains_key(&f.0.join(dirty)), "{dirty}");
        }
        assert!(cached.contains_key(&f.0.join("build/\u{65b0}")));
    }
    #[test]
    fn many_events_under_one_new_non_ascii_folder_do_not_scan_the_siblings() {
        // A wide folder of non-ASCII children and an event burst under one new non-ASCII name.
        let f = Fixture::new();
        let mut snapshot = f.full();
        let wide = 8_000u32;
        edit_nodes(&mut snapshot, |nodes, root| {
            let template = nodes
                .iter()
                .find(|n| n.is_directory && n.id != 0)
                .unwrap()
                .clone();
            for i in 0..wide {
                let mut node = template.clone();
                node.id = nodes.len() as u32;
                node.parent = Some(0);
                node.children.clear();
                node.name = format!("caf\u{e9}{i}");
                node.path = root.join(&node.name);
                nodes[0].children.push(node.id);
                nodes.push(node);
            }
        });
        snapshot.stats.entries = snapshot.node_count() as u64;
        let snapshot = Arc::new(snapshot);
        let listings = |events: u32| {
            let started = std::time::Instant::now();
            let reuse = Reuse::new(IncrementalSeed {
                snapshot: snapshot.clone(),
                dirty_paths: (0..events)
                    .map(|i| f.0.join(format!("\u{65b0}/{i}")))
                    .collect(),
                dirty_directories: vec![],
                aliases_resolved: false,
            });
            let mut kept: Vec<_> = reuse.listings.values().map(|(id, _)| *id).collect();
            kept.sort_unstable();
            (kept, started.elapsed())
        };
        let (one, _) = listings(1);
        let (many, elapsed) = listings(wide);
        assert_eq!(many, one);
        // Each event is one indexed lookup; a per-event sibling scan would take seconds.
        assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn compare(a: &Snapshot, b: &Snapshot) {
        let normalize = |s: &Snapshot| {
            let mut nodes: Vec<_> = s
                .iter_nodes()
                .map(|n| s.legacy_node(n.id).unwrap())
                .map(|n| {
                    (
                        n.path.clone(),
                        format!("{:?}", n.identity),
                        n.length,
                        n.own_allocated_size,
                        n.allocated_size,
                        n.hard_link,
                        n.excluded,
                    )
                })
                .collect();
            nodes.sort();
            nodes
        };
        assert_eq!(normalize(a), normalize(b));
        assert_eq!(a.stats.entries, b.stats.entries);
        assert_eq!(a.stats.directories, b.stats.directories);
        assert_eq!(a.stats.allocated_bytes, b.stats.allocated_bytes);
        assert_eq!(a.stats.errors, b.stats.errors);
        assert_eq!(a.state, b.state);
    }
    #[test]
    fn validation_pool_bounds_and_ineligible_plans() {
        assert_eq!(validation_worker_count(3, 4, 4096), 4);
        assert_eq!(validation_worker_count(3, usize::MAX, 4096), MAX_WORKERS);
        assert_eq!(validation_worker_count(8, 4, 4096), 8);
        assert_eq!(validation_worker_count(0, 0, 4096), 1);
        assert_eq!(validation_worker_count(usize::MAX, 4, 4096), MAX_WORKERS);
        assert_eq!(validation_worker_count(3, 4, 4095), 3);
        let f = Fixture::new();
        for (paths, directories) in [
            (vec![], vec![]),
            (vec![f.0.clone()], vec![]),
            (vec![], vec![f.0.clone()]),
            (vec![f.0.join("unknown")], vec![]),
        ] {
            let reuse = Reuse::new(IncrementalSeed {
                snapshot: Arc::new(f.full()),
                dirty_paths: paths,
                dirty_directories: directories,
                aliases_resolved: false,
            });
            assert_eq!(validation_worker_count(3, 4, reuse.listings.len()), 3);
        }
    }

    #[test]
    fn large_validation_pool_preserves_opens_parity_and_retry_worker_count() {
        let f = Fixture::new();
        for index in 0..MIN_VALIDATION_LISTINGS {
            let directory = f.0.join(format!("validation-{index}"));
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("file"), b"cached").unwrap();
        }
        let seed = f.full();
        // Successful reuse, fully dirty plan, invalid seed, and a new alias retry.
        for variant in 0..4 {
            let mut snapshot = seed.clone();
            let dirty_paths = match variant {
                1 => vec![f.0.clone()],
                2 => {
                    snapshot.stats.incomplete = true;
                    vec![]
                }
                3 => {
                    fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("dirty/new-alias"))
                        .unwrap();
                    vec![f.0.join("dirty/new-alias")]
                }
                _ => vec![],
            };
            let store = Arc::new(RwLock::new(Snapshot::empty(f.0.clone())));
            let workers = Mutex::new(Vec::new());
            let stats = scan_live_incremental_authorized_with_validation_workers(
                ScanRoot::open(f.0.clone(), platform_identity(&f.0).unwrap()).unwrap(),
                ScanOptions {
                    workers: 3,
                    ..ScanOptions::default()
                },
                Arc::new(AtomicBool::new(false)),
                store.clone(),
                |_| {},
                IncrementalSeed {
                    snapshot: Arc::new(snapshot),
                    dirty_paths,
                    dirty_directories: vec![],
                    aliases_resolved: false,
                },
                4,
                |count| workers.lock().unwrap().push(count),
            )
            .unwrap();
            let actual = store.read().unwrap();
            compare(&actual, &f.full());
            assert!(!actual.stats.incomplete);
            assert_eq!(actual.stats.errors, 0);
            assert_eq!(
                workers.into_inner().unwrap(),
                match variant {
                    0 => vec![4],
                    3 => vec![4, 3],
                    _ => vec![3],
                }
            );
            assert_eq!(actual.worker_stats.len(), if variant == 0 { 4 } else { 3 });
            let opens = actual
                .worker_stats
                .iter()
                .map(|worker| worker.relative_opens + worker.absolute_reopens)
                .sum::<u64>();
            assert_eq!(opens, actual.stats.directories - 1);
            if variant == 0 {
                assert!(stats.reused_entries >= MIN_VALIDATION_LISTINGS as u64);
                assert!(stats.unchanged_metadata);
                assert_eq!(stats.rescanned_directories, 1);
            } else {
                assert_eq!(stats.reused_entries, 0);
                assert!(!stats.unchanged_metadata);
                assert_eq!(stats.fallback_reason.is_some(), variant != 1);
            }
        }
    }

    fn valid_seed_path_oracle(snapshot: &Snapshot) -> bool {
        let nodes: Vec<_> = (0..snapshot.node_count())
            .map(|id| snapshot.legacy_node(id as u32))
            .collect::<Result<_, _>>()
            .unwrap_or_default();
        if nodes.len() >= MAX_NODES || snapshot.stats.entries != nodes.len() as u64 {
            return false;
        }
        let mut links = vec![false; nodes.len()];
        let mut paths = HashSet::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            if !paths.insert(node.path.as_path())
                || node.id as usize != index
                || node.is_directory != (node.identity.kind == NodeKind::Directory)
            {
                return false;
            }
            if index == 0 {
                if node.parent.is_some() || node.path != snapshot.root {
                    return false;
                }
            } else {
                let Some(parent) = node.parent.map(|p| p as usize).filter(|p| *p < index) else {
                    return false;
                };
                if node.path.file_name().is_none()
                    || !nodes[parent].is_directory
                    || node.path.parent() != Some(nodes[parent].path.as_path())
                {
                    return false;
                }
            }
            for &child in &node.children {
                let child = child as usize;
                if child <= index
                    || child >= nodes.len()
                    || links[child]
                    || nodes[child].parent != Some(node.id)
                {
                    return false;
                }
                links[child] = true;
            }
        }
        links.iter().skip(1).all(|linked| *linked)
    }

    #[test]
    fn compact_component_proof_matches_independent_full_path_oracle() {
        use std::os::unix::ffi::OsStringExt;
        fn root() -> Node {
            Node {
                clone_metadata: None,
                id: 0,
                parent: None,
                children: vec![1, 2],
                name: "original label".into(),
                path: "/scope".into(),
                identity: Identity {
                    device: 1,
                    inode: 1,
                    kind: NodeKind::Directory,
                },
                allocated_size: 0,
                length: 0,
                own_allocated_size: 0,
                is_directory: true,
                hard_link: false,
                excluded: false,
            }
        }
        fn fixture(name: &std::ffi::OsStr) -> Snapshot {
            let mut s = Snapshot::empty("/scope".into());
            s.append_legacy_node(root()).unwrap();
            for (id, name) in [(1, name), (2, std::ffi::OsStr::new("second"))] {
                let mut n = root();
                n.id = id;
                n.parent = Some(0);
                n.children.clear();
                n.path = Path::new("/scope").join(name);
                n.name = name.to_string_lossy().into_owned();
                n.is_directory = false;
                n.identity.kind = NodeKind::File;
                n.identity.inode = id as u64 + 1;
                s.append_legacy_node(n).unwrap();
            }
            s.stats.entries = 3;
            s
        }
        let nul = std::ffi::OsString::from_vec(b"native\0name".to_vec());
        for name in [std::ffi::OsStr::new("ordinary"), nul.as_os_str()] {
            let original = fixture(name);
            assert!(!original.has_path_overrides());
            assert!(valid_seed_path_oracle(&original));
            assert!(valid_seed(&original));
            for spelling in ["/changed", "relative/root", "//double//root//", ""] {
                let mut s = original.clone();
                s.root = spelling.into();
                assert_eq!(
                    valid_seed(&s),
                    valid_seed_path_oracle(&s),
                    "root spelling={spelling:?}"
                );
            }
            let mut s = original.clone();
            s.update_node(2, |n| n.parent = Some(1)).unwrap();
            assert_eq!(valid_seed(&s), valid_seed_path_oracle(&s));
            assert!(!valid_seed(&s));
            s.update_node(1, |n| {
                n.is_directory = true;
                n.identity.kind = NodeKind::Directory;
            })
            .unwrap();
            s.set_children(0, &[1]).unwrap();
            s.set_children(1, &[2]).unwrap();
            assert_eq!(valid_seed(&s), valid_seed_path_oracle(&s));
            assert!(valid_seed(&s));
            let mut s = original.clone();
            s.update_node(1, |n| n.parent = Some(2)).unwrap();
            assert!(!valid_seed(&s));
            assert_eq!(valid_seed(&s), valid_seed_path_oracle(&s));
            let mut s = original.clone();
            s.set_children(0, &[1, 1, 2]).unwrap();
            assert!(!valid_seed(&s));
            assert_eq!(valid_seed(&s), valid_seed_path_oracle(&s));
            let mut s = original.clone();
            let first = s.tree.records[1].clone();
            s.tree.records[2].name_offset = first.name_offset;
            s.tree.records[2].name_len = first.name_len;
            assert!(!valid_seed(&s));
            assert_eq!(valid_seed(&s), valid_seed_path_oracle(&s));
            let mut s = original;
            let mut bad = s.legacy_node(1).unwrap();
            bad.path = "/outside/raw".into();
            s.replace_node(1, bad).unwrap();
            assert!(s.has_path_overrides());
            assert!(!valid_seed(&s));
            assert_eq!(valid_seed(&s), valid_seed_path_oracle(&s));
        }
        // This supplied record is packed losslessly into a raw override and
        // retains the original validator's missing-file-name rejection.
        let mut malformed = root();
        malformed.id = 1;
        malformed.parent = Some(0);
        malformed.children.clear();
        malformed.path = "/scope/..".into();
        let mut builder = crate::LegacyTreeBuilder::new("/scope".into());
        let mut root = root();
        root.children = vec![1];
        builder.append(root).unwrap();
        builder.append(malformed.clone()).unwrap();
        let mut s = builder.finish();
        s.stats.entries = 2;
        assert!(s.has_path_overrides());
        assert_eq!(s.path(1).unwrap().as_os_str(), malformed.path.as_os_str());
        assert!(!valid_seed(&s));
    }
    #[test]
    fn sibling_uniqueness_matches_full_path_oracle() {
        use std::os::unix::ffi::OsStringExt;
        let f = Fixture::new();
        let original = f.full();
        assert!(valid_seed_path_oracle(&original));
        assert!(valid_seed(&original));
        for spelling in [
            b"raw\xff".to_vec(),
            b"raw\xfe".to_vec(),
            b"./raw//name".to_vec(),
        ] {
            let mut snapshot = original.clone();
            edit_nodes(&mut snapshot, |nodes, _root| {
                let index = nodes.iter().position(|n| !n.is_directory).unwrap();
                let parent = nodes[index].parent.unwrap() as usize;
                let name = std::ffi::OsString::from_vec(spelling);
                nodes[index].path = nodes[parent].path.join(name);
            });
            assert_eq!(valid_seed(&snapshot), valid_seed_path_oracle(&snapshot));
        }
        for directory in [false, true] {
            let mut snapshot = original.clone();
            edit_nodes(&mut snapshot, |nodes, _root| {
                let index = nodes
                    .iter()
                    .position(|n| n.parent == Some(0) && n.is_directory == directory)
                    .unwrap_or_else(|| {
                        nodes
                            .iter()
                            .position(|n| n.is_directory == directory && n.parent.is_some())
                            .unwrap()
                    });
                let mut duplicate = nodes[index].clone();
                duplicate.id = nodes.len() as u32;
                duplicate.children.clear();
                duplicate.path = duplicate
                    .path
                    .parent()
                    .unwrap()
                    .join(".")
                    .join(duplicate.path.file_name().unwrap());
                nodes[duplicate.parent.unwrap() as usize]
                    .children
                    .push(duplicate.id);
                nodes.push(duplicate);
            });
            snapshot.stats.entries += 1;
            assert!(!valid_seed_path_oracle(&snapshot));
            assert!(!valid_seed(&snapshot));
        }
        let mut snapshot = original;
        edit_nodes(&mut snapshot, |nodes, _root| {
            let index = nodes.iter().position(|n| !n.is_directory).unwrap();
            let parent = nodes[index].parent.unwrap() as usize;
            nodes[index].path = nodes[parent].path.join("..");
        });
        assert!(!valid_seed(&snapshot));
    }

    #[test]
    #[ignore = "opt-in release seed validation CPU microbenchmark"]
    fn seed_validation_cpu_benchmark() {
        let f = Fixture::new();
        let original = f.full();
        let file = original
            .iter_nodes()
            .map(|n| original.legacy_node(n.id).unwrap())
            .find(|n| !n.is_directory)
            .unwrap();
        let mut snapshot = original.clone();
        snapshot.root = PathBuf::from(
            "/Users/benchmark/Documents/GitHub/project/target/release/dependencies/cache/deep/tree",
        );
        edit_nodes(&mut snapshot, |nodes, _root| {
            *nodes = vec![original.legacy_node(0).unwrap().clone()];
            nodes[0].path = _root.clone();
            nodes[0].children.clear();
            let count = 1_000_000;
            for index in 1..=count {
                let mut node = file.clone();
                node.id = index;
                node.parent = Some(0);
                node.path = _root.join(format!("entry-{index}"));
                node.children.clear();
                nodes[0].children.push(index);
                nodes.push(node);
            }
        });
        snapshot.stats.entries = snapshot.node_count() as u64;
        for round in 0..6 {
            type ValidationCheck = (&'static str, fn(&Snapshot) -> bool);
            let checks: [ValidationCheck; 2] = if round % 2 == 0 {
                [("path", valid_seed_path_oracle), ("sibling", valid_seed)]
            } else {
                [("sibling", valid_seed), ("path", valid_seed_path_oracle)]
            };
            for (label, check) in checks {
                let start = std::time::Instant::now();
                assert!(check(std::hint::black_box(&snapshot)));
                eprintln!(
                    "validation round={round} strategy={label} elapsed_ms={:.3}",
                    start.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
    }

    #[test]
    fn dense_preparation_matches_conservative_path_eligibility() {
        let f = Fixture::new();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("dirty/alias")).unwrap();
        use std::os::unix::ffi::OsStringExt;
        let raw =
            f.0.join("clean/deep")
                .join(std::ffi::OsString::from_vec(b"raw\xff".to_vec()));
        let other_raw =
            f.0.join("dirty")
                .join(std::ffi::OsString::from_vec(b"raw\xfe".to_vec()));
        let mut snapshot = f.full();
        // APFS need not accept arbitrary bytes, but cached path data must keep
        // byte identity rather than lossy display names. Exercise it in memory.
        edit_nodes(&mut snapshot, |nodes, _root| {
            for node in nodes.iter_mut() {
                if node.path == f.0.join("clean/deep/kept") {
                    node.path = raw.clone();
                }
                if node.path == f.0.join("dirty/alias") {
                    node.path = other_raw.clone();
                }
            }
        });
        assert!(valid_seed(&snapshot));
        let mut cases: Vec<(Vec<PathBuf>, Vec<PathBuf>)> = [
            (vec![], vec![]),
            (vec!["dirty/nested/grow"], vec![]),
            (vec!["dirty"], vec![]),
            (vec![], vec!["dirty"]),
            (vec!["clean/deep/kept"], vec![]),
            (vec!["missing/transient"], vec![]),
            (vec![], vec!["missing/listing"]),
            (vec!["dirty/nested/grow", "clean/deep"], vec!["dirty"]),
            (vec![], vec!["clean/deep"]),
            (vec![], vec!["dirty/nested/grow"]),
            (vec![""], vec![]),
        ]
        .into_iter()
        .map(|(paths, directories)| {
            (
                paths.into_iter().map(|p| f.0.join(p)).collect(),
                directories.into_iter().map(|p| f.0.join(p)).collect(),
            )
        })
        .collect();
        cases.extend([
            (vec![raw], vec![]),
            (vec![f.0.join("clean/./deep//kept")], vec![]),
            (vec![], vec![f.0.join("dirty/./nested")]),
            (vec![f.0.join("dirty/nested/../alias")], vec![]),
            (vec![other_raw], vec![f.0.join("missing/../transient")]),
        ]);
        for (paths, directories) in cases {
            let paths: HashSet<_> = paths.into_iter().collect();
            let directories: HashSet<_> = directories.into_iter().collect();
            let known: HashSet<_> = snapshot
                .iter_nodes()
                .map(|n| snapshot.legacy_node(n.id).unwrap())
                .filter(|n| n.hard_link)
                .map(|n| (n.identity.device, n.identity.inode))
                .collect();
            let ambiguous = paths.iter().chain(&directories).any(|p| {
                !snapshot
                    .iter_nodes()
                    .map(|n| snapshot.legacy_node(n.id).unwrap())
                    .any(|n| n.path == *p)
            });
            let below =
                |path: &Path, dirty: &HashSet<PathBuf>| path.ancestors().any(|p| dirty.contains(p));
            let touched: HashSet<_> = snapshot
                .iter_nodes()
                .map(|n| snapshot.legacy_node(n.id).unwrap())
                .filter(|n| {
                    (ambiguous && n.hard_link)
                        || below(&n.path, &paths)
                        || n.path.parent().is_some_and(|p| directories.contains(p))
                })
                .map(|n| (n.identity.device, n.identity.inode))
                .collect();
            let mut expanded = paths.clone();
            expanded.extend(
                snapshot
                    .iter_nodes()
                    .map(|n| snapshot.legacy_node(n.id).unwrap())
                    .filter(|n| {
                        let inode = (n.identity.device, n.identity.inode);
                        known.contains(&inode) && touched.contains(&inode)
                    })
                    .map(|n| n.path.clone()),
            );
            let mut ancestors = HashSet::from([snapshot.root.clone()]);
            for path in expanded.iter().chain(&directories) {
                ancestors.extend(
                    path.ancestors()
                        .filter(|p| p.starts_with(&snapshot.root))
                        .map(Path::to_path_buf),
                );
            }
            let expected: HashSet<_> = snapshot
                .iter_nodes()
                .map(|n| snapshot.legacy_node(n.id).unwrap())
                .filter(|n| {
                    n.is_directory && !ancestors.contains(&n.path) && !below(&n.path, &expanded)
                })
                .map(|n| n.path.clone())
                .collect();
            let reuse = Reuse::new(IncrementalSeed {
                snapshot: Arc::new(snapshot.clone()),
                dirty_paths: paths.into_iter().collect(),
                dirty_directories: directories.into_iter().collect(),
                aliases_resolved: false,
            });
            let listings = listing_paths(&reuse);
            assert_eq!(listings.keys().cloned().collect::<HashSet<_>>(), expected);
            for (path, id) in &listings {
                let entries = CachedListing {
                    seed: reuse.seed.clone(),
                    id: *id,
                    has_directories: false,
                };
                let actual: Vec<_> = entries
                    .materialize()
                    .into_iter()
                    .map(|e| {
                        let e = e.unwrap();
                        assert_eq!(e.path.components().count(), 1, "bare basename");
                        path.join(&e.path)
                    })
                    .collect();
                let expected: Vec<_> = snapshot
                    .iter_nodes()
                    .map(|n| snapshot.legacy_node(n.id).unwrap())
                    .filter(|n| n.path.parent() == Some(path.as_path()))
                    .map(|n| n.path.clone())
                    .collect();
                assert_eq!(actual, expected);
            }
        }
    }
    #[test]
    fn unchanged_reuses_metadata_and_matches_full() {
        let f = Fixture::new();
        let seed = f.full();
        let (actual, stats) = f.incremental(seed, vec![]);
        compare(&actual, &f.full());
        assert!(stats.reused_entries > 0);
        assert_eq!(stats.rescanned_directories, 1);
        assert!(stats.unchanged_metadata);
    }
    #[test]
    fn reused_listings_with_long_names_match_a_fresh_scan() {
        // Names over 22 bytes leave inline storage; reused children must still rebuild exact paths.
        let f = Fixture::new();
        let long = "a-directory-name-longer-than-inline";
        let deeper = f.0.join("clean").join(long).join(format!("{long}-2"));
        fs::create_dir_all(&deeper).unwrap();
        fs::write(deeper.join(format!("{long}.txt")), b"long").unwrap();
        fs::write(f.0.join("clean/日本語-ファイル"), b"wide").unwrap();
        let seed = f.full();
        let (actual, stats) = f.incremental(seed, vec![]);
        compare(&actual, &f.full());
        assert!(actual.iter_nodes().any(|n| n.native_name().len() > 22));
        assert!(stats.unchanged_metadata);
        // Only the root is read (itself plus its two children): the rest, long names included, is reused.
        assert_eq!(stats.rescanned_directories, 1);
        assert_eq!(stats.reused_entries, actual.stats.entries - 3);
    }
    #[test]
    fn archive_retention_proof_rejects_duplicate_root_records_and_noncanonical_names() {
        let f = Fixture::new();
        let seed = f.full();
        let first = seed
            .iter_nodes()
            .map(|n| seed.legacy_node(n.id).unwrap())
            .find(|n| n.parent == Some(0))
            .unwrap();
        let entries = (0..seed.legacy_node(0).unwrap().children.len())
            .map(|_| {
                Ok(Entry {
                    clone_checked: false,
                    path: first.path.clone().into(),
                    device: first.identity.clone().device,
                    inode: first.identity.clone().inode,
                    kind: first.identity.clone().kind,
                    length: first.length,
                    allocated: first.own_allocated_size,
                    hard_link: first.hard_link,
                })
            })
            .collect();
        let mut reuse = Reuse::new(IncrementalSeed {
            snapshot: Arc::new(seed.clone()),
            dirty_paths: vec![],
            dirty_directories: vec![],
            aliases_resolved: false,
        });
        reuse.observe_root(&Ok(entries));
        assert!(!reuse.unchanged_possible);
        let mut seed = seed;
        let mut root = seed.legacy_node(0).unwrap();
        root.name = "different cached label".into();
        seed.replace_node(0, root).unwrap();
        let (actual, stats) = f.incremental(seed, vec![]);
        compare(&actual, &f.full());
        assert!(!stats.unchanged_metadata);
    }
    #[test]
    fn archive_retention_proof_rejects_new_root_entries_and_dirty_history() {
        let f = Fixture::new();
        let seed = f.full();
        fs::write(f.0.join("new-root-file"), b"new").unwrap();
        let (actual, stats) = f.incremental(seed, vec![]);
        compare(&actual, &f.full());
        assert!(!stats.unchanged_metadata);
        let (actual, stats) = f.incremental(actual, vec![f.0.join("clean/deep/kept")]);
        compare(&actual, &f.full());
        assert!(!stats.unchanged_metadata);
        let (actual, stats) = f.incremental_with_listings(actual, vec![], vec![f.0.join("clean")]);
        compare(&actual, &f.full());
        assert!(!stats.unchanged_metadata);
    }
    #[test]
    fn directory_listing_refresh_preserves_descendant_reuse_and_detects_replacements() {
        let f = Fixture::new();
        let seed = f.full();
        let (actual, stats) = f.incremental_with_listings(seed, vec![], vec![f.0.join("clean")]);
        compare(&actual, &f.full());
        assert!(stats.reused_entries > 0);
        // A new directory with the same pathname cannot reuse the old listing.
        let seed = actual;
        fs::rename(f.0.join("clean/deep"), f.0.join("moved")).unwrap();
        fs::create_dir(f.0.join("clean/deep")).unwrap();
        fs::write(f.0.join("clean/deep/new"), b"replacement").unwrap();
        let (actual, _) = f.incremental_with_listings(seed, vec![], vec![f.0.join("clean")]);
        compare(&actual, &f.full());
    }
    #[test]
    fn dirty_nested_add_delete_rename_grow_sparse_and_symlink_match_full() {
        let f = Fixture::new();
        let seed = f.full();
        let dir = f.0.join("dirty/nested");
        fs::rename(dir.join("grow"), dir.join("renamed")).unwrap();
        fs::write(dir.join("renamed"), vec![42; 12345]).unwrap();
        fs::write(dir.join("new"), b"new").unwrap();
        let sparse = fs::File::create(dir.join("sparse")).unwrap();
        sparse.set_len(1024 * 1024).unwrap();
        symlink(f.0.join("clean"), dir.join("link")).unwrap();
        let (actual, stats) = f.incremental(
            seed,
            vec![
                dir.join("grow"),
                dir.join("renamed"),
                dir.join("new"),
                dir.join("sparse"),
                dir.join("link"),
            ],
        );
        compare(&actual, &f.full());
        assert!(stats.reused_entries > 0);
        let seed = actual;
        fs::remove_file(dir.join("new")).unwrap();
        let (actual, _) = f.incremental(seed, vec![dir.join("new")]);
        compare(&actual, &f.full());
    }
    #[test]
    fn directory_event_invalidates_descendants() {
        let f = Fixture::new();
        let seed = f.full();
        fs::write(f.0.join("dirty/nested/grow"), vec![1; 45678]).unwrap();
        let (actual, stats) = f.incremental(seed, vec![f.0.join("dirty")]);
        compare(&actual, &f.full());
        assert!(stats.reused_entries > 0);
    }
    #[test]
    fn cached_listing_needs_its_inode_at_its_path() {
        let f = Fixture::new();
        let seed = Arc::new(f.full());
        let plan = || {
            Reuse::new(IncrementalSeed {
                snapshot: seed.clone(),
                dirty_paths: vec![],
                dirty_directories: vec![],
                aliases_resolved: false,
            })
        };
        let deep = f.0.join("clean/deep");
        let expected = platform_identity(&deep).unwrap();
        assert!(plan().take_checked(&deep, &expected).is_some());
        // The inode is indexed on its own: the seed path must still match, and so must the inode.
        assert!(plan()
            .take_checked(&f.0.join("clean/moved"), &expected)
            .is_none());
        let mut replacement = expected.clone();
        replacement.inode += 1;
        assert!(plan().take_checked(&deep, &replacement).is_none());
    }
    #[test]
    fn cached_leaf_replacement_and_symlink_fail_fresh_identity_without_reuse() {
        for link in [false, true] {
            let f = Fixture::new();
            let seed = f.full();
            let outside = Fixture::new();
            let original_leaf_entries = seed
                .iter_nodes()
                .map(|n| seed.legacy_node(n.id).unwrap())
                .filter(|n| n.path.parent() == Some(f.0.join("clean/deep").as_path()))
                .count() as u64;
            let mut staged = Reuse::new(IncrementalSeed {
                snapshot: Arc::new(seed.clone()),
                dirty_paths: vec![],
                dirty_directories: vec![],
                aliases_resolved: false,
            });
            let expected = platform_identity(&f.0.join("clean/deep")).unwrap();
            let leaf = staged
                .take_checked(&f.0.join("clean/deep"), &expected)
                .unwrap();
            assert!(!leaf.has_directories);
            assert_eq!(staged.stats.reused_entries, 0);
            fs::rename(f.0.join("clean/deep"), outside.0.join("original-deep")).unwrap();
            if link {
                symlink(outside.0.join("original-deep"), f.0.join("clean/deep")).unwrap();
            } else {
                fs::create_dir(f.0.join("clean/deep")).unwrap();
                fs::write(f.0.join("clean/deep/outside-marker"), b"x").unwrap();
            }
            let (actual, stats) = f.incremental(seed, vec![]);
            assert_eq!(actual.stats.errors, 0);
            assert!(!actual.stats.incomplete);
            // The old contents (moved out, or behind the symlink) never come back.
            assert!(!actual
                .iter_nodes()
                .map(|n| actual.legacy_node(n.id).unwrap())
                .any(|n| n.path == f.0.join("clean/deep/kept")));
            // The replaced cached folder is a missed event: a full retry reads what is there now.
            assert!(stats.fallback_reason.unwrap().contains("journal event"));
            assert_eq!(stats.reused_entries, 0);
            assert!(original_leaf_entries > 0);
            assert!(!stats.unchanged_metadata);
            compare(&actual, &f.full());
        }
    }
    #[test]
    fn cached_folder_deleted_without_an_event_retries_fully_without_a_phantom() {
        let f = Fixture::new();
        let seed = f.full();
        fs::remove_dir_all(f.0.join("clean/deep")).unwrap();
        let (actual, stats) = f.incremental(seed, vec![]);
        assert!(stats.fallback_reason.unwrap().contains("journal event"));
        assert_eq!(actual.stats.errors, 0);
        assert!(!actual.stats.incomplete);
        compare(&actual, &f.full());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn new_file_in_a_reused_folder_without_an_event_retries_fully() {
        let f = Fixture::new();
        let seed = f.full();
        fs::write(f.0.join("clean/deep/new"), b"new").unwrap();
        let (actual, stats) = f.incremental(seed, vec![]);
        assert!(stats.fallback_reason.unwrap().contains("unresolved"));
        compare(&actual, &f.full());
        assert!(actual.iter_nodes().any(|n| n.native_name() == "new"));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn new_file_in_a_reused_folder_holding_only_a_subfolder_retries_fully() {
        let f = Fixture::new();
        let seed = f.full();
        // `clean` saved no file, so nothing in it needed a clone probe.
        fs::write(f.0.join("clean/big.iso"), b"new").unwrap();
        let (actual, stats) = f.incremental(seed, vec![]);
        assert!(stats.fallback_reason.unwrap().contains("unresolved"));
        compare(&actual, &f.full());
        assert!(actual.iter_nodes().any(|n| n.native_name() == "big.iso"));
    }
    #[test]
    fn changed_directory_identity_never_reuses_old_children() {
        let f = Fixture::new();
        let seed = f.full();
        fs::rename(f.0.join("clean"), f.0.join("moved")).unwrap();
        fs::create_dir(f.0.join("clean")).unwrap();
        fs::write(f.0.join("clean/new"), b"replacement").unwrap();
        let (actual, _) = f.incremental(seed, vec![f.0.join("moved")]);
        compare(&actual, &f.full());
    }
    #[test]
    fn new_hardlinks_retry_and_cached_hardlinks_reuse() {
        let f = Fixture::new();
        let seed = f.full();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("dirty/link")).unwrap();
        let (actual, stats) = f.incremental(
            seed,
            vec![f.0.join("dirty/link"), f.0.join("clean/deep/kept")],
        );
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.unwrap().contains("new hard link"));
        assert_eq!(stats.reused_entries, 0);
        let (actual, stats) = f.incremental(actual, vec![]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_none());
        assert!(stats.reused_entries > 0);
        assert!(stats.unchanged_metadata);
    }
    #[test]
    fn new_file_dirties_cached_hard_links_only_while_aliases_are_unresolved() {
        let f = Fixture::new();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("clean/alias")).unwrap();
        let seed = f.full();
        // Which of the hard links' folders keep a reusable cached listing.
        let reusable = |path: &str, aliases_resolved| {
            let reuse = Reuse::new(IncrementalSeed {
                snapshot: Arc::new(seed.clone()),
                dirty_paths: vec![f.0.join(path)],
                dirty_directories: vec![],
                aliases_resolved,
            });
            ["clean", "clean/deep"].map(|d| listing_paths(&reuse).contains_key(&f.0.join(d)))
        };
        assert_eq!(reusable("dirty/new.bin", true), [true, true]);
        assert_eq!(reusable("dirty/new.bin", false), [false, false]);
        // A path that may spell a cached directory differently hides its hard links from `touched`.
        assert_eq!(reusable("dirty/NESTED/new.bin", true), [false, false]);
        fs::write(f.0.join("dirty/new.bin"), b"new").unwrap();
        let (actual, stats) =
            f.incremental_resolving(seed, vec![f.0.join("dirty/new.bin")], vec![], true);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_none());
        assert!(stats.reused_entries > 0);
    }
    #[test]
    fn changed_alias_invalidates_all_cached_aliases() {
        let f = Fixture::new();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("dirty/link")).unwrap();
        let seed = f.full();
        fs::write(f.0.join("dirty/link"), vec![1; 32768]).unwrap();
        let (actual, stats) = f.incremental(seed, vec![f.0.join("dirty/link")]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_none());
        assert!(!stats.unchanged_metadata);
    }
    #[test]
    fn hardlink_removal_and_rename_reconcile_remaining_alias() {
        let f = Fixture::new();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("dirty/link")).unwrap();
        let seed = f.full();
        fs::rename(f.0.join("dirty/link"), f.0.join("dirty/renamed")).unwrap();
        let (actual, stats) = f.incremental(seed, vec![f.0.join("dirty")]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_none());
        fs::remove_file(f.0.join("dirty/renamed")).unwrap();
        let (actual, stats) = f.incremental_with_listings(actual, vec![], vec![f.0.join("dirty")]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_none());
        assert!(!actual
            .iter_nodes()
            .map(|n| actual.legacy_node(n.id).unwrap())
            .any(|n| n.hard_link));
    }
    #[test]
    fn new_known_alias_write_refreshes_cached_members() {
        let f = Fixture::new();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("clean/alias")).unwrap();
        let seed = f.full();
        let new_alias = f.0.join("dirty/new");
        fs::hard_link(f.0.join("clean/alias"), &new_alias).unwrap();
        fs::write(&new_alias, vec![3; 65536]).unwrap();
        let (actual, stats) = f.incremental(seed, vec![new_alias]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_none());
    }
    #[test]
    fn transient_alias_write_refreshes_cached_members_after_removal() {
        let f = Fixture::new();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("clean/alias")).unwrap();
        let seed = f.full();
        let transient_alias = f.0.join("dirty/transient");
        fs::hard_link(f.0.join("clean/alias"), &transient_alias).unwrap();
        fs::write(&transient_alias, vec![4; 65536]).unwrap();
        fs::remove_file(&transient_alias).unwrap();
        let (actual, stats) = f.incremental(seed, vec![transient_alias]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_none());
    }
    #[test]
    fn ambiguous_alias_set_deduplicates_and_preserves_clean_descendants() {
        let f = Fixture::new();
        fs::create_dir_all(f.0.join("untouched/deep")).unwrap();
        fs::write(f.0.join("untouched/deep/file"), b"clean").unwrap();
        for index in 0..64 {
            fs::hard_link(
                f.0.join("clean/deep/kept"),
                f.0.join(format!("dirty/link-{index}")),
            )
            .unwrap();
        }
        let seed = f.full();
        let mut paths: Vec<_> = seed
            .iter_nodes()
            .map(|n| seed.legacy_node(n.id).unwrap())
            .filter(|n| n.hard_link)
            .map(|n| n.path.clone())
            .collect();
        paths.extend(paths.clone());
        fs::write(f.0.join("dirty/link-0"), vec![2; 16384]).unwrap();
        let (actual, stats) = f.incremental(seed, paths);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_none());
        assert!(stats.reused_entries > 0);
        assert!(!stats.unchanged_metadata);
    }
    #[test]
    fn newly_linked_clean_ordinary_inode_retries_fully() {
        let f = Fixture::new();
        let seed = f.full();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("dirty/link")).unwrap();
        let (actual, stats) = f.incremental(seed, vec![f.0.join("dirty/link")]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_some());
        assert_eq!(stats.reused_entries, 0);
    }
    #[test]
    fn hardlink_retry_publishes_only_final_stable_topology() {
        let f = Fixture::new();
        let seed = f.full();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("dirty/link")).unwrap();
        let authority = ScanRoot::open(f.0.clone(), platform_identity(&f.0).unwrap()).unwrap();
        let store = Arc::new(RwLock::new(Snapshot::empty(f.0.clone())));
        let running_reports = std::sync::atomic::AtomicUsize::new(0);
        let final_reports = std::sync::atomic::AtomicUsize::new(0);
        let stats = scan_live_incremental_authorized(
            authority,
            ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            store.clone(),
            |_| {
                let live = store.read().unwrap();
                if live.state == ScanState::Running {
                    // Includes first-attempt Completed notification: no candidate
                    // node ID can enter caller caches or frontend focus state.
                    assert!((live.node_count() == 0));
                    running_reports.fetch_add(1, Ordering::Relaxed);
                } else {
                    assert_eq!(live.state, ScanState::Completed);
                    assert!(!(live.node_count() == 0));
                    final_reports.fetch_add(1, Ordering::Relaxed);
                }
            },
            IncrementalSeed {
                dirty_directories: vec![],
                snapshot: Arc::new(seed),
                dirty_paths: vec![f.0.join("dirty/link"), f.0.join("clean/deep/kept")],
                aliases_resolved: false,
            },
        )
        .unwrap();
        assert!(stats.fallback_reason.unwrap().contains("new hard link"));
        assert!(running_reports.load(Ordering::Relaxed) >= 2);
        assert_eq!(final_reports.load(Ordering::Relaxed), 1);
        compare(&store.read().unwrap(), &f.full());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn new_clone_of_clean_reused_source_retries_privately_with_normal_workers() {
        use std::os::unix::ffi::OsStrExt;
        let f = Fixture::new();
        let source = f.0.join("clean/deep/kept");
        let destination = f.0.join("dirty/clone");
        fs::write(&source, vec![7u8; 128 * 1024]).unwrap();
        let seed = f.full();
        assert!(
            seed.incremental_accounting_eligible(),
            "{:?}",
            seed.accounting
        );
        let a = std::ffi::CString::new(source.as_os_str().as_bytes()).unwrap();
        let b = std::ffi::CString::new(destination.as_os_str().as_bytes()).unwrap();
        unsafe extern "C" {
            fn clonefile(a: *const libc::c_char, b: *const libc::c_char, flags: u32) -> i32;
        }
        assert_eq!(
            unsafe { clonefile(a.as_ptr(), b.as_ptr(), 0) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        // The source branch has no dirty event and would normally reuse its listing.
        let replacement_seed = seed.clone();
        let original_source_identity = platform_identity(&source).unwrap();
        let seed_for_reuse = seed.clone();
        let planned = Reuse::new(IncrementalSeed {
            snapshot: Arc::new(seed_for_reuse),
            dirty_paths: vec![destination.clone()],
            dirty_directories: vec![],
            aliases_resolved: false,
        });
        assert!(listing_paths(&planned).contains_key(&f.0.join("clean/deep")));
        drop(planned);
        let authority = ScanRoot::open(f.0.clone(), platform_identity(&f.0).unwrap()).unwrap();
        let store = Arc::new(RwLock::new(Snapshot::empty(f.0.clone())));
        let running = std::sync::atomic::AtomicUsize::new(0);
        let completed = std::sync::atomic::AtomicUsize::new(0);
        let workers = Mutex::new(vec![]);
        let stats = scan_live_incremental_authorized_with_validation_workers(
            authority,
            ScanOptions {
                workers: 4,
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
            store.clone(),
            |_| {
                let live = store.read().unwrap();
                if live.state == ScanState::Running {
                    assert_eq!(live.node_count(), 0);
                    running.fetch_add(1, Ordering::Relaxed);
                } else {
                    assert_eq!(live.state, ScanState::Completed);
                    assert!(live.node_count() > 0);
                    completed.fetch_add(1, Ordering::Relaxed);
                }
            },
            IncrementalSeed {
                snapshot: Arc::new(seed),
                dirty_paths: vec![destination],
                dirty_directories: vec![],
                aliases_resolved: false,
            },
            2,
            |count| workers.lock().unwrap().push(count),
        )
        .unwrap();
        assert!(stats
            .fallback_reason
            .as_deref()
            .unwrap()
            .contains("sharing"));
        assert_eq!(stats.reused_entries, 0);
        assert!(!stats.unchanged_metadata);
        assert!(running.load(Ordering::Relaxed) >= 2);
        assert_eq!(completed.load(Ordering::Relaxed), 1);
        let workers = workers.into_inner().unwrap();
        assert_eq!(workers.len(), 2);
        // Small scopes preserve the established normal-worker floor.
        assert_eq!(workers, vec![4, 4]);
        let actual = store.read().unwrap();
        let fresh = f.full();
        compare(&actual, &fresh);
        assert!(actual.accounting.shared);
        assert!(actual.accounting.deduction_bytes > 0);
        assert_eq!(
            actual.accounting.deduction_bytes,
            fresh.accounting.deduction_bytes
        );
        assert!(!actual.incremental_accounting_eligible());
        drop(actual);
        // Same-sized replacement cannot inherit the cached source's clone claim.
        fs::rename(&source, f.0.join("retired-source")).unwrap();
        fs::write(&source, vec![9u8; 128 * 1024]).unwrap();
        assert_ne!(
            platform_identity(&source).unwrap(),
            original_source_identity
        );
        let (replaced, _) = f.incremental(replacement_seed, vec![f.0.join("dirty/clone")]);
        compare(&replaced, &f.full());
        let replaced_node = replaced
            .iter_nodes()
            .find(|n| n.path().unwrap() == source)
            .unwrap();
        assert_eq!(replaced_node.identity, platform_identity(&source).unwrap());
        assert!(replaced_node.clone_metadata().is_none());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn reused_folders_are_vouched_in_bulk_and_account_like_a_full_scan() {
        let f = Fixture::new();
        let deep = f.0.join("clean/deep");
        for i in 0..6u8 {
            fs::write(deep.join(format!("file{i}")), vec![i; 5000]).unwrap();
        }
        fs::hard_link(deep.join("file0"), deep.join("linked")).unwrap();
        let seed = f.full();
        assert!(seed.incremental_accounting_eligible());
        let (actual, stats) = f.incremental(seed, vec![]);
        let fresh = f.full();
        compare(&actual, &fresh);
        assert!(stats.fallback_reason.is_none());
        assert!(stats.reused_entries >= 8);
        // Each reused file used to cost one getattrlistat; the bulk listing answers all of them.
        assert_eq!(actual.accounting.metadata_queries, 0);
        let accounting = |s: &Snapshot| {
            let a = &s.accounting;
            (
                a.checks_complete,
                a.shared,
                a.deduction_bytes,
                a.warnings.clone(),
                s.tree.clones.len(),
            )
        };
        assert_eq!(accounting(&actual), accounting(&fresh));
        assert!(actual.incremental_accounting_eligible());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn unreported_clean_file_size_drift_forces_private_accounting_retry() {
        let f = Fixture::new();
        let seed = f.full();
        assert!(seed.incremental_accounting_eligible());
        // Reused listing evidence is now stale despite an unchanged file identity.
        // The descriptor-relative accounting probe must reject its size claim.
        fs::write(f.0.join("clean/deep/kept"), vec![5u8; 16384]).unwrap();
        let new = f.0.join("dirty/new");
        fs::write(&new, b"new").unwrap();
        let authority = ScanRoot::open(f.0.clone(), platform_identity(&f.0).unwrap()).unwrap();
        let store = Arc::new(RwLock::new(Snapshot::empty(f.0.clone())));
        let finals = std::sync::atomic::AtomicUsize::new(0);
        let selections = Mutex::new(vec![]);
        let stats = scan_live_incremental_authorized_with_validation_workers(
            authority,
            ScanOptions {
                workers: 3,
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
            store.clone(),
            |_| {
                let live = store.read().unwrap();
                if live.state == ScanState::Running {
                    assert_eq!(live.node_count(), 0);
                } else {
                    assert_eq!(live.state, ScanState::Completed);
                    finals.fetch_add(1, Ordering::Relaxed);
                }
            },
            IncrementalSeed {
                snapshot: Arc::new(seed),
                dirty_paths: vec![new],
                dirty_directories: vec![],
                aliases_resolved: false,
            },
            2,
            |count| selections.lock().unwrap().push(count),
        )
        .unwrap();
        assert!(stats
            .fallback_reason
            .as_deref()
            .unwrap()
            .contains("unresolved"));
        assert_eq!(stats.reused_entries, 0);
        assert_eq!(finals.load(Ordering::Relaxed), 1);
        assert_eq!(selections.into_inner().unwrap(), vec![3, 3]);
        let actual = store.read().unwrap();
        compare(&actual, &f.full());
        assert!(actual.accounting.warnings.is_empty());
        assert!(actual.incremental_accounting_eligible());
    }
    #[test]
    fn unresolved_accounting_seed_falls_back_once_without_reuse() {
        let f = Fixture::new();
        let mut seed = f.full();
        seed.accounting.checks_complete = false;
        seed.accounting
            .warnings
            .push("injected unresolved metadata observation".into());
        let (actual, stats) = f.incremental(seed, vec![]);
        assert!(stats
            .fallback_reason
            .as_deref()
            .unwrap()
            .contains("unresolved"));
        assert_eq!(stats.reused_entries, 0);
        compare(&actual, &f.full());
        assert!(actual.incremental_accounting_eligible());
    }
    #[test]
    fn cancelled_candidate_is_published_once_with_stable_partial_ids() {
        let f = Fixture::new();
        let seed = f.full();
        let authority = ScanRoot::open(f.0.clone(), platform_identity(&f.0).unwrap()).unwrap();
        let store = Arc::new(RwLock::new(Snapshot::empty(f.0.clone())));
        let final_reports = std::sync::atomic::AtomicUsize::new(0);
        scan_live_incremental_authorized(
            authority,
            ScanOptions::default(),
            Arc::new(AtomicBool::new(true)),
            store.clone(),
            |_| {
                let live = store.read().unwrap();
                if live.state == ScanState::Running {
                    assert!((live.node_count() == 0));
                } else {
                    assert_eq!(live.state, ScanState::Cancelled);
                    assert!(live.stats.incomplete);
                    assert_eq!(live.legacy_node(0).unwrap().id, 0);
                    final_reports.fetch_add(1, Ordering::Relaxed);
                }
            },
            IncrementalSeed {
                dirty_directories: vec![],
                snapshot: Arc::new(seed),
                dirty_paths: vec![],
                aliases_resolved: false,
            },
        )
        .unwrap();
        assert_eq!(final_reports.load(Ordering::Relaxed), 1);
        assert_eq!(store.read().unwrap().state, ScanState::Cancelled);
    }
    #[test]
    fn incomplete_and_excluded_seeds_fall_back() {
        let f = Fixture::new();
        for variant in 0..4 {
            let mut seed = f.full();
            match variant {
                0 => seed.state = ScanState::Cancelled,
                1 => seed.stats.incomplete = true,
                2 => seed.stats.excluded = 1,
                _ => seed.stats.errors = 1,
            }
            let (actual, stats) = f.incremental(seed, vec![]);
            compare(&actual, &f.full());
            assert!(stats.fallback_reason.is_some());
        }
    }
    #[test]
    fn cancellation_is_not_completed_or_retried() {
        let f = Fixture::new();
        fs::hard_link(f.0.join("clean/deep/kept"), f.0.join("dirty/link")).unwrap();
        let seed = f.full();
        let authority = ScanRoot::open(f.0.clone(), platform_identity(&f.0).unwrap()).unwrap();
        let store = Arc::new(RwLock::new(Snapshot::empty(f.0.clone())));
        scan_live_incremental_authorized(
            authority,
            ScanOptions::default(),
            Arc::new(AtomicBool::new(true)),
            store.clone(),
            |_| {},
            IncrementalSeed {
                dirty_directories: vec![],
                snapshot: Arc::new(seed),
                dirty_paths: vec![],
                aliases_resolved: false,
            },
        )
        .unwrap();
        assert_eq!(store.read().unwrap().state, ScanState::Cancelled);
    }
    #[test]
    fn malformed_seed_and_out_of_scope_events_fall_back() {
        let f = Fixture::new();
        let mut seed = f.full();
        seed.update_node(1, |n| n.parent = Some(u32::MAX)).unwrap();
        let (actual, stats) = f.incremental(seed, vec![]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_some());
        let (actual, stats) = f.incremental(f.full(), vec![PathBuf::from("/outside")]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.is_some());
    }
    #[test]
    fn duplicate_cached_namespace_falls_back_without_losing_children() {
        let f = Fixture::new();
        fs::remove_dir_all(f.0.join("clean")).unwrap();
        fs::remove_dir_all(f.0.join("dirty")).unwrap();
        fs::create_dir(f.0.join("d")).unwrap();
        fs::write(f.0.join("d/left"), b"left bytes").unwrap();
        fs::write(f.0.join("d/right"), b"different right bytes").unwrap();
        let fresh = f.full();
        let at = |name: &str| {
            fresh
                .iter_nodes()
                .map(|n| fresh.legacy_node(n.id).unwrap())
                .find(|n| n.path == f.0.join(name))
                .unwrap()
                .clone()
        };
        let mut seed = fresh.clone();
        // Both directory records satisfy the previous dense topology checks,
        // but split the real listing between distinct ids for the same path.
        edit_nodes(&mut seed, |nodes, _root| {
            *nodes = vec![
                fresh.legacy_node(0).unwrap().clone(),
                at("d"),
                at("d"),
                at("d/left"),
                at("d/right"),
            ];
            for (id, node) in nodes.iter_mut().enumerate() {
                node.id = id as u32;
                node.children.clear();
            }
            nodes[0].children = vec![1, 2];
            nodes[1].parent = Some(0);
            nodes[1].children = vec![3];
            nodes[2].parent = Some(0);
            nodes[2].children = vec![4];
            nodes[3].parent = Some(1);
            nodes[4].parent = Some(2);
        });
        seed.stats.entries = 5;
        seed.stats.directories = 3;
        aggregate(&mut seed);
        let (actual, stats) = f.incremental(seed, vec![]);
        assert_eq!(
            stats.fallback_reason.as_deref(),
            Some("cached topology or event scope is invalid")
        );
        assert_eq!(stats.reused_entries, 0);
        assert!(valid_seed(&actual));
        compare(&actual, &fresh);
        // Duplicate ordinary-file records must also fail closed, even if their
        // own sizes and identities agree and parent/child links are coherent.
        let mut seed = fresh.clone();
        edit_nodes(&mut seed, |nodes, _root| {
            let mut duplicate = at("d/left");
            duplicate.id = nodes.len() as u32;
            let parent = duplicate.parent.unwrap();
            nodes[parent as usize].children.push(duplicate.id);
            nodes.push(duplicate);
        });
        seed.stats.entries += 1;
        aggregate(&mut seed);
        let (actual, stats) = f.incremental(seed, vec![]);
        assert_eq!(
            stats.fallback_reason.as_deref(),
            Some("cached topology or event scope is invalid")
        );
        assert_eq!(stats.reused_entries, 0);
        assert!(valid_seed(&actual));
        compare(&actual, &fresh);
    }
    #[test]
    fn directory_hard_link_copies_never_reuse_listings() {
        let f = Fixture::new();
        fs::create_dir(f.0.join("twin")).unwrap();
        let mut seed = f.full();
        let identity_at = |seed: &Snapshot, name: &str| {
            seed.iter_nodes()
                .map(|n| seed.legacy_node(n.id).unwrap())
                .find(|n| n.path == f.0.join(name))
                .unwrap()
                .identity
        };
        // The scan lists only one copy of a hard-linked directory; the other
        // stays an empty node. Model that by giving `twin` the inode of
        // `clean/deep`, which has children.
        let shared = identity_at(&seed, "clean/deep");
        edit_nodes(&mut seed, |nodes, _root| {
            nodes
                .iter_mut()
                .find(|n| n.path == f.0.join("twin"))
                .unwrap()
                .identity = shared.clone();
        });
        assert!(valid_seed(&seed));
        let mut reuse = Reuse::new(IncrementalSeed {
            snapshot: Arc::new(seed.clone()),
            dirty_paths: vec![],
            dirty_directories: vec![],
            aliases_resolved: false,
        });
        for name in ["clean/deep", "twin"] {
            assert!(reuse
                .take_checked(&f.0.join(name), &identity_at(&seed, name))
                .is_none());
        }
        assert!(reuse
            .take_checked(
                &f.0.join("dirty/nested"),
                &identity_at(&seed, "dirty/nested")
            )
            .is_some());
    }
    #[test]
    fn root_replacement_declines_cached_authority() {
        let f = Fixture::new();
        let seed = f.full();
        let moved = f.0.with_extension("moved");
        fs::rename(&f.0, &moved).unwrap();
        fs::create_dir(&f.0).unwrap();
        fs::write(f.0.join("replacement"), b"new").unwrap();
        let (actual, stats) = f.incremental(seed, vec![]);
        compare(&actual, &f.full());
        assert!(stats.fallback_reason.unwrap().contains("root identity"));
        fs::remove_dir_all(moved).unwrap();
    }
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn native_names_round_trip_through_reused_listings() {
        use std::os::unix::ffi::OsStringExt;
        let f = Fixture::new();
        let path =
            f.0.join("clean/deep")
                .join(std::ffi::OsString::from_vec(vec![0xff, b'x']));
        fs::write(path, b"native").unwrap();
        let (actual, stats) = f.incremental(f.full(), vec![]);
        compare(&actual, &f.full());
        assert!(stats.reused_entries > 0);
    }
    #[test]
    #[ignore = "bounded native metadata-reuse benchmark"]
    fn benchmark_twenty_thousand_entries() {
        let f = Fixture::new();
        for dir in 0..20 {
            let path = f.0.join(format!("group-{dir}"));
            fs::create_dir(&path).unwrap();
            for file in 0..1000 {
                fs::write(path.join(format!("entry-{file}")), b"x").unwrap();
            }
        }
        let started = Instant::now();
        let seed = f.full();
        let full = started.elapsed();
        let started = Instant::now();
        let (actual, stats) = f.incremental(seed, vec![]);
        let incremental = started.elapsed();
        compare(&actual, &f.full());
        eprintln!("entries={} full={full:?} incremental={incremental:?} reused={} enumerated_directories={}", actual.stats.entries, stats.reused_entries, stats.rescanned_directories);
        assert!(stats.reused_entries >= 20_000);
        assert_eq!(stats.rescanned_directories, 1);
    }
}
