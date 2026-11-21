//! Unix descriptor-relative enumeration: path replacements can never enable symlink traversal.
use crate::{Entry, Identity, NodeKind};
use std::{
    ffi::{CStr, CString},
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::{Component, Path},
    sync::atomic::{AtomicBool, Ordering},
};
fn cstr(name: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))
}
/// A validated single component, encoded once when directory work is queued.
pub(crate) struct ChildName(CString);
impl ChildName {
    #[cfg(target_os = "macos")]
    pub(crate) fn as_c_str(&self) -> &CStr {
        &self.0
    }
    pub(crate) fn new(name: &std::ffi::OsStr) -> io::Result<Self> {
        if !crate::tree::is_exact_native_component(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "single child name required",
            ));
        }
        cstr(name).map(Self)
    }
}
fn stat_kind(stat: &libc::stat) -> NodeKind {
    match stat.st_mode & libc::S_IFMT {
        libc::S_IFDIR => NodeKind::Directory,
        libc::S_IFREG => NodeKind::File,
        libc::S_IFLNK => NodeKind::Symlink,
        _ => NodeKind::Other,
    }
}
#[allow(clippy::unnecessary_cast)] // Native inode field width varies across Unix targets.
fn stat_identity(stat: &libc::stat) -> Identity {
    Identity {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
        kind: stat_kind(stat),
    }
}
pub(crate) fn syscall(mut operation: impl FnMut() -> libc::c_int) -> io::Result<libc::c_int> {
    crate::retry::interrupted(None, |_| {
        let result = operation();
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result)
        }
    })
}
pub(crate) fn identity_fd(fd: &OwnedFd) -> io::Result<Identity> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    syscall(|| unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) })?;
    Ok(stat_identity(&stat))
}
/// Fresh no-follow identity check for a cached directory edge.
#[cfg(target_os = "macos")]
pub(crate) fn identity_child(fd: &OwnedFd, name: &ChildName) -> io::Result<Identity> {
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    syscall(|| unsafe {
        libc::fstatat(
            fd.as_raw_fd(),
            name.0.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    Ok(stat_identity(&stat))
}
/// Opens every component without following links, then validates the queued directory identity.
pub(crate) fn open(path: &Path, expected: &Identity) -> io::Result<OwnedFd> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "absolute path required",
        ));
    }
    let raw = syscall(|| unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    })?;
    let mut fd = unsafe { OwnedFd::from_raw_fd(raw) };
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let name = cstr(name)?;
                let raw = syscall(|| unsafe {
                    libc::openat(
                        fd.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                })?;
                fd = unsafe { OwnedFd::from_raw_fd(raw) };
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "non-normal directory path",
                ))
            }
        }
    }
    if identity_fd(&fd)? != *expected {
        return Err(replaced());
    }
    Ok(fd)
}
#[cfg(target_os = "macos")]
fn file_length(fd: &OwnedFd, name: &CStr, expected: &Identity) -> io::Result<u64> {
    let mut list: libc::attrlist = unsafe { std::mem::zeroed() };
    list.bitmapcount = 5;
    list.commonattr = libc::ATTR_CMN_RETURNED_ATTRS
        | libc::ATTR_CMN_DEVID
        | libc::ATTR_CMN_OBJTYPE
        | libc::ATTR_CMN_FILEID;
    list.fileattr = libc::ATTR_FILE_TOTALSIZE;
    // Darwin attribute buffers align even 64-bit values to four bytes.
    #[repr(C, packed(4))]
    struct Attributes {
        length: u32,
        returned: libc::attribute_set_t,
        device: i32,
        kind: u32,
        inode: u64,
        file_length: i64,
    }
    let mut attrs: Attributes = unsafe { std::mem::zeroed() };
    syscall(|| unsafe {
        libc::getattrlistat(
            fd.as_raw_fd(),
            name.as_ptr(),
            (&mut list as *mut libc::attrlist).cast(),
            (&mut attrs as *mut Attributes).cast(),
            std::mem::size_of::<Attributes>(),
            libc::FSOPT_NOFOLLOW as libc::c_ulong,
        )
    })?;
    // The common attributes come first, so identity is readable even when a directory omits the file size.
    if (attrs.length as usize) < std::mem::offset_of!(Attributes, file_length)
        || attrs.returned.commonattr != list.commonattr
    {
        return Err(io::Error::other(
            "file attributes changed since enumeration",
        ));
    }
    // Another file, directory or link now holds the name (atomic save): the enumerated file vanished.
    if attrs.device as u64 != expected.device || attrs.inode != expected.inode || attrs.kind != 1
    // VREG
    {
        return Err(io::Error::from_raw_os_error(libc::ENOENT));
    }
    if attrs.length as usize != std::mem::size_of::<Attributes>()
        || attrs.returned.volattr != 0
        || attrs.returned.dirattr != 0
        || attrs.returned.fileattr != list.fileattr
        || attrs.returned.forkattr != 0
        || attrs.file_length < 0
    {
        return Err(io::Error::other(
            "file attributes changed since enumeration",
        ));
    }
    Ok(attrs.file_length as u64)
}
pub(crate) fn entry_at(fd: &OwnedFd, path: &Path, name: &std::ffi::OsStr) -> io::Result<Entry> {
    let name_c = cstr(name)?;
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    syscall(|| unsafe {
        libc::fstatat(
            fd.as_raw_fd(),
            name_c.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    let identity = stat_identity(&stat);
    let directory = identity.kind == NodeKind::Directory;
    let length = if directory {
        0
    } else {
        stat.st_size.max(0) as u64
    };
    #[cfg(target_os = "macos")]
    let length = if identity.kind == NodeKind::File {
        // Match Bulk's total-fork sizes without opening file contents.
        file_length(fd, &name_c, &identity)?
    } else {
        length
    };
    Ok(Entry {
        clone_checked: false,
        path: crate::child_path(path, name).into(),
        hard_link: identity.kind == NodeKind::File && stat.st_nlink > 1,
        device: identity.device,
        inode: identity.inode,
        kind: identity.kind,
        length,
        allocated: if directory {
            0
        } else {
            (stat.st_blocks.max(0) as u64).saturating_mul(512)
        },
    })
}
#[cfg(target_os = "macos")]
fn errno() -> *mut libc::c_int {
    unsafe { libc::__error() }
}
#[cfg(not(target_os = "macos"))]
fn errno() -> *mut libc::c_int {
    unsafe { libc::__errno_location() }
}
pub(crate) fn enumerate(
    fd: &OwnedFd,
    path: &Path,
    cancel: &AtomicBool,
) -> io::Result<Vec<io::Result<Entry>>> {
    let duplicate = syscall(|| unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) })?;
    let directory = unsafe { libc::fdopendir(duplicate) };
    if directory.is_null() {
        let error = io::Error::last_os_error();
        unsafe { libc::close(duplicate) };
        return Err(error);
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            unsafe { libc::closedir(self.0) };
        }
    }
    let directory = Directory(directory);
    unsafe {
        libc::rewinddir(directory.0);
    }
    let mut entries = vec![];
    while !cancel.load(Ordering::Relaxed) {
        unsafe { *errno() = 0 };
        let item = unsafe { libc::readdir(directory.0) };
        if item.is_null() {
            let error = unsafe { *errno() };
            if error != 0 {
                return crate::retry::terminal_listing(
                    entries,
                    io::Error::from_raw_os_error(error),
                );
            }
            break;
        }
        let bytes = unsafe { CStr::from_ptr((*item).d_name.as_ptr()) }.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let name = std::ffi::OsStr::from_bytes(bytes);
        entries.push(crate::entry_result(entry_at(fd, path, name), || {
            path.join(name)
        }));
    }
    Ok(entries)
}

/// A nonblocking budget caps retained parent descriptors; pressure falls back to secure path walks.
#[derive(Clone)]
pub(crate) struct Budget(std::sync::Arc<BudgetInner>);
struct BudgetInner {
    count: std::sync::atomic::AtomicUsize,
    peak: std::sync::atomic::AtomicUsize,
    limit: usize,
}
pub(crate) struct Lease {
    pub(crate) fd: OwnedFd,
    budget: Budget,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget.0.count.fetch_sub(1, Ordering::Relaxed);
    }
}
pub(crate) enum Opened {
    Retained(std::sync::Arc<Lease>),
    Temporary(OwnedFd),
}
impl Opened {
    pub(crate) fn fd(&self) -> &OwnedFd {
        match self {
            Self::Retained(l) => &l.fd,
            Self::Temporary(fd) => fd,
        }
    }
    pub(crate) fn parent(&self) -> Option<std::sync::Arc<Lease>> {
        match self {
            Self::Retained(l) => Some(l.clone()),
            Self::Temporary(_) => None,
        }
    }
}
impl Budget {
    pub(crate) fn limit(&self) -> usize {
        self.0.limit
    }
    pub(crate) fn peak(&self) -> usize {
        self.0.peak.load(Ordering::Relaxed)
    }
    /// What one scan retains when nothing narrows it: RLIMIT_NOFILE less headroom, clamped 1..=512.
    pub(crate) fn default_limit() -> usize {
        let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0 {
            (limit.rlim_cur as usize).saturating_sub(64).clamp(1, 512)
        } else {
            128
        }
    }
    pub(crate) fn with_limit(limit: usize) -> Self {
        Self(std::sync::Arc::new(BudgetInner {
            count: std::sync::atomic::AtomicUsize::new(0),
            peak: std::sync::atomic::AtomicUsize::new(0),
            limit: limit.max(1),
        }))
    }
    pub(crate) fn retain(&self, fd: OwnedFd) -> Opened {
        let mut count = self.0.count.load(Ordering::Relaxed);
        loop {
            if count >= self.0.limit {
                return Opened::Temporary(fd);
            }
            match self.0.count.compare_exchange_weak(
                count,
                count + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.0.peak.fetch_max(count + 1, Ordering::Relaxed);
                    return Opened::Retained(std::sync::Arc::new(Lease {
                        fd,
                        budget: self.clone(),
                    }));
                }
                Err(next) => count = next,
            }
        }
    }
}
/// A held parent descriptor remains authorized even if its namespace ancestors are renamed.
#[cfg(test)]
pub(crate) fn open_child(
    parent: &OwnedFd,
    name: &std::ffi::OsStr,
    expected: &Identity,
) -> io::Result<OwnedFd> {
    open_child_name(parent, &ChildName::new(name)?, expected)
}
pub(crate) fn open_child_name(
    parent: &OwnedFd,
    name: &ChildName,
    expected: &Identity,
) -> io::Result<OwnedFd> {
    open_at(parent, &name.0, expected, None)
}
/// As `open_child_name`, with the `openat` call itself bounded by `gate`.
pub(crate) fn open_child_name_gated(
    parent: &OwnedFd,
    name: &ChildName,
    expected: &Identity,
    gate: &OpenGate,
) -> io::Result<OwnedFd> {
    open_at(parent, &name.0, expected, Some(gate))
}
/// Names and identities from `id` up to (excluding) the root, leaf first, plus the root identity.
/// Collected under the store lock so the opens can run after it is released.
pub(crate) fn path_names(
    s: &crate::Snapshot,
    id: u32,
) -> io::Result<(Vec<(ChildName, Identity)>, Identity)> {
    let mut names = vec![];
    let mut next = Some(id);
    while let Some(i) = next.filter(|i| *i != 0) {
        let n = s.node(i).unwrap();
        names.push((ChildName::new(n.native_name())?, n.identity));
        next = n.parent;
    }
    Ok((names, s.node(0).unwrap().identity))
}
/// Reopens a `path_names` chain by walking its names down from the held root. A renamed ancestor
/// is not the leaf vanishing: it reads as a plain failure so the scan is marked incomplete
/// instead of silently dropping the subtree.
pub(crate) fn open_names(
    root: &OwnedFd,
    root_identity: &Identity,
    names: &[(ChildName, Identity)],
    gate: Option<&OpenGate>,
) -> io::Result<OwnedFd> {
    let mut fd = reopen(root, root_identity)?;
    for (i, (name, identity)) in names.iter().enumerate().rev() {
        fd = open_at(&fd, &name.0, identity, gate).map_err(|e| {
            let gone = matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR));
            if i > 0 && (gone || is_replaced(&e)) {
                io::Error::other("an ancestor directory changed during the scan")
            } else {
                e
            }
        })?;
    }
    Ok(fd)
}
/// Open a fresh directory cursor anchored to an existing authorized descriptor.
pub(crate) fn reopen(parent: &OwnedFd, expected: &Identity) -> io::Result<OwnedFd> {
    open_at(parent, c".", expected, None)
}
/// Bounds concurrent directory `openat` calls across scan workers. Trees of
/// many tiny directories otherwise contend inside the kernel lookup path, and
/// extra concurrent opens make every open slower. Listing, identity checks and
/// commits stay fully parallel; a limit at or above the worker count is inert.
pub(crate) struct OpenGate {
    limit: usize,
    active: std::sync::Mutex<usize>,
    released: std::sync::Condvar,
}
impl OpenGate {
    pub(crate) fn new(limit: usize, workers: usize) -> Self {
        Self {
            limit: if limit >= workers { 0 } else { limit.max(1) },
            active: std::sync::Mutex::new(0),
            released: std::sync::Condvar::new(),
        }
    }
    fn enter(&self) -> Option<OpenPermit<'_>> {
        if self.limit == 0 {
            return None;
        }
        let mut active = self.active.lock().unwrap();
        while *active >= self.limit {
            active = self.released.wait(active).unwrap();
        }
        *active += 1;
        Some(OpenPermit(self))
    }
    #[cfg(test)]
    fn active(&self) -> usize {
        *self.active.lock().unwrap()
    }
}
struct OpenPermit<'a>(&'a OpenGate);
impl Drop for OpenPermit<'_> {
    fn drop(&mut self) {
        *self.0.active.lock().unwrap() -= 1;
        self.0.released.notify_one();
    }
}
/// The directory at a path is no longer the one enumerated (deleted and recreated). Callers still
/// refuse to descend; the kind lets a fresh listing record it as a vanish, and survives the
/// error rebuilds in the walk. A real ESTALE carries an errno, which `is_replaced` excludes.
fn replaced() -> io::Error {
    io::Error::new(
        io::ErrorKind::StaleNetworkFileHandle,
        "directory identity changed since enumeration",
    )
}
pub(crate) fn is_replaced(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::StaleNetworkFileHandle && error.raw_os_error().is_none()
}
fn open_at(
    parent: &OwnedFd,
    name: &CStr,
    expected: &Identity,
    gate: Option<&OpenGate>,
) -> io::Result<OwnedFd> {
    let permit = gate.and_then(OpenGate::enter);
    let raw = syscall(|| unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    });
    drop(permit);
    let fd = unsafe { OwnedFd::from_raw_fd(raw?) };
    if identity_fd(&fd)? != *expected {
        return Err(replaced());
    }
    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(label: &str) -> std::path::PathBuf {
        let p =
            std::env::temp_dir().join(format!("hahakigami-lease-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir(&p).unwrap();
        std::fs::canonicalize(p).unwrap()
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn replaced_file_reads_as_vanished() {
        let p = fixture("replaced-file");
        std::fs::write(p.join("f"), b"old").unwrap();
        let expected = crate::platform_identity(&p.join("f")).unwrap();
        std::fs::write(p.join("tmp"), b"new").unwrap();
        std::fs::rename(p.join("tmp"), p.join("f")).unwrap();
        let fd = open(&p, &crate::platform_identity(&p).unwrap()).unwrap();
        let error = file_length(&fd, &cstr("f".as_ref()).unwrap(), &expected).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
        // A directory taking the name reads as vanished too, not as changed attributes.
        std::fs::remove_file(p.join("f")).unwrap();
        std::fs::create_dir(p.join("f")).unwrap();
        let error = file_length(&fd, &cstr("f".as_ref()).unwrap(), &expected).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ENOENT));
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn renamed_ancestor_is_not_a_vanished_leaf() {
        let p = fixture("renamed-ancestor");
        std::fs::create_dir_all(p.join("a/b/c")).unwrap();
        let root = open(&p, &crate::platform_identity(&p).unwrap()).unwrap();
        let root_identity = crate::platform_identity(&p).unwrap();
        // Leaf first, as `path_names` collects them.
        let names: Vec<_> = ["a/b/c", "a/b", "a"]
            .iter()
            .map(|rel| {
                let path = p.join(rel);
                (
                    ChildName::new(path.file_name().unwrap()).unwrap(),
                    crate::platform_identity(&path).unwrap(),
                )
            })
            .collect();
        assert!(open_names(&root, &root_identity, &names, None).is_ok());
        std::fs::rename(p.join("a/b/c"), p.join("a/b/c2")).unwrap();
        let leaf = open_names(&root, &root_identity, &names, None).unwrap_err();
        assert!(crate::vanished(&leaf));
        std::fs::rename(p.join("a/b/c2"), p.join("a/b/c")).unwrap();
        std::fs::rename(p.join("a"), p.join("a2")).unwrap();
        let ancestor = open_names(&root, &root_identity, &names, None).unwrap_err();
        assert!(!crate::vanished(&ancestor) && !crate::replaced(&ancestor));
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn open_gate_is_inert_at_or_above_worker_count() {
        for (limit, workers) in [(3, 3), (3, 2), (8, 7)] {
            let gate = OpenGate::new(limit, workers);
            let permits: Vec<_> = (0..workers).map(|_| gate.enter()).collect();
            assert!(permits.iter().all(Option::is_none));
            assert_eq!(gate.active(), 0);
        }
    }
    #[test]
    fn open_gate_bounds_concurrent_opens_and_releases_on_error() {
        use std::sync::atomic::AtomicUsize;
        let p = fixture("open-gate");
        let mut children = vec![];
        for i in 0..32 {
            let path = p.join(format!("d{i}"));
            std::fs::create_dir(&path).unwrap();
            children.push((
                ChildName::new(path.file_name().unwrap()).unwrap(),
                crate::platform_identity(&path).unwrap(),
            ));
        }
        let parent = open(&p, &crate::platform_identity(&p).unwrap()).unwrap();
        let gate = OpenGate::new(2, 8);
        let (inside, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let (gate, parent, children) = (&gate, &parent, &children);
                let (inside, peak) = (&inside, &peak);
                scope.spawn(move || {
                    for (name, expected) in children.iter().skip(worker % 4) {
                        let permit = gate.enter().expect("bounded gate");
                        let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        std::thread::yield_now();
                        inside.fetch_sub(1, Ordering::SeqCst);
                        drop(permit);
                        let fd = open_child_name_gated(parent, name, expected, gate).unwrap();
                        assert_eq!(identity_fd(&fd).unwrap(), *expected);
                    }
                });
            }
        });
        assert!(peak.load(Ordering::SeqCst) <= 2);
        assert_eq!(gate.active(), 0);
        // A failed or replaced open still returns its permit.
        let (name, expected) = &children[0];
        std::fs::remove_dir(p.join("d0")).unwrap();
        assert!(open_child_name_gated(&parent, name, expected, &gate).is_err());
        std::fs::create_dir(p.join("d0")).unwrap();
        assert!(open_child_name_gated(&parent, name, expected, &gate).is_err());
        assert_eq!(gate.active(), 0);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn file_sizes_keep_descriptor_authority_and_reject_replaced_entries() {
        let p = fixture("file-sizes");
        std::fs::write(p.join("file"), b"data").unwrap();
        std::fs::write(p.join("file/..namedfork/rsrc"), b"resource").unwrap();
        std::os::unix::fs::symlink("file", p.join("link")).unwrap();
        let fd = open(&p, &crate::platform_identity(&p).unwrap()).unwrap();
        let expected = crate::platform_identity(&p.join("file")).unwrap();
        let renamed = p.with_extension("renamed");
        let _ = std::fs::remove_dir_all(&renamed);
        std::fs::rename(&p, &renamed).unwrap();
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("file"), b"replacement").unwrap();
        assert_eq!(file_length(&fd, c"file", &expected).unwrap(), 12);
        let mut wrong = expected.clone();
        wrong.inode = wrong.inode.wrapping_add(1);
        assert!(file_length(&fd, c"file", &wrong).is_err());
        assert!(file_length(&fd, c"link", &expected).is_err());
        assert!(file_length(&fd, c"missing", &expected).is_err());
        std::fs::remove_dir_all(p).unwrap();
        std::fs::remove_dir_all(renamed).unwrap();
    }
    #[test]
    fn child_names_reject_sentinels_and_preserve_native_bytes() {
        for bytes in [b"".as_slice(), b".", b"..", b"a/b", b"a\0b"] {
            assert!(ChildName::new(std::ffi::OsStr::from_bytes(bytes)).is_err());
        }
        let p = fixture("native-child");
        let encoded_native = ChildName::new(std::ffi::OsStr::from_bytes(b"raw-\xff")).unwrap();
        assert_eq!(encoded_native.0.as_bytes(), b"raw-\xff");
        let name = std::ffi::OsStr::new("child");
        std::fs::create_dir(p.join(name)).unwrap();
        let expected = crate::platform_identity(&p.join(name)).unwrap();
        let parent = open(&p, &crate::platform_identity(&p).unwrap()).unwrap();
        let encoded = ChildName::new(name).unwrap();
        let budget = Budget::with_limit(1);
        let opened = Opened::Temporary(open_child_name(&parent, &encoded, &expected).unwrap());
        assert_eq!(identity_fd(opened.fd()).unwrap(), expected);
        assert!(opened.parent().is_none());
        assert_eq!(budget.peak(), 0);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn held_parent_survives_ancestor_swap_and_rejects_child_swap() {
        let p = fixture("parent");
        let outside = fixture("outside");
        std::fs::create_dir_all(p.join("parent/child")).unwrap();
        std::fs::write(p.join("parent/child/original-marker"), b"x").unwrap();
        std::fs::write(outside.join("outside-marker"), b"x").unwrap();
        let parent = p.join("parent");
        let child = parent.join("child");
        let expected = crate::platform_identity(&child).unwrap();
        let fd = open(&parent, &crate::platform_identity(&parent).unwrap()).unwrap();
        std::fs::rename(&parent, p.join("renamed")).unwrap();
        std::os::unix::fs::symlink(&outside, &parent).unwrap();
        let child_fd = open_child(&fd, std::ffi::OsStr::new("child"), &expected).unwrap();
        let entries = enumerate(&child_fd, &child, &AtomicBool::new(false)).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].as_ref().unwrap().path.file_name().unwrap(),
            "original-marker"
        );
        std::fs::rename(p.join("renamed/child"), p.join("original-child")).unwrap();
        std::os::unix::fs::symlink(&outside, p.join("renamed/child")).unwrap();
        assert!(open_child(&fd, std::ffi::OsStr::new("child"), &expected).is_err());
        // A real directory in its place is "replaced", which fresh listings treat as a vanish.
        std::fs::remove_file(p.join("renamed/child")).unwrap();
        std::fs::create_dir(p.join("renamed/child")).unwrap();
        let error = open_child(&fd, std::ffi::OsStr::new("child"), &expected).unwrap_err();
        assert!(is_replaced(&error));
        assert!(!is_replaced(&io::Error::from_raw_os_error(libc::ESTALE)));
        std::fs::remove_dir_all(p).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }
    #[test]
    fn retained_descriptor_cap_has_nonblocking_secure_fallback() {
        let p = fixture("budget");
        let expected = crate::platform_identity(&p).unwrap();
        let budget = Budget::with_limit(2);
        let a = budget.retain(open(&p, &expected).unwrap());
        let b = budget.retain(open(&p, &expected).unwrap());
        let fallback = budget.retain(open(&p, &expected).unwrap());
        assert!(matches!(fallback, Opened::Temporary(_)));
        assert_eq!(budget.0.count.load(Ordering::Relaxed), 2);
        assert_eq!(budget.limit(), 2);
        assert_eq!(budget.peak(), 2);
        let cloned = a.parent().unwrap();
        drop(a);
        assert_eq!(budget.0.count.load(Ordering::Relaxed), 2);
        drop(cloned);
        drop(b);
        drop(fallback);
        assert_eq!(budget.0.count.load(Ordering::Relaxed), 0);
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn fresh_retry_cursor_keeps_original_scope_and_does_not_share_position() {
        let p = fixture("retry-cursor");
        std::fs::write(p.join("original"), b"x").unwrap();
        let expected = crate::platform_identity(&p).unwrap();
        let fd = open(&p, &expected).unwrap();
        let renamed = p.with_extension("renamed");
        let _ = std::fs::remove_dir_all(&renamed);
        std::fs::rename(&p, &renamed).unwrap();
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("replacement"), b"x").unwrap();
        let mut attempts = 0;
        let listing = crate::retry::interrupted(None, |_| {
            attempts += 1;
            let cursor = reopen(&fd, &expected)?;
            let entries = enumerate(&cursor, &p, &AtomicBool::new(false))?;
            assert_eq!(entries.len(), 1);
            assert_eq!(
                entries[0].as_ref().unwrap().path.file_name().unwrap(),
                "original"
            );
            if attempts < 3 {
                Err(io::Error::from_raw_os_error(libc::EINTR))
            } else {
                Ok(entries)
            }
        })
        .unwrap();
        assert_eq!(attempts, 3);
        assert_eq!(listing.len(), 1);
        let fresh = reopen(&fd, &expected).unwrap();
        use std::os::fd::AsRawFd;
        assert_eq!(
            unsafe { libc::lseek(fresh.as_raw_fd(), 0, libc::SEEK_CUR) },
            0
        );
        std::fs::remove_dir_all(p).unwrap();
        std::fs::remove_dir_all(renamed).unwrap();
    }
}
