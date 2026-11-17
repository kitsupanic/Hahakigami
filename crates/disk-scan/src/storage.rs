//! Untimed retained-buffer estimates, excluding allocator headers, fragmentation and scan scratch.
use crate::{NodeKind, ScanIssue, Snapshot, WorkerStats};
use serde::Serialize;
use std::mem::size_of;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct BufferStorage {
    pub length_bytes: usize,
    pub capacity_bytes: usize,
}
impl BufferStorage {
    fn add(&mut self, length: usize, capacity: usize) {
        self.length_bytes += length;
        self.capacity_bytes += capacity;
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct NodeCounts {
    pub directories: usize,
    pub files: usize,
    pub symlinks: usize,
    pub other: usize,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct StorageStats {
    pub schema_version: u32,
    pub nodes_length: usize,
    pub nodes_capacity: usize,
    pub node_size_bytes: usize,
    pub layout: &'static str,
    pub records: BufferStorage,
    pub record_storage: &'static str,
    pub record_chunks: BufferStorage,
    pub clones: BufferStorage,
    pub clone_attributions: BufferStorage,
    pub accounting: BufferStorage,
    pub native_names: BufferStorage,
    pub path_overrides: BufferStorage,
    pub display_name_overrides: BufferStorage,
    pub children: BufferStorage,
    pub root: BufferStorage,
    pub issues: BufferStorage,
    pub workers: BufferStorage,
    pub counts: NodeCounts,
    pub retained_length_bytes: usize,
    pub retained_capacity_bytes: usize,
    /// Snapshot owns no query index; callers must measure their own indexes separately.
    pub snapshot_index_bytes: usize,
}
/// Estimate storage owned by a snapshot without cloning its tree. Capacity is requested buffer
/// storage, not allocator-reported bytes or RSS. Scan queues, hash sets and caller indexes are absent.
pub(crate) fn buffer_stats(snapshot: &Snapshot) -> StorageStats {
    let mut result = StorageStats {
        schema_version: 2,
        layout: "compact_native_v1",
        record_storage: "chunked_4096_v1",
        nodes_length: snapshot.node_count(),
        nodes_capacity: snapshot.tree.records.capacity(),
        node_size_bytes: size_of::<crate::tree::Record>(),
        ..StorageStats::default()
    };
    result.clone_attributions.add(
        snapshot.tree.clone_attributions.len() * size_of::<(u32, (bool, u64))>(),
        snapshot.tree.clone_attributions.capacity() * size_of::<(u32, (bool, u64))>(),
    );
    result.clones.add(
        snapshot.tree.clones.len() * size_of::<(u32, crate::CloneMetadata)>(),
        snapshot.tree.clones.capacity() * size_of::<(u32, crate::CloneMetadata)>(),
    );
    result.accounting.add(
        snapshot.accounting.warnings.len() * size_of::<String>(),
        snapshot.accounting.warnings.capacity() * size_of::<String>(),
    );
    for warning in &snapshot.accounting.warnings {
        result.accounting.add(warning.len(), warning.capacity());
    }
    result.records.add(
        snapshot.tree.records.len() * size_of::<crate::tree::Record>(),
        snapshot.tree.records.capacity() * size_of::<crate::tree::Record>(),
    );
    let (length, capacity) = snapshot.tree.records.header_storage();
    result.record_chunks.add(length, capacity);
    result
        .native_names
        .add(snapshot.tree.names.len(), snapshot.tree.names.capacity());
    result.children.add(
        snapshot.tree.children.len() * 4,
        snapshot.tree.children.capacity() * 4,
    );
    result
        .root
        .add(snapshot.root.as_os_str().len(), snapshot.root.capacity());
    result.path_overrides.add(
        snapshot.tree.paths.len() * size_of::<(u32, std::path::PathBuf)>(),
        snapshot.tree.paths.capacity() * size_of::<(u32, std::path::PathBuf)>(),
    );
    for (_, p) in &snapshot.tree.paths {
        result.path_overrides.add(p.as_os_str().len(), p.capacity());
    }
    result.display_name_overrides.add(
        snapshot.tree.labels.len() * size_of::<(u32, String)>(),
        snapshot.tree.labels.capacity() * size_of::<(u32, String)>(),
    );
    for (_, label) in &snapshot.tree.labels {
        result
            .display_name_overrides
            .add(label.len(), label.capacity());
    }
    for issues in [&snapshot.errors, &snapshot.exclusions] {
        result.issues.add(
            issues.len() * size_of::<ScanIssue>(),
            issues.capacity() * size_of::<ScanIssue>(),
        );
        for issue in issues {
            result
                .issues
                .add(issue.path.as_os_str().len(), issue.path.capacity());
            result
                .issues
                .add(issue.message.len(), issue.message.capacity());
            if let Some(kind) = &issue.error_kind {
                result.issues.add(kind.len(), kind.capacity());
            }
        }
    }
    result.workers.add(
        snapshot.worker_stats.len() * size_of::<WorkerStats>(),
        snapshot.worker_stats.capacity() * size_of::<WorkerStats>(),
    );
    for worker in &snapshot.worker_stats {
        result.workers.add(worker.qos.len(), worker.qos.capacity());
    }
    for buffer in [
        result.records,
        result.record_chunks,
        result.clones,
        result.clone_attributions,
        result.accounting,
        result.native_names,
        result.path_overrides,
        result.display_name_overrides,
        result.children,
        result.root,
        result.issues,
        result.workers,
    ] {
        result.retained_length_bytes += buffer.length_bytes;
        result.retained_capacity_bytes += buffer.capacity_bytes;
    }
    result
}

pub fn storage_stats(snapshot: &Snapshot) -> StorageStats {
    let mut result = buffer_stats(snapshot);
    for node in snapshot.iter_nodes() {
        match node.identity.kind {
            NodeKind::Directory => result.counts.directories += 1,
            NodeKind::File => result.counts.files += 1,
            NodeKind::Symlink => result.counts.symlinks += 1,
            NodeKind::Other => result.counts.other += 1,
        }
    }
    result
}
