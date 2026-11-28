//! Allowed locations (ADR 0007): the disks and folders the person picked in an Open dialog so the Lite edition may
//! read them, remembered across launches. The Full edition never builds a store.
//!
//! Bookmark creation, resolution and access go through `Bookmarks`: native security-scoped `NSURL` bookmarks in the
//! app (`allowed_locations_native`), a fake in the tests. A location that cannot be resolved, or resolves stale and
//! cannot be refreshed, is not allowed, which is a state and not an error: the person grants it again in one click.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The start of the error a refused scan carries. The frontend recognises it and offers "Allow access…" instead of
/// showing a generic failure; what follows is a sentence in plain words.
pub const NEEDS_ACCESS: &str = "NEEDS_ACCESS";

/// A resolved bookmark, with access to it started.
pub struct Resolved {
    pub path: PathBuf,
    /// The bookmark still works but should be created again from `path`.
    pub stale: bool,
}

/// Creates and resolves security-scoped bookmarks.
pub trait Bookmarks: Send {
    fn create(&mut self, path: &Path) -> Result<Vec<u8>, String>;
    /// Resolves `bookmark` and starts security-scoped access to it, kept until `stop`.
    fn resolve(&mut self, bookmark: &[u8]) -> Result<Resolved, String>;
    /// Ends the access `resolve` started for `path`. Does nothing for a path that is not accessed.
    fn stop(&mut self, path: &Path);
}

struct Entry {
    path: PathBuf,
    bookmark: Vec<u8>,
    /// Access is started: the location resolved and is readable. An entry that is not allowed stays in the file, so
    /// a disk that is away at launch is allowed again when it comes back.
    allowed: bool,
}

pub struct AllowedLocations {
    bookmarks: Box<dyn Bookmarks>,
    file: PathBuf,
    entries: Vec<Entry>,
}

impl AllowedLocations {
    /// Reads the store in `file` and starts access to every location in it that still resolves, for the app's
    /// lifetime. A missing or corrupt file, or an entry that cannot be read, counts as empty.
    pub fn restore_all(bookmarks: Box<dyn Bookmarks>, file: PathBuf) -> Self {
        let mut store = Self {
            bookmarks,
            entries: read_entries(&file),
            file,
        };
        if store.resolve_all() {
            store.persist();
        }
        store
    }

    /// Allows `path` (a folder just picked in an Open dialog) and keeps it for later launches.
    pub fn remember(&mut self, path: &Path) -> Result<(), String> {
        let bookmark = self.bookmarks.create(path)?;
        // A location picked again replaces its old entry, so access is started once.
        let Self {
            bookmarks, entries, ..
        } = self;
        entries.retain(|e| {
            let same = e.path == path;
            if same && e.allowed {
                bookmarks.stop(&e.path);
            }
            !same
        });
        let resolved = self.bookmarks.resolve(&bookmark);
        if let Ok(resolved) = &resolved {
            self.entries.push(Entry {
                path: resolved.path.clone(),
                bookmark,
                allowed: true,
            });
        }
        self.persist();
        resolved.map(|_| ())
    }

    /// Forgets `path` and ends access to it. False when it was not a remembered location.
    pub fn remove(&mut self, path: &Path) -> bool {
        let Some(index) = self.entries.iter().position(|e| e.path == path) else {
            return false;
        };
        let entry = self.entries.remove(index);
        if entry.allowed {
            self.bookmarks.stop(&entry.path);
        }
        self.persist();
        true
    }

    /// The Allowed location that `path` is, or lies inside: the innermost one.
    pub fn containing(&self, path: &Path) -> Option<&Path> {
        self.entries
            .iter()
            .filter(|e| e.allowed && path.starts_with(&e.path))
            .max_by_key(|e| e.path.components().count())
            .map(|e| e.path.as_path())
    }

    /// The Allowed location other than `path` itself that `path` lies inside: the innermost one.
    pub fn outer(&self, path: &Path) -> Option<&Path> {
        self.entries
            .iter()
            .filter(|e| e.allowed && e.path != path && path.starts_with(&e.path))
            .max_by_key(|e| e.path.components().count())
            .map(|e| e.path.as_path())
    }

    /// Ends access to every Allowed location whose path `exists` no longer finds (a disk was ejected, a folder moved
    /// or deleted) and marks it not allowed, so the next lookup resolves its bookmark again: the disk may be back
    /// under another name. The entry stays in the file.
    pub fn lose_missing(&mut self, exists: impl Fn(&Path) -> bool) {
        for entry in self.entries.iter_mut().filter(|e| e.allowed) {
            if !exists(&entry.path) {
                self.bookmarks.stop(&entry.path);
                entry.allowed = false;
            }
        }
    }

    /// Tries again the locations that were not allowed (a disk that was away has come back).
    pub fn reresolve(&mut self) {
        if self.resolve_all() {
            self.persist();
        }
    }

    /// Ok when the Scan target is, or lies inside, an Allowed location; otherwise the "needs access" refusal.
    pub fn require_allowed(&mut self, target: &Path) -> Result<(), String> {
        if self.containing(target).is_none() && self.entries.iter().any(|e| !e.allowed) {
            self.reresolve();
        }
        match self.containing(target) {
            Some(_) => Ok(()),
            None => Err(format!(
                "{NEEDS_ACCESS}: Allow access to {} before scanning it.",
                target.display()
            )),
        }
    }

    /// Starts access to every entry that is not allowed yet. True when an entry changed and the file is stale.
    fn resolve_all(&mut self) -> bool {
        let mut changed = false;
        for entry in self.entries.iter_mut().filter(|e| !e.allowed) {
            let Ok(resolved) = self.bookmarks.resolve(&entry.bookmark) else {
                continue;
            };
            if resolved.stale {
                match self.bookmarks.create(&resolved.path) {
                    Ok(fresh) => {
                        entry.bookmark = fresh;
                        changed = true;
                    }
                    Err(_) => {
                        self.bookmarks.stop(&resolved.path);
                        continue;
                    }
                }
            }
            changed |= entry.path != resolved.path;
            entry.path = resolved.path;
            entry.allowed = true;
        }
        changed
    }

    /// Writes the store. A failure only means the next launch asks again, so the locations stay allowed.
    fn persist(&self) {
        if let Err(error) = write_entries(&self.file, &self.entries) {
            eprintln!("Could not save the allowed locations: {error}");
        }
    }
}

fn read_entries(file: &Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let Some(Value::Array(items)) = root.get("locations") else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            Some(Entry {
                path: PathBuf::from(item.get("path")?.as_str()?),
                bookmark: from_hex(item.get("bookmark")?.as_str()?)?,
                allowed: false,
            })
        })
        .collect()
}

fn write_entries(file: &Path, entries: &[Entry]) -> std::io::Result<()> {
    let locations: Vec<Value> = entries
        .iter()
        .filter_map(|e| {
            // A path that is not UTF-8 cannot be stored in JSON; it stays allowed for this launch only.
            Some(json!({"path": e.path.to_str()?, "bookmark": to_hex(&e.bookmark)}))
        })
        .collect();
    let text = json!({"version": 1, "locations": locations}).to_string();
    if let Some(directory) = file.parent() {
        std::fs::create_dir_all(directory)?;
    }
    // Whole or not at all: a crash must not leave half a file that reads as empty.
    let temporary = file.with_extension("json.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, file)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    /// A bookmark is "generation|path". `unreachable` paths fail to resolve, like a disk that is not mounted. A
    /// bookmark made up to a path's `stale_through` generation resolves stale; a newer one does not.
    #[derive(Default)]
    struct World {
        unreachable: Vec<PathBuf>,
        stale_through: HashMap<PathBuf, u32>,
        renamed: HashMap<PathBuf, PathBuf>,
        refuse_create: Vec<PathBuf>,
        accessed: Vec<PathBuf>,
        created: u32,
    }
    impl World {
        /// Every bookmark made so far is stale from now on.
        fn go_stale(&mut self, path: &str) {
            self.stale_through.insert(path.into(), self.created);
        }
    }
    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<World>>);
    impl Bookmarks for Fake {
        fn create(&mut self, path: &Path) -> Result<Vec<u8>, String> {
            let mut world = self.0.lock().unwrap();
            if world.refuse_create.iter().any(|p| p == path) {
                return Err("no bookmark".into());
            }
            world.created += 1;
            Ok(format!("{}|{}", world.created, path.to_str().unwrap()).into_bytes())
        }
        fn resolve(&mut self, bookmark: &[u8]) -> Result<Resolved, String> {
            let mut world = self.0.lock().unwrap();
            let text = std::str::from_utf8(bookmark).map_err(|e| e.to_string())?;
            let (generation, path) = text.split_once('|').ok_or("not a bookmark")?;
            let (generation, path) = (generation.parse::<u32>().unwrap(), PathBuf::from(path));
            if world.unreachable.contains(&path) {
                return Err("not found".into());
            }
            let stale = world
                .stale_through
                .get(&path)
                .is_some_and(|through| generation <= *through);
            let path = world.renamed.get(&path).cloned().unwrap_or(path);
            world.accessed.push(path.clone());
            Ok(Resolved { path, stale })
        }
        fn stop(&mut self, path: &Path) {
            self.0.lock().unwrap().accessed.retain(|p| p != path);
        }
    }
    fn locations(allowed: &AllowedLocations) -> Vec<PathBuf> {
        allowed
            .entries
            .iter()
            .filter(|e| e.allowed)
            .map(|e| e.path.clone())
            .collect()
    }
    fn store(fake: &Fake, dir: &Path) -> AllowedLocations {
        AllowedLocations::restore_all(Box::new(fake.clone()), dir.join("allowed-locations.json"))
    }

    #[test]
    fn a_remembered_folder_and_everything_in_it_is_allowed() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Users/a/Projects")).unwrap();
        assert_eq!(
            allowed.containing(Path::new("/Users/a/Projects")),
            Some(Path::new("/Users/a/Projects"))
        );
        assert_eq!(
            allowed.containing(Path::new("/Users/a/Projects/x/y")),
            Some(Path::new("/Users/a/Projects"))
        );
        assert_eq!(allowed.containing(Path::new("/Users/a")), None);
    }

    #[test]
    fn a_sibling_that_shares_a_name_prefix_is_not_allowed() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Users/a/Project")).unwrap();
        assert_eq!(allowed.containing(Path::new("/Users/a/Project2")), None);
        assert_eq!(
            allowed.containing(Path::new("/Users/a/Project-old/x")),
            None
        );
    }

    #[test]
    fn a_volume_root_allows_everything_on_it() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/")).unwrap();
        assert_eq!(allowed.containing(Path::new("/")), Some(Path::new("/")));
        assert_eq!(
            allowed.containing(Path::new("/Users/a/x")),
            Some(Path::new("/"))
        );
        let other = tempfile::tempdir().unwrap();
        let mut external = store(&fake, other.path());
        external.remember(Path::new("/Volumes/Backup")).unwrap();
        assert_eq!(
            external.containing(Path::new("/Volumes/Backup/x")),
            Some(Path::new("/Volumes/Backup"))
        );
        assert_eq!(external.containing(Path::new("/Volumes/Backup 2")), None);
        assert_eq!(external.containing(Path::new("/Volumes")), None);
    }

    #[test]
    fn the_innermost_covering_location_is_found_regardless_of_order() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/")).unwrap();
        allowed.remember(Path::new("/Users/a")).unwrap();
        assert_eq!(
            allowed.containing(Path::new("/Users/a/x")),
            Some(Path::new("/Users/a"))
        );
    }

    #[test]
    fn locations_come_back_after_a_relaunch() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        store(&fake, dir.path())
            .remember(Path::new("/Users/a/Projects"))
            .unwrap();
        // A new process: nothing is accessed until the store restores.
        fake.0.lock().unwrap().accessed.clear();
        let relaunched = store(&fake, dir.path());
        assert_eq!(
            relaunched.containing(Path::new("/Users/a/Projects/x")),
            Some(Path::new("/Users/a/Projects"))
        );
        assert_eq!(
            fake.0.lock().unwrap().accessed,
            [PathBuf::from("/Users/a/Projects")]
        );
    }

    #[test]
    fn a_location_that_does_not_resolve_is_not_allowed_and_not_an_error() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        store(&fake, dir.path())
            .remember(Path::new("/Volumes/Backup"))
            .unwrap();
        fake.0
            .lock()
            .unwrap()
            .unreachable
            .push("/Volumes/Backup".into());
        let relaunched = store(&fake, dir.path());
        assert_eq!(relaunched.containing(Path::new("/Volumes/Backup/x")), None);
    }

    #[test]
    fn a_location_that_could_not_resolve_is_allowed_again_when_it_comes_back() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        store(&fake, dir.path())
            .remember(Path::new("/Volumes/Backup"))
            .unwrap();
        fake.0
            .lock()
            .unwrap()
            .unreachable
            .push("/Volumes/Backup".into());
        let mut relaunched = store(&fake, dir.path());
        fake.0.lock().unwrap().unreachable.clear();
        relaunched.reresolve();
        assert_eq!(
            relaunched.containing(Path::new("/Volumes/Backup/x")),
            Some(Path::new("/Volumes/Backup"))
        );
        // A launch while it is away must not forget it: the file keeps it.
        fake.0
            .lock()
            .unwrap()
            .unreachable
            .push("/Volumes/Backup".into());
        drop(relaunched);
        let mut again = store(&fake, dir.path());
        fake.0.lock().unwrap().unreachable.clear();
        again.reresolve();
        assert!(again.containing(Path::new("/Volumes/Backup")).is_some());
    }

    #[test]
    fn a_stale_bookmark_is_refreshed_and_stays_allowed() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        store(&fake, dir.path())
            .remember(Path::new("/Users/a/Projects"))
            .unwrap();
        fake.0.lock().unwrap().go_stale("/Users/a/Projects");
        let relaunched = store(&fake, dir.path());
        assert!(relaunched
            .containing(Path::new("/Users/a/Projects"))
            .is_some());
        // The refreshed bookmark is what the file holds now: the next launch needs no refresh.
        let made = fake.0.lock().unwrap().created;
        assert!(store(&fake, dir.path())
            .containing(Path::new("/Users/a/Projects"))
            .is_some());
        assert_eq!(fake.0.lock().unwrap().created, made);
    }

    #[test]
    fn a_stale_bookmark_that_cannot_be_refreshed_is_not_allowed() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        store(&fake, dir.path())
            .remember(Path::new("/Users/a/Projects"))
            .unwrap();
        {
            let mut world = fake.0.lock().unwrap();
            world.accessed.clear();
            world.go_stale("/Users/a/Projects");
            world.refuse_create.push("/Users/a/Projects".into());
        }
        let relaunched = store(&fake, dir.path());
        assert_eq!(relaunched.containing(Path::new("/Users/a/Projects")), None);
        // The access it started for the stale bookmark ended again.
        assert!(fake.0.lock().unwrap().accessed.is_empty());
    }

    #[test]
    fn a_renamed_location_is_found_under_its_new_path() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        store(&fake, dir.path())
            .remember(Path::new("/Users/a/Old"))
            .unwrap();
        fake.0
            .lock()
            .unwrap()
            .renamed
            .insert("/Users/a/Old".into(), "/Users/a/New".into());
        let relaunched = store(&fake, dir.path());
        assert_eq!(
            relaunched.containing(Path::new("/Users/a/New/x")),
            Some(Path::new("/Users/a/New"))
        );
        assert_eq!(relaunched.containing(Path::new("/Users/a/Old")), None);
    }

    #[test]
    fn a_missing_or_corrupt_store_is_empty() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        assert_eq!(store(&fake, dir.path()).containing(Path::new("/")), None);
        for junk in [
            "",
            "not json",
            "{\"locations\": 7}",
            "[1,2]",
            "{\"locations\":[{\"path\":1}]}",
        ] {
            std::fs::write(dir.path().join("allowed-locations.json"), junk).unwrap();
            let mut allowed = store(&fake, dir.path());
            assert_eq!(allowed.containing(Path::new("/")), None, "{junk}");
            // And it is still usable, replacing the junk.
            allowed.remember(Path::new("/Users/a")).unwrap();
            assert!(store(&fake, dir.path())
                .containing(Path::new("/Users/a"))
                .is_some());
        }
    }

    #[test]
    fn an_entry_that_is_not_valid_is_skipped_and_the_rest_restore() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        store(&fake, dir.path())
            .remember(Path::new("/Users/a"))
            .unwrap();
        let file = dir.path().join("allowed-locations.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        json["locations"]
            .as_array_mut()
            .unwrap()
            .insert(0, serde_json::json!({"path": "/x", "bookmark": "zz"}));
        std::fs::write(&file, json.to_string()).unwrap();
        assert!(store(&fake, dir.path())
            .containing(Path::new("/Users/a"))
            .is_some());
    }

    #[test]
    fn remembering_a_location_again_keeps_one_entry() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Users/a")).unwrap();
        allowed.remember(Path::new("/Users/a")).unwrap();
        assert_eq!(locations(&allowed), [PathBuf::from("/Users/a")]);
        assert_eq!(fake.0.lock().unwrap().accessed, [PathBuf::from("/Users/a")]);
        assert_eq!(
            locations(&store(&fake, dir.path())),
            [PathBuf::from("/Users/a")]
        );
    }

    #[test]
    fn a_location_that_cannot_be_bookmarked_is_refused_and_not_allowed() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        fake.0.lock().unwrap().refuse_create.push("/Users/a".into());
        let mut allowed = store(&fake, dir.path());
        assert!(allowed.remember(Path::new("/Users/a")).is_err());
        assert_eq!(allowed.containing(Path::new("/Users/a")), None);
    }

    #[test]
    fn a_removed_location_is_forgotten_and_its_access_ends() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Users/a")).unwrap();
        allowed.remember(Path::new("/Users/b")).unwrap();
        assert!(allowed.remove(Path::new("/Users/a")));
        assert!(!allowed.remove(Path::new("/Users/a")));
        assert_eq!(allowed.containing(Path::new("/Users/a/x")), None);
        assert_eq!(fake.0.lock().unwrap().accessed, [PathBuf::from("/Users/b")]);
        let relaunched = store(&fake, dir.path());
        assert_eq!(locations(&relaunched), [PathBuf::from("/Users/b")]);
    }

    #[test]
    fn a_removed_location_stays_forgotten_even_when_it_could_resolve_again() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Volumes/Backup")).unwrap();
        assert!(allowed.remove(Path::new("/Volumes/Backup")));
        allowed.reresolve();
        assert!(allowed
            .require_allowed(Path::new("/Volumes/Backup/x"))
            .unwrap_err()
            .starts_with("NEEDS_ACCESS: "));
        assert!(fake.0.lock().unwrap().accessed.is_empty());
        assert!(locations(&store(&fake, dir.path())).is_empty());
    }

    #[test]
    fn removing_a_location_that_is_not_allowed_forgets_it_without_ending_any_access() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Volumes/Backup")).unwrap();
        allowed.remember(Path::new("/Users/a")).unwrap();
        fake.0
            .lock()
            .unwrap()
            .unreachable
            .push("/Volumes/Backup".into());
        drop(allowed);
        let mut relaunched = store(&fake, dir.path());
        assert!(relaunched.remove(Path::new("/Volumes/Backup")));
        // It would have come back with the disk; now it does not.
        fake.0.lock().unwrap().unreachable.clear();
        assert_eq!(relaunched.containing(Path::new("/Volumes/Backup")), None);
        relaunched.reresolve();
        assert_eq!(relaunched.containing(Path::new("/Volumes/Backup")), None);
        assert_eq!(
            locations(&store(&fake, dir.path())),
            [PathBuf::from("/Users/a")]
        );
    }

    #[test]
    fn removing_a_location_keeps_the_ones_inside_and_around_it() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/")).unwrap();
        allowed.remember(Path::new("/Users/a")).unwrap();
        allowed.remember(Path::new("/Users/a/Projects")).unwrap();
        assert!(allowed.remove(Path::new("/Users/a")));
        assert_eq!(
            allowed.containing(Path::new("/Users/a/Projects/x")),
            Some(Path::new("/Users/a/Projects"))
        );
        assert_eq!(
            allowed.containing(Path::new("/Users/a/x")),
            Some(Path::new("/"))
        );
        assert_eq!(
            fake.0.lock().unwrap().accessed,
            [PathBuf::from("/"), PathBuf::from("/Users/a/Projects")]
        );
    }

    #[test]
    fn a_location_that_never_resolves_stays_in_the_file_until_it_is_allowed_again_or_removed() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        store(&fake, dir.path())
            .remember(Path::new("/Users/a/Deleted"))
            .unwrap();
        fake.0
            .lock()
            .unwrap()
            .unreachable
            .push("/Users/a/Deleted".into());
        // Launch after launch it is simply not allowed: no error, and the entry is kept for a disk that returns.
        for _ in 0..2 {
            let mut allowed = store(&fake, dir.path());
            assert!(allowed
                .require_allowed(Path::new("/Users/a/Deleted"))
                .unwrap_err()
                .starts_with("NEEDS_ACCESS: "));
        }
        // Granting it again replaces the dead entry instead of piling up beside it.
        fake.0.lock().unwrap().unreachable.clear();
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Users/a/Deleted")).unwrap();
        assert_eq!(locations(&allowed), [PathBuf::from("/Users/a/Deleted")]);
        assert_eq!(
            fake.0.lock().unwrap().accessed,
            [PathBuf::from("/Users/a/Deleted")]
        );
        assert!(allowed.remove(Path::new("/Users/a/Deleted")));
        assert!(!allowed.remove(Path::new("/Users/a/Deleted")));
    }

    #[test]
    fn a_scan_target_outside_every_location_needs_access() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Users/a")).unwrap();
        assert_eq!(allowed.require_allowed(Path::new("/Users/a/x")), Ok(()));
        let refused = allowed
            .require_allowed(Path::new("/Volumes/Backup"))
            .unwrap_err();
        assert!(refused.starts_with("NEEDS_ACCESS: "), "{refused}");
        assert!(refused.contains("/Volumes/Backup"));
    }

    #[test]
    fn a_scan_target_on_a_disk_that_came_back_is_allowed_again() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        store(&fake, dir.path())
            .remember(Path::new("/Volumes/Backup"))
            .unwrap();
        fake.0
            .lock()
            .unwrap()
            .unreachable
            .push("/Volumes/Backup".into());
        let mut allowed = store(&fake, dir.path());
        assert!(allowed
            .require_allowed(Path::new("/Volumes/Backup"))
            .is_err());
        fake.0.lock().unwrap().unreachable.clear();
        assert_eq!(
            allowed.require_allowed(Path::new("/Volumes/Backup")),
            Ok(())
        );
    }

    #[test]
    fn a_disk_that_was_ejected_and_comes_back_under_another_name_is_allowed_again() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Volumes/Backup")).unwrap();
        // Ejected, then plugged in again as "Backup 1".
        fake.0
            .lock()
            .unwrap()
            .renamed
            .insert("/Volumes/Backup".into(), "/Volumes/Backup 1".into());
        allowed.lose_missing(|_| false);
        assert_eq!(allowed.containing(Path::new("/Volumes/Backup")), None);
        assert!(fake.0.lock().unwrap().accessed.is_empty());
        assert_eq!(
            allowed.require_allowed(Path::new("/Volumes/Backup 1/x")),
            Ok(())
        );
        assert_eq!(
            fake.0.lock().unwrap().accessed,
            [PathBuf::from("/Volumes/Backup 1")]
        );
        // The new spelling is what the file keeps.
        assert_eq!(
            locations(&store(&fake, dir.path())),
            [PathBuf::from("/Volumes/Backup 1")]
        );
    }

    #[test]
    fn locations_that_still_exist_stay_allowed_when_the_mounts_change() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Users/a")).unwrap();
        allowed.remember(Path::new("/Volumes/Backup")).unwrap();
        allowed.lose_missing(|path| path != Path::new("/Volumes/Backup"));
        assert_eq!(locations(&allowed), [PathBuf::from("/Users/a")]);
        assert_eq!(fake.0.lock().unwrap().accessed, [PathBuf::from("/Users/a")]);
    }

    #[test]
    fn the_outer_location_ignores_the_paths_own_entry() {
        let (fake, dir) = (Fake::default(), tempfile::tempdir().unwrap());
        let mut allowed = store(&fake, dir.path());
        allowed.remember(Path::new("/Users/a")).unwrap();
        allowed.remember(Path::new("/Users/a/Projects")).unwrap();
        assert_eq!(allowed.outer(Path::new("/Users/a")), None);
        assert_eq!(
            allowed.outer(Path::new("/Users/a/Projects")),
            Some(Path::new("/Users/a"))
        );
        assert_eq!(
            allowed.outer(Path::new("/Users/a/Projects/x")),
            Some(Path::new("/Users/a/Projects"))
        );
    }
}
