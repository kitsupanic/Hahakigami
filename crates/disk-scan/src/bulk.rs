//! Checked parsing of Darwin's packed getattrlistbulk records. No unaligned references.
#[cfg(target_os = "macos")]
use crate::{Entry, NodeKind};
use std::{io, ops::Range};
#[cfg(target_os = "macos")]
pub(crate) const BULK_BUFFER_BYTES: usize = 64 * 1024;
#[cfg(target_os = "macos")]
const LARGE_BULK_BUFFER_BYTES: usize = 256 * 1024;
pub(crate) fn u32_at(b: &[u8], at: usize) -> io::Result<u32> {
    let bytes = at.checked_add(4).and_then(|end| b.get(at..end));
    Ok(u32::from_ne_bytes(
        bytes.ok_or_else(invalid)?.try_into().unwrap(),
    ))
}
pub(crate) fn u64_at(b: &[u8], at: usize) -> io::Result<u64> {
    let bytes = at.checked_add(8).and_then(|end| b.get(at..end));
    Ok(u64::from_ne_bytes(
        bytes.ok_or_else(invalid)?.try_into().unwrap(),
    ))
}
/// The volume (not one file) cannot answer: the single rule for "unsupported" across listing and probes.
#[cfg(target_os = "macos")]
pub(crate) fn is_unsupported(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Unsupported
        || matches!(
            error.raw_os_error(),
            Some(libc::ENOTSUP | libc::EOPNOTSUPP | libc::EINVAL)
        )
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "malformed getattrlistbulk record",
    )
}
#[derive(Debug, Clone)]
pub(crate) struct ParsedRecord<N> {
    pub(crate) name: N,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) kind: u32,
    pub(crate) length: u64,
    pub(crate) allocated: u64,
    pub(crate) links: u32,
    pub(crate) clone_metadata: Option<crate::CloneMetadata>,
    pub(crate) clone_checked: bool,
}
pub(crate) type Record<'a> = ParsedRecord<&'a [u8]>;
/// The node kind of a record's vnode type (VREG, VDIR, VLNK).
pub(crate) fn node_kind(vtype: u32) -> NodeKind {
    match vtype {
        1 => NodeKind::File,
        2 => NodeKind::Directory,
        5 => NodeKind::Symlink,
        _ => NodeKind::Other,
    }
}
#[cfg(any(target_os = "macos", test))]
type BufferedRecord = ParsedRecord<Range<usize>>;
#[cfg(test)]
fn parse(b: &[u8], count: usize, rich: bool) -> io::Result<Vec<Record<'_>>> {
    parse_with_payload(b, count, rich, &mut 0)
}
fn parse_with_payload<'a>(
    b: &'a [u8],
    count: usize,
    rich: bool,
    used: &mut usize,
) -> io::Result<Vec<Record<'a>>> {
    let mut out = Vec::new();
    parse_into(b, count, rich, used, &mut out, |name, _| name)?;
    Ok(out)
}
#[cfg(any(target_os = "macos", test))]
fn parse_offsets_with_payload(
    b: &[u8],
    count: usize,
    rich: bool,
    used: &mut usize,
    out: &mut Vec<BufferedRecord>,
) -> io::Result<()> {
    parse_into(b, count, rich, used, out, |_, range| range)
}
fn parse_into<'a, N>(
    b: &'a [u8],
    count: usize,
    rich: bool,
    used: &mut usize,
    out: &mut Vec<ParsedRecord<N>>,
    mut map_name: impl FnMut(&'a [u8], Range<usize>) -> N,
) -> io::Result<()> {
    out.clear();
    *used = 0;
    if count > b.len() / 24 {
        return Err(invalid());
    }
    if count > out.capacity() {
        out.reserve_exact(count);
    }
    let parsed = (|| {
        let mut start = 0;
        for _ in 0..count {
            let len = u32_at(b, start)? as usize;
            if len < 24 || !len.is_multiple_of(4) {
                return Err(invalid());
            }
            let end = start.checked_add(len).ok_or_else(invalid)?;
            let r = b.get(start..end).ok_or_else(invalid)?;
            let common = u32_at(r, 4)?;
            let file = u32_at(r, 16)?;
            // Returned attributes appear first; absent attributes are not packed.
            if common != 0x8200000b
                || file & !(if rich { 0x607 } else { 7 }) != 0
                || u32_at(r, 8)? != 0
                || u32_at(r, 12)? != 0
                || u32_at(r, 20)? & !(if rich { 0x1300 } else { 0 }) != 0
            {
                return Err(invalid());
            }
            let reference = 24;
            let offset = u32_at(r, reference)? as i32;
            let name_len = u32_at(r, reference + 4)? as usize;
            // Darwin dev_t is a signed 32-bit field; match fstat sign extension.
            let device = u32_at(r, 32)? as i32 as u64;
            let kind = u32_at(r, 36)?;
            let inode = u64_at(r, 40)?;
            if kind == 1 && file & 7 != 7 {
                return Err(invalid());
            }
            let name_start = (reference as i64)
                .checked_add(offset as i64)
                .filter(|v| *v >= 48)
                .ok_or_else(invalid)? as usize;
            let name_end = name_start.checked_add(name_len).ok_or_else(invalid)?;
            let name = r.get(name_start..name_end).ok_or_else(invalid)?;
            if name.len() < 2
                || name.last() != Some(&0)
                || name[..name.len() - 1]
                    .iter()
                    .any(|byte| matches!(byte, 0 | b'/'))
            {
                return Err(invalid());
            }
            let mut at = 48;
            let links = if file & 1 != 0 {
                let v = u32_at(r, at)?;
                at += 4;
                v
            } else {
                0
            };
            let length = if file & 2 != 0 {
                let v = u64_at(r, at)?;
                if v > i64::MAX as u64 {
                    return Err(invalid());
                }
                at += 8;
                v
            } else {
                0
            };
            let allocated = if file & 4 != 0 {
                let v = u64_at(r, at)?;
                if v > i64::MAX as u64 {
                    return Err(invalid());
                }
                at += 8;
                v
            } else {
                0
            };
            // Omitted fields consume no bytes in Darwin's canonical packed order.
            let ext = u32_at(r, 20)?;
            let mut optional_size = |mask: u32| -> io::Result<Option<u64>> {
                if file & mask == 0 {
                    return Ok(None);
                }
                let value = u64_at(r, at)?;
                at += 8;
                if value > i64::MAX as u64 {
                    return Err(invalid());
                }
                Ok(Some(value))
            };
            let data_length = optional_size(0x200)?;
            let data_allocated = optional_size(0x400)?;
            let clone_id = if ext & 0x100 != 0 {
                let v = u64_at(r, at)?;
                at += 8;
                Some(v)
            } else {
                None
            };
            let flags = if ext & 0x200 != 0 {
                let v = u64_at(r, at)?;
                at += 8;
                Some(v)
            } else {
                None
            };
            let refs = if ext & 0x1000 != 0 {
                let v = u32_at(r, at)?;
                at += 4;
                Some(v)
            } else {
                None
            };
            if data_length.is_some_and(|v| v > length)
                || data_allocated.is_some_and(|v| v > allocated)
            {
                return Err(invalid());
            }
            let clone_checked = rich && kind == 1 && file == 0x607 && ext == 0x1300;
            if kind == 1 && links == 0 {
                return Err(invalid());
            }
            let clone_metadata = if clone_checked && (flags.unwrap() & 1 != 0 || refs.unwrap() > 1)
            {
                Some(crate::CloneMetadata {
                    clone_id: clone_id.unwrap(),
                    data_allocated_size: data_allocated.unwrap(),
                    data_length: data_length.unwrap(),
                    reference_count: refs.unwrap(),
                    may_share_blocks: flags.unwrap() & 1 != 0,
                    verified: false,
                })
            } else {
                None
            };
            if name_start < at {
                return Err(invalid());
            }
            let name = &name[..name.len() - 1];
            if name == b"." || name == b".." {
                return Err(invalid());
            }
            out.push(ParsedRecord {
                name: map_name(name, start + name_start..start + name_end - 1),
                device,
                inode,
                kind,
                length,
                allocated,
                links,
                clone_metadata,
                clone_checked,
            });
            start = end;
        }
        *used = start;
        Ok(())
    })();
    if parsed.is_err() {
        out.clear();
    }
    parsed
}
#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn enumerate_with_accounting(
    path: &std::path::Path,
    cancel: &std::sync::atomic::AtomicBool,
    fd: &std::os::fd::OwnedFd,
    device: u64,
    rich_refused: &std::sync::Mutex<std::collections::HashSet<u64>>,
    warnings: &mut Vec<String>,
    observations: &mut std::collections::VecDeque<(usize, crate::CloneMetadata)>,
    buffer: &mut Vec<u8>,
) -> io::Result<Vec<io::Result<Entry>>> {
    enumerate_remembering(
        path,
        cancel,
        fd,
        device,
        rich_refused,
        warnings,
        observations,
        buffer,
        &mut read_batch,
    )
}
/// Asks for clone fields first, unless this device already refused them in a listing. The memo is the
/// listing's own: the per-file probes' map also records one file's missing reply, which says nothing
/// about the volume's listing.
#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn enumerate_remembering(
    path: &std::path::Path,
    cancel: &std::sync::atomic::AtomicBool,
    fd: &std::os::fd::OwnedFd,
    device: u64,
    rich_refused: &std::sync::Mutex<std::collections::HashSet<u64>>,
    warnings: &mut Vec<String>,
    observations: &mut std::collections::VecDeque<(usize, crate::CloneMetadata)>,
    buffer: &mut Vec<u8>,
    reader: &mut impl FnMut(&std::os::fd::OwnedFd, &mut [u8], bool) -> io::Result<usize>,
) -> io::Result<Vec<io::Result<Entry>>> {
    let rich = !rich_refused.lock().unwrap().contains(&device);
    enumerate_using(
        path,
        cancel,
        fd,
        rich,
        warnings,
        observations,
        buffer,
        &mut |fd, buffer, rich| {
            let batch = reader(fd, buffer, rich);
            if rich && batch.as_ref().is_err_and(is_unsupported) {
                rich_refused.lock().unwrap().insert(device);
            }
            batch
        },
    )
}
#[cfg(target_os = "macos")]
fn read_batch(fd: &std::os::fd::OwnedFd, buffer: &mut [u8], rich: bool) -> io::Result<usize> {
    use std::os::fd::AsRawFd;
    let mut attrs = libc::attrlist {
        bitmapcount: 5,
        reserved: 0,
        commonattr: 0x8200000b,
        volattr: 0,
        dirattr: 0,
        fileattr: if rich { 0x607 } else { 7 },
        forkattr: if rich { 0x1300 } else { 0 },
    };
    let count = unsafe {
        libc::getattrlistbulk(
            fd.as_raw_fd(),
            (&mut attrs as *mut libc::attrlist).cast(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            if rich { 0x21 } else { 1 },
        )
    };
    if count < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(count as usize)
    }
}
#[cfg(target_os = "macos")]
fn reserve_listing_batch<T>(entries: &mut Vec<T>, batch_len: usize) {
    let needed = entries.len().saturating_add(batch_len);
    if needed <= entries.capacity() {
        return;
    }
    if needed <= 16_384 {
        // Match push's small-vector doubling before extend can reserve an exact
        // batch-sized prefix.
        let size = std::mem::size_of::<T>();
        let minimum = if size == 1 {
            8
        } else if size <= 1024 {
            4
        } else {
            1
        };
        let target = needed
            .checked_next_power_of_two()
            .unwrap_or(needed)
            .max(minimum);
        entries.reserve_exact(target.saturating_sub(entries.len()));
        return;
    }
    // Reserve once per validated batch. Only very wide listings grow by 1.5x;
    // ordinary growth avoids extra reallocations in medium-sized directories.
    let target = needed.max(
        entries
            .capacity()
            .saturating_add(entries.capacity().div_ceil(2)),
    );
    entries.reserve_exact(target.saturating_sub(entries.len()));
}
#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn enumerate_using(
    path: &std::path::Path,
    cancel: &std::sync::atomic::AtomicBool,
    fd: &std::os::fd::OwnedFd,
    rich: bool,
    warnings: &mut Vec<String>,
    observations: &mut std::collections::VecDeque<(usize, crate::CloneMetadata)>,
    buffer: &mut Vec<u8>,
    reader: &mut impl FnMut(&std::os::fd::OwnedFd, &mut [u8], bool) -> io::Result<usize>,
) -> io::Result<Vec<io::Result<Entry>>> {
    use std::os::unix::ffi::OsStrExt;
    // A scan worker owns this bounded buffer across directory listings.
    buffer.resize(BULK_BUFFER_BYTES, 0);
    let mut entries = vec![];
    let mut records = Vec::new();
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let mut used = 0;
        let parsed = match reader(fd, buffer, rich) {
            Ok(count) => parse_offsets_with_payload(buffer, count, rich, &mut used, &mut records),
            Err(error) => {
                records.clear();
                Err(error)
            }
        };
        match parsed {
            Ok(()) => {}
            // EINTR is the caller's `retry::interrupted` to relist; recovering here
            // would drop clone data and add a spurious warning.
            // A dataless folder fails the plain listing too; it is no accounting gap (`dataless`).
            Err(error)
                if rich
                    && error.kind() != io::ErrorKind::Interrupted
                    && !crate::dataless(&error) =>
            {
                let unsupported = is_unsupported(&error);
                let expected = crate::descriptor::identity_fd(fd);
                observations.clear();
                drop(records);
                return recover_listing(entries, error, unsupported, warnings, |warnings| {
                    let fresh = crate::descriptor::reopen(fd, &expected?)?;
                    enumerate_using(
                        path,
                        cancel,
                        &fresh,
                        false,
                        warnings,
                        observations,
                        buffer,
                        reader,
                    )
                });
            }
            Err(error) => {
                drop(records);
                return crate::retry::terminal_listing(entries, error);
            }
        };
        if records.is_empty() {
            break;
        }
        let base = entries.len();
        reserve_listing_batch(&mut entries, records.len());
        entries.extend(records.iter().enumerate().map(|(index, r)| {
            let kind = node_kind(r.kind);
            let name = std::ffi::OsStr::from_bytes(&buffer[r.name.clone()]);
            // Bulk IDs at firmlinks/mount points can describe the covered namespace inode.
            // Resolve directory metadata relative to the held descriptor before queueing it.
            if matches!(kind, NodeKind::Symlink | NodeKind::Directory) {
                return crate::entry_result(crate::descriptor::entry_at(fd, path, name), || {
                    crate::child_path(path, name)
                });
            }
            if let Some(metadata) = &r.clone_metadata {
                observations.push_back((base + index, metadata.clone()));
            }
            Ok(Entry {
                // Only the basename is retained in the compact tree. Preserve
                // native bytes without repeating the listing's directory prefix.
                path: crate::EntryPath::from_native(name),
                hard_link: kind == NodeKind::File && r.links > 1,
                clone_checked: r.clone_checked,
                device: r.device,
                inode: r.inode,
                kind,
                length: r.length,
                allocated: r.allocated,
            })
        }));
        grow_packed_buffer(buffer, used);
    }
    drop(records);
    Ok(entries)
}
#[cfg(target_os = "macos")]
fn recover_listing(
    entries: Vec<io::Result<Entry>>,
    error: io::Error,
    unsupported: bool,
    warnings: &mut Vec<String>,
    fallback: impl FnOnce(&mut Vec<String>) -> io::Result<Vec<io::Result<Entry>>>,
) -> io::Result<Vec<io::Result<Entry>>> {
    // Successful rich records are private and must not survive the restart.
    // Permanent entry failures remain visible even if the baseline later succeeds.
    let mut permanent: Vec<_> = entries
        .into_iter()
        .filter(|e| {
            e.as_ref()
                .is_err_and(|e| e.kind() != io::ErrorKind::Interrupted)
        })
        .collect();
    match fallback(warnings) {
        Ok(mut baseline) => {
            // Only a plain listing that worked lost clone evidence; one that fails too is an ordinary scan error.
            if !unsupported {
                warnings.push(format!("clone bulk accounting unavailable: {error}"));
                for e in baseline.iter_mut().filter_map(|e| e.as_mut().ok()) {
                    if e.kind == NodeKind::File {
                        e.clone_checked = true;
                    }
                }
            }
            // The baseline lists the whole folder again and reports the same per-entry
            // failures; keep one error per path.
            let again: std::collections::HashSet<_> = baseline
                .iter()
                .filter_map(|e| e.as_ref().err().and_then(crate::entry_error_path))
                .collect();
            permanent.retain(|e| {
                !e.as_ref()
                    .err()
                    .and_then(crate::entry_error_path)
                    .is_some_and(|path| again.contains(path))
            });
            permanent.append(&mut baseline);
            Ok(permanent)
        }
        Err(error) => crate::retry::terminal_listing(permanent, error),
    }
}
/// Owned batch payload diagnostics; these do not measure allocator or RSS usage.
#[cfg(target_os = "macos")]
#[derive(Default)]
pub(crate) struct CloneReadStats {
    pub queries: u64,
    buffer: Vec<u8>,
}
#[cfg(target_os = "macos")]
impl CloneReadStats {
    /// Reuses a caller-owned read buffer; hand it back with `into_buffer`.
    pub(crate) fn with_buffer(buffer: Vec<u8>) -> Self {
        Self {
            buffer,
            ..Self::default()
        }
    }
    pub(crate) fn into_buffer(self) -> Vec<u8> {
        self.buffer
    }
}
/// `rich` asks for the clone fields too; without them the records carry names, kinds and sizes only,
/// which every volume answers (HFS+, exFAT).
#[cfg(target_os = "macos")]
pub(crate) fn visit_clone_records(
    fd: &std::os::fd::OwnedFd,
    cancel: &std::sync::atomic::AtomicBool,
    stats: &mut CloneReadStats,
    rich: bool,
    mut visitor: impl FnMut(&Record) -> io::Result<()>,
) -> io::Result<()> {
    visit_records_with_reader(
        cancel,
        stats,
        rich,
        |buffer| read_batch(fd, buffer, rich),
        &mut visitor,
    )
}
#[cfg(all(test, target_os = "macos"))]
fn visit_with_reader(
    cancel: &std::sync::atomic::AtomicBool,
    stats: &mut CloneReadStats,
    reader: impl FnMut(&mut [u8]) -> io::Result<usize>,
    visitor: &mut impl FnMut(&Record) -> io::Result<()>,
) -> io::Result<()> {
    visit_records_with_reader(cancel, stats, true, reader, visitor)
}
#[cfg(target_os = "macos")]
fn visit_records_with_reader(
    cancel: &std::sync::atomic::AtomicBool,
    stats: &mut CloneReadStats,
    rich: bool,
    mut reader: impl FnMut(&mut [u8]) -> io::Result<usize>,
    visitor: &mut impl FnMut(&Record) -> io::Result<()>,
) -> io::Result<()> {
    let mut buffer = std::mem::take(&mut stats.buffer);
    buffer.resize(BULK_BUFFER_BYTES, 0);
    let result = (|| loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(io::Error::from_raw_os_error(libc::EINTR));
        }
        stats.queries += 1;
        // No retry on this descriptor: the kernel may have advanced the offset, so an EINTR reaches
        // the caller (cancellation is checked above), like the listing path, which relists on a fresh one.
        let count = reader(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        let mut used = 0;
        let records = parse_with_payload(&buffer, count, rich, &mut used)?;
        for record in &records {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(io::Error::from_raw_os_error(libc::EINTR));
            }
            visitor(record)?;
        }
        drop(records);
        grow_packed_buffer(&mut buffer, used);
    })();
    stats.buffer = buffer;
    result
}
#[cfg(target_os = "macos")]
fn grow_packed_buffer(buffer: &mut Vec<u8>, used: usize) {
    if buffer.len() < LARGE_BULK_BUFFER_BYTES && used >= buffer.len() / 4 * 3 {
        buffer.resize(LARGE_BULK_BUFFER_BYTES, 0);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn record_view<N: std::fmt::Debug>(record: &ParsedRecord<N>, name: &[u8]) -> String {
        format!(
            "{:?}",
            (
                name,
                record.device,
                record.inode,
                record.kind,
                record.length,
                record.allocated,
                record.links,
                &record.clone_metadata,
                record.clone_checked,
            )
        )
    }
    /// Both parsers must agree on every byte; returns the shared outcome for golden comparison.
    fn parse_outcome(bytes: &[u8], count: usize, rich: bool) -> String {
        let (mut new_used, mut offset_used) = (99, 99);
        let borrowed = parse_with_payload(bytes, count, rich, &mut new_used);
        let mut offsets = Vec::new();
        let buffered =
            parse_offsets_with_payload(bytes, count, rich, &mut offset_used, &mut offsets);
        assert_eq!(new_used, offset_used);
        match (borrowed, buffered) {
            (Ok(borrowed), Ok(())) => {
                assert_eq!(borrowed.len(), offsets.len());
                let mut views = Vec::new();
                for (new, offset) in borrowed.iter().zip(&offsets) {
                    let current_name = bytes
                        .get(offset.name.clone())
                        .expect("checked absolute name range");
                    assert_eq!(
                        record_view(new, new.name),
                        record_view(offset, current_name)
                    );
                    assert_eq!(new.name.as_ptr(), current_name.as_ptr());
                    views.push(record_view(new, new.name));
                }
                format!("ok {new_used} {views:?}")
            }
            (Err(new), Err(buffered)) => {
                assert_eq!(
                    (new.kind(), new.to_string()),
                    (buffered.kind(), buffered.to_string())
                );
                assert!(
                    offsets.is_empty(),
                    "failed parse retained stale or partial ranges"
                );
                format!("err {new_used} {:?} {new}", new.kind())
            }
            _ => panic!("borrowed and offset parser acceptance differs"),
        }
    }

    #[test]
    fn parsers_agree_and_reproduce_golden_bytes_and_diagnostics() {
        // Captured from the byte-exact reference parser that the shared generic parser replaced.
        let (mut digest, mut total, mut accepted) = (crate::DIGEST_START, 0, 0);
        let mut check = |bytes: &[u8], count: usize, rich: bool| {
            let outcome = parse_outcome(bytes, count, rich);
            total += 1;
            accepted += usize::from(outcome.starts_with("ok"));
            crate::fold_digest(&mut digest, &outcome);
        };
        let files = [
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            0x200,
            0x207,
            0x400,
            0x407,
            0x600,
            0x607,
            8,
            u32::MAX,
        ];
        let extended = [
            0,
            0x100,
            0x200,
            0x1000,
            0x300,
            0x1100,
            0x1200,
            0x1300,
            1,
            u32::MAX,
        ];
        for rich in [false, true] {
            for kind in [1, 2, 5, 4] {
                for file in files {
                    for ext in extended {
                        let record = rich_record(file, ext, kind);
                        check(&record, 1, rich);
                    }
                }
            }
        }
        for rich in [false, true] {
            let original = rich_record(
                if rich { 0x607 } else { 7 },
                if rich { 0x1300 } else { 0 },
                1,
            );
            for end in 0..=original.len() {
                check(&original[..end], 1, rich);
            }
            for at in 0..original.len() {
                for value in [0, 1, 0x7f, 0x80, 0xff] {
                    let mut changed = original.clone();
                    changed[at] = value;
                    check(&changed, 1, rich);
                }
            }
            for position in 0..3 {
                for byte in 0..=255 {
                    let mut changed = original.clone();
                    let index = changed.len() - 4 + position;
                    changed[index] = byte;
                    check(&changed, 1, rich);
                }
            }
            for name in [
                b".\0".as_slice(),
                b"..\0",
                b"a/\0",
                b"a\0b\0",
                "日\0".as_bytes(),
            ] {
                let mut changed = original.clone();
                let start = changed.len() - 4;
                changed[28..32].copy_from_slice(&(name.len() as u32).to_ne_bytes());
                changed[start..start + name.len()].copy_from_slice(name);
                check(&changed, 1, rich);
            }
            let mut pair = original.clone();
            pair.extend_from_slice(&original);
            check(&pair, 2, rich);
            pair[original.len()..original.len() + 4].fill(0);
            check(&pair, 2, rich);
            check(&original, 0, rich);
            check(&original, usize::MAX, rich);
        }
        assert_eq!(
            (total, accepted, digest),
            (3916, 2402, 0x06db_6750_3508_e545)
        );
    }

    #[test]
    fn offset_scratch_reuses_capacity_across_buffer_replacement_growth_errors_and_eof() {
        let mut scratch = Vec::new();
        let mut buffer = Vec::new();
        let mut growths = 0;
        let record_len = rich_record(0x607, 0x1300, 1).len();
        for (round, count) in [0, 1, 10, 600, 3, 0, 2184, 1, 0, 2].into_iter().enumerate() {
            let mut replacement = Vec::new();
            for index in 0..count {
                let mut record = rich_record(0x607, 0x1300, 1);
                let name_start = record.len() - 4;
                record[name_start] = b'a' + round as u8;
                record[40..48]
                    .copy_from_slice(&(round as u64 * 10000 + index as u64).to_ne_bytes());
                replacement.extend_from_slice(&record);
            }
            if round % 2 == 0 {
                buffer.clear();
                buffer.extend_from_slice(&replacement);
            } else {
                buffer = replacement;
            }
            let old_capacity = scratch.capacity();
            let old_pointer = scratch.as_ptr();
            if count > old_capacity {
                growths += 1;
            }
            let mut used = 99;
            parse_offsets_with_payload(&buffer, count, true, &mut used, &mut scratch).unwrap();
            assert_eq!(used, count * record_len);
            assert_eq!(scratch.len(), count);
            if count <= old_capacity {
                assert_eq!(scratch.capacity(), old_capacity);
                assert_eq!(scratch.as_ptr(), old_pointer);
            }
            for (index, record) in scratch.iter().enumerate() {
                let name = buffer.get(record.name.clone()).unwrap();
                let expected = Record {
                    name: &[b'a' + round as u8, b'b', b'c'],
                    device: 0,
                    inode: round as u64 * 10000 + index as u64,
                    kind: 1,
                    length: 128,
                    allocated: 1024,
                    links: 1,
                    clone_metadata: Some(crate::CloneMetadata {
                        clone_id: 42,
                        data_allocated_size: 512,
                        data_length: 123,
                        reference_count: 2,
                        may_share_blocks: true,
                        verified: false,
                    }),
                    clone_checked: true,
                };
                assert_eq!(
                    record_view(record, name),
                    record_view(&expected, expected.name)
                );
            }
            // The numeric ranges do not borrow the old backing allocation.
            buffer.fill(0xff);
            let pointer = scratch.as_ptr();
            let capacity = scratch.capacity();
            assert!(
                parse_offsets_with_payload(&buffer, usize::MAX, true, &mut used, &mut scratch)
                    .is_err()
            );
            assert!(scratch.is_empty());
            assert_eq!(used, 0);
            assert_eq!((scratch.as_ptr(), scratch.capacity()), (pointer, capacity));
            parse_offsets_with_payload(&buffer, 0, true, &mut used, &mut scratch).unwrap();
            assert!(scratch.is_empty());
            assert_eq!((scratch.as_ptr(), scratch.capacity()), (pointer, capacity));
        }
        assert_eq!(growths, 4);
        assert_eq!(scratch.capacity(), 2184);
        assert!(std::mem::size_of::<Record<'_>>() <= 96);
        assert!(std::mem::size_of::<BufferedRecord>() <= 96);
        assert!(std::mem::size_of::<crate::tree::Record>() <= 72);
        assert!(std::mem::size_of::<io::Result<crate::Entry>>() <= 64);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn batch_extension_across_failures_and_restarts() {
        use std::{
            collections::VecDeque,
            os::unix::{ffi::OsStrExt, fs::symlink},
            sync::atomic::{AtomicBool, Ordering},
        };
        #[derive(Clone)]
        enum Step {
            Batch(Vec<(u32, [u8; 4])>),
            Malformed,
            Error(i32),
            Cancel(Vec<(u32, [u8; 4])>),
        }
        fn error_key(error: &io::Error) -> (String, Option<i32>, String) {
            let native = crate::native_entry_error(error);
            (
                format!("{:?}", native.kind()),
                native.raw_os_error(),
                error.to_string(),
            )
        }
        // Entries resolved from the filesystem (directories, symlinks) carry host-specific ids.
        fn outcome(result: io::Result<Vec<io::Result<Entry>>>) -> String {
            match result {
                Err(error) => format!("terminal {:?}", error_key(&error)),
                Ok(entries) => entries
                    .into_iter()
                    .map(|entry| match entry {
                        Err(error) => format!("error {:?}", error_key(&error)),
                        Ok(entry)
                            if matches!(entry.kind, NodeKind::Directory | NodeKind::Symlink) =>
                        {
                            format!(
                                "{:?} {:?}",
                                std::path::Path::new(entry.path.as_os_str()).file_name(),
                                entry.kind
                            )
                        }
                        Ok(entry) => format!(
                            "{:?} {:?} {} {} {} {} {} {}",
                            entry.path.as_os_str().as_bytes(),
                            entry.kind,
                            entry.device,
                            entry.inode,
                            entry.length,
                            entry.allocated,
                            entry.hard_link,
                            entry.clone_checked,
                        ),
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
            }
        }
        let path = std::env::temp_dir().join(format!("bulk-batch-oracle-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::create_dir(path.join("dir")).unwrap();
        symlink("dir", path.join("lnk")).unwrap();
        let fd = std::fs::File::open(&path).unwrap().into();
        let file = (1, *b"f\xffl\0");
        let missing = (2, *b"bad\0");
        let mixed = vec![
            file,
            missing,
            (2, *b"dir\0"),
            (5, *b"lnk\0"),
            (4, *b"oth\0"),
            file,
        ];
        let mut cases = Vec::new();
        cases.push((true, vec![Step::Batch(mixed.clone()), Step::Batch(vec![])]));
        for terminal in [libc::EIO, libc::EINVAL, libc::ENOTSUP] {
            cases.push((
                true,
                vec![
                    Step::Batch(vec![file, missing, file]),
                    Step::Error(terminal),
                    Step::Batch(vec![file, missing, file]),
                    Step::Batch(vec![]),
                ],
            ));
        }
        cases.push((
            true,
            vec![
                Step::Batch(vec![file, missing]),
                Step::Malformed,
                Step::Batch(vec![file]),
                Step::Batch(vec![]),
            ],
        ));
        cases.push((
            true,
            vec![Step::Cancel(mixed.clone()), Step::Error(libc::EIO)],
        ));
        cases.push((
            false,
            vec![Step::Batch(vec![file]), Step::Error(libc::EINTR)],
        ));
        // The bulk-resolved file `f\xffl` (inode 100 + its index) and the failed `bad` lookup.
        let file = |inode: u64, checked: bool| {
            format!("[102, 255, 108] File 0 {inode} 128 1024 false {checked}")
        };
        let gone = "error (\"NotFound\", Some(2), \"No such file or directory (os error 2)\")";
        let mixed_entries = [
            file(100, true).as_str(),
            gone,
            "Some(\"dir\") Directory",
            "Some(\"lnk\") Symlink",
            "[111, 116, 104] Other 0 104 128 1024 false false",
            file(105, true).as_str(),
        ]
        .join("; ");
        let metadata = crate::CloneMetadata {
            clone_id: 42,
            data_allocated_size: 512,
            data_length: 123,
            reference_count: 2,
            may_share_blocks: true,
            verified: false,
        };
        let observed = |indexes: &[usize]| {
            indexes
                .iter()
                .map(|index| (*index, metadata.clone()))
                .collect::<Vec<_>>()
        };
        // A failed rich read restarts on a fresh descriptor without extended attributes.
        let restarted = [(true, BULK_BUFFER_BYTES), (true, BULK_BUFFER_BYTES)]
            .into_iter()
            .chain([(false, BULK_BUFFER_BYTES), (false, BULK_BUFFER_BYTES)])
            .collect::<Vec<_>>();
        let unsupported = |clone_checked| {
            (
                [
                    file(100, clone_checked).as_str(),
                    gone,
                    file(102, clone_checked).as_str(),
                ]
                .join("; "),
                restarted.clone(),
                Vec::<String>::new(),
                observed(&[]),
                BULK_BUFFER_BYTES,
                false,
            )
        };
        let expected = [
            (
                mixed_entries.clone(),
                vec![(true, BULK_BUFFER_BYTES), (true, BULK_BUFFER_BYTES)],
                vec![],
                observed(&[0, 5]),
                BULK_BUFFER_BYTES,
                false,
            ),
            // EIO is reported as a warning; unsupported errors are expected and silent.
            (
                unsupported(true).0,
                restarted.clone(),
                vec!["clone bulk accounting unavailable: Input/output error (os error 5)".into()],
                observed(&[]),
                BULK_BUFFER_BYTES,
                false,
            ),
            unsupported(false),
            unsupported(false),
            (
                [gone, file(100, true).as_str()].join("; "),
                restarted.clone(),
                vec!["clone bulk accounting unavailable: malformed getattrlistbulk record".into()],
                observed(&[]),
                BULK_BUFFER_BYTES,
                false,
            ),
            // Cancelled after the first batch: no second read, entries read so far are kept.
            (
                mixed_entries,
                vec![(true, BULK_BUFFER_BYTES)],
                vec![],
                observed(&[0, 5]),
                BULK_BUFFER_BYTES,
                true,
            ),
            (
                "terminal (\"Interrupted\", Some(4), \"Interrupted system call (os error 4)\")"
                    .into(),
                vec![(false, BULK_BUFFER_BYTES), (false, BULK_BUFFER_BYTES)],
                vec![],
                observed(&[]),
                BULK_BUFFER_BYTES,
                false,
            ),
        ];
        for ((rich, script), expected) in cases.into_iter().zip(expected) {
            let cancel = AtomicBool::new(false);
            let mut calls = Vec::new();
            let mut warnings = Vec::new();
            let mut observations = VecDeque::new();
            let mut buffer = Vec::new();
            let mut steps = script.into_iter();
            let mut reader = |_: &std::os::fd::OwnedFd, buffer: &mut [u8], rich: bool| {
                calls.push((rich, buffer.len()));
                let step = steps.next().expect("unexpected reader call");
                let records = match step {
                    Step::Error(code) => return Err(io::Error::from_raw_os_error(code)),
                    Step::Malformed => {
                        let record = rich_record(7, 0, 1);
                        buffer[..record.len()].copy_from_slice(&record);
                        // Valid first row, invalid second row beyond allowance.
                        buffer[record.len()..record.len() + 24].fill(0);
                        return Ok(2);
                    }
                    Step::Cancel(records) => {
                        cancel.store(true, Ordering::Relaxed);
                        records
                    }
                    Step::Batch(records) => records,
                };
                let mut at = 0;
                for (index, (kind, name)) in records.iter().enumerate() {
                    let mut record = rich_record(
                        if rich { 0x607 } else { 7 },
                        if rich { 0x1300 } else { 0 },
                        *kind,
                    );
                    let len = record.len();
                    record[len - 4..].copy_from_slice(name);
                    record[40..48].copy_from_slice(&(index as u64 + 100).to_ne_bytes());
                    buffer[at..at + len].copy_from_slice(&record);
                    at += len;
                }
                Ok(records.len())
            };
            let result = enumerate_using(
                &path,
                &cancel,
                &fd,
                rich,
                &mut warnings,
                &mut observations,
                &mut buffer,
                &mut reader,
            );
            let state = (
                outcome(result),
                calls,
                warnings,
                observations.into_iter().collect::<Vec<_>>(),
                buffer.len(),
                cancel.load(Ordering::Relaxed),
            );
            assert_eq!(state, expected, "rich={rich}");
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn extension_reservations_bound_large_batches_and_preserve_small_minimums() {
        // Capacity after each growth: small batches double like push; past 16 384 entries a batch
        // reserves max(needed, 1.5x capacity).
        for (batches, growth) in [
            (
                vec![100],
                vec![128, 256, 512, 1024, 2048, 4096, 8192, 16384, 24576, 36864],
            ),
            (
                vec![546, 2184, 2184],
                vec![1024, 4096, 8192, 16384, 24576, 36864],
            ),
            (
                vec![1, 257, 4096, 100],
                vec![4, 512, 8192, 16384, 24576, 36864],
            ),
            (vec![16_384], vec![16384, 32768, 49152]),
            (vec![33_333], vec![33333]),
        ] {
            let mut actual: Vec<io::Result<Entry>> = Vec::new();
            let mut observed = Vec::new();
            let mut cursor = 0;
            while actual.len() < 33_333 {
                let count = batches[cursor % batches.len()].min(33_333 - actual.len());
                reserve_listing_batch(&mut actual, count);
                actual.extend((0..count).map(|_| Err(io::Error::other("entry"))));
                assert!(actual.capacity() <= 49_152);
                if observed.last() != Some(&actual.capacity()) {
                    observed.push(actual.capacity());
                }
                cursor += 1;
            }
            assert_eq!(observed, growth, "{batches:?}");
            if batches == [100] || batches == [546, 2184, 2184] {
                assert!(actual.capacity() < 40_000);
            }
        }
        assert!(std::mem::size_of::<Record<'_>>() <= 96);
        assert!(std::mem::size_of::<io::Result<Entry>>() <= 64);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn listing_batch_reservation_preserves_small_growth_and_bounds_large_growth() {
        let mut ordinary = Vec::new();
        let mut bounded = Vec::new();
        for end in [1, 7, 257, 4096, 4097, 8192, 8193, 16_384] {
            let additional = end - bounded.len();
            reserve_listing_batch(&mut bounded, additional);
            while ordinary.len() < end {
                ordinary.push(ordinary.len());
                bounded.push(bounded.len());
            }
            assert_eq!(ordinary.capacity(), bounded.capacity());
        }
        assert_eq!(ordinary, bounded);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn wide_listing_keeps_order_with_bounded_capacity() {
        let path = std::env::temp_dir();
        let fd = std::fs::File::open(&path).unwrap().into();
        let mut emitted = 0usize;
        let mut warnings = Vec::new();
        let mut observations = std::collections::VecDeque::new();
        let entries = enumerate_using(
            &path,
            &std::sync::atomic::AtomicBool::new(false),
            &fd,
            false,
            &mut warnings,
            &mut observations,
            &mut Vec::new(),
            &mut |_, buffer, _| {
                assert_eq!(buffer.len(), BULK_BUFFER_BYTES);
                let count = (33_333usize - emitted).min(100);
                let mut at = 0;
                for _ in 0..count {
                    let mut record = rich_record(7, 0, 1);
                    record[40..48].copy_from_slice(&(emitted as u64).to_ne_bytes());
                    buffer[at..at + record.len()].copy_from_slice(&record);
                    at += record.len();
                    emitted += 1;
                }
                Ok(count)
            },
        )
        .unwrap();
        assert_eq!(entries.len(), 33_333);
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(entry.as_ref().unwrap().inode, index as u64);
            assert_eq!(
                entry.as_ref().unwrap().path.as_ref(),
                std::path::Path::new("abc")
            );
        }
        assert!(entries.capacity() < 40_000);
        assert!(warnings.is_empty());
        assert!(observations.is_empty());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn rich_interrupted_read_is_left_to_the_caller_retry() {
        let path = std::env::temp_dir();
        let fd = std::fs::File::open(&path).unwrap().into();
        let mut warnings = Vec::new();
        let result = enumerate_using(
            &path,
            &std::sync::atomic::AtomicBool::new(false),
            &fd,
            true,
            &mut warnings,
            &mut std::collections::VecDeque::new(),
            &mut Vec::new(),
            &mut |_, _, _| Err(io::Error::from_raw_os_error(libc::EINTR)),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(warnings.is_empty());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn basename_listing_preserves_native_error_context_and_clone_ordinals() {
        use std::os::unix::ffi::OsStrExt;
        let path = std::env::temp_dir().join(format!("bulk-basename-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let fd = std::fs::File::open(&path).unwrap().into();
        let mut calls = 0;
        let mut observations = std::collections::VecDeque::new();
        let entries = enumerate_using(
            &path,
            &std::sync::atomic::AtomicBool::new(false),
            &fd,
            true,
            &mut Vec::new(),
            &mut observations,
            &mut Vec::new(),
            &mut |_, buffer, _| {
                calls += 1;
                if calls > 1 {
                    return Ok(0);
                }
                let mut at = 0;
                // The nonexistent directory must use descriptor metadata and
                // report its full native path; following file ordinals cannot shift.
                for kind in [1, 2, 1] {
                    let mut record = rich_record(0x607, 0x1300, kind);
                    let len = record.len();
                    record[len - 4..].copy_from_slice(b"a\xffb\0");
                    buffer[at..at + len].copy_from_slice(&record);
                    at += len;
                }
                Ok(3)
            },
        )
        .unwrap();
        assert_eq!(
            observations
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        for index in [0, 2] {
            assert_eq!(
                entries[index].as_ref().unwrap().path.as_os_str().as_bytes(),
                b"a\xffb"
            );
        }
        let error = entries[1].as_ref().unwrap_err();
        let context = error
            .get_ref()
            .unwrap()
            .downcast_ref::<crate::EntryMetadataError>()
            .unwrap();
        assert_eq!(
            context.path,
            path.join(std::ffi::OsStr::from_bytes(b"a\xffb"))
        );
        assert_eq!(
            crate::native_entry_error(error).raw_os_error(),
            Some(libc::ENOENT)
        );
        std::fs::remove_dir_all(path).unwrap();
    }
    fn rich_record(file: u32, ext: u32, kind: u32) -> Vec<u8> {
        let mut b = vec![0u8; 48];
        b[4..8].copy_from_slice(&0x8200000bu32.to_ne_bytes());
        b[16..20].copy_from_slice(&file.to_ne_bytes());
        b[20..24].copy_from_slice(&ext.to_ne_bytes());
        b[36..40].copy_from_slice(&kind.to_ne_bytes());
        for (mask, bytes) in [
            (1, 1u32.to_ne_bytes().to_vec()),
            (2, 128u64.to_ne_bytes().to_vec()),
            (4, 1024u64.to_ne_bytes().to_vec()),
            (0x200, 123u64.to_ne_bytes().to_vec()),
            (0x400, 512u64.to_ne_bytes().to_vec()),
        ] {
            if file & mask != 0 {
                b.extend(bytes);
            }
        }
        for (mask, bytes) in [
            (0x100, 42u64.to_ne_bytes().to_vec()),
            (0x200, 1u64.to_ne_bytes().to_vec()),
            (0x1000, 2u32.to_ne_bytes().to_vec()),
        ] {
            if ext & mask != 0 {
                b.extend(bytes);
            }
        }
        let offset = (b.len() - 24) as u32;
        b[24..28].copy_from_slice(&offset.to_ne_bytes());
        b[28..32].copy_from_slice(&4u32.to_ne_bytes());
        b.extend(b"abc\0");
        let len = b.len() as u32;
        b[..4].copy_from_slice(&len.to_ne_bytes());
        b
    }
    #[test]
    fn rich_variable_masks_and_nonfiles() {
        let record = rich_record(0x607, 0x1300, 1);
        let r = parse(&record, 1, true).unwrap();
        assert!(r[0].clone_checked);
        assert_eq!(r[0].clone_metadata.as_ref().unwrap().clone_id, 42);
        assert!(!r[0].clone_metadata.as_ref().unwrap().verified);
        for ext in [0, 0x100, 0x200, 0x1000, 0x300, 0x1100] {
            let record = rich_record(7, ext, 1);
            let r = parse(&record, 1, true).unwrap();
            assert!(!r[0].clone_checked);
            assert!(r[0].clone_metadata.is_none());
        }
        for file in [7, 0x207, 0x407] {
            let record = rich_record(file, 0x1300, 1);
            let r = parse(&record, 1, true).unwrap();
            assert!(!r[0].clone_checked);
            assert!(r[0].clone_metadata.is_none());
        }
        for kind in [2, 5] {
            let record = rich_record(if kind == 2 { 0 } else { 0x607 }, 0x1300, kind);
            let r = parse(&record, 1, true).unwrap();
            assert!(!r[0].clone_checked);
            assert!(r[0].clone_metadata.is_none());
        }
        let mut nonsharing = rich_record(0x607, 0x1300, 1);
        nonsharing[92..100].copy_from_slice(&0u64.to_ne_bytes());
        nonsharing[100..104].copy_from_slice(&1u32.to_ne_bytes());
        let r = parse(&nonsharing, 1, true).unwrap();
        assert!(r[0].clone_checked);
        assert!(r[0].clone_metadata.is_none());
        nonsharing[84..92].copy_from_slice(&0u64.to_ne_bytes());
        nonsharing[92..100].copy_from_slice(&1u64.to_ne_bytes());
        assert!(parse(&nonsharing, 1, true).unwrap()[0]
            .clone_metadata
            .is_some());
    }
    #[test]
    fn rich_rejects_malformed_and_impossible_fields() {
        let b = rich_record(0x607, 0x1300, 1);
        for end in 0..b.len() {
            assert!(parse(&b[..end], 1, true).is_err());
        }
        for at in [52, 60, 68, 76] {
            let mut bad = b.clone();
            bad[at..at + 8].copy_from_slice(&u64::MAX.to_ne_bytes());
            assert!(parse(&bad, 1, true).is_err());
        }
        for (at, value) in [(16, 0x807u32), (20, 0x1301), (24, 24)] {
            let mut bad = b.clone();
            bad[at..at + 4].copy_from_slice(&value.to_ne_bytes());
            assert!(parse(&bad, 1, true).is_err());
        }
        let mut bad = b.clone();
        bad[76..84].copy_from_slice(&1025u64.to_ne_bytes());
        assert!(parse(&bad, 1, true).is_err());
        bad = b;
        bad[68..76].copy_from_slice(&129u64.to_ne_bytes());
        assert!(parse(&bad, 1, true).is_err());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn real_bulk_clone_metadata_and_descriptor_nonfiles() {
        use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
        let p = std::env::temp_dir().join(format!("disk-bulk-clones-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        let a = p.join("a");
        let b = p.join("b");
        std::fs::write(&a, vec![7u8; 128 * 1024]).unwrap();
        let ac = std::ffi::CString::new(a.as_os_str().as_bytes()).unwrap();
        let bc = std::ffi::CString::new(b.as_os_str().as_bytes()).unwrap();
        unsafe extern "C" {
            fn clonefile(a: *const libc::c_char, b: *const libc::c_char, flags: u32) -> i32;
        }
        if unsafe { clonefile(ac.as_ptr(), bc.as_ptr(), 0) } != 0 {
            crate::skip_unsupported_fixture(&format!(
                "APFS bulk fixture unsupported: {}",
                io::Error::last_os_error()
            ));
            std::fs::remove_dir_all(p).unwrap();
            return;
        }
        std::fs::create_dir(p.join("dir")).unwrap();
        std::os::unix::fs::symlink("a", p.join("link")).unwrap();
        let pc = std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap();
        let raw = unsafe {
            libc::open(
                pc.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(raw >= 0);
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut observations = std::collections::VecDeque::new();
        let entries = enumerate_with_accounting(
            &p,
            &cancel,
            &fd,
            0,
            &std::sync::Mutex::new(std::collections::HashSet::new()),
            &mut Vec::new(),
            &mut observations,
            &mut Vec::new(),
        )
        .unwrap();
        let entries: Vec<_> = entries.into_iter().map(Result::unwrap).collect();
        assert_eq!(entries.len(), 4);
        let a = entries
            .iter()
            .find(|e| e.path.file_name().unwrap() == "a")
            .unwrap();
        let b = entries
            .iter()
            .find(|e| e.path.file_name().unwrap() == "b")
            .unwrap();
        assert!(a.clone_checked && b.clone_checked);
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].1.clone_id, observations[1].1.clone_id);
        assert!(observations
            .iter()
            .all(|(index, m)| !m.verified && entries[*index].kind == NodeKind::File));
        for e in entries.iter().filter(|e| e.kind != crate::NodeKind::File) {
            assert!(!e.clone_checked);
        }
        let expected = crate::descriptor::identity_fd(&fd).unwrap();
        let fresh = crate::descriptor::reopen(&fd, &expected).unwrap();
        let mut stats = CloneReadStats::default();
        let mut visited = 0;
        visit_clone_records(&fresh, &cancel, &mut stats, true, |record| {
            visited += 1;
            if record.kind == 1 {
                assert!(record.clone_checked);
                assert!(record.clone_metadata.is_some());
                assert_eq!(record.device, a.device);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(visited, 4);
        assert!(stats.queries >= 2);
        // Basic bulk fallback keeps basenames too. Clone probes must still
        // validate exact identity/sizes through the held parent descriptor.
        let fresh = crate::descriptor::reopen(&fd, &expected).unwrap();
        let baseline = enumerate_using(
            &p,
            &cancel,
            &fresh,
            false,
            &mut Vec::new(),
            &mut std::collections::VecDeque::new(),
            &mut Vec::new(),
            &mut read_batch,
        )
        .unwrap();
        let mut probes = crate::clones::prepare(
            &baseline,
            &fresh,
            &std::sync::Mutex::new(std::collections::HashMap::new()),
            &cancel,
            std::collections::VecDeque::new(),
            true,
        );
        assert_eq!(probes.queries, 2);
        for (index, e) in baseline.iter().enumerate() {
            let e = e.as_ref().unwrap();
            if e.kind == NodeKind::File {
                assert!(!e.path.is_absolute());
                let metadata = probes.take_at(index).unwrap().unwrap().unwrap();
                assert_eq!(metadata.clone_id, observations[0].1.clone_id);
            } else {
                assert!(probes.take_at(index).is_none());
            }
        }
        std::fs::remove_dir_all(p).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn every_unsupported_errno_recovers_silently_on_the_rich_listing() {
        let path = std::env::temp_dir().join(format!("bulk-unsupported-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let fd = std::fs::File::open(&path).unwrap().into();
        for (code, warns) in [
            (libc::ENOTSUP, false),
            (libc::EOPNOTSUPP, false),
            (libc::EINVAL, false),
            (libc::EIO, true),
        ] {
            let mut warnings = Vec::new();
            let mut reader = |_: &std::os::fd::OwnedFd, _: &mut [u8], rich: bool| {
                if rich {
                    Err(io::Error::from_raw_os_error(code))
                } else {
                    Ok(0)
                }
            };
            enumerate_using(
                &path,
                &std::sync::atomic::AtomicBool::new(false),
                &fd,
                true,
                &mut warnings,
                &mut std::collections::VecDeque::new(),
                &mut Vec::new(),
                &mut reader,
            )
            .unwrap();
            assert_eq!(!warnings.is_empty(), warns, "errno {code}");
        }
        std::fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn accounting_recovery_discards_rich_records_and_keeps_permanent_errors() {
        fn entry() -> Entry {
            Entry {
                path: "file".into(),
                device: 1,
                inode: 2,
                kind: NodeKind::File,
                length: 128,
                allocated: 1024,
                hard_link: false,
                clone_checked: false,
            }
        }
        for cause in [invalid(), io::Error::from_raw_os_error(libc::EACCES)] {
            let mut warnings = Vec::new();
            let prior = vec![
                Ok(entry()),
                Err(io::Error::from_raw_os_error(libc::EIO)),
                Err(io::Error::from_raw_os_error(libc::EINTR)),
            ];
            let result = recover_listing(prior, cause, false, &mut warnings, |_| {
                Ok(vec![Ok(entry())])
            })
            .unwrap();
            assert_eq!(warnings.len(), 1);
            assert_eq!(result.len(), 2);
            assert_eq!(
                result[0].as_ref().unwrap_err().raw_os_error(),
                Some(libc::EIO)
            );
            assert!(result[1].as_ref().unwrap().clone_checked);
        }
        let mut warnings = Vec::new();
        let empty = recover_listing(vec![Ok(entry())], invalid(), false, &mut warnings, |_| {
            Ok(vec![])
        })
        .unwrap();
        assert!(empty.is_empty());
        assert_eq!(warnings.len(), 1);
        let failed = recover_listing(vec![Ok(entry())], invalid(), false, &mut warnings, |_| {
            Err(io::Error::from_raw_os_error(libc::EACCES))
        });
        assert_eq!(failed.unwrap_err().raw_os_error(), Some(libc::EACCES));
        assert_eq!(warnings.len(), 1, "a failed relisting is no accounting gap");
        let permanent = recover_listing(
            vec![Err(io::Error::from_raw_os_error(libc::EIO))],
            invalid(),
            false,
            &mut warnings,
            |_| Err(io::Error::from_raw_os_error(libc::EACCES)),
        )
        .unwrap();
        assert_eq!(permanent.len(), 2);
        // The relisting reports the same failing entry again: one error per path, not two.
        // A kept failure the relisting does not report (or one without a path) stays.
        let at = |name: &str, code| {
            Err(
                crate::entry_result(Err(io::Error::from_raw_os_error(code)), || name.into())
                    .unwrap_err(),
            )
        };
        let mut warnings = Vec::new();
        let result = recover_listing(
            vec![at("both", libc::EACCES), at("only-first", libc::EIO)],
            invalid(),
            false,
            &mut warnings,
            |_| Ok(vec![at("both", libc::EACCES), Ok(entry())]),
        )
        .unwrap();
        let paths: Vec<_> = result
            .iter()
            .map(|e| e.as_ref().err().and_then(crate::entry_error_path))
            .collect();
        assert_eq!(
            paths,
            [
                Some(std::path::Path::new("only-first")),
                Some(std::path::Path::new("both")),
                None
            ]
        );
        let mut warnings = Vec::new();
        let unsupported = recover_listing(
            vec![Ok(entry())],
            io::Error::from_raw_os_error(libc::ENOTSUP),
            true,
            &mut warnings,
            |_| Ok(vec![Ok(entry())]),
        )
        .unwrap();
        assert!(warnings.is_empty());
        assert!(!unsupported[0].as_ref().unwrap().clone_checked);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn refused_clone_fields_are_remembered_per_device() {
        let path = std::env::temp_dir();
        let fd: std::os::fd::OwnedFd = std::fs::File::open(&path).unwrap().into();
        let device = crate::descriptor::identity_fd(&fd).unwrap().device;
        let refused = std::sync::Mutex::new(std::collections::HashSet::new());
        let mut asked = Vec::new();
        let mut list = |refused: &std::sync::Mutex<_>| {
            enumerate_remembering(
                &path,
                &std::sync::atomic::AtomicBool::new(false),
                &fd,
                device,
                refused,
                &mut Vec::new(),
                &mut std::collections::VecDeque::new(),
                &mut Vec::new(),
                &mut |_, _, rich| {
                    asked.push(rich);
                    if rich {
                        Err(io::Error::from_raw_os_error(libc::ENOTSUP))
                    } else {
                        Ok(0)
                    }
                },
            )
            .unwrap()
        };
        list(&refused);
        list(&refused);
        assert_eq!(asked, [true, false, false]);
        assert!(refused.lock().unwrap().contains(&device));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn malformed_later_rich_batch_restarts_baseline_without_publishing_rich_success() {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        };
        let p = std::env::temp_dir().join(format!("bulk-restart-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        let pc = std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap();
        let raw = unsafe {
            libc::open(
                pc.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        assert!(raw >= 0);
        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) };
        let mut warnings = Vec::new();
        let mut calls = 0;
        let mut observations = std::collections::VecDeque::new();
        let mut workspace = vec![0xa5; 256 * 1024];
        let allocation = workspace.as_ptr();
        let entries = enumerate_using(
            &p,
            &std::sync::atomic::AtomicBool::new(false),
            &fd,
            true,
            &mut warnings,
            &mut observations,
            &mut workspace,
            &mut |attempt_fd, buffer, rich| {
                calls += 1;
                match calls {
                    1 => {
                        assert!(rich);
                        let record = rich_record(0x607, 0x1300, 1);
                        buffer[..record.len()].copy_from_slice(&record);
                        Ok(1)
                    }
                    2 => {
                        assert!(rich);
                        buffer[..4].copy_from_slice(&0u32.to_ne_bytes());
                        Ok(1)
                    }
                    3 => {
                        assert!(!rich);
                        assert_ne!(attempt_fd.as_raw_fd(), fd.as_raw_fd());
                        let record = rich_record(7, 0, 1);
                        buffer[..record.len()].copy_from_slice(&record);
                        Ok(1)
                    }
                    4 => {
                        assert!(!rich);
                        Ok(0)
                    }
                    _ => panic!("unbounded restart"),
                }
            },
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].as_ref().unwrap().path.as_ref(),
            std::path::Path::new("abc")
        );
        assert_eq!(warnings.len(), 1);
        let entry = entries[0].as_ref().unwrap();
        assert!(entry.clone_checked);
        assert!(observations.is_empty());
        assert_eq!(workspace.as_ptr(), allocation);
        assert_eq!(workspace.len(), BULK_BUFFER_BYTES);
        assert_eq!(workspace.capacity(), 256 * 1024);
        // A later listing keeps the same allocation and does not clear unused
        // bytes; the parser still consumes only the kernel-reported records.
        let last = workspace.len() - 1;
        workspace[last] = 0x5a;
        let empty = enumerate_using(
            &p,
            &std::sync::atomic::AtomicBool::new(false),
            &fd,
            true,
            &mut warnings,
            &mut observations,
            &mut workspace,
            &mut |_, buffer, _| {
                assert_eq!(buffer[buffer.len() - 1], 0x5a);
                Ok(0)
            },
        )
        .unwrap();
        assert!(empty.is_empty());
        assert_eq!(workspace.as_ptr(), allocation);
        let cancelled = enumerate_using(
            &p,
            &std::sync::atomic::AtomicBool::new(true),
            &fd,
            true,
            &mut warnings,
            &mut observations,
            &mut workspace,
            &mut |_, _, _| panic!("cancelled listing queried the filesystem"),
        )
        .unwrap();
        assert!(cancelled.is_empty());
        assert_eq!(workspace.as_ptr(), allocation);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn streaming_checks_every_batch_and_cancellation_without_retry() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let record = rich_record(0x607, 0x1300, 1);
        let cancel = AtomicBool::new(false);
        let mut stats = CloneReadStats::default();
        let mut calls = 0;
        let mut visited = 0;
        let result = visit_with_reader(
            &cancel,
            &mut stats,
            |buffer| {
                calls += 1;
                if calls == 1 {
                    buffer[..record.len()].copy_from_slice(&record);
                } else {
                    buffer[..4].copy_from_slice(&0u32.to_ne_bytes());
                }
                Ok(1)
            },
            &mut |_| {
                visited += 1;
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(visited, 1);
        assert_eq!(stats.queries, 2);
        let mut stats = CloneReadStats::default();
        let result = visit_with_reader(
            &cancel,
            &mut stats,
            |buffer| {
                buffer[..record.len()].copy_from_slice(&record);
                buffer[record.len()..record.len() * 2].copy_from_slice(&record);
                Ok(2)
            },
            &mut |_| {
                cancel.store(true, Ordering::Relaxed);
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EINTR));
        assert_eq!(stats.queries, 1);
        let mut stats = CloneReadStats::default();
        visit_with_reader(
            &cancel,
            &mut stats,
            |_| panic!("cancelled before syscall"),
            &mut |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(stats.queries, 0);
        cancel.store(false, Ordering::Relaxed);
        let mut stats = CloneReadStats::default();
        let result = visit_with_reader(
            &cancel,
            &mut stats,
            |_| Err(io::Error::from_raw_os_error(libc::EINTR)),
            &mut |_| Ok(()),
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EINTR));
        assert_eq!(stats.queries, 1);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn interrupted_read_reaches_the_caller_instead_of_retrying_the_descriptor() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut stats = CloneReadStats::default();
        let mut calls = 0;
        let result = visit_with_reader(
            &cancel,
            &mut stats,
            |_| {
                calls += 1;
                Err(io::Error::from_raw_os_error(libc::EINTR))
            },
            &mut |_| Ok(()),
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EINTR));
        assert_eq!(calls, 1);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn adaptive_streaming_grows_only_after_valid_packed_batches_and_resets_next_call() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let cancel = AtomicBool::new(false);
        let small = rich_record(0x607, 0x1300, 1);
        let mut packed = small.clone();
        packed.resize(BULK_BUFFER_BYTES / 4 * 3, 0);
        let length = packed.len() as u32;
        packed[..4].copy_from_slice(&length.to_ne_bytes());
        let mut stats = CloneReadStats::default();
        let mut reads = 0;
        let mut visited = 0;
        visit_with_reader(
            &cancel,
            &mut stats,
            |buffer| {
                reads += 1;
                match reads {
                    1 => {
                        assert_eq!(buffer.len(), BULK_BUFFER_BYTES);
                        buffer[..small.len()].copy_from_slice(&small);
                        Ok(1)
                    }
                    2 => {
                        assert_eq!(buffer.len(), BULK_BUFFER_BYTES);
                        buffer[..packed.len()].copy_from_slice(&packed);
                        Ok(1)
                    }
                    3 => {
                        assert_eq!(buffer.len(), LARGE_BULK_BUFFER_BYTES);
                        Ok(0)
                    }
                    _ => panic!("unexpected read"),
                }
            },
            &mut |_| {
                visited += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!((reads, visited), (3, 2));
        let capacity = stats.buffer.capacity();
        visit_with_reader(
            &cancel,
            &mut stats,
            |buffer| {
                assert_eq!(buffer.len(), BULK_BUFFER_BYTES);
                Ok(0)
            },
            &mut |_| panic!("empty batch visited"),
        )
        .unwrap();
        assert_eq!(stats.buffer.capacity(), capacity);

        let mut stats = CloneReadStats::default();
        let result = visit_with_reader(
            &cancel,
            &mut stats,
            |buffer| {
                buffer[..packed.len()].copy_from_slice(&packed);
                buffer[..4].copy_from_slice(&((BULK_BUFFER_BYTES + 4) as u32).to_ne_bytes());
                Ok(1)
            },
            &mut |_| panic!("malformed batch visited"),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(stats.buffer.len(), BULK_BUFFER_BYTES);
        assert_eq!(stats.buffer.capacity(), BULK_BUFFER_BYTES);

        let result = visit_with_reader(
            &cancel,
            &mut stats,
            |buffer| {
                buffer[..packed.len()].copy_from_slice(&packed);
                Ok(1)
            },
            &mut |_| Err(io::Error::from_raw_os_error(libc::EIO)),
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EIO));
        assert_eq!(stats.buffer.len(), BULK_BUFFER_BYTES);
        assert_eq!(stats.buffer.capacity(), BULK_BUFFER_BYTES);

        let result = visit_with_reader(
            &cancel,
            &mut stats,
            |buffer| {
                buffer[..packed.len()].copy_from_slice(&packed);
                Ok(1)
            },
            &mut |_| {
                cancel.store(true, Ordering::Relaxed);
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EINTR));
        assert_eq!(stats.buffer.len(), LARGE_BULK_BUFFER_BYTES);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn streaming_rejects_record_extending_into_retained_inactive_capacity() {
        let mut stale = rich_record(0x607, 0x1300, 1);
        stale.resize(BULK_BUFFER_BYTES + 4, 0xa5);
        stale[..4].copy_from_slice(&((BULK_BUFFER_BYTES + 4) as u32).to_ne_bytes());
        assert!(parse(&stale, 1, true).is_ok());
        stale.resize(256 * 1024, 0xa5);
        let pointer = stale.as_ptr();
        let retained_capacity = stale.capacity();
        let mut stats = CloneReadStats {
            buffer: stale,
            ..CloneReadStats::default()
        };
        let mut visited = 0;
        let result = visit_with_reader(
            &std::sync::atomic::AtomicBool::new(false),
            &mut stats,
            |buffer| {
                assert_eq!(buffer.len(), BULK_BUFFER_BYTES);
                Ok(1)
            },
            &mut |_| {
                visited += 1;
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(visited, 0);
        assert_eq!(stats.queries, 1);
        assert_eq!(stats.buffer.as_ptr(), pointer);
        assert_eq!(stats.buffer.len(), BULK_BUFFER_BYTES);
        assert_eq!(stats.buffer.capacity(), retained_capacity);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn streaming_reuses_worker_buffer_after_success_errors_and_cancellation() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let cancel = AtomicBool::new(false);
        let mut stats = CloneReadStats::default();
        visit_with_reader(&cancel, &mut stats, |_| Ok(0), &mut |_| Ok(())).unwrap();
        let pointer = stats.buffer.as_ptr();
        let capacity = stats.buffer.capacity();
        assert_eq!(stats.queries, 1);
        for error in [libc::EIO, libc::EINTR] {
            let result = visit_with_reader(
                &cancel,
                &mut stats,
                |_| Err(io::Error::from_raw_os_error(error)),
                &mut |_| Ok(()),
            );
            assert_eq!(result.unwrap_err().raw_os_error(), Some(error));
            assert_eq!(stats.buffer.as_ptr(), pointer);
            assert_eq!(stats.buffer.capacity(), capacity);
        }
        let before_cancel = stats.queries;
        cancel.store(true, Ordering::Relaxed);
        visit_with_reader(
            &cancel,
            &mut stats,
            |_| panic!("cancelled"),
            &mut |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(stats.queries, before_cancel);
        assert_eq!(stats.buffer.as_ptr(), pointer);
        cancel.store(false, Ordering::Relaxed);
        let result = visit_with_reader(
            &cancel,
            &mut stats,
            |buffer| {
                buffer[..4].copy_from_slice(&0u32.to_ne_bytes());
                Ok(1)
            },
            &mut |_| Ok(()),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(stats.buffer.as_ptr(), pointer);
        visit_with_reader(&cancel, &mut stats, |_| Ok(0), &mut |_| Ok(())).unwrap();
        assert_eq!(stats.buffer.as_ptr(), pointer);
    }
    #[test]
    fn rejects_short_and_overflow() {
        assert!(parse(&[0; 3], 1, false).is_err());
        let mut b = vec![0; 24];
        b[..4].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert!(parse(&b, 1, false).is_err());
    }
    #[test]
    fn parses_and_checks_name() {
        let mut b = vec![0u8; 72];
        b[..4].copy_from_slice(&72u32.to_ne_bytes());
        b[4..8].copy_from_slice(&0x8200000bu32.to_ne_bytes());
        b[16..20].copy_from_slice(&7u32.to_ne_bytes());
        b[24..28].copy_from_slice(&44u32.to_ne_bytes());
        b[28..32].copy_from_slice(&4u32.to_ne_bytes());
        b[36..40].copy_from_slice(&1u32.to_ne_bytes());
        b[48..52].copy_from_slice(&1u32.to_ne_bytes());
        b[52..60].copy_from_slice(&123u64.to_ne_bytes());
        b[60..68].copy_from_slice(&512u64.to_ne_bytes());
        b[68..72].copy_from_slice(b"abc\0");
        for end in 0..b.len() {
            assert!(parse(&b[..end], 1, false).is_err());
        }
        let r = parse(&b, 1, false).unwrap();
        assert_eq!(r[0].length, 123);
        assert_eq!(r[0].links, 1);
        let mut high_bit = b.clone();
        high_bit[32..36].copy_from_slice(&0x8000_0001u32.to_ne_bytes());
        assert_eq!(
            parse(&high_bit, 1, false).unwrap()[0].device,
            (-2147483647i64) as u64
        );
        let mut missing_links = b.clone();
        missing_links[16..20].copy_from_slice(&6u32.to_ne_bytes());
        assert!(parse(&missing_links, 1, false).is_err());
        b[69] = b'/';
        assert!(parse(&b, 1, false).is_err());
        b[24..28].copy_from_slice(&(-1i32).to_ne_bytes());
        assert!(parse(&b, 1, false).is_err());
    }
}
