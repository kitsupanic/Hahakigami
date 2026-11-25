//! Full Disk Access status, found with silent probes (ADR 0002:
//! `docs/adr/0002-detect-full-disk-access-with-silent-probes.md`).
//!
//! macOS has no public API for this. On macOS we open a few files that only Full
//! Disk Access can read, read-only, never read a byte, and close them at once.
//! The user and system `TCC.db` files are authoritative: only a refusal there
//! reports off. Other gated files can only confirm on. Anything else is unknown,
//! and unknown never blocks a scan.
//!
//! Hard rules: never probe a per-folder protected location (Documents, Desktop,
//! Downloads, removable volumes), because that shows the consent prompt this avoids.
//! Never read TCC contents, use private APIs, or change grants.
//!
//! The probe set is undocumented behaviour that changes between macOS releases,
//! so it is the small `PROBES` table below. Change the table, not the code.
use serde::Serialize;
use std::path::Path;

pub const OFF: &str = "FULL_DISK_ACCESS_OFF";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    On,
    Off,
    Unknown,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupInfo {
    pub status: Status,
    pub scan_anyway: bool,
    pub app_path: String,
}

#[derive(Default)]
pub struct AccessSetup {
    app_path: String,
    // Per process, never persisted: the user chose to scan despite status off.
    scan_anyway: bool,
    #[cfg(test)]
    forced: Option<Status>,
}
impl AccessSetup {
    pub fn new(executable: &Path) -> Self {
        let app = executable
            .ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .unwrap_or(executable);
        Self {
            app_path: app.to_string_lossy().into_owned(),
            ..Self::default()
        }
    }
    #[cfg(test)]
    pub fn forcing(status: Status) -> Self {
        Self {
            forced: Some(status),
            ..Self::default()
        }
    }
    fn status(&self) -> Status {
        #[cfg(test)]
        if let Some(status) = self.forced {
            return status;
        }
        detect()
    }
    /// Probes again on every call; a few `open(2)` calls are cheap.
    pub fn info(&self) -> SetupInfo {
        SetupInfo {
            status: self.status(),
            scan_anyway: self.scan_anyway,
            app_path: self.app_path.clone(),
        }
    }
    pub fn allow_scan_anyway(&mut self) -> SetupInfo {
        self.scan_anyway = true;
        self.info()
    }
    pub fn require_scan_allowed(&self) -> Result<(), String> {
        if self.scan_anyway || self.status() != Status::Off {
            Ok(())
        } else {
            Err(OFF.into())
        }
    }
}

#[cfg(target_os = "macos")]
fn detect() -> Status {
    probe::status()
}
#[cfg(not(target_os = "macos"))]
fn detect() -> Status {
    Status::On
}

#[cfg(target_os = "macos")]
mod probe {
    use super::Status;
    use std::{
        fs::File,
        path::{Path, PathBuf},
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Outcome {
        Granted,
        Denied,
        Unavailable,
        Inconclusive,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Authority {
        Authoritative,
        PositiveOnly,
    }

    /// Any grant means on. Only a refusal from an authoritative file means off.
    pub fn verdict(results: impl IntoIterator<Item = (Authority, Outcome)>) -> Status {
        let mut status = Status::Unknown;
        for (authority, outcome) in results {
            match (authority, outcome) {
                (_, Outcome::Granted) => return Status::On,
                (Authority::Authoritative, Outcome::Denied) => status = Status::Off,
                _ => {}
            }
        }
        status
    }

    struct Probe {
        /// A leading `~/` is the user's home folder.
        path: &'static str,
        authority: Authority,
        /// Not used from this macOS major version on.
        until_macos: Option<u32>,
    }
    // Edit this table when a macOS release changes what only Full Disk Access can open.
    // Never add Documents, Desktop, Downloads or removable volumes: opening those prompts.
    const PROBES: &[Probe] = &[
        Probe {
            path: "~/Library/Application Support/com.apple.TCC/TCC.db",
            authority: Authority::Authoritative,
            until_macos: Some(27),
        },
        Probe {
            path: "/Library/Application Support/com.apple.TCC/TCC.db",
            authority: Authority::Authoritative,
            until_macos: None,
        },
        Probe {
            path: "~/Library/Safari/Bookmarks.plist",
            authority: Authority::PositiveOnly,
            until_macos: None,
        },
        Probe {
            path: "~/Library/Messages/chat.db",
            authority: Authority::PositiveOnly,
            until_macos: None,
        },
        Probe {
            path: "/Library/Preferences/com.apple.TimeMachine.plist",
            authority: Authority::PositiveOnly,
            until_macos: None,
        },
    ];

    fn macos_major() -> Option<u32> {
        let mut buffer = [0u8; 32];
        let mut length = buffer.len();
        // SAFETY: the name is NUL-terminated and `buffer` is writable for `length` bytes.
        let failed = unsafe {
            libc::sysctlbyname(
                c"kern.osproductversion".as_ptr(),
                buffer.as_mut_ptr().cast(),
                &mut length,
                std::ptr::null_mut(),
                0,
            )
        } != 0;
        if failed {
            return None;
        }
        let version = std::str::from_utf8(&buffer[..length]).ok()?;
        version
            .trim_end_matches('\0')
            .split('.')
            .next()?
            .parse()
            .ok()
    }

    /// Only a refusal means denied: a file that is not there, or any other failure, says nothing about access.
    fn failure(errno: Option<i32>) -> Outcome {
        match errno {
            Some(libc::EPERM | libc::EACCES) => Outcome::Denied,
            Some(libc::ENOENT | libc::ENOTDIR) => Outcome::Unavailable,
            _ => Outcome::Inconclusive,
        }
    }
    /// Opens and closes the file at once; no byte is read.
    fn outcome(path: &Path) -> Outcome {
        match File::open(path) {
            Ok(_) => Outcome::Granted,
            Err(error) => failure(error.raw_os_error()),
        }
    }
    /// Whether `probe` is used on macOS `major`. Without a known version, a version-limited probe could wrongly
    /// report off, so it is skipped.
    fn applies(probe: &Probe, major: Option<u32>) -> bool {
        probe
            .until_macos
            .is_none_or(|until| major.is_some_and(|major| major < until))
    }

    /// The path to open for a probe: a `~/` path is built from `home`, and skipped without one.
    fn probe_path(path: &str, home: Option<&Path>) -> Option<PathBuf> {
        match path.strip_prefix("~/") {
            Some(relative) => home.map(|home| home.join(relative)),
            None => Some(PathBuf::from(path)),
        }
    }

    pub fn status() -> Status {
        let major = macos_major();
        // The real home from the user database: in the Lite sandbox `$HOME` is the app's container.
        let home = crate::platform::home_dir();
        verdict(PROBES.iter().filter_map(|probe| {
            if !applies(probe, major) {
                return None;
            }
            let path = probe_path(probe.path, home.as_deref())?;
            Some((probe.authority, outcome(&path)))
        }))
    }

    #[cfg(test)]
    mod tests {
        use super::{Authority::*, Outcome::*, *};

        #[test]
        fn verdict_follows_the_probe_rules() {
            let cases: &[(&[(Authority, Outcome)], Status)] = &[
                (&[], Status::Unknown),
                (&[(Authoritative, Granted)], Status::On),
                (&[(PositiveOnly, Granted)], Status::On),
                (&[(Authoritative, Denied)], Status::Off),
                (&[(PositiveOnly, Denied)], Status::Unknown),
                (&[(Authoritative, Unavailable)], Status::Unknown),
                (&[(PositiveOnly, Unavailable)], Status::Unknown),
                (&[(Authoritative, Inconclusive)], Status::Unknown),
                (&[(PositiveOnly, Inconclusive)], Status::Unknown),
                // Any grant wins over a refusal elsewhere.
                (
                    &[(Authoritative, Denied), (PositiveOnly, Granted)],
                    Status::On,
                ),
                (
                    &[(Authoritative, Denied), (Authoritative, Granted)],
                    Status::On,
                ),
                // Only an authoritative refusal reports off, and other probes cannot undo it.
                (
                    &[
                        (Authoritative, Denied),
                        (Authoritative, Unavailable),
                        (PositiveOnly, Inconclusive),
                    ],
                    Status::Off,
                ),
                (
                    &[
                        (Authoritative, Unavailable),
                        (PositiveOnly, Denied),
                        (PositiveOnly, Inconclusive),
                    ],
                    Status::Unknown,
                ),
            ];
            for (results, expected) in cases {
                assert_eq!(verdict(results.iter().copied()), *expected, "{results:?}");
            }
        }

        fn probe(until_macos: Option<u32>) -> Probe {
            Probe {
                path: "/x",
                authority: Authoritative,
                until_macos,
            }
        }

        #[test]
        fn a_version_limited_probe_applies_only_below_its_limit_on_a_known_version() {
            let limited = probe(Some(27));
            assert!(applies(&limited, Some(26)));
            assert!(applies(&limited, Some(15)));
            // At and past the limit, and on a version nobody could read, it could report off wrongly.
            assert!(!applies(&limited, Some(27)));
            assert!(!applies(&limited, Some(28)));
            assert!(!applies(&limited, None));
            // A probe with no limit always applies.
            for major in [None, Some(14), Some(27), Some(99)] {
                assert!(applies(&probe(None), major), "{major:?}");
            }
        }

        #[test]
        fn only_a_refusal_is_denied_and_a_missing_file_never_is() {
            for errno in [libc::EPERM, libc::EACCES] {
                assert_eq!(failure(Some(errno)), Denied);
            }
            for errno in [libc::ENOENT, libc::ENOTDIR] {
                assert_eq!(failure(Some(errno)), Unavailable);
            }
            for errno in [Some(libc::EIO), Some(libc::EINTR), Some(libc::EMFILE), None] {
                assert_eq!(failure(errno), Inconclusive, "{errno:?}");
            }
            // The real thing, on a path that does not exist.
            let missing = Path::new("/definitely/not/here/TCC.db");
            assert_eq!(outcome(missing), Unavailable);
        }

        #[test]
        fn the_probe_table_never_touches_a_folder_that_prompts() {
            const PROMPTING: [&str; 3] = ["Documents", "Desktop", "Downloads"];
            assert!(!PROBES.is_empty());
            for probe in PROBES {
                let path = Path::new(probe.path);
                assert!(!path.starts_with("/Volumes"), "{}", probe.path);
                for component in path.components() {
                    let name = component.as_os_str().to_string_lossy();
                    assert!(!PROMPTING.contains(&name.as_ref()), "{}", probe.path);
                }
            }
        }

        #[test]
        fn home_probes_are_built_from_the_given_home_and_others_stay_absolute() {
            let home = Path::new("/Users/real");
            assert_eq!(
                probe_path("~/Library/Safari/Bookmarks.plist", Some(home)),
                Some(PathBuf::from("/Users/real/Library/Safari/Bookmarks.plist"))
            );
            assert_eq!(
                probe_path(
                    "/Library/Application Support/com.apple.TCC/TCC.db",
                    Some(home)
                ),
                Some(PathBuf::from(
                    "/Library/Application Support/com.apple.TCC/TCC.db"
                ))
            );
            // Without a home, a home probe is skipped; an absolute one still runs.
            assert_eq!(probe_path("~/Library/Messages/chat.db", None), None);
            assert!(probe_path("/Library/Preferences/x.plist", None).is_some());
        }

        /// Run with `cargo test -- --ignored`. The answer depends on this Mac's grants.
        #[test]
        #[ignore = "probes the machine running the test"]
        fn real_probe_reaches_a_verdict_on_this_mac() {
            let status = status();
            println!("Full Disk Access status on this Mac: {status:?}");
            assert_ne!(status, Status::Unknown);
        }
    }
}
