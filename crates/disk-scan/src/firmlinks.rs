//! macOS startup Data traversal excludes only verified public firmlink aliases.
use crate::{platform_identity, Identity};
use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
};
#[derive(Default)]
pub(crate) struct Policy {
    aliases: HashMap<PathBuf, Identity>,
    alias_parents: HashSet<PathBuf>,
}
fn mappings(text: &str, data_root: &Path) -> Vec<(PathBuf, PathBuf)> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let public = PathBuf::from(words.next()?);
            let relative = PathBuf::from(words.next()?);
            if words.next().is_some()
                || !public.is_absolute()
                || relative.is_absolute()
                || relative
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                return None;
            }
            Some((data_root.join(relative), public))
        })
        .collect()
}
/// The public paths of the startup disk's firmlinks (`/Users`, `/usr/local`, ...). Each one is a seam where a
/// path on the System volume continues on the Data volume while `st_dev` stays the same, so a device number alone
/// cannot tell the two volumes apart. Empty where the list is missing (other platforms).
pub fn public_paths() -> Vec<PathBuf> {
    std::fs::read_to_string("/usr/share/firmlinks")
        .map(|text| {
            mappings(&text, Path::new("/System/Volumes/Data"))
                .into_iter()
                .map(|(_, public)| public)
                .collect()
        })
        .unwrap_or_default()
}
impl Policy {
    pub(crate) fn load(root: &Path) -> Self {
        if root != Path::new("/") {
            return Self::default();
        }
        let Ok(text) = std::fs::read_to_string("/usr/share/firmlinks") else {
            return Self::default();
        };
        Self::from_mappings(mappings(&text, Path::new("/System/Volumes/Data")))
    }
    fn from_mappings(mappings: Vec<(PathBuf, PathBuf)>) -> Self {
        let aliases: HashMap<PathBuf, Identity> = mappings
            .into_iter()
            .filter_map(|(data, public)| {
                let identity = platform_identity(&data).ok()?;
                if platform_identity(&public).ok()? == identity {
                    Some((data, identity))
                } else {
                    None
                }
            })
            .collect();
        let alias_parents = aliases
            .keys()
            .filter_map(|path| path.parent().map(Path::to_path_buf))
            .collect();
        Self {
            aliases,
            alias_parents,
        }
    }
    pub(crate) fn may_have_alias_children(&self, path: &Path) -> bool {
        // A listing needs full child paths only when it can contain an exact alias.
        self.alias_parents.contains(path)
    }
    pub(crate) fn is_mirror(&self, path: &Path, identity: &Identity) -> bool {
        self.aliases.get(path) == Some(identity)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_verified_aliases_are_mirrors() {
        let root = std::env::temp_dir().join(format!("hahakigami-firmlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Data/System/Library")).unwrap();
        std::fs::create_dir_all(root.join("Public")).unwrap();
        std::fs::write(root.join("Data/System/Library/item"), b"x").unwrap();
        std::fs::hard_link(
            root.join("Data/System/Library/item"),
            root.join("Public/item"),
        )
        .unwrap();
        let text = "/Public/item\tSystem/Library/item\n/Bad\t../bad\n/Bad\t/absolute\n";
        let mappings = mappings(text, &root.join("Data"));
        assert_eq!(mappings.len(), 1);
        let p = Policy::from_mappings(vec![(
            root.join("Data/System/Library/item"),
            root.join("Public/item"),
        )]);
        let id = platform_identity(&root.join("Public/item")).unwrap();
        assert!(p.may_have_alias_children(&root.join("Data/System/Library")));
        assert!(p.is_mirror(&root.join("Data/System/Library/item"), &id));
        assert!(!p.is_mirror(&root.join("Data/System/Library"), &id));
        assert!(!p.is_mirror(&root.join("Data/.Spotlight-V100"), &id));
        let mut replaced = id.clone();
        replaced.inode += 1;
        assert!(!p.is_mirror(&root.join("Data/System/Library/item"), &replaced));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_and_rejected_policies_have_no_alias_parents() {
        assert!(!Policy::default().may_have_alias_children(Path::new("/")));
        let root = std::env::temp_dir().join(format!(
            "hahakigami-firmlink-rejected-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("data"), b"x").unwrap();
        std::fs::write(root.join("public"), b"x").unwrap();
        let policy = Policy::from_mappings(vec![
            (root.join("data"), root.join("public")),
            (root.join("missing"), root.join("public")),
        ]);
        assert!(policy.aliases.is_empty());
        assert!(!policy.may_have_alias_children(&root));
        // Loading any ordinary scan root must not enable startup alias filtering.
        assert!(!Policy::load(&root).may_have_alias_children(&root));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_exact_parents_of_file_and_directory_aliases_match() {
        let root = std::env::temp_dir().join(format!(
            "hahakigami-firmlink-parents-{}",
            std::process::id()
        ));
        let parent = root.join("Data/System");
        let nested = parent.join("Library");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("item");
        std::fs::write(&file, b"x").unwrap();
        // Same-path mappings provide verified directory and file identities without
        // depending on macOS firmlinks or requiring directory hard-link creation.
        let policy = Policy::from_mappings(vec![
            (nested.clone(), nested.clone()),
            (file.clone(), file.clone()),
        ]);
        assert!(policy.may_have_alias_children(&parent));
        assert!(policy.may_have_alias_children(&nested));
        for path in [
            root.join("Data"),
            root.join("Data/Systematic"),
            nested.join("child"),
            file.clone(),
        ] {
            assert!(!policy.may_have_alias_children(&path), "{path:?}");
        }
        for path in [&nested, &file] {
            let identity = platform_identity(path).unwrap();
            assert!(policy.is_mirror(path, &identity));
            let mut replaced = identity;
            replaced.inode += 1;
            assert!(!policy.is_mirror(path, &replaced));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn alias_parent_keeps_native_path_bytes() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let root = Path::new("/Data");
        let parent = root.join(OsStr::from_bytes(b"native-\xff\nparent"));
        let alias = parent.join("item");
        let identity = Identity {
            device: 1,
            inode: 2,
            kind: crate::NodeKind::File,
        };
        // Some filesystems reject invalid UTF-8 names, so exercise native lookup
        // directly rather than relying on the host accepting this spelling.
        let policy = Policy {
            aliases: HashMap::from([(alias.clone(), identity.clone())]),
            alias_parents: HashSet::from([parent.clone()]),
        };
        assert!(policy.may_have_alias_children(&parent));
        assert!(!policy.may_have_alias_children(root));
        assert!(!policy.may_have_alias_children(Path::new(&parent.to_string_lossy().into_owned())));
        assert!(policy.is_mirror(&alias, &identity));
    }
}
