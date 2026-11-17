//! One worker and descriptor budget for the whole process, split across the scans that run at once.
//!
//! A scan takes its share when it starts and gives it back when its `Grant` drops, on every way a scan
//! can end. Scans already running keep what they hold; nothing is rebalanced.
//! - Workers are never a reason to refuse a start. Alone, a scan gets what it asks for. Beside others it
//!   gets at most half of what is still free, but at least one, so a Mac with few threads still scans two
//!   disks (oversubscribing by a thread each) and a scan stuck on a dead mount cannot lock the rest out.
//! - Descriptors are a hard total, because the process limit is. Alone, a scan gets what it asks for.
//!   Beside others it gets half of what is free, never less than `MIN_DESCRIPTORS`. A start is refused only
//!   when none are left, which takes seven scans at once with the usual 512.
use std::sync::{Arc, Mutex, PoisonError};

/// Enough for a scan to keep its root and a few parents open; a scan with fewer falls back to path walks.
const MIN_DESCRIPTORS: usize = 8;
/// macOS rejects a soft limit above OPEN_MAX, and RLIM_INFINITY.
const MAX_SOFT_LIMIT: libc::rlim_t = 10240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Share {
    pub workers: usize,
    pub descriptors: usize,
}

struct Pool {
    // Can exceed the total: every start gets at least one worker.
    held_workers: usize,
    free_descriptors: usize,
    held: usize,
}

#[derive(Clone)]
pub struct Budget {
    total: Share,
    pool: Arc<Mutex<Pool>>,
}

/// A running scan's share. Dropping it returns the share to the budget.
pub struct Grant {
    share: Share,
    budget: Budget,
}

impl Budget {
    pub fn new(total: Share) -> Self {
        let total = Share {
            workers: total.workers.max(1),
            descriptors: total.descriptors.max(1),
        };
        Self {
            total,
            pool: Arc::new(Mutex::new(Pool {
                held_workers: 0,
                free_descriptors: total.descriptors,
                held: 0,
            })),
        }
    }
    /// A scan that wants up to `workers` workers. It asks for half the descriptors, never more than it
    /// needs: the measured peak of a 1.4M-entry scan is 57 of 512, and the rest stays for later scans.
    pub fn grant(&self, workers: usize) -> Result<Grant, String> {
        let mut pool = self.pool.lock().unwrap_or_else(PoisonError::into_inner);
        let sharing = pool.held > 0;
        let free_workers = self.total.workers.saturating_sub(pool.held_workers);
        let free = pool.free_descriptors;
        let ask = self.total.descriptors.div_ceil(2);
        let share = Share {
            workers: workers
                .min(if sharing {
                    free_workers.div_ceil(2)
                } else {
                    free_workers
                })
                .max(1),
            descriptors: if sharing {
                ask.min(free.div_ceil(2)).max(MIN_DESCRIPTORS)
            } else {
                ask
            }
            .min(free),
        };
        if share.descriptors == 0 {
            return Err(
                "Other scans are holding every file this app may keep open for scanning. Wait for one to finish or cancel it, then scan again."
                    .into(),
            );
        }
        pool.held_workers += share.workers;
        pool.free_descriptors -= share.descriptors;
        pool.held += 1;
        Ok(Grant {
            share,
            budget: self.clone(),
        })
    }
    /// What no scan holds: workers never below zero, since starts may oversubscribe them.
    #[cfg(test)]
    pub fn free(&self) -> Share {
        let pool = self.pool.lock().unwrap();
        Share {
            workers: self.total.workers.saturating_sub(pool.held_workers),
            descriptors: pool.free_descriptors,
        }
    }
    #[cfg(test)]
    pub fn total(&self) -> Share {
        self.total
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(Share {
            workers: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(8)
                .min(disk_scan::MAX_WORKERS),
            descriptors: disk_scan::default_descriptor_limit(),
        })
    }
}

impl Grant {
    pub fn share(&self) -> Share {
        self.share
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        let mut pool = self
            .budget
            .pool
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        pool.held_workers -= self.share.workers;
        pool.free_descriptors += self.share.descriptors;
        pool.held -= 1;
    }
}

/// Raises this process's own soft RLIMIT_NOFILE toward its hard limit, capped at `MAX_SOFT_LIMIT`.
/// An app launched from Finder starts at 256, which would shrink every scan's descriptor share. If the
/// system refuses, the old limit stays. Touches only this process: no launchctl, no system setting.
pub fn raise_descriptor_limit() {
    let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return;
    }
    if let Some(soft) = raised_soft_limit(limit.rlim_cur, limit.rlim_max) {
        let raised = libc::rlimit {
            rlim_cur: soft,
            rlim_max: limit.rlim_max,
        };
        let _ = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) };
    }
}
fn raised_soft_limit(soft: libc::rlim_t, hard: libc::rlim_t) -> Option<libc::rlim_t> {
    let target = hard.min(MAX_SOFT_LIMIT);
    (target > soft).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(workers: usize, descriptors: usize) -> Budget {
        Budget::new(Share {
            workers,
            descriptors,
        })
    }
    // Descriptors always add up to the total; workers may exceed it, never going below the minimum of one each.
    fn held(budget: &Budget, grants: &[Grant]) -> Share {
        let total = budget.total();
        let free = budget.free();
        let sum = Share {
            workers: grants.iter().map(|g| g.share().workers).sum(),
            descriptors: grants.iter().map(|g| g.share().descriptors).sum(),
        };
        assert_eq!(sum.descriptors + free.descriptors, total.descriptors);
        assert_eq!(free.workers, total.workers.saturating_sub(sum.workers));
        sum
    }

    #[test]
    fn a_lone_scan_gets_what_it_asks_for_and_returns_it() {
        let budget = budget(12, 512);
        let grant = budget.grant(6).unwrap();
        // Half the descriptors: a scan never needs more, and the rest serves later scans.
        assert_eq!(
            grant.share(),
            Share {
                workers: 6,
                descriptors: 256
            }
        );
        assert_eq!(budget.free().workers, 6);
        drop(grant);
        assert_eq!(budget.free(), budget.total());
        // An oversized ask is capped at the total, and a zero ask still gets one worker.
        assert_eq!(budget.grant(99).unwrap().share().workers, 12);
        assert_eq!(budget.grant(0).unwrap().share().workers, 1);
    }

    #[test]
    fn scans_beside_others_get_at_most_half_of_what_is_free() {
        let budget = budget(12, 512);
        let grants: Vec<_> = [4, 4, 4, 4]
            .into_iter()
            .map(|ask| budget.grant(ask).unwrap())
            .collect();
        let workers: Vec<_> = grants.iter().map(|g| g.share().workers).collect();
        assert_eq!(workers, [4, 4, 2, 1]);
        let descriptors: Vec<_> = grants.iter().map(|g| g.share().descriptors).collect();
        assert_eq!(descriptors, [256, 128, 64, 32]);
        held(&budget, &grants);
    }

    #[test]
    fn a_four_thread_mac_scans_a_second_disk_beside_a_scan_holding_every_worker() {
        let budget = budget(4, 512);
        // A volume scan asks for four workers on a four-thread Mac and takes them all.
        let first = budget.grant(4).unwrap();
        assert_eq!(first.share().workers, 4);
        assert_eq!(budget.free().workers, 0);
        // The next scans still start, with one worker each (oversubscribing by a thread).
        let second = budget.grant(4).unwrap();
        let third = budget.grant(4).unwrap();
        assert_eq!(second.share().workers, 1);
        assert_eq!(third.share().workers, 1);
        let grants = [first, second, third];
        let sum = held(&budget, &grants);
        assert_eq!(sum.workers, 6);
        assert_eq!(budget.free().workers, 0);
        // Everything is returned, the oversubscribed workers included.
        drop(grants);
        assert_eq!(budget.free(), budget.total());
        assert_eq!(budget.pool.lock().unwrap().held_workers, 0);
    }

    #[test]
    fn a_scan_stuck_holding_every_worker_does_not_lock_the_others_out() {
        let budget = budget(8, 512);
        let _stuck = budget.grant(8).unwrap();
        for _ in 0..3 {
            let grant = budget.grant(5).unwrap();
            assert_eq!(grant.share().workers, 1);
        }
    }

    #[test]
    fn descriptors_never_exceed_the_total_and_run_out_only_after_a_handful_of_scans() {
        for (workers, descriptors) in [(1, 512), (4, 512), (12, 512), (16, 512), (4, 192)] {
            let budget = budget(workers, descriptors);
            let mut grants = vec![];
            while let Ok(grant) = budget.grant(8) {
                assert!(grant.share().workers >= 1);
                assert!(grant.share().descriptors >= 1);
                grants.push(grant);
                let sum = held(&budget, &grants);
                assert!(sum.descriptors <= descriptors);
                // Workers exceed the total only by the one-worker minimum of each later scan.
                assert!(sum.workers < workers + grants.len());
                assert!(grants.len() < 64, "never ran out");
            }
            // Seven scans at once with 512; six with the 192 an unraised Finder launch would leave.
            assert!(grants.len() >= if descriptors == 512 { 7 } else { 6 });
            assert_eq!(budget.free().descriptors, 0);
            let refused = budget.grant(1).err().unwrap();
            assert!(refused.contains("every file"), "{refused}");
            drop(grants);
            assert_eq!(budget.free(), budget.total());
        }
    }

    #[test]
    fn descriptor_shares_run_down_to_the_floor_and_then_the_start_is_refused() {
        let shares = |total| {
            let budget = budget(8, total);
            let grants: Vec<_> = std::iter::from_fn(|| budget.grant(4).ok()).collect();
            grants
                .iter()
                .map(|g| g.share().descriptors)
                .collect::<Vec<_>>()
        };
        assert_eq!(shares(512), [256, 128, 64, 32, 16, 8, 8]);
        // The 192 left by a soft limit of 256 that was not raised.
        assert_eq!(shares(192), [96, 48, 24, 12, 8, 4]);
    }

    #[test]
    fn three_concurrent_scans_keep_a_share_above_the_measured_peak() {
        let budget = budget(10, 512);
        let a = budget.grant(5).unwrap();
        let b = budget.grant(5).unwrap();
        let c = budget.grant(5).unwrap();
        assert!([&a, &b, &c].iter().all(|g| g.share().descriptors >= 64));
        let sum = held(&budget, &[a, b, c]);
        assert!(sum.descriptors <= 512);
    }

    #[test]
    fn a_refused_start_takes_nothing_and_a_finished_scan_makes_room() {
        let budget = budget(3, 16);
        let first = budget.grant(2).unwrap();
        let second = budget.grant(2).unwrap();
        assert_eq!(budget.free().descriptors, 0);
        let free = budget.free();
        assert!(budget.grant(1).err().unwrap().contains("every file"));
        assert_eq!(budget.free(), free);
        drop(first);
        assert_eq!(budget.free().descriptors, 8);
        assert_eq!(budget.grant(2).unwrap().share().descriptors, 8);
        drop(second);
    }

    #[test]
    fn a_share_is_returned_on_every_way_a_scan_ends() {
        fn scan(grant: Grant, outcome: Result<(), &'static str>) -> Result<(), &'static str> {
            let _grant = grant;
            outcome?;
            Ok(())
        }
        let budget = budget(8, 512);
        // Finished, failed and a panicking scan thread all drop the grant.
        assert!(scan(budget.grant(4).unwrap(), Ok(())).is_ok());
        assert!(scan(budget.grant(4).unwrap(), Err("failed")).is_err());
        let grant = budget.grant(4).unwrap();
        let panicked = std::thread::spawn(move || {
            let _grant = grant;
            panic!("scan thread failed");
        })
        .join();
        assert!(panicked.is_err());
        assert_eq!(budget.free(), budget.total());
        assert_eq!(budget.pool.lock().unwrap().held, 0);
    }

    #[test]
    fn the_soft_descriptor_limit_is_raised_toward_the_hard_limit_but_never_lowered_or_past_the_cap()
    {
        let infinity = libc::RLIM_INFINITY;
        assert_eq!(raised_soft_limit(256, 1 << 20), Some(10240));
        assert_eq!(raised_soft_limit(256, infinity), Some(10240));
        assert_eq!(raised_soft_limit(256, 4096), Some(4096));
        assert_eq!(raised_soft_limit(256, 256), None);
        assert_eq!(raised_soft_limit(10240, infinity), None);
        assert_eq!(raised_soft_limit(20000, 1 << 20), None);
    }

    #[test]
    fn raising_the_descriptor_limit_keeps_or_widens_this_process_limit() {
        let read = || {
            let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
            assert_eq!(
                unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
                0
            );
            limit
        };
        let before = read();
        raise_descriptor_limit();
        let after = read();
        assert!(after.rlim_cur >= before.rlim_cur);
        assert_eq!(after.rlim_max, before.rlim_max);
        assert!(after.rlim_cur <= before.rlim_max.min(MAX_SOFT_LIMIT).max(before.rlim_cur));
    }
}
