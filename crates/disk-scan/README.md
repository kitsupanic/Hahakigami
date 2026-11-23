# disk-scan

Metadata-only disk scanning, with Darwin `getattrlistbulk` enumeration and a portable fallback. It keeps a flat indexed tree in a caller-owned `Arc<RwLock<Snapshot>>`; progress callbacks run without holding that store lock. File contents are never opened. Symbolic links are listed without traversal. Cancelling keeps the partial tree.

`ScanOptions.workers` is bounded to `1..=MAX_WORKERS` (16); the library default remains 8. Desktop settings additionally cap the choice to available CPU threads and capture it once per scan. The CLI reports its effective count using the same shared bound. These are worker threads for concurrent folder enumeration, not a fixed CPU-percentage budget.

`ScanOptions.qos` accepts `ScanQos::Automatic` (default), `UserInitiated`, or `Background`. Automatic makes no setter call. On macOS, explicit choices apply the requested QoS once to each fresh scan worker thread; the UI/coordinator and process-wide policies stay untouched. QoS does not promise performance/efficiency core placement. A setter failure appears in the worker's `qos_error` (`qosError` in JSON), independently of filesystem errors; explicit requests report unsupported status on other platforms.

`Snapshot.worker_stats` records ordered per-worker entry/task/issue counts, elapsed and processing wall times, CPU time, metadata/store/commit/progress/dispatch-wait times, requested QoS and directory-open attempts. Counters/timers are local to each worker and published per directory and on exit. CPU time uses two thread-clock reads per worker; unavailable readings remain `None`. Entries sum to total scan entries minus the pre-created root. Metadata time includes opening/identity checks, filesystem waits, parsing and path construction; CPU time overlaps these phases. `finalization_us` records coordinator aggregation, outside worker CPU totals. `descriptor_limit` and `descriptor_peak` describe retained leases; absolute reopens count secure path-walk fallback attempts. Instrumentation does not change traversal, priorities, affinity, descriptor policy or worker defaults.

`ScanRoot::open(path, registered_identity)` anchors desktop authority without canonicalizing or retargeting it. Open it before launching the worker, then pass the opaque handle to `scan_live_authorized`; replacing root or ancestor paths cannot redirect that scan. `scan_live` resolves an explicitly supplied CLI path once and anchors its original identity before scanning; `scan` is a convenience API returning a cloned snapshot. Use `platform_identity` for native path identity comparisons. Node IDs are stable only within one scan. Live allocated totals deduplicate hard links immediately; final aggregation assigns their allocation to the lexicographically first native path. The root descriptor is opened one component at a time with no-follow flags; queued children open relative to retained authorized parent descriptors and validate their identities. Retained descriptors are capped at 512 (or a lower limit preserving process headroom); under pressure, traversal falls back to secure component-by-component opens. The queue uses depth-first scheduling. Both native and portable enumeration operate relative to that validated descriptor; Auto falls back only for unsupported native enumeration. Only metadata-reported multiply-linked files need allocation ownership tracking; final aggregation runs in linear time without sorting the complete tree. Live ancestor updates are batched once per directory. Final macOS allocation additionally assigns verified full-clone data-fork allocation once, preserving each distinct inode’s resource-fork allocation. Raw allocation remains available; attributed allocation does not estimate reclaimable disk space.

The startup disk allows its system device and Data device, includes Data-only directories beneath `/System/Volumes/Data`, excludes verified aliases listed in `/usr/share/firmlinks` and `/Volumes`, and deduplicates directory identities. Other scan targets stop at device boundaries. Excluded mounted locations are expected traversal boundaries, recorded separately in `ScanStats.excluded` and `Snapshot.exclusions`; they do not make a result incomplete. Inaccessible or disappearing entries remain actual errors and mark the result incomplete. Errors and exclusions each retain at most 10,000 detail records while preserving their separate full counters.

`Snapshot.accounting` distinguishes historical `hardLinksV1` from current `fullClonesV1` results. Initial clone attributes accompany checked bulk enumeration outside the snapshot lock, with sparse descriptor-relative per-file fallback results. Single-inode clone groups retain raw allocation and unverified metadata without a final query, because they cannot produce a clone deduction. Every member and hard-link alias in a multi-inode group remains subject to revalidation. Final validation follows recorded directory identities from the held root, rechecks cached ancestry on parent switches, and uses bounded existing scan workers. Dense groups stream checked attributes from independently reopened cursors; small/sparse groups retain per-file queries. Every participating file is checked, and any failed member invalidates its whole clone group after workers join. Only complete, mutually consistent clone claims authorize deductions. Missing/malformed/changed metadata retains raw allocation and adds separate accounting warnings. A successful metadata query establishes device support even when the advertised capability underreports it; only explicit unsupported errors disable probing. Shared or unresolved baselines cannot seed incremental reuse, and newly discovered sharing causes one private full fallback. Sparse clone metadata and derived inspector attribution preserve the 80-byte compact record layout. Records grow in bounded 4,096-record chunks; completed chunks stay in place when later nodes are appended. `storage_identity` is an opaque first-payload identity, not a contiguous-array guarantee. Diagnostic `record_chunks` storage includes the outer chunk headers; archive/cache wire formats are unchanged.

## Checks and benchmark commands

Run from the workspace root:

```
cargo test -p disk-scan
cargo run --release -p disk-scan --example scan -- /path/to/folder 8 bulk
cargo run --release -p disk-scan --example scan -- /path/to/folder 4 portable
cargo run --release -p disk-scan --example scan -- /path/to/folder 8 bulk --qos userInitiated
cargo run --release -p disk-scan --example synthetic -- 1000000
```

`scan_qos` reports the requested policy; each worker’s `qos` reports its requested class sampled after the setter attempt, not effective priority or CPU placement. The optional trailing `--qos` flag preserves existing CLI positional arguments.

`peak_rss_native_units` is bytes on macOS, kilobytes on Linux. Release timings include enumeration and final aggregation. The synthetic example measures construction, final aggregation, and sorting a 100-entry page; it does not exercise Tauri or Canvas.

## Exploratory measurements (2026-10-10)

Apple Silicon host, release build, GitHub directory with approximately 659,000 entries. One run per worker count, in ascending order; the directory changed slightly during early runs. These are tuning measurements, not the required controlled OpenDisk comparison.

| Backend | Workers | Scan ms | Peak RSS bytes |
| --- | ---: | ---: | ---: |
| Bulk | 1 | 11241 | 332955648 |
| Bulk | 2 | 6241 | 373145600 |
| Bulk | 4 | 3938 | 408043520 |
| Bulk | 8 | 2942 | 409468928 |
| Portable | 4 | 5649 | 372178944 |

The final Bulk/8 and Portable/4 runs agreed exactly: 658,650 entries; 61,819,572,224 reported allocated bytes; zero errors. Default worker count is eight, the fastest measured option.

Synthetic million-entry run: 77 ms construction, 578 ms final aggregation, 3 ms sort/page, 277,626,880 bytes peak RSS. Native parser, filesystem fixture parity, cancellation, hard links, symlink cycles, mount policy, permissions, disappearing directories, unusual names, and progress callback access are covered by unit tests.

The initial tuning measurements predate descriptor traversal hardening. Rerun benchmarks against the current release build before treating those timings as current performance evidence.

`ScanIssue.error_kind` preserves the standard I/O error-kind name and `raw_os_error` preserves errno when present; both fields default to `None` for legacy/static issues. Filesystem issue counts are unchanged.

Interrupted Unix metadata operations use at most four attempts. An interrupted directory listing discards its partial results and restarts on a fresh independently opened cursor for the same held, identity-checked directory. A persistent interrupt keeps its original kind/errno; cancellation is checked between listing attempts. Non-interrupted issues prevent restart and remain visible, while parser/budget failures and close operations are never retried.

System-restriction presentation requires macOS, a native PermissionDenied kind and EPERM/EACCES, and a conservative known OS path. `is_system_restriction` never reads the filesystem or matches message text. `ScanStats.restricted_system_issues` counts all such events, including beyond retained detail limits, while every event still increments errors and marks the scan incomplete. User paths, `/usr/local`, ambiguous paths, and unrelated I/O failures remain ordinary issues.

## Conservative incremental API

`scan_live_incremental_authorized` consumes an `IncrementalSeed` containing a shared (`Arc`) completed snapshot and dirty paths. The application is responsible for establishing trustworthy journal continuity and local-filesystem eligibility. The core validates cached topology/root identity and rejects incomplete, excluded, startup, shared or unresolved-accounting seeds. Known hard-link groups remain eligible with the existing alias validation. Clean listings move into the normal descriptor traversal; all visited directories still open with their expected identities. Dirty subtrees enumerate normally; an event path whose parent has no cached node, or whose last name has none under a cached parent, is compared with the nearest cached ancestor's children after Unicode (NFD) normalization and case folding, and only the cached sibling it spells that way (a differently spelled or normalized directory) enumerates whole; no match means the name is new. New hard links, clone sharing or unresolved fresh metadata cause one private full retry against a reopened held-root descriptor.

`ReuseStats` reports reused metadata entries, enumerated directories and any full-fallback reason. Candidate trees remain private through retries; progress exposes counts only until final/cancelled publication. Error results are labeled Failed. The standalone `scan` helper now transfers its final snapshot from the owned store rather than cloning the entire tree. No peak-memory reduction percentage is claimed.

Fixture parity and publication regressions run in the ordinary core suite. The opt-in release fixture can be run with `cargo test --release -p disk-scan incremental::tests::benchmark_twenty_thousand_entries -- --ignored --nocapture`. It excludes application journal replay and cache persistence.
