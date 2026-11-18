//! Versioned wire format of the private rescan cache.
use crate::cache_store::{err, ScanMetadata};
use disk_scan::{
    AccountingInfo, Identity, LegacyTreeBuilder, Node, NodeKind, ScanState, ScanStats, Snapshot,
    WorkerStats,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    io::{Read, Write},
    os::unix::ffi::{OsStrExt, OsStringExt},
};
const MAGIC: &[u8; 8] = b"HAHCAC04";
const FIELD_LIMIT: usize = 64 * 1024;
const HEADER_LIMIT: usize = 1024 * 1024;
const RECORD_BYTES: usize = 61;
pub(crate) const WRITE_BATCH: usize = 128 * 1024;
#[derive(Serialize, Deserialize)]
struct Header {
    metadata: ScanMetadata,
    accounting: AccountingInfo,
    stats: ScanStats,
    worker_stats: Vec<WorkerStats>,
    finalization_us: u64,
    descriptor_limit: Option<usize>,
    descriptor_peak: Option<usize>,
}
fn kind(k: &NodeKind) -> u8 {
    match k {
        NodeKind::Directory => 0,
        NodeKind::File => 1,
        NodeKind::Symlink => 2,
        NodeKind::Other => 3,
    }
}
pub(crate) fn encode(w: &mut impl Write, s: &Snapshot, m: &ScanMetadata) -> Result<(), String> {
    if !s.incremental_accounting_eligible()
        || !s.accounting.warnings.is_empty()
        || s.accounting.deduction_bytes != 0
        || s.iter_nodes().any(|n| n.clone_metadata().is_some())
    {
        return Err("Cache requires verified nonshared clone accounting".into());
    }
    if !s.errors.is_empty() || !s.exclusions.is_empty() {
        return Err("Cache cannot contain issues".into());
    }
    let header = serde_json::to_vec(&Header {
        metadata: m.clone(),
        accounting: s.accounting.clone(),
        stats: s.stats.clone(),
        worker_stats: s.worker_stats.clone(),
        finalization_us: s.finalization_us,
        descriptor_limit: s.descriptor_limit,
        descriptor_peak: s.descriptor_peak,
    })
    .map_err(err)?;
    if header.len() > HEADER_LIMIT {
        return Err("Cache header exceeds limit".into());
    }
    w.write_all(MAGIC).map_err(err)?;
    w.write_all(&(header.len() as u32).to_le_bytes())
        .map_err(err)?;
    w.write_all(&header).map_err(err)?;
    w.write_all(&(s.node_count() as u32).to_le_bytes())
        .map_err(err)?;
    let mut paths = s.path_cursor();
    for n in s.iter_nodes() {
        let path = paths.path(n.id)?;
        let path = path.as_os_str().as_bytes();
        let label = n.display_name();
        let name = label.as_bytes();
        if path.len() > FIELD_LIMIT
            || name.len() > FIELD_LIMIT
            || n.children().len() > s.node_count()
            || n.clone_metadata().is_some()
        {
            return Err("Cache node exceeds field limit".into());
        }
        let mut fixed = [0u8; RECORD_BYTES];
        fixed[0..4].copy_from_slice(&n.id.to_le_bytes());
        fixed[4..8].copy_from_slice(&n.parent.unwrap_or(u32::MAX).to_le_bytes());
        for (i, value) in [
            n.identity.device,
            n.identity.inode,
            n.allocated_size,
            n.length,
            n.own_allocated_size,
        ]
        .iter()
        .enumerate()
        {
            fixed[8 + i * 8..16 + i * 8].copy_from_slice(&value.to_le_bytes());
        }
        fixed[48] = kind(&n.identity.kind)
            | (u8::from(n.is_directory) << 2)
            | (u8::from(n.hard_link) << 3)
            | (u8::from(n.excluded) << 4);
        for (i, value) in [path.len(), name.len(), n.children().len()]
            .iter()
            .enumerate()
        {
            fixed[49 + i * 4..53 + i * 4].copy_from_slice(&(*value as u32).to_le_bytes());
        }
        w.write_all(&fixed).map_err(err)?;
        w.write_all(path).map_err(err)?;
        w.write_all(name).map_err(err)?;
        for child in n.children() {
            w.write_all(&child.to_le_bytes()).map_err(err)?;
        }
    }
    Ok(())
}
fn read_u32(r: &mut impl Read) -> Result<u32, String> {
    let mut b = [0; 4];
    r.read_exact(&mut b).map_err(err)?;
    Ok(u32::from_le_bytes(b))
}
/// Reads exactly `count` bytes into `out`, growing only with bytes actually
/// present: a hostile length on a truncated stream never reserves `count`.
fn read_bytes(
    r: &mut impl Read,
    out: &mut Vec<u8>,
    count: usize,
    limit: usize,
) -> Result<(), String> {
    if count > limit {
        return Err("Cache field exceeds limit".into());
    }
    out.clear();
    if count <= FIELD_LIMIT {
        // Small fields: a bounded reserve keeps the hot path to one read.
        out.resize(count, 0);
        return r.read_exact(out).map_err(err);
    }
    r.take(count as u64).read_to_end(out).map_err(err)?;
    if out.len() != count {
        return Err("Cache field is truncated".into());
    }
    Ok(())
}
fn bytes(r: &mut impl Read, count: usize, limit: usize) -> Result<Vec<u8>, String> {
    let mut b = Vec::new();
    read_bytes(r, &mut b, count, limit)?;
    Ok(b)
}
pub(crate) fn decode(
    r: &mut impl Read,
    node_limit: usize,
) -> Result<(Snapshot, ScanMetadata), String> {
    let mut magic = [0; 8];
    r.read_exact(&mut magic).map_err(err)?;
    if &magic != MAGIC {
        return Err("Unsupported cache codec".into());
    }
    let header_len = read_u32(r)? as usize;
    let h: Header = serde_json::from_slice(&bytes(r, header_len, HEADER_LIMIT)?).map_err(err)?;
    if h.accounting.model != disk_scan::AccountingModel::FullClonesV1
        || !h.accounting.checks_complete
        || h.accounting.shared
        || !h.accounting.warnings.is_empty()
        || h.accounting.deduction_bytes != 0
    {
        return Err("Cache requires verified nonshared clone accounting".into());
    }
    let mut snapshot = read_nodes(r, node_limit)?;
    snapshot.stats = h.stats;
    snapshot.accounting = h.accounting;
    snapshot.state = ScanState::Completed;
    snapshot.worker_stats = h.worker_stats;
    snapshot.finalization_us = h.finalization_us;
    snapshot.descriptor_limit = h.descriptor_limit;
    snapshot.descriptor_peak = h.descriptor_peak;
    Ok((snapshot, h.metadata))
}
/// Reads the node count, every record and the end of stream.
fn read_nodes(r: &mut impl Read, node_limit: usize) -> Result<Snapshot, String> {
    let count = read_u32(r)? as usize;
    if count == 0 || count > node_limit {
        return Err("Cache exceeds node limit or has no root".into());
    }
    // Never reserve an untrusted count: a truncated stream allocates at most a bounded record.
    let mut builder = LegacyTreeBuilder::new(std::path::PathBuf::new());
    // One transient wire record owns reusable buffers. The builder copies all
    // retained data, including exact sparse overrides, before the next read.
    let mut node = Node {
        id: 0,
        parent: None,
        children: Vec::new(),
        name: String::new(),
        path: std::path::PathBuf::new(),
        identity: Identity {
            device: 0,
            inode: 0,
            kind: NodeKind::Other,
        },
        allocated_size: 0,
        length: 0,
        own_allocated_size: 0,
        is_directory: false,
        hard_link: false,
        excluded: false,
        clone_metadata: None,
    };
    let mut children_left = count - 1;
    for _ in 0..count {
        let mut fixed = [0; RECORD_BYTES];
        r.read_exact(&mut fixed).map_err(err)?;
        let u32_at = |offset| u32::from_le_bytes(fixed[offset..offset + 4].try_into().unwrap());
        let u64_at = |offset| u64::from_le_bytes(fixed[offset..offset + 8].try_into().unwrap());
        let flags = fixed[48];
        if flags & !31 != 0
            || (flags & 4 != 0) != (flags & 3 == 0)
            || (flags & 8 != 0 && flags & 3 != 1)
        {
            return Err("Invalid cache node flags".into());
        }
        let path_count = u32_at(49) as usize;
        let name_count = u32_at(53) as usize;
        let child_count = u32_at(57) as usize;
        if path_count > FIELD_LIMIT || name_count > FIELD_LIMIT {
            return Err("Cache field exceeds limit".into());
        }
        if child_count > children_left {
            return Err("Cache child count exceeds topology limit".into());
        }
        children_left -= child_count;
        let mut path = std::mem::take(&mut node.path).into_os_string().into_vec();
        read_bytes(r, &mut path, path_count, FIELD_LIMIT)?;
        node.path = OsString::from_vec(path).into();
        let mut name = std::mem::take(&mut node.name).into_bytes();
        read_bytes(r, &mut name, name_count, FIELD_LIMIT)?;
        node.name = String::from_utf8(name).map_err(err)?;
        node.children.clear();
        // Grow only from actual decoded child IDs, never an untrusted reserve.
        for _ in 0..child_count {
            node.children.push(read_u32(r)?);
        }
        let parent = u32_at(4);
        node.id = u32_at(0);
        node.parent = if parent == u32::MAX {
            None
        } else {
            Some(parent)
        };
        node.identity = Identity {
            device: u64_at(8),
            inode: u64_at(16),
            kind: match flags & 3 {
                0 => NodeKind::Directory,
                1 => NodeKind::File,
                2 => NodeKind::Symlink,
                _ => NodeKind::Other,
            },
        };
        node.allocated_size = u64_at(24);
        node.length = u64_at(32);
        node.own_allocated_size = u64_at(40);
        node.is_directory = flags & 4 != 0;
        node.hard_link = flags & 8 != 0;
        node.excluded = flags & 16 != 0;
        builder.append_ref(&node)?;
    }
    let mut extra = [0];
    if r.read(&mut extra).map_err(err)? != 0 {
        return Err("Cache contains trailing decoded data".into());
    }
    Ok(builder.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};
    fn fixture() -> (tempfile::TempDir, Snapshot, ScanMetadata) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("nested")).unwrap();
        std::fs::write(d.path().join("nested/file"), b"hello").unwrap();
        let mut s = disk_scan::scan(
            d.path().into(),
            disk_scan::ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        // macOS filesystems reject invalid UTF-8 names; the wire format still
        // preserves native paths in imported/in-memory snapshots.
        let id = s.iter_nodes().find(|n| !n.is_directory).unwrap().id;
        let mut n = s.legacy_node(id).unwrap();
        n.path = d
            .path()
            .join("nested")
            .join(OsString::from_vec(b"native-\xff".to_vec()));
        s.replace_node(id, n).unwrap();
        let m = ScanMetadata {
            started_at_ms: 1,
            completed_at_ms: 2,
            workers: 1,
            worker_mode: "auto".into(),
            worker_source: "hardware".into(),
            qos: disk_scan::ScanQos::Automatic,
        };
        (d, s, m)
    }
    #[test]
    fn old_magic_and_missing_accounting_evidence_are_rejected() {
        let (_directory, snapshot, metadata) = fixture();
        let mut encoded = Vec::new();
        encode(&mut encoded, &snapshot, &metadata).unwrap();
        for magic in [b"HAHCAC02", b"HAHCAC03"] {
            let mut old = encoded.clone();
            old[..8].copy_from_slice(magic);
            assert_eq!(
                decode(&mut old.as_slice(), 8_000_000).unwrap_err(),
                "Unsupported cache codec"
            );
        }
        let length = u32::from_le_bytes(encoded[8..12].try_into().unwrap()) as usize;
        let mut header: serde_json::Value =
            serde_json::from_slice(&encoded[12..12 + length]).unwrap();
        header.as_object_mut().unwrap().remove("accounting");
        let header = serde_json::to_vec(&header).unwrap();
        let mut missing = MAGIC.to_vec();
        missing.extend_from_slice(&(header.len() as u32).to_le_bytes());
        missing.extend_from_slice(&header);
        missing.extend_from_slice(&encoded[12 + length..]);
        assert!(decode(&mut missing.as_slice(), 8_000_000).is_err());
    }

    #[test]
    fn compact_roundtrip_preserves_native_paths_and_all_snapshot_fields() {
        let (_d, s, m) = fixture();
        let mut b = vec![];
        encode(&mut b, &s, &m).unwrap();
        let (decoded, metadata) = decode(&mut b.as_slice(), 8_000_000).unwrap();
        let mut roundtrip = Vec::new();
        encode(&mut roundtrip, &decoded, &metadata).unwrap();
        assert_eq!(roundtrip, b); // Existing HAHCAC04 bytes and field ordering.
                                  // Snapshot's generic JSON cannot represent invalid UTF-8 paths; compare
                                  // native bytes independently, then compare every other serialized field.
        let normalized = |s: Snapshot| {
            let mut nodes: Vec<_> = s
                .iter_nodes()
                .map(|n| s.legacy_node(n.id).unwrap())
                .collect();
            let paths: Vec<_> = nodes.iter().map(|n| n.path.clone()).collect();
            for n in &mut nodes {
                n.path = "/normalized".into();
            }
            (
                paths,
                serde_json::to_value(nodes).unwrap(),
                serde_json::to_value(&s.stats).unwrap(),
                serde_json::to_value(&s.worker_stats).unwrap(),
                s.finalization_us,
                s.descriptor_limit,
                s.descriptor_peak,
                s.state,
            )
        };
        assert_eq!(normalized(decoded), normalized(s));
        assert_eq!(
            serde_json::to_value(metadata).unwrap(),
            serde_json::to_value(m).unwrap()
        );
    }
    #[test]
    fn nr1p2_cache_root_spellings_preserve_canonical_and_variant_children() {
        let (_directory, mut snapshot, metadata) = fixture();
        // The codec-only fixture intentionally has a historical noncanonical
        // path. Anchor its file at the recorded parent for this topology test.
        let file_id = snapshot.iter_nodes().find(|n| !n.is_directory).unwrap().id;
        let mut file = snapshot.legacy_node(file_id).unwrap();
        file.path = snapshot
            .path(file.parent.unwrap())
            .unwrap()
            .join(OsString::from_vec(b"native-\xff".to_vec()));
        snapshot.replace_node(file_id, file).unwrap();
        crate::cache_store::validate(&snapshot, 8_000_000).unwrap();
        let original_root = snapshot.root.as_os_str().as_bytes();
        for suffix in [b"//".as_slice(), b"/./".as_slice()] {
            for variant_children in [false, true] {
                let mut root_bytes = original_root.to_vec();
                root_bytes.extend_from_slice(suffix);
                let recorded_root =
                    std::path::PathBuf::from(OsString::from_vec(root_bytes.clone()));
                // Original cache validation compares Path components. The wire
                // has no separate root field: decode derives it from node zero.
                assert_eq!(recorded_root, snapshot.root);
                let mut compact = Snapshot::empty(snapshot.root.clone());
                let mut expected = Vec::new();
                for view in snapshot.iter_nodes() {
                    let mut node = snapshot.legacy_node(view.id).unwrap();
                    if node.id == 0 {
                        node.path = recorded_root.clone();
                    } else if variant_children {
                        let bytes = node.path.as_os_str().as_bytes();
                        let mut raw = root_bytes.clone();
                        raw.extend_from_slice(&bytes[original_root.len()..]);
                        node.path = OsString::from_vec(raw).into();
                    }
                    expected.push(node.path.as_os_str().as_bytes().to_vec());
                    compact.append_legacy_node(node).unwrap();
                }
                compact.stats = snapshot.stats.clone();
                compact.accounting = snapshot.accounting.clone();
                compact.state = snapshot.state;
                compact.worker_stats = snapshot.worker_stats.clone();
                compact.finalization_us = snapshot.finalization_us;
                compact.descriptor_limit = snapshot.descriptor_limit;
                compact.descriptor_peak = snapshot.descriptor_peak;
                crate::cache_store::validate(&compact, 8_000_000).unwrap();
                let mut encoded = Vec::new();
                encode(&mut encoded, &compact, &metadata).unwrap();
                let (decoded, metadata) = decode(&mut encoded.as_slice(), 8_000_000).unwrap();
                crate::cache_store::validate(&decoded, 8_000_000).unwrap();
                assert_eq!(decoded.root.as_os_str().as_bytes(), root_bytes);
                for (id, path) in expected.iter().enumerate() {
                    assert_eq!(
                        decoded.path(id as u32).unwrap().as_os_str().as_bytes(),
                        path
                    );
                }
                let mut reencoded = Vec::new();
                encode(&mut reencoded, &decoded, &metadata).unwrap();
                assert_eq!(reencoded, encoded);
            }
        }
    }

    // Frozen HAHCAC04 writer oracle uses owned legacy DTOs, never the compact
    // encoder or its fixed-record offsets, so decoder/encoder changes cannot
    // manufacture matching expectations from each other.
    fn legacy_cache_bytes(snapshot: &Snapshot, metadata: &ScanMetadata, nodes: &[Node]) -> Vec<u8> {
        #[derive(Serialize)]
        struct LegacyHeader<'a> {
            metadata: &'a ScanMetadata,
            accounting: &'a AccountingInfo,
            stats: &'a ScanStats,
            worker_stats: &'a [WorkerStats],
            finalization_us: u64,
            descriptor_limit: Option<usize>,
            descriptor_peak: Option<usize>,
        }
        let header = serde_json::to_vec(&LegacyHeader {
            metadata,
            accounting: &snapshot.accounting,
            stats: &snapshot.stats,
            worker_stats: &snapshot.worker_stats,
            finalization_us: snapshot.finalization_us,
            descriptor_limit: snapshot.descriptor_limit,
            descriptor_peak: snapshot.descriptor_peak,
        })
        .unwrap();
        let mut bytes = b"HAHCAC04".to_vec();
        bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
        for node in nodes {
            bytes.extend_from_slice(&node.id.to_le_bytes());
            bytes.extend_from_slice(&node.parent.unwrap_or(u32::MAX).to_le_bytes());
            for value in [
                node.identity.device,
                node.identity.inode,
                node.allocated_size,
                node.length,
                node.own_allocated_size,
            ] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            let kind = match node.identity.kind {
                NodeKind::Directory => 0,
                NodeKind::File => 1,
                NodeKind::Symlink => 2,
                NodeKind::Other => 3,
            };
            bytes.push(
                kind | (u8::from(node.is_directory) << 2)
                    | (u8::from(node.hard_link) << 3)
                    | (u8::from(node.excluded) << 4),
            );
            for count in [
                node.path.as_os_str().as_bytes().len(),
                node.name.len(),
                node.children.len(),
            ] {
                bytes.extend_from_slice(&(count as u32).to_le_bytes());
            }
            bytes.extend_from_slice(node.path.as_os_str().as_bytes());
            bytes.extend_from_slice(node.name.as_bytes());
            for child in &node.children {
                bytes.extend_from_slice(&child.to_le_bytes());
            }
        }
        bytes
    }
    #[test]
    fn reused_wire_buffers_match_legacy_writer_and_own_sparse_overrides() {
        let (_directory, mut snapshot, metadata) = fixture();
        let mut nodes: Vec<_> = snapshot
            .iter_nodes()
            .map(|node| snapshot.legacy_node(node.id).unwrap())
            .collect();
        let file_id = nodes.iter().position(|node| !node.is_directory).unwrap();
        let parent = nodes[file_id].parent.unwrap() as usize;
        nodes[file_id].path = nodes[parent]
            .path
            .join(OsString::from_vec(b"native-\xff".to_vec()));
        let template = nodes[file_id].clone();
        for (suffix, label) in [
            (OsString::from(".."), "large".repeat(1024)),
            (OsString::from("short"), "short".into()),
            (
                OsString::from_vec(b"last-\xff".to_vec()),
                "last display override".into(),
            ),
        ] {
            let mut node = template.clone();
            node.id = nodes.len() as u32;
            node.identity.inode = 100 + u64::from(node.id);
            node.path = nodes[parent].path.join(suffix);
            node.name = label;
            nodes[parent].children.push(node.id);
            nodes.push(node);
        }
        for id in (0..nodes.len()).rev() {
            if nodes[id].is_directory {
                nodes[id].allocated_size = nodes[id]
                    .children
                    .iter()
                    .fold(nodes[id].own_allocated_size, |sum, &child| {
                        sum.saturating_add(nodes[child as usize].allocated_size)
                    });
            }
        }
        snapshot.stats.entries = nodes.len() as u64;
        snapshot.stats.allocated_bytes = nodes[0].allocated_size;
        let expected = legacy_cache_bytes(&snapshot, &metadata, &nodes);
        let (decoded, decoded_metadata) = decode(&mut expected.as_slice(), 8_000_000).unwrap();
        for node in &nodes {
            let actual = decoded.legacy_node(node.id).unwrap();
            assert_eq!(
                actual.path.as_os_str().as_bytes(),
                node.path.as_os_str().as_bytes()
            );
            assert_eq!(actual.name, node.name);
            assert_eq!(actual.children, node.children);
            assert_eq!(
                serde_json::to_value(Node {
                    path: "/normalized".into(),
                    ..actual
                })
                .unwrap(),
                serde_json::to_value(Node {
                    path: "/normalized".into(),
                    ..node.clone()
                })
                .unwrap()
            );
        }
        crate::cache_store::validate(&decoded, 8_000_000).unwrap();
        assert_eq!(
            serde_json::to_value(decoded_metadata.clone()).unwrap(),
            serde_json::to_value(metadata).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&decoded.stats).unwrap(),
            serde_json::to_value(&snapshot.stats).unwrap()
        );
        let mut reencoded = Vec::new();
        encode(&mut reencoded, &decoded, &decoded_metadata).unwrap();
        assert_eq!(reencoded, expected);
    }

    #[test]
    fn buffering_preserves_bytes_and_propagates_short_write_failures() {
        let (_d, mut s, m) = fixture();
        let node = s.legacy_node((s.node_count() - 1) as u32).unwrap();
        for _ in 0..4096 {
            let mut node = node.clone();
            node.id = s.node_count() as u32;
            s.append_legacy_node(node).unwrap();
        }
        let mut expected = Vec::new();
        encode(&mut expected, &s, &m).unwrap();
        struct Sink {
            bytes: Vec<u8>,
            calls: usize,
            fail_at: Option<usize>,
        }
        impl Write for Sink {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                if self.fail_at == Some(self.bytes.len()) {
                    return Err(std::io::Error::other("injected write failure"));
                }
                let n = b.len().min(777).min(
                    self.fail_at
                        .map_or(usize::MAX, |limit| limit - self.bytes.len()),
                );
                self.bytes.extend_from_slice(&b[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        struct Count {
            bytes: Vec<u8>,
            calls: usize,
        }
        impl Write for Count {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                self.bytes.extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut old = Count {
            bytes: vec![],
            calls: 0,
        };
        let mut batched = Count {
            bytes: vec![],
            calls: 0,
        };
        let mut old_buffered = std::io::BufWriter::with_capacity(8192, &mut old);
        encode(&mut old_buffered, &s, &m).unwrap();
        old_buffered.flush().unwrap();
        drop(old_buffered);
        let mut buffered = std::io::BufWriter::with_capacity(WRITE_BATCH, &mut batched);
        encode(&mut buffered, &s, &m).unwrap();
        buffered.flush().unwrap();
        drop(buffered);
        assert_eq!(old.bytes, batched.bytes);
        assert_eq!(batched.calls, expected.len().div_ceil(WRITE_BATCH));
        assert!(old.calls > batched.calls * 8);
        let mut sink = Sink {
            bytes: vec![],
            calls: 0,
            fail_at: None,
        };
        encode(&mut sink, &s, &m).unwrap();
        assert_eq!(sink.bytes, expected);
        for fail_at in [0, 131072, expected.len() - 1] {
            let mut sink = Sink {
                bytes: vec![],
                calls: 0,
                fail_at: Some(fail_at),
            };
            let mut buffered = std::io::BufWriter::with_capacity(WRITE_BATCH, &mut sink);
            let result = encode(&mut buffered, &s, &m).and_then(|()| buffered.flush().map_err(err));
            assert!(result.unwrap_err().contains("injected"));
            // Avoid best-effort drop retries after the injected write error.
            let _ = buffered.into_parts();
            assert_eq!(sink.bytes, expected[..fail_at]);
        }
    }
    #[test]
    #[ignore = "opt-in bounded codec CPU benchmark"]
    fn codec_write_batch_benchmark() {
        let (_d, mut s, m) = fixture();
        let node = s.legacy_node((s.node_count() - 1) as u32).unwrap();
        let root = s.root.clone();
        for i in 0u32..200_000 {
            let mut node = node.clone();
            node.name = format!("file-{i:08x}-{}", i.wrapping_mul(2654435761u32));
            node.path = root.join("nested").join(&node.name);
            node.identity.inode = u64::from(i) + 1;
            node.id = s.node_count() as u32;
            s.append_legacy_node(node).unwrap();
        }
        if let Some(path) = std::env::var_os("HAH_CODEC_PROBE_CACHE_DIR") {
            let store = crate::cache_store::Store::new(path.into());
            let id = store.list().unwrap().first().unwrap().id.clone();
            s = store.load(&id).unwrap().0;
        }
        let mut old = Vec::new();
        encode(&mut old, &s, &m).unwrap();
        for round in 0..6 {
            for level in if round % 2 == 0 {
                [1, -1, -3]
            } else {
                [-3, -1, 1]
            } {
                let start = std::time::Instant::now();
                let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), level).unwrap();
                encoder.include_checksum(true).unwrap();
                let mut buffered = std::io::BufWriter::with_capacity(WRITE_BATCH, &mut encoder);
                encode(&mut buffered, &s, &m).unwrap();
                buffered.flush().unwrap();
                drop(buffered);
                let compressed = encoder.finish().unwrap();
                eprintln!(
                    "codec level={level} round={round} ms={:.3} bytes={} compressed={}",
                    start.elapsed().as_secs_f64() * 1000.0,
                    old.len(),
                    compressed.len()
                );
                assert_eq!(
                    zstd::stream::decode_all(compressed.as_slice()).unwrap(),
                    old
                );
            }
        }
    }
    #[test]
    fn malformed_counts_fields_flags_truncation_and_trailing_bytes_fail() {
        let (_d, s, m) = fixture();
        let mut valid = vec![];
        encode(&mut valid, &s, &m).unwrap();
        let header_len = u32::from_le_bytes(valid[8..12].try_into().unwrap()) as usize;
        let count_offset = 12 + header_len;
        let record = count_offset + 4;
        for (offset, value) in [
            (8, (HEADER_LIMIT + 1) as u32),
            (count_offset, 8_000_001),
            (record + 49, (FIELD_LIMIT + 1) as u32),
            (record + 53, (FIELD_LIMIT + 1) as u32),
            (record + 57, s.node_count() as u32),
        ] {
            let mut bad = valid.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode(&mut bad.as_slice(), 8_000_000).is_err());
        }
        // Invalid fixed metadata must fail before reading/allocating any path,
        // name or child payload, including when a later field is invalid.
        struct CountReads<'a> {
            data: &'a [u8],
            consumed: usize,
        }
        impl Read for CountReads<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let count = output.len().min(self.data.len());
                output[..count].copy_from_slice(&self.data[..count]);
                self.data = &self.data[count..];
                self.consumed += count;
                Ok(count)
            }
        }
        for (offset, value) in [
            (record + 49, (FIELD_LIMIT + 1) as u32),
            (record + 53, (FIELD_LIMIT + 1) as u32),
            (record + 57, s.node_count() as u32),
        ] {
            let mut bad = valid.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            let mut reader = CountReads {
                data: &bad,
                consumed: 0,
            };
            assert!(decode(&mut reader, 8_000_000).is_err());
            assert_eq!(reader.consumed, record + RECORD_BYTES);
        }
        let path_len =
            u32::from_le_bytes(valid[record + 49..record + 53].try_into().unwrap()) as usize;
        let mut bad_name = valid.clone();
        bad_name[record + RECORD_BYTES + path_len] = 0xff;
        assert!(decode(&mut bad_name.as_slice(), 8_000_000).is_err());
        let mut bad = valid.clone();
        bad[record + 48] = 255;
        assert!(decode(&mut bad.as_slice(), 8_000_000).is_err());
        for flags in [0, 12] {
            // directory bit absent, or hard-link directory
            let mut bad = valid.clone();
            bad[record + 48] = flags;
            assert!(decode(&mut bad.as_slice(), 8_000_000).is_err());
        }
        for len in [0, 7, 12, record, valid.len() - 1] {
            assert!(decode(&mut &valid[..len], 8_000_000).is_err());
        }
        valid.push(0);
        assert!(decode(&mut valid.as_slice(), 8_000_000)
            .unwrap_err()
            .contains("trailing"));
    }
}
