//! Version-three saved-scan archive: a JSON envelope followed by the binary node
//! records of `cache_codec`. A distinct magic keeps archives and private caches
//! from ever being mistaken for each other.
use super::{
    archive_codec, cache_codec, err, issues, native_path, worker_stats, IssueWire, IssuesRef,
    ScanMetadata,
};
use disk_scan::{AccountingInfo, ScanState, ScanStats, Snapshot, WorkerStats};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
};

pub(super) const MAGIC: &[u8; 8] = b"HAHARC03";
// Issue paths are JSON byte arrays (about four characters per byte); the
// retained-size budget of the deserializers bounds what survives decoding.
const HEADER_LIMIT: usize = 256 * 1024 * 1024;

#[derive(Serialize)]
struct HeaderRef<'a> {
    metadata: &'a ScanMetadata,
    root: &'a [u8],
    stats: &'a ScanStats,
    state: ScanState,
    errors: IssuesRef<'a>,
    exclusions: IssuesRef<'a>,
    worker_stats: &'a [WorkerStats],
    finalization_us: u64,
    descriptor_limit: Option<usize>,
    descriptor_peak: Option<usize>,
    accounting: &'a AccountingInfo,
}
#[derive(Deserialize)]
struct Header {
    metadata: ScanMetadata,
    #[serde(deserialize_with = "native_path")]
    root: Vec<u8>,
    stats: ScanStats,
    state: ScanState,
    #[serde(deserialize_with = "issues")]
    errors: Vec<IssueWire>,
    #[serde(deserialize_with = "issues")]
    exclusions: Vec<IssueWire>,
    #[serde(deserialize_with = "worker_stats")]
    worker_stats: Vec<WorkerStats>,
    finalization_us: u64,
    descriptor_limit: Option<usize>,
    descriptor_peak: Option<usize>,
    accounting: AccountingInfo,
}

pub(super) fn encode(w: &mut impl Write, s: &Snapshot, m: &ScanMetadata) -> Result<(), String> {
    let header = serde_json::to_vec(&HeaderRef {
        metadata: m,
        root: s.root.as_os_str().as_bytes(),
        stats: &s.stats,
        state: s.state,
        errors: IssuesRef(&s.errors),
        exclusions: IssuesRef(&s.exclusions),
        worker_stats: &s.worker_stats,
        finalization_us: s.finalization_us,
        descriptor_limit: s.descriptor_limit,
        descriptor_peak: s.descriptor_peak,
        accounting: &s.accounting,
    })
    .map_err(err)?;
    if header.len() > HEADER_LIMIT {
        return Err("Saved scan header exceeds limit".into());
    }
    w.write_all(MAGIC).map_err(err)?;
    w.write_all(&(header.len() as u32).to_le_bytes())
        .map_err(err)?;
    w.write_all(&header).map_err(err)?;
    cache_codec::write_nodes(w, s, archive_codec::MAX_FIELD_BYTES, true)
}

pub(super) fn decode(
    r: &mut impl Read,
    node_limit: usize,
) -> Result<(Snapshot, ScanMetadata), String> {
    let mut magic = [0; 8];
    r.read_exact(&mut magic).map_err(err)?;
    if &magic != MAGIC {
        return Err("Unsupported saved scan codec".into());
    }
    let mut len = [0; 4];
    r.read_exact(&mut len).map_err(err)?;
    let mut raw = Vec::new();
    cache_codec::read_bytes(r, &mut raw, u32::from_le_bytes(len) as usize, HEADER_LIMIT)?;
    let h: Header = serde_json::from_slice(&raw).map_err(err)?;
    drop(raw);
    let mut s = cache_codec::read_nodes(r, node_limit, archive_codec::MAX_FIELD_BYTES, true)?;
    // Same root contract as the JSON archive: node zero may spell the root differently.
    let root: PathBuf = std::ffi::OsString::from_vec(h.root).into();
    if s.root != root {
        return Err("Invalid saved scan root".into());
    }
    s.restore_legacy_root(root)?;
    s.stats = h.stats;
    s.state = h.state;
    s.errors = h.errors.into_iter().map(IssueWire::into_issue).collect();
    s.exclusions = h
        .exclusions
        .into_iter()
        .map(IssueWire::into_issue)
        .collect();
    s.worker_stats = h.worker_stats;
    s.finalization_us = h.finalization_us;
    s.descriptor_limit = h.descriptor_limit;
    s.descriptor_peak = h.descriptor_peak;
    s.accounting = h.accounting;
    Ok((s, h.metadata))
}
