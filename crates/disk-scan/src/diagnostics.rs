//! Bounded per-thread diagnostics, plus an optional requested QoS applied only to each scan worker.
//! Automatic preserves native behavior; explicit QoS never assigns CPU affinity.
//! `busy_us` includes active directory tasks (including lock waits) and final row publication;
//! `queue_wait_us` covers only dispatch/pop mutex and condition-variable waits. Queue publication
//! and path-copy overhead remain unclassified within busy time. Other phase fields are disjoint
//! intervals inside busy time, with microsecond rounding. CPU time is sampled only at thread start
//! and exit, so live rows carry None. QoS is the requested class sampled at startup, not effective
//! priority, CPU affinity, or evidence of performance/efficiency core placement.
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerStats {
    pub id: usize,
    pub entries: u64,
    pub directories: u64,
    pub relative_opens: u64,
    pub absolute_reopens: u64,
    pub errors: u64,
    pub elapsed_us: u64,
    pub busy_us: u64,
    pub cpu_us: Option<u64>,
    pub queue_wait_us: u64,
    pub store_wait_us: u64,
    pub metadata_us: u64,
    pub commit_us: u64,
    pub progress_us: u64,
    pub qos: String,
    pub qos_error: Option<i32>,
}
pub(crate) fn micros(duration: Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}
pub(crate) struct Tracker {
    pub stats: WorkerStats,
    started: Instant,
    cpu_start: Option<u64>,
}
impl Tracker {
    pub fn new(id: usize) -> Self {
        let started = Instant::now();
        let cpu_start = cpu_clock();
        Self {
            stats: WorkerStats {
                id,
                qos: qos_class(),
                ..WorkerStats::default()
            },
            started,
            cpu_start,
        }
    }
    /// A live row, written into `row` so its `qos` buffer is reused: the caller holds the store lock.
    pub fn snapshot_into(&self, row: &mut WorkerStats) {
        // Derived `Clone` leaves `clone_from` at its default, which reallocates `qos`.
        // The literal names every field, so a new one cannot be forgotten.
        let s = &self.stats;
        let mut qos = std::mem::take(&mut row.qos);
        qos.clone_from(&s.qos);
        *row = WorkerStats {
            id: s.id,
            entries: s.entries,
            directories: s.directories,
            relative_opens: s.relative_opens,
            absolute_reopens: s.absolute_reopens,
            errors: s.errors,
            elapsed_us: micros(self.started.elapsed()),
            busy_us: s.busy_us,
            cpu_us: s.cpu_us,
            queue_wait_us: s.queue_wait_us,
            store_wait_us: s.store_wait_us,
            metadata_us: s.metadata_us,
            commit_us: s.commit_us,
            progress_us: s.progress_us,
            qos,
            qos_error: s.qos_error,
        };
    }
    pub fn finish(&mut self) {
        self.stats.cpu_us = self
            .cpu_start
            .zip(cpu_clock())
            .and_then(|(start, end)| end.checked_sub(start));
        self.stats.elapsed_us = micros(self.started.elapsed());
    }
}
#[cfg(any(target_os = "macos", test))]
fn apply_qos_with(
    policy: crate::ScanQos,
    mut setter: impl FnMut(libc::c_uint) -> libc::c_int,
) -> Option<i32> {
    let class = match policy {
        crate::ScanQos::Automatic => return None,
        crate::ScanQos::UserInitiated => 0x19,
        crate::ScanQos::Background => 0x09,
    };
    let status = setter(class);
    if status == 0 {
        None
    } else {
        Some(status)
    }
}
#[cfg(target_os = "macos")]
pub(crate) fn apply_qos(policy: crate::ScanQos) -> Option<i32> {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(
            class: libc::c_uint,
            relative_priority: libc::c_int,
        ) -> libc::c_int;
    }
    apply_qos_with(policy, |class| unsafe {
        pthread_set_qos_class_self_np(class, 0)
    })
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn apply_qos(policy: crate::ScanQos) -> Option<i32> {
    match policy {
        crate::ScanQos::Automatic => None,
        _ => Some(libc::ENOTSUP),
    }
}
/// Make this thread's listings fail (EDEADLK) on dataless File Provider folders instead of
/// materializing them over the network. Best effort: the result is ignored.
#[cfg(target_os = "macos")]
pub(crate) fn disable_dataless_materialization() {
    unsafe extern "C" {
        fn setiopolicy_np(
            iotype: libc::c_int,
            scope: libc::c_int,
            policy: libc::c_int,
        ) -> libc::c_int;
    }
    unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_THREAD,
            IOPOL_MATERIALIZE_DATALESS_FILES_OFF,
        );
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn disable_dataless_materialization() {}
// Values from <sys/resource.h>; libc does not export them.
#[cfg(target_os = "macos")]
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: libc::c_int = 3;
#[cfg(target_os = "macos")]
const IOPOL_SCOPE_THREAD: libc::c_int = 1;
#[cfg(target_os = "macos")]
const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: libc::c_int = 1;
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn cpu_clock() -> Option<u64> {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } != 0
        || ts.tv_sec < 0
        || !(0..1_000_000_000).contains(&ts.tv_nsec)
    {
        return None;
    }
    (ts.tv_sec as u64)
        .checked_mul(1_000_000)?
        .checked_add(ts.tv_nsec as u64 / 1000)
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn cpu_clock() -> Option<u64> {
    None
}
#[cfg(target_os = "macos")]
fn qos_class() -> String {
    unsafe extern "C" {
        fn pthread_get_qos_class_np(
            thread: libc::pthread_t,
            class: *mut libc::c_uint,
            relative_priority: *mut libc::c_int,
        ) -> libc::c_int;
    }
    let mut class = 0;
    let status =
        unsafe { pthread_get_qos_class_np(libc::pthread_self(), &mut class, std::ptr::null_mut()) };
    if status != 0 {
        return "unavailable".into();
    }
    match class {
        0 => "unspecified",
        0x09 => "background",
        0x11 => "utility",
        0x15 => "default",
        0x19 => "userInitiated",
        0x21 => "userInteractive",
        _ => return format!("unknown({class})"),
    }
    .into()
}
#[cfg(not(target_os = "macos"))]
fn qos_class() -> String {
    "unavailable".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thread_telemetry_getters_are_read_only_and_valid() {
        let mut t = Tracker::new(1);
        let mut row = WorkerStats::default();
        t.snapshot_into(&mut row);
        assert!(!row.qos.is_empty());
        assert!(row.cpu_us.is_none());
        t.finish();
        assert_eq!(t.stats.id, 1);
        #[cfg(target_os = "macos")]
        assert_ne!(t.stats.qos, "unavailable");
        assert!(t.stats.elapsed_us >= row.elapsed_us);
    }
    #[test]
    fn snapshot_into_overwrites_a_stale_row_and_reuses_its_qos_buffer() {
        let mut t = Tracker::new(2);
        t.stats = WorkerStats {
            entries: 1,
            directories: 2,
            relative_opens: 3,
            absolute_reopens: 4,
            errors: 5,
            busy_us: 6,
            cpu_us: Some(7),
            queue_wait_us: 8,
            store_wait_us: 9,
            metadata_us: 10,
            commit_us: 11,
            progress_us: 12,
            qos_error: Some(13),
            ..t.stats
        };
        let mut row = WorkerStats {
            id: 99,
            errors: 99,
            cpu_us: None,
            qos: String::with_capacity(64),
            qos_error: None,
            ..WorkerStats::default()
        };
        let buffer = row.qos.as_ptr();
        t.snapshot_into(&mut row);
        assert_eq!(row.qos.as_ptr(), buffer);
        let mut expected = t.stats.clone();
        expected.elapsed_us = row.elapsed_us;
        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
    }
    #[test]
    fn serializes_worker_fields_in_camel_case() {
        let v = serde_json::to_value(WorkerStats::default()).unwrap();
        assert!(v.get("queueWaitUs").is_some());
        assert!(v.get("relativeOpens").is_some());
        assert!(v.get("absoluteReopens").is_some());
        assert!(v.get("cpuUs").unwrap().is_null());
        assert!(v.get("qosError").unwrap().is_null());
        assert!(v.get("queue_wait_us").is_none());
    }
    #[test]
    fn automatic_does_not_call_setter_and_errors_are_recorded() {
        assert_eq!(
            apply_qos_with(crate::ScanQos::Automatic, |_| panic!(
                "Automatic must not set QoS"
            )),
            None
        );
        assert_eq!(
            apply_qos_with(crate::ScanQos::UserInitiated, |class| {
                assert_eq!(class, 0x19);
                libc::EPERM
            }),
            Some(libc::EPERM)
        );
        assert_eq!(
            apply_qos_with(crate::ScanQos::Background, |class| {
                assert_eq!(class, 0x09);
                0
            }),
            None
        );
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn explicit_qos_applies_only_to_disposable_worker_threads() {
        for (policy, label) in [
            (crate::ScanQos::UserInitiated, "userInitiated"),
            (crate::ScanQos::Background, "background"),
        ] {
            std::thread::spawn(move || {
                assert_eq!(apply_qos(policy), None);
                assert_eq!(qos_class(), label);
            })
            .join()
            .unwrap();
        }
    }
    #[test]
    #[cfg(target_os = "macos")]
    fn worker_threads_do_not_materialize_dataless_files() {
        unsafe extern "C" {
            fn getiopolicy_np(iotype: libc::c_int, scope: libc::c_int) -> libc::c_int;
        }
        std::thread::spawn(|| {
            disable_dataless_materialization();
            let policy = unsafe {
                getiopolicy_np(
                    IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                    IOPOL_SCOPE_THREAD,
                )
            };
            assert_eq!(policy, IOPOL_MATERIALIZE_DATALESS_FILES_OFF);
        })
        .join()
        .unwrap();
    }
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn explicit_qos_reports_unsupported_without_changing_default() {
        assert_eq!(apply_qos(crate::ScanQos::Automatic), None);
        assert_eq!(apply_qos(crate::ScanQos::Background), Some(libc::ENOTSUP));
    }
}
