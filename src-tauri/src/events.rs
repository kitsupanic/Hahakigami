//! Conservative, bounded historical FSEvents replay. No long-lived watcher.
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

/// Volume events are resolved against the exact cached tree after decoding.
/// External hard-link aliases can mutate cached files, so their inode IDs matter
/// even when their paths lie outside the selected folder.
#[derive(Default, Debug)]
pub struct Changes {
    pub paths: Vec<PathBuf>,
    pub directories: Vec<PathBuf>,
    pub shared_inodes: HashSet<u64>,
    /// An external change cannot be resolved by inode (including directory removal).
    pub uncertain_aliases: bool,
}
impl Changes {
    pub fn resolve(mut self, snapshot: &disk_scan::Snapshot) -> Result<Self, String> {
        let device = snapshot.node(0).map(|n| n.identity.device);
        for node in snapshot.iter_nodes() {
            if (self.uncertain_aliases && node.hard_link)
                || (Some(node.identity.device) == device
                    && self.shared_inodes.contains(&node.identity.inode))
            {
                self.paths.push(node.path()?);
            }
        }
        dedup_paths(&mut self.paths);
        dedup_paths(&mut self.directories);
        Ok(self)
    }
}

// Consumers use these as sets. Keep the first representative, just as the
// previous stable sort/dedup did, using Path's component-aware Eq and Hash.
// Borrow during membership checks so long paths do not need another clone.
fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::with_capacity(paths.len());
    let keep: Vec<bool> = paths
        .iter()
        .map(|path| seen.insert(path.as_path()))
        .collect();
    drop(seen);
    let mut keep = keep.into_iter();
    paths.retain(|_| keep.next().unwrap());
}

#[cfg(all(test, unix))]
mod resolution_tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    // Independent oracle retains the previous resolver, including stable sort.
    fn old_resolve(mut changes: Changes, nodes: &[disk_scan::Node]) -> Changes {
        let device = nodes.first().map(|n| n.identity.device);
        for node in nodes {
            if (changes.uncertain_aliases && node.hard_link)
                || (Some(node.identity.device) == device
                    && changes.shared_inodes.contains(&node.identity.inode))
            {
                changes.paths.push(node.path.clone());
            }
        }
        changes.paths.sort();
        changes.paths.dedup();
        changes.directories.sort();
        changes.directories.dedup();
        changes
    }

    fn copy_changes(changes: &Changes) -> Changes {
        Changes {
            paths: changes.paths.clone(),
            directories: changes.directories.clone(),
            shared_inodes: changes.shared_inodes.clone(),
            uncertain_aliases: changes.uncertain_aliases,
        }
    }

    fn assert_equivalent(expected: &Changes, actual: &Changes) {
        for (old, new) in [
            (&expected.paths, &actual.paths),
            (&expected.directories, &actual.directories),
        ] {
            let mut new = new.clone();
            new.sort();
            assert_eq!(old, &new);
            for (old, new) in old.iter().zip(&new) {
                assert_eq!(old.as_os_str().as_bytes(), new.as_os_str().as_bytes());
            }
        }
        assert_eq!(expected.shared_inodes, actual.shared_inodes);
        assert_eq!(expected.uncertain_aliases, actual.uncertain_aliases);
    }

    #[test]
    fn alias_resolution_preserves_path_sets_and_first_representative_bytes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file"), b"data").unwrap();
        let mut snapshot = disk_scan::scan(
            dir.path().to_path_buf(),
            disk_scan::ScanOptions::default(),
            std::sync::Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        let raw = PathBuf::from(std::ffi::OsStr::from_bytes(b"/scope/raw\xff"));
        let paths = [
            PathBuf::from("/scope/a/.//b"),
            PathBuf::from("/scope/a/b"),
            raw.clone(),
            PathBuf::from("/scope/other/../b"),
            PathBuf::from("/scope/b"),
        ];
        let template = snapshot.legacy_node(1).unwrap();
        let root = snapshot.legacy_node(0).unwrap();
        let root_device = root.identity.device;
        let mut oracle = vec![root.clone()];
        snapshot = disk_scan::Snapshot::empty(snapshot.root.clone());
        snapshot.append_legacy_node(root).unwrap();
        for (index, path) in paths.iter().cycle().take(10).enumerate() {
            let mut node = template.clone();
            node.id = (index + 1) as u32;
            node.path = path.clone();
            node.identity.inode = 42;
            node.identity.device = root_device + u64::from(index >= 5);
            node.hard_link = index % 2 == 0;
            // Kind and flags agree; alias resolution itself is independent of
            // directory type. Non-files cannot carry the hard-link flag.
            node.is_directory = index % 3 == 0 && !node.hard_link;
            node.identity.kind = if node.is_directory {
                disk_scan::NodeKind::Directory
            } else {
                disk_scan::NodeKind::File
            };
            oracle.push(node.clone());
            snapshot.append_legacy_node(node).unwrap();
        }
        for rootless in [false, true] {
            let mut snapshot = snapshot.clone();
            if rootless {
                snapshot = disk_scan::Snapshot::empty(snapshot.root.clone());
            }
            for uncertain_aliases in [false, true] {
                for shared_inodes in [HashSet::new(), HashSet::from([42, u64::MAX])] {
                    let changes = Changes {
                        paths: vec![paths[0].clone(), raw.clone(), paths[1].clone(), raw.clone()],
                        // Keep categories separate, with a different first representative.
                        directories: vec![paths[1].clone(), paths[0].clone(), raw.clone()],
                        shared_inodes,
                        uncertain_aliases,
                    };
                    let expected =
                        old_resolve(copy_changes(&changes), if rootless { &[] } else { &oracle });
                    let actual = changes.resolve(&snapshot).unwrap();
                    assert_equivalent(&expected, &actual);
                }
            }
        }
    }

    #[test]
    fn invalid_alias_reconstruction_is_an_error_not_a_partial_resolution() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("file"), b"data").unwrap();
        let mut snapshot = disk_scan::scan(
            directory.path().into(),
            disk_scan::ScanOptions::default(),
            std::sync::Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        let inode = snapshot.node(1).unwrap().identity.inode;
        snapshot
            .update_node(1, |node| {
                node.parent = Some(1);
                node.hard_link = true;
            })
            .unwrap();
        for changes in [
            Changes {
                shared_inodes: HashSet::from([inode]),
                ..Default::default()
            },
            Changes {
                uncertain_aliases: true,
                ..Default::default()
            },
        ] {
            assert!(changes.resolve(&snapshot).is_err());
        }
    }

    #[test]
    #[ignore = "opt-in existing private cache resolver benchmark"]
    fn benchmark_cached_alias_resolution() {
        use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, time::Instant};
        let cache_root = PathBuf::from(std::env::var_os("HAH_ALIAS_CACHE_ROOT").unwrap());
        let cache_id = std::env::var("HAH_ALIAS_CACHE_ID").unwrap();
        let report = PathBuf::from(std::env::var_os("HAH_ALIAS_REPORT").unwrap());
        assert!(cache_root.is_absolute() && report.is_absolute());
        assert!(report.starts_with("/private/tmp"));
        let (snapshot, _) = crate::cache_store::Store::new(cache_root)
            .load(&cache_id)
            .unwrap();
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(report)
            .unwrap();
        let oracle: Vec<_> = snapshot
            .iter_nodes()
            .map(|n| snapshot.legacy_node(n.id).unwrap())
            .collect();
        let mut rows = Vec::new();
        for uncertain_aliases in [false, true] {
            let changes = Changes {
                shared_inodes: snapshot
                    .iter_nodes()
                    .take(100_000)
                    .map(|n| n.identity.inode)
                    .collect(),
                uncertain_aliases,
                ..Default::default()
            };
            for round in 0..6 {
                let mut results = Vec::new();
                for old in if round % 2 == 0 {
                    [true, false]
                } else {
                    [false, true]
                } {
                    let input = copy_changes(&changes);
                    let began = Instant::now();
                    let resolved = if old {
                        old_resolve(input, &oracle)
                    } else {
                        input.resolve(&snapshot).unwrap()
                    };
                    let elapsed_ms = began.elapsed().as_secs_f64() * 1000.0;
                    rows.push(serde_json::json!({
                        "uncertain_aliases": uncertain_aliases, "round": round,
                        "initial": round == 0, "old": old, "elapsed_ms": elapsed_ms,
                        "resolved_paths": resolved.paths.len()
                    }));
                    results.push((old, resolved));
                }
                let expected = &results.iter().find(|(old, _)| *old).unwrap().1;
                let actual = &results.iter().find(|(old, _)| !*old).unwrap().1;
                assert_equivalent(expected, actual);
            }
        }
        serde_json::to_writer_pretty(
            &mut output,
            &serde_json::json!({
                "complete": true, "exact_path_sets_and_representatives": true,
                "entries": snapshot.node_count(), "rows": rows
            }),
        )
        .unwrap();
        output.write_all(b"\n").unwrap();
        output.sync_all().unwrap();
    }
}

#[cfg(not(target_os = "macos"))]
pub fn current_id() -> Option<u64> {
    None
}
#[cfg(not(target_os = "macos"))]
pub fn journal_identity(_: &Path) -> Option<String> {
    None
}
#[cfg(not(target_os = "macos"))]
pub fn boot_session() -> Option<String> {
    None
}
#[cfg(not(target_os = "macos"))]
pub fn replay_with_abortable_load_on_volume<T: Send>(
    _: &Path,
    _: &Path,
    _: u64,
    _: &AtomicBool,
    _: impl FnOnce(&AtomicBool) -> Result<T, String> + Send,
) -> Result<(T, Changes), String> {
    Err("Filesystem event history is unsupported on this platform".into())
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::{
        ffi::{c_char, c_void, CStr},
        ptr,
        sync::{atomic::Ordering, Condvar, Mutex},
        time::{Duration, Instant},
    };
    type Ref = *const c_void;
    const HISTORY_DONE: u32 = 0x10;
    const UNSAFE: u32 = 0xef; // dropped/coalesced history, wrapped IDs, root/mount changes
    const KNOWN: u32 = 0x7f_ffff;
    const MAX_PATHS: usize = 10_000;
    // A detailed directory creation/metadata event changes its own listing and
    // metadata, not every descendant. Rename/removal/unknown events stay broad.
    fn listing_only(flags: u32) -> bool {
        flags & 0x20000 != 0
            && flags & !(0x20000 | 0x100 | 0x400 | 0x2000 | 0x4000 | 0x8000 | 0x80000) == 0
    }
    const HARD_LINK: u32 = 0x30_0000; // hard link and last hard link
    const MAX_EVENTS: usize = 10_000;
    #[repr(C)]
    struct Context {
        version: isize,
        info: *mut c_void,
        retain: Ref,
        release: Ref,
        description: Ref,
    }
    #[link(name = "CoreServices", kind = "framework")]
    extern "C" {
        fn FSEventsGetCurrentEventId() -> u64;
        fn FSEventsCopyUUIDForDevice(device: libc::dev_t) -> Ref;
        fn FSEventStreamCreate(
            a: Ref,
            callback: unsafe extern "C" fn(
                Ref,
                *mut c_void,
                usize,
                *mut c_void,
                *const u32,
                *const u64,
            ),
            context: *mut Context,
            paths: Ref,
            since: u64,
            latency: f64,
            flags: u32,
        ) -> Ref;
        fn FSEventStreamSetDispatchQueue(stream: Ref, queue: Ref);
        fn FSEventStreamStart(stream: Ref) -> u8;
        fn FSEventStreamFlushAsync(stream: Ref) -> u64;
        fn FSEventStreamStop(stream: Ref);
        fn FSEventStreamInvalidate(stream: Ref);
        fn FSEventStreamRelease(stream: Ref);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithBytes(
            a: Ref,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> Ref;
        fn CFArrayCreate(a: Ref, values: *const Ref, count: isize, callbacks: Ref) -> Ref;
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
        fn CFGetTypeID(value: Ref) -> usize;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFNumberGetTypeID() -> usize;
        fn CFStringGetLength(string: Ref) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFNumberGetValue(number: Ref, kind: isize, value: *mut c_void) -> u8;
        fn CFRelease(value: Ref);
        fn CFUUIDCreateString(a: Ref, uuid: Ref) -> Ref;
        fn CFStringGetCString(string: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    }
    extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: Ref) -> Ref;
        fn dispatch_sync_f(
            queue: Ref,
            context: *mut c_void,
            work: unsafe extern "C" fn(*mut c_void),
        );
        fn dispatch_release(queue: Ref);
    }
    #[derive(Default)]
    struct Collected {
        changes: Changes,
        examined: usize,
        max_id: u64,
        done: bool,
        error: Option<String>,
    }
    fn replay_drained(collected: &Collected, marker: Option<u64>) -> bool {
        collected.done && marker.is_some_and(|id| id == 0 || collected.max_id >= id)
    }
    struct State {
        root: PathBuf,
        since: u64,
        collected: Mutex<Collected>,
        ready: Condvar,
        /// Set once the replay has failed, for the cache loader to stop early.
        abort: AtomicBool,
    }
    fn wait_for_replay(
        state: &State,
        cancel: &AtomicBool,
        timeout: Duration,
        mut flush: impl FnMut() -> u64,
    ) -> Result<(), String> {
        let mut deadline = Instant::now() + timeout;
        let mut flush_marker = None;
        loop {
            if cancel.load(Ordering::Relaxed) {
                break Err("Filesystem event replay cancelled".into());
            }
            let collected = match state.collected.lock() {
                Ok(value) => value,
                Err(_) => break Err("Filesystem event callback failed".into()),
            };
            if let Some(error) = &collected.error {
                break Err(error.clone());
            }
            if replay_drained(&collected, flush_marker) {
                break Ok(());
            }
            if collected.done && flush_marker.is_none() {
                drop(collected);
                // FlushAsync returns the largest event already queued for
                // this stream. Drain that marker as well as HistoryDone;
                // never discard newly delivered paths by a prior ID cutoff.
                flush_marker = Some(flush());
                // The flush joins the cache loader, which can outlast the whole timeout; the marker gets its own.
                deadline = Instant::now() + timeout;
                continue;
            }
            if Instant::now() >= deadline {
                break Err("Filesystem event history did not complete before timeout".into());
            }
            // The callback signals under the same mutex, so callbacks
            // between testing readiness and waiting cannot be lost.
            // Bound waits to observe cancellation without a callback.
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25));
            if state.ready.wait_timeout(collected, wait).is_err() {
                break Err("Filesystem event callback failed".into());
            }
        }
    }
    fn namespace(path: &Path) -> PathBuf {
        let path = path
            .strip_prefix("/System/Volumes/Data")
            .ok()
            .map(|p| Path::new("/").join(p))
            .unwrap_or_else(|| path.to_path_buf());
        if ["/var", "/tmp", "/etc"].iter().any(|p| path.starts_with(p)) {
            Path::new("/private").join(path.strip_prefix("/").unwrap())
        } else {
            path
        }
    }
    fn mapped(root: &Path, path: &Path) -> Option<PathBuf> {
        let base = namespace(root);
        let normalized = namespace(path);
        normalized
            .strip_prefix(base)
            .ok()
            .map(|relative| root.join(relative))
    }
    fn old_unrelated_mount(root: &Path, path: &Path, flags: u32, id: u64, since: u64) -> bool {
        if !root.is_absolute()
            || !path.is_absolute()
            || !root.as_os_str().as_encoded_bytes().is_ascii()
            || !path.as_os_str().as_encoded_bytes().is_ascii()
        {
            return false;
        }
        let root = PathBuf::from(namespace(root).to_string_lossy().to_lowercase());
        let path = PathBuf::from(namespace(path).to_string_lossy().to_lowercase());
        id != 0
            && id <= since
            && flags != 0
            && flags & !0xc0 == 0
            && !path.starts_with(&root)
            && !root.starts_with(path)
    }
    fn classify(root: &Path, path: &Path, flags: u32) -> Result<bool, String> {
        if flags & UNSAFE != 0 || flags & !KNOWN != 0 {
            return Err(format!(
                "Filesystem event history requires a full scan (flags {flags:#x})"
            ));
        }
        if flags & HISTORY_DONE != 0 {
            return Ok(false);
        }
        let base = namespace(root);
        let normalized = namespace(path);
        if path.is_absolute() && base.starts_with(&normalized) && normalized != base {
            if listing_only(flags) {
                return Ok(false);
            }
            return Err(format!(
                "A scan ancestor changed; a full scan is required ({}, flags {flags:#x})",
                path.display()
            ));
        }
        if !path.is_absolute() {
            return Err("Filesystem event path is not absolute".into());
        }
        if !normalized.starts_with(&base) {
            // A differently cased spelling can name the same object on macOS.
            // Treat possible aliases as uncertain rather than silently dropping them.
            let lower_base = base.to_string_lossy().to_lowercase();
            let lower_path = normalized.to_string_lossy().to_lowercase();
            if Path::new(&lower_path).starts_with(&lower_base)
                || Path::new(&lower_base).starts_with(&lower_path)
            {
                return Err(
                    "Filesystem event namespace is ambiguous; a full scan is required".into(),
                );
            }
        }
        Ok(normalized.starts_with(base))
    }
    fn collect_event(
        state: &State,
        collected: &mut Collected,
        path: &Path,
        flag: u32,
        id: u64,
        inode: Option<u64>,
    ) {
        if old_unrelated_mount(&state.root, path, flag, id, state.since) {
            return;
        }
        // Validate history and namespace before ignoring an out-of-scope path.
        let in_scope = match classify(&state.root, path, flag) {
            Ok(in_scope) => in_scope,
            Err(error) => {
                collected.error = Some(error);
                return;
            }
        };
        if flag & HARD_LINK != 0 {
            let Some(inode) = inode.filter(|inode| *inode != 0) else {
                collected.error =
                    Some("Hard-link event lacks a file identity; a full scan is required".into());
                return;
            };
            collected.changes.shared_inodes.insert(inode);
        }
        // Do not rely on ItemIsHardlink being attached to every content/metadata
        // event. Extended records identify all file events, even external aliases.
        // Missing identities and external subtree namespace changes cannot prove
        // that a cached multiply-linked inode stayed unchanged.
        if flag & HISTORY_DONE == 0 {
            if flag & 0x20000 == 0 {
                if let Some(inode) = inode.filter(|inode| *inode != 0) {
                    collected.changes.shared_inodes.insert(inode);
                } else if !in_scope {
                    collected.changes.uncertain_aliases = true;
                }
            } else if !in_scope && !listing_only(flag) {
                collected.changes.uncertain_aliases = true;
            }
        }
        if in_scope {
            if collected.changes.paths.len() + collected.changes.directories.len() >= MAX_PATHS {
                collected.error = Some("Filesystem event history exceeds the path limit".into());
            } else if let Some(path) = mapped(&state.root, path) {
                if listing_only(flag) {
                    collected.changes.directories.push(path);
                } else {
                    collected.changes.paths.push(path);
                }
            } else {
                collected.error = Some("Filesystem event namespace cannot be mapped".into());
            }
        }
    }
    /// The event path's bytes as the filesystem stores them. Not `CFStringGetFileSystemRepresentation`: it
    /// decomposes (NFD), so `café` stored composed would never match the scanned node and its change would be lost.
    unsafe fn string_path(path: Ref) -> Result<PathBuf, String> {
        let capacity = CFStringGetMaximumSizeForEncoding(CFStringGetLength(path), 0x08000100);
        if !(0..=1024 * 1024).contains(&capacity) {
            return Err("Unrepresentable filesystem event path".into());
        }
        let mut bytes = vec![0i8; capacity as usize + 1];
        if CFStringGetCString(path, bytes.as_mut_ptr(), bytes.len() as isize, 0x08000100) == 0 {
            return Err("Unrepresentable filesystem event path".into());
        }
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(std::ffi::OsStr::from_bytes(
            CStr::from_ptr(bytes.as_ptr()).to_bytes(),
        )))
    }
    unsafe fn event_record(record: Ref) -> Result<(PathBuf, Option<u64>), String> {
        if record.is_null() || CFGetTypeID(record) != CFDictionaryGetTypeID() {
            return Err("Malformed filesystem event record".into());
        }
        let key = |bytes: &[u8]| {
            CFStringCreateWithBytes(
                ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                0x08000100,
                0,
            )
        };
        let path_key = key(b"path");
        let inode_key = key(b"fileID");
        if path_key.is_null() || inode_key.is_null() {
            if !path_key.is_null() {
                CFRelease(path_key);
            }
            if !inode_key.is_null() {
                CFRelease(inode_key);
            }
            return Err("Cannot decode filesystem event keys".into());
        }
        let path = CFDictionaryGetValue(record, path_key);
        let inode = CFDictionaryGetValue(record, inode_key);
        CFRelease(path_key);
        CFRelease(inode_key);
        if path.is_null() || CFGetTypeID(path) != CFStringGetTypeID() {
            return Err("Missing filesystem event path".into());
        }
        let path = string_path(path)?;
        let mut value = 0i64;
        let inode = (!inode.is_null()
            && CFGetTypeID(inode) == CFNumberGetTypeID()
            && CFNumberGetValue(inode, 4, (&mut value as *mut i64).cast()) != 0
            && value > 0)
            .then_some(value as u64);
        Ok((path, inode))
    }
    unsafe extern "C" fn callback(
        _: Ref,
        info: *mut c_void,
        count: usize,
        paths: *mut c_void,
        flags: *const u32,
        ids: *const u64,
    ) {
        // Stop/invalidate and drain the serial queue before State is freed.
        let state = &*(info as *const State);
        let Ok(mut collected) = state.collected.lock() else {
            return;
        };
        if collected.error.is_some() {
            return;
        }
        let array = paths.cast_const();
        if array.is_null() || CFArrayGetCount(array) != count as isize {
            collected.error = Some("Malformed filesystem event array".into());
            state.abort.store(true, Ordering::Relaxed);
            state.ready.notify_one();
            return;
        }
        for i in 0..count {
            collected.examined += 1;
            if collected.examined > MAX_EVENTS {
                collected.error = Some("Filesystem event history exceeds the event limit".into());
                break;
            }
            let id = *ids.add(i);
            collected.max_id = collected.max_id.max(id);
            let flag = *flags.add(i);
            if flag & (UNSAFE & !0xc0) != 0 || flag & !KNOWN != 0 {
                collected.error = Some(format!(
                    "Incomplete filesystem event history (flags {flag:#x})"
                ));
                break;
            }
            if flag & HISTORY_DONE != 0 {
                collected.done = true;
                continue;
            }
            match event_record(CFArrayGetValueAtIndex(array, i as isize)) {
                Ok((path, inode)) => collect_event(state, &mut collected, &path, flag, id, inode),
                Err(error) => collected.error = Some(error),
            }
            if collected.error.is_some() {
                break;
            }
        }
        // The loader may be what `wait_for_replay` is waiting on: tell it to stop now.
        if collected.error.is_some() {
            state.abort.store(true, Ordering::Relaxed);
        }
        state.ready.notify_one();
    }
    unsafe extern "C" fn barrier(_: *mut c_void) {}
    pub fn current_id() -> Option<u64> {
        let id = unsafe { FSEventsGetCurrentEventId() };
        (id != 0).then_some(id)
    }
    pub fn journal_identity(root: &Path) -> Option<String> {
        use std::os::unix::fs::MetadataExt;
        let device = std::fs::metadata(root).ok()?.dev() as libc::dev_t;
        unsafe {
            let uuid = FSEventsCopyUUIDForDevice(device);
            if uuid.is_null() {
                return None;
            }
            let string = CFUUIDCreateString(ptr::null(), uuid);
            CFRelease(uuid);
            if string.is_null() {
                return None;
            }
            let mut bytes = [0i8; 64];
            let success =
                CFStringGetCString(string, bytes.as_mut_ptr(), bytes.len() as isize, 0x08000100);
            CFRelease(string);
            if success == 0 {
                return None;
            }
            CStr::from_ptr(bytes.as_ptr())
                .to_str()
                .ok()
                .map(str::to_owned)
        }
    }
    pub fn boot_session() -> Option<String> {
        let mut bytes = [0u8; 128];
        let mut len = bytes.len();
        let result = unsafe {
            libc::sysctlbyname(
                c"kern.bootsessionuuid".as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut len,
                ptr::null_mut(),
                0,
            )
        };
        if result != 0 || len == 0 || len > bytes.len() {
            return None;
        }
        let value = std::str::from_utf8(&bytes[..len])
            .ok()?
            .trim_end_matches('\0');
        (!value.is_empty()).then(|| value.to_owned())
    }
    fn volume_mount(root: &Path) -> Result<PathBuf, String> {
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(root.as_os_str().as_bytes())
            .map_err(|_| "Invalid filesystem event root")?;
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        let mounted = (unsafe { libc::statfs(name.as_ptr(), &mut stat) } == 0)
            .then(|| crate::platform::mount_of(&stat));
        let Some((mount, true)) = mounted else {
            return Err("Filesystem event volume is unavailable or not local".into());
        };
        if !mount.is_absolute() {
            return Err("Filesystem event volume mount is not absolute".into());
        }
        Ok(mount)
    }
    fn validate_volume(root: &Path, volume: &Path) -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;
        // The caller obtains the mount from its held root descriptor. Recheck
        // the pathname device and public mount metadata before stream creation.
        if volume_mount(root)? != volume || volume_mount(volume)? != volume {
            return Err("Filesystem event volume mapping is uncertain".into());
        }
        let root_device = std::fs::metadata(root)
            .map_err(|_| "Filesystem event root unavailable")?
            .dev();
        let volume_device = std::fs::metadata(volume)
            .map_err(|_| "Filesystem event volume unavailable")?
            .dev();
        if root_device != volume_device {
            return Err("Filesystem event root and volume devices differ".into());
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn replay(root: &Path, since: u64, cancel: &AtomicBool) -> Result<Changes, String> {
        let volume = volume_mount(root)?;
        replay_on_volume(root, &volume, since, cancel)
    }
    #[cfg(test)]
    pub fn replay_on_volume(
        root: &Path,
        volume: &Path,
        since: u64,
        cancel: &AtomicBool,
    ) -> Result<Changes, String> {
        replay_with_load_on_volume(root, volume, since, cancel, || Ok(()))
            .map(|(_, changes)| changes)
    }
    #[cfg(test)]
    pub fn replay_with_load_on_volume<T: Send>(
        root: &Path,
        volume: &Path,
        since: u64,
        cancel: &AtomicBool,
        load: impl FnOnce() -> Result<T, String> + Send,
    ) -> Result<(T, Changes), String> {
        replay_with_abortable_load_on_volume(root, volume, since, cancel, |_| load())
    }
    /// Decode while one journal stream starts, then flush after decoding finishes.
    /// Joining a loader is not interruptible, matching synchronous cache decoding, so
    /// `load` gets a flag that is set once the replay has failed: its result is thrown
    /// away, and it should stop instead of making the failure wait for it.
    pub fn replay_with_abortable_load_on_volume<T: Send>(
        root: &Path,
        volume: &Path,
        since: u64,
        cancel: &AtomicBool,
        load: impl FnOnce(&AtomicBool) -> Result<T, String> + Send,
    ) -> Result<(T, Changes), String> {
        validate_volume(root, volume)?;
        let volume_text = volume
            .to_str()
            .ok_or("Unrepresentable filesystem event volume")?;
        let journal =
            journal_identity(root).ok_or("Filesystem event history unavailable for this volume")?;
        let until = current_id().ok_or("Filesystem event history unavailable")?;
        if since == 0 || since > until {
            return Err("Invalid filesystem event history cursor".into());
        }
        let _root_text = root
            .to_str()
            .filter(|_| root.is_absolute())
            .ok_or("Unrepresentable scan root")?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Filesystem event replay cancelled".into());
        }
        let state = Box::new(State {
            root: root.into(),
            since,
            collected: Mutex::new(Collected::default()),
            ready: Condvar::new(),
            abort: AtomicBool::new(false),
        });
        let loaded = std::thread::scope(|scope| {
            let mut worker = Some(scope.spawn(|| load(&state.abort)));
            let mut loaded = None;
            // Every early stream failure still joins the loader explicitly, so
            // loader panics become errors rather than escaping the scoped join.
            let replay_result = (|| -> Result<(), String> {
                unsafe {
                    // Keep per-host cursor semantics, but watch the entire selected volume.
                    // Hard-link aliases outside the root remain visible; unrelated volumes
                    // cannot share inodes and need not force fallback for their clone events.
                    // No directory enumeration or content access is performed here.
                    let string = CFStringCreateWithBytes(
                        ptr::null(),
                        volume_text.as_ptr(),
                        volume_text.len() as isize,
                        0x08000100,
                        0,
                    );
                    if string.is_null() {
                        return Err("Cannot encode filesystem event root".into());
                    }
                    let array = CFArrayCreate(ptr::null(), &string, 1, ptr::null());
                    if array.is_null() {
                        CFRelease(string);
                        return Err("Cannot create filesystem event roots".into());
                    }
                    let queue =
                        dispatch_queue_create(c"app.hahakigami.history".as_ptr(), ptr::null());
                    let mut context = Context {
                        version: 0,
                        info: (&*state as *const State).cast_mut().cast(),
                        retain: ptr::null(),
                        release: ptr::null(),
                        description: ptr::null(),
                    };
                    let stream = FSEventStreamCreate(
                        ptr::null(),
                        callback,
                        &mut context,
                        array,
                        since,
                        0.01,
                        0xd5, // FileEvents | WatchRoot | FullHistory | UseCFTypes | UseExtendedData
                    );
                    CFRelease(array);
                    // The array uses borrowed pointers (NULL callbacks); retain the
                    // CFString owner until the stream and dispatch callbacks are gone.
                    if queue.is_null() || stream.is_null() {
                        if !stream.is_null() {
                            FSEventStreamRelease(stream);
                        }
                        if !queue.is_null() {
                            dispatch_release(queue);
                        }
                        CFRelease(string);
                        return Err("Cannot create filesystem event stream".into());
                    }
                    FSEventStreamSetDispatchQueue(stream, queue);
                    let started = FSEventStreamStart(stream) != 0;
                    let result = if !started {
                        Err("Cannot start filesystem event history".into())
                    } else {
                        FSEventStreamFlushAsync(stream);
                        wait_for_replay(&state, cancel, Duration::from_secs(5), || {
                            let joined = worker.take().expect("loader joined once").join();
                            loaded = Some(
                                joined
                                    .unwrap_or_else(|_| Err("Cache loading worker failed".into())),
                            );
                            FSEventStreamFlushAsync(stream)
                        })
                    };
                    if started {
                        FSEventStreamStop(stream);
                    }
                    FSEventStreamInvalidate(stream);
                    dispatch_sync_f(queue, ptr::null_mut(), barrier);
                    FSEventStreamRelease(stream);
                    dispatch_release(queue);
                    CFRelease(string);
                    result?;
                }
                Ok(())
            })();
            if replay_result.is_err() {
                state.abort.store(true, Ordering::Relaxed);
            }
            if let Some(worker) = worker {
                loaded = Some(
                    worker
                        .join()
                        .unwrap_or_else(|_| Err("Cache loading worker failed".into())),
                );
            }
            replay_result?;
            loaded.expect("loader always joined")
        })?;
        let mut collected = state
            .collected
            .into_inner()
            .map_err(|_| "Filesystem event callback failed")?;
        if let Some(error) = collected.error {
            return Err(error);
        }
        validate_volume(root, volume)?;
        if journal_identity(root).as_ref() != Some(&journal) {
            return Err("Filesystem event journal changed during replay".into());
        }
        collected.changes.paths.sort();
        collected.changes.paths.dedup();
        Ok((loaded, collected.changes))
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn event_paths_keep_composed_and_decomposed_names_as_stored() {
            // "é" stored composed (NFC) and decomposed (NFD): each must come back byte for byte.
            for name in ["/scope/caf\u{e9}/\u{ac00}", "/scope/cafe\u{301}"] {
                let string = unsafe {
                    CFStringCreateWithBytes(
                        ptr::null(),
                        name.as_ptr(),
                        name.len() as isize,
                        0x08000100,
                        0,
                    )
                };
                assert!(!string.is_null());
                let path = unsafe { string_path(string) };
                unsafe { CFRelease(string) };
                assert_eq!(path.unwrap(), Path::new(name));
            }
        }
        #[test]
        #[ignore = "opt-in native journal: a composed (NFC) folder name"]
        fn native_event_under_a_composed_name_maps_to_its_scanned_path() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap().join("caf\u{e9}");
            std::fs::create_dir(&root).unwrap();
            let root = root.canonicalize().unwrap();
            assert!(
                root.to_str().unwrap().ends_with("caf\u{e9}"),
                "APFS keeps the NFC spelling"
            );
            let volume = volume_mount(&root).unwrap();
            let since = current_id().unwrap();
            let path = root.join("changed");
            let (_, changes) =
                replay_with_load_on_volume(&root, &volume, since, &AtomicBool::new(false), || {
                    std::thread::sleep(Duration::from_millis(400));
                    std::fs::write(&path, "during decoding").unwrap();
                    Ok(())
                })
                .unwrap();
            assert!(
                changes.paths.iter().any(|p| p == &path || p == &root)
                    || changes.directories.iter().any(|p| p == &root),
                "{changes:?}"
            );
        }
        #[test]
        #[ignore = "opt-in native loader journal overlap"]
        fn native_loading_changes_and_failures() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let volume = volume_mount(&root).unwrap();
            let since = current_id().unwrap();
            let path = root.join("changed-during-load");
            let (value, changes) =
                replay_with_load_on_volume(&root, &volume, since, &AtomicBool::new(false), || {
                    // Outlast stream startup/history on ordinary local volumes.
                    std::thread::sleep(Duration::from_millis(400));
                    std::fs::write(&path, "during decoding").unwrap();
                    Ok(42)
                })
                .unwrap();
            assert_eq!(value, 42);
            assert!(
                changes.paths.iter().any(|p| p == &path || p == &root)
                    || changes.directories.iter().any(|p| p == &root),
                "{changes:?}"
            );
            let error = replay_with_load_on_volume::<()>(
                &root,
                &volume,
                current_id().unwrap(),
                &AtomicBool::new(false),
                || Err("decode failure".into()),
            )
            .unwrap_err();
            assert_eq!(error, "decode failure");
            let error = replay_with_load_on_volume::<()>(
                &root,
                &volume,
                current_id().unwrap(),
                &AtomicBool::new(false),
                || panic!("simulated decoder panic"),
            )
            .unwrap_err();
            assert_eq!(error, "Cache loading worker failed");
            replay_on_volume(
                &root,
                &volume,
                current_id().unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap();
        }
        #[test]
        #[ignore = "opt-in native loader stop after a failed replay"]
        fn a_failed_replay_tells_the_loader_to_stop() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let volume = volume_mount(&root).unwrap();
            let cancel = AtomicBool::new(false);
            let stopped = AtomicBool::new(false);
            let error = replay_with_abortable_load_on_volume::<()>(
                &root,
                &volume,
                current_id().unwrap(),
                &cancel,
                |abort| {
                    // Fails the replay while this loader runs, then waits to be told to stop.
                    cancel.store(true, Ordering::Relaxed);
                    let give_up = Instant::now() + Duration::from_secs(10);
                    while !abort.load(Ordering::Relaxed) && Instant::now() < give_up {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    stopped.store(abort.load(Ordering::Relaxed), Ordering::Relaxed);
                    Ok(())
                },
            )
            .unwrap_err();
            assert_eq!(error, "Filesystem event replay cancelled");
            assert!(stopped.load(Ordering::Relaxed));
        }
        #[test]
        fn an_error_from_the_stream_tells_the_loader_to_stop() {
            // History is done, so the flush waits on the loader; the stream then fails.
            let state = State {
                root: PathBuf::from("/private/tmp/root"),
                since: 10,
                collected: Mutex::new(Collected {
                    done: true,
                    ..Collected::default()
                }),
                ready: Condvar::new(),
                abort: AtomicBool::new(false),
            };
            let stopped = AtomicBool::new(false);
            let error = std::thread::scope(|scope| {
                let state = &state;
                let (send, receive) = std::sync::mpsc::channel();
                scope.spawn(move || {
                    receive.recv().unwrap();
                    // A batch with no path array: the callback records an error.
                    unsafe {
                        callback(
                            ptr::null(),
                            (state as *const State).cast_mut().cast(),
                            1,
                            ptr::null_mut(),
                            ptr::null(),
                            ptr::null(),
                        )
                    };
                });
                wait_for_replay(
                    state,
                    &AtomicBool::new(false),
                    Duration::from_secs(5),
                    || {
                        send.send(()).unwrap();
                        let give_up = Instant::now() + Duration::from_secs(2);
                        while !state.abort.load(Ordering::Relaxed) && Instant::now() < give_up {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        stopped.store(state.abort.load(Ordering::Relaxed), Ordering::Relaxed);
                        0
                    },
                )
                .unwrap_err()
            });
            assert_eq!(error, "Malformed filesystem event array");
            assert!(stopped.load(Ordering::Relaxed));
        }
        #[test]
        fn invalid_history_does_not_start_loader() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let volume = volume_mount(&root).unwrap();
            let called = AtomicBool::new(false);
            assert!(
                replay_with_load_on_volume(&root, &volume, 0, &AtomicBool::new(false), || {
                    called.store(true, Ordering::Relaxed);
                    Ok(())
                })
                .is_err()
            );
            assert!(!called.load(Ordering::Relaxed));
        }
        #[test]
        fn history_completion_requires_the_post_history_marker() {
            let mut collected = Collected {
                max_id: 20,
                ..Default::default()
            };
            assert!(!replay_drained(&collected, Some(20)));
            collected.done = true;
            assert!(!replay_drained(&collected, None));
            assert!(!replay_drained(&collected, Some(21)));
            assert!(replay_drained(&collected, Some(20)));
            assert!(replay_drained(&collected, Some(0)));
        }
        #[test]
        fn replay_wait_observes_cancel_error_and_deadline() {
            let state = State {
                root: PathBuf::from("/private/tmp/root"),
                since: 10,
                collected: Mutex::new(Collected::default()),
                ready: Condvar::new(),
                abort: AtomicBool::new(false),
            };
            assert!(
                wait_for_replay(&state, &AtomicBool::new(true), Duration::ZERO, || 0)
                    .unwrap_err()
                    .contains("cancelled")
            );
            assert!(
                wait_for_replay(&state, &AtomicBool::new(false), Duration::ZERO, || 0)
                    .unwrap_err()
                    .contains("timeout")
            );
            state.collected.lock().unwrap().error = Some("callback error".into());
            assert_eq!(
                wait_for_replay(&state, &AtomicBool::new(false), Duration::ZERO, || 0).unwrap_err(),
                "callback error"
            );
        }
        #[test]
        fn replay_wait_wakes_for_marker_delivery() {
            let state = State {
                root: PathBuf::from("/private/tmp/root"),
                since: 10,
                collected: Mutex::new(Collected {
                    done: true,
                    ..Collected::default()
                }),
                ready: Condvar::new(),
                abort: AtomicBool::new(false),
            };
            std::thread::scope(|scope| {
                let (send, receive) = std::sync::mpsc::channel();
                let state_ref = &state;
                let waiter = scope.spawn(move || {
                    wait_for_replay(
                        state_ref,
                        &AtomicBool::new(false),
                        Duration::from_secs(1),
                        || {
                            send.send(()).unwrap();
                            21
                        },
                    )
                });
                receive.recv().unwrap();
                let mut collected = state.collected.lock().unwrap();
                collected.max_id = 21;
                state.ready.notify_one();
                drop(collected);
                waiter.join().unwrap().unwrap();
            });
        }
        #[test]
        fn replay_wait_gives_the_marker_a_fresh_timeout_after_a_slow_flush() {
            let state = State {
                root: PathBuf::from("/private/tmp/root"),
                since: 10,
                collected: Mutex::new(Collected {
                    done: true,
                    ..Collected::default()
                }),
                ready: Condvar::new(),
                abort: AtomicBool::new(false),
            };
            std::thread::scope(|scope| {
                let (send, receive) = std::sync::mpsc::channel();
                let state_ref = &state;
                // Delivers the marker shortly after the flush returned, when the first timeout is long gone.
                scope.spawn(move || {
                    receive.recv().unwrap();
                    std::thread::sleep(Duration::from_millis(20));
                    let mut collected = state_ref.collected.lock().unwrap();
                    collected.max_id = 21;
                    state_ref.ready.notify_one();
                });
                wait_for_replay(
                    &state,
                    &AtomicBool::new(false),
                    Duration::from_millis(50),
                    || {
                        std::thread::sleep(Duration::from_millis(100));
                        send.send(()).unwrap();
                        21
                    },
                )
                .unwrap();
            });
        }
        #[test]
        fn volume_scope_is_the_local_mount_not_the_scan_root() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let volume = volume_mount(&root).unwrap();
            assert_ne!(volume, root);
            validate_volume(&root, &volume).unwrap();
            assert!(validate_volume(&root, &root).is_err());
            assert!(validate_volume(&root, Path::new("relative")).is_err());
        }
        #[test]
        #[ignore = "opt-in native unchanged-volume replay timing"]
        fn native_unchanged_timing() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            for _ in 0..5 {
                let since = current_id().unwrap();
                let start = Instant::now();
                let result = replay(&root, since, &AtomicBool::new(false));
                match result {
                    Ok(paths) => eprintln!(
                        "replay_ms={} changed_paths={}",
                        start.elapsed().as_millis(),
                        paths.paths.len()
                    ),
                    Err(error) => {
                        eprintln!("replay_ms={} fallback={error}", start.elapsed().as_millis())
                    }
                }
            }
        }
        #[test]
        fn unsafe_flags_force_fallback() {
            for flag in [1, 2, 4, 8, 32, 64, 128, 0x800000] {
                assert!(classify(Path::new("/tmp/root"), Path::new("/tmp/root/a"), flag).is_err());
            }
        }
        #[test]
        fn paths_and_sentinel() {
            assert!(classify(Path::new("/parent/scanned"), Path::new("/parent"), 0x800).is_err());

            assert!(old_unrelated_mount(
                Path::new("/Users/demo"),
                Path::new("/Volumes/other"),
                0xc0,
                1,
                2
            ));
            assert!(!old_unrelated_mount(
                Path::new("/Users/demo"),
                Path::new("/Users/demo/mount"),
                0xc0,
                1,
                2
            ));
            assert!(!old_unrelated_mount(
                Path::new("/Users/demo"),
                Path::new("/Volumes/other"),
                4,
                1,
                2
            ));
            assert_eq!(
                mapped(
                    Path::new("/Users/demo"),
                    Path::new("/System/Volumes/Data/Users/demo/a")
                ),
                Some(PathBuf::from("/Users/demo/a"))
            );
            assert_eq!(
                mapped(
                    Path::new("/private/var/example"),
                    Path::new("/var/example/a")
                ),
                Some(PathBuf::from("/private/var/example/a"))
            );
            assert!(classify(
                Path::new("/Users/Demo"),
                Path::new("/users/demo/file"),
                0x100
            )
            .is_err());
            let root = Path::new("/private/tmp/root");
            assert_eq!(classify(root, &root.join("a"), 0x10100), Ok(true));
            assert_eq!(classify(root, Path::new(""), HISTORY_DONE), Ok(false));
            assert_eq!(classify(root, Path::new("/tmp/root/a"), 0), Ok(true));
            assert_eq!(
                classify(root, Path::new("/outside/clone"), 0x410900),
                Ok(false)
            );
            assert_eq!(classify(root, &root.join("clone"), 0x410900), Ok(true));
            assert!(classify(root, Path::new("relative"), 0).is_err());
        }
        #[test]
        fn overlap_and_old_dropped_notices_are_preserved() {
            let state = State {
                root: PathBuf::from("/private/tmp/root"),
                since: 10,
                collected: Mutex::new(Collected::default()),
                ready: Condvar::new(),
                abort: AtomicBool::new(false),
            };
            let mut collected = state.collected.lock().unwrap();
            collect_event(
                &state,
                &mut collected,
                Path::new("/private/tmp/root/file"),
                0x10100,
                1,
                Some(42),
            );
            assert_eq!(collected.changes.paths.len(), 1);
            collect_event(
                &state,
                &mut collected,
                Path::new("/private/tmp/root/file"),
                4,
                1,
                Some(42),
            );
            assert!(collected.error.is_some());
        }
        #[test]
        fn external_shared_events_resolve_only_matching_cached_inodes() {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("file"), b"data").unwrap();
            let snapshot = disk_scan::scan(
                dir.path().to_path_buf(),
                disk_scan::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let node = snapshot
                .iter_nodes()
                .find(|node| !node.is_directory)
                .unwrap();
            let state = State {
                root: snapshot.root.clone(),
                since: 10,
                collected: Mutex::new(Collected::default()),
                ready: Condvar::new(),
                abort: AtomicBool::new(false),
            };
            let mut collected = Collected::default();
            collect_event(
                &state,
                &mut collected,
                Path::new("/outside/alias"),
                0x110400,
                1,
                Some(node.identity.inode),
            );
            assert!(collected.error.is_none());
            assert!(collected.changes.paths.is_empty());
            assert_eq!(
                collected.changes.resolve(&snapshot).unwrap().paths,
                vec![node.path().unwrap()]
            );
            let changes = Changes {
                shared_inodes: HashSet::from([u64::MAX]),
                ..Default::default()
            };
            assert!(changes.resolve(&snapshot).unwrap().paths.is_empty());
            let changes = Changes {
                shared_inodes: HashSet::from([node.identity.inode]),
                ..Default::default()
            };
            let mut other_device = snapshot.clone();
            other_device
                .update_node(node.id, |n| n.identity.device ^= 1)
                .unwrap();
            assert!(changes.resolve(&other_device).unwrap().paths.is_empty());
        }
        #[test]
        fn external_file_events_without_hardlink_flags_resolve_all_aliases() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("scope");
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("a"), b"data").unwrap();
            std::fs::hard_link(root.join("a"), root.join("b")).unwrap();
            let snapshot = disk_scan::scan(
                root,
                disk_scan::ScanOptions::default(),
                std::sync::Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let inode = snapshot
                .iter_nodes()
                .find(|n| n.hard_link)
                .unwrap()
                .identity
                .inode;
            let state = State {
                root: snapshot.root.clone(),
                since: 10,
                collected: Mutex::new(Collected::default()),
                ready: Condvar::new(),
                abort: AtomicBool::new(false),
            };
            let mut collected = Collected::default();
            collect_event(
                &state,
                &mut collected,
                Path::new("/outside/alias"),
                0x10400,
                11,
                Some(inode),
            ); // ItemIsFile | ItemModified, no hard-link flag
            let changes = collected.changes.resolve(&snapshot).unwrap();
            assert_eq!(changes.paths.len(), 2);
            assert!(snapshot
                .iter_nodes()
                .filter(|n| n.hard_link)
                .all(|n| changes.paths.contains(&n.path().unwrap())));
            for (flag, identity) in [(0x10400, None), (0x20200, Some(99))] {
                let mut collected = Collected::default();
                collect_event(
                    &state,
                    &mut collected,
                    Path::new("/outside/alias"),
                    flag,
                    11,
                    identity,
                );
                let changes = collected.changes.resolve(&snapshot).unwrap();
                assert_eq!(changes.paths.len(), 2);
                assert!(snapshot
                    .iter_nodes()
                    .filter(|n| n.hard_link)
                    .all(|n| changes.paths.contains(&n.path().unwrap())));
            }
        }
        #[test]
        fn directory_metadata_invalidates_listings_but_namespace_changes_stay_broad() {
            let state = State {
                root: PathBuf::from("/private/tmp/root"),
                since: 10,
                collected: Mutex::new(Collected::default()),
                ready: Condvar::new(),
                abort: AtomicBool::new(false),
            };
            let mut collected = Collected::default();
            collect_event(
                &state,
                &mut collected,
                Path::new("/private/tmp"),
                0x28100,
                1,
                Some(42),
            );
            assert!(collected.error.is_none());
            collect_event(
                &state,
                &mut collected,
                Path::new("/private/tmp/root/child"),
                0x28100,
                1,
                Some(43),
            );
            assert!(collected.changes.paths.is_empty());
            assert_eq!(
                collected.changes.directories,
                vec![PathBuf::from("/private/tmp/root/child")]
            );
            for flag in [0x20800, 0x20200, 0x21000, 0] {
                let mut collected = Collected::default();
                collect_event(
                    &state,
                    &mut collected,
                    Path::new("/private/tmp"),
                    flag,
                    1,
                    Some(42),
                );
                assert!(collected.error.is_some());
            }
        }
        #[test]
        fn shared_event_identity_and_history_guards() {
            let state = State {
                root: PathBuf::from("/private/tmp/root"),
                since: 10,
                collected: Mutex::new(Collected::default()),
                ready: Condvar::new(),
                abort: AtomicBool::new(false),
            };
            for flags in [0x100000, 0x200000, 0x510900] {
                for inode in [None, Some(0)] {
                    let mut collected = Collected::default();
                    collect_event(
                        &state,
                        &mut collected,
                        Path::new("/outside/alias"),
                        flags,
                        1,
                        inode,
                    );
                    assert!(collected.error.unwrap().contains("identity"));
                }
            }
            for flags in [0x410901, 0x410902, 0x410904, 0x410908, 0xc10900] {
                let mut collected = Collected::default();
                collect_event(
                    &state,
                    &mut collected,
                    Path::new("/outside/clone"),
                    flags,
                    1,
                    Some(42),
                );
                assert!(collected.error.is_some());
            }
            for path in ["/private/tmp", "/PRIVATE/TMP/ROOT/file"] {
                let mut collected = Collected::default();
                collect_event(
                    &state,
                    &mut collected,
                    Path::new(path),
                    0x410900,
                    1,
                    Some(42),
                );
                assert!(collected.error.is_some());
            }
            let mut collected = Collected::default();
            collect_event(
                &state,
                &mut collected,
                Path::new("/outside/clone"),
                0x410900,
                1,
                None,
            );
            assert!(collected.error.is_none());
            assert!(collected.changes.paths.is_empty());
        }
        #[test]
        fn invalid_cursor_and_cancellation() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            assert!(replay(&root, 0, &AtomicBool::new(false)).is_err());
            assert!(replay(&root, u64::MAX, &AtomicBool::new(false)).is_err());
            assert!(replay(&root, current_id().unwrap(), &AtomicBool::new(true))
                .unwrap_err()
                .contains("cancelled"));
        }
        #[test]
        #[ignore = "opt-in native FSEvents outside-root hardlink fixture"]
        fn native_outside_hardlink() {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path().canonicalize().unwrap();
            let root = base.join("scanned");
            std::fs::create_dir(&root).unwrap();
            let original = root.join("original");
            std::fs::write(&original, b"seed").unwrap();
            std::thread::sleep(Duration::from_millis(200));
            let since = current_id().unwrap();
            let alias = base.join("outside-alias");
            std::fs::hard_link(&original, &alias).unwrap();
            std::fs::write(&alias, b"larger fixture data through external alias").unwrap();
            std::thread::sleep(Duration::from_millis(200));
            if let Ok(paths) = replay(&root, since, &AtomicBool::new(false)) {
                assert!(
                    paths
                        .paths
                        .iter()
                        .any(|path| path == &root || path == &original)
                        || !paths.shared_inodes.is_empty(),
                    "External alias modification must invalidate original or force full scan"
                );
            }
        }
        #[test]
        #[ignore = "opt-in native external hard-link path replacement"]
        fn native_external_alias_replacement() {
            use std::os::unix::fs::MetadataExt;
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path().canonicalize().unwrap();
            let root = base.join("scanned");
            std::fs::create_dir(&root).unwrap();
            let first = root.join("first");
            let second = root.join("second");
            std::fs::write(&first, b"first seed").unwrap();
            std::fs::write(&second, b"second seed").unwrap();
            let original_inodes = [
                std::fs::metadata(&first).unwrap().ino(),
                std::fs::metadata(&second).unwrap().ino(),
            ];
            let since = current_id().unwrap();
            let alias = base.join("outside-alias");
            for file in [&first, &second] {
                std::fs::hard_link(file, &alias).unwrap();
                std::fs::write(&alias, b"larger content through reused external alias").unwrap();
                std::fs::remove_file(&alias).unwrap();
            }
            std::fs::write(&alias, b"unrelated replacement inode").unwrap();
            let changes = replay(&root, since, &AtomicBool::new(false)).unwrap();
            eprintln!(
                "alias replacement: affected_originals={}, retained_originals={}",
                original_inodes.len(),
                original_inodes
                    .iter()
                    .filter(|inode| changes.shared_inodes.contains(inode))
                    .count()
            );
            assert!(
                original_inodes
                    .iter()
                    .all(|inode| changes.shared_inodes.contains(inode)),
                "coalesced external aliases must preserve every affected inode"
            );
        }
        #[test]
        #[ignore = "opt-in native FSEvents ancestor rename round trip"]
        fn native_ancestor_round_trip() {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path().canonicalize().unwrap();
            let parent = base.join("parent");
            let root = parent.join("scanned");
            std::fs::create_dir_all(root.join("clean")).unwrap();
            std::fs::write(root.join("clean/file"), b"seed").unwrap();
            std::thread::sleep(Duration::from_millis(200));
            let since = current_id().unwrap();
            let moved = base.join("moved");
            std::fs::rename(&parent, &moved).unwrap();
            std::fs::write(
                moved.join("scanned/clean/file"),
                b"larger content after ancestor relocation",
            )
            .unwrap();
            std::fs::rename(&moved, &parent).unwrap();
            // Immediate replay also exercises flushing queued events after HistoryDone.
            let result = replay(&root, since, &AtomicBool::new(false));
            match result {
                Err(_) => {}
                Ok(paths) => assert!(
                    paths
                        .paths
                        .iter()
                        .any(|p| p == &root || p == &root.join("clean/file")),
                    "ancestor relocation must invalidate cached descendants"
                ),
            }
        }
        #[test]
        #[ignore = "opt-in native FSEvents replay on disposable local directory"]
        fn native_history() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let since = current_id().unwrap();
            std::fs::write(root.join("changed"), b"fixture").unwrap();
            std::thread::sleep(Duration::from_millis(200));
            match replay(&root, since, &AtomicBool::new(false)) {
                Ok(paths) => assert!(paths
                    .paths
                    .iter()
                    .any(|path| path == &root || path == &root.join("changed"))),
                Err(error) => assert!(
                    error.contains("full scan")
                        || error.contains("Incomplete filesystem event history"),
                    "{error}"
                ),
            }
        }
    }
}
#[cfg(all(test, target_os = "macos"))]
pub use native::replay_with_load_on_volume;
#[cfg(target_os = "macos")]
pub use native::{
    boot_session, current_id, journal_identity, replay_with_abortable_load_on_volume,
};
