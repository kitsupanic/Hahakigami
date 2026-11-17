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

    /// Opens and closes the file at once; no byte is read.
    fn outcome(path: &Path) -> Outcome {
        match File::open(path) {
            Ok(_) => Outcome::Granted,
            Err(error) => match error.raw_os_error() {
                Some(libc::EPERM | libc::EACCES) => Outcome::Denied,
                Some(libc::ENOENT | libc::ENOTDIR) => Outcome::Unavailable,
                _ => Outcome::Inconclusive,
            },
        }
    }

    pub fn status() -> Status {
        let major = macos_major();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        verdict(PROBES.iter().filter_map(|probe| {
            // Without a known version, a version-limited probe could wrongly report off.
            if probe
                .until_macos
                .is_some_and(|until| major.is_none_or(|major| major >= until))
            {
                return None;
            }
            let path = match probe.path.strip_prefix("~/") {
                Some(relative) => home.as_ref()?.join(relative),
                None => PathBuf::from(probe.path),
            };
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
