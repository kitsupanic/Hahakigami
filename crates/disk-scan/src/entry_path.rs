//! Temporary listing paths: short native basenames need no heap allocation.
use std::{
    ffi::OsStr,
    ops::Deref,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub(crate) struct EntryPath(Storage);

#[derive(Clone, Debug)]
enum Storage {
    #[cfg(unix)]
    Short {
        bytes: [u8; 22],
        length: u8,
    },
    Owned(Box<Path>),
}

impl EntryPath {
    pub(crate) fn native_name_or_path(&self) -> &OsStr {
        match &self.0 {
            #[cfg(unix)]
            Storage::Short { bytes, length } => {
                use std::os::unix::ffi::OsStrExt;
                // The checked short constructor only stores normal basenames.
                OsStr::from_bytes(&bytes[..usize::from(*length)])
            }
            Storage::Owned(path) => path.file_name().unwrap_or(path.as_os_str()),
        }
    }

    pub(crate) fn from_native(name: &OsStr) -> Self {
        #[cfg(unix)]
        if let Some(short) = Self::short(name) {
            return short;
        }
        Self(Storage::Owned(PathBuf::from(name).into_boxed_path()))
    }

    #[cfg(unix)]
    fn short(name: &OsStr) -> Option<Self> {
        use std::os::unix::ffi::OsStrExt;
        let name = name.as_bytes();
        if name.is_empty()
            || name.len() > 22
            || name == b"."
            || name == b".."
            || name.iter().any(|byte| matches!(byte, 0 | b'/'))
        {
            return None;
        }
        let mut bytes = [0; 22];
        bytes[..name.len()].copy_from_slice(name);
        Some(Self(Storage::Short {
            bytes,
            length: name.len() as u8,
        }))
    }
}

impl Deref for EntryPath {
    type Target = Path;
    fn deref(&self) -> &Path {
        match &self.0 {
            #[cfg(unix)]
            Storage::Short { bytes, length } => {
                use std::os::unix::ffi::OsStrExt;
                Path::new(OsStr::from_bytes(&bytes[..usize::from(*length)]))
            }
            Storage::Owned(path) => path,
        }
    }
}
impl AsRef<Path> for EntryPath {
    fn as_ref(&self) -> &Path {
        self
    }
}
impl From<PathBuf> for EntryPath {
    fn from(path: PathBuf) -> Self {
        #[cfg(unix)]
        if let Some(short) = Self::short(path.as_os_str()) {
            return short;
        }
        Self(Storage::Owned(path.into_boxed_path()))
    }
}
impl From<&str> for EntryPath {
    fn from(path: &str) -> Self {
        Self::from_native(OsStr::new(path))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn native_bytes_and_path_semantics_survive_short_and_owned_storage() {
        let long_name = vec![0xff; 255];
        let cases: &[(&[u8], bool)] = &[
            (b"a", true),
            (b"123456789012345678901", true),
            (b"1234567890123456789012", true),
            (b"12345678901234567890123", false),
            ("日本語".as_bytes(), true),
            (b"native-\xff\nname", true),
            (&long_name, false),
            (b"", false),
            (b".", false),
            (b"..", false),
            (b"relative/path", false),
            (b"./child", false),
            (b"../child", false),
            (b"/absolute/../path/", false),
            (b"/", false),
            (b"a/", false),
            (b"a\0b", false),
        ];
        for &(bytes, should_inline) in cases {
            let native = OsStr::from_bytes(bytes);
            let expected = Path::new(native);
            for actual in [
                EntryPath::from_native(native),
                EntryPath::from(expected.to_path_buf()),
            ] {
                assert_eq!(actual.as_os_str().as_bytes(), bytes);
                assert_eq!(actual.file_name(), expected.file_name());
                assert_eq!(
                    actual.native_name_or_path(),
                    expected.file_name().unwrap_or(expected.as_os_str())
                );
                assert_eq!(actual.parent(), expected.parent());
                assert_eq!(
                    Path::new("/parent").join(&actual),
                    Path::new("/parent").join(expected)
                );
                assert_eq!(matches!(actual.0, Storage::Short { .. }), should_inline);
                assert_eq!(actual.clone().as_os_str().as_bytes(), bytes);
            }
        }
    }
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn short_names_keep_the_existing_entry_layout() {
        assert_eq!(std::mem::size_of::<EntryPath>(), 24);
        assert_eq!(std::mem::size_of::<std::io::Result<crate::Entry>>(), 64);
        assert_eq!(std::mem::size_of::<crate::ScannedNode>(), 96);
    }
}
