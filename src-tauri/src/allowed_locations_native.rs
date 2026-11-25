//! Security-scoped `NSURL` bookmarks behind the `Bookmarks` adapter. Not unit tested: the sandbox that gives them
//! meaning only exists in a signed Lite build (docs/VERIFICATION.md checks it by hand).
use crate::allowed_locations::{Bookmarks, Resolved};
use objc2::{rc::Retained, runtime::Bool};
use objc2_foundation::{
    NSData, NSError, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSURL,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub struct NativeBookmarks {
    /// Resolved URLs with access started, by path. They stay accessed until `stop` or the process ends.
    accessed: HashMap<PathBuf, Vec<Retained<NSURL>>>,
}

fn message(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

impl Bookmarks for NativeBookmarks {
    fn create(&mut self, path: &Path) -> Result<Vec<u8>, String> {
        let url = NSURL::from_directory_path(path).ok_or("The folder's path is not valid.")?;
        url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::WithSecurityScope,
            None,
            None,
        )
        .map(|data| data.to_vec())
        .map_err(|error| message(&error))
    }

    fn resolve(&mut self, bookmark: &[u8]) -> Result<Resolved, String> {
        let data = NSData::with_bytes(bookmark);
        let mut stale = Bool::NO;
        // SAFETY: `stale` is a valid pointer for the call.
        let url = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions::WithSecurityScope,
                None,
                &mut stale,
            )
        }
        .map_err(|error| message(&error))?;
        let path = url
            .path()
            .map(|p| PathBuf::from(p.to_string()))
            .ok_or("The location has no path.")?;
        // SAFETY: balanced by `stop`, or released when the process exits.
        if !unsafe { url.startAccessingSecurityScopedResource() } {
            return Err("Access to the location was refused.".into());
        }
        self.accessed.entry(path.clone()).or_default().push(url);
        Ok(Resolved {
            path,
            stale: stale.as_bool(),
        })
    }

    fn stop(&mut self, path: &Path) {
        for url in self.accessed.remove(path).into_iter().flatten() {
            // SAFETY: balances the `startAccessingSecurityScopedResource` of `resolve`.
            unsafe { url.stopAccessingSecurityScopedResource() };
        }
    }
}
