//! Read-only, path-based comparison of validated historical snapshots.
#[cfg(test)]
use disk_scan::Node;
use disk_scan::{Identity, NodeKind, NodeRef, Snapshot};
use serde::Serialize;
use std::{
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

// JavaScript consumes numeric deltas; keep arithmetic exact until this wire boundary.
fn serialize_delta<S: serde::Serializer>(value: &i128, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_f64(*value as f64)
}
const MAX_COMBINED_NODES: usize = 16_000_000;
const RANK_LIMIT: usize = 100;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Totals {
    entries: u64,
    logical_bytes: u64,
    allocated_bytes: u64,
    incomplete: bool,
    errors: u64,
    excluded: u64,
}
impl Totals {
    fn of(s: &Snapshot) -> Self {
        Self {
            entries: s.stats.entries,
            logical_bytes: s.stats.logical_bytes,
            allocated_bytes: s.stats.allocated_bytes,
            incomplete: s.stats.incomplete,
            errors: s.stats.errors,
            excluded: s.stats.excluded,
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Comparison {
    before_id: String,
    after_id: String,
    root_path: String,
    before: Totals,
    after: Totals,
    #[serde(serialize_with = "serialize_delta")]
    logical_delta: i128,
    #[serde(serialize_with = "serialize_delta")]
    allocated_delta: i128,
    added: u64,
    removed: u64,
    changed: u64,
    unchanged: u64,
    warnings: Vec<String>,
    rankings: Vec<ComparisonRanking>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComparisonRanking {
    metric: &'static str,
    kind: &'static str,
    direction: &'static str,
    total: u64,
    items: Vec<ComparisonRow>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComparisonRow {
    path: String,
    kind: &'static str,
    status: &'static str,
    before_logical_bytes: u64,
    after_logical_bytes: u64,
    before_allocated_bytes: u64,
    after_allocated_bytes: u64,
    #[serde(serialize_with = "serialize_delta")]
    logical_delta: i128,
    #[serde(serialize_with = "serialize_delta")]
    allocated_delta: i128,
}
#[derive(PartialEq, Eq)]
struct Record {
    identity: Identity,
    logical: u64,
    allocated: u64,
    own_logical: u64,
    own_allocated: u64,
    excluded: bool,
    hard_link: bool,
    clone_metadata: Option<Box<disk_scan::CloneMetadata>>,
}
impl Record {
    fn of(n: &NodeRef<'_>) -> Self {
        Self {
            identity: n.identity.clone(),
            logical: n.logical_size,
            allocated: n.allocated_size,
            own_logical: n.own_logical_size,
            own_allocated: n.own_allocated_size,
            excluded: n.excluded,
            hard_link: n.hard_link,
            clone_metadata: n.clone_metadata().cloned().map(Box::new),
        }
    }
}
pub(crate) struct Baseline {
    root: PathBuf,
    root_identity: Identity,
    count: usize,
    records: Vec<(Vec<u8>, Record, bool)>,
    totals: Totals,
    accounting_model: disk_scan::AccountingModel,
    accounting_warnings: Vec<String>,
}
struct Ranked {
    path: PathBuf,
    kind: NodeKind,
    status: &'static str,
    before_logical: u64,
    after_logical: u64,
    before_allocated: u64,
    after_allocated: u64,
}
impl Ranked {
    fn delta(&self, allocated: bool) -> i128 {
        if allocated {
            i128::from(self.after_allocated) - i128::from(self.before_allocated)
        } else {
            i128::from(self.after_logical) - i128::from(self.before_logical)
        }
    }
    fn row(self, root: &std::path::Path) -> ComparisonRow {
        let logical_delta = self.delta(false);
        let allocated_delta = self.delta(true);
        ComparisonRow {
            path: root.join(self.path).to_string_lossy().into_owned(),
            kind: match self.kind {
                NodeKind::Directory => "directory",
                NodeKind::File => "file",
                NodeKind::Symlink => "symlink",
                NodeKind::Other => "other",
            },
            status: self.status,
            before_logical_bytes: self.before_logical,
            after_logical_bytes: self.after_logical,
            before_allocated_bytes: self.before_allocated,
            after_allocated_bytes: self.after_allocated,
            logical_delta,
            allocated_delta,
        }
    }
}
struct Bucket {
    allocated: bool,
    folders: bool,
    direction: &'static str,
    total: u64,
    items: Vec<Ranked>,
}
impl Bucket {
    fn consider(
        &mut self,
        path: &std::path::Path,
        before: Option<&Record>,
        after: Option<&Record>,
        status: &'static str,
    ) {
        let kind = after
            .or(before)
            .expect("comparison has a record")
            .identity
            .kind;
        if (kind == NodeKind::Directory) != self.folders {
            return;
        }
        let candidate = Ranked {
            path: PathBuf::new(),
            kind,
            status,
            before_logical: before.map_or(0, |r| r.logical),
            after_logical: after.map_or(0, |r| r.logical),
            before_allocated: before.map_or(0, |r| r.allocated),
            after_allocated: after.map_or(0, |r| r.allocated),
        };
        let delta = candidate.delta(self.allocated);
        let included = match self.direction {
            "growth" => delta > 0,
            "shrinkage" => delta < 0,
            category => status == category,
        };
        if !included {
            return;
        }
        self.total += 1;
        let magnitude = delta.abs();
        let pos = self.items.partition_point(|r| {
            let other = r.delta(self.allocated).abs();
            other > magnitude
                || (other == magnitude
                    && r.path.as_os_str().as_bytes() < path.as_os_str().as_bytes())
        });
        if pos < RANK_LIMIT {
            let mut candidate = candidate;
            candidate.path = path.to_path_buf();
            self.items.insert(pos, candidate);
            if self.items.len() > RANK_LIMIT {
                self.items.pop();
            }
        }
    }
}
fn relative_bytes<'a>(
    root: &std::path::Path,
    path: &'a std::path::Path,
) -> Result<&'a [u8], String> {
    let prefix = root.as_os_str().as_bytes();
    let bytes = path.as_os_str().as_bytes();
    let prefix = prefix.strip_suffix(b"/").unwrap_or(prefix);
    bytes
        .strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_prefix(b"/"))
        .filter(|suffix| !suffix.is_empty())
        .ok_or_else(|| "Saved scan contains a path outside its root".into())
}
/// Validate paths before sorting immutable IDs. Ordinary single-component
/// basenames need only a sibling index; legacy spellings use exact raw paths.
fn validate_paths(snapshot: &Snapshot) -> Result<(), String> {
    let mut left = PathBuf::new();
    let mut right = PathBuf::new();
    let root = snapshot.node(0).ok_or("Saved scan has no root")?;
    snapshot.path_into(0, &mut left)?;
    if root.parent.is_some() || left != snapshot.root {
        return Err("Saved scan has no root".into());
    }
    let mut ordinary = !snapshot.has_path_overrides();
    for node in snapshot.iter_nodes().skip(1) {
        snapshot.path_into(node.id, &mut left)?;
        relative_bytes(&snapshot.root, &left)?;
        let mut components = std::path::Path::new(node.native_name()).components();
        ordinary &= matches!(components.next(), Some(std::path::Component::Normal(_)))
            && components.next().is_none();
    }
    let mut ids: Vec<u32> = (0..snapshot.node_count() as u32).collect();
    if ordinary {
        ids.sort_unstable_by(|&a, &b| {
            let a = snapshot.node(a).expect("dense IDs");
            let b = snapshot.node(b).expect("dense IDs");
            a.parent
                .cmp(&b.parent)
                .then_with(|| a.native_name().as_bytes().cmp(b.native_name().as_bytes()))
        });
        if ids.windows(2).any(|pair| {
            let a = snapshot.node(pair[0]).expect("dense IDs");
            let b = snapshot.node(pair[1]).expect("dense IDs");
            a.parent == b.parent && a.native_name().as_bytes() == b.native_name().as_bytes()
        }) {
            return Err("Saved scan contains duplicate paths".into());
        }
    } else {
        ids.sort_unstable_by(|&a, &b| {
            snapshot
                .path_into(a, &mut left)
                .expect("prevalidated comparison path");
            snapshot
                .path_into(b, &mut right)
                .expect("prevalidated comparison path");
            left.as_os_str()
                .as_bytes()
                .cmp(right.as_os_str().as_bytes())
        });
        for pair in ids.windows(2) {
            snapshot.path_into(pair[0], &mut left)?;
            snapshot.path_into(pair[1], &mut right)?;
            if left.as_os_str().as_bytes() == right.as_os_str().as_bytes() {
                return Err("Saved scan contains duplicate paths".into());
            }
        }
    }
    Ok(())
}
impl Baseline {
    pub(crate) fn from_snapshot(snapshot: &Snapshot) -> Result<Self, String> {
        if snapshot.node_count() > MAX_COMBINED_NODES {
            return Err("Comparison exceeds the 16 million combined-node limit".into());
        }
        validate_paths(snapshot)?;
        let root = snapshot.node(0).ok_or("Saved scan has no root")?;
        let mut scratch = PathBuf::new();
        let mut records = Vec::with_capacity(snapshot.node_count().saturating_sub(1));
        for node in snapshot.iter_nodes() {
            if node.id == root.id {
                continue;
            }
            snapshot.path_into(node.id, &mut scratch)?;
            let path = relative_bytes(&snapshot.root, &scratch)?;
            records.push((path.to_vec(), Record::of(&node), false));
        }
        records.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        if records.windows(2).any(|p| p[0].0 == p[1].0) {
            return Err("Saved scan contains duplicate paths".into());
        }
        Ok(Self {
            root: snapshot.root.clone(),
            root_identity: root.identity.clone(),
            count: snapshot.node_count(),
            records,
            totals: Totals::of(snapshot),
            accounting_model: snapshot.accounting.model,
            accounting_warnings: snapshot.accounting.warnings.clone(),
        })
    }
    pub(crate) fn compare(
        mut self,
        after: &Snapshot,
        before_id: String,
        after_id: String,
    ) -> Result<Comparison, String> {
        if self.root.as_os_str().as_bytes() != after.root.as_os_str().as_bytes() {
            return Err("Choose saved scans of the same location".into());
        }
        if self
            .count
            .checked_add(after.node_count())
            .is_none_or(|n| n > MAX_COMBINED_NODES)
        {
            return Err("Comparison exceeds the 16 million combined-node limit".into());
        }
        validate_paths(after)?;
        let root = after.node(0).ok_or("Saved scan has no root")?;
        let mut warnings = vec!["Changes describe recorded metadata, not file contents; renames are shown as additions and removals.".into()];
        warnings.push("Folder sizes include descendants, so ranked changes must not be added together. Hard-link allocated-byte ownership can move between paths without changing total storage. Recorded allocation does not estimate reclaimable space; full-clone ownership can also move between paths.".into());
        if self.accounting_model != after.accounting.model {
            warnings.push("These scans use different allocation accounting models; allocated differences can reflect full-clone accounting changes.".into());
        }
        warnings.extend(self.accounting_warnings.iter().cloned());
        warnings.extend(after.accounting.warnings.iter().cloned());
        if self.totals.incomplete
            || after.stats.incomplete
            || self.totals.errors > 0
            || after.stats.errors > 0
            || self.totals.excluded > 0
            || after.stats.excluded > 0
        {
            warnings.push("Coverage is incomplete or excludes locations. Additions and removals are observed differences, not proof that files were created or deleted.".into());
        }
        if self.root_identity != root.identity {
            warnings.push("The location's recorded filesystem identity changed; it may have been replaced or remounted.".into());
        }
        let mut buckets = Vec::new();
        for allocated in [false, true] {
            for folders in [false, true] {
                for direction in ["growth", "shrinkage", "added", "removed", "changed"] {
                    buckets.push(Bucket {
                        allocated,
                        folders,
                        direction,
                        total: 0,
                        items: Vec::new(),
                    });
                }
            }
        }
        let (mut added, mut removed, mut changed, mut unchanged) = (0, 0, 0, 0);
        let mut scratch = PathBuf::new();
        for node in after.iter_nodes() {
            if node.id == root.id {
                continue;
            }
            after.path_into(node.id, &mut scratch)?;
            let path_bytes = relative_bytes(&after.root, &scratch)?;
            let path = std::path::Path::new(std::ffi::OsStr::from_bytes(path_bytes));
            let old = match self
                .records
                .binary_search_by(|r| r.0.as_slice().cmp(path.as_os_str().as_bytes()))
            {
                Ok(index) => {
                    if self.records[index].2 {
                        return Err("Saved scan contains duplicate paths".into());
                    }
                    self.records[index].2 = true;
                    Some(&self.records[index].1)
                }
                Err(_) => None,
            };
            let new = Record::of(&node);
            let status = match old {
                None => {
                    added += 1;
                    "added"
                }
                Some(r) if *r != new => {
                    changed += 1;
                    "changed"
                }
                Some(_) => {
                    unchanged += 1;
                    continue;
                }
            };
            for bucket in &mut buckets {
                bucket.consider(path, old, Some(&new), status);
            }
        }
        for (bytes, record, matched) in self.records {
            if matched {
                continue;
            }
            let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
            removed += 1;
            for bucket in &mut buckets {
                bucket.consider(&path, Some(&record), None, "removed");
            }
        }
        let after_totals = Totals::of(after);
        Ok(Comparison {
            before_id,
            after_id,
            root_path: self.root.to_string_lossy().into_owned(),
            logical_delta: i128::from(after_totals.logical_bytes)
                - i128::from(self.totals.logical_bytes),
            allocated_delta: i128::from(after_totals.allocated_bytes)
                - i128::from(self.totals.allocated_bytes),
            before: self.totals,
            after: after_totals,
            added,
            removed,
            changed,
            unchanged,
            warnings,
            rankings: buckets
                .into_iter()
                .map(|b| ComparisonRanking {
                    metric: if b.allocated { "allocated" } else { "logical" },
                    kind: if b.folders { "folders" } else { "files" },
                    direction: b.direction,
                    total: b.total,
                    items: b.items.into_iter().map(|r| r.row(&self.root)).collect(),
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{atomic::AtomicBool, Arc},
    };
    fn scan(path: &std::path::Path) -> Snapshot {
        disk_scan::scan(
            path.to_path_buf(),
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap()
    }
    fn compare(before: &Snapshot, after: &Snapshot) -> Comparison {
        Baseline::from_snapshot(before)
            .unwrap()
            .compare(after, "before".into(), "after".into())
            .unwrap()
    }
    #[test]
    fn mixed_accounting_models_remain_comparable_with_warning() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("file"), b"data").unwrap();
        let after = scan(directory.path());
        let mut before = after.clone();
        before.accounting.model = disk_scan::AccountingModel::HardLinksV1;
        let result = compare(&before, &after);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("different allocation accounting models")));
        assert_eq!(result.allocated_delta, 0);
    }

    #[test]
    fn real_paths_sizes_links_and_metadata_changes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("same"), b"same").unwrap();
        fs::write(dir.path().join("removed"), b"gone").unwrap();
        fs::write(dir.path().join("grown"), b"x").unwrap();
        let non_utf8 = dir.path().join("original");
        fs::write(&non_utf8, b"one").unwrap();
        fs::hard_link(&non_utf8, dir.path().join("alias")).unwrap();
        let sparse = fs::File::create(dir.path().join("sparse")).unwrap();
        sparse.set_len(16 * 1024 * 1024).unwrap();
        std::os::unix::fs::symlink("same", dir.path().join("link")).unwrap();
        let before = scan(dir.path());
        fs::remove_file(dir.path().join("removed")).unwrap();
        fs::write(dir.path().join("added"), b"new").unwrap();
        fs::write(dir.path().join("grown"), b"longer").unwrap();
        let mut after = scan(dir.path());
        let same = after
            .iter_nodes()
            .find(|n| n.display_name() == "same")
            .unwrap()
            .id;
        after
            .update_node(same, |n| {
                n.identity.inode = n.identity.inode.wrapping_add(100)
            })
            .unwrap();
        let result = compare(&before, &after);
        assert_eq!((result.added, result.removed, result.changed), (1, 1, 2));
        assert_eq!(result.unchanged, before.node_count() as u64 - 4);
        assert_eq!(
            result.logical_delta,
            i128::from(after.stats.logical_bytes) - i128::from(before.stats.logical_bytes)
        );
        assert_eq!(
            result.before.logical_bytes,
            before.node(0).unwrap().logical_size
        );
        assert_eq!(
            result.before.allocated_bytes,
            before.node(0).unwrap().allocated_size
        );
        assert!(before.stats.logical_bytes > before.stats.allocated_bytes);
        assert_eq!(
            result
                .rankings
                .iter()
                .find(|b| b.metric == "logical" && b.kind == "files" && b.direction == "growth")
                .unwrap()
                .total,
            2
        );
        // Two distinct invalid UTF-8 names must not collapse into their identical display text.
        let mut before = before.clone();
        let alias = before
            .iter_nodes()
            .find(|n| n.display_name() == "alias")
            .unwrap()
            .id;
        let mut node = before.legacy_node(alias).unwrap();
        node.path = before.root.join(std::ffi::OsString::from_vec(vec![0xff]));
        before.replace_node(alias, node).unwrap();
        let mut after = before.clone();
        let mut node = after.legacy_node(alias).unwrap();
        node.path = before.root.join(std::ffi::OsString::from_vec(vec![0xfe]));
        after.replace_node(alias, node).unwrap();
        assert_eq!(
            (
                compare(&before, &after).added,
                compare(&before, &after).removed
            ),
            (1, 1)
        );
    }
    #[test]
    fn folders_and_unsigned_extremes_keep_signed_deltas() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();
        fs::write(dir.path().join("folder/file"), b"data").unwrap();
        let mut before = scan(dir.path());
        let folder = before
            .iter_nodes()
            .find(|n| n.display_name() == "folder")
            .unwrap()
            .id;
        before
            .update_node(folder, |n| {
                n.logical_size = u64::MAX;
                n.allocated_size = u64::MAX;
            })
            .unwrap();
        before.stats.logical_bytes = u64::MAX;
        before.stats.allocated_bytes = u64::MAX;
        let mut after = before.clone();
        after
            .update_node(folder, |n| {
                n.logical_size = 0;
                n.allocated_size = 1;
            })
            .unwrap();
        after.stats.logical_bytes = 0;
        after.stats.allocated_bytes = 1;
        let result = compare(&before, &after);
        assert_eq!(result.logical_delta, -i128::from(u64::MAX));
        assert_eq!(result.allocated_delta, 1 - i128::from(u64::MAX));
        let bucket = result
            .rankings
            .iter()
            .find(|b| b.metric == "logical" && b.kind == "folders" && b.direction == "shrinkage")
            .unwrap();
        assert_eq!((bucket.total, bucket.items.len()), (1, 1));
        assert_eq!(bucket.items[0].logical_delta, -i128::from(u64::MAX));
        let json = serde_json::to_value(result).unwrap();
        assert!(json.get("beforeId").is_some());
        assert!(json.get("logicalDelta").unwrap().is_number());
    }

    #[test]
    fn zero_size_entries_and_equal_size_changes_remain_visible() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("removed"), b"").unwrap();
        fs::write(dir.path().join("changed"), b"same").unwrap();
        let before = scan(dir.path());
        fs::remove_file(dir.path().join("removed")).unwrap();
        fs::write(dir.path().join("added"), b"").unwrap();
        let mut after = scan(dir.path());
        let changed = after
            .iter_nodes()
            .find(|n| n.display_name() == "changed")
            .unwrap()
            .id;
        after
            .update_node(changed, |n| n.identity.inode += 100)
            .unwrap();
        let result = compare(&before, &after);
        assert_eq!(result.rankings.len(), 20);
        for metric in ["logical", "allocated"] {
            for status in ["added", "removed", "changed"] {
                let bucket = result
                    .rankings
                    .iter()
                    .find(|b| b.metric == metric && b.kind == "files" && b.direction == status)
                    .unwrap();
                assert_eq!((bucket.total, bucket.items.len()), (1, 1));
                assert_eq!(bucket.items[0].status, status);
                assert_eq!(bucket.items[0].logical_delta, 0);
                assert_eq!(bucket.items[0].allocated_delta, 0);
            }
        }
        assert!(result
            .rankings
            .iter()
            .filter(|b| b.direction == "growth" || b.direction == "shrinkage")
            .all(|b| b.items.is_empty()));
    }

    #[test]
    fn rejects_duplicate_added_paths() {
        let dir = tempfile::tempdir().unwrap();
        let before = scan(dir.path());
        fs::write(dir.path().join("added"), b"x").unwrap();
        let mut after = scan(dir.path());
        let mut duplicate = after.legacy_node(1).unwrap();
        duplicate.id = after.node_count() as u32;
        after.append_legacy_node(duplicate).unwrap();
        assert!(Baseline::from_snapshot(&before)
            .unwrap()
            .compare(&after, "a".into(), "b".into())
            .unwrap_err()
            .contains("duplicate"));
    }
    #[test]
    fn root_paths_and_native_duplicate_validation() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();
        fs::write(dir.path().join("folder/file"), b"x").unwrap();
        let snapshot = scan(dir.path());
        assert!(!snapshot.has_path_overrides());
        let file = snapshot
            .iter_nodes()
            .find(|n| n.display_name() == "file")
            .unwrap()
            .id;
        let mut duplicate = snapshot.legacy_node(file).unwrap();
        duplicate.id = snapshot.node_count() as u32;
        let mut ordinary = snapshot.clone();
        ordinary.append_legacy_node(duplicate).unwrap();
        assert!(!ordinary.has_path_overrides());
        assert!(validate_paths(&ordinary).unwrap_err().contains("duplicate"));
        let mut invalid = snapshot.clone();
        invalid
            .update_node(file, |n| n.parent = Some(file))
            .unwrap();
        assert!(validate_paths(&invalid).is_err());
        let mut invalid = snapshot.clone();
        invalid.update_node(0, |n| n.parent = Some(0)).unwrap();
        assert!(Baseline::from_snapshot(&invalid).is_err());
        let mut legacy = snapshot.clone();
        let mut override_node = legacy.legacy_node(file).unwrap();
        override_node.path = legacy.root.join("outside-layout");
        legacy.replace_node(file, override_node.clone()).unwrap();
        override_node.id = legacy.node_count() as u32;
        legacy.append_legacy_node(override_node).unwrap();
        assert!(legacy.has_path_overrides());
        assert!(validate_paths(&legacy).unwrap_err().contains("duplicate"));
        let mut outside = snapshot.clone();
        let mut node = outside.legacy_node(file).unwrap();
        node.path = PathBuf::from("/different-root/file");
        outside.replace_node(file, node).unwrap();
        assert!(validate_paths(&outside).unwrap_err().contains("outside"));
    }

    #[test]
    fn retained_root_spelling_preserves_component_equality_and_raw_output() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file"), b"x").unwrap();
        let canonical = scan(dir.path());
        for suffix in ["//", "/./"] {
            let mut retained = canonical.clone();
            let child = retained.legacy_node(1).unwrap();
            let mut root = retained.legacy_node(0).unwrap();
            let mut bytes = retained.root.as_os_str().as_bytes().to_vec();
            bytes.extend_from_slice(suffix.as_bytes());
            root.path = PathBuf::from(std::ffi::OsString::from_vec(bytes.clone()));
            retained.replace_node(0, root).unwrap();
            retained.replace_node(1, child).unwrap();
            assert_eq!(retained.path(0).unwrap().as_os_str().as_bytes(), bytes);
            let result = compare(&canonical, &retained);
            assert_eq!(
                (
                    result.added,
                    result.removed,
                    result.changed,
                    result.unchanged
                ),
                (0, 0, 0, 1)
            );
            assert_eq!(
                result.root_path.as_bytes(),
                canonical.root.as_os_str().as_bytes()
            );
            assert_eq!(
                retained.path(1).unwrap().as_os_str().as_bytes(),
                canonical.path(1).unwrap().as_os_str().as_bytes()
            );
            assert_eq!(
                Baseline::from_snapshot(&retained).unwrap().records[0].0,
                b"file"
            );
        }
    }

    #[test]
    fn legacy_raw_spellings_remain_distinct() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), b"x").unwrap();
        fs::write(dir.path().join("b"), b"x").unwrap();
        let mut snapshot = scan(dir.path());
        let a = snapshot
            .iter_nodes()
            .find(|n| n.display_name() == "a")
            .unwrap()
            .id;
        let b = snapshot
            .iter_nodes()
            .find(|n| n.display_name() == "b")
            .unwrap()
            .id;
        for (id, spelling) in [(a, "nested//file"), (b, "nested/file")] {
            let mut node = snapshot.legacy_node(id).unwrap();
            node.path = snapshot.root.join(spelling);
            snapshot.replace_node(id, node).unwrap();
        }
        validate_paths(&snapshot).unwrap();
        let result = compare(&snapshot, &snapshot);
        assert_eq!(
            (
                result.added,
                result.removed,
                result.changed,
                result.unchanged
            ),
            (0, 0, 0, 2)
        );
    }

    #[test]
    #[ignore = "million-entry synthetic performance fixture"]
    fn million_entry_profile() {
        fn synthetic(size: u64) -> Snapshot {
            let mut s = Snapshot::empty(PathBuf::from("/comparison-fixture"));
            for id in 0..=1_000_000 {
                let root = id == 0;
                let kind = if root {
                    NodeKind::Directory
                } else {
                    NodeKind::File
                };
                s.append_legacy_node(Node {
                    id,
                    parent: if root { None } else { Some(0) },
                    children: vec![],
                    name: id.to_string(),
                    path: if root {
                        s.root.clone()
                    } else {
                        s.root.join(format!("{id:07}"))
                    },
                    identity: Identity {
                        device: 1,
                        inode: u64::from(id),
                        kind,
                    },
                    logical_size: size,
                    allocated_size: size,
                    own_logical_size: size,
                    own_allocated_size: size,
                    is_directory: root,
                    hard_link: false,
                    excluded: false,
                    clone_metadata: None,
                })
                .unwrap();
            }
            s.set_children(0, &(1..=1_000_000).collect::<Vec<_>>())
                .unwrap();
            s
        }
        let before = synthetic(1);
        let start = std::time::Instant::now();
        let baseline = Baseline::from_snapshot(&before).unwrap();
        let baseline_ms = start.elapsed().as_millis();
        drop(before);
        let after = synthetic(2);
        let start = std::time::Instant::now();
        let result = baseline.compare(&after, "a".into(), "b".into()).unwrap();
        let compare_ms = start.elapsed().as_millis();
        assert_eq!(result.changed, 1_000_000);
        assert!(result.rankings.iter().all(|b| b.items.len() <= 100));
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        assert_eq!(
            unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
            0
        );
        let rss = unsafe { usage.assume_init() }.ru_maxrss;
        eprintln!("comparison_profile: entries=1000000 baseline_ms={baseline_ms} compare_ms={compare_ms} peak_rss_native={rss}");
    }

    #[test]
    fn coverage_identity_limits_and_rank_cap() {
        let dir = tempfile::tempdir().unwrap();
        let before = scan(dir.path());
        for i in (0..105).rev() {
            fs::write(dir.path().join(format!("{i:03}")), b"x").unwrap();
        }
        let mut after = scan(dir.path());
        after.stats.incomplete = true;
        after.stats.excluded = 1;
        after.update_node(0, |n| n.identity.inode += 1).unwrap();
        let result = compare(&before, &after);
        assert_eq!(result.warnings.len(), 4);
        let bucket = result
            .rankings
            .iter()
            .find(|b| b.metric == "logical" && b.kind == "files" && b.direction == "growth")
            .unwrap();
        assert_eq!((bucket.total, bucket.items.len()), (105, 100));
        assert!(bucket.items[0].path.ends_with("000"));
        assert!(bucket.items[99].path.ends_with("099"));
        let other = tempfile::tempdir().unwrap();
        assert!(Baseline::from_snapshot(&before)
            .unwrap()
            .compare(&scan(other.path()), "a".into(), "b".into())
            .err()
            .unwrap()
            .contains("same location"));
        let mut baseline = Baseline::from_snapshot(&before).unwrap();
        baseline.count = MAX_COMBINED_NODES;
        assert!(baseline
            .compare(&after, "a".into(), "b".into())
            .err()
            .unwrap()
            .contains("16 million"));
    }
}
