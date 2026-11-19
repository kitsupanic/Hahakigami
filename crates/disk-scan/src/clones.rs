//! Conservative full-data-stream clone attribution. Raw metadata is never overwritten.
#[cfg(test)]
use crate::AccountingModel;
#[cfg(any(unix, test))]
use crate::NodeKind;
use crate::{CloneMetadata, Snapshot};
#[cfg(unix)]
use std::collections::HashMap;

mod accounting;
pub(crate) use accounting::internal_allocation_plan;
pub use accounting::{allocation_plan, AllocationPlan};
#[cfg(test)]
pub(crate) use accounting::{attributed_own_allocations, hardlink_allocations};

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ClaimStatus {
    Drop,
    #[cfg(target_os = "macos")]
    Raw,
    Unverified,
    Verified,
}
pub(crate) struct Validation {
    pub statuses: Vec<ClaimStatus>,
    pub accounting: crate::AccountingInfo,
}
impl Validation {
    pub(crate) fn apply(self, s: &mut Snapshot) {
        assert_eq!(self.statuses.len(), s.tree.clones.len());
        let mut index = 0;
        s.tree.clones.retain_mut(|(_, m)| {
            let status = self.statuses[index];
            index += 1;
            m.verified = status == ClaimStatus::Verified;
            status != ClaimStatus::Drop
        });
        s.accounting = self.accounting;
    }
}
/// Without native clone queries nothing is re-verified; keep what the scan already knew.
#[cfg(not(target_os = "macos"))]
pub(crate) fn validate_unverified(
    s: &Snapshot,
    cancel: &std::sync::atomic::AtomicBool,
) -> Validation {
    let mut accounting = s.accounting.clone();
    accounting.checks_complete &= !cancel.load(std::sync::atomic::Ordering::Relaxed);
    Validation {
        statuses: s
            .tree
            .clones
            .iter()
            .map(|(_, m)| {
                if m.verified {
                    ClaimStatus::Verified
                } else {
                    ClaimStatus::Unverified
                }
            })
            .collect(),
        accounting,
    }
}
#[cfg(unix)]
pub(crate) fn validate(
    s: &Snapshot,
    root: &std::os::fd::OwnedFd,
    cancel: &std::sync::atomic::AtomicBool,
    qos: crate::ScanQos,
) -> Validation {
    #[cfg(target_os = "macos")]
    {
        native::validate(s, root, cancel, qos)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (root, qos);
        validate_unverified(s, cancel)
    }
}
pub(crate) struct ProbeBatch {
    observations: std::collections::VecDeque<(usize, CloneMetadata)>,
    results: std::collections::VecDeque<(usize, std::io::Result<Option<CloneMetadata>>)>,
    pub cancelled: bool,
    pub queries: u64,
    pub micros: u64,
}
impl ProbeBatch {
    /// Consume both sparse queues in listing order, including skipped/error rows.
    pub(crate) fn take_at(
        &mut self,
        index: usize,
    ) -> Option<std::io::Result<Option<CloneMetadata>>> {
        if self
            .observations
            .front()
            .is_some_and(|(ordinal, _)| *ordinal == index)
        {
            let metadata = self.observations.pop_front().unwrap().1;
            return (metadata.may_share_blocks || metadata.reference_count > 1)
                .then_some(Ok(Some(metadata)));
        }
        if self
            .results
            .front()
            .is_some_and(|(ordinal, _)| *ordinal == index)
        {
            return self.results.pop_front().map(|(_, result)| result);
        }
        None
    }
}
#[cfg(unix)]
pub(crate) fn prepare(
    entries: &[std::io::Result<crate::Entry>],
    fd: &std::os::fd::OwnedFd,
    caps: &std::sync::Mutex<HashMap<u64, bool>>,
    cancel: &std::sync::atomic::AtomicBool,
    observations: std::collections::VecDeque<(usize, CloneMetadata)>,
    device_answered: bool,
) -> ProbeBatch {
    prepare_core(entries, fd, caps, observations, device_answered, || {
        cancel.load(std::sync::atomic::Ordering::Relaxed)
    })
}
/// A reused listing carries no clone evidence, so every cached file would be probed on its own.
/// One rich bulk listing of the directory vouches for each file it still describes exactly
/// (marking it `clone_checked`, with a sparse observation when it shares); anything else stays
/// unchecked and is probed per file, with the same warnings as before. Call with empty `observations`.
#[cfg(unix)]
pub(crate) fn vouch_reused(
    entries: &mut [std::io::Result<crate::Entry>],
    fd: &std::os::fd::OwnedFd,
    caps: &std::sync::Mutex<HashMap<u64, bool>>,
    cancel: &std::sync::atomic::AtomicBool,
    observations: &mut std::collections::VecDeque<(usize, CloneMetadata)>,
    buffer: &mut Vec<u8>,
) {
    #[cfg(target_os = "macos")]
    native::vouch_reused(entries, fd, caps, cancel, observations, buffer);
    #[cfg(not(target_os = "macos"))]
    let _ = (entries, fd, caps, cancel, observations, buffer);
}
#[cfg(unix)]
fn prepare_core(
    entries: &[std::io::Result<crate::Entry>],
    fd: &std::os::fd::OwnedFd,
    caps: &std::sync::Mutex<HashMap<u64, bool>>,
    observations: std::collections::VecDeque<(usize, CloneMetadata)>,
    device_answered: bool,
    mut cancelled: impl FnMut() -> bool,
) -> ProbeBatch {
    let mut batch = ProbeBatch {
        observations,
        results: Default::default(),
        cancelled: false,
        queries: 0,
        micros: 0,
    };
    // A bulk reply with clone fields (fresh rich listing, or `vouch_reused`) means this device
    // answered, so one file's reply without them is that file's warning, not a volume signal.
    // shortcut: one device per listing (files of a directory share it), as `vouch_reused` assumes.
    // A listing recovered from a failed rich read marks files `clone_checked` without any reply
    // (`device_answered` is false), so it proves nothing about the device.
    #[cfg(target_os = "macos")]
    if let Some(e) = entries
        .iter()
        .flatten()
        .find(|e| device_answered && e.kind == NodeKind::File && e.clone_checked)
    {
        caps.lock().unwrap().insert(e.device, true);
    }
    // Borrow the original queue; no second metadata queue is built. Only the
    // visited prefix can be published if preparation is cancelled midway.
    let mut observations = batch.observations.iter().peekable();
    let mut visited_observations = 0;
    for (index, entry) in entries.iter().enumerate() {
        if cancelled() {
            batch.cancelled = true;
            break;
        }
        if observations
            .peek()
            .is_some_and(|(ordinal, _)| *ordinal == index)
        {
            observations.next();
            visited_observations += 1;
            continue;
        }
        let result = match entry {
            Ok(e) if e.kind == NodeKind::File && !e.clone_checked => {
                #[cfg(target_os = "macos")]
                {
                    let started = std::time::Instant::now();
                    let result = native::prepare_entry(e, fd, caps, &mut batch.queries);
                    batch.micros = batch
                        .micros
                        .saturating_add(started.elapsed().as_micros() as u64);
                    result
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (fd, caps, device_answered);
                    Ok(None)
                }
            }
            _ => Ok(None),
        };
        // Only direct fallback evidence needs a new sparse queue. Nonsharing
        // observations need no per-file result record.
        if result.as_ref().map_or(true, |m| {
            m.as_ref()
                .is_some_and(|m| m.may_share_blocks || m.reference_count > 1)
        }) {
            batch.results.push_back((index, result));
        }
    }
    batch.observations.truncate(visited_observations);
    batch
}
pub(crate) fn commit(s: &mut Snapshot, id: u32, result: std::io::Result<Option<CloneMetadata>>) {
    match result {
        Err(e) => {
            s.accounting.warn(e.to_string());
        }
        Ok(Some(m)) => {
            s.accounting.shared |= m.may_share_blocks || m.reference_count > 1;
            if m.clone_id != 0 && m.reference_count > 1 {
                s.tree.clones.push((id, m));
            }
        }
        Ok(None) => {}
    }
}
#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use crate::bulk::is_unsupported;
    use std::{
        io,
        os::{
            fd::{AsRawFd, OwnedFd},
            unix::ffi::OsStrExt,
        },
        sync::atomic::{AtomicBool, Ordering},
    };
    const COMMON: u32 = 0x80000000 | 0x2 | 0x8 | 0x02000000;
    const FILE: u32 = 0x1 | 0x2 | 0x4 | 0x200 | 0x400;
    const EXT: u32 = 0x100 | 0x200 | 0x1000;
    fn error(message: &str) -> io::Error {
        io::Error::other(message)
    }
    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "clone attributes unsupported")
    }
    // Cursor over the shared bounds-checked readers in bulk.rs.
    fn u32at(b: &[u8], o: &mut usize) -> io::Result<u32> {
        let v = crate::bulk::u32_at(b, *o).map_err(|_| error("truncated clone attributes"))?;
        *o += 4;
        Ok(v)
    }
    fn u64at(b: &[u8], o: &mut usize) -> io::Result<u64> {
        let v = crate::bulk::u64_at(b, *o).map_err(|_| error("truncated clone attributes"))?;
        *o += 8;
        Ok(v)
    }
    fn size(b: &[u8], o: &mut usize) -> io::Result<u64> {
        let v = u64at(b, o)? as i64;
        u64::try_from(v).map_err(|_| error("negative clone size"))
    }
    fn parse(
        b: &[u8],
        identity: &crate::Identity,
        own_length: u64,
        own_allocated: u64,
        hard_link: bool,
    ) -> io::Result<CloneMetadata> {
        let mut o = 0;
        let len = u32at(b, &mut o)? as usize;
        let mut returned = [0; 5];
        for mask in &mut returned {
            *mask = u32at(b, &mut o)?;
        }
        // Volumes without clone mapping (HFS+, exFAT, network) omit both clone
        // fields. Classify that as unsupported so the device is cached once.
        if len >= o && returned[4] & 0x1100 == 0 {
            return Err(unsupported());
        }
        if len != 96 || len > b.len() {
            return Err(error("invalid clone attribute length"));
        }
        if returned != [COMMON, 0, 0, FILE, EXT] {
            return Err(error("missing clone attributes"));
        }
        if u32at(b, &mut o)? as i32 as u64 != identity.device
            || u32at(b, &mut o)? != 1
            || u64at(b, &mut o)? != identity.inode
        {
            return Err(error("clone file identity changed"));
        }
        let links = u32at(b, &mut o)?;
        if links == 0 || (links > 1) != hard_link {
            return Err(error("clone link count changed"));
        }
        let total = size(b, &mut o)?;
        let allocation = size(b, &mut o)?;
        let data_length = size(b, &mut o)?;
        let data = size(b, &mut o)?;
        let clone_id = u64at(b, &mut o)?;
        let flags = u64at(b, &mut o)?;
        let reference_count = u32at(b, &mut o)?;
        if (total != own_length && data_length != own_length)
            || allocation != own_allocated
            || data_length > total
            || data > allocation
        {
            return Err(error("clone file sizes changed"));
        }
        Ok(CloneMetadata {
            clone_id,
            data_allocated_size: data,
            data_length,
            reference_count,
            may_share_blocks: flags & 1 != 0,
            verified: false,
        })
    }
    fn parent(s: &Snapshot, id: u32, root: &OwnedFd) -> io::Result<OwnedFd> {
        let (names, root_identity) =
            crate::descriptor::path_names(s, s.node(id).unwrap().parent.unwrap_or(0))?;
        crate::descriptor::open_names(root, &root_identity, &names, None)
    }
    struct CachedDirectory {
        id: u32,
        name: crate::descriptor::ChildName,
        fd: OwnedFd,
    }
    /// Bounded ancestor handles, always rechecked from the original held root
    /// on parent switches. Same-parent files still get individual fresh probes.
    struct ParentCache {
        chain: Vec<CachedDirectory>,
        spare: Vec<CachedDirectory>,
        ancestors: Vec<u32>,
        fallback: Option<OwnedFd>,
        current: Option<u32>,
        limit: usize,
        peak_bytes: usize,
    }
    impl ParentCache {
        #[cfg(test)]
        fn new(s: &Snapshot) -> Self {
            Self::with_limit(s.descriptor_limit.unwrap_or(0).saturating_sub(2).min(32))
        }
        fn with_limit(limit: usize) -> Self {
            Self {
                chain: Vec::new(),
                spare: Vec::new(),
                ancestors: Vec::new(),
                fallback: None,
                current: None,
                limit,
                peak_bytes: 0,
            }
        }
        fn observe_storage(&mut self) {
            self.peak_bytes = self.peak_bytes.max(
                self.chain.capacity() * std::mem::size_of::<CachedDirectory>()
                    + self.spare.capacity() * std::mem::size_of::<CachedDirectory>()
                    + self.ancestors.capacity() * std::mem::size_of::<u32>()
                    + self
                        .chain
                        .iter()
                        .chain(&self.spare)
                        .map(|c| c.name.storage_bytes())
                        .sum::<usize>(),
            );
        }
        fn clear(&mut self) {
            self.observe_storage();
            self.chain.clear();
            self.spare.clear();
            self.fallback = None;
            self.current = None;
        }
        /// Retain only strict ancestors, with oldest spare handles evicted first.
        fn retire_suffix(&mut self, common: usize, target: u32) {
            let old_target = self.chain.last().map(|cached| cached.id);
            while self.chain.len() > common {
                let cached = self.chain.pop().unwrap();
                if Some(cached.id) != old_target && cached.id != target {
                    self.spare.push(cached);
                }
            }
            self.observe_storage();
        }
        fn ensure(&mut self, s: &Snapshot, id: u32, root: &OwnedFd) -> io::Result<()> {
            self.prepare(s, id, root, false)
        }
        /// Prepare an independently opened directory stream for a complete bulk
        /// listing. Replace its cached target handle instead of reopening it twice.
        fn fresh_cursor(&mut self, s: &Snapshot, id: u32, root: &OwnedFd) -> io::Result<()> {
            self.prepare(s, id, root, true)
        }
        fn prepare(
            &mut self,
            s: &Snapshot,
            id: u32,
            root: &OwnedFd,
            fresh_cursor: bool,
        ) -> io::Result<()> {
            let target = s
                .node(id)
                .unwrap()
                .parent
                .ok_or_else(|| error("clone file has no parent"))?;
            if !fresh_cursor && self.current == Some(target) {
                return Ok(());
            }
            self.current = None;
            self.fallback = None;
            let result = self.switch(s, id, target, root, fresh_cursor);
            if result.is_err() {
                self.clear();
            } else {
                self.current = Some(target);
            }
            result
        }
        fn switch(
            &mut self,
            s: &Snapshot,
            id: u32,
            target: u32,
            root: &OwnedFd,
            fresh_cursor: bool,
        ) -> io::Result<()> {
            if crate::descriptor::identity_fd(root)? != s.node(0).unwrap().identity {
                return Err(error("clone root identity changed"));
            }
            self.ancestors.clear();
            let mut next = Some(target);
            while let Some(id) = next {
                if id == 0 {
                    break;
                }
                self.ancestors.push(id);
                next = s.node(id).unwrap().parent;
            }
            self.ancestors.reverse();
            self.observe_storage();
            if self.ancestors.len() > self.limit {
                self.chain.clear();
                self.spare.clear();
                self.fallback = Some(parent(s, id, root)?);
                return Ok(());
            }
            if target == 0 {
                self.chain.clear();
                self.spare.clear();
                if fresh_cursor {
                    self.fallback = Some(crate::descriptor::reopen(
                        root,
                        &s.node(0).unwrap().identity,
                    )?);
                }
                return Ok(());
            }
            let mut common = self
                .chain
                .iter()
                .zip(&self.ancestors)
                .take_while(|(cached, id)| cached.id == **id)
                .count();
            // A cached target may already be exhausted. Keep only its parents
            // so open_child_name supplies the one fresh, identity-checked stream.
            if fresh_cursor && common == self.ancestors.len() && common > 0 {
                common -= 1;
            }
            self.retire_suffix(common, target);
            for i in 0..common {
                let cached = &self.chain[i];
                let parent = if i == 0 { root } else { &self.chain[i - 1].fd };
                if crate::descriptor::identity_child(parent, &cached.name)?
                    != s.node(cached.id).unwrap().identity
                {
                    return Err(error("clone directory identity changed since enumeration"));
                }
            }
            for index in common..self.ancestors.len() {
                let id = self.ancestors[index];
                let n = s.node(id).unwrap();
                let cached = self
                    .spare
                    .iter()
                    .position(|cached| cached.id == id)
                    .map(|position| self.spare.remove(position));
                if index + 1 < self.ancestors.len() {
                    if let Some(cached) = cached {
                        let parent = self.chain.last().map_or(root, |cached| &cached.fd);
                        if crate::descriptor::identity_child(parent, &cached.name)? != n.identity
                            || crate::descriptor::identity_fd(&cached.fd)? != n.identity
                        {
                            return Err(error(
                                "clone spare directory identity changed since enumeration",
                            ));
                        }
                        self.chain.push(cached);
                        self.observe_storage();
                        continue;
                    }
                } else {
                    // Final targets always receive a new independent cursor.
                    drop(cached);
                }
                while self.chain.len() + self.spare.len() >= self.limit {
                    self.spare.remove(0);
                }
                let name = crate::descriptor::ChildName::new(n.native_name())?;
                let parent = self.chain.last().map_or(root, |cached| &cached.fd);
                let fd = crate::descriptor::open_child_name(parent, &name, &n.identity)?;
                self.chain.push(CachedDirectory { id, name, fd });
                self.observe_storage();
            }
            Ok(())
        }
        fn fd<'a>(&'a self, root: &'a OwnedFd) -> &'a OwnedFd {
            self.fallback
                .as_ref()
                .unwrap_or_else(|| self.chain.last().map_or(root, |cached| &cached.fd))
        }
    }
    fn read(
        fd: &OwnedFd,
        name: &std::ffi::OsStr,
        identity: &crate::Identity,
        length: u64,
        allocated: u64,
        hard_link: bool,
    ) -> io::Result<CloneMetadata> {
        let name = crate::descriptor::ChildName::new(name)?;
        let mut list = libc::attrlist {
            bitmapcount: 5,
            reserved: 0,
            commonattr: COMMON,
            volattr: 0,
            dirattr: 0,
            fileattr: FILE,
            forkattr: EXT,
        };
        let mut b = [0u8; 96];
        crate::descriptor::syscall(|| unsafe {
            libc::getattrlistat(
                fd.as_raw_fd(),
                name.as_c_str().as_ptr(),
                (&mut list as *mut libc::attrlist).cast(),
                b.as_mut_ptr().cast(),
                b.len(),
                0x1 | 0x20,
            )
        })?;
        parse(&b, identity, length, allocated, hard_link)
    }
    fn probe(s: &Snapshot, id: u32, fd: &OwnedFd) -> io::Result<CloneMetadata> {
        let n = s.node(id).unwrap();
        read(
            fd,
            n.native_name(),
            &n.identity,
            n.length,
            n.own_allocated_size,
            n.hard_link,
        )
    }
    pub(super) fn prepare_entry(
        e: &crate::Entry,
        fd: &OwnedFd,
        caps: &std::sync::Mutex<HashMap<u64, bool>>,
        queries: &mut u64,
    ) -> io::Result<Option<CloneMetadata>> {
        let known = caps.lock().unwrap().get(&e.device).copied();
        if known == Some(false) {
            return Ok(None);
        }
        *queries += 1;
        let result = read(
            fd,
            e.path
                .file_name()
                .ok_or_else(|| error("missing basename"))?,
            &e.identity(),
            e.length,
            e.allocated,
            e.hard_link,
        );
        settle(e.device, result, caps)
    }
    /// Cache a device as unsupported only on a volume-level signal: the syscall's own
    /// ENOTSUP/EINVAL, or a reply without clone fields from a device that never answered
    /// with them. The same reply from a device that already did is that one file's fault.
    fn settle(
        device: u64,
        result: io::Result<CloneMetadata>,
        caps: &std::sync::Mutex<HashMap<u64, bool>>,
    ) -> io::Result<Option<CloneMetadata>> {
        let mut caps = caps.lock().unwrap();
        match result {
            Ok(m) => {
                caps.insert(device, true);
                Ok(Some(m))
            }
            Err(failure) if is_unsupported(&failure) => {
                if failure.raw_os_error().is_none() && caps.get(&device) == Some(&true) {
                    return Err(error("clone attributes missing from one file's reply"));
                }
                caps.insert(device, false);
                Ok(None)
            }
            Err(failure) => Err(failure),
        }
    }
    pub(super) fn vouch_reused(
        entries: &mut [io::Result<crate::Entry>],
        fd: &OwnedFd,
        caps: &std::sync::Mutex<HashMap<u64, bool>>,
        cancel: &AtomicBool,
        observations: &mut std::collections::VecDeque<(usize, CloneMetadata)>,
        buffer: &mut Vec<u8>,
    ) {
        let mut wanted = HashMap::new();
        let mut device = None;
        for (i, entry) in entries.iter().enumerate() {
            let Ok(e) = entry else { continue };
            if e.kind == NodeKind::File && !e.clone_checked {
                let Some(name) = e.path.file_name() else {
                    return;
                };
                if wanted.insert(name.as_bytes(), i).is_some() {
                    return;
                }
                device = Some(e.device);
            }
        }
        // No files to probe, or a volume that already answered "unsupported" (one cheap probe each).
        let Some(device) = device else { return };
        if caps.lock().unwrap().get(&device) == Some(&false) {
            return;
        }
        let mut matched = vec![false; entries.len()];
        let mut shared = Vec::new();
        // The worker's bulk buffer is lent for the read so each reused folder avoids a fresh 64 KiB.
        let mut stats = crate::bulk::CloneReadStats::with_buffer(std::mem::take(buffer));
        let result = crate::bulk::visit_clone_records(fd, cancel, &mut stats, |r| {
            let Some(&i) = wanted.get(r.name) else {
                return Ok(());
            };
            let e = entries[i].as_ref().unwrap();
            // The same facts the per-file probe and final validation insist on.
            if r.clone_checked
                && same_file(r, e.device, e.inode, e.allocated, e.hard_link)
                && same_length(r, e.length)
            {
                if std::mem::replace(&mut matched[i], true) {
                    return Err(error("duplicate clone record in reused directory"));
                }
                shared.extend(r.clone_metadata.clone().map(|m| (i, m)));
            }
            Ok(())
        });
        drop(wanted);
        *buffer = stats.into_buffer();
        // A failed read vouches for nothing: every file keeps its per-file probe.
        if result.is_ok() {
            for (entry, _) in entries.iter_mut().zip(&matched).filter(|(_, m)| **m) {
                entry.as_mut().unwrap().clone_checked = true;
            }
            shared.sort_unstable_by_key(|(i, _)| *i);
            observations.extend(shared);
        }
    }
    fn warning(info: &mut crate::AccountingInfo, e: io::Error) {
        info.warn(e.to_string());
    }
    // One definition of the record facts the vouch and the final validation both check.
    fn same_file(
        r: &crate::bulk::Record,
        device: u64,
        inode: u64,
        allocated: u64,
        hard_link: bool,
    ) -> bool {
        r.kind == 1
            && r.device == device
            && r.inode == inode
            && r.allocated == allocated
            && (r.links > 1) == hard_link
    }
    fn same_length(r: &crate::bulk::Record, length: u64) -> bool {
        r.length == length
            || r.clone_metadata
                .as_ref()
                .is_some_and(|m| m.data_length == length)
    }
    fn same_observation(a: &CloneMetadata, b: &CloneMetadata) -> bool {
        a.clone_id == b.clone_id
            && a.data_allocated_size == b.data_allocated_size
            && a.data_length == b.data_length
            && a.reference_count == b.reference_count
            && a.may_share_blocks == b.may_share_blocks
    }
    pub(super) fn validate(
        s: &Snapshot,
        root: &OwnedFd,
        cancel: &AtomicBool,
        qos: crate::ScanQos,
    ) -> Validation {
        validate_with_hook(s, root, cancel, qos, || {})
    }
    fn validate_with_hook(
        s: &Snapshot,
        root: &OwnedFd,
        cancel: &AtomicBool,
        qos: crate::ScanQos,
        before: impl FnOnce(),
    ) -> Validation {
        validate_core(
            s,
            root,
            cancel,
            before,
            |_| {},
            ValidationStrategy {
                qos,
                ..ValidationStrategy::automatic()
            },
        )
    }
    #[cfg(test)]
    fn validate_with_file_hook(
        s: &Snapshot,
        root: &OwnedFd,
        cancel: &AtomicBool,
        before: impl FnOnce(),
        before_file: impl FnMut(u32) + Send,
    ) -> Validation {
        validate_core(
            s,
            root,
            cancel,
            before,
            before_file,
            ValidationStrategy::direct(),
        )
    }
    type BulkReader = fn(
        &OwnedFd,
        &AtomicBool,
        &mut crate::bulk::CloneReadStats,
        &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
    ) -> io::Result<()>;
    fn read_bulk(
        fd: &OwnedFd,
        cancel: &AtomicBool,
        stats: &mut crate::bulk::CloneReadStats,
        visitor: &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
    ) -> io::Result<()> {
        crate::bulk::visit_clone_records(fd, cancel, stats, visitor)
    }
    #[derive(Clone, Copy)]
    struct ValidationStrategy {
        bulk: bool,
        adaptive: bool,
        reader: BulkReader,
        qos: crate::ScanQos,
    }
    impl ValidationStrategy {
        fn automatic() -> Self {
            Self {
                bulk: true,
                adaptive: true,
                reader: read_bulk,
                qos: crate::ScanQos::Automatic,
            }
        }
        #[cfg(test)]
        fn direct() -> Self {
            Self {
                bulk: false,
                adaptive: false,
                reader: read_bulk,
                qos: crate::ScanQos::Automatic,
            }
        }
        #[cfg(test)]
        fn batch(reader: BulkReader) -> Self {
            Self {
                bulk: true,
                adaptive: false,
                reader,
                qos: crate::ScanQos::Automatic,
            }
        }
        fn use_batch(&self, wanted: usize, children: usize) -> bool {
            self.bulk && (!self.adaptive || (wanted >= 4 && wanted >= children.div_ceil(4)))
        }
    }
    struct MemberContext<'a> {
        s: &'a Snapshot,
        root: &'a OwnedFd,
        cancel: &'a AtomicBool,
        strategy: ValidationStrategy,
        cache_limit: usize,
    }
    struct MemberWorker {
        info: crate::AccountingInfo,
        parents: ParentCache,
        read_stats: crate::bulk::CloneReadStats,
        peak: usize,
        active: Vec<usize>,
        seen: Vec<bool>,
        fallback: Vec<usize>,
    }
    impl MemberWorker {
        fn begin_task(&mut self) {
            // Another worker may have processed intervening ranges. A repeated
            // parent ID still needs fresh root and cached-ancestor checks.
            self.parents.current = None;
        }
        fn new(cache_limit: usize) -> Self {
            Self {
                info: crate::AccountingInfo {
                    checks_complete: true,
                    ..Default::default()
                },
                parents: ParentCache::with_limit(cache_limit),
                read_stats: Default::default(),
                peak: 0,
                active: Vec::new(),
                seen: Vec::new(),
                fallback: Vec::new(),
            }
        }
    }
    struct ParentTask {
        parent_id: u32,
        range: std::ops::Range<usize>,
        query_work: usize,
        statuses: std::sync::Mutex<Vec<ClaimStatus>>,
    }
    fn parent_tasks(
        s: &Snapshot,
        claims: &[(u32, CloneMetadata)],
        statuses: &[ClaimStatus],
        strategy: ValidationStrategy,
    ) -> Vec<ParentTask> {
        let mut tasks = Vec::new();
        let mut start = 0;
        while start < claims.len() {
            let parent = s.node(claims[start].0).unwrap().parent;
            let mut end = start + 1;
            while end < claims.len() && s.node(claims[end].0).unwrap().parent == parent {
                end += 1;
            }
            let active = statuses[start..end]
                .iter()
                .filter(|status| matches!(status, ClaimStatus::Unverified | ClaimStatus::Verified))
                .count();
            let children = parent
                .and_then(|id| s.node(id))
                .map_or(0, |n| n.children().len());
            let query_work = if active == 0 {
                0
            } else if strategy.use_batch(active, children) {
                children
            } else {
                active
            };
            tasks.push(ParentTask {
                parent_id: parent.unwrap_or(0),
                range: start..end,
                query_work,
                statuses: std::sync::Mutex::new(statuses[start..end].to_vec()),
            });
            start = end;
        }
        prioritize_tasks(&mut tasks);
        tasks
    }
    fn prioritize_tasks(tasks: &mut [ParentTask]) {
        // Start expensive whole parents early. Nearby directory IDs group sibling
        // parents for the bounded ancestry cache; original claim ranges stay intact.
        tasks.sort_unstable_by(|a, b| {
            let a_heavy = a.query_work >= 4096;
            let b_heavy = b.query_work >= 4096;
            b_heavy
                .cmp(&a_heavy)
                .then_with(|| {
                    if a_heavy && b_heavy {
                        b.query_work.cmp(&a.query_work)
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| {
                    if !a_heavy && !b_heavy {
                        a.parent_id.cmp(&b.parent_id)
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| a.range.start.cmp(&b.range.start))
        });
    }
    fn validation_workers(s: &Snapshot, bulk: bool, claims: usize, available: usize) -> usize {
        if bulk && claims >= 4096 {
            s.worker_stats
                .len()
                .clamp(1, crate::MAX_WORKERS)
                .min((available / 2).max(1))
        } else {
            1
        }
    }
    fn validate_members(
        ctx: &MemberContext<'_>,
        claims: &[(u32, CloneMetadata)],
        statuses: &mut [ClaimStatus],
        before_file: &mut impl FnMut(u32),
        worker: &mut MemberWorker,
    ) {
        let MemberContext {
            s,
            root,
            cancel,
            strategy,
            cache_limit: _,
        } = *ctx;
        let ValidationStrategy { bulk, reader, .. } = strategy;
        let MemberWorker {
            info,
            parents,
            read_stats,
            peak: member_peak,
            active,
            seen,
            fallback,
        } = worker;
        let mut start = 0;
        while start < claims.len() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let parent_id = s.node(claims[start].0).unwrap().parent;
            let mut end = start + 1;
            while end < claims.len() && s.node(claims[end].0).unwrap().parent == parent_id {
                end += 1;
            }
            active.clear();
            seen.clear();
            fallback.clear();
            active.extend((start..end).filter(|i| {
                matches!(
                    statuses[*i],
                    ClaimStatus::Unverified | ClaimStatus::Verified
                )
            }));
            *member_peak = (*member_peak).max(
                active.capacity() * std::mem::size_of::<usize>()
                    + seen.capacity() * std::mem::size_of::<bool>()
                    + fallback.capacity() * std::mem::size_of::<usize>()
                    + read_stats.workspace_bytes()
                    + parents.peak_bytes,
            );
            if active.is_empty() {
                start = end;
                continue;
            }
            let started = std::time::Instant::now();
            info.revalidation_queries += active.len() as u64;
            let id = claims[active[0]].0;
            if parents.current != parent_id {
                info.revalidation_parent_walks += 1;
            }
            let batch = strategy.use_batch(
                active.len(),
                parent_id
                    .and_then(|id| s.node(id))
                    .map_or(0, |n| n.children().len()),
            );
            let result = if batch {
                parents.fresh_cursor(s, id, root)
            } else {
                parents.ensure(s, id, root)
            };
            let mut direct = !batch;
            let mut local_peak = 0;
            let parent_valid = result.is_ok();
            if let Err(e) = result {
                warning(info, e);
            } else if batch {
                before_file(id);
                let parent_fd = parents.fd(root);
                let expected_parent = s.node(parent_id.unwrap()).unwrap().identity.clone();
                let mut wanted = HashMap::new();
                let mut duplicate = false;
                for &i in active.iter() {
                    let n = s.node(claims[i].0).unwrap();
                    duplicate |= wanted.insert(n.native_name().as_bytes(), i).is_some();
                }
                seen.resize(end - start, false);
                let queries_before = read_stats.queries;
                read_stats.peak_bytes = 0;
                let result = if duplicate {
                    Err(error("duplicate clone basename in one directory"))
                } else {
                    crate::descriptor::identity_fd(parent_fd).and_then(|actual| {
                        if actual != expected_parent {
                            return Err(error("clone parent identity changed"));
                        }
                        reader(parent_fd, cancel, read_stats, &mut |r| {
                            let Some(&i) = wanted.get(r.name) else {
                                return Ok(());
                            };
                            if std::mem::replace(&mut seen[i - start], true) {
                                return Err(error(
                                    "duplicate clone record in directory validation",
                                ));
                            }
                            let n = s.node(claims[i].0).unwrap();
                            if !same_file(
                                r,
                                n.identity.device,
                                n.identity.inode,
                                n.own_allocated_size,
                                n.hard_link,
                            ) {
                                warning(info, error("clone file identity, sizes or link count changed during validation"));
                                return Ok(());
                            }
                            if !r.clone_checked {
                                fallback.push(i);
                            } else if same_length(r, n.length)
                                && r.clone_metadata
                                    .as_ref()
                                    .is_some_and(|m| same_observation(m, &claims[i].1))
                            {
                                statuses[i] = ClaimStatus::Verified;
                            } else {
                                warning(info, error("clone metadata or sizes changed during validation"));
                            }
                            Ok(())
                        })
                    })
                };
                info.revalidation_bulk_queries += read_stats.queries - queries_before;
                local_peak += wanted.capacity() * std::mem::size_of::<(&[u8], usize)>()
                    + read_stats
                        .peak_bytes
                        .saturating_sub(read_stats.workspace_bytes());
                match result {
                    Ok(()) => {}
                    Err(e) => {
                        for &i in active.iter() {
                            statuses[i] = ClaimStatus::Unverified;
                        }
                        fallback.clear();
                        if is_unsupported(&e) {
                            direct = true;
                        } else {
                            warning(info, e);
                        }
                    }
                }
            } else {
                direct = true;
            }
            // Unsupported rich validation and explicitly omitted attributes use
            // the original per-file reader, through the same checked parent.
            if parent_valid {
                let to_probe: &[usize] = if direct { active } else { fallback };
                for &i in to_probe {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let id = claims[i].0;
                    if !bulk {
                        before_file(id);
                    }
                    info.revalidation_direct_queries += 1;
                    match probe(s, id, parents.fd(root)) {
                        Ok(actual) if same_observation(&actual, &claims[i].1) => {
                            statuses[i] = ClaimStatus::Verified
                        }
                        Ok(_) => warning(info, error("clone metadata changed during validation")),
                        Err(e) => warning(info, e),
                    }
                }
            }
            info.revalidation_us = info
                .revalidation_us
                .saturating_add(started.elapsed().as_micros() as u64);
            // Retained workspace is live even in a later direct/failing parent.
            local_peak += read_stats.workspace_bytes()
                + active.capacity() * std::mem::size_of::<usize>()
                + seen.capacity() * std::mem::size_of::<bool>()
                + fallback.capacity() * std::mem::size_of::<usize>();
            *member_peak = (*member_peak).max(local_peak + parents.peak_bytes);
            start = end;
        }
    }
    /// Only groups with multiple observed inodes can deduct data allocation.
    /// All aliases of a potentially deducting group still need validation.
    /// Singleton metadata stays present but never becomes verified or an owner.
    fn retain_singleton_allocation(
        s: &Snapshot,
        claims: &[(u32, CloneMetadata)],
        statuses: &mut [ClaimStatus],
    ) -> usize {
        let mut groups: HashMap<(u64, u64), (u64, bool)> = HashMap::new();
        for (index, (id, metadata)) in claims.iter().enumerate() {
            if statuses[index] == ClaimStatus::Drop {
                continue;
            }
            let n = s.node(*id).unwrap();
            let group = groups
                .entry((n.identity.device, metadata.clone_id))
                .or_insert((n.identity.inode, false));
            group.1 |= group.0 != n.identity.inode;
        }
        for (index, (id, metadata)) in claims.iter().enumerate() {
            if statuses[index] != ClaimStatus::Drop
                && !groups[&(s.node(*id).unwrap().identity.device, metadata.clone_id)].1
            {
                statuses[index] = ClaimStatus::Raw;
            }
        }
        groups.capacity() * std::mem::size_of::<((u64, u64), (u64, bool))>()
    }
    fn validate_core(
        s: &Snapshot,
        root: &OwnedFd,
        cancel: &AtomicBool,
        before: impl FnOnce(),
        mut before_file: impl FnMut(u32) + Send,
        strategy: ValidationStrategy,
    ) -> Validation {
        let bulk = strategy.bulk;
        before();
        let claims = &s.tree.clones;
        let mut statuses = vec![ClaimStatus::Unverified; claims.len()];
        let mut info = s.accounting.clone();
        info.checks_complete &= !cancel.load(Ordering::Relaxed);
        // Conflicting aliases indicate mutation; discard that clone group's claims,
        // preserving unrelated groups and the historical hard-link policy.
        let mut aliases = HashMap::new();
        let mut conflicting = std::collections::HashSet::new();
        for (id, m) in &s.tree.clones {
            let n = s.node(*id).unwrap();
            aliases
                .entry((n.identity.device, n.identity.inode))
                .or_insert((*id, m));
        }
        let alias_lookup = super::accounting::AliasLookup::new(&aliases);
        let mut alias_claims = super::accounting::ClaimCursor::new(claims);
        for n in s.iter_nodes().filter(|n| n.identity.kind == NodeKind::File) {
            let current = alias_claims.get(n.id);
            if let Some((first, m)) = alias_lookup.get(&(n.identity.device, n.identity.inode)) {
                let prior = s.node(*first).unwrap();
                if current != Some(*m)
                    || n.length != prior.length
                    || n.own_allocated_size != prior.own_allocated_size
                    || (n.id != *first && (!n.hard_link || !prior.hard_link))
                {
                    conflicting.insert((n.identity.device, m.clone_id));
                    if let Some(current) = current {
                        conflicting.insert((n.identity.device, current.clone_id));
                    }
                }
            }
        }
        if !conflicting.is_empty() {
            for (index, (id, m)) in claims.iter().enumerate() {
                if conflicting.contains(&(s.tree.records[*id as usize].device, m.clone_id)) {
                    statuses[index] = ClaimStatus::Drop;
                }
            }
            warning(
                &mut info,
                error("clone inode aliases changed during enumeration"),
            );
        }
        let status_bytes = statuses.capacity() * std::mem::size_of::<ClaimStatus>();
        let alias_peak = status_bytes
            + aliases.capacity() * std::mem::size_of::<((u64, u64), (u32, &CloneMetadata))>()
            + conflicting.capacity() * std::mem::size_of::<(u64, u64)>();
        drop(aliases);
        drop(conflicting);
        // Alias conflicts must be established first. A singleton has no second
        // in-scope inode to deduct from, including when its other clones are
        // outside this root. Keep its observation unverified and retain raw.
        let singleton_peak = status_bytes + retain_singleton_allocation(s, claims, &mut statuses);
        // Each worker owns disjoint status slots and directory caches. No tree
        // mutation occurs until all workers join and whole-group rejection runs.
        let available = s.descriptor_limit.unwrap_or(1).saturating_sub(1);
        let workers = validation_workers(s, bulk, claims.len(), available);
        let cache_limit = (available / workers).saturating_sub(1).min(32);
        let ctx = MemberContext {
            s,
            root,
            cancel,
            strategy,
            cache_limit,
        };
        // Sum of worker peak requested payloads is a conservative bound; their
        // individual peaks need not occur at the same instant.
        let mut member_peak = 0;
        info.revalidation_workers = workers as u32;
        let validating = std::time::Instant::now();
        let mut merge = |part: crate::AccountingInfo, peak| {
            info.checks_complete &= part.checks_complete;
            info.revalidation_queries += part.revalidation_queries;
            info.revalidation_parent_walks += part.revalidation_parent_walks;
            info.revalidation_us = info.revalidation_us.saturating_add(part.revalidation_us);
            info.revalidation_bulk_queries += part.revalidation_bulk_queries;
            info.revalidation_direct_queries += part.revalidation_direct_queries;
            let remaining = 32usize.saturating_sub(info.warnings.len());
            info.warnings
                .extend(part.warnings.into_iter().take(remaining));
            member_peak += peak;
        };
        let mut scheduler_bytes = 0;
        if workers == 1 {
            // Thread-scoped I/O policies must not stick to the caller (a reused pool thread).
            let worker = std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        crate::diagnostics::apply_qos(ctx.strategy.qos);
                        crate::diagnostics::disable_dataless_materialization();
                        let mut worker = MemberWorker::new(cache_limit);
                        validate_members(
                            &ctx,
                            claims,
                            &mut statuses,
                            &mut before_file,
                            &mut worker,
                        );
                        worker
                    })
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            });
            merge(worker.info, worker.peak);
        } else {
            // A task is an entire contiguous parent range. No split can turn a
            // dense parent into repeated bulk enumerations or sparse probes.
            let tasks = parent_tasks(s, claims, &statuses, strategy);
            scheduler_bytes = tasks.capacity() * std::mem::size_of::<ParentTask>()
                + tasks
                    .iter()
                    .map(|task| {
                        task.statuses.lock().unwrap().capacity()
                            * std::mem::size_of::<ClaimStatus>()
                    })
                    .sum::<usize>()
                + workers * std::mem::size_of::<std::thread::ScopedJoinHandle<'_, MemberWorker>>()
                + std::mem::size_of::<std::sync::atomic::AtomicUsize>();
            let next = std::sync::atomic::AtomicUsize::new(0);
            std::thread::scope(|scope| {
                let handles: Vec<_> = (0..workers)
                    .map(|_| {
                        let ctx = &ctx;
                        let tasks = &tasks;
                        let next = &next;
                        scope.spawn(move || {
                            crate::diagnostics::apply_qos(ctx.strategy.qos);
                            crate::diagnostics::disable_dataless_materialization();
                            let mut worker = MemberWorker::new(ctx.cache_limit);
                            while !ctx.cancel.load(Ordering::Relaxed) {
                                let index = next.fetch_add(1, Ordering::Relaxed);
                                let Some(task) = tasks.get(index) else {
                                    break;
                                };
                                let mut statuses = task.statuses.lock().unwrap();
                                worker.begin_task();
                                validate_members(
                                    ctx,
                                    &claims[task.range.clone()],
                                    &mut statuses,
                                    &mut |_| {},
                                    &mut worker,
                                );
                            }
                            worker
                        })
                    })
                    .collect();
                for handle in handles {
                    let worker = handle.join().expect("clone validation worker");
                    merge(worker.info, worker.peak);
                }
            });
            // Publication remains serial and private until every worker exits.
            for task in tasks {
                statuses[task.range].copy_from_slice(&task.statuses.into_inner().unwrap());
            }
        }
        member_peak += status_bytes + scheduler_bytes;
        info.revalidation_wall_us = validating.elapsed().as_micros() as u64;
        if cancel.load(Ordering::Relaxed) {
            for status in &mut statuses {
                if *status != ClaimStatus::Drop {
                    *status = ClaimStatus::Unverified;
                }
            }
            warning(&mut info, error("clone validation cancelled"));
        }
        let mut invalid = std::collections::HashSet::new();
        for (i, (id, m)) in claims.iter().enumerate() {
            if statuses[i] == ClaimStatus::Unverified {
                invalid.insert((s.tree.records[*id as usize].device, m.clone_id));
            }
        }
        let mut group_evidence = HashMap::new();
        for (index, (id, m)) in claims.iter().enumerate() {
            if statuses[index] == ClaimStatus::Drop {
                continue;
            }
            let device = s.node(*id).unwrap().identity.device;
            let evidence = (m.data_length, m.data_allocated_size);
            if group_evidence
                .insert((device, m.clone_id), evidence)
                .is_some_and(|prior| prior != evidence)
            {
                invalid.insert((device, m.clone_id));
            }
        }
        if !invalid.is_empty() {
            warning(
                &mut info,
                error("clone group validation incomplete or inconsistent"),
            );
        }
        for (index, (id, m)) in claims.iter().enumerate() {
            if statuses[index] != ClaimStatus::Drop
                && invalid.contains(&(s.tree.records[*id as usize].device, m.clone_id))
            {
                statuses[index] = ClaimStatus::Unverified;
            }
        }
        let group_peak = status_bytes
            + invalid.capacity() * std::mem::size_of::<(u64, u64)>()
            + group_evidence.capacity() * std::mem::size_of::<((u64, u64), (u64, u64))>();
        info.grouping_peak_bytes = info.grouping_peak_bytes.max(
            alias_peak
                .max(singleton_peak)
                .max(member_peak)
                .max(group_peak),
        );
        Validation {
            statuses,
            accounting: info,
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::ffi::CString;
        struct Mounted(std::path::PathBuf);
        impl Drop for Mounted {
            fn drop(&mut self) {
                let _ = std::process::Command::new("hdiutil")
                    .args(["detach", "-quiet", "-force"])
                    .arg(&self.0)
                    .status();
            }
        }
        /// An attached HFS+ image, whose replies carry no clone fields: (scratch dir, mount).
        /// `None` when hdiutil cannot make one. The mount detaches on drop.
        fn hfs_volume(label: &str) -> Option<(std::path::PathBuf, Mounted)> {
            let base = std::env::temp_dir().join(format!("clone-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).unwrap();
            let image = base.join("volume.dmg");
            let mount = base.join("mount");
            let created = std::process::Command::new("hdiutil")
                .args(["create", "-quiet", "-size", "8m", "-fs", "HFS+", "-volname"])
                .arg("CloneHFS")
                .arg(&image)
                .status()
                .is_ok_and(|s| s.success());
            let attached = created
                && std::process::Command::new("hdiutil")
                    .args(["attach", "-quiet", "-nobrowse", "-mountpoint"])
                    .arg(&mount)
                    .arg(&image)
                    .status()
                    .is_ok_and(|s| s.success());
            if !attached {
                eprintln!("skipping: hdiutil could not create or attach an HFS+ image");
                let _ = std::fs::remove_dir_all(&base);
                return None;
            }
            Some((base, Mounted(mount)))
        }
        fn bytes(n: &crate::NodeRef<'_>) -> Vec<u8> {
            let mut b = vec![];
            for value in [96, COMMON, 0, 0, FILE, EXT, n.identity.device as u32, 1] {
                b.extend(value.to_ne_bytes());
            }
            b.extend(n.identity.inode.to_ne_bytes());
            b.extend(if n.hard_link { 2u32 } else { 1u32 }.to_ne_bytes());
            for value in [n.length, n.own_allocated_size, n.length, 100, 42, 1] {
                b.extend(value.to_ne_bytes());
            }
            b.extend(2u32.to_ne_bytes());
            b
        }
        #[test]
        fn packed_parser_rejects_missing_truncated_negative_and_changed_identity() {
            let s = super::super::tests::fixture();
            let n = s.node(2).unwrap();
            let b = bytes(&n);
            assert!(parse(&b, &n.identity, n.length, n.own_allocated_size, n.hard_link).is_ok());
            for len in 0..b.len() {
                assert!(parse(
                    &b[..len],
                    &n.identity,
                    n.length,
                    n.own_allocated_size,
                    n.hard_link
                )
                .is_err());
            }
            let mut link_changed = b.clone();
            link_changed[40..44].copy_from_slice(&2u32.to_ne_bytes());
            assert!(parse(
                &link_changed,
                &n.identity,
                n.length,
                n.own_allocated_size,
                n.hard_link
            )
            .is_err());
            for offset in [4, 8, 12, 16, 20, 24, 28, 32] {
                let mut corrupt = b.clone();
                corrupt[offset] ^= 1;
                assert!(
                    parse(
                        &corrupt,
                        &n.identity,
                        n.length,
                        n.own_allocated_size,
                        n.hard_link
                    )
                    .is_err(),
                    "offset {offset}"
                );
            }
            for offset in [44, 52, 60, 68] {
                let mut corrupt = b.clone();
                corrupt[offset..offset + 8].copy_from_slice(&(-1i64).to_ne_bytes());
                assert!(parse(
                    &corrupt,
                    &n.identity,
                    n.length,
                    n.own_allocated_size,
                    n.hard_link
                )
                .is_err());
            }
        }
        #[test]
        fn omitted_clone_fields_classify_as_unsupported_volume() {
            let s = super::super::tests::fixture();
            let n = s.node(2).unwrap();
            let parse_ext = |ext: u32| {
                let mut b = bytes(&n);
                b[20..24].copy_from_slice(&ext.to_ne_bytes());
                // Unsupported volumes also return a shorter reply.
                b[0..4].copy_from_slice(&80u32.to_ne_bytes());
                parse(&b, &n.identity, n.length, n.own_allocated_size, n.hard_link).unwrap_err()
            };
            for ext in [0, 0x200] {
                assert!(is_unsupported(&parse_ext(ext)), "ext {ext:#x}");
            }
            // One clone field without the other is malformed, not unsupported.
            for ext in [0x100, 0x1000, 0x1200] {
                assert!(!is_unsupported(&parse_ext(ext)), "ext {ext:#x}");
            }
            for code in [libc::ENOTSUP, libc::EOPNOTSUPP, libc::EINVAL] {
                assert!(is_unsupported(&io::Error::from_raw_os_error(code)));
            }
            assert!(!is_unsupported(&io::Error::from_raw_os_error(libc::EACCES)));
            assert!(!is_unsupported(&error("missing clone attributes")));
        }
        #[test]
        fn one_reply_without_clone_fields_does_not_disable_a_device_that_answered() {
            let mut s = super::super::tests::fixture();
            let n = s.node(2).unwrap();
            let answer = || {
                parse(
                    &bytes(&n),
                    &n.identity,
                    n.length,
                    n.own_allocated_size,
                    n.hard_link,
                )
                .unwrap()
            };
            let caps = std::sync::Mutex::new(HashMap::new());
            let known = |device| caps.lock().unwrap().get(&device).copied();
            // A device that never answered: the omission is a volume signal, cached once.
            assert!(settle(1, Err(unsupported()), &caps).unwrap().is_none());
            assert_eq!(known(1), Some(false));
            // A device that did answer: one file's omission is that file's warning only.
            assert!(settle(2, Ok(answer()), &caps).unwrap().is_some());
            let failure = settle(2, Err(unsupported()), &caps).unwrap_err();
            assert_eq!(known(2), Some(true));
            assert!(settle(2, Ok(answer()), &caps).unwrap().is_some());
            s.accounting.checks_complete = true;
            super::super::commit(&mut s, 2, Err(failure));
            assert!(!s.accounting.checks_complete);
            assert_eq!(s.accounting.warnings.len(), 1);
            // The syscall's own rejection is still volume-level, even after an answer.
            for code in [libc::ENOTSUP, libc::EINVAL] {
                caps.lock().unwrap().insert(2, true);
                let rejected = io::Error::from_raw_os_error(code);
                assert!(settle(2, Err(rejected), &caps).unwrap().is_none());
                assert_eq!(known(2), Some(false));
            }
        }
        #[test]
        fn a_file_without_clone_fields_beside_bulk_checked_files_warns_without_disabling_the_device(
        ) {
            let Some((base, guard)) = hfs_volume("hfs-answered") else {
                return;
            };
            // Descriptor opens refuse symlinked components such as /var.
            let mount = std::fs::canonicalize(&guard.0).unwrap();
            for name in ["answered", "odd"] {
                std::fs::write(mount.join(name), vec![1u8; 10_000]).unwrap();
            }
            let identity = crate::platform_identity(&mount).unwrap();
            let fd = crate::descriptor::open(&mount, &identity).unwrap();
            let entry = |name: &str| {
                crate::descriptor::entry_at(&fd, &mount, std::ffi::OsStr::new(name)).unwrap()
            };
            let run = |entries: Vec<io::Result<crate::Entry>>, caps: &_| {
                super::super::prepare(
                    &entries,
                    &fd,
                    caps,
                    &AtomicBool::new(false),
                    Default::default(),
                    true,
                )
            };
            // Control: an HFS+ reply without clone fields, from a device that never answered, is
            // a volume signal and caches the device as unsupported without a warning.
            let caps = std::sync::Mutex::new(HashMap::new());
            let mut batch = run(vec![Ok(entry("odd"))], &caps);
            assert!(batch.take_at(0).is_none());
            assert_eq!(caps.lock().unwrap().get(&identity.device), Some(&false));
            // The bulk listing returned a clone-checked file on this device, so the same reply
            // for the other file is that file's warning and the device keeps answering.
            let mut checked = entry("answered");
            checked.clone_checked = true;
            let caps = std::sync::Mutex::new(HashMap::new());
            let mut batch = run(vec![Ok(checked), Ok(entry("odd"))], &caps);
            assert_eq!(caps.lock().unwrap().get(&identity.device), Some(&true));
            let failure = batch.take_at(1).unwrap();
            assert!(failure.is_err());
            let mut s = Snapshot::empty(mount.clone());
            s.accounting.checks_complete = true;
            super::super::commit(&mut s, 1, failure);
            assert!(!s.accounting.checks_complete);
            assert_eq!(s.accounting.warnings.len(), 1);
            drop(fd);
            drop(guard);
            let _ = std::fs::remove_dir_all(&base);
        }
        #[test]
        fn files_of_a_recovered_listing_are_not_evidence_the_device_answered() {
            let (p, s, fd) = real_clone_fixture("recovered-evidence");
            let checked = || {
                let mut e =
                    crate::descriptor::entry_at(&fd, &s.root, std::ffi::OsStr::new("a")).unwrap();
                e.clone_checked = true;
                e
            };
            for (answered, expected) in [(false, None), (true, Some(true))] {
                let caps = std::sync::Mutex::new(HashMap::new());
                super::super::prepare(
                    &[Ok(checked())],
                    &fd,
                    &caps,
                    &AtomicBool::new(false),
                    Default::default(),
                    answered,
                );
                assert_eq!(
                    caps.lock().unwrap().get(&checked().device).copied(),
                    expected
                );
            }
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn non_apfs_volume_scans_without_clone_warnings() {
            let Some((base, guard)) = hfs_volume("hfs") else {
                return;
            };
            let mount = guard.0.clone();
            for i in 0..6 {
                std::fs::write(mount.join(format!("f{i}")), vec![i as u8; 10_000]).unwrap();
            }
            let s = crate::scan(
                mount.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            drop(guard);
            let _ = std::fs::remove_dir_all(&base);
            assert!(
                s.accounting.warnings.is_empty(),
                "{:?}",
                s.accounting.warnings
            );
            assert!(s.accounting.checks_complete);
            assert_eq!(s.accounting.deduction_bytes, 0);
            assert!(s.tree.clones.is_empty());
            // The device is cached as unsupported after its first query.
            assert!(s.accounting.metadata_queries < 6);
        }
        fn real_clone_fixture(label: &str) -> (std::path::PathBuf, Snapshot, OwnedFd) {
            let p = std::env::temp_dir().join(format!("clone-{label}-{}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            for (a, b, byte) in [("a", "b", 3u8), ("c", "d", 7u8)] {
                let source = p.join(a);
                let destination = p.join(b);
                std::fs::write(&source, vec![byte; 128 * 1024]).unwrap();
                let ac = CString::new(source.as_os_str().as_bytes()).unwrap();
                let bc = CString::new(destination.as_os_str().as_bytes()).unwrap();
                unsafe extern "C" {
                    fn clonefile(a: *const libc::c_char, b: *const libc::c_char, flags: u32)
                        -> i32;
                }
                assert_eq!(unsafe { clonefile(ac.as_ptr(), bc.as_ptr(), 0) }, 0);
            }
            let mut s = crate::scan(
                p.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            assert!(s.accounting.deduction_bytes > 0);
            for (_, m) in &mut s.tree.clones {
                m.verified = false;
            }
            let fd = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            (p, s, fd)
        }
        #[test]
        fn bulk_validation_matches_direct_queries_and_fails_closed() {
            fn unsupported(
                _: &OwnedFd,
                _: &AtomicBool,
                stats: &mut crate::bulk::CloneReadStats,
                _: &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
            ) -> io::Result<()> {
                stats.queries += 1;
                Err(io::Error::from_raw_os_error(libc::ENOTSUP))
            }
            fn missing(
                fd: &OwnedFd,
                c: &AtomicBool,
                stats: &mut crate::bulk::CloneReadStats,
                visitor: &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
            ) -> io::Result<()> {
                read_bulk(fd, c, stats, &mut |r| {
                    let mut record = r.clone();
                    record.clone_checked = false;
                    record.clone_metadata = None;
                    visitor(&record)
                })
            }
            fn late_error(
                fd: &OwnedFd,
                c: &AtomicBool,
                stats: &mut crate::bulk::CloneReadStats,
                visitor: &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
            ) -> io::Result<()> {
                read_bulk(fd, c, stats, &mut |r| {
                    visitor(r)?;
                    Err(error("malformed later clone record"))
                })
            }
            fn omit(
                fd: &OwnedFd,
                c: &AtomicBool,
                stats: &mut crate::bulk::CloneReadStats,
                visitor: &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
            ) -> io::Result<()> {
                read_bulk(fd, c, stats, &mut |r| {
                    if r.name == b"b" {
                        Ok(())
                    } else {
                        visitor(r)
                    }
                })
            }
            fn duplicate(
                fd: &OwnedFd,
                c: &AtomicBool,
                stats: &mut crate::bulk::CloneReadStats,
                visitor: &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
            ) -> io::Result<()> {
                read_bulk(fd, c, stats, &mut |r| {
                    visitor(r)?;
                    visitor(r)
                })
            }
            let (p, original, fd) = real_clone_fixture("bulk-contract");
            let run = |bulk, reader| {
                let mut s = original.clone();
                s.accounting.revalidation_queries = 0;
                s.accounting.revalidation_bulk_queries = 0;
                s.accounting.revalidation_direct_queries = 0;
                validate_core(
                    &s,
                    &fd,
                    &AtomicBool::new(false),
                    || {},
                    |_| {},
                    if bulk {
                        ValidationStrategy::batch(reader)
                    } else {
                        ValidationStrategy::direct()
                    },
                )
                .apply(&mut s);
                crate::aggregate(&mut s);
                s
            };
            let direct = run(false, read_bulk);
            for reader in [read_bulk as BulkReader, unsupported, missing] {
                let s = run(true, reader);
                assert!(s.accounting.checks_complete, "{:?}", s.accounting.warnings);
                assert_eq!(
                    s.accounting.deduction_bytes,
                    direct.accounting.deduction_bytes
                );
                assert_eq!(s.stats.allocated_bytes, direct.stats.allocated_bytes);
                assert_eq!(s.tree.clone_attributions, direct.tree.clone_attributions);
                assert_eq!(s.accounting.revalidation_queries, 4);
            }
            let normal = run(true, read_bulk);
            assert_eq!(normal.accounting.revalidation_direct_queries, 0);
            assert_eq!(normal.accounting.revalidation_bulk_queries, 2);
            assert!(
                normal.accounting.grouping_peak_bytes >= crate::bulk::BULK_BUFFER_BYTES,
                "retained real read workspace must be counted"
            );
            let mut automatic = original.clone();
            automatic.accounting.revalidation_direct_queries = 0;
            automatic.accounting.revalidation_bulk_queries = 0;
            validate(
                &automatic,
                &fd,
                &AtomicBool::new(false),
                crate::ScanQos::Automatic,
            )
            .apply(&mut automatic);
            crate::aggregate(&mut automatic);
            assert_eq!(
                automatic.stats.allocated_bytes,
                normal.stats.allocated_bytes
            );
            assert_eq!(
                automatic.tree.clone_attributions,
                normal.tree.clone_attributions
            );
            assert_eq!(automatic.accounting.revalidation_direct_queries, 0);
            assert_eq!(automatic.accounting.revalidation_bulk_queries, 2);
            assert!(!ValidationStrategy::automatic().use_batch(2, 100000));
            assert!(!ValidationStrategy::automatic().use_batch(8, 100000));
            assert!(ValidationStrategy::automatic().use_batch(8, 8));
            for reader in [late_error as BulkReader, duplicate] {
                let s = run(true, reader);
                assert!(!s.accounting.checks_complete);
                assert_eq!(s.accounting.deduction_bytes, 0);
                assert_eq!(
                    s.stats.allocated_bytes,
                    crate::hardlink_allocations(&s).unwrap().iter().sum::<u64>()
                );
                assert_eq!(s.stats.errors, 0);
                assert!(!s.stats.incomplete);
                assert!(!s.incremental_accounting_eligible());
            }
            let s = run(true, omit);
            assert!(!s.accounting.checks_complete);
            assert_eq!(s.accounting.deduction_bytes, 128 * 1024);
            assert_eq!(s.stats.errors, 0);
            assert!(!s.stats.incomplete);
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn singleton_pruning_counts_inodes_per_device_and_retains_hardlink_accounting() {
            for separate_device in [false, true] {
                let mut s = super::super::tests::fixture();
                if separate_device {
                    s.update_node(2, |n| n.identity.device = 2).unwrap();
                } else {
                    s.tree
                        .clones
                        .iter_mut()
                        .find(|(id, _)| *id == 2)
                        .unwrap()
                        .1
                        .clone_id = 99;
                }
                let expected = attributed_own_allocations(&s).unwrap();
                let mut statuses = vec![ClaimStatus::Unverified; s.tree.clones.len()];
                let bytes = retain_singleton_allocation(&s, &s.tree.clones, &mut statuses);
                assert!(bytes > 0);
                assert!(statuses.iter().all(|status| *status == ClaimStatus::Raw));
                Validation {
                    statuses,
                    accounting: s.accounting.clone(),
                }
                .apply(&mut s);
                assert_eq!(attributed_own_allocations(&s).unwrap(), expected);
                assert!(allocation_plan(&s).unwrap().clone_attributions.is_empty());
                assert!(s.tree.clones.iter().all(|(_, m)| !m.verified));
                assert!(!s.incremental_accounting_eligible());
            }
            let s = super::super::tests::fixture();
            let mut statuses = vec![ClaimStatus::Unverified; s.tree.clones.len()];
            retain_singleton_allocation(&s, &s.tree.clones, &mut statuses);
            assert!(statuses
                .iter()
                .all(|status| *status == ClaimStatus::Unverified));
            statuses[1] = ClaimStatus::Drop;
            retain_singleton_allocation(&s, &s.tree.clones, &mut statuses);
            assert!(statuses[1] == ClaimStatus::Drop);
            assert!(statuses[0] == ClaimStatus::Raw && statuses[2] == ClaimStatus::Raw);
        }
        #[test]
        fn outside_scope_clones_and_in_scope_hardlinks_keep_raw_without_validation() {
            let (p, _, _) = real_clone_fixture("outside-scope");
            let inside = p.join("inside");
            std::fs::create_dir(&inside).unwrap();
            for name in ["a", "c"] {
                std::fs::rename(p.join(name), inside.join(name)).unwrap();
            }
            std::fs::hard_link(inside.join("a"), inside.join("alias")).unwrap();
            let mut s = crate::scan(
                inside,
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            assert_eq!(s.tree.clones.len(), 3);
            assert!(s.tree.clones.iter().all(|(_, m)| !m.verified));
            assert!(s.accounting.shared);
            assert!(s.accounting.checks_complete);
            assert!(s.accounting.warnings.is_empty());
            assert_eq!(s.accounting.deduction_bytes, 0);
            assert_eq!(s.accounting.revalidation_queries, 0);
            assert_eq!(s.accounting.revalidation_bulk_queries, 0);
            assert_eq!(s.accounting.revalidation_direct_queries, 0);
            assert_eq!(s.accounting.revalidation_parent_walks, 0);
            assert!(s.tree.clone_attributions.is_empty());
            assert_eq!(
                s.stats.allocated_bytes,
                hardlink_allocations(&s).unwrap().iter().sum::<u64>()
            );
            assert!(!s.incremental_accounting_eligible());
            let raw = s.tree.clones.clone();
            let allocated = s.stats.allocated_bytes;
            let fd = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            validate(&s, &fd, &AtomicBool::new(false), crate::ScanQos::Automatic).apply(&mut s);
            crate::aggregate(&mut s);
            assert_eq!(s.tree.clones, raw);
            assert_eq!(s.stats.allocated_bytes, allocated);
            let cancelled = validate(&s, &fd, &AtomicBool::new(true), crate::ScanQos::Automatic);
            assert!(cancelled
                .statuses
                .iter()
                .all(|status| *status == ClaimStatus::Unverified));
            cancelled.apply(&mut s);
            assert!(!s.accounting.checks_complete);
            assert!(s.tree.clones.iter().all(|(_, m)| !m.verified));
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn deducting_groups_validate_hardlink_aliases_and_reject_a_late_bad_alias() {
            let (p, _, _) = real_clone_fixture("alias-validation");
            std::fs::hard_link(p.join("a"), p.join("alias")).unwrap();
            let mut s = crate::scan(
                p.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let root = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            s.accounting.revalidation_queries = 0;
            s.accounting.revalidation_direct_queries = 0;
            let alias = s
                .iter_nodes()
                .find(|n| n.native_name() == "alias")
                .unwrap()
                .id;
            let bad_clone = s.node(alias).unwrap().clone_metadata().unwrap().clone_id;
            let mut visits = Vec::new();
            validate_with_file_hook(
                &s,
                &root,
                &AtomicBool::new(false),
                || {},
                |id| {
                    visits.push(id);
                    if id == alias {
                        std::fs::remove_file(p.join("alias")).unwrap();
                        std::fs::write(p.join("alias"), b"replacement").unwrap();
                    }
                },
            )
            .apply(&mut s);
            assert_eq!(visits.len(), 5);
            assert_eq!(s.accounting.revalidation_direct_queries, 5);
            assert!(visits.contains(&alias));
            assert!(!s.accounting.checks_complete);
            assert!(s
                .tree
                .clones
                .iter()
                .filter(|(_, m)| m.clone_id == bad_clone)
                .all(|(_, m)| !m.verified));
            assert!(s
                .tree
                .clones
                .iter()
                .filter(|(_, m)| m.clone_id != bad_clone)
                .all(|(_, m)| m.verified));
            crate::aggregate(&mut s);
            assert_eq!(s.accounting.deduction_bytes, 128 * 1024);
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn automatic_validation_batches_four_to_seven_dense_members() {
            for count in 4..=7 {
                let (p, _, _) = real_clone_fixture(&format!("dense-{count}"));
                for i in 4..count {
                    let source = CString::new(p.join("a").as_os_str().as_bytes()).unwrap();
                    let target =
                        CString::new(p.join(format!("extra-{i}")).as_os_str().as_bytes()).unwrap();
                    unsafe extern "C" {
                        fn clonefile(
                            a: *const libc::c_char,
                            b: *const libc::c_char,
                            flags: u32,
                        ) -> i32;
                    }
                    assert_eq!(unsafe { clonefile(source.as_ptr(), target.as_ptr(), 0) }, 0);
                }
                let s = crate::scan(
                    p.clone(),
                    crate::ScanOptions::default(),
                    std::sync::Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .unwrap();
                let root = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
                let run = |strategy| {
                    let mut result = s.clone();
                    result.accounting.revalidation_direct_queries = 0;
                    result.accounting.revalidation_bulk_queries = 0;
                    validate_core(
                        &result,
                        &root,
                        &AtomicBool::new(false),
                        || {},
                        |_| {},
                        strategy,
                    )
                    .apply(&mut result);
                    crate::aggregate(&mut result);
                    result
                };
                let batch = run(ValidationStrategy::automatic());
                let direct = run(ValidationStrategy::direct());
                assert!(batch.accounting.checks_complete);
                assert_eq!(batch.accounting.revalidation_direct_queries, 0);
                assert!(batch.accounting.revalidation_bulk_queries > 0);
                assert_eq!(batch.stats.allocated_bytes, direct.stats.allocated_bytes);
                assert_eq!(
                    batch.tree.clone_attributions,
                    direct.tree.clone_attributions
                );
                assert!(!ValidationStrategy::automatic().use_batch(count, count * 4 + 1));
                std::fs::remove_dir_all(p).unwrap();
            }
        }
        #[test]
        fn light_parent_locality_preserves_original_claim_slots() {
            let mut tasks: Vec<_> = [12, 4, 12, 1, 7]
                .into_iter()
                .enumerate()
                .map(|(index, parent_id)| ParentTask {
                    parent_id,
                    range: index * 2..index * 2 + 2,
                    query_work: 1,
                    statuses: std::sync::Mutex::new(vec![ClaimStatus::Drop, ClaimStatus::Verified]),
                })
                .collect();
            prioritize_tasks(&mut tasks);
            assert_eq!(
                tasks.iter().map(|task| task.parent_id).collect::<Vec<_>>(),
                vec![1, 4, 7, 12, 12],
            );
            assert_eq!(
                tasks
                    .iter()
                    .map(|task| task.range.start)
                    .collect::<Vec<_>>(),
                vec![6, 2, 8, 0, 4],
            );
            let mut restored = [ClaimStatus::Unverified; 10];
            for task in tasks {
                restored[task.range].copy_from_slice(&task.statuses.into_inner().unwrap());
            }
            assert!(restored.iter().enumerate().all(|(index, status)| *status
                == if index % 2 == 0 {
                    ClaimStatus::Drop
                } else {
                    ClaimStatus::Verified
                }));
        }

        #[test]
        fn heavy_parent_priority_preserves_ties_light_order_and_status_ranges() {
            let ranges = [
                (3, 2),
                (8000, 1),
                (1, 3),
                (10000, 2),
                (8000, 1),
                (4095, 2),
                (4096, 1),
            ];
            let mut offset = 0;
            let mut tasks: Vec<_> = ranges
                .into_iter()
                .map(|(query_work, length)| {
                    let range = offset..offset + length;
                    offset += length;
                    ParentTask {
                        parent_id: 0,
                        query_work,
                        statuses: std::sync::Mutex::new(
                            (range.clone())
                                .map(|i| {
                                    if i % 2 == 0 {
                                        ClaimStatus::Drop
                                    } else {
                                        ClaimStatus::Verified
                                    }
                                })
                                .collect(),
                        ),
                        range,
                    }
                })
                .collect();
            prioritize_tasks(&mut tasks);
            assert_eq!(
                tasks.iter().map(|task| task.query_work).collect::<Vec<_>>(),
                vec![10000, 8000, 8000, 4096, 3, 1, 4095]
            );
            assert_eq!(
                tasks
                    .iter()
                    .filter(|task| task.query_work == 8000)
                    .map(|task| task.range.start)
                    .collect::<Vec<_>>(),
                vec![2, 8]
            );
            let mut reconstructed = vec![ClaimStatus::Unverified; offset];
            let mut seen = vec![false; offset];
            for task in tasks {
                for i in task.range.clone() {
                    assert!(!std::mem::replace(&mut seen[i], true));
                }
                reconstructed[task.range].copy_from_slice(&task.statuses.into_inner().unwrap());
            }
            assert!(seen.into_iter().all(|seen| seen));
            assert!(reconstructed
                .into_iter()
                .enumerate()
                .all(|(i, status)| status
                    == if i % 2 == 0 {
                        ClaimStatus::Drop
                    } else {
                        ClaimStatus::Verified
                    }));
        }
        #[test]
        fn validation_worker_policy_respects_size_and_descriptor_headroom() {
            let (p, mut s, _) = real_clone_fixture("worker-policy");
            s.worker_stats = vec![crate::diagnostics::Tracker::new(1).stats; 3];
            assert_eq!(validation_workers(&s, true, 4095, 512), 1);
            assert_eq!(validation_workers(&s, false, 4096, 512), 1);
            assert_eq!(validation_workers(&s, true, 4096, 512), 3);
            for available in 0..8 {
                let workers = validation_workers(&s, true, 4096, available);
                assert!(workers <= (available / 2).max(1));
                assert!(workers >= 1);
            }
            s.worker_stats
                .resize(32, crate::diagnostics::Tracker::new(1).stats);
            assert_eq!(validation_workers(&s, true, 4096, 512), crate::MAX_WORKERS);
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn parallel_validation_keeps_group_rejection_and_stable_owners() {
            for worker_count in [3, 8] {
                let (p, _, _) = real_clone_fixture(&format!("parallel-contract-{worker_count}"));
                unsafe extern "C" {
                    fn clonefile(a: *const libc::c_char, b: *const libc::c_char, flags: u32)
                        -> i32;
                }
                for source in ["a", "c"] {
                    let ac = CString::new(p.join(source).as_os_str().as_bytes()).unwrap();
                    for i in 0..2100 {
                        let bc =
                            CString::new(p.join(format!("{source}-{i}")).as_os_str().as_bytes())
                                .unwrap();
                        assert_eq!(unsafe { clonefile(ac.as_ptr(), bc.as_ptr(), 0) }, 0);
                    }
                }
                let mut original = crate::scan(
                    p.clone(),
                    crate::ScanOptions {
                        workers: worker_count,
                        ..Default::default()
                    },
                    std::sync::Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .unwrap();
                // Even thousands of claims in one parent form exactly one task:
                // do not repeat its listing when increasing worker parallelism.
                static READS: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(0);
                fn counted(
                    fd: &OwnedFd,
                    c: &AtomicBool,
                    stats: &mut crate::bulk::CloneReadStats,
                    visitor: &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
                ) -> io::Result<()> {
                    READS.fetch_add(1, Ordering::Relaxed);
                    read_bulk(fd, c, stats, visitor)
                }
                READS.store(0, Ordering::Relaxed);
                let root_fd =
                    crate::descriptor::open(&original.root, &original.node(0).unwrap().identity)
                        .unwrap();
                validate_core(
                    &original,
                    &root_fd,
                    &AtomicBool::new(false),
                    || {},
                    |_| {},
                    ValidationStrategy::batch(counted),
                );
                assert_eq!(READS.load(Ordering::Relaxed), 1);
                for i in 0..12 {
                    std::fs::create_dir(p.join(format!("parent-{i:02}"))).unwrap();
                }
                for source in ["a", "c"] {
                    for i in 0..2100 {
                        let name = format!("{source}-{i}");
                        std::fs::rename(
                            p.join(&name),
                            p.join(format!("parent-{:02}", i % 12)).join(&name),
                        )
                        .unwrap();
                    }
                }
                original = crate::scan(
                    p.clone(),
                    crate::ScanOptions {
                        workers: worker_count,
                        ..Default::default()
                    },
                    std::sync::Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .unwrap();
                for (_, m) in &mut original.tree.clones {
                    m.verified = false;
                }
                let tasks = parent_tasks(
                    &original,
                    &original.tree.clones,
                    &vec![ClaimStatus::Unverified; original.tree.clones.len()],
                    ValidationStrategy::batch(read_bulk),
                );
                assert!(tasks.len() >= 13);
                assert_eq!(
                    tasks.iter().map(|task| task.range.len()).sum::<usize>(),
                    original.tree.clones.len()
                );
                for task in tasks {
                    let parent = original
                        .node(original.tree.clones[task.range.start].0)
                        .unwrap()
                        .parent;
                    assert!(original.tree.clones[task.range]
                        .iter()
                        .all(|(id, _)| original.node(*id).unwrap().parent == parent));
                }
                let fd =
                    crate::descriptor::open(&original.root, &original.node(0).unwrap().identity)
                        .unwrap();
                let run = |workers, reader| {
                    let mut s = original.clone();
                    s.worker_stats.truncate(workers);
                    if workers == 1 {
                        s.descriptor_limit = Some(3);
                    }
                    s.accounting.revalidation_bulk_queries = 0;
                    validate_core(
                        &s,
                        &fd,
                        &AtomicBool::new(false),
                        || {},
                        |_| {},
                        ValidationStrategy::batch(reader),
                    )
                    .apply(&mut s);
                    crate::aggregate(&mut s);
                    s
                };
                let serial = run(1, read_bulk);
                let parallel = run(worker_count, read_bulk);
                let mut automatic = original.clone();
                validate(
                    &automatic,
                    &fd,
                    &AtomicBool::new(false),
                    crate::ScanQos::Automatic,
                )
                .apply(&mut automatic);
                crate::aggregate(&mut automatic);
                assert_eq!(
                    automatic.stats.allocated_bytes,
                    parallel.stats.allocated_bytes
                );
                assert_eq!(
                    automatic.tree.clone_attributions,
                    parallel.tree.clone_attributions
                );
                assert!(automatic.accounting.revalidation_bulk_queries > 0);
                assert_eq!(
                    parallel.accounting.revalidation_workers,
                    worker_count as u32
                );
                assert!(parallel.accounting.checks_complete);
                assert_eq!(parallel.stats.allocated_bytes, serial.stats.allocated_bytes);
                assert_eq!(
                    parallel.accounting.deduction_bytes,
                    serial.accounting.deduction_bytes
                );
                assert_eq!(
                    parallel.tree.clone_attributions,
                    serial.tree.clone_attributions
                );
                fn changed(
                    fd: &OwnedFd,
                    c: &AtomicBool,
                    stats: &mut crate::bulk::CloneReadStats,
                    visitor: &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
                ) -> io::Result<()> {
                    read_bulk(fd, c, stats, &mut |r| {
                        let mut record = r.clone();
                        if record.name == b"a-0" {
                            record.kind = 5;
                        }
                        visitor(&record)
                    })
                }
                let changed = run(worker_count, changed);
                let bad = original
                    .iter_nodes()
                    .find(|n| n.native_name() == "a")
                    .unwrap()
                    .clone_metadata()
                    .unwrap()
                    .clone_id;
                assert!(!changed.accounting.checks_complete);
                assert!(changed
                    .tree
                    .clones
                    .iter()
                    .filter(|(_, m)| m.clone_id == bad)
                    .all(|(_, m)| !m.verified));
                assert!(changed
                    .tree
                    .clones
                    .iter()
                    .filter(|(_, m)| m.clone_id != bad)
                    .all(|(_, m)| m.verified));
                assert!(changed.accounting.deduction_bytes > 0);
                assert!(changed.accounting.deduction_bytes < serial.accounting.deduction_bytes);
                assert_eq!(changed.stats.errors, 0);
                assert!(!changed.stats.incomplete);
                fn cancelled(
                    fd: &OwnedFd,
                    c: &AtomicBool,
                    stats: &mut crate::bulk::CloneReadStats,
                    visitor: &mut dyn FnMut(&crate::bulk::Record) -> io::Result<()>,
                ) -> io::Result<()> {
                    read_bulk(fd, c, stats, &mut |r| {
                        if r.name == b"a-0" {
                            c.store(true, Ordering::Relaxed);
                        }
                        visitor(r)
                    })
                }
                let cancelled = run(worker_count, cancelled);
                assert!(!cancelled.accounting.checks_complete);
                assert!(cancelled.tree.clones.iter().all(|(_, m)| !m.verified));
                assert_eq!(cancelled.accounting.deduction_bytes, 0);
                // Low budgets force one worker, with bounded deep fallback.
                let mut constrained = original.clone();
                constrained.descriptor_limit = Some(3);
                validate(
                    &constrained,
                    &fd,
                    &AtomicBool::new(false),
                    crate::ScanQos::Automatic,
                )
                .apply(&mut constrained);
                assert_eq!(constrained.accounting.revalidation_workers, 1);
                assert!(constrained.accounting.checks_complete);
                std::fs::remove_dir_all(p).unwrap();
            }
        }
        #[test]
        fn initial_probe_queue_is_sparse_and_keeps_listing_ordinals() {
            let (p, s, fd) = real_clone_fixture("sparse-probes");
            let a = crate::descriptor::entry_at(&fd, &s.root, std::ffi::OsStr::new("a")).unwrap();
            let mut checked =
                crate::descriptor::entry_at(&fd, &s.root, std::ffi::OsStr::new("b")).unwrap();
            checked.clone_checked = true;
            let bulk_metadata = s
                .iter_nodes()
                .find(|n| n.native_name() == "b")
                .unwrap()
                .clone_metadata()
                .cloned();
            let mut nonshared =
                crate::descriptor::entry_at(&fd, &s.root, std::ffi::OsStr::new("d")).unwrap();
            nonshared.clone_checked = true;
            let mut changed =
                crate::descriptor::entry_at(&fd, &s.root, std::ffi::OsStr::new("c")).unwrap();
            changed.inode ^= 1;
            let directory = crate::Entry {
                path: s.root.clone().into(),
                device: s.node(0).unwrap().identity.clone().device,
                inode: s.node(0).unwrap().identity.clone().inode,
                kind: s.node(0).unwrap().identity.clone().kind,
                length: 0,
                allocated: 0,
                hard_link: false,
                clone_checked: false,
            };
            let entries = vec![
                Err(io::Error::from_raw_os_error(libc::EIO)),
                Ok(a),
                Ok(directory),
                Ok(checked),
                Ok(nonshared),
                Ok(changed),
            ];
            let mut batch = super::super::prepare(
                &entries,
                &fd,
                &std::sync::Mutex::new(HashMap::new()),
                &AtomicBool::new(false),
                std::collections::VecDeque::from([(3, bulk_metadata.clone().unwrap())]),
                true,
            );
            assert_eq!(batch.queries, 2);
            assert_eq!(
                batch.results.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
                vec![1, 5]
            );
            assert_eq!(batch.observations.len(), 1);
            let published: Vec<_> = (0..entries.len())
                .filter_map(|i| batch.take_at(i).map(|result| (i, result)))
                .collect();
            assert_eq!(
                published.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
                vec![1, 3, 5]
            );
            assert!(published[0].1.as_ref().unwrap().is_some());
            assert!(published[1].1.as_ref().unwrap().is_some());
            assert!(published[2].1.is_err());
            assert!(batch.observations.is_empty() && batch.results.is_empty());
            // Even an unchecked entry's initial observation takes precedence.
            // Retain the initial queue's exact backing allocation, and discard
            // any unvisited suffix on cancellation without publishing it.
            for stop_at in [0, 3, entries.len()] {
                let mut observations = std::collections::VecDeque::with_capacity(17);
                observations.push_back((1, bulk_metadata.clone().unwrap()));
                observations.push_back((3, bulk_metadata.clone().unwrap()));
                let capacity = observations.capacity();
                let pointer = observations.as_slices().0.as_ptr();
                let mut visited = 0;
                let mut prepared = super::super::prepare_core(
                    &entries,
                    &fd,
                    &std::sync::Mutex::new(HashMap::new()),
                    observations,
                    true,
                    || {
                        let cancelled = visited == stop_at;
                        visited += 1;
                        cancelled
                    },
                );
                assert_eq!(prepared.cancelled, stop_at < entries.len());
                assert_eq!(prepared.observations.capacity(), capacity);
                assert_eq!(prepared.observations.as_slices().0.as_ptr(), pointer);
                assert_eq!(
                    prepared.observations.len(),
                    if stop_at == 0 {
                        0
                    } else if stop_at == 3 {
                        1
                    } else {
                        2
                    }
                );
                assert_eq!(prepared.queries, u64::from(stop_at == entries.len()));
                let published: Vec<_> = (0..entries.len())
                    .filter_map(|i| prepared.take_at(i).map(|result| (i, result)))
                    .collect();
                let ordinals: Vec<_> = published.iter().map(|(i, _)| *i).collect();
                assert_eq!(
                    ordinals,
                    if stop_at == 0 {
                        vec![]
                    } else if stop_at == 3 {
                        vec![1]
                    } else {
                        vec![1, 3, 5]
                    }
                );
                if stop_at == entries.len() {
                    assert!(published.last().unwrap().1.is_err());
                }
            }
            let cancelled = super::super::prepare(
                &entries,
                &fd,
                &std::sync::Mutex::new(HashMap::new()),
                &AtomicBool::new(true),
                std::collections::VecDeque::from([(3, bulk_metadata.unwrap())]),
                true,
            );
            assert!(cancelled.cancelled);
            assert_eq!(cancelled.queries, 0);
            assert!(cancelled.results.is_empty());
            assert!(cancelled.observations.is_empty());
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn reused_files_are_vouched_by_one_bulk_listing_and_mismatches_stay_per_file() {
            let (p, s, fd) = real_clone_fixture("vouch-reused");
            let entry = |name: &str| {
                crate::descriptor::entry_at(&fd, &s.root, std::ffi::OsStr::new(name)).unwrap()
            };
            let mut stale = entry("c");
            stale.allocated += 4096;
            let mut gone = entry("d");
            gone.path = s.root.join("gone").into();
            let mut entries = vec![Ok(entry("a")), Ok(entry("b")), Ok(stale), Ok(gone)];
            let mut observations = std::collections::VecDeque::new();
            let caps = std::sync::Mutex::new(HashMap::new());
            vouch_reused(
                &mut entries,
                &fd,
                &caps,
                &AtomicBool::new(false),
                &mut observations,
                &mut Vec::new(),
            );
            let checked = |entries: &[io::Result<crate::Entry>]| -> Vec<bool> {
                entries
                    .iter()
                    .map(|e| e.as_ref().unwrap().clone_checked)
                    .collect()
            };
            assert_eq!(checked(&entries), [true, true, false, false]);
            // The sparse observations equal what a per-file probe of the same files reports.
            assert_eq!(
                observations.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
                [0, 1]
            );
            for (i, bulk) in &observations {
                let mut queries = 0;
                let probed =
                    prepare_entry(&entry(["a", "b"][*i]), &fd, &caps, &mut queries).unwrap();
                assert!(same_observation(bulk, &probed.unwrap()));
            }
            // Only the two files the listing could not vouch for are probed one by one.
            let mut batch = super::super::prepare(
                &entries,
                &fd,
                &std::sync::Mutex::new(HashMap::new()),
                &AtomicBool::new(false),
                observations,
                true,
            );
            assert_eq!(batch.queries, 2);
            let published: Vec<_> = (0..entries.len()).map(|i| batch.take_at(i)).collect();
            assert!(published[0].as_ref().unwrap().as_ref().unwrap().is_some());
            assert!(published[1].as_ref().unwrap().as_ref().unwrap().is_some());
            assert!(published[2].as_ref().unwrap().is_err());
            assert!(published[3].as_ref().unwrap().is_err());
            // A read that fails or is cancelled vouches for nothing.
            let mut fresh = vec![Ok(entry("a")), Ok(entry("b"))];
            let mut observations = std::collections::VecDeque::new();
            vouch_reused(
                &mut fresh,
                &fd,
                &caps,
                &AtomicBool::new(true),
                &mut observations,
                &mut Vec::new(),
            );
            assert_eq!(checked(&fresh), [false, false]);
            assert!(observations.is_empty());
            // So does a device already known to lack clone fields.
            caps.lock().unwrap().insert(entry("a").device, false);
            vouch_reused(
                &mut fresh,
                &fd,
                &caps,
                &AtomicBool::new(false),
                &mut observations,
                &mut Vec::new(),
            );
            assert_eq!(checked(&fresh), [false, false]);
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn consecutive_files_reuse_one_parent_but_query_every_file() {
            let (p, mut s, fd) = real_clone_fixture("parent-reuse");
            s.accounting.revalidation_parent_walks = 0;
            s.accounting.revalidation_queries = 0;
            let v = validate(&s, &fd, &AtomicBool::new(false), crate::ScanQos::Automatic);
            v.apply(&mut s);
            assert_eq!(s.accounting.revalidation_parent_walks, 1);
            assert_eq!(s.accounting.revalidation_queries, 4);
            assert!(s.tree.clones.iter().all(|(_, metadata)| metadata.verified));
            std::fs::remove_dir_all(p).unwrap();
        }
        fn nested_clone_fixture(label: &str) -> (std::path::PathBuf, Snapshot, OwnedFd) {
            let (p, _, _) = real_clone_fixture(label);
            let inside = p.join("inside");
            std::fs::create_dir(&inside).unwrap();
            for name in ["a", "b", "c", "d"] {
                std::fs::rename(p.join(name), inside.join(name)).unwrap();
            }
            let mut s = crate::scan(
                p.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            for (_, metadata) in &mut s.tree.clones {
                metadata.verified = false;
            }
            s.accounting.revalidation_parent_walks = 0;
            s.accounting.revalidation_queries = 0;
            let fd = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            (p, s, fd)
        }
        #[test]
        fn repeated_bulk_tasks_refresh_root_and_nested_cursors() {
            for nested in [false, true] {
                let (p, s, root) = if nested {
                    nested_clone_fixture("repeat-bulk-nested")
                } else {
                    real_clone_fixture("repeat-bulk-root")
                };
                // Exhaust the shared held root too: bulk validation must never
                // consume or reset its offset, even for root-level claims.
                let cancel = AtomicBool::new(false);
                read_bulk(&root, &cancel, &mut Default::default(), &mut |_| Ok(())).unwrap();
                let ctx = MemberContext {
                    s: &s,
                    root: &root,
                    cancel: &cancel,
                    strategy: ValidationStrategy::batch(read_bulk),
                    cache_limit: 32,
                };
                let mut worker = MemberWorker::new(32);
                let mut statuses = vec![ClaimStatus::Unverified; s.tree.clones.len()];
                for task in 0..3 {
                    if task == 2 {
                        worker.begin_task();
                    }
                    statuses.fill(ClaimStatus::Unverified);
                    validate_members(
                        &ctx,
                        &s.tree.clones,
                        &mut statuses,
                        &mut |_| {},
                        &mut worker,
                    );
                    assert!(statuses
                        .iter()
                        .all(|status| *status == ClaimStatus::Verified));
                    assert!(worker.info.checks_complete, "{:?}", worker.info.warnings);
                    assert!(worker.parents.chain.len() <= 32);
                    assert_eq!(worker.parents.fallback.is_some(), !nested);
                }
                let mut remaining = 0;
                read_bulk(&root, &cancel, &mut Default::default(), &mut |_| {
                    remaining += 1;
                    Ok(())
                })
                .unwrap();
                assert_eq!(remaining, 0, "held root cursor must remain exhausted");
                std::fs::remove_dir_all(p).unwrap();
            }
        }
        #[test]
        fn bulk_prepared_parent_rejects_permission_revocation_before_read() {
            use std::os::unix::fs::PermissionsExt;

            // A superuser can bypass mode-based directory access checks.
            if unsafe { libc::geteuid() } == 0 {
                return;
            }
            let (p, mut s, root) = nested_clone_fixture("bulk-held-permission-revocation");
            let inside = p.join("inside");
            let original_permissions = std::fs::metadata(&inside).unwrap().permissions();
            let mut hooks = 0;
            let v = validate_core(
                &s,
                &root,
                &AtomicBool::new(false),
                || {},
                |_| {
                    hooks += 1;
                    // Namespace preparation has already opened the cursor.
                    // getattrlistbulk must still enforce current access rights.
                    std::fs::set_permissions(&inside, std::fs::Permissions::from_mode(0o0))
                        .unwrap();
                },
                ValidationStrategy::batch(read_bulk),
            );
            // Restore access before any assertions or fixture cleanup.
            std::fs::set_permissions(&inside, original_permissions).unwrap();
            v.apply(&mut s);
            assert_eq!(hooks, 1);
            assert!(!s.accounting.checks_complete);
            assert!(!s.accounting.warnings.is_empty());
            assert!(s.tree.clones.iter().all(|(_, m)| !m.verified));
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn bulk_prepared_parent_survives_namespace_replacement_after_hook() {
            for symlink in [false, true] {
                let (p, mut s, root) = nested_clone_fixture(if symlink {
                    "bulk-held-symlink"
                } else {
                    "bulk-held-directory"
                });
                let mut hooks = 0;
                let v = validate_core(
                    &s,
                    &root,
                    &AtomicBool::new(false),
                    || {},
                    |_| {
                        hooks += 1;
                        std::fs::rename(p.join("inside"), p.join("held-original")).unwrap();
                        if symlink {
                            std::os::unix::fs::symlink("held-original", p.join("inside")).unwrap();
                        } else {
                            std::fs::create_dir(p.join("inside")).unwrap();
                        }
                    },
                    ValidationStrategy::batch(read_bulk),
                );
                v.apply(&mut s);
                assert_eq!(hooks, 1);
                assert!(s.accounting.checks_complete, "{:?}", s.accounting.warnings);
                assert!(s.tree.clones.iter().all(|(_, m)| m.verified));
                std::fs::remove_dir_all(p).unwrap();
            }
        }
        #[test]
        fn repeated_dynamic_parent_task_rechecks_cached_ancestry() {
            let (p, s, root) = nested_clone_fixture("dynamic-parent-refresh");
            let cancel = AtomicBool::new(false);
            let ctx = MemberContext {
                s: &s,
                root: &root,
                cancel: &cancel,
                strategy: ValidationStrategy::automatic(),
                cache_limit: 32,
            };
            let mut worker = MemberWorker::new(32);
            let mut statuses = vec![ClaimStatus::Unverified; s.tree.clones.len()];
            worker.begin_task();
            validate_members(
                &ctx,
                &s.tree.clones,
                &mut statuses,
                &mut |_| {},
                &mut worker,
            );
            assert!(statuses
                .iter()
                .all(|status| *status == ClaimStatus::Verified));
            let retained = (
                worker.active.capacity(),
                worker.seen.capacity(),
                worker.fallback.capacity(),
            );
            assert!(retained.0 >= statuses.len());
            assert!(retained.1 >= statuses.len());
            std::fs::rename(p.join("inside"), p.join("held-original")).unwrap();
            std::os::unix::fs::symlink("held-original", p.join("inside")).unwrap();
            statuses.fill(ClaimStatus::Unverified);
            worker.begin_task();
            validate_members(
                &ctx,
                &s.tree.clones,
                &mut statuses,
                &mut |_| {},
                &mut worker,
            );
            assert!(statuses
                .iter()
                .all(|status| *status == ClaimStatus::Unverified));
            assert!(!worker.info.checks_complete);
            assert_eq!(
                retained,
                (
                    worker.active.capacity(),
                    worker.seen.capacity(),
                    worker.fallback.capacity()
                )
            );
            assert!(worker.seen.is_empty());
            assert!(worker.fallback.is_empty());
            assert!(
                worker.peak
                    >= retained.0 * std::mem::size_of::<usize>()
                        + retained.1 * std::mem::size_of::<bool>()
            );
            assert_eq!(worker.info.revalidation_parent_walks, 2);
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn cached_parent_is_not_redirected_by_directory_or_symlink_replacement() {
            for symlink in [false, true] {
                let (p, mut s, fd) = nested_clone_fixture(if symlink {
                    "parent-symlink"
                } else {
                    "parent-directory"
                });
                let inside = p.join("inside");
                let held = p.join("held-original");
                let outside = p.join("replacement");
                std::fs::create_dir(&outside).unwrap();
                for name in ["a", "b", "c", "d"] {
                    std::fs::write(outside.join(name), b"replacement inode").unwrap();
                }
                let mut calls = 0;
                let v = validate_with_file_hook(
                    &s,
                    &fd,
                    &AtomicBool::new(false),
                    || {},
                    |_| {
                        calls += 1;
                        if calls == 2 {
                            std::fs::rename(&inside, &held).unwrap();
                            if symlink {
                                std::os::unix::fs::symlink(&outside, &inside).unwrap();
                            } else {
                                std::fs::rename(&outside, &inside).unwrap();
                            }
                        }
                    },
                );
                // The descriptor stays on the original authorized directory,
                // rather than resolving the replaced pathname for later files.
                assert_eq!(calls, 4);
                v.apply(&mut s);
                assert_eq!(s.accounting.revalidation_parent_walks, 1);
                assert_eq!(s.accounting.revalidation_queries, 4);
                assert!(s.accounting.checks_complete);
                assert!(s.tree.clones.iter().all(|(_, metadata)| metadata.verified));
                if symlink {
                    std::fs::remove_file(&inside).unwrap();
                }
                std::fs::remove_dir_all(p).unwrap();
            }
        }
        #[test]
        fn switching_back_to_a_parent_walks_again_and_rejects_replaced_ancestry() {
            let (p, _, _) = real_clone_fixture("parent-switch");
            for directory in ["first", "second", "outside"] {
                std::fs::create_dir(p.join(directory)).unwrap();
            }
            for (file, directory) in [
                ("a", "first"),
                ("b", "first"),
                ("c", "second"),
                ("d", "second"),
            ] {
                std::fs::rename(p.join(file), p.join(directory).join(file)).unwrap();
            }
            let original = crate::scan(
                p.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let by_name = |name: &str| {
                original
                    .iter_nodes()
                    .find(|n| n.native_name() == name)
                    .unwrap()
                    .id
            };
            // Legacy topology permits children of different parents to be
            // interleaved. Parent IDs must remain the cache authority key.
            let mut s = Snapshot::empty(original.root.clone());
            let rows = [
                (0, 0, None, vec![1, 2]),
                (by_name("first"), 1, Some(0), vec![3, 5]),
                (by_name("second"), 2, Some(0), vec![4, 6]),
                (by_name("a"), 3, Some(1), vec![]),
                (by_name("c"), 4, Some(2), vec![]),
                (by_name("b"), 5, Some(1), vec![]),
                (by_name("d"), 6, Some(2), vec![]),
            ];
            for (old, id, parent_id, children) in rows {
                let mut node = original.legacy_node(old).unwrap();
                node.id = id;
                node.parent = parent_id;
                node.children = children;
                if let Some(metadata) = &mut node.clone_metadata {
                    metadata.verified = false;
                }
                s.append_legacy_node(node).unwrap();
            }
            s.accounting = original.accounting.clone();
            s.accounting.revalidation_parent_walks = 0;
            s.accounting.revalidation_queries = 0;
            s.descriptor_limit = Some(8);
            let fd = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            let v = validate_with_file_hook(
                &s,
                &fd,
                &AtomicBool::new(false),
                || {},
                |id| {
                    if id == 3 {
                        std::fs::rename(p.join("first"), p.join("held-first")).unwrap();
                        std::os::unix::fs::symlink(p.join("outside"), p.join("first")).unwrap();
                    }
                },
            );
            v.apply(&mut s);
            assert_eq!(s.accounting.revalidation_parent_walks, 4);
            assert_eq!(s.accounting.revalidation_queries, 4);
            assert!(!s.accounting.checks_complete);
            for (id, metadata) in &s.tree.clones {
                assert_eq!(metadata.verified, *id == 4 || *id == 6);
            }
            std::fs::remove_file(p.join("first")).unwrap();
            std::fs::remove_dir_all(p).unwrap();
        }
        fn sibling_clone_fixture(label: &str) -> (std::path::PathBuf, Snapshot, OwnedFd, u32, u32) {
            let (p, _, _) = nested_clone_fixture(label);
            for name in ["left", "right"] {
                std::fs::create_dir(p.join("inside").join(name)).unwrap();
            }
            for (name, side) in [("a", "left"), ("b", "left"), ("c", "right"), ("d", "right")] {
                std::fs::rename(
                    p.join("inside").join(name),
                    p.join("inside").join(side).join(name),
                )
                .unwrap();
            }
            let s = crate::scan(
                p.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let a = s.iter_nodes().find(|n| n.native_name() == "a").unwrap().id;
            let c = s.iter_nodes().find(|n| n.native_name() == "c").unwrap().id;
            let fd = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            (p, s, fd, a, c)
        }
        fn deep_branch_clone_fixture(
            label: &str,
        ) -> (std::path::PathBuf, Snapshot, OwnedFd, u32, u32) {
            let (p, _, _, _, _) = sibling_clone_fixture(label);
            for side in ["left", "right"] {
                let branch = p.join("inside").join(side);
                std::fs::create_dir(branch.join("deep")).unwrap();
                for name in if side == "left" {
                    ["a", "b"]
                } else {
                    ["c", "d"]
                } {
                    std::fs::rename(branch.join(name), branch.join("deep").join(name)).unwrap();
                }
            }
            let s = crate::scan(
                p.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let a = s.iter_nodes().find(|n| n.native_name() == "a").unwrap().id;
            let c = s.iter_nodes().find(|n| n.native_name() == "c").unwrap().id;
            let root = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            (p, s, root, a, c)
        }
        #[test]
        fn branch_cache_reuses_strict_ancestor_and_keeps_target_stream_fresh() {
            let (p, s, root, a, c) = deep_branch_clone_fixture("spare-reuse");
            for limit in [0, 1, 2, 3, 4, 6] {
                let mut cache = ParentCache::with_limit(limit);
                let mut left_ancestor = None;
                for id in [a, c, a] {
                    cache.fresh_cursor(&s, id, &root).unwrap();
                    assert!(cache.chain.len() + cache.spare.len() <= limit);
                    assert!(
                        cache.fallback.is_none()
                            || (cache.chain.is_empty() && cache.spare.is_empty())
                    );
                    if limit >= 4 && id == a {
                        let fd = cache.chain[1].fd.as_raw_fd();
                        if let Some(original) = left_ancestor {
                            assert_eq!(original, fd);
                        }
                        left_ancestor = Some(fd);
                    }
                    let mut count = 0;
                    read_bulk(
                        cache.fd(&root),
                        &AtomicBool::new(false),
                        &mut Default::default(),
                        &mut |_| {
                            count += 1;
                            Ok(())
                        },
                    )
                    .unwrap();
                    assert_eq!(count, 2);
                }
                // Repeating a target must replace its exhausted stream too.
                cache.fresh_cursor(&s, a, &root).unwrap();
                let mut count = 0;
                read_bulk(
                    cache.fd(&root),
                    &AtomicBool::new(false),
                    &mut Default::default(),
                    &mut |_| {
                        count += 1;
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(count, 2);
                if limit >= 4 {
                    cache.fresh_cursor(&s, c, &root).unwrap();
                    assert!(!cache.spare.is_empty());
                    cache.limit = 2;
                    cache.fresh_cursor(&s, a, &root).unwrap();
                    assert!(cache.chain.is_empty());
                    assert!(cache.spare.is_empty());
                    assert!(cache.fallback.is_some());
                }
            }
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn branch_cache_rejects_retired_ancestor_namespace_replacement() {
            for symlink in [false, true] {
                let (p, s, root, a, c) = deep_branch_clone_fixture(if symlink {
                    "spare-symlink"
                } else {
                    "spare-replacement"
                });
                let mut cache = ParentCache::with_limit(6);
                cache.fresh_cursor(&s, a, &root).unwrap();
                cache.fresh_cursor(&s, c, &root).unwrap();
                assert!(!cache.spare.is_empty());
                let left = p.join("inside/left");
                std::fs::rename(&left, p.join("inside/held-left")).unwrap();
                if symlink {
                    std::os::unix::fs::symlink("held-left", &left).unwrap();
                } else {
                    std::fs::create_dir(&left).unwrap();
                }
                assert!(cache.fresh_cursor(&s, a, &root).is_err());
                assert!(cache.chain.is_empty());
                assert!(cache.spare.is_empty());
                assert!(cache.fallback.is_none());
                assert!(cache.current.is_none());
                std::fs::remove_dir_all(p).unwrap();
            }
        }
        #[test]
        fn branch_cache_rejects_revoked_search_permission_on_retired_ancestor() {
            use std::os::unix::fs::PermissionsExt;
            if unsafe { libc::geteuid() } == 0 {
                return;
            }
            let (p, s, root, a, c) = deep_branch_clone_fixture("spare-search-revoked");
            let mut cache = ParentCache::with_limit(6);
            cache.fresh_cursor(&s, a, &root).unwrap();
            cache.fresh_cursor(&s, c, &root).unwrap();
            let left = p.join("inside/left");
            let permissions = std::fs::metadata(&left).unwrap().permissions();
            std::fs::set_permissions(&left, std::fs::Permissions::from_mode(0o400)).unwrap();
            let result = cache.fresh_cursor(&s, a, &root);
            std::fs::set_permissions(&left, permissions).unwrap();
            assert!(result.is_err());
            assert!(cache.chain.is_empty());
            assert!(cache.spare.is_empty());
            assert!(cache.fallback.is_none());
            assert!(cache.current.is_none());
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn parent_cache_reuses_common_ancestor_with_bounded_deep_fallback() {
            let (p, s, fd, a, c) = sibling_clone_fixture("common-cache");
            assert!(s.descriptor_limit.is_some());
            let mut cache = ParentCache::new(&s);
            cache.limit = 2;
            cache.ensure(&s, a, &fd).unwrap();
            assert_eq!(cache.chain.len(), 2);
            let shared = cache.chain[0].fd.as_raw_fd();
            cache.ensure(&s, c, &fd).unwrap();
            assert_eq!(cache.chain[0].fd.as_raw_fd(), shared);
            assert_eq!(cache.chain.len(), 2);
            for limit in [0, 1] {
                cache.limit = limit;
                cache.clear();
                cache.ensure(&s, a, &fd).unwrap();
                assert!(cache.chain.is_empty());
                assert!(cache.fallback.is_some());
                assert!(probe(&s, a, cache.fd(&fd)).is_ok());
                cache.ensure(&s, c, &fd).unwrap();
                assert!(cache.chain.is_empty());
                assert!(probe(&s, c, cache.fd(&fd)).is_ok());
            }
            let mut bounded = ParentCache::new(&s);
            assert!(bounded.limit <= 32);
            bounded.limit = 2;
            bounded.ensure(&s, a, &fd).unwrap();
            let mut wrong_root = s.clone();
            wrong_root
                .update_node(0, |n| n.identity.inode ^= 1)
                .unwrap();
            assert!(bounded
                .ensure(&wrong_root, c, &fd)
                .unwrap_err()
                .to_string()
                .contains("root identity"));
            assert!(bounded.chain.is_empty());
            assert!(bounded.current.is_none());
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn fresh_parent_cursors_switch_between_cached_ancestors_and_children() {
            let (p, s, root, a, c) = sibling_clone_fixture("fresh-cache-switch");
            let left = s.node(a).unwrap().parent.unwrap();
            for limit in [0, 1, 2] {
                let mut cache = ParentCache::with_limit(limit);
                // left's parent is inside, so this also switches from a cached
                // child target back to an already cached ancestor target.
                for (id, expected) in [(a, 2), (a, 2), (left, 2), (c, 2), (left, 2)] {
                    cache.fresh_cursor(&s, id, &root).unwrap();
                    let mut records = 0;
                    read_bulk(
                        cache.fd(&root),
                        &AtomicBool::new(false),
                        &mut Default::default(),
                        &mut |_| {
                            records += 1;
                            Ok(())
                        },
                    )
                    .unwrap();
                    assert_eq!(records, expected);
                    assert!(cache.chain.len() <= limit);
                    assert!(
                        cache.chain.len() + usize::from(cache.fallback.is_some()) <= limit.max(1)
                    );
                }
            }
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn fresh_cached_target_rejects_directory_and_symlink_replacement() {
            for symlink in [false, true] {
                let (p, s, root) = nested_clone_fixture(if symlink {
                    "fresh-target-symlink"
                } else {
                    "fresh-target-directory"
                });
                let id = s.tree.clones[0].0;
                let mut cache = ParentCache::with_limit(2);
                cache.fresh_cursor(&s, id, &root).unwrap();
                std::fs::rename(p.join("inside"), p.join("held-original")).unwrap();
                if symlink {
                    std::os::unix::fs::symlink("held-original", p.join("inside")).unwrap();
                } else {
                    std::fs::create_dir(p.join("inside")).unwrap();
                }
                assert!(cache.fresh_cursor(&s, id, &root).is_err());
                assert!(cache.chain.is_empty());
                assert!(cache.fallback.is_none());
                assert!(cache.current.is_none());
                std::fs::remove_dir_all(p).unwrap();
            }
        }
        #[test]
        fn common_ancestor_switch_rejects_directory_and_symlink_replacement() {
            for symlink in [false, true] {
                let (p, s, fd, a, c) = sibling_clone_fixture(if symlink {
                    "common-symlink"
                } else {
                    "common-directory"
                });
                let mut cache = ParentCache::new(&s);
                cache.limit = 2;
                cache.ensure(&s, a, &fd).unwrap();
                std::fs::rename(p.join("inside"), p.join("held-inside")).unwrap();
                if symlink {
                    std::os::unix::fs::symlink(p.join("held-inside"), p.join("inside")).unwrap();
                } else {
                    std::fs::create_dir(p.join("inside")).unwrap();
                }
                assert!(cache.ensure(&s, c, &fd).is_err());
                assert!(cache.chain.is_empty());
                assert!(cache.fallback.is_none());
                assert!(cache.current.is_none());
                if symlink {
                    std::fs::remove_file(p.join("inside")).unwrap();
                }
                std::fs::remove_dir_all(p).unwrap();
            }
        }
        #[test]
        fn cancelled_validation_unverifies_all_claims_without_moving_snapshot_metadata() {
            let (p, mut s, fd) = real_clone_fixture("cancel-statuses");
            for (_, m) in &mut s.tree.clones {
                m.verified = true;
            }
            s.accounting.revalidation_queries = 0;
            let prior = s.tree.clones.clone();
            let v = validate(&s, &fd, &AtomicBool::new(true), crate::ScanQos::Automatic);
            assert_eq!(s.tree.clones, prior);
            assert_eq!(v.statuses.capacity(), prior.len());
            v.apply(&mut s);
            assert!(s.tree.clones.iter().all(|(_, m)| !m.verified));
            assert!(!s.accounting.checks_complete);
            assert_eq!(s.accounting.revalidation_queries, 0);
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn link_created_after_observation_invalidates_only_its_clone_group() {
            let (p, mut s, fd) = real_clone_fixture("new-link");
            std::fs::hard_link(p.join("a"), p.join("alias")).unwrap();
            let source = s.iter_nodes().find(|n| n.native_name() == "a").unwrap().id;
            assert!(!s.node(source).unwrap().hard_link);
            assert!(probe(&s, source, &fd)
                .unwrap_err()
                .to_string()
                .contains("link count"));
            let v = validate(&s, &fd, &AtomicBool::new(false), crate::ScanQos::Automatic);
            v.apply(&mut s);
            assert!(!s.accounting.checks_complete);
            assert!(s
                .accounting
                .warnings
                .iter()
                .any(|w| w.contains("link count")));
            let broken_id = s.node(source).unwrap().clone_metadata().unwrap().clone_id;
            assert!(s
                .tree
                .clones
                .iter()
                .filter(|(_, m)| m.clone_id == broken_id)
                .all(|(_, m)| !m.verified));
            assert!(s
                .tree
                .clones
                .iter()
                .filter(|(_, m)| m.clone_id != broken_id)
                .all(|(_, m)| m.verified));
            crate::aggregate(&mut s);
            assert_eq!(s.accounting.deduction_bytes, 128 * 1024);
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn contradictory_alias_flags_drop_only_affected_native_group() {
            let (p, _, _) = real_clone_fixture("alias-flags");
            std::fs::hard_link(p.join("a"), p.join("alias")).unwrap();
            let mut s = crate::scan(
                p.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let fd = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            let ids: Vec<_> = s
                .iter_nodes()
                .filter(|n| n.native_name() == "a" || n.native_name() == "alias")
                .map(|n| n.id)
                .collect();
            let bad = s.node(ids[0]).unwrap().clone_metadata().unwrap().clone_id;
            for (_, m) in &mut s.tree.clones {
                m.verified = false;
            }
            for id in ids {
                s.update_node(id, |n| n.hard_link = false).unwrap();
            }
            let v = validate(&s, &fd, &AtomicBool::new(false), crate::ScanQos::Automatic);
            v.apply(&mut s);
            assert!(!s.accounting.checks_complete);
            assert!(!s.accounting.warnings.is_empty());
            assert!(s.tree.clones.iter().all(|(_, m)| m.clone_id != bad));
            assert_eq!(s.tree.clones.len(), 2);
            assert!(s.tree.clones.iter().all(|(_, m)| m.verified));
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn metadata_validation_allows_concurrent_snapshot_reads() {
            let p = std::env::temp_dir().join(format!("clone-read-{}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            let s = crate::scan(
                p.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let fd = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            let store = std::sync::Arc::new(std::sync::RwLock::new(s));
            let (entered, ready) = std::sync::mpsc::channel();
            let (release, resume) = std::sync::mpsc::channel();
            let shared = store.clone();
            let thread = std::thread::spawn(move || {
                let s = shared.read().unwrap();
                validate_with_hook(
                    &s,
                    &fd,
                    &AtomicBool::new(false),
                    crate::ScanQos::Automatic,
                    || {
                        entered.send(()).unwrap();
                        resume.recv().unwrap();
                    },
                );
            });
            ready
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            assert!(store.try_read().unwrap().node(0).is_some());
            release.send(()).unwrap();
            thread.join().unwrap();
            std::fs::remove_dir_all(p).unwrap();
        }
        #[test]
        fn held_parent_probe_rejects_symlink_and_file_replacement() {
            let p = std::env::temp_dir().join(format!("clone-race-{}", std::process::id()));
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("a"), b"data").unwrap();
            let s = crate::scan(
                p.clone(),
                crate::ScanOptions::default(),
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let fd = crate::descriptor::open(&s.root, &s.node(0).unwrap().identity).unwrap();
            let id = s.iter_nodes().find(|n| n.native_name() == "a").unwrap().id;
            std::fs::rename(p.join("a"), p.join("original")).unwrap();
            std::fs::write(p.join("a"), b"data").unwrap();
            assert!(probe(&s, id, &fd).is_err());
            std::fs::remove_file(p.join("a")).unwrap();
            std::os::unix::fs::symlink("original", p.join("a")).unwrap();
            assert!(probe(&s, id, &fd).is_err());
            std::fs::remove_dir_all(p).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Identity, Node, ScanState};
    pub(super) fn fixture() -> Snapshot {
        let mut s = Snapshot::empty("/fixture".into());
        s.accounting.model = AccountingModel::FullClonesV1;
        for (id, name, inode, raw) in [
            (0, "", 0, 0),
            (1, "z", 1, 120),
            (2, "a", 2, 140),
            (3, "alias", 1, 120),
        ] {
            s.append_legacy_node(Node {
                id,
                parent: (id != 0).then_some(0),
                children: if id == 0 { vec![1, 2, 3] } else { vec![] },
                name: name.into(),
                path: if id == 0 {
                    "/fixture".into()
                } else {
                    format!("/fixture/{name}").into()
                },
                identity: Identity {
                    device: 1,
                    inode,
                    kind: if id == 0 {
                        NodeKind::Directory
                    } else {
                        NodeKind::File
                    },
                },
                allocated_size: raw,
                length: if id == 0 { 0 } else { 100 },
                own_allocated_size: raw,
                is_directory: id == 0,
                hard_link: id == 1 || id == 3,
                excluded: false,
                clone_metadata: if id == 0 {
                    None
                } else {
                    Some(CloneMetadata {
                        clone_id: 42,
                        data_allocated_size: 100,
                        data_length: 100,
                        reference_count: 2,
                        may_share_blocks: true,
                        verified: true,
                    })
                },
            })
            .unwrap();
        }
        s
    }
    #[test]
    fn stable_clone_owner_retains_resource_forks_and_hardlinks() {
        let mut s = fixture();
        assert_eq!(attributed_own_allocations(&s).unwrap(), vec![0, 0, 140, 20]);
        crate::aggregate(&mut s);
        assert_eq!(s.stats.allocated_bytes, 160);
        assert_eq!(s.accounting.deduction_bytes, 100);
        crate::aggregate(&mut s);
        assert_eq!(s.stats.allocated_bytes, 160);
        assert_eq!(s.node(1).unwrap().clone_attribution(), Some((false, 0)));
    }
    #[test]
    fn unverified_or_inconsistent_group_retains_raw() {
        let mut s = fixture();
        s.tree.clones[1].1.verified = false;
        assert_eq!(
            attributed_own_allocations(&s).unwrap(),
            vec![0, 0, 140, 120]
        );
        s.tree.clones[1].1.verified = true;
        s.tree.clones[1].1.data_allocated_size = 90;
        assert_eq!(
            attributed_own_allocations(&s).unwrap(),
            vec![0, 0, 140, 120]
        );
    }
    #[test]
    fn devices_and_singletons_are_independent() {
        let mut s = fixture();
        s.update_node(2, |n| n.identity.device = 2).unwrap();
        assert_eq!(
            attributed_own_allocations(&s).unwrap(),
            vec![0, 0, 140, 120]
        );
    }
    #[test]
    fn hostile_metadata_is_rejected() {
        let mut s = fixture();
        s.tree.clones[0].1.data_allocated_size = 121;
        assert!(attributed_own_allocations(&s).is_err());
        let mut s = fixture();
        s.accounting.model = AccountingModel::HardLinksV1;
        assert!(attributed_own_allocations(&s).is_err());
        let mut s = fixture();
        s.tree.clones[0].1.clone_id = 0;
        assert!(attributed_own_allocations(&s).is_err());
    }
    #[test]
    fn contradictory_cloned_inode_aliases_reject_only_modern_model() {
        for flags in [(false, false), (true, false), (false, true)] {
            let mut s = fixture();
            s.update_node(1, |n| n.hard_link = flags.0).unwrap();
            s.update_node(3, |n| n.hard_link = flags.1).unwrap();
            assert!(attributed_own_allocations(&s)
                .unwrap_err()
                .contains("alias"));
            s.accounting.model = AccountingModel::HardLinksV1;
            s.tree.clones.clear();
            assert!(attributed_own_allocations(&s).is_ok());
        }
        let mut s = fixture();
        s.tree.clones.retain(|(id, _)| *id != 3);
        assert!(attributed_own_allocations(&s).is_err());
    }
    #[test]
    fn sparse_clones_keep_one_owner_without_negative_allocation() {
        let mut s = fixture();
        for (_, m) in &mut s.tree.clones {
            m.data_allocated_size = 0;
        }
        crate::aggregate(&mut s);
        assert_eq!(s.accounting.deduction_bytes, 0);
        assert_eq!(s.node(2).unwrap().clone_attribution(), Some((true, 0)));
        assert_eq!(s.node(3).unwrap().clone_attribution(), Some((false, 0)));
    }
    #[test]
    fn accounting_failures_do_not_change_filesystem_coverage() {
        let mut s = fixture();
        s.accounting.checks_complete = true;
        commit(
            &mut s,
            1,
            Err(std::io::Error::from_raw_os_error(libc::EACCES)),
        );
        assert!(!s.accounting.checks_complete);
        assert_eq!(s.accounting.warnings.len(), 1);
        assert_eq!(s.stats.errors, 0);
        assert!(s.errors.is_empty());
    }
    #[test]
    fn eligibility_requires_explicit_nonshared_checks() {
        let mut s = fixture();
        s.state = ScanState::Completed;
        assert!(!s.incremental_accounting_eligible());
        s.accounting.checks_complete = true;
        s.accounting.shared = true;
        assert!(!s.incremental_accounting_eligible());
        s.accounting.shared = false;
        assert!(!s.incremental_accounting_eligible());
        s.tree.clones.clear();
        assert!(s.incremental_accounting_eligible());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn real_apfs_full_clones_and_broken_clone() {
        use std::os::unix::ffi::OsStrExt;
        let p = std::env::temp_dir().join(format!("disk-clones-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        let a = p.join("a");
        let b = p.join("b");
        std::fs::write(&a, vec![7u8; 1024 * 128]).unwrap();
        let ac = std::ffi::CString::new(a.as_os_str().as_bytes()).unwrap();
        let bc = std::ffi::CString::new(b.as_os_str().as_bytes()).unwrap();
        unsafe extern "C" {
            fn clonefile(a: *const libc::c_char, b: *const libc::c_char, flags: u32) -> i32;
        }
        if unsafe { clonefile(ac.as_ptr(), bc.as_ptr(), 0) } != 0 {
            eprintln!(
                "APFS fixture unsupported: {}",
                std::io::Error::last_os_error()
            );
            std::fs::remove_dir_all(p).unwrap();
            return;
        }
        // Distinct resource forks must remain attributed to each cloned inode.
        std::fs::write(a.join("..namedfork/rsrc"), vec![1u8; 8192]).unwrap();
        std::fs::write(b.join("..namedfork/rsrc"), vec![2u8; 16384]).unwrap();
        std::fs::hard_link(&a, p.join("alias")).unwrap();
        for backend in [crate::Backend::Bulk, crate::Backend::Portable] {
            let run = || {
                crate::scan(
                    p.clone(),
                    crate::ScanOptions {
                        backend,
                        ..Default::default()
                    },
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    |_| {},
                )
                .unwrap()
            };
            let s = run();
            assert!(s.accounting.checks_complete, "{:?}", s.accounting.warnings);
            if s.tree.clones.is_empty() {
                eprintln!("APFS full clone mapping unsupported by this volume ({backend:?}); raw/hardlink accounting retained");
                continue;
            }
            assert!(s.accounting.shared);
            assert!(s.accounting.deduction_bytes > 0);
            let raw = hardlink_allocations(&s).unwrap().into_iter().sum::<u64>();
            assert_eq!(s.stats.allocated_bytes, raw - s.accounting.deduction_bytes);
            for (id, m) in &s.tree.clones {
                assert!(m.data_allocated_size < s.node(*id).unwrap().own_allocated_size);
            }
        }
        std::fs::write(&b, vec![8u8; 1024 * 128]).unwrap();
        let s = crate::scan(
            p.clone(),
            crate::ScanOptions::default(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        assert_eq!(s.accounting.deduction_bytes, 0);
        std::fs::remove_dir_all(p).unwrap();
    }
}
