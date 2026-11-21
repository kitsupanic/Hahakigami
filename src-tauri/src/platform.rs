use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: String,
    pub name: String,
    pub path: String,
    pub total_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
    /// The user's home folder, offered on the welcome page; the startup disk is selected by default.
    pub home: bool,
    /// The startup disk or a mounted volume, listed under Disks in the sidebar.
    pub volume: bool,
}

/// Hardware core classes, not the placement of any particular thread.
#[cfg(all(target_os = "macos", any(test, feature = "performance-benchmark")))]
pub fn cpu_thread_classes() -> (Option<usize>, Option<usize>) {
    fn integer(name: &std::ffi::CStr) -> Option<usize> {
        let mut value: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&value);
        let status = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut value as *mut libc::c_int).cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        (status == 0 && length == std::mem::size_of_val(&value) && value >= 0)
            .then_some(value as usize)
    }
    fn name(key: &std::ffi::CStr) -> Option<String> {
        let mut bytes = [0u8; 64];
        let mut length = bytes.len();
        let status = unsafe {
            libc::sysctlbyname(
                key.as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 || length > bytes.len() {
            return None;
        }
        std::ffi::CStr::from_bytes_until_nul(&bytes[..length])
            .ok()?
            .to_str()
            .ok()
            .map(str::to_owned)
    }
    let p = (name(c"hw.perflevel0.name").as_deref() == Some("Performance"))
        .then(|| integer(c"hw.perflevel0.logicalcpu"))
        .flatten();
    let e = (name(c"hw.perflevel1.name").as_deref() == Some("Efficiency"))
        .then(|| integer(c"hw.perflevel1.logicalcpu"))
        .flatten();
    (p, e)
}
#[cfg(all(not(target_os = "macos"), any(test, feature = "performance-benchmark")))]
pub fn cpu_thread_classes() -> (Option<usize>, Option<usize>) {
    (None, None)
}

/// Read-only system scheduling inputs. Unknown values leave heuristics neutral.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ThermalState {
    #[default]
    Unknown,
    Nominal,
    Fair,
    Serious,
    Critical,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulingConditions {
    pub active_cpu_threads: Option<usize>,
    pub low_power_mode: bool,
    pub thermal_state: ThermalState,
}
#[cfg(any(target_os = "macos", test))]
fn map_thermal_state(value: isize) -> ThermalState {
    match value {
        0 => ThermalState::Nominal,
        1 => ThermalState::Fair,
        2 => ThermalState::Serious,
        3 => ThermalState::Critical,
        _ => ThermalState::Unknown,
    }
}
#[cfg(target_os = "macos")]
pub fn scheduling_conditions() -> SchedulingConditions {
    use objc2_foundation::NSProcessInfo;
    objc2::rc::autoreleasepool(|_| {
        let info = NSProcessInfo::processInfo();
        let active = info.activeProcessorCount();
        SchedulingConditions {
            active_cpu_threads: (active > 0).then_some(active),
            low_power_mode: info.isLowPowerModeEnabled(),
            thermal_state: map_thermal_state(info.thermalState().0),
        }
    })
}
#[cfg(not(target_os = "macos"))]
pub fn scheduling_conditions() -> SchedulingConditions {
    SchedulingConditions::default()
}
/// Classify scheduling scope from a held root; this grants no authority and resolves no paths.
#[cfg(target_os = "macos")]
pub fn is_volume_root(root: &disk_scan::ScanRoot) -> bool {
    use std::os::fd::{AsFd, AsRawFd};
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(root.as_fd().as_raw_fd(), info.as_mut_ptr()) } != 0 {
        return false;
    }
    let info = unsafe { info.assume_init() };
    mount_of(&info).0.as_os_str() == root.path().as_os_str()
}
#[cfg(not(target_os = "macos"))]
pub fn is_volume_root(root: &disk_scan::ScanRoot) -> bool {
    root.path() == Path::new("/")
}

pub fn target(path: &Path, name: String) -> Target {
    let (total_bytes, free_bytes) = capacity(path);
    Target {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        path: path.to_string_lossy().into_owned(),
        total_bytes,
        free_bytes,
        home: false,
        volume: false,
    }
}

/// "/" and the `/Volumes/*` mounts, from the mount table only (no statvfs, so it never blocks).
pub fn volume_paths() -> Vec<PathBuf> {
    // A home folder mounted at /Volumes/<name> is listed as home, not as a disk.
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut paths = vec![PathBuf::from("/")];
    // An unreadable mount table only costs the extra disks; "/" is always listed.
    paths.extend(
        mounted_paths()
            .unwrap_or_default()
            .into_iter()
            .filter(|path| is_volume_mount(path) && home.as_ref() != Some(path)),
    );
    paths
}

/// `volume_paths` without network mounts: the paths `discover` lists as disks, still from the mount table only.
pub fn local_volume_paths() -> Vec<PathBuf> {
    let mounts = mounted_flags().unwrap_or_default();
    volume_paths()
        .into_iter()
        .filter(|path| is_local_path(path, &mounts))
        .collect()
}

/// A disk Finder would show: directly under /Volumes and not hidden. Skips Time
/// Machine's nested and hidden snapshot mounts (e.g. /Volumes/.timemachine/...).
fn is_volume_mount(path: &Path) -> bool {
    path.parent() == Some(Path::new("/Volumes"))
        && path
            .file_name()
            .is_some_and(|name| !name.to_string_lossy().starts_with('.'))
}

/// Whether `path` is listed under Disks: the startup disk or a mounted volume, never the home folder.
pub fn is_disk_path(path: &Path) -> bool {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    home.as_deref() != Some(path) && (path == Path::new("/") || is_volume_mount(path))
}

pub fn discover() -> Vec<(Target, PathBuf)> {
    let mut paths = vec![(PathBuf::from("/"), "Macintosh HD".to_string())];
    let mut home_path = None;
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        home_path = Some(home.clone());
        // A network home sits below its mount point, so the mount-table filter below misses it.
        if ensure_local_volume(&home).is_ok() {
            paths.push((home.clone(), folder_name(&home)));
        }
    }
    for path in volume_paths().into_iter().skip(1) {
        let name = folder_name(&path);
        paths.push((path, name));
    }
    // One mount-table read for every path: statfs per path would block on an unresponsive mount.
    let mounts = mounted_flags().unwrap_or_default();
    paths
        .into_iter()
        .filter(|(path, _)| is_local_path(path, &mounts))
        .map(|(path, name)| {
            let mut item = target(&path, name);
            item.home = home_path.as_deref() == Some(path.as_path());
            item.volume = is_disk_path(&path);
            (item, path)
        })
        .collect()
}

/// The folder's own name, as the folder picker labels it.
pub(crate) fn folder_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// `statfs` of `path`: the volume it is on.
#[cfg(target_os = "macos")]
pub(crate) fn statfs_of(path: &Path) -> std::io::Result<libc::statfs> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "The path contains a NUL byte.",
        )
    })?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(c_path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { info.assume_init() })
}

/// A mount point and whether `info`'s mount is backed by a local device.
#[cfg(target_os = "macos")]
pub(crate) fn mount_of(info: &libc::statfs) -> (PathBuf, bool) {
    use std::os::unix::ffi::OsStrExt;
    let bytes: Vec<u8> = info
        .f_mntonname
        .iter()
        .map(|c| *c as u8)
        .take_while(|c| *c != 0)
        .collect();
    (
        PathBuf::from(std::ffi::OsStr::from_bytes(&bytes)),
        info.f_flags & libc::MNT_LOCAL as u32 != 0,
    )
}

/// A path is local unless the mount table lists a non-local mount at exactly that path.
fn is_local_path(path: &Path, mounts: &[(PathBuf, bool)]) -> bool {
    mounts.iter().all(|(mount, local)| mount != path || *local)
}

/// Refuses a path on a network share: `statfs` on the path itself, so it also covers folders below a mount point.
#[cfg(target_os = "macos")]
pub fn ensure_local_volume(path: &Path) -> Result<(), String> {
    let info = statfs_of(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidInput => e.to_string(),
        _ => format!("Can't read the volume: {e}"),
    })?;
    if mount_of(&info).1 {
        Ok(())
    } else {
        Err("Network shares aren't supported. Choose a folder on a local disk.".into())
    }
}
#[cfg(not(target_os = "macos"))]
pub fn ensure_local_volume(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// Whether `dir` is on an msdos or exfat volume, where the file id follows the on-disk location. False when unreadable.
#[cfg(target_os = "macos")]
pub(crate) fn is_fat_like(dir: &Path) -> bool {
    let Ok(info) = statfs_of(dir) else {
        return false;
    };
    let name = unsafe { std::ffi::CStr::from_ptr(info.f_fstypename.as_ptr()) };
    matches!(name.to_bytes(), b"msdos" | b"exfat")
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn is_fat_like(_dir: &Path) -> bool {
    false
}

/// `None` when the mount table can't be read; callers that guard mounts must not treat that as "no mounts".
pub fn mounted_paths() -> Option<Vec<PathBuf>> {
    mounted_flags().map(|mounts| mounts.into_iter().map(|(path, _)| path).collect())
}

/// Mount points and whether each is local, from the mount table only (MNT_NOWAIT: no probing of the mounts).
#[cfg(target_os = "macos")]
fn mounted_flags() -> Option<Vec<(PathBuf, bool)>> {
    // getfsstat writes into caller-owned memory. Unlike getmntinfo, concurrent
    // calls never invalidate another caller's native array.
    for _ in 0..4 {
        let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
        if count < 0 {
            return None;
        }
        let capacity = count as usize + 16;
        let mut mounts: Vec<libc::statfs> = Vec::with_capacity(capacity);
        let bytes = capacity * std::mem::size_of::<libc::statfs>();
        let written =
            unsafe { libc::getfsstat(mounts.as_mut_ptr(), bytes as i32, libc::MNT_NOWAIT) };
        if written < 0 {
            return None;
        }
        if written as usize >= capacity {
            continue;
        }
        // Only the returned records have been initialized by the kernel.
        unsafe {
            mounts.set_len(written as usize);
        }
        return Some(
            mounts
                .iter()
                .map(mount_of)
                .filter(|(path, _)| !path.as_os_str().is_empty())
                .collect(),
        );
    }
    None
}
#[cfg(not(target_os = "macos"))]
fn mounted_flags() -> Option<Vec<(PathBuf, bool)>> {
    Some(vec![])
}

#[allow(clippy::unnecessary_cast)] // statvfs field widths differ on Unix targets.
fn capacity(path: &Path) -> (Option<u64>, Option<u64>) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        if let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) {
            // statvfs writes a complete structure only on success.
            let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
            if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } == 0 {
                let info = unsafe { info.assume_init() };
                return (
                    Some((info.f_blocks as u64).saturating_mul(info.f_frsize as u64)),
                    Some((info.f_bavail as u64).saturating_mul(info.f_frsize as u64)),
                );
            }
        }
    }
    (None, None)
}

pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        run_open([
            std::ffi::OsStr::new("-R"),
            std::ffi::OsStr::new("--"),
            path.as_os_str(),
        ])
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("Finder integration is available on macOS.".into())
    }
}

pub fn full_disk_access() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        run_open([std::ffi::OsStr::new(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
        )])
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Full Disk Access is a macOS setting.".into())
    }
}

#[cfg(target_os = "macos")]
fn run_open<const N: usize>(args: [&std::ffi::OsStr; N]) -> Result<(), String> {
    let status = Command::new("/usr/bin/open")
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("macOS could not open the requested location.".into())
    }
}

/// Moves `path` to the Trash and returns where it landed (None if macOS did not say). The path is resolved again
/// here, so callers that checked it earlier must check the receipt too.
pub fn trash(path: &Path, is_directory: bool) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSFileManager, NSURL};
        objc2::rc::autoreleasepool(|_| {
            let url = NSURL::from_path(path, is_directory, None).ok_or("Invalid native path")?;
            let manager = NSFileManager::defaultManager();
            let mut resulting_url = None;
            manager
                .trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting_url))
                .map_err(|e| e.localizedDescription().to_string())?;
            Ok(resulting_url.and_then(|url| url.to_file_path()))
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, is_directory);
        Err("Trash is not implemented for this platform.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_mounts_exclude_the_volumes_folder_and_other_mounts() {
        assert!(is_volume_mount(Path::new("/Volumes/Backup")));
        assert!(!is_volume_mount(Path::new("/Volumes")));
        assert!(!is_volume_mount(Path::new("/System/Volumes/Data")));
        assert!(!is_volume_mount(Path::new(
            "/Volumes/.timemachine/ABC/2026-11-08.backup"
        )));
        assert!(!is_volume_mount(Path::new(
            "/Volumes/com.apple.TimeMachine.localsnapshots/Backups.backupdb/Mac"
        )));
        assert!(!is_volume_mount(Path::new("/")));
        assert!(is_disk_path(Path::new("/")));
        assert!(is_disk_path(Path::new("/Volumes/Backup")));
        assert!(!is_disk_path(Path::new("/Volumes")));
        assert!(!is_disk_path(Path::new("/Users/ada/Photos")));
        assert!(!is_disk_path(Path::new("/Volumes/.timemachine/x")));
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert!(!is_disk_path(&home));
    }
    #[test]
    fn mount_flags_tell_local_disks_from_network_shares_that_discovery_drops() {
        let mounts = [
            (PathBuf::from("/"), true),
            (PathBuf::from("/Volumes/Backup"), true),
            (PathBuf::from("/Volumes/media"), false),
        ];
        assert!(is_local_path(Path::new("/"), &mounts));
        assert!(is_local_path(Path::new("/Volumes/Backup"), &mounts));
        assert!(!is_local_path(Path::new("/Volumes/media"), &mounts));
        // Not a mount point (a home folder), or missing from the table: local.
        assert!(is_local_path(Path::new("/Users/ada"), &mounts));
        assert!(is_local_path(Path::new("/Volumes/New"), &[]));
    }
    #[test]
    fn folders_on_the_local_disk_are_accepted() {
        assert!(ensure_local_volume(&std::env::temp_dir()).is_ok());
    }
    #[test]
    fn discover_flags_the_startup_disk_but_not_home() {
        let items = discover();
        assert!(items.iter().any(|(t, p)| t.volume && p == Path::new("/")));
        assert!(items.iter().filter(|(t, _)| t.home).all(|(t, _)| !t.volume));
        assert!(volume_paths().first().is_some_and(|p| p == Path::new("/")));
    }
    #[test]
    fn home_target_is_named_after_its_folder_and_flagged() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        let (item, path) = discover()
            .into_iter()
            .find(|(t, _)| t.home)
            .expect("home target");
        assert_eq!(path, home);
        assert_eq!(item.name, folder_name(&home));
    }
    #[test]
    fn folder_name_uses_the_last_component() {
        assert_eq!(folder_name(Path::new("/Users/ada")), "ada");
        assert_eq!(folder_name(Path::new("/")), "/");
    }
    #[test]
    fn scheduling_conditions_have_neutral_defaults_and_checked_thermal_mapping() {
        assert_eq!(
            SchedulingConditions::default(),
            SchedulingConditions {
                active_cpu_threads: None,
                low_power_mode: false,
                thermal_state: ThermalState::Unknown
            }
        );
        for (value, expected) in [
            (0, ThermalState::Nominal),
            (1, ThermalState::Fair),
            (2, ThermalState::Serious),
            (3, ThermalState::Critical),
            (-1, ThermalState::Unknown),
            (4, ThermalState::Unknown),
        ] {
            assert_eq!(map_thermal_state(value), expected);
        }
        assert_eq!(
            serde_json::to_value(ThermalState::Serious).unwrap(),
            "serious"
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn scheduling_conditions_read_native_getters_without_mutating_policy() {
        let conditions = scheduling_conditions();
        assert!(conditions.active_cpu_threads.is_some_and(|count| count > 0));
    }
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn unsupported_scheduling_conditions_are_neutral() {
        assert_eq!(scheduling_conditions(), SchedulingConditions::default());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn volume_classification_uses_held_root_not_replacement_path() {
        let path = PathBuf::from("/");
        let root =
            disk_scan::ScanRoot::open(path.clone(), disk_scan::platform_identity(&path).unwrap())
                .unwrap();
        assert!(is_volume_root(&root));
        let fixture = tempfile::tempdir().unwrap();
        let original = std::fs::canonicalize(fixture.path()).unwrap();
        let held = disk_scan::ScanRoot::open(
            original.clone(),
            disk_scan::platform_identity(&original).unwrap(),
        )
        .unwrap();
        assert!(!is_volume_root(&held));
        let renamed = original.with_extension("held-root");
        std::fs::rename(&original, &renamed).unwrap();
        std::fs::create_dir(&original).unwrap();
        assert!(!is_volume_root(&held));
        std::fs::remove_dir_all(renamed).unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn concurrent_mount_discovery_owns_buffers() {
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..20 {
                        assert!(mounted_paths().unwrap().iter().any(|p| p == Path::new("/")));
                    }
                });
            }
        });
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Moves only a disposable fixture to native Trash and restores it; run explicitly on macOS"]
    fn native_trash_and_recovery() {
        let d = tempfile::tempdir().unwrap();
        let original = d
            .path()
            .join(format!("hahakigami-recovery-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&original, b"disposable recovery fixture").unwrap();
        let recycled = trash(&original, false)
            .expect("native Trash move")
            .expect("native Trash receipt");
        assert!(!original.exists());
        std::fs::rename(&recycled, &original).expect("restore the disposable fixture");
        assert_eq!(
            std::fs::read(&original).unwrap(),
            b"disposable recovery fixture"
        );
    }
}

/// The UUID of the filesystem holding `path` (not following a final symlink): Ok(None) when that filesystem has none, Err when it cannot be read.
/// One getattrlist call on the object itself: never a statfs of other paths.
#[cfg(target_os = "macos")]
pub fn volume_uuid(path: &Path) -> std::io::Result<Option<String>> {
    let mut attributes = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: 0,
        volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let attributes = (&mut attributes as *mut libc::attrlist).cast();
    let mut buffer = [0u8; 20];
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(std::io::Error::other)?;
    let result = unsafe {
        libc::getattrlist(
            path.as_ptr(),
            attributes,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc::FSOPT_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // A filesystem without the attribute returns only the 4-byte length.
    match u32::from_ne_bytes(buffer[..4].try_into().unwrap()) {
        4 => Ok(None),
        20 => Ok(parse_volume_uuid(&buffer)),
        _ => Err(std::io::Error::other("Malformed volume attributes")),
    }
}
#[cfg(not(target_os = "macos"))]
pub fn volume_uuid(_path: &Path) -> std::io::Result<Option<String>> {
    Ok(None)
}
#[cfg(any(target_os = "macos", test))]
fn parse_volume_uuid(buffer: &[u8]) -> Option<String> {
    if buffer.len() != 20 || u32::from_ne_bytes(buffer.get(..4)?.try_into().ok()?) != 20 {
        return None;
    }
    let bytes: [u8; 16] = buffer.get(4..20)?.try_into().ok()?;
    let id = uuid::Uuid::from_bytes(bytes);
    (!id.is_nil()).then(|| id.to_string())
}
#[cfg(test)]
mod volume_tests {
    use super::*;
    #[test]
    fn checked_uuid_records_reject_bad_lengths_and_nil() {
        let mut bytes = [0u8; 20];
        bytes[..4].copy_from_slice(&20u32.to_ne_bytes());
        assert!(parse_volume_uuid(&bytes).is_none());
        bytes[4] = 1;
        assert!(parse_volume_uuid(&bytes).is_some());
        for end in 0..20 {
            assert!(parse_volume_uuid(&bytes[..end]).is_none());
        }
        bytes[..4].copy_from_slice(&24u32.to_ne_bytes());
        assert!(parse_volume_uuid(&bytes).is_none());
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn volume_uuid_reads_host_filesystem_and_rejects_missing_paths() {
        let path = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(volume_uuid(&path).unwrap().is_some());
        assert!(volume_uuid(&path.join("missing-hahakigami")).is_err());
    }
}
