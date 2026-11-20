//! Presentation classification only: no filesystem queries, coverage changes, or omitted errors.
use crate::ScanIssue;
use std::{io, path::Path};
pub fn is_system_restriction(issue: &ScanIssue) -> bool {
    metadata(
        &issue.path,
        if issue.error_kind.as_deref() == Some("PermissionDenied") {
            io::ErrorKind::PermissionDenied
        } else {
            io::ErrorKind::Other
        },
        issue.raw_os_error,
    )
}
pub(crate) fn metadata(path: &Path, kind: io::ErrorKind, code: Option<i32>) -> bool {
    #[cfg(target_os = "macos")]
    {
        kind == io::ErrorKind::PermissionDenied
            && matches!(code, Some(libc::EPERM) | Some(libc::EACCES))
            && system_path(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, kind, code);
        false
    }
}
#[cfg(any(target_os = "macos", test))]
fn system_path(path: &Path) -> bool {
    #[cfg(unix)]
    let raw = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes()
    };
    #[cfg(not(unix))]
    let text = path.to_string_lossy();
    #[cfg(not(unix))]
    let raw = text.as_bytes();
    if !raw.starts_with(b"/") || raw.len() < 2 {
        return false;
    }
    let parts: Vec<&[u8]> = raw[1..].split(|b| *b == b'/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == b"." || *p == b".." || p.contains(&0))
    {
        return false;
    }
    let parts = if parts.starts_with(&[b"System".as_slice(), b"Volumes", b"Data"]) {
        &parts[3..]
    } else {
        parts.as_slice()
    };
    let prefixes: &[&[&[u8]]] = &[
        &[b"System", b"Library"],
        &[b"System", b"Applications"],
        &[b"Library", b"Caches"],
        &[b"Library", b"Application Support", b"Apple"],
        &[b"private", b"etc", b"cups", b"certs"],
        &[b"usr", b"bin"],
        &[b"usr", b"sbin"],
        &[b"usr", b"lib"],
        &[b"usr", b"libexec"],
        &[b"usr", b"share"],
        &[b"bin"],
        &[b"sbin"],
    ];
    // These are volume-level names: also match below an external volume's mount point.
    let volume_parts = match parts {
        [b"Volumes", _, rest @ ..] => rest,
        _ => parts,
    };
    if volume_parts.first().is_some_and(|name| {
        [
            b".DocumentRevisions-V100".as_slice(),
            b".fseventsd",
            b".Spotlight-V100",
            b".Trashes",
        ]
        .contains(name)
    }) {
        return true;
    }
    if prefixes.iter().any(|prefix| parts.starts_with(prefix)) {
        return true;
    }
    if parts.starts_with(&[b"private".as_slice(), b"var"]) {
        let rest = &parts[2..];
        if rest.first().is_some_and(|name| {
            [
                b"db".as_slice(),
                b"protected",
                b"agentx",
                b"backups",
                b"dirs_cleaner",
                b"run",
                b"root",
                b"audit",
                b"OOPJit",
                b"jabberd",
                b"spool",
                b"vm",
                b"networkd",
                b"install",
                b"ma",
                b"at",
            ]
            .contains(name)
        }) {
            return true;
        }
        if rest.starts_with(&[b"lib".as_slice(), b"postfix"]) {
            return true;
        }
        if rest.first() == Some(&b"folders".as_slice()) {
            let bucket = &rest[1..];
            let valid_hash = |bytes: &[u8]| {
                bytes.len() == 30
                    && bytes
                        .iter()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
            };
            if bucket.first() == Some(&b"zz".as_slice()) {
                return bucket.len() == 1 || bucket.get(1).is_some_and(|hash| valid_hash(hash));
            }
            let valid_bucket = bucket.first().is_some_and(|name| {
                name.len() == 2
                    && name
                        .iter()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            });
            if valid_bucket
                && bucket.get(1).is_some_and(|hash| valid_hash(hash))
                && bucket
                    .get(2)
                    .is_some_and(|area| *area == b"C" || *area == b"0")
            {
                if bucket[2] == b"0" && bucket.get(3) == Some(&b"dmd".as_slice()) {
                    return true;
                }
                return bucket.get(3).is_some_and(|name| {
                    name.starts_with(b"com.apple.")
                        && name.len() > 10
                        && name
                            .iter()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(b))
                });
            }
        }
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conservative_path_patterns_cover_known_os_areas_and_data_aliases() {
        for path in [
            "/System/Library/Assets",
            "/System/Applications/Stock.app",
            "/Library/Caches/com.apple.service",
            "/Library/Application Support/Apple/asset",
            "/private/var/db/store",
            "/private/var/protected/store",
            "/private/var/agentx",
            "/private/var/root",
            "/private/etc/cups/certs/0",
            "/usr/bin/tool",
            "/usr/sbin/tool",
            "/usr/libexec/tool",
            "/usr/share/locale",
            "/sbin/tool",
        ] {
            assert!(system_path(Path::new(path)), "{path}");
            assert!(system_path(Path::new(&format!(
                "/System/Volumes/Data{path}"
            ))));
        }
    }
    #[test]
    fn user_paths_lookalikes_and_ambiguous_paths_stay_unclassified() {
        for path in [
            "/Users/person/Library/Caches",
            "/Applications/Personal.app",
            "/System/Volumes/Data/Users/person/Library",
            "/System/Volumes/Data/Applications",
            "/Library/Application Support/ApplePersonal",
            "/usr/local/bin",
            "/usr/binary",
            "/private/var/db-personal",
            "/System/Library/../Users",
            "/System//Library",
            "/System/Library/./thing",
            "System/Library",
            "/System/Library/",
        ] {
            assert!(!system_path(Path::new(path)), "{path}");
        }
    }
    #[test]
    fn folder_buckets_require_daemon_or_direct_apple_owned_area() {
        let hash = "000000000000000000000000000000";
        for path in [
            format!("/private/var/folders/zz/{hash}/T/service"),
            format!("/private/var/folders/ab/{hash}/C/com.apple.service"),
            format!("/private/var/folders/ab/{hash}/0/com.apple.agent/state"),
        ] {
            assert!(system_path(Path::new(&path)), "{path}");
        }
        for path in [
            format!("/private/var/folders/ab/{hash}/T/com.apple.service"),
            format!("/private/var/folders/ab/{hash}/C/projects/com.apple.service"),
            format!("/private/var/folders/ab/{hash}/C/com.applePersonal"),
            format!("/private/var/folders/ab/{hash}/0/projects"),
            "/private/var/folders/ab/invalid/C/com.apple.service".into(),
            "/private/var/folders/zz/userproject".into(),
        ] {
            assert!(!system_path(Path::new(&path)), "{path}");
        }
    }
    #[test]
    fn native_permission_kind_and_errno_are_both_required() {
        for (kind, code) in [
            (io::ErrorKind::NotFound, Some(libc::EACCES)),
            (io::ErrorKind::Other, Some(libc::EPERM)),
            (io::ErrorKind::PermissionDenied, Some(libc::EIO)),
            (io::ErrorKind::PermissionDenied, None),
        ] {
            assert!(!metadata(Path::new("/System/Library"), kind, code));
        }
        #[cfg(target_os = "macos")]
        assert!(metadata(
            Path::new("/System/Library"),
            io::ErrorKind::PermissionDenied,
            Some(libc::EPERM)
        ));
        #[cfg(not(target_os = "macos"))]
        assert!(!metadata(
            Path::new("/System/Library"),
            io::ErrorKind::PermissionDenied,
            Some(libc::EPERM)
        ));
        let issue = ScanIssue {
            path: "/Users/person/project".into(),
            message: "Operation not permitted".into(),
            error_kind: Some("PermissionDenied".into()),
            raw_os_error: Some(libc::EPERM),
        };
        assert!(!is_system_restriction(&issue));
    }
    #[test]
    fn startup_metadata_names_match_only_root_components() {
        for name in [
            ".DocumentRevisions-V100",
            ".fseventsd",
            ".Spotlight-V100",
            ".Trashes",
        ] {
            assert!(system_path(Path::new(&format!("/{name}/database"))));
            assert!(system_path(Path::new(&format!(
                "/System/Volumes/Data/{name}/database"
            ))));
            assert!(!system_path(Path::new(&format!("/{name}-personal"))));
            assert!(!system_path(Path::new(&format!("/Users/person/{name}"))));
        }
    }
    #[test]
    fn volume_metadata_names_match_below_a_volume_mount_point_only() {
        for name in [
            ".DocumentRevisions-V100",
            ".fseventsd",
            ".Spotlight-V100",
            ".Trashes",
        ] {
            for volume in [
                "/Volumes/USB",
                "/Volumes/My Disk",
                "/System/Volumes/Data/Volumes/USB",
            ] {
                assert!(
                    system_path(Path::new(&format!("{volume}/{name}"))),
                    "{volume}/{name}"
                );
                assert!(system_path(Path::new(&format!("{volume}/{name}/db"))));
                assert!(!system_path(Path::new(&format!(
                    "{volume}/{name}-personal"
                ))));
                assert!(!system_path(Path::new(&format!(
                    "{volume}/Users/person/{name}"
                ))));
            }
            // The volume itself, or a volume named like one, is not metadata.
            assert!(!system_path(Path::new(&format!("/Volumes/{name}"))));
            assert!(!system_path(Path::new(&format!("/Volumes/{name}/data"))));
        }
        // Only the volume-level names move below a mount point, not the system prefixes.
        assert!(!system_path(Path::new("/Volumes/USB/usr/bin/tool")));
        assert!(!system_path(Path::new("/Volumes/USB/System/Library")));
    }
    #[test]
    fn specific_scheduler_postfix_and_management_daemon_paths_are_bounded() {
        let hash = "000000000000000000000000000000";
        for path in [
            "/private/var/at/tmp".to_owned(),
            "/private/var/at/tabs".into(),
            "/private/var/lib/postfix/state".into(),
            format!("/private/var/folders/ab/{hash}/0/dmd/state"),
        ] {
            assert!(system_path(Path::new(&path)), "{path}");
            assert!(system_path(Path::new(&format!(
                "/System/Volumes/Data{path}"
            ))));
        }
        for path in [
            "/private/var/atlas/project".to_owned(),
            "/private/var/lib/personal".into(),
            "/private/var/lib/postfix-personal".into(),
            format!("/private/var/folders/ab/{hash}/T/dmd"),
            format!("/private/var/folders/ab/{hash}/C/dmd"),
            format!("/private/var/folders/ab/{hash}/0/dmd-personal"),
            format!("/private/var/folders/ab/{hash}/0/projects/dmd"),
            "/private/var/folders/ab/bad/0/dmd".into(),
        ] {
            assert!(!system_path(Path::new(&path)), "{path}");
        }
    }
}
