//! Bounded retries for interrupted, idempotent metadata work. Never used for close(2).
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};
pub(crate) const ATTEMPTS: usize = 4;
pub(crate) fn interrupted<T>(
    cancel: Option<&AtomicBool>,
    mut operation: impl FnMut(usize) -> io::Result<T>,
) -> io::Result<T> {
    for attempt in 0..ATTEMPTS {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(io::Error::from_raw_os_error(libc::EINTR));
        }
        match operation(attempt) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted && attempt + 1 < ATTEMPTS => {}
            result => return result,
        }
    }
    unreachable!("bounded retry always returns")
}
/// Keep previous entry failures and the terminal cause together; never conceal an earlier failure.
pub(crate) fn terminal_listing<T>(
    mut entries: Vec<io::Result<T>>,
    terminal: io::Error,
) -> io::Result<Vec<io::Result<T>>> {
    if entries.iter().any(Result::is_err) {
        entries.push(Err(terminal));
        Ok(entries)
    } else {
        Err(terminal)
    }
}
/// An interrupted entry invalidates its complete listing, including already-read successful entries.
pub(crate) fn complete_listing<T>(entries: Vec<io::Result<T>>) -> io::Result<Vec<io::Result<T>>> {
    // A permanent issue must not be concealed by a successful later pass of the same listing.
    // Vanished and dataless entries are dropped silently by the walk, so they are not issues.
    if entries
        .iter()
        .filter_map(|entry| entry.as_ref().err())
        .any(|error| {
            error.kind() != io::ErrorKind::Interrupted
                && !crate::vanished(error)
                && !crate::dataless(error)
        })
    {
        return Ok(entries);
    }
    if let Some(error) = entries
        .iter()
        .filter_map(|entry| entry.as_ref().err())
        .find(|error| error.kind() == io::ErrorKind::Interrupted)
    {
        return Err(crate::native_entry_error(error).raw_os_error().map_or_else(
            || io::Error::new(error.kind(), error.to_string()),
            io::Error::from_raw_os_error,
        ));
    }
    Ok(entries)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_success_exhaustion_and_permanent_errors() {
        let mut calls = 0;
        assert_eq!(
            interrupted(None, |_| {
                calls += 1;
                if calls < 3 {
                    Err(io::Error::from_raw_os_error(libc::EINTR))
                } else {
                    Ok(7)
                }
            })
            .unwrap(),
            7
        );
        assert_eq!(calls, 3);
        let mut calls = 0;
        let error = interrupted::<()>(None, |_| {
            calls += 1;
            Err(io::Error::from_raw_os_error(libc::EINTR))
        })
        .unwrap_err();
        assert_eq!(calls, ATTEMPTS);
        assert_eq!(error.raw_os_error(), Some(libc::EINTR));
        let mut calls = 0;
        let error = interrupted::<()>(None, |_| {
            calls += 1;
            Err(io::Error::from_raw_os_error(libc::EACCES))
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(error.raw_os_error(), Some(libc::EACCES));
    }
    #[test]
    fn cancellation_stops_between_attempts() {
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let error = interrupted::<()>(Some(&cancel), |_| {
            calls += 1;
            cancel.store(true, Ordering::Relaxed);
            Err(io::Error::from_raw_os_error(libc::EINTR))
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(error.raw_os_error(), Some(libc::EINTR));
    }
    #[test]
    fn exhausted_interrupted_entry_discards_partial_listing_before_retry() {
        let mut calls = 0;
        let result = interrupted(None, |_| {
            calls += 1;
            complete_listing(if calls == 1 {
                vec![Ok(1), Err(io::Error::from_raw_os_error(libc::EINTR))]
            } else {
                vec![Ok(2), Ok(3)]
            })
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(
            result.into_iter().collect::<io::Result<Vec<_>>>().unwrap(),
            vec![2, 3]
        );
    }
    #[test]
    fn permanent_entry_error_prevents_listing_retry() {
        let mut calls = 0;
        let result = interrupted(None, |_| {
            calls += 1;
            complete_listing(vec![
                Ok(1),
                Err(io::Error::from_raw_os_error(libc::EACCES)),
                Err(io::Error::from_raw_os_error(libc::EINTR)),
            ])
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(
            result[1].as_ref().unwrap_err().raw_os_error(),
            Some(libc::EACCES)
        );
        assert_eq!(
            result[2].as_ref().unwrap_err().raw_os_error(),
            Some(libc::EINTR)
        );
    }
    #[test]
    fn permanent_entry_before_terminal_interrupt_never_becomes_clean_on_retry() {
        for backend in ["bulk", "portable"] {
            for prior in [libc::EACCES, libc::EMFILE] {
                let mut calls = 0;
                let listing = interrupted(None, |_| {
                    calls += 1;
                    if calls > 1 {
                        return Ok(vec![Ok(9)]);
                    }
                    terminal_listing(
                        vec![Ok(1), Err(io::Error::from_raw_os_error(prior))],
                        io::Error::from_raw_os_error(libc::EINTR),
                    )
                    .and_then(complete_listing)
                })
                .unwrap();
                assert_eq!(calls, 1, "{backend}");
                assert_eq!(listing[1].as_ref().unwrap_err().raw_os_error(), Some(prior));
                assert_eq!(
                    listing[2].as_ref().unwrap_err().raw_os_error(),
                    Some(libc::EINTR)
                );
            }
        }
    }
    #[test]
    fn silently_dropped_entry_errors_do_not_block_the_interrupt_retry() {
        for prior in [libc::ENOENT, libc::ENOTDIR, libc::EDEADLK] {
            let mut calls = 0;
            let listing = interrupted(None, |_| {
                calls += 1;
                if calls > 1 {
                    return Ok(vec![Ok(9)]);
                }
                terminal_listing(
                    vec![Ok(1), Err(io::Error::from_raw_os_error(prior))],
                    io::Error::from_raw_os_error(libc::EINTR),
                )
                .and_then(complete_listing)
            })
            .unwrap();
            assert_eq!(calls, 2, "errno {prior}");
            assert_eq!(
                listing.into_iter().collect::<io::Result<Vec<_>>>().unwrap(),
                vec![9]
            );
        }
    }
    #[test]
    fn terminal_permanent_failure_is_retained_alongside_earlier_error() {
        let listing = terminal_listing::<u8>(
            vec![Err(io::Error::from_raw_os_error(libc::EMFILE))],
            io::Error::from_raw_os_error(libc::EIO),
        )
        .and_then(complete_listing)
        .unwrap();
        assert_eq!(listing.len(), 2);
        assert_eq!(
            listing[0].as_ref().unwrap_err().raw_os_error(),
            Some(libc::EMFILE)
        );
        assert_eq!(
            listing[1].as_ref().unwrap_err().raw_os_error(),
            Some(libc::EIO)
        );
        let budget = terminal_listing::<u8>(
            vec![Err(io::Error::from_raw_os_error(libc::EMFILE))],
            io::Error::other("calibration directory listing budget exceeded"),
        )
        .and_then(complete_listing)
        .unwrap();
        assert_eq!(
            budget[0].as_ref().unwrap_err().raw_os_error(),
            Some(libc::EMFILE)
        );
        assert_eq!(
            budget[1].as_ref().unwrap_err().to_string(),
            "calibration directory listing budget exceeded"
        );
        let mut calls = 0;
        let listing = interrupted(None, |_| {
            calls += 1;
            if calls == 1 {
                terminal_listing(vec![Ok(1)], io::Error::from_raw_os_error(libc::EINTR))
            } else {
                Ok(vec![Ok(2)])
            }
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(listing[0].as_ref().unwrap(), &2);
    }
}
